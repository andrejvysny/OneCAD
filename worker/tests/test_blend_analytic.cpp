// test_blend_analytic.cpp — the analytic tangency certificate and the targeted
// recognizer entry point.
//
// Two cases are load-bearing, and both say the same thing from opposite ends:
//   * `bspline_decoy_is_refused_although_sampling_accepts` measures ONE surface
//     twice — the production sampler reports a perfectly constant 2 mm section
//     radius; the analytic gate returns `Unknown`.
//   * `a_bspline_body_is_recognized_by_sampling_and_refused_analytically` runs
//     the FULL recognizer over a converted body with real shared boundaries.
//     It comes back "constant-radius two-support blend"; the analytic gate still
//     returns `Unknown`.
// That contrast is the reason the gate exists — every other check here would
// also pass with sampling alone.
//
// `cylindrical_support_from_real_geometry_is_proved` is the numerical
// regression: it prints the axis separation measured both ways, and the
// line-to-line form is wrong by millimetres on a genuine blend.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_CurveType.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_List.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <GeomConvert.hxx>
#include <Geom_BSplineSurface.hxx>
#include <Geom_RectangularTrimmedSurface.hxx>
#include <Geom_Surface.hxx>
#include <Precision.hxx>
#include <ShapeCustom.hxx>
#include <ShapeCustom_RestrictionParameters.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Ax3.hxx>
#include <gp_Cylinder.hxx>
#include <gp_Trsf.hxx>
#include <gp_Dir.hxx>
#include <gp_Lin.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Sphere.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/AnalyticBlend.h"
#include "kernel/fillet/BlendRecognizer.h"

namespace ft = onecad::tests::fillet;
namespace kf = onecad::kernel::fillet;

namespace {
using EdgeFaces = NCollection_IndexedDataMap<
    TopoDS_Shape, NCollection_List<TopoDS_Shape>, TopTools_ShapeMapHasher>;

constexpr double kPi = 3.14159265358979323846;
int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

std::vector<TopoDS_Face> cylindrical_faces(const TopoDS_Shape &shape) {
  TopTools_IndexedMapOfShape map;
  TopExp::MapShapes(shape, TopAbs_FACE, map);
  std::vector<TopoDS_Face> out;
  for (int i = 1; i <= map.Extent(); ++i) {
    const TopoDS_Face face = TopoDS::Face(map(i));
    if (BRepAdaptor_Surface(face).GetType() == GeomAbs_Cylinder)
      out.push_back(face);
  }
  return out;
}

TopoDS_Face plane_face(const gp_Pnt &origin, const gp_Dir &normal) {
  return BRepBuilderAPI_MakeFace(gp_Pln(origin, normal), -50.0, 50.0, -50.0,
                                 50.0)
      .Face();
}

TopoDS_Face cylinder_face(const gp_Pnt &origin, const gp_Dir &axis,
                          double radius) {
  return BRepBuilderAPI_MakeFace(gp_Cylinder(gp_Ax3(origin, axis), radius), 0.0,
                                 0.5 * kPi, -10.0, 10.0)
      .Face();
}

// The blend under test in every synthetic case: radius 2, axis = world Z
// through the origin.
TopoDS_Face synthetic_blend() {
  return cylinder_face(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0), 2.0);
}

// A 10 mm box with one vertical edge rounded at R2 — the canonical blend.
TopoDS_Shape filleted_box() {
  const TopoDS_Shape sharp = ft::box();
  BRepFilletAPI_MakeFillet fillet(sharp);
  fillet.Add(2.0, ft::vertical_edges(sharp).front());
  fillet.Build();
  if (!fillet.IsDone())
    return TopoDS_Shape();
  return fillet.Shape();
}

