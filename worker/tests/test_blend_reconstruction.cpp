// test_blend_reconstruction.cpp — WP3 C4.
//
// THE IDENTITY ROUND TRIP is the whole point of this file: suppress a recognized
// blend and rebuild it at the same radius with NOTHING in between, and the body has
// to come back. C5 inserts an offset between the two halves; if the round trip with
// no offset does not reproduce the body, every result C5 publishes is a
// coincidence. Asserted in both signs of convexity, to 1e-9 relative volume with an
// identical face count, plus every reproduction postcondition.
//
// The exact volumes are analytic, not recorded. A 90-degree corner of radius R over
// a run of length L encloses, between the sharp corner and the arc,
//
//     L * (R^2 - pi*R^2/4)   =   10 * (4 - pi)   =   8.58407346410207 mm^3
//
// for R = 2, L = 10. A CONVEX round on a 10^3 box removes that (1000 -> 991.4159...)
// so suppressing it must ADD it back; a CONCAVE fillet in the reentrant edge of a
// 640 mm^3 L-prism adds it (640 -> 648.5840...) so suppressing it must REMOVE it.
// Same magnitude, opposite sign — which is exactly what makes the volume-direction
// postcondition a real test rather than a restatement of "something changed".
//
// The refusal paths are as load-bearing as the round trip: a destructive engine that
// cannot say no is not safe, and each refusal here is asserted BY REASON CODE, never
// by "it failed".
//
// ── COVERAGE IS A DECISION, NOT AN ACCIDENT ─────────────────────────────────
// `BlendReconstruction` names 32 refusal codes. Coverage of them is stated here
// rather than left to be inferred from which assertions happen to exist.
//
// A PERMANENT FAILING PATH (13): BLEND_NOT_RECOGNIZED · SUPPRESSION_INPUT_INVALID ·
// SUPPRESSION_NOT_DONE · SUPPRESSION_FACE_COUNT · SUPPRESSION_ANCESTRY_NOT_UNIQUE ·
// SUPPRESSION_SEED_EDGE_NOT_FOUND · SUPPRESSION_VOLUME_DIRECTION ·
// REBLEND_INPUT_INVALID · REBLEND_FAILED · REBLEND_FACE_NOT_UNIQUE ·
// REBLEND_NOT_CYLINDER · REBLEND_SUPPORTS_MISMATCH · REBLEND_CONVEXITY.
//
// PROVEN LIVE BY NEUTRALIZATION ONLY (2). Both are structurally shadowed: an earlier
// rung refuses first for every input this corpus can forge, so they were exercised by
// disabling that rung, recording the result and restoring it.
//   REBLEND_BOUNDARY_LENGTH — reached with layer 3's seed-length match disabled;
//     catches the seed that binds to a 10 mm edge for a 6 mm blend.
//   REBLEND_NOT_PROVED — reached with REBLEND_SUPPORTS_MISMATCH disabled; catches a
//     support the recognizer agrees with but analytic tangency does not.
//
// DECLARED UNTESTED (17). Each needs geometry no honest fixture in this corpus
// produces; fabricating one to colour a line green would be worse than saying so.
//   BLEND_NOT_PROVED               needs a face that samples as a blend and is not an
//                                  exact cylinder. C1's test_blend_analytic.cpp covers
//                                  the underlying proof; the certify-level wrapper is
//                                  uncovered.
//   BLEND_CONVEXITY_UNKNOWN        structurally unreachable: `Proved` implies an exact
//                                  cylinder, which always has a radial normal.
//   BLEND_EVIDENCE_INSUFFICIENT    needs a trimmed domain too small for 64 samples.
//   BLEND_BOUNDARY_NOT_SIMPLE      needs a support touching a blend along two edges.
//   BLEND_BOUNDARY_NOT_LINEAR      needs a curved blend boundary, which fails the
//                                  analytic proof one rung earlier.
//   BLEND_BOUNDARY_LENGTH_MISMATCH needs an asymmetrically trimmed round.
//   SUPPRESSION_NOT_PUBLISHABLE    needs a defeature that succeeds into an invalid or
//                                  multi-solid body.
//   SUPPRESSION_TOLERANCE_CAP      needs a stage that adds > 1e-6 mm of tolerance.
//   SUPPRESSION_BLEND_SURVIVED     needs a defeature that reports done, keeps the face
//                                  count, and leaves the blend present.
//   SUPPRESSION_SUPPORT_SURFACE_DRIFT  needs a support whose surface MOVES, or one that
//                                  is neither plane nor cylinder. No fixture here has
//                                  either.
//   SUPPRESSION_SEED_EDGE_AMBIGUOUS  needs two same-length new edges between one
//                                  support pair, or two blends resolving to one seed.
//                                  Not constructible with cylinder-only blends.
//   REBLEND_NOT_GENERATED          shadowed: OCCT refuses a foreign contour, so
//                                  REBLEND_FAILED fires before this rung.
//   REBLEND_RADIUS                 unreachable: a clean fillet always builds the radius
//                                  it was asked for.
//   REBLEND_NOT_RECOGNIZED         needs a rebuilt face the recognizer rejects.
//   REBLEND_BOUNDARY_NOT_SIMPLE    as BLEND_BOUNDARY_NOT_SIMPLE, on the rebuilt face.
//   REBLEND_EVIDENCE               needs a rebuilt blend that is thin or out of profile.
//   REBLEND_TOLERANCE_CAP          as SUPPRESSION_TOLERANCE_CAP.
// These belong in C6's adversarial corpus, where the decoy fixtures live.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRep_Tool.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/BlendReconstruction.h"

namespace ft = onecad::tests::fillet;
namespace kf = onecad::kernel::fillet;

