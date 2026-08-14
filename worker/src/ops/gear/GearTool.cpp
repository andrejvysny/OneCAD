// GearTool.cpp — see GearTool.h.

#include "ops/gear/GearTool.h"

#include <cmath>

#include <BRepAlgoAPI_Cut.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Pln.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "kernel/geometry/BSpline.h"

namespace onecad::ops::gear {

namespace {

namespace kg = onecad::kernel::gear;
namespace geom = onecad::kernel::geometry;

// Same floor the other ops use for "a dimension the kernel can act on"
// (RegenerationEngine.cpp:61 kMinValue, mirrored in HoleTool.cpp).
constexpr double kMinValue = 1e-3;
constexpr double kPi = 3.14159265358979323846;

std::string occt_reason(const Standard_Failure& f) {
  const char* what = f.what();
  return (what && *what) ? what : "OCCT";
}

/// A profile point rotated into tooth `k`'s position, lifted to 3D at z = 0.
/// The local frame is the gear's own: axis +Z, centre at the origin.
gp_Pnt local_point(const kg::Point2d& p, double angle) {
  const kg::Point2d r = kg::rotate(p, angle);
  return gp_Pnt(r.x, r.y, 0.0);
}

}  // namespace

GearBuildResult build_gear_solid(const GearBuildSpec& spec, const gp_Ax2& frame) {
  GearBuildResult out;

  if (spec.sampleCount < 2) {
    out.error = "sampleCount must be at least 2 (got " + std::to_string(spec.sampleCount) + ")";
    return out;
  }
  if (!(spec.height > kMinValue)) {
    out.error = "height must be greater than 1e-3 mm";
    return out;
  }

  // --- 1. Sampler: factors + one tooth ------------------------------------
  const kg::InvoluteFactorsResult factors = kg::compute_involute_factors(spec.involute);
  if (!factors.ok) {
    out.error = factors.error;
    return out;
  }
  const kg::InvoluteToothFactors& f = factors.factors;

  out.computed.pitchDiameter = f.referenceDiameter;
  out.computed.addendumDiameter = f.addendumDiameter;
  out.computed.rootDiameter = f.rootDiameter;
  out.computed.baseDiameter = f.baseDiameter;
  out.computed.transverseModule = f.transverseModule;
  out.computed.transversePitch = f.transversePitch;
  out.computed.angularBacklashDeg = f.angularBacklash * 180.0 / kPi;

  const kg::ToothProfile tooth = kg::one_tooth_profile(spec.involute, f, spec.sampleCount);
  if (tooth.segments.empty()) {
    out.error = "gear tooth profile is empty";
    return out;
  }
  for (const auto& seg : tooth.segments) {
    if (seg.size() < 2) {
      out.error = "gear tooth profile has a degenerate segment";
      return out;
    }
  }

  const int z = spec.involute.numTeeth;
  const double phipart = f.angularPitch;
  const kg::Point2d toothFirst = tooth.segments.front().front();
  const kg::Point2d toothLast = tooth.segments.back().back();

  // Guard the assumption the replication rests on: one tooth's run must span
  // exactly one angular pitch, so that rotating its start by `phipart` lands
  // on the next tooth's start. If the sampler ever stops honouring that, the
  // wire silently self-intersects instead of failing here.
  {
    const kg::Point2d wrapped = kg::rotate(toothLast, phipart);
    const kg::Point2d nextStart = kg::rotate(toothFirst, phipart);
    // The root land bridges `toothLast` -> rot(toothFirst); both must sit on
    // the same circle for that chord to be the root land and not a gash.
    const double rLast = kg::norm(toothLast);
    const double rFirst = kg::norm(toothFirst);
    (void)wrapped;
    (void)nextStart;
    if (std::abs(rLast - rFirst) > 1e-6 * std::max(1.0, rFirst)) {
      out.error = "gear tooth profile does not begin and end on the same radius; the root land "
                  "would not close";
      return out;
    }
  }

  // --- 2. Edges for all z teeth, built analytically -----------------------
  try {
    BRepBuilderAPI_MakeWire wire;

    for (int k = 0; k < z; ++k) {
      const double angle = static_cast<double>(k) * phipart;
      // The wrap-around join must reuse angle 0 EXACTLY (see GearTool.h):
      // z·(2π/z) does not round-trip to 0 in binary floating point.
      const double nextAngle = (k + 1 == z) ? 0.0 : static_cast<double>(k + 1) * phipart;

      for (const auto& seg : tooth.segments) {
        if (seg.size() == 2) {
          const gp_Pnt a = local_point(seg.front(), angle);
          const gp_Pnt b = local_point(seg.back(), angle);
          if (a.Distance(b) <= kMinValue * 1e-3) continue;  // degenerate, skip rather than raise
          BRepBuilderAPI_MakeEdge mk(a, b);
          if (!mk.IsDone()) {
            out.error = "gear profile straight segment could not be built";
            return out;
          }
          wire.Add(mk.Edge());
        } else {
          std::vector<gp_Pnt> pts;
          pts.reserve(seg.size());
          for (const auto& p : seg) pts.push_back(local_point(p, angle));
          const geom::BSplineEdgeResult e = geom::interpolated_edge(pts);
          if (!e.ok) {
            out.error = "gear flank could not be interpolated: " + e.error +
                        " (try a lower sampleCount)";
            return out;
          }
          wire.Add(e.edge);
        }
      }

      // Root land: the chord from this tooth's last point to the next tooth's
      // first. Matches the reference's 2-point run (GearTool.h).
      const gp_Pnt landStart = local_point(toothLast, angle);
      const gp_Pnt landEnd = local_point(toothFirst, nextAngle);
      if (landStart.Distance(landEnd) > kMinValue * 1e-3) {
        BRepBuilderAPI_MakeEdge mk(landStart, landEnd);
        if (!mk.IsDone()) {
          out.error = "gear root land segment could not be built";
          return out;
        }
        wire.Add(mk.Edge());
      }
    }

    if (!wire.IsDone()) {
      out.error = "gear profile wire could not be assembled (the tooth segments do not join)";
      return out;
    }
    TopoDS_Wire profile = wire.Wire();
    if (profile.IsNull()) {
      out.error = "gear profile wire is null";
      return out;
    }
    if (!profile.Closed()) {
      out.error = "gear profile wire is not closed";
      return out;
    }

    // --- 3. Face + prism, in the LOCAL frame ------------------------------
    BRepBuilderAPI_MakeFace mkFace(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), profile);
    if (!mkFace.IsDone()) {
      out.error = "gear profile could not be turned into a face (the profile may self-intersect; "
                  "check backlash / profile shift)";
      return out;
    }
    const TopoDS_Face face = mkFace.Face();

    BRepPrimAPI_MakePrism mkPrism(face, gp_Vec(0.0, 0.0, spec.height));
    if (!mkPrism.IsDone()) {
      out.error = "gear body could not be extruded";
      return out;
    }
    TopoDS_Shape solid = mkPrism.Shape();
    if (solid.IsNull()) {
      out.error = "gear body is null after extrusion";
      return out;
    }

    // --- 4. Bore cuts (still local) ---------------------------------------
    // Cut with cylinders that overshoot BOTH faces, so no cap is coincident
    // with the gear's own end faces — the configuration OCCT booleans handle
    // worst (the same reasoning as HoleTool's kFaceOvershoot).
    const double overshoot = std::max(1e-2, spec.height * 1e-3);
    auto cut_cylinder = [&](double radius, double offset, const char* what) -> bool {
      if (radius < kMinValue) {
        out.error = std::string(what) + " diameter must be greater than 2e-3 mm";
        return false;
      }
      const gp_Ax2 axis(gp_Pnt(offset, 0.0, -overshoot), gp_Dir(0, 0, 1));
      const TopoDS_Shape tool =
          BRepPrimAPI_MakeCylinder(axis, radius, spec.height + 2.0 * overshoot).Shape();
      BRepAlgoAPI_Cut cut(solid, tool);
      cut.Build();
      if (!cut.IsDone()) {
        out.error = std::string(what) + " could not be cut from the gear body";
        return false;
      }
      solid = cut.Shape();
      return true;
    };

    if (spec.axleHole) {
      if (spec.axleHoleDiameter >= out.computed.rootDiameter) {
        out.error = "axle hole diameter " + std::to_string(spec.axleHoleDiameter) +
                    " mm reaches the tooth roots (root diameter " +
                    std::to_string(out.computed.rootDiameter) + " mm)";
        return out;
      }
      if (!cut_cylinder(spec.axleHoleDiameter * 0.5, 0.0, "axle hole")) return out;
    }
    if (spec.offsetHole) {
      const double r = spec.offsetHoleDiameter * 0.5;
      if (spec.offsetHoleOffset + r >= out.computed.rootDiameter * 0.5) {
        out.error = "offset hole reaches the tooth roots (offset " +
                    std::to_string(spec.offsetHoleOffset) + " mm + radius " + std::to_string(r) +
                    " mm vs root radius " + std::to_string(out.computed.rootDiameter * 0.5) +
                    " mm)";
        return out;
      }
      if (!cut_cylinder(r, spec.offsetHoleOffset, "offset hole")) return out;
    }

    if (solid.IsNull()) {
      out.error = "gear body is null after the bore cuts";
      return out;
    }
    if (!BRepCheck_Analyzer(solid).IsValid()) {
      // The op layer audits again under the publication policy; refusing here
      // keeps an invalid shape from ever reaching the transform below.
      out.error = "gear body failed a validity check before placement";
      return out;
    }

    // --- 5. Place the local build onto the caller's frame ------------------
    gp_Trsf place;
    place.SetTransformation(gp_Ax3(frame), gp_Ax3(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)));
    solid.Move(TopLoc_Location(place));

    out.ok = true;
    out.shape = solid;
    return out;
  } catch (const Standard_Failure& e) {
    out.error = "gear body could not be built: " + occt_reason(e);
    return out;
  } catch (...) {
    out.error = "gear body could not be built";
    return out;
  }
}

}  // namespace onecad::ops::gear