void real_blend_face_is_proved() {
  const TopoDS_Shape body = filleted_box();
  check(!body.IsNull(), "filleted box: fixture builds");
  if (body.IsNull())
    return;
  const std::vector<TopoDS_Face> cylinders = cylindrical_faces(body);
  check(cylinders.size() == 1, "filleted box: exactly one cylindrical face");
  if (cylinders.size() != 1)
    return;

  const kf::TargetedBlendRecognition at =
      kf::recognize_blend_at(cylinders.front(), body);
  check(at.recognized, "targeted: the blend face is recognized");
  check(at.proof == kf::AnalyticBlendProof::Proved,
        "targeted: the blend face is analytically PROVED");
  check(std::abs(at.recognition.radius - 2.0) <= 2.0e-6,
        "targeted: measured radius is 2 mm");
  check(at.convexity == kf::BlendConvexity::Convex,
        "targeted: an outside round on a box is convex");
  check(at.support_faces.size() == 2 &&
            at.recognition.support_face_ordinals.size() == 2,
        "targeted: exactly two supports, reported as faces and as ordinals");
  if (at.support_faces.size() != 2)
    return;

  check(kf::analytic_cylinder_blend(cylinders.front(), at.support_faces[0],
                                    at.support_faces[1]) ==
            kf::AnalyticBlendProof::Proved,
        "analytic: a real R2 fillet against its two planar supports is proved");

  // The targeted entry point and the whole-body sweep run the same
  // classification; a divergence here means the shared path forked.
  const std::vector<kf::BlendRecognition> sweep =
      kf::recognize_constant_radius_blends(body).recognized();
  check(sweep.size() == 1 &&
            sweep.front().face_ordinal == at.recognition.face_ordinal &&
            sweep.front().radius == at.recognition.radius &&
            sweep.front().support_face_ordinals ==
                at.recognition.support_face_ordinals,
        "targeted: identical verdict to the whole-body sweep");
}

void concave_blend_is_proved_and_reported_concave() {
  const TopoDS_Shape big = BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape();
  const TopoDS_Shape notch =
      BRepPrimAPI_MakeBox(gp_Pnt(10.0, 10.0, -1.0), 11.0, 11.0, 12.0).Shape();
  const TopoDS_Shape l_prism = BRepAlgoAPI_Cut(big, notch).Shape();
  check(!l_prism.IsNull(), "L prism: fixture builds");
  if (l_prism.IsNull())
    return;

  TopoDS_Edge concave;
  for (const TopoDS_Edge &edge : ft::vertical_edges(l_prism)) {
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    const gp_Pnt point = BRep_Tool::Pnt(first);
    if (std::abs(point.X() - 10.0) <= 1e-9 && std::abs(point.Y() - 10.0) <= 1e-9)
      concave = edge;
  }
  check(!concave.IsNull(), "L prism: the concave vertical edge is located");
  if (concave.IsNull())
    return;

  BRepFilletAPI_MakeFillet fillet(l_prism);
  fillet.Add(2.0, concave);
  fillet.Build();
  check(fillet.IsDone(), "L prism: the concave R2 fillet builds");
  if (!fillet.IsDone())
    return;

  const std::vector<TopoDS_Face> cylinders = cylindrical_faces(fillet.Shape());
  check(cylinders.size() == 1, "L prism: exactly one cylindrical face");
  if (cylinders.size() != 1)
    return;

  const kf::TargetedBlendRecognition at =
      kf::recognize_blend_at(cylinders.front(), fillet.Shape());
  check(at.recognized, "concave blend: recognized");
  check(at.proof == kf::AnalyticBlendProof::Proved,
        "concave blend: analytically proved — the rule is sign-agnostic");
  check(std::abs(at.recognition.radius - 2.0) <= 2.0e-6,
        "concave blend: measured radius is 2 mm");
  check(at.convexity == kf::BlendConvexity::Concave,
        "concave blend: reported concave");
}

void wrong_axis_distance_to_a_plane_fails() {
  const TopoDS_Face blend = synthetic_blend();
  const gp_Dir x(1.0, 0.0, 0.0);

  // Tangent: the plane sits exactly R away from the axis.
  const TopoDS_Face tangent = plane_face(gp_Pnt(2.0, 0.0, 0.0), x);
  check(kf::analytic_cylinder_blend(blend, tangent, tangent) ==
            kf::AnalyticBlendProof::Proved,
        "plane rule: axis-to-plane distance == R is proved");

  // 1 mm too far — the case the whole gate exists for.
  const TopoDS_Face too_far = plane_face(gp_Pnt(3.0, 0.0, 0.0), x);
  check(kf::analytic_cylinder_blend(blend, too_far, too_far) ==
            kf::AnalyticBlendProof::Failed,
        "plane rule: a cylinder at the wrong axis distance FAILS");

  // The tolerance in use is the precision context's semantic length (1e-6 mm
  // floor), pinned from both sides.
  check(kf::analytic_cylinder_blend(blend,
                                    plane_face(gp_Pnt(2.0 + 1.0e-8, 0.0, 0.0), x),
                                    tangent) == kf::AnalyticBlendProof::Proved,
        "plane rule: 1e-8 mm off is inside the semantic length budget");
  check(kf::analytic_cylinder_blend(blend,
                                    plane_face(gp_Pnt(2.0 + 1.0e-5, 0.0, 0.0), x),
                                    tangent) == kf::AnalyticBlendProof::Failed,
        "plane rule: 1e-5 mm off is outside it");

  // Right distance, wrong orientation: the axis is not parallel to the plane.
  check(kf::analytic_cylinder_blend(
            blend, plane_face(gp_Pnt(0.0, 0.0, 2.0), gp_Dir(0.0, 0.0, 1.0)),
            tangent) == kf::AnalyticBlendProof::Failed,
        "plane rule: an axis normal to the plane FAILS despite distance == R");
}