namespace {

// Harness contract: the process exit code IS the failure count, so `check` is the
// only thing that may report one and it always counts. `checks_run` is printed so a
// section that silently skipped (an early return, a fixture that did not build) is
// visible as a dropped count rather than a vacuous green.
int failures = 0;
int checks_run = 0;

void check(bool condition, const std::string &message) {
  ++checks_run;
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

double volume(const TopoDS_Shape &shape) {
  GProp_GProps properties;
  BRepGProp::VolumeProperties(shape, properties);
  return properties.Mass();
}

int face_count(const TopoDS_Shape &shape) {
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(shape, TopAbs_FACE, faces);
  return faces.Extent();
}

bool near_relative(double measured, double expected, double relative) {
  return std::abs(measured - expected) <= std::abs(expected) * relative;
}

bool starts_with(const std::string &value, const std::string &prefix) {
  return value.rfind(prefix, 0) == 0;
}

// The exact corner volume: 10 mm of run, 90 degrees, R2 and R3.
const double kCornerVolume = 10.0 * (4.0 - 3.14159265358979323846);
const double kCornerVolumeR3 = 10.0 * (9.0 - 9.0 * 3.14159265358979323846 / 4.0);

// The CHARACTERIZED two-stage B-Rep tolerance, measured on OCCT 8.0.1. See
// `kRoundTripToleranceCapMm`, which is the policy ceiling above it.
const double kMeasuredRoundTrip = 1.5e-7;

// --- fixtures ---------------------------------------------------------------

TopoDS_Edge vertical_edge_at(const TopoDS_Shape &shape, double x, double y) {
  for (const TopoDS_Edge &edge : ft::vertical_edges(shape)) {
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    const gp_Pnt point = BRep_Tool::Pnt(first);
    if (std::abs(point.X() - x) <= 1e-9 && std::abs(point.Y() - y) <= 1e-9)
      return edge;
  }
  return {};
}

TopoDS_Face only_cylindrical_face(const TopoDS_Shape &shape) {
  TopoDS_Face found;
  int seen = 0;
  for (TopExp_Explorer explorer(shape, TopAbs_FACE); explorer.More(); explorer.Next()) {
    const TopoDS_Face face = TopoDS::Face(explorer.Current());
    if (BRepAdaptor_Surface(face).GetType() != GeomAbs_Cylinder)
      continue;
    ++seen;
    found = face;
  }
  return seen == 1 ? found : TopoDS_Face();
}

std::vector<TopoDS_Edge> shared_edges_of(const TopoDS_Face &a, const TopoDS_Face &b) {
  TopTools_IndexedMapOfShape a_edges;
  TopExp::MapShapes(a, TopAbs_EDGE, a_edges);
  TopTools_IndexedMapOfShape b_edges;
  TopExp::MapShapes(b, TopAbs_EDGE, b_edges);
  std::vector<TopoDS_Edge> out;
  for (int i = 1; i <= a_edges.Extent(); ++i) {
    if (b_edges.Contains(a_edges(i)))
      out.push_back(TopoDS::Edge(a_edges(i)));
  }
  return out;
}

std::vector<TopoDS_Face> cylindrical_faces(const TopoDS_Shape &shape) {
  std::vector<TopoDS_Face> out;
  for (TopExp_Explorer explorer(shape, TopAbs_FACE); explorer.More(); explorer.Next()) {
    const TopoDS_Face face = TopoDS::Face(explorer.Current());
    if (BRepAdaptor_Surface(face).GetType() == GeomAbs_Cylinder)
      out.push_back(face);
  }
  std::sort(out.begin(), out.end(), [](const TopoDS_Face &a, const TopoDS_Face &b) {
    return BRepAdaptor_Surface(a).Cylinder().Radius() <
           BRepAdaptor_Surface(b).Cylinder().Radius();
  });
  return out;
}

struct Fixture {
  bool ok = false;
  TopoDS_Shape body;
  TopoDS_Face blend;
};

// box(10) with the (10,10) edge rounded R2 and the (0,0) edge rounded R3. Two
// blends, ONE convexity, TWO radii — the set that the convexity guard alone waves
// through.
struct MixedRadiusFixture {
  bool ok = false;
  TopoDS_Shape body;
  std::vector<TopoDS_Face> blends;  // ascending radius: R2 then R3
};

MixedRadiusFixture mixed_radius_rounds() {
  MixedRadiusFixture out;
  const TopoDS_Shape sharp = ft::box();
  BRepFilletAPI_MakeFillet fillet(sharp);
  fillet.Add(2.0, vertical_edge_at(sharp, 10.0, 10.0));
  fillet.Add(3.0, vertical_edge_at(sharp, 0.0, 0.0));
  fillet.Build();
  if (!fillet.IsDone() || fillet.Shape().IsNull())
    return out;
  out.body = fillet.Shape();
  out.blends = cylindrical_faces(out.body);
  out.ok = out.blends.size() == 2;
  return out;
}

// box(10) with one vertical edge rounded R2 — a CONVEX round.
Fixture convex_round() {
  Fixture out;
  const TopoDS_Shape sharp = ft::box();
  BRepFilletAPI_MakeFillet fillet(sharp);
  fillet.Add(2.0, vertical_edge_at(sharp, 10.0, 10.0));
  fillet.Build();
  if (!fillet.IsDone() || fillet.Shape().IsNull())
    return out;
  out.body = fillet.Shape();
  out.blend = only_cylindrical_face(out.body);
  out.ok = !out.blend.IsNull();
  return out;
}

// An L-prism (box(10) minus a 6x6 corner column) with its reentrant vertical edge
// filleted R2 — a CONCAVE fillet.
Fixture concave_fillet() {
  Fixture out;
  const TopoDS_Shape cut =
      BRepPrimAPI_MakeBox(gp_Pnt(4.0, 4.0, -1.0), 7.0, 7.0, 12.0).Shape();
  BRepAlgoAPI_Cut cutter(ft::box(), cut);
  cutter.Build();
  if (!cutter.IsDone() || cutter.Shape().IsNull())
    return out;
  const TopoDS_Shape prism = cutter.Shape();

  const TopoDS_Edge reentrant = vertical_edge_at(prism, 4.0, 4.0);
  if (reentrant.IsNull())
    return out;
  BRepFilletAPI_MakeFillet fillet(prism);
  fillet.Add(2.0, reentrant);
  fillet.Build();
  if (!fillet.IsDone() || fillet.Shape().IsNull())
    return out;
  out.body = fillet.Shape();
  out.blend = only_cylindrical_face(out.body);
  out.ok = !out.blend.IsNull();
  return out;
}

// A slab with a half-round end: a cylinder of radius 5 tangent to two PARALLEL
// planes 10 apart. It recognizes, it proves, it is convex, and its two boundary
// edges are equal-length lines parallel to the axis — layers 1-3 all pass. It is
// still not a suppressible blend, because the two supports never meet.
Fixture parallel_support_cap() {
  Fixture out;
  const gp_Pnt a(0.0, 0.0, 0.0);
  const gp_Pnt b(10.0, 0.0, 0.0);
  const gp_Pnt c(15.0, 5.0, 0.0);
  const gp_Pnt d(10.0, 10.0, 0.0);
  const gp_Pnt e(0.0, 10.0, 0.0);
  const GC_MakeArcOfCircle arc(b, c, d);
  if (!arc.IsDone())
    return out;
  BRepBuilderAPI_MakeWire wire(BRepBuilderAPI_MakeEdge(a, b).Edge());
  wire.Add(BRepBuilderAPI_MakeEdge(arc.Value()).Edge());
  wire.Add(BRepBuilderAPI_MakeEdge(d, e).Edge());
  wire.Add(BRepBuilderAPI_MakeEdge(e, a).Edge());
  if (!wire.IsDone())
    return out;
  BRepBuilderAPI_MakeFace face(wire.Wire());
  if (!face.IsDone())
    return out;
  BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(0.0, 0.0, 10.0));
  prism.Build();
  if (!prism.IsDone() || prism.Shape().IsNull())
    return out;
  out.body = prism.Shape();
  out.blend = only_cylindrical_face(out.body);
  out.ok = !out.blend.IsNull();
  return out;
}

// FIX 6's fixture: a round boss half-buried in a wall, with one of the two straight
// junction edges filleted R1. The blend's supports are a PLANE and a CYLINDER whose
// axes are parallel, so the seed edge is still a line — the only configuration in
// which a cylinder-supported blend has the straight boundary layer 3 requires. This
// is the only fixture here that exercises `same_surface`'s cylinder branch and
// `analytic_cylinder_blend`'s cylinder-support rule; every other one is
// plane-supported throughout.
Fixture cylinder_supported_fillet() {
  Fixture out;
  const TopoDS_Shape wall = BRepPrimAPI_MakeBox(20.0, 10.0, 10.0).Shape();
  const TopoDS_Shape boss =
      BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(20.0, 5.0, 0.0), gp_Dir(0.0, 0.0, 1.0)), 3.0,
                               10.0)
          .Shape();
  BRepAlgoAPI_Fuse fuse(wall, boss);
  fuse.Build();
  if (!fuse.IsDone() || fuse.Shape().IsNull())
    return out;
  const TopoDS_Shape body = fuse.Shape();

  const TopoDS_Edge junction = vertical_edge_at(body, 20.0, 2.0);
  if (junction.IsNull())
    return out;
  BRepFilletAPI_MakeFillet fillet(body);
  fillet.Add(1.0, junction);
  fillet.Build();
  if (!fillet.IsDone() || fillet.Shape().IsNull())
    return out;
  out.body = fillet.Shape();
  // Two cylinders survive here — the boss and the blend — so pick the R1 one.
  for (const TopoDS_Face &face : cylindrical_faces(out.body)) {
    if (std::abs(BRepAdaptor_Surface(face).Cylinder().Radius() - 1.0) <= 1.0e-9) {
      out.blend = face;
      break;
    }
  }
  out.ok = !out.blend.IsNull();
  return out;
}

