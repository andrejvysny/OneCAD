#include "kernel/fillet/BlendOverflow.h"

#include <algorithm>
#include <cmath>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepTools.hxx>
#include <BRep_Tool.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <GProp_GProps.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <Precision.hxx>
#include <Standard_Failure.hxx>
#include <TopLoc_Location.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_IndexedMap.hxx>
#include <NCollection_List.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>
#include <gp_XY.hxx>

#include "kernel/validation/GeometryPrecision.h"
#include "kernel/validation/ShapeAudit.h"

#include "kernel/fillet/BlendOverflowPlan.h"

namespace onecad::kernel::fillet {

using namespace overflow_detail;

const char *overflow_kind_name(OverflowKind kind) {
  return kind == OverflowKind::Cliff ? "cliff" : "extendedPlane";
}

namespace {

using ShapeMap = NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher>;
bool stop_if_cancelled(OverflowResult &r, const onecad::CancelToken *cancel) {
  if (cancel == nullptr || !cancel->cancelled())
    return false;
  r.cancelled = true;
  fail(r, "OVERFLOW_CANCELLED", "cancelled");
  return true;
}

double surface_area(const TopoDS_Shape &s) {
  GProp_GProps g;
  BRepGProp::SurfaceProperties(s, g);
  return g.Mass();
}

double volume_of(const TopoDS_Shape &s) {
  GProp_GProps g;
  BRepGProp::VolumeProperties(s, g);
  return g.Mass();
}


// Prism the profile along the seed. `Generated(edge)` names the blend face
// without any geometric search.
bool make_tool(const Frame &f, const Plan &p, OverflowResult &r) {
  gp_Trsf tr;
  tr.SetTranslation(f.e * -p.pad);
  const TopoDS_Face moved = TopoDS::Face(p.profile.Moved(TopLoc_Location(tr)));
  const gp_Pnt want = p.blend_mid.Translated(f.e * -p.pad);
  TopoDS_Edge blend_edge;
  double best = 1.0e300;
  for (TopExp_Explorer ex(moved, TopAbs_EDGE); ex.More(); ex.Next()) {
    BRepAdaptor_Curve c(TopoDS::Edge(ex.Current()));
    const double d = c.Value(0.5 * (c.FirstParameter() + c.LastParameter())).Distance(want);
    if (d < best) {
      best = d;
      blend_edge = TopoDS::Edge(ex.Current());
    }
  }
  BRepPrimAPI_MakePrism prism(moved, f.e * (f.length + 2.0 * p.pad));
  if (blend_edge.IsNull() || !prism.IsDone() || prism.Generated(blend_edge).IsEmpty())
    return fail(r, "OVERFLOW_BOOLEAN_FAILED", "tool prism failed");
  r.tool = prism.Shape();
  if (volume_of(r.tool) < 0.0)
    r.tool.Reverse(); // wire winding is data-dependent; the Cut needs a positive solid
  r.tool_blend_face = TopoDS::Face(prism.Generated(blend_edge).First());
  return true;
}

std::vector<TopoDS_Face> images_in_result(BRepAlgoAPI_Cut &cut, const TopoDS_Shape &face,
                                          const ShapeMap &faces) {
  std::vector<TopoDS_Face> out;
  if (faces.Contains(face))
    out.push_back(TopoDS::Face(face));
  for (const TopoDS_Shape &m : cut.Modified(face))
    if (faces.Contains(m))
      out.push_back(TopoDS::Face(m));
  return out;
}

// The Cut itself is NOT interruptible: cancel is best-effort, polled on both sides
// of it. The Cut measures in milliseconds on the supported envelope.
bool run_cut(const OverflowRequest &q, OverflowResult &r, const onecad::CancelToken *cancel) {
  if (stop_if_cancelled(r, cancel))
    return false;
  auto cut = std::make_shared<BRepAlgoAPI_Cut>();
  NCollection_List<TopoDS_Shape> args, tools;
  args.Append(q.body);
  tools.Append(r.tool);
  cut->SetArguments(args);
  cut->SetTools(tools);
  cut->SetNonDestructive(true);
  cut->SetRunParallel(false);
  cut->Build();
  if (stop_if_cancelled(r, cancel))
    return false;
  if (!cut->IsDone() || cut->HasErrors() || cut->Shape().IsNull())
    return fail(r, "OVERFLOW_BOOLEAN_FAILED", "tool Cut failed");
  r.cut = cut;
  r.shape = cut->Shape();
  return true;
}

// Certification a/b: history says the tool only made the blend face and only
// consumed / trimmed the faces the construction predicts.
bool check_history(const Frame &f, const Plan &p, OverflowResult &r) {
  ShapeMap faces, tool_faces;
  TopExp::MapShapes(r.shape, TopAbs_FACE, faces);
  TopExp::MapShapes(r.tool, TopAbs_FACE, tool_faces);
  auto touched = [&](const std::string &why) {
    return fail(r, "OVERFLOW_TOOL_TOUCHED_BODY", why);
  };
  const auto blend = images_in_result(*r.cut, r.tool_blend_face, faces);
  if (blend.size() != 1)
    return touched("blend face has " + std::to_string(blend.size()) + " images");
  r.blend_face = blend.front();
  for (int i = 1; i <= tool_faces.Extent(); ++i) {
    if (tool_faces(i).IsSame(r.tool_blend_face))
      continue;
    if (!r.cut->IsDeleted(tool_faces(i)) ||
        !images_in_result(*r.cut, tool_faces(i), faces).empty())
      return touched("a non-blend tool face survives in the result");
  }
  const Support &c = f.s[p.consumed], &sj = f.s[1 - p.consumed];
  r.consumed_face = c.face;
  if (!r.cut->IsDeleted(c.face) || faces.Contains(c.face))
    return touched("consumed face survives");
  const auto surv = images_in_result(*r.cut, sj.face, faces);
  if (surv.empty())
    return touched("surviving support has no image");
  r.surviving_support = surv.front();
  const auto cl = images_in_result(*r.cut, r.cliff_face, faces);
  r.cliff_face = cl.empty() ? TopoDS_Face() : cl.front();
  return true;
}

bool check_volume(const OverflowRequest &q, const Frame &f, const Plan &p, OverflowResult &r) {
  const double v0 = volume_of(q.body);
  r.measured_removed_volume = v0 - volume_of(r.shape);
  r.analytic_removed_volume = p.expected_area * f.length;
  // A difference of two GProp volumes carries ~eps*V0 numeric error, so the bound
  // must not scale with V0 alone; the analytic term absorbs quadrature error on
  // the cylinder. Locality (the tool touched nothing else) is NOT proved by this
  // number: the structural provenance checks (a, b) carry that.
  const double tol = 1.0e-9 + 1.0e-9 * v0 + 1.0e-6 * r.analytic_removed_volume;
  if (std::abs(r.measured_removed_volume - r.analytic_removed_volume) > tol)
    return fail(r, "OVERFLOW_VOLUME_MISMATCH",
                "measured removed volume " + std::to_string(r.measured_removed_volume) +
                    " vs analytic " + std::to_string(r.analytic_removed_volume));
  return true;
}

bool check_blend_surface(const OverflowRequest &q, const Frame &f, const Plan &p,
                         OverflowResult &r) {
  BRepAdaptor_Surface s(r.blend_face);
  if (q.mode == EdgeOpMode::Chamfer) {
    if (s.GetType() != GeomAbs_Plane)
      return fail(r, "OVERFLOW_TOOL_TOUCHED_BODY", "chamfer blend face is not planar");
    // Planar is not enough: it must be THE plane through the two setback points.
    const gp_Pln pl = s.Plane();
    const gp_Vec nrm(pl.Axis().Direction());
    if (pl.Distance(p.line_a) > f.res * 1.0e-3 || pl.Distance(p.line_b) > f.res * 1.0e-3 ||
        std::abs(nrm * f.e) > kParallel)
      return fail(r, "OVERFLOW_TOOL_TOUCHED_BODY",
                  "chamfer plane misses its setback points or is not parallel to the seed");
    return true;
  }
  if (s.GetType() != GeomAbs_Cylinder ||
      std::abs(s.Cylinder().Radius() - q.value) > 1.0e-9 * std::max(1.0, q.value) ||
      std::abs(s.Cylinder().Axis().Direction().XYZ() * f.e.XYZ()) < 1.0 - kParallel)
    return fail(r, "OVERFLOW_TOOL_TOUCHED_BODY", "fillet blend face is not the expected cylinder");
  return true;
}

bool check_publication(const OverflowRequest &q, OverflowResult &r,
                       const onecad::CancelToken *cancel) {
  using namespace validation;
  const ShapeEvidence input = collect_shape_evidence(q.body);
  if (stop_if_cancelled(r, cancel))
    return false;
  const ShapeEvidence out = collect_shape_evidence(r.shape, PublicationTier::TierB, cancel);
  // A cancel inside the audit yields truncated evidence; never report it as a refusal.
  if (stop_if_cancelled(r, cancel))
    return false;
  PublicationPolicy policy = single_solid_policy(
      q.mode == EdgeOpMode::Chamfer ? "Chamfer overflow" : "Fillet overflow",
      PublicationTier::TierB);
  // Same budget as FilletBuilder::accept_result: grown from the INPUT tolerance.
  policy.maximum_tolerance =
      precision_of(q.body).tolerance_ceiling(input.tolerances.maximum(), 2.0, 1.0e-6);
  const PublicationDecision d = evaluate_publication_policy(out, policy);
  if (stop_if_cancelled(r, cancel))
    return false;
  if (!d.publishable())
    return fail(r, "OVERFLOW_PUBLICATION_REFUSED", d.message);
  return true;
}

// Faces the blend created or reshaped must not be slivers.
bool check_remnant(const OverflowRequest &q, const Frame &f, OverflowResult &r) {
  ShapeMap before, after;
  TopExp::MapShapes(q.body, TopAbs_FACE, before);
  TopExp::MapShapes(r.shape, TopAbs_FACE, after);
  for (int i = 1; i <= after.Extent(); ++i) {
    if (before.Contains(after(i)))
      continue;
    if (surface_area(after(i)) < f.res * f.res)
      return fail(r, "OVERFLOW_REMNANT", "result face below resolution area");
    for (TopExp_Explorer ex(after(i), TopAbs_EDGE); ex.More(); ex.Next()) {
      const TopoDS_Edge e = TopoDS::Edge(ex.Current());
      if (BRep_Tool::Degenerated(e))
        continue;
      GProp_GProps g;
      BRepGProp::LinearProperties(e, g);
      if (g.Mass() < f.res)
        return fail(r, "OVERFLOW_REMNANT", "result edge below resolution length");
    }
  }
  return true;
}

bool certify(const OverflowRequest &q, const Frame &f, const Plan &p, OverflowResult &r,
             const onecad::CancelToken *cancel) {
  return check_history(f, p, r) && check_volume(q, f, p, r) && check_blend_surface(q, f, p, r) &&
         check_publication(q, r, cancel) && check_remnant(q, f, r);
}

// The fallback must not launder an invalid body: the native Fillet refuses an
// input that fails the TierB single-solid audit, and so does this path.
bool check_input(const OverflowRequest &q, OverflowResult &r) {
  using namespace validation;
  const PublicationDecision d = evaluate_publication_policy(
      collect_shape_evidence(q.body),
      single_solid_policy("Overflow input", PublicationTier::TierB));
  if (!d.publishable())
    return fail(r, "OVERFLOW_INPUT_INVALID", "input body failed shape audit: " + d.message);
  return true;
}

bool build_checked(const OverflowRequest &q, OverflowResult &r,
                   const onecad::CancelToken *cancel) {
  if (!check_input(q, r))
    return false;
  Frame f;
  f.res = validation::precision_of(q.body).authoring_resolution();
  Plan p;
  if (!qualify(q, f, p, r) || !plan_blend(q, f, p, r) || !make_tool(f, p, r))
    return false;
  r.kind = p.kind;
  r.setback = p.setback;
  if (!run_cut(q, r, cancel) || !certify(q, f, p, r, cancel))
    return false;
  r.ok = true;
  return true;
}

} // namespace

OverflowResult build_overflow(const OverflowRequest &request,
                              const onecad::CancelToken *cancel) {
  OverflowResult r;
  r.kind = request.mode == EdgeOpMode::Chamfer ? OverflowKind::ExtendedPlane : OverflowKind::Cliff;
  if (request.mode != EdgeOpMode::Fillet && request.mode != EdgeOpMode::Chamfer) {
    fail(r, "OVERFLOW_UNSUPPORTED_MODE", "only fillet and chamfer overflow are supported");
    return r;
  }
  if (request.body.IsNull()) {
    fail(r, "OVERFLOW_NOT_ISOLATED", "body is null");
    return r;
  }
  try {
    if (!build_checked(request, r, cancel) && r.reason_code.empty())
      fail(r, "OVERFLOW_BOOLEAN_FAILED", "overflow build failed");
  } catch (const Standard_Failure &ex) {
    fail(r, "OVERFLOW_BOOLEAN_FAILED", std::string("OCCT failure: ") + ex.what());
  } catch (...) {
    fail(r, "OVERFLOW_BOOLEAN_FAILED", "unexpected failure");
  }
  if (!r.ok) {
    r.shape = TopoDS_Shape();
    r.cut.reset();
  }
  return r;
}

} // namespace onecad::kernel::fillet