void cylindrical_support_junctions() {
  const TopoDS_Face blend = synthetic_blend();
  const gp_Dir z(0.0, 0.0, 1.0);

  const TopoDS_Face external = cylinder_face(gp_Pnt(7.0, 0.0, 0.0), z, 5.0);
  check(kf::analytic_cylinder_blend(blend, external, external) ==
            kf::AnalyticBlendProof::Proved,
        "cylinder rule: axis distance R_s + R is proved");

  const TopoDS_Face internal = cylinder_face(gp_Pnt(3.0, 0.0, 0.0), z, 5.0);
  check(kf::analytic_cylinder_blend(blend, internal, internal) ==
            kf::AnalyticBlendProof::Proved,
        "cylinder rule: axis distance |R_s - R| is proved");

  const TopoDS_Face off = cylinder_face(gp_Pnt(6.0, 0.0, 0.0), z, 5.0);
  check(kf::analytic_cylinder_blend(blend, off, off) ==
            kf::AnalyticBlendProof::Failed,
        "cylinder rule: any other axis distance FAILS");

  const TopoDS_Face skew =
      cylinder_face(gp_Pnt(7.0, 0.0, 0.0), gp_Dir(1.0, 0.0, 0.0), 5.0);
  check(kf::analytic_cylinder_blend(blend, skew, skew) ==
            kf::AnalyticBlendProof::Failed,
        "cylinder rule: non-parallel axes FAIL");
}