// A PARTIALLY-TRIMMED round: box(10) rounded R2 on one vertical edge, then the top
// 4 mm of that round filled back in. The blend now runs only z in [0, 6] while its
// two supports already meet SHARPLY over z in [6, 10]. Suppression exposes a sharp
// edge spanning the whole 10 mm, and a rebuild seeded from it would round an edge
// the user never rounded.
Fixture partially_trimmed_round() {
  Fixture out;
  const Fixture round = convex_round();
  if (!round.ok)
    return out;
  const TopoDS_Shape block =
      BRepPrimAPI_MakeBox(gp_Pnt(8.0, 8.0, 6.0), 2.0, 2.0, 4.0).Shape();
  BRepAlgoAPI_Fuse fuse(round.body, block);
  fuse.Build();
  if (!fuse.IsDone() || fuse.Shape().IsNull())
    return out;
  ShapeUpgrade_UnifySameDomain unify(fuse.Shape(), Standard_True, Standard_True,
                                     Standard_False);
  unify.Build();
  if (unify.Shape().IsNull())
    return out;
  out.body = unify.Shape();
  out.blend = only_cylindrical_face(out.body);
  out.ok = !out.blend.IsNull();
  return out;
}

// --- the identity round trip ------------------------------------------------

struct RoundTrip {
  bool ok = false;
  kf::SuppressionResult suppressed;
  kf::ReblendResult reblended;
};

RoundTrip identity_round_trip(const char *label, const Fixture &fixture,
                              kf::BlendConvexity expected_convexity,
                              double expected_input_volume,
                              double expected_suppressed_volume) {
  RoundTrip out;
  check(fixture.ok, std::string("fixture: ") + label + " builds with one cylindrical face");
  if (!fixture.ok)
    return out;

  const double input_volume = volume(fixture.body);
  const int input_faces = face_count(fixture.body);
  std::fprintf(stderr, "\n=== identity round trip: %s ===\n", label);
  std::fprintf(stderr, "input        volume = %.13f  faces = %d\n", input_volume, input_faces);
  check(near_relative(input_volume, expected_input_volume, 1.0e-9),
        std::string(label) + ": the fixture's own volume is the analytic one");

  // --- layers 1-3 -----------------------------------------------------------
  const kf::BlendCertification certified = kf::certify_blend(fixture.blend, fixture.body);
  check(certified.ok, std::string(label) + ": the blend certifies — " + certified.reason);
  if (!certified.ok)
    return out;
  std::fprintf(stderr, "certified    radius = %.13f  boundaryLength = %.13f\n",
               certified.blend.radius, certified.blend.boundary_length);
  std::fprintf(stderr, "raised budget       samples = %d  boundaries = %d  profileErr = %.3e\n",
               certified.evidence.samples, certified.evidence.boundaries,
               certified.evidence.maximum_profile_error);
  check(near_relative(certified.blend.radius, 2.0, 1.0e-12),
        std::string(label) + ": the certified radius is the analytic 2.0");
  check(near_relative(certified.blend.boundary_length, 10.0, 1.0e-12),
        std::string(label) + ": the certified boundary length is the full 10 mm run");
  check(certified.blend.convexity == expected_convexity,
        std::string(label) + ": convexity is measured, not assumed");
  check(certified.evidence.samples >= kf::kMinimumReconstructionSamples,
        std::string(label) + ": the raised budget yields at least 64 samples");
  check(certified.evidence.boundaries == 2,
        std::string(label) + ": exactly two measured boundaries");

  // --- suppress -------------------------------------------------------------
  out.suppressed = kf::suppress_blends(fixture.body, {certified.blend});
  check(out.suppressed.ok,
        std::string(label) + ": suppression passes all eight postconditions — " +
            out.suppressed.reason);
  if (!out.suppressed.ok)
    return out;
  std::fprintf(stderr, "suppressed   volume = %.13f  faces = %d  maxTol = %.3e\n",
               out.suppressed.volume, face_count(out.suppressed.suppressed_shape),
               out.suppressed.maximum_tolerance);
  check(near_relative(out.suppressed.volume, expected_suppressed_volume, 1.0e-9),
        std::string(label) + ": the suppressed body is exactly the un-blended one");
  check(face_count(out.suppressed.suppressed_shape) == input_faces - 1,
        std::string(label) + ": suppression removed exactly the blend face");
  check(out.suppressed.seed_edges.size() == 1,
        std::string(label) + ": exactly one seed edge was exposed");
  if (out.suppressed.seed_edges.size() != 1)
    return out;
  check(near_relative(out.suppressed.seed_edges.front().boundary_length, 10.0, 1.0e-12),
        std::string(label) + ": the seed carries the original 10 mm extent");

  // The volume DIRECTION, asserted as a sign rather than only as a magnitude —
  // this is the postcondition-8 derivation stated from the test's side.
  const double delta = out.suppressed.volume - input_volume;
  const bool convex = expected_convexity == kf::BlendConvexity::Convex;
  check(convex ? delta > 0.0 : delta < 0.0,
        std::string(label) + ": suppressing a " + (convex ? "convex round ADDS" : "concave fillet REMOVES") +
            " material");
  check(near_relative(std::abs(delta), kCornerVolume, 1.0e-9),
        std::string(label) + ": the moved volume is the exact corner volume L*(R^2 - pi*R^2/4)");

  // --- reblend at the SAME radius, with nothing in between ------------------
  out.reblended = kf::reblend(out.suppressed.suppressed_shape, out.suppressed.seed_edges,
                              certified.blend.radius);
  check(out.reblended.ok,
        std::string(label) + ": the rebuild passes every reproduction postcondition — " +
            out.reblended.reason);
  if (!out.reblended.ok)
    return out;
  std::fprintf(stderr, "reblended    volume = %.13f  faces = %d  maxTol = %.3e\n",
               out.reblended.volume, face_count(out.reblended.shape),
               out.reblended.maximum_tolerance);
  std::fprintf(stderr, "reblend evidence    samples = %d  boundaries = %d  profileErr = %.3e  "
                       "tangency = %.3e\n",
               out.reblended.evidence.samples, out.reblended.evidence.boundaries,
               out.reblended.evidence.maximum_profile_error,
               out.reblended.evidence.maximum_tangency_radians);

  // THE ASSERTION THIS FILE EXISTS FOR.
  check(near_relative(out.reblended.volume, input_volume, 1.0e-9),
        std::string(label) + ": the round trip reproduces the input volume to 1e-9 relative");
  check(face_count(out.reblended.shape) == input_faces,
        std::string(label) + ": the round trip reproduces the input face count");
  check(out.reblended.blend_faces.size() == 1,
        std::string(label) + ": exactly one blend face was rebuilt");
  check(out.reblended.evidence.boundaries == 2,
        std::string(label) + ": the rebuilt blend measures exactly two boundaries");
  check(out.reblended.evidence.samples >= kf::kMinimumReconstructionSamples,
        std::string(label) + ": the rebuilt blend measures at the raised budget");
  if (!out.reblended.blend_faces.empty()) {
    const BRepAdaptor_Surface rebuilt(out.reblended.blend_faces.front());
    check(rebuilt.GetType() == GeomAbs_Cylinder,
          std::string(label) + ": the rebuilt face is an exact cylinder");
    check(rebuilt.GetType() == GeomAbs_Cylinder &&
              near_relative(rebuilt.Cylinder().Radius(), 2.0, 1.0e-12),
          std::string(label) + ": the rebuilt cylinder is R2");
    check(kf::blend_convexity(out.reblended.blend_faces.front()) == expected_convexity,
          std::string(label) + ": the rebuilt blend has the original convexity");
  }

  out.ok = true;
  return out;
}

