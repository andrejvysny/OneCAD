#include "kernel/fillet/AnalyticBlend.h"

#include <cmath>

#include <BRepAdaptor_Surface.hxx>
#include <BRepLProp_SLProps.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_Orientation.hxx>
#include <gp.hxx>
#include <gp_Ax1.hxx>
#include <gp_Cylinder.hxx>
#include <gp_Dir.hxx>
#include <gp_Lin.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "kernel/validation/GeometryPrecision.h"

namespace onecad::kernel::fillet {
namespace {

namespace validation = onecad::kernel::validation;

// Same normal-resolution argument BlendEvidence hands BRepLProp_SLProps.
constexpr double kNormalTolerance = 1.0e-9;

AnalyticBlendProof against_plane(const gp_Ax1 &axis, double radius,
                                 const gp_Pln &plane,
                                 const validation::GeometryPrecisionContext &precision) {
  // A rolling ball of radius R touching a plane leaves its centre line parallel
  // to that plane, R away. Both halves are exact statements about the analytic
  // surfaces; neither is sampled.
  if (!axis.Direction().IsNormal(plane.Axis().Direction(),
                                 precision.semantic_angular()))
    return AnalyticBlendProof::Failed;
  if (std::abs(plane.Distance(axis.Location()) - radius) >
      precision.semantic_length())
    return AnalyticBlendProof::Failed;
  return AnalyticBlendProof::Proved;
}

AnalyticBlendProof against_cylinder(const gp_Ax1 &axis, double radius,
                                    const gp_Cylinder &support,
                                    const validation::GeometryPrecisionContext &precision) {
  if (!axis.Direction().IsParallel(support.Axis().Direction(),
                                   precision.semantic_angular()))
    return AnalyticBlendProof::Failed;
  // POINT to line, never line to line. `gp_Lin::Distance(const gp_Lin&)` takes
  // its parallel branch only at `gp::Resolution()` (~DBL_MIN), so a pair we just
  // accepted as parallel at 1e-7 rad still falls into its SKEW branch, where the
  // common perpendicular is a cross product of magnitude ~1e-16 whose direction
  // is pure rounding noise. That returns |offset|·cos(random angle) and can fail
  // a genuine blend. Once the axes are parallel, the distance from either
  // location to the other line is the well-conditioned answer.
  const double distance = gp_Lin(axis).Distance(support.Axis().Location());
  const double tolerance = precision.semantic_length();
  // A face is not a blend of itself. When the support is the SAME surface as the
  // blend — coaxial and equal radius — the internal branch below is satisfied
  // trivially by |R_s - R| == 0. That form is REACHABLE: the cylindrical face of
  // a drilled hole split by its seams makes each half a G1 "support" of the
  // other, and accepting it would authorize destroying the hole.
  if (distance <= tolerance && std::abs(support.Radius() - radius) <= tolerance)
    return AnalyticBlendProof::Failed;
  const double external = support.Radius() + radius;
  const double internal = std::abs(support.Radius() - radius);
  if (std::abs(distance - external) <= tolerance ||
      std::abs(distance - internal) <= tolerance)
    return AnalyticBlendProof::Proved;
  return AnalyticBlendProof::Failed;
}

AnalyticBlendProof against_support(const gp_Ax1 &axis, double radius,
                                   const TopoDS_Face &support,
                                   const validation::GeometryPrecisionContext &precision) {
  if (support.IsNull())
    return AnalyticBlendProof::Unknown;
  BRepAdaptor_Surface surface(support);
  switch (surface.GetType()) {
  case GeomAbs_Plane:
    return against_plane(axis, radius, surface.Plane(), precision);
  case GeomAbs_Cylinder:
    return against_cylinder(axis, radius, surface.Cylinder(), precision);
  default:
    return AnalyticBlendProof::Unknown;
  }
}

} // namespace

AnalyticBlendProof analytic_cylinder_blend(const TopoDS_Face &blend_face,
                                           const TopoDS_Face &support_a,
                                           const TopoDS_Face &support_b) {
  if (blend_face.IsNull())
    return AnalyticBlendProof::Unknown;
  try {
    BRepAdaptor_Surface blend(blend_face);
    // The load-bearing gate. A B-spline that samples as a perfect cylinder is
    // not a proven cylinder, so it never reaches the distance rules below.
    if (blend.GetType() != GeomAbs_Cylinder)
      return AnalyticBlendProof::Unknown;
    const gp_Cylinder cylinder = blend.Cylinder();
    const validation::GeometryPrecisionContext precision =
        validation::precision_of(blend_face);
    const AnalyticBlendProof a = against_support(
        cylinder.Axis(), cylinder.Radius(), support_a, precision);
    const AnalyticBlendProof b = against_support(
        cylinder.Axis(), cylinder.Radius(), support_b, precision);
    if (a == AnalyticBlendProof::Failed || b == AnalyticBlendProof::Failed)
      return AnalyticBlendProof::Failed;
    if (a == AnalyticBlendProof::Unknown || b == AnalyticBlendProof::Unknown)
      return AnalyticBlendProof::Unknown;
    return AnalyticBlendProof::Proved;
  } catch (const Standard_Failure &) {
    // Imported geometry can make an adaptor query throw. A query we could not
    // run is `Unknown` — never a proof, and never a measured mismatch.
    return AnalyticBlendProof::Unknown;
  }
}

BlendConvexity blend_convexity(const TopoDS_Face &blend_face) {
  if (blend_face.IsNull())
    return BlendConvexity::Unknown;
  try {
    BRepAdaptor_Surface surface(blend_face);
    if (surface.GetType() != GeomAbs_Cylinder)
      return BlendConvexity::Unknown;
    const double u0 = surface.FirstUParameter();
    const double u1 = surface.LastUParameter();
    const double v0 = surface.FirstVParameter();
    const double v1 = surface.LastVParameter();
    if (!std::isfinite(u0) || !std::isfinite(u1) || !std::isfinite(v0) ||
        !std::isfinite(v1))
      return BlendConvexity::Unknown;
    BRepLProp_SLProps properties(surface, 0.5 * (u0 + u1), 0.5 * (v0 + v1), 1,
                                 kNormalTolerance);
    if (!properties.IsNormalDefined())
      return BlendConvexity::Unknown;
    gp_Dir normal = properties.Normal();
    if (blend_face.Orientation() == TopAbs_REVERSED)
      normal.Reverse();
    const gp_Ax1 axis = surface.Cylinder().Axis();
    const gp_Vec direction(axis.Direction());
    const gp_Vec to_point(axis.Location(), properties.Value());
    const gp_Vec radial = to_point - direction * to_point.Dot(direction);
    if (radial.Magnitude() <= gp::Resolution())
      return BlendConvexity::Unknown;
    return radial.Dot(gp_Vec(normal)) > 0.0 ? BlendConvexity::Convex
                                            : BlendConvexity::Concave;
  } catch (const Standard_Failure &) {
    return BlendConvexity::Unknown;
  }
}

} // namespace onecad::kernel::fillet