// REGRESSION (line-line distance): every synthetic case above shares one literal
// gp_Dir, so its axes are bit-identical and any distance formula takes a
// well-conditioned branch. This fixture takes its support cylinder from a real
// OCCT cut + fillet on a shape rotated off every axis, so the blend axis and the
// support axis are separately computed doubles. `gp_Lin::Distance(const gp_Lin&)`
// treats such a pair as SKEW — its parallel branch needs gp::Resolution() — and
// returns a rounding-noise projection instead of the axis separation.
void cylindrical_support_from_real_geometry_is_proved() {
  const TopoDS_Shape block = BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape();
  const TopoDS_Shape drill =
      BRepPrimAPI_MakeCylinder(
          gp_Ax2(gp_Pnt(10.0, 0.0, -1.0), gp_Dir(0.0, 0.0, 1.0)), 5.0, 12.0)
          .Shape();
  gp_Trsf rotation;
  rotation.SetRotation(gp_Ax1(gp_Pnt(3.0, 4.0, 5.0), gp_Dir(1.0, 2.0, 3.0)), 0.6);
  const TopoDS_Shape grooved =
      BRepBuilderAPI_Transform(BRepAlgoAPI_Cut(block, drill).Shape(), rotation,
                               Standard_True)
          .Shape();
  check(!grooved.IsNull(), "groove: fixture builds");
  if (grooved.IsNull())
    return;

  // The junction between the groove's cylindrical wall and the flat face it cuts
  // into: a straight edge with one planar and one cylindrical neighbour.
  EdgeFaces edge_faces;
  TopExp::MapShapesAndAncestors(grooved, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  TopoDS_Edge junction;
  for (int i = 1; i <= edge_faces.Extent(); ++i) {
    const auto &adjacent = edge_faces.FindFromIndex(i);
    if (adjacent.Extent() != 2)
      continue;
    const TopoDS_Edge edge = TopoDS::Edge(edge_faces.FindKey(i));
    if (BRepAdaptor_Curve(edge).GetType() != GeomAbs_Line)
      continue;
    const GeomAbs_SurfaceType first =
        BRepAdaptor_Surface(TopoDS::Face(adjacent.First())).GetType();
    const GeomAbs_SurfaceType last =
        BRepAdaptor_Surface(TopoDS::Face(adjacent.Last())).GetType();
    if ((first == GeomAbs_Plane && last == GeomAbs_Cylinder) ||
        (first == GeomAbs_Cylinder && last == GeomAbs_Plane))
      junction = edge;
  }
  check(!junction.IsNull(), "groove: the plane/cylinder junction edge is found");
  if (junction.IsNull())
    return;

  BRepFilletAPI_MakeFillet fillet(grooved);
  fillet.Add(2.0, junction);
  fillet.Build();
  check(fillet.IsDone(), "groove: the R2 junction fillet builds");
  if (!fillet.IsDone())
    return;

  TopoDS_Face blend;
  for (const TopoDS_Face &face : cylindrical_faces(fillet.Shape())) {
    if (std::abs(BRepAdaptor_Surface(face).Cylinder().Radius() - 2.0) <= 1.0e-6)
      blend = face;
  }
  check(!blend.IsNull(), "groove: the R2 cylindrical blend face is found");
  if (blend.IsNull())
    return;

  const kf::TargetedBlendRecognition at =
      kf::recognize_blend_at(blend, fillet.Shape());
  check(at.support_faces.size() == 2, "groove: two G1 supports");
  if (at.support_faces.size() != 2)
    return;

  // The condition that makes the line-line form wrong: parallel to within
  // semantic_angular, but NOT bit-identical, so a cross product of the two
  // directions is pure noise.
  const gp_Ax1 blend_axis = BRepAdaptor_Surface(blend).Cylinder().Axis();
  for (const TopoDS_Face &support : at.support_faces) {
    BRepAdaptor_Surface surface(support);
    if (surface.GetType() != GeomAbs_Cylinder)
      continue;
    const gp_Ax1 support_axis = surface.Cylinder().Axis();
    const double angle =
        blend_axis.Direction().Angle(support_axis.Direction());
    std::fprintf(stderr,
                 "groove: axis angle=%.3e point-to-line=%.12f line-to-line=%.12f\n",
                 std::min(angle, kPi - angle),
                 gp_Lin(blend_axis).Distance(support_axis.Location()),
                 gp_Lin(blend_axis).Distance(gp_Lin(support_axis)));
  }

  check(at.recognized, "groove: the blend is recognized");
  check(at.proof == kf::AnalyticBlendProof::Proved,
        "groove: a real cylinder-supported blend is tangency-proved");
}

// FIX 2 decoy. A face is not a blend of itself, and the co-surface pair is
// reachable: a drilled hole's cylindrical wall split by its seam makes each half
// a G1 neighbour of the other on the SAME surface. Accepting that as a blend
// would authorize suppressing the hole.
void a_co_surface_support_is_refused() {
  const gp_Cylinder wall(gp_Ax3(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0)),
                         3.0);
  const TopoDS_Face lower =
      BRepBuilderAPI_MakeFace(wall, 0.0, kPi, -10.0, 10.0).Face();
  const TopoDS_Face upper =
      BRepBuilderAPI_MakeFace(wall, kPi, 2.0 * kPi, -10.0, 10.0).Face();
  check(kf::analytic_cylinder_blend(lower, upper, upper) ==
            kf::AnalyticBlendProof::Failed,
        "co-surface: the other half of the same cylinder is not a support");
  check(kf::analytic_cylinder_blend(lower, lower, lower) ==
            kf::AnalyticBlendProof::Failed,
        "co-surface: a face never certifies against itself");

  // The same refusal on a hole that a real boolean produced.
  const TopoDS_Shape block = BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape();
  const TopoDS_Shape drill =
      BRepPrimAPI_MakeCylinder(
          gp_Ax2(gp_Pnt(10.0, 10.0, -1.0), gp_Dir(0.0, 0.0, 1.0)), 3.0, 12.0)
          .Shape();
  const std::vector<TopoDS_Face> holes =
      cylindrical_faces(BRepAlgoAPI_Cut(block, drill).Shape());
  check(!holes.empty(), "co-surface: the drilled hole has a cylindrical wall");
  for (const TopoDS_Face &hole : holes) {
    check(kf::analytic_cylinder_blend(hole, hole, hole) ==
              kf::AnalyticBlendProof::Failed,
          "co-surface: a real drilled hole wall is not its own blend");
  }
}