// --- the characterized two-stage tolerance cap ------------------------------

void tolerance_cap_is_characterized(const char *label, const Fixture &fixture,
                                    const RoundTrip &trip) {
  if (!trip.ok)
    return;
  const kf::RoundTripTolerance tolerance =
      kf::roundtrip_tolerance_cap(fixture.body, trip.suppressed, trip.reblended);
  std::fprintf(stderr,
               "tolerance %-8s input = %.6e  afterSuppress = %.6e  afterReblend = %.6e  "
               "inflation = %.4f  contribution = %.6e  cap = %.1e\n",
               label, tolerance.input, tolerance.after_suppress, tolerance.after_reblend,
               tolerance.inflation(), tolerance.contribution(),
               kf::kRoundTripToleranceCapMm);
  check(tolerance.within_cap(),
        std::string(label) + ": what the round trip ADDED stays under the characterized cap");
  // The cap has 6.7x headroom, so it alone would not notice a stage that started
  // inflating. `kMeasuredRoundTrip` is the value actually observed on OCCT 8.0.1 —
  // an input at Precision::Confusion() (1e-7) leaves both stages at 1.5e-7, and the
  // second stage adds nothing at all. Any drift off that number reds HERE, one
  // assertion before the ceiling would have been reached, which is the difference
  // between a characterized cap and a cap that gets raised when it fires.
  check(tolerance.after_suppress <= kMeasuredRoundTrip &&
            tolerance.after_reblend <= kMeasuredRoundTrip,
        std::string(label) + ": the round trip ends at the pinned measured tolerance " +
            std::to_string(kMeasuredRoundTrip));
}

// --- refusal paths, each asserted BY REASON ---------------------------------

// A cylinder tangent to two PARALLEL planes certifies through every layer this
// module runs before it touches geometry, and is still not suppressible: extending
// the two supports never produces the sharp edge a rebuild would need.
void refuses_a_blend_whose_supports_never_meet() {
  const Fixture fixture = parallel_support_cap();
  check(fixture.ok, "fixture: the half-round slab end builds with one cylindrical face");
  if (!fixture.ok)
    return;
  std::fprintf(stderr, "\n=== refusal: supports never meet (half-round slab end) ===\n");
  std::fprintf(stderr, "input        volume = %.13f  faces = %d\n", volume(fixture.body),
               face_count(fixture.body));

  const kf::BlendCertification certified = kf::certify_blend(fixture.blend, fixture.body);
  std::fprintf(stderr, "certify -> ok=%d reason=%s\n", certified.ok ? 1 : 0,
               certified.reason.c_str());
  // Layers 1-3 passing here is the POINT: this is a false positive that only the
  // suppression postconditions can catch.
  check(certified.ok,
        std::string("parallel supports: layers 1-3 accept it (that is why layer 4 exists) — ") +
            certified.reason);
  if (!certified.ok)
    return;

  const kf::SuppressionResult suppressed =
      kf::suppress_blends(fixture.body, {certified.blend});
  std::fprintf(stderr, "suppress -> ok=%d reason=%s\n", suppressed.ok ? 1 : 0,
               suppressed.reason.c_str());
  check(!suppressed.ok, "parallel supports: suppression refuses");
  check(suppressed.suppressed_shape.IsNull() && suppressed.seed_edges.empty(),
        "parallel supports: a refusal carries no partial result");
  check(starts_with(suppressed.reason, "SUPPRESSION_NOT_DONE"),
        "parallel supports: postcondition 1 refuses");
  check(suppressed.reason.find("BOPAlgo_AlertUnableToRemoveTheFeature") != std::string::npos,
        "parallel supports: the refusal quotes the kernel's own alert");
}

// A planar box face forged into a `RecognizedBlend`. Nothing about it is a blend;
// the kernel and the postconditions have to say so.
void refuses_a_face_that_is_not_a_blend() {
  const TopoDS_Shape body = ft::box();
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(body, TopAbs_FACE, faces);
  check(faces.Extent() == 6, "fixture: the box has six faces");
  if (faces.Extent() != 6)
    return;
  std::fprintf(stderr, "\n=== refusal: a planar box face forged as a blend ===\n");

  const TopoDS_Face target = TopoDS::Face(faces(1));
  const kf::BlendCertification certified = kf::certify_blend(target, body);
  std::fprintf(stderr, "certify -> ok=%d reason=%s\n", certified.ok ? 1 : 0,
               certified.reason.c_str());
  check(!certified.ok, "box face: certification refuses a planar face");
  check(starts_with(certified.reason, "BLEND_NOT_RECOGNIZED"),
        "box face: layer 1's sampled recognition is what refuses it");

  // Forge the struct certification would never produce, so the SUPPRESSION side is
  // exercised rather than short-circuited by the layer above it.
  std::vector<TopoDS_Face> neighbours;
  TopTools_IndexedMapOfShape target_edges;
  TopExp::MapShapes(target, TopAbs_EDGE, target_edges);
  for (int i = 2; i <= faces.Extent() && neighbours.size() < 2; ++i) {
    const TopoDS_Face candidate = TopoDS::Face(faces(i));
    for (TopExp_Explorer explorer(candidate, TopAbs_EDGE); explorer.More(); explorer.Next()) {
      if (target_edges.Contains(explorer.Current())) {
        neighbours.push_back(candidate);
        break;
      }
    }
  }
  check(neighbours.size() == 2, "fixture: two neighbouring box faces were found");
  if (neighbours.size() != 2)
    return;

  kf::RecognizedBlend forged;
  forged.face = target;
  forged.support_a = neighbours[0];
  forged.support_b = neighbours[1];
  forged.radius = 2.0;
  forged.convexity = kf::BlendConvexity::Convex;
  forged.boundary_length = 10.0;

  const kf::SuppressionResult suppressed = kf::suppress_blends(body, {forged});
  std::fprintf(stderr, "suppress -> ok=%d reason=%s\n", suppressed.ok ? 1 : 0,
               suppressed.reason.c_str());
  check(!suppressed.ok, "box face: suppression refuses a face that is not a blend");
  check(suppressed.suppressed_shape.IsNull() && suppressed.seed_edges.empty(),
        "box face: a refusal carries no partial result");
  check(starts_with(suppressed.reason, "SUPPRESSION_NOT_DONE"),
        "box face: postcondition 1 refuses");
  check(suppressed.reason.find("BOPAlgo_AlertUnableToRemoveTheFeature") != std::string::npos,
        "box face: the refusal quotes the kernel's own alert");
}

// Postconditions 7 and 8, reached with a FORGED `RecognizedBlend` over the real
// convex fixture. Both exist for cases the kernel is happy to build and only the
// postcondition can reject, and neither is reachable from a certified blend by
// construction — `certify_blend` measures the extent and the convexity itself. The
// forgery is the same technique the box-face case uses: describe the face
// incorrectly and require the engine to notice.
void postconditions_reject_a_misdescribed_blend() {
  const Fixture fixture = convex_round();
  check(fixture.ok, "fixture: the convex round builds");
  if (!fixture.ok)
    return;
  const kf::BlendCertification certified = kf::certify_blend(fixture.blend, fixture.body);
  check(certified.ok, "fixture: the convex round certifies");
  if (!certified.ok)
    return;
  std::fprintf(stderr, "\n=== refusal: a misdescribed blend ===\n");

  // POSTCONDITION 7. The blend claims a 6 mm extent; suppression exposes the real
  // 10 mm sharp edge. This is the partially-trimmed round's signature — a blend
  // whose trim is shorter than the run its supports actually share — reached by
  // description rather than by topology, because OCCT refuses to defeature the
  // topological version outright (see below).
  kf::RecognizedBlend short_extent = certified.blend;
  short_extent.boundary_length = 6.0;
  const kf::SuppressionResult mismatched =
      kf::suppress_blends(fixture.body, {short_extent});
  std::fprintf(stderr, "wrong extent  -> ok=%d reason=%s\n", mismatched.ok ? 1 : 0,
               mismatched.reason.c_str());
  check(!mismatched.ok, "wrong extent: suppression refuses");
  check(starts_with(mismatched.reason, "SUPPRESSION_SEED_EDGE_NOT_FOUND"),
        "wrong extent: postcondition 7 refuses rather than adopting the 10 mm edge");
  check(mismatched.seed_edges.empty(), "wrong extent: a refusal carries no seed");

  // POSTCONDITION 8. The face is a convex round, so suppressing it ADDS material.
  // Describing it as concave inverts the expected direction, and the postcondition
  // has to notice — this is the derivation asserted from the failing side.
  kf::RecognizedBlend wrong_sign = certified.blend;
  wrong_sign.convexity = kf::BlendConvexity::Concave;
  const kf::SuppressionResult inverted = kf::suppress_blends(fixture.body, {wrong_sign});
  std::fprintf(stderr, "wrong sign    -> ok=%d reason=%s\n", inverted.ok ? 1 : 0,
               inverted.reason.c_str());
  check(!inverted.ok, "wrong sign: suppression refuses");
  check(starts_with(inverted.reason, "SUPPRESSION_VOLUME_DIRECTION"),
        "wrong sign: postcondition 8 refuses a volume that moved the other way");
  check(inverted.suppressed_shape.IsNull(), "wrong sign: a refusal carries no partial result");

  // Mixed convexity in one set has no single expected direction, so it is refused
  // at the input rather than measured.
  const kf::SuppressionResult mixed =
      kf::suppress_blends(fixture.body, {certified.blend, wrong_sign});
  std::fprintf(stderr, "mixed sign    -> ok=%d reason=%s\n", mixed.ok ? 1 : 0,
               mixed.reason.c_str());
  check(starts_with(mixed.reason, "SUPPRESSION_INPUT_INVALID"),
        "mixed convexity: refused at the input, before any geometry is destroyed");
}

// Layer 3's reason for existing. The blend runs 6 mm; the sharp edge suppression
// exposes runs 10 mm; binding the seed anyway would round 4 mm the user never
// rounded. The length match refuses instead.
void refuses_a_partially_trimmed_round() {
  const Fixture fixture = partially_trimmed_round();
  check(fixture.ok, "fixture: the partially-trimmed round builds with one cylindrical face");
  if (!fixture.ok)
    return;
  std::fprintf(stderr, "\n=== refusal: partially-trimmed round ===\n");
  std::fprintf(stderr, "input        volume = %.13f  faces = %d\n", volume(fixture.body),
               face_count(fixture.body));
  // 1000 minus the 6 mm of run that is still rounded.
  check(near_relative(volume(fixture.body), 1000.0 - 0.6 * kCornerVolume, 1.0e-9),
        "partially-trimmed round: the fixture is the analytic partially-rounded box");

  const kf::BlendCertification certified = kf::certify_blend(fixture.blend, fixture.body);
  std::fprintf(stderr, "certify -> ok=%d reason=%s boundaryLength=%.13f\n",
               certified.ok ? 1 : 0, certified.reason.c_str(),
               certified.blend.boundary_length);
  check(certified.ok,
        std::string("partially-trimmed round: layers 1-3 accept the trimmed face — ") +
            certified.reason);
  if (!certified.ok)
    return;
  check(near_relative(certified.blend.boundary_length, 6.0, 1.0e-9),
        "partially-trimmed round: the certified extent is the 6 mm that is actually rounded");

  const kf::SuppressionResult suppressed =
      kf::suppress_blends(fixture.body, {certified.blend});
  std::fprintf(stderr, "suppress -> ok=%d reason=%s\n", suppressed.ok ? 1 : 0,
               suppressed.reason.c_str());
  check(!suppressed.ok, "partially-trimmed round: suppression refuses");
  // MEASURED: OCCT 8.0.1 refuses this one at the kernel, before the seed-length
  // check gets a turn — `BOPAlgo_RemoveFeatures` will not reconstruct supports that
  // already meet sharply beside the trimmed round. That is the right answer for the
  // wrong reason as far as layer 3 is concerned, so the seed-length postcondition is
  // exercised separately by `postconditions_reject_a_misdescribed_blend`. Pinned
  // exactly, so an OCCT release that starts defeaturing this reds here and forces
  // layer 3 to be re-proved on the topological case rather than only the described
  // one.
  check(starts_with(suppressed.reason, "SUPPRESSION_NOT_DONE"),
        "partially-trimmed round: postcondition 1 refuses (pinned OCCT behaviour)");
  check(suppressed.reason.find("BOPAlgo_AlertUnableToRemoveTheFeature") != std::string::npos,
        "partially-trimmed round: the refusal quotes the kernel's own alert");
}