void unsupported_geometry_is_unknown_and_failure_dominates() {
  const TopoDS_Face blend = synthetic_blend();
  const TopoDS_Face tangent =
      plane_face(gp_Pnt(2.0, 0.0, 0.0), gp_Dir(1.0, 0.0, 0.0));
  const TopoDS_Face sphere =
      BRepBuilderAPI_MakeFace(gp_Sphere(gp_Ax3(gp_Pnt(0.0, 0.0, 0.0),
                                               gp_Dir(0.0, 0.0, 1.0)),
                                        4.0),
                              0.0, kPi, -0.5 * kPi, 0.5 * kPi)
          .Face();

  check(kf::analytic_cylinder_blend(blend, tangent, sphere) ==
            kf::AnalyticBlendProof::Unknown,
        "support rule: a support that is neither plane nor cylinder is Unknown");
  check(kf::analytic_cylinder_blend(
            blend, plane_face(gp_Pnt(3.0, 0.0, 0.0), gp_Dir(1.0, 0.0, 0.0)),
            sphere) == kf::AnalyticBlendProof::Failed,
        "support rule: a measured mismatch dominates an unmeasurable support");
  check(kf::analytic_cylinder_blend(TopoDS_Face(), tangent, tangent) ==
            kf::AnalyticBlendProof::Unknown,
        "support rule: a null blend face is Unknown");
}

void bspline_decoy_is_refused_although_sampling_accepts() {
  const TopoDS_Shape body = filleted_box();
  const std::vector<TopoDS_Face> cylinders = cylindrical_faces(body);
  if (cylinders.size() != 1)
    return;
  const TopoDS_Face blend = cylinders.front();
  const kf::TargetedBlendRecognition at = kf::recognize_blend_at(blend, body);
  if (at.support_faces.size() != 2)
    return;

  BRepAdaptor_Surface adaptor(blend);
  const gp_Cylinder cylinder = adaptor.Cylinder();
  const Handle(Geom_Surface) trimmed = new Geom_RectangularTrimmedSurface(
      BRep_Tool::Surface(blend), adaptor.FirstUParameter(),
      adaptor.LastUParameter(), adaptor.FirstVParameter(),
      adaptor.LastVParameter());
  // An EXACT conversion is deliberately the hardest decoy available: a sloppier
  // approximation would only be easier to reject. Nothing about the analytic
  // gate's answer depends on how good the approximation is.
  const Handle(Geom_BSplineSurface) spline =
      GeomConvert::SurfaceToBSplineSurface(trimmed);
  check(!spline.IsNull(), "decoy: the blend patch converts to a B-spline");
  if (spline.IsNull())
    return;
  const TopoDS_Face decoy =
      BRepBuilderAPI_MakeFace(spline, Precision::Confusion()).Face();
  check(!decoy.IsNull(), "decoy: the B-spline patch becomes a face");
  if (decoy.IsNull())
    return;
  check(BRepAdaptor_Surface(decoy).GetType() == GeomAbs_BSplineSurface,
        "decoy: the face really is a B-spline, not a cylinder");

  double u0 = 0.0, u1 = 0.0, v0 = 0.0, v1 = 0.0;
  spline->Bounds(u0, u1, v0, v1);
  double deviation = 0.0;
  for (int i = 0; i < 5; ++i) {
    for (int j = 0; j < 5; ++j) {
      const double u = u0 + (u1 - u0) * (i + 0.5) / 5.0;
      const double v = v0 + (v1 - v0) * (j + 0.5) / 5.0;
      deviation = std::max(
          deviation, std::abs(gp_Lin(cylinder.Axis()).Distance(spline->Value(u, v)) -
                              cylinder.Radius()));
    }
  }
  check(deviation <= 1.0e-7,
        "decoy: the B-spline reproduces the blend surface to 1e-7 mm");

  // THE LOAD-BEARING ASSERTION. The production sampler, at its default budget,
  // is handed the decoy and reports a perfectly constant 2 mm section radius.
  // Sampling WOULD have called this a blend. (No shared boundary edge exists on
  // a standalone face, so this measures curvature only — which is exactly the
  // evidence the recognizer's constant-radius verdict rests on.)
  const kf::BlendEvidence evidence =
      kf::measure_blend_evidence(decoy, {decoy}, {}, 2.0);
  const double spread =
      evidence.maximum_section_radius - evidence.minimum_section_radius;
  const double measured = 0.5 * (evidence.maximum_section_radius +
                                 evidence.minimum_section_radius);
  std::fprintf(stderr,
               "decoy: deviation=%.3e samples=%d radius=%.12f spread=%.3e\n",
               deviation, evidence.samples, measured, spread);
  check(evidence.samples == 25,
        "decoy: the default budget takes a 25-point sample grid");
  check(spread <= 1.0e-6,
        "decoy: sampled section radius is constant within 1e-6 mm");
  check(std::abs(measured - 2.0) <= 1.0e-6,
        "decoy: sampled section radius measures the blend's 2 mm");

  check(kf::analytic_cylinder_blend(decoy, at.support_faces[0],
                                    at.support_faces[1]) ==
            kf::AnalyticBlendProof::Unknown,
        "decoy: the analytic gate refuses it as Unknown, never Proved");
  check(kf::blend_convexity(decoy) == kf::BlendConvexity::Unknown,
        "decoy: convexity of a non-cylinder is Unknown");
}