// FIX 6. The cylinder-support paths — `same_surface`'s `GeomAbs_Cylinder` branch and
// `analytic_cylinder_blend`'s parallel-axis rule — are unexercised by every other
// fixture here, all of which are plane-supported end to end. This drives them.
// Measured, not assumed: whichever way OCCT answers is pinned.
void cylinder_supported_round_trip() {
  const Fixture fixture = cylinder_supported_fillet();
  check(fixture.ok, "fixture: the boss-in-wall fillet builds with an R1 cylindrical blend");
  if (!fixture.ok)
    return;
  std::fprintf(stderr, "\n=== cylinder-supported blend (boss in wall, R1) ===\n");
  const double input_volume = volume(fixture.body);
  const int input_faces = face_count(fixture.body);
  std::fprintf(stderr, "input        volume = %.13f  faces = %d\n", input_volume, input_faces);

  const kf::BlendCertification certified = kf::certify_blend(fixture.blend, fixture.body);
  std::fprintf(stderr, "certify -> ok=%d reason=%s radius=%.13f boundaryLength=%.13f conv=%d\n",
               certified.ok ? 1 : 0, certified.reason.c_str(), certified.blend.radius,
               certified.blend.boundary_length,
               static_cast<int>(certified.blend.convexity));
  check(certified.ok,
        std::string("cylinder support: the blend certifies against a plane AND a cylinder — ") +
            certified.reason);
  if (!certified.ok)
    return;
  check(near_relative(certified.blend.radius, 1.0, 1.0e-12),
        "cylinder support: the certified radius is the analytic 1.0");
  check(certified.blend.convexity == kf::BlendConvexity::Concave,
        "cylinder support: a boss meeting a wall is a REENTRANT corner");
  check(near_relative(certified.blend.boundary_length, 10.0, 1.0e-9),
        "cylinder support: the straight boundary runs the full 10 mm height");
  const bool cylinder_support =
      BRepAdaptor_Surface(certified.blend.support_a).GetType() == GeomAbs_Cylinder ||
      BRepAdaptor_Surface(certified.blend.support_b).GetType() == GeomAbs_Cylinder;
  check(cylinder_support, "cylinder support: one support really is a cylinder");

  const kf::SuppressionResult suppressed =
      kf::suppress_blends(fixture.body, {certified.blend});
  std::fprintf(stderr,
               "suppress -> ok=%d reason=%s volume=%.13f faces=%d tol %.6e -> %.6e\n",
               suppressed.ok ? 1 : 0, suppressed.reason.c_str(), suppressed.volume,
               suppressed.ok ? face_count(suppressed.suppressed_shape) : 0,
               suppressed.input_tolerance, suppressed.maximum_tolerance);
  if (!suppressed.ok) {
    // MEASURED, and it is the case postcondition 3 was written for. OCCT removes the
    // blend correctly — the suppressed volume is exactly the un-filleted fused body,
    // 2000 + pi*3^2*10/2 = 2141.3716694 — and then HEALS the body from 14 faces down
    // to 8, unifying the boolean's split boss and wall faces along the way. That is
    // not the operation that was requested: five faces the caller may hold
    // ElementIds for vanish with no unique ancestor, so the bijection postcondition
    // 3 asserts is broken and the whole thing is refused.
    //
    // This is the first fixture in the corpus to reach that postcondition, and it is
    // why FIX 6 was worth attempting. Pinned exactly: an OCCT release that stops
    // unifying reds here, and the cylinder-support identity ROUND TRIP (which needs
    // a fixture whose boolean does not leave split faces to unify) is owed to C6.
    check(near_relative(suppressed.volume, 2000.0 + 3.14159265358979323846 * 9.0 * 10.0 / 2.0,
                        1.0e-9),
          "cylinder support: the kernel's suppression geometry is exact even so");
    check(starts_with(suppressed.reason, "SUPPRESSION_FACE_COUNT"),
          "cylinder support: postcondition 3 refuses a healed body (pinned OCCT behaviour)");
    check(suppressed.seed_edges.empty(), "cylinder support: a refusal carries no seed");
    return;
  }
  check(suppressed.volume < input_volume,
        "cylinder support: suppressing a concave fillet REMOVES material");

  const kf::ReblendResult reblended = kf::reblend(suppressed.suppressed_shape,
                                                  suppressed.seed_edges,
                                                  certified.blend.radius);
  std::fprintf(stderr, "reblend  -> ok=%d reason=%s volume=%.13f faces=%d maxTol=%.3e\n",
               reblended.ok ? 1 : 0, reblended.reason.c_str(), reblended.volume,
               reblended.ok ? face_count(reblended.shape) : 0, reblended.maximum_tolerance);
  check(reblended.ok,
        std::string("cylinder support: the rebuild passes the reproduction proof — ") +
            reblended.reason);
  if (!reblended.ok)
    return;
  check(near_relative(reblended.volume, input_volume, 1.0e-9),
        "cylinder support: the round trip reproduces the input volume to 1e-9 relative");
  check(face_count(reblended.shape) == input_faces,
        "cylinder support: the round trip reproduces the input face count");
}

// Two blends, one convexity, TWO radii. Nothing in the eight suppression
// postconditions looks at radius, so the set sails through all of them; a rebuild
// then runs at ONE radius and returns the R3 round as an R2 round with ok=true.
// That is precisely the "reconstruction differs from the original" failure the
// module's safety argument forbids, so it is refused at BOTH layers: the set at the
// input, and the seed's own radius in the rebuild.
void refuses_mixed_radii() {
  const MixedRadiusFixture fixture = mixed_radius_rounds();
  check(fixture.ok, "fixture: the two-radius box builds with two cylindrical faces");
  if (!fixture.ok)
    return;
  std::fprintf(stderr, "\n=== refusal: mixed blend radii ===\n");
  const double input_volume = volume(fixture.body);
  std::fprintf(stderr, "input        volume = %.13f  faces = %d\n", input_volume,
               face_count(fixture.body));
  check(near_relative(input_volume, 1000.0 - kCornerVolume - kCornerVolumeR3, 1.0e-9),
        "mixed radii: the fixture is the analytic two-radius box");

  const kf::BlendCertification small = kf::certify_blend(fixture.blends[0], fixture.body);
  const kf::BlendCertification large = kf::certify_blend(fixture.blends[1], fixture.body);
  check(small.ok && large.ok, "mixed radii: both blends certify");
  if (!small.ok || !large.ok)
    return;
  check(near_relative(small.blend.radius, 2.0, 1.0e-12) &&
            near_relative(large.blend.radius, 3.0, 1.0e-12),
        "mixed radii: the two certified radii are 2.0 and 3.0");
  check(small.blend.convexity == large.blend.convexity,
        "mixed radii: the convexity guard cannot separate them — both are convex");

  const kf::SuppressionResult mixed =
      kf::suppress_blends(fixture.body, {small.blend, large.blend});
  std::fprintf(stderr, "mixed set     -> ok=%d reason=%s\n", mixed.ok ? 1 : 0,
               mixed.reason.c_str());
  check(!mixed.ok, "mixed radii: suppression refuses the set");
  check(starts_with(mixed.reason, "SUPPRESSION_INPUT_INVALID"),
        "mixed radii: refused at the input, mirroring the mixed-convexity guard");

  // And the seed carries its OWN radius, so a rebuild at the wrong one is refused
  // even when the set was uniform. Suppress the R3 blend alone — a legal
  // single-radius set — then ask for R2.
  const kf::SuppressionResult single = kf::suppress_blends(fixture.body, {large.blend});
  check(single.ok, std::string("mixed radii: the R3 blend alone suppresses — ") + single.reason);
  if (!single.ok)
    return;
  check(single.seed_edges.size() == 1 &&
            near_relative(single.seed_edges.front().radius, 3.0, 1.0e-12),
        "mixed radii: the seed carries the ORIGINAL R3, not the caller's number");
  const kf::ReblendResult wrong = kf::reblend(single.suppressed_shape, single.seed_edges, 2.0);
  std::fprintf(stderr, "R3 seed at R2 -> ok=%d reason=%s volume=%.13f\n", wrong.ok ? 1 : 0,
               wrong.reason.c_str(), wrong.volume);
  check(!wrong.ok, "mixed radii: rebuilding an R3 seed at R2 refuses");
  check(starts_with(wrong.reason, "REBLEND_INPUT_INVALID"),
        "mixed radii: the seed's own radius is what the rebuild is held to");
  check(wrong.shape.IsNull(), "mixed radii: a refusal publishes nothing");
}

// FIX 4's negative sweep. Every rung of the reproduction ladder that a forged seed
// can honestly reach, reached — the codes with no failing path after this are listed
// as DECLARED in the gate row rather than left to look covered.
void reblend_refuses_forged_seeds() {
  const Fixture fixture = convex_round();
  const kf::BlendCertification certified = kf::certify_blend(fixture.blend, fixture.body);
  const kf::SuppressionResult suppressed =
      kf::suppress_blends(fixture.body, {certified.blend});
  check(fixture.ok && certified.ok && suppressed.ok,
        "fixture: the convex round certifies and suppresses");
  if (!fixture.ok || !certified.ok || !suppressed.ok)
    return;
  const TopoDS_Shape body = suppressed.suppressed_shape;
  const double radius = certified.blend.radius;
  std::fprintf(stderr, "\n=== refusal: forged seeds down the reproduction ladder ===\n");

  const kf::ReblendResult empty = kf::reblend(body, {}, radius);
  std::fprintf(stderr, "empty set     -> ok=%d reason=%s\n", empty.ok ? 1 : 0,
               empty.reason.c_str());
  check(starts_with(empty.reason, "REBLEND_INPUT_INVALID"),
        "forged seed: an empty seed set is refused at the input");

  std::vector<kf::SuppressedBlendSeed> nulled = suppressed.seed_edges;
  nulled.front().edge = TopoDS_Edge();
  const kf::ReblendResult null_edge = kf::reblend(body, nulled, radius);
  std::fprintf(stderr, "null edge     -> ok=%d reason=%s\n", null_edge.ok ? 1 : 0,
               null_edge.reason.c_str());
  check(starts_with(null_edge.reason, "REBLEND_INPUT_INVALID"),
        "forged seed: an incomplete seed is refused at the input");

  // FIX 2. The identity round trip cannot invert a corner's sign, so this gate is
  // dead code until C5 puts an offset between the halves — which is exactly why it
  // is proven now, by description, rather than added later with no red-first test.
  std::vector<kf::SuppressedBlendSeed> flipped = suppressed.seed_edges;
  flipped.front().convexity = kf::BlendConvexity::Concave;
  const kf::ReblendResult wrong_sign = kf::reblend(body, flipped, radius);
  std::fprintf(stderr, "flipped sign  -> ok=%d reason=%s\n", wrong_sign.ok ? 1 : 0,
               wrong_sign.reason.c_str());
  check(!wrong_sign.ok, "forged seed: a flipped convexity is refused");
  check(starts_with(wrong_sign.reason, "REBLEND_CONVEXITY"),
        "forged seed: the material side is checked, which analytic tangency cannot do");
  check(wrong_sign.shape.IsNull(), "forged seed: a refusal publishes nothing");

  // A seed edge belonging to no body the rebuild has ever seen.
  std::vector<kf::SuppressedBlendSeed> foreign = suppressed.seed_edges;
  foreign.front().edge = ft::vertical_edges(ft::box(3.0)).front();
  const kf::ReblendResult stranger = kf::reblend(body, foreign, radius);
  std::fprintf(stderr, "foreign edge  -> ok=%d reason=%s\n", stranger.ok ? 1 : 0,
               stranger.reason.c_str());
  check(!stranger.ok, "forged seed: an edge from an unrelated body is refused");
  // MEASURED: OCCT refuses the contour outright, so `FilletBuilder` reds before the
  // generated-face rung is reached. `REBLEND_NOT_GENERATED` therefore has no failing
  // path in this corpus and is DECLARED untested.
  check(starts_with(stranger.reason, "REBLEND_FAILED"),
        "forged seed: the builder refuses the contour before the ladder starts");

  // Two seeds naming the SAME edge: one rebuilt face, two claimants.
  std::vector<kf::SuppressedBlendSeed> doubled = suppressed.seed_edges;
  doubled.push_back(doubled.front());
  const kf::ReblendResult duplicated = kf::reblend(body, doubled, radius);
  std::fprintf(stderr, "duplicate seed-> ok=%d reason=%s\n", duplicated.ok ? 1 : 0,
               duplicated.reason.c_str());
  check(!duplicated.ok, "forged seed: two seeds on one edge are refused");
  check(starts_with(duplicated.reason, "REBLEND_FACE_NOT_UNIQUE"),
        "forged seed: one rebuilt face may not answer for two seeds");

  // A seed on a CURVED edge: the fillet builds a torus, not a cylinder, and the
  // ladder's surface-type rung is the one that has to notice. Driven on the
  // half-round slab end, the only fixture here with an arc edge between two faces
  // that both survive a fillet.
  const Fixture cap = parallel_support_cap();
  if (cap.ok) {
    TopTools_IndexedMapOfShape cap_faces;
    TopExp::MapShapes(cap.body, TopAbs_FACE, cap_faces);
    TopoDS_Face lid;
    for (int i = 1; i <= cap_faces.Extent() && lid.IsNull(); ++i) {
      const TopoDS_Face candidate = TopoDS::Face(cap_faces(i));
      const BRepAdaptor_Surface surface(candidate);
      if (surface.GetType() == GeomAbs_Plane &&
          std::abs(surface.Plane().Axis().Direction().Z()) > 0.9)
        lid = candidate;
    }
    std::vector<TopoDS_Edge> arcs;
    for (const TopoDS_Edge &edge : shared_edges_of(cap.blend, lid)) {
      if (BRepAdaptor_Curve(edge).GetType() != GeomAbs_Line)
        arcs.push_back(edge);
    }
    check(!lid.IsNull() && arcs.size() == 1,
          "fixture: the slab end shares exactly one arc with its lid");
    if (!lid.IsNull() && arcs.size() == 1) {
      kf::SuppressedBlendSeed curved;
      curved.edge = arcs.front();
      curved.support_a = cap.blend;
      curved.support_b = lid;
      curved.radius = 1.0;
      curved.boundary_length = 10.0;
      curved.convexity = kf::BlendConvexity::Convex;
      const kf::ReblendResult torus = kf::reblend(cap.body, {curved}, 1.0);
      std::fprintf(stderr, "curved seed   -> ok=%d reason=%s\n", torus.ok ? 1 : 0,
                   torus.reason.c_str());
      check(!torus.ok, "forged seed: a curved seed edge is refused");
      check(starts_with(torus.reason, "REBLEND_NOT_CYLINDER"),
            "forged seed: the fillet builds a torus and the surface-type rung says so");
    }
  }
}