// The decoy above proves the contrast at the sampler. This proves it through the
// WHOLE recognizer: the same body with only its cylindrical surface converted to
// a B-spline, boundaries and pcurves rebuilt by OCCT, so `classify_face` runs on
// real shared edges. The recognizer still says "constant-radius two-support
// blend"; the analytic gate still says `Unknown`.
void a_bspline_body_is_recognized_by_sampling_and_refused_analytically() {
  const TopoDS_Shape body = filleted_box();
  if (body.IsNull())
    return;
  Handle(ShapeCustom_RestrictionParameters) parameters =
      new ShapeCustom_RestrictionParameters();
  parameters->ConvertCylindricalSurf() = true;
  parameters->ConvertPlane() = false;
  const TopoDS_Shape converted = ShapeCustom::BSplineRestriction(
      body, 1.0e-7, 1.0e-7, 9, 1000, GeomAbs_C2, GeomAbs_C2, true, false,
      parameters);
  check(!converted.IsNull(), "b-spline body: the conversion produces a shape");
  if (converted.IsNull())
    return;

  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(converted, TopAbs_FACE, faces);
  TopoDS_Face spline_face;
  for (int i = 1; i <= faces.Extent(); ++i) {
    const TopoDS_Face face = TopoDS::Face(faces(i));
    if (BRepAdaptor_Surface(face).GetType() == GeomAbs_BSplineSurface)
      spline_face = face;
  }
  check(!spline_face.IsNull(),
        "b-spline body: the blend face converted to a B-spline");
  if (spline_face.IsNull())
    return;

  const kf::TargetedBlendRecognition at =
      kf::recognize_blend_at(spline_face, converted);
  std::fprintf(stderr,
               "bspline body: recognized=%d radius=%.12f spread=%.3e "
               "tangency=%.3e samples=%d boundaries=%d reason=%s\n",
               at.recognized ? 1 : 0, at.recognition.radius,
               at.recognition.evidence.maximum_section_radius -
                   at.recognition.evidence.minimum_section_radius,
               at.recognition.evidence.maximum_tangency_radians,
               at.recognition.evidence.samples,
               at.recognition.evidence.boundaries,
               at.recognition.reason.c_str());
  check(at.recognized,
        "b-spline body: the sampled recognizer STILL accepts it as a blend");
  check(std::abs(at.recognition.radius - 2.0) <= 2.0e-6,
        "b-spline body: sampling measures the 2 mm radius");
  check(at.proof == kf::AnalyticBlendProof::Unknown,
        "b-spline body: the analytic gate refuses the same face");
}

void a_foreign_face_is_refused_with_a_reason() {
  const TopoDS_Shape body = filleted_box();
  TopTools_IndexedMapOfShape other;
  TopExp::MapShapes(ft::box(3.0), TopAbs_FACE, other);
  const kf::TargetedBlendRecognition at =
      kf::recognize_blend_at(TopoDS::Face(other(1)), body);
  check(!at.recognized && at.proof == kf::AnalyticBlendProof::Unknown,
        "targeted: a face from another body is not recognized");
  check(at.recognition.reason == "face does not belong to the body",
        "targeted: the refusal names its reason");
}

} // namespace

int main() {
  real_blend_face_is_proved();
  concave_blend_is_proved_and_reported_concave();
  wrong_axis_distance_to_a_plane_fails();
  cylindrical_support_junctions();
  cylindrical_support_from_real_geometry_is_proved();
  a_co_surface_support_is_refused();
  unsupported_geometry_is_unknown_and_failure_dominates();
  bspline_decoy_is_refused_although_sampling_accepts();
  a_bspline_body_is_recognized_by_sampling_and_refused_analytically();
  a_foreign_face_is_refused_with_a_reason();
  if (failures == 0)
    std::fprintf(stderr, "blend analytic: all checks passed\n");
  return failures;
}