// The certification and suppression sides of the same sweep.
void suppression_refuses_forged_blends() {
  const Fixture fixture = convex_round();
  const kf::BlendCertification certified = kf::certify_blend(fixture.blend, fixture.body);
  check(fixture.ok && certified.ok, "fixture: the convex round certifies");
  if (!fixture.ok || !certified.ok)
    return;
  std::fprintf(stderr, "\n=== refusal: forged blends into suppression ===\n");

  const kf::SuppressionResult empty = kf::suppress_blends(fixture.body, {});
  std::fprintf(stderr, "empty set     -> ok=%d reason=%s\n", empty.ok ? 1 : 0,
               empty.reason.c_str());
  check(starts_with(empty.reason, "SUPPRESSION_INPUT_INVALID"),
        "forged blend: an empty set is refused at the input");

  kf::RecognizedBlend foreign = certified.blend;
  foreign.support_a = TopoDS::Face(
      TopExp_Explorer(ft::box(3.0), TopAbs_FACE).Current());
  const kf::SuppressionResult stranger = kf::suppress_blends(fixture.body, {foreign});
  std::fprintf(stderr, "foreign supp  -> ok=%d reason=%s\n", stranger.ok ? 1 : 0,
               stranger.reason.c_str());
  check(starts_with(stranger.reason, "SUPPRESSION_INPUT_INVALID"),
        "forged blend: a support from another body is refused at the input");

  // A support that IS the blend face. It passes input validation (it belongs to the
  // body) and then has no image to compare against, because it was removed.
  kf::RecognizedBlend self_supported = certified.blend;
  self_supported.support_b = certified.blend.face;
  const kf::SuppressionResult self = kf::suppress_blends(fixture.body, {self_supported});
  std::fprintf(stderr, "self support  -> ok=%d reason=%s\n", self.ok ? 1 : 0,
               self.reason.c_str());
  check(!self.ok, "forged blend: a blend supported by itself is refused");
  check(starts_with(self.reason, "SUPPRESSION_ANCESTRY_NOT_UNIQUE"),
        "forged blend: postcondition 4 refuses a support with no surviving image");
}

// The reproduction proof is a LADDER, and this is the case that walks it. The seed
// names the top cap instead of one of the two real supports; the rebuild itself
// succeeds (the seed edge is untouched), so nothing but the reproduction proof can
// tell that the rebuilt face is not the blend that was described.
//
// Measured by neutralizing each rung in turn — with the supports check off the
// analytic proof reds (`Failed`, the cap is perpendicular to the blend axis rather
// than tangent to it), and with the analytic proof off too the boundary-length check
// reds (the cap shares the blend's top ARC, pi*R/2 = 3.14159 mm, not a 10 mm line).
// Three independent gates, any one of which refuses on its own.
void refuses_a_seed_with_a_wrong_support() {
  const Fixture fixture = convex_round();
  check(fixture.ok, "fixture: the convex round builds");
  if (!fixture.ok)
    return;
  const kf::BlendCertification certified = kf::certify_blend(fixture.blend, fixture.body);
  const kf::SuppressionResult suppressed =
      kf::suppress_blends(fixture.body, {certified.blend});
  check(certified.ok && suppressed.ok, "fixture: the convex round suppresses");
  if (!certified.ok || !suppressed.ok)
    return;
  std::fprintf(stderr, "\n=== refusal: a seed naming the wrong support ===\n");

  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(suppressed.suppressed_shape, TopAbs_FACE, faces);
  TopoDS_Face cap;
  for (int i = 1; i <= faces.Extent() && cap.IsNull(); ++i) {
    const TopoDS_Face candidate = TopoDS::Face(faces(i));
    const BRepAdaptor_Surface surface(candidate);
    if (surface.GetType() == GeomAbs_Plane &&
        std::abs(surface.Plane().Axis().Direction().Z()) > 0.9)
      cap = candidate;
  }
  check(!cap.IsNull(), "fixture: the suppressed box has a horizontal cap face");
  if (cap.IsNull())
    return;

  std::vector<kf::SuppressedBlendSeed> seeds = suppressed.seed_edges;
  seeds.front().support_b = cap;
  const kf::ReblendResult wrong =
      kf::reblend(suppressed.suppressed_shape, seeds, certified.blend.radius);
  std::fprintf(stderr, "reblend -> ok=%d reason=%s\n", wrong.ok ? 1 : 0,
               wrong.reason.c_str());
  check(!wrong.ok, "wrong support: the reproduction proof refuses");
  check(wrong.shape.IsNull() && wrong.blend_faces.empty(),
        "wrong support: a refusal publishes nothing and mints no face");
  check(starts_with(wrong.reason, "REBLEND_SUPPORTS_MISMATCH"),
        "wrong support: the first rung of the reproduction ladder catches it");
}

} // namespace

int main() {
  const Fixture convex = convex_round();
  const RoundTrip convex_trip =
      identity_round_trip("convex round (box 10^3, R2)", convex, kf::BlendConvexity::Convex,
                          1000.0 - kCornerVolume, 1000.0);

  const Fixture concave = concave_fillet();
  const RoundTrip concave_trip =
      identity_round_trip("concave fillet (L-prism, R2)", concave, kf::BlendConvexity::Concave,
                          640.0 + kCornerVolume, 640.0);

  std::fprintf(stderr, "\n=== characterized two-stage tolerance cap ===\n");
  tolerance_cap_is_characterized("convex", convex, convex_trip);
  tolerance_cap_is_characterized("concave", concave, concave_trip);

  cylinder_supported_round_trip();

  refuses_a_blend_whose_supports_never_meet();
  refuses_a_face_that_is_not_a_blend();
  postconditions_reject_a_misdescribed_blend();
  refuses_mixed_radii();
  suppression_refuses_forged_blends();
  reblend_refuses_forged_seeds();
  refuses_a_seed_with_a_wrong_support();
  refuses_a_partially_trimmed_round();

  std::fprintf(stderr, "\nblend reconstruction: %d checks run, %d failed\n", checks_run,
               failures);
  return failures;
}
