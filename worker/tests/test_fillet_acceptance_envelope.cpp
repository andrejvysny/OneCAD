// WP-G: the fillet-acceptance envelope, promoted from a measuring probe to an
// asserting ctest. Encodes the NEW acceptance rule (docs/design "Rule" 1-5,
// scratchpad wpg-design.md):
//   * cases 1-2 are APPROXIMATED (ChFi3d B-spline) blends. The shipped kernel
//     refuses them FILLET_SEMANTIC_CHECK_FAILED; the new rule must accept them
//     with a `warning` diagnostic FILLET_BLEND_APPROXIMATED and evidence
//     `blendSurfaceClass == "approximated"`. RED today, by design.
//     This file also carries the MEASURED numbers SCHEMA §14 records for those
//     two cases — the protocol fixture leaves them `$any` because NDJSON
//     compares numbers exactly, so the pins have to live here.
//   * cases 3-5 are ANALYTIC blends and must stay accepted, with no such
//     warning, and (once the evidence field ships) `blendSurfaceClass ==
//     "analytic"`.
//   * case 5b is the periodic seam edge itself: OCCT never accepts it as a
//     fillet contour; the refusal code must stay FILLET_CONTOUR_INVALID.
//
// A "coarseness ceiling" sixth case (rule item 4, FILLET_BLEND_TOO_COARSE) is
// deliberately NOT constructed here: no probed geometry drives the adaptive
// budget past `min(r/100, 0.05mm)` without also tripping the profile-error
// gate for an unrelated reason, so a reliable case is not achievable from
// primitives alone. Exercise that refusal is left to G2 (implementer, with
// the kernel change in hand) or a targeted unit test at that point.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeHalfSpace.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_List.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Pln.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/BlendEvidence.h"
#include "kernel/fillet/FilletBuilder.h"
#include "session/PlanExecutor.h"

namespace ft = onecad::tests::fillet;
namespace kf = onecad::kernel::fillet;
namespace session = onecad::session;

namespace {

int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  ++failures;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
}

std::string num(double value) {
  char buffer[64];
  std::snprintf(buffer, sizeof(buffer), "%.6g", value);
  return buffer;
}

// ---------------------------------------------------------------- topology
// (identical selection helpers to the WP-G probe this file replaces)

using EdgeFaces = NCollection_IndexedDataMap<
    TopoDS_Shape, NCollection_List<TopoDS_Shape>, TopTools_ShapeMapHasher>;

EdgeFaces edge_faces(const TopoDS_Shape &shape) {
  EdgeFaces map;
  TopExp::MapShapesAndAncestors(shape, TopAbs_EDGE, TopAbs_FACE, map);
  return map;
}

std::vector<TopoDS_Face> neighbours(const EdgeFaces &map,
                                    const TopoDS_Edge &edge) {
  std::vector<TopoDS_Face> out;
  if (!map.Contains(edge))
    return out;
  for (const TopoDS_Shape &face : map.FindFromKey(edge))
    out.push_back(TopoDS::Face(face));
  return out;
}

bool is_cylinder(const TopoDS_Face &face, double radius) {
  BRepAdaptor_Surface surface(face);
  return surface.GetType() == GeomAbs_Cylinder &&
         std::abs(surface.Cylinder().Radius() - radius) <= 1.0e-6;
}

bool is_cone(const TopoDS_Face &face) {
  return BRepAdaptor_Surface(face).GetType() == GeomAbs_Cone;
}

bool is_plane(const TopoDS_Face &face, bool z_normal) {
  BRepAdaptor_Surface surface(face);
  if (surface.GetType() != GeomAbs_Plane)
    return false;
  const double axial = std::abs(surface.Plane().Axis().Direction().Z());
  return z_normal ? axial > 1.0 - 1.0e-9 : axial < 1.0 - 1.0e-6;
}

int valence(const EdgeFaces &map, const gp_Pnt &point) {
  int count = 0;
  for (int i = 1; i <= map.Extent(); ++i) {
    const TopoDS_Edge edge = TopoDS::Edge(map.FindKey(i));
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    if ((!first.IsNull() && BRep_Tool::Pnt(first).Distance(point) <= 1.0e-6) ||
        (!last.IsNull() && BRep_Tool::Pnt(last).Distance(point) <= 1.0e-6))
      ++count;
  }
  return count;
}

// ---------------------------------------------------------------- drivers

// Direct kernel path: `kf::FilletBuilder::build`. Used wherever the op path
// resolution ladder is not the thing under test (case 5, case 5b).
kf::FilletBuildResult build_direct(const TopoDS_Shape &body,
                                   const std::vector<TopoDS_Edge> &edges,
                                   double radius) {
  kf::FilletBuildResult built;
  try {
    kf::FilletBuilder builder(body, ft::resolved(body, edges), radius);
    built = builder.build();
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    built.ok = false;
    built.error_code = "THREW";
    built.message = message ? message : "OCCT";
  }
  return built;
}

// The WHOLE shipped op path (resolution ladder + FilletChamferOp), same as the
// probe's `op_path_status` but returning the full CandidateResult so
// assertions can inspect `diagnostics[]` and evidence, not just a summary.
session::CandidateResult run_op_path(const TopoDS_Shape &body,
                                     const std::vector<TopoDS_Edge> &edges,
                                     double radius) {
  session::ScratchJob job;
  job.bodies.create("body_box", "op_box", body);
  std::string last_sketch;
  try {
    return session::execute_candidate_op(job, ft::op(body, edges, radius),
                                         "op_fillet", last_sketch,
                                         onecad::CancelToken{});
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    session::CandidateResult result;
    result.status = session::CandidateResult::Status::Failed;
    result.error_code = "THREW";
    result.error_message = message ? message : "OCCT";
    return result;
  }
}

const nlohmann::json *find_diagnostic(const session::CandidateResult &candidate,
                                      const std::string &code) {
  for (const nlohmann::json &diagnostic : candidate.diagnostics) {
    if (diagnostic.value("code", std::string()) == code)
      return &diagnostic;
  }
  return nullptr;
}

const char *status_name(session::CandidateResult::Status status) {
  static const char *kNames[] = {"Ok", "Failed", "Unsupported", "NeedsRepair",
                                 "Cancelled"};
  return kNames[static_cast<int>(status)];
}

// Asserts the case-1/2 (APPROXIMATED) shape: op path publishes Ok and the step
// carries the FILLET_BLEND_APPROXIMATED warning with the evidence the design
// promises. Every check is independent so a partially-landed kernel change
// still reports which sub-claim is still red.
void check_approximated_accepted(const char *label, const TopoDS_Shape &body,
                                 const std::vector<TopoDS_Edge> &edges,
                                 double radius, double expected_section,
                                 double expected_tangency) {
  const session::CandidateResult candidate = run_op_path(body, edges, radius);
  const std::string where = std::string(label) + ": ";
  check(candidate.status == session::CandidateResult::Status::Ok,
       where + "op path must publish Ok for an approximated blend, got " +
           status_name(candidate.status) + "/" + candidate.error_code +
           (candidate.error_message.empty() ? "" : (":" + candidate.error_message)));
  if (candidate.status != session::CandidateResult::Status::Ok)
    return;

  const nlohmann::json *warning =
      find_diagnostic(candidate, "FILLET_BLEND_APPROXIMATED");
  check(warning != nullptr,
       where + "expected a FILLET_BLEND_APPROXIMATED diagnostic on the "
               "published step");
  if (!warning)
    return;
  check(warning->value("severity", std::string()) == "warning",
       where + "FILLET_BLEND_APPROXIMATED must be severity=warning, got " +
           warning->value("severity", std::string("<missing>")));

  // SCHEMA §7.2: ONE vocabulary. The evidence IS the `blendEvidence` object a
  // refusal already emits, plus the class keys and the two counts. There are no
  // alias spellings, so these names are the contract.
  const nlohmann::json evidence = warning->value(
      "evidence", nlohmann::json::object());
  check(evidence.contains("blendSurfaceClass") &&
           evidence["blendSurfaceClass"] == "approximated",
       where + "evidence.blendSurfaceClass must be \"approximated\"");
  for (const char *key : {"approximationTolerance", "maximumSectionRadiusError",
                          "allowedSectionRadiusError", "maximumTangencyRadians",
                          "allowedTangencyRadians", "maxContourVertexValence",
                          "approximatedContours", "approximatedBlendFaces"}) {
    check(evidence.contains(key) && evidence[key].is_number() &&
             std::isfinite(evidence[key].get<double>()),
         where + "evidence." + key + " must be a finite number");
  }
  // The alias spellings the §7.2 draft briefly carried must NOT be here: two
  // names for one number is how the two tracks drift apart.
  for (const char *alias : {"maxProfileError", "allowedProfileError",
                            "maxTangencyRadians", "contour", "blendFaces"}) {
    check(!evidence.contains(alias),
         where + "evidence must not carry the alias key " + alias);
  }
  if (!evidence.contains("maximumSectionRadiusError") ||
      !evidence.contains("allowedSectionRadiusError") ||
      !evidence.contains("maximumTangencyRadians") ||
      !evidence.contains("allowedTangencyRadians"))
    return;

  const double section = evidence["maximumSectionRadiusError"].get<double>();
  const double allowed_section = evidence["allowedSectionRadiusError"].get<double>();
  const double tangency = evidence["maximumTangencyRadians"].get<double>();
  const double allowed_tangency = evidence["allowedTangencyRadians"].get<double>();
  check(section <= allowed_section,
       where + "maximumSectionRadiusError (" + num(section) +
           ") must be <= allowedSectionRadiusError (" + num(allowed_section) + ")");
  check(tangency <= allowed_tangency,
       where + "maximumTangencyRadians (" + num(tangency) +
           ") must be <= allowedTangencyRadians (" + num(allowed_tangency) + ")");

  // The MEASURED numbers SCHEMA §14 records, asserted rather than merely
  // bounded. "Inside the budget" would still pass if OCCT's blend quietly got
  // an order of magnitude worse and the budget were widened to match; these
  // pin what the kernel actually produces. Bands are wide enough to absorb a
  // different platform's arithmetic (10 % on the section residual, 20 % on the
  // G1 error, which is the noisier of the two) and far too narrow to absorb a
  // change of kind.
  std::fprintf(stderr,
              "MEASURED %-28s maximumSectionRadiusError=%.17g "
              "maximumTangencyRadians=%.17g approximationTolerance=%.17g\n",
              label, section, tangency,
              evidence["approximationTolerance"].get<double>());
  check(std::abs(section - expected_section) <= 0.10 * expected_section,
       where + "maximumSectionRadiusError " + num(section) + " is outside 10% of the "
               "measured " + num(expected_section));
  check(std::abs(tangency - expected_tangency) <= 0.20 * expected_tangency,
       where + "maximumTangencyRadians " + num(tangency) + " is outside 20% of the "
               "measured " + num(expected_tangency));
}

// Asserts the case-3/4 (ANALYTIC) shape via the op path: Ok, no
// FILLET_BLEND_APPROXIMATED warning, and — once evidence exposes it —
// blendSurfaceClass == "analytic".
void check_analytic_accepted_op_path(const char *label, const TopoDS_Shape &body,
                                     const std::vector<TopoDS_Edge> &edges,
                                     double radius) {
  const session::CandidateResult candidate = run_op_path(body, edges, radius);
  const std::string where = std::string(label) + ": ";
  check(candidate.status == session::CandidateResult::Status::Ok,
       where + "op path must publish Ok for an analytic blend, got " +
           status_name(candidate.status) + "/" + candidate.error_code +
           (candidate.error_message.empty() ? "" : (":" + candidate.error_message)));
  check(find_diagnostic(candidate, "FILLET_BLEND_APPROXIMATED") == nullptr,
       where + "an analytic blend must not carry FILLET_BLEND_APPROXIMATED");
  for (const nlohmann::json &diagnostic : candidate.diagnostics) {
    if (!diagnostic.contains("evidence"))
      continue;
    const nlohmann::json &evidence = diagnostic["evidence"];
    if (evidence.contains("blendSurfaceClass"))
      check(evidence["blendSurfaceClass"] == "analytic",
           where + "evidence.blendSurfaceClass must be \"analytic\" when present");
  }
}

// Case 5 is driven through `FilletBuilder` directly, per the brief: the op
// path's resolution ladder returns NeedsRepair for this edge selection (a
// ref-construction gap in the probe/test's own body wiring, not the subject
// of this test), so asserting through the op path would test the wrong
// thing. Confirmed still true on this repo's `session::execute_candidate_op`
// contour resolution as of writing.
void check_analytic_accepted_direct(const char *label, const TopoDS_Shape &body,
                                    const std::vector<TopoDS_Edge> &edges,
                                    double radius) {
  const kf::FilletBuildResult built = build_direct(body, edges, radius);
  const std::string where = std::string(label) + ": ";
  check(built.ok, where + "FilletBuilder::build must accept an analytic blend, got " +
                      built.error_code +
                      (built.message.empty() ? "" : (":" + built.message)));
  // Asserted on the builder's OWN evidence, not inside an `if (contains)` over
  // a diagnostics list that is empty on success — that shape passes whether or
  // not the classifier works.
  check(built.fillet_evidence.surface_class == kf::BlendSurfaceClass::Analytic,
        where + "evidence.surface_class must be Analytic, got \"" +
            kf::blend_surface_class_name(built.fillet_evidence.surface_class) +
            "\"");
  check(built.fillet_evidence.approximation_tolerance == 0.0,
        where + "an analytic blend reports approximationTolerance 0");
  bool saw_warning = false;
  // FilletBuildResult carries no on-success diagnostics field beyond
  // `diagnostics` (populated on failure today); once G2 wires a success-path
  // warning through, it will still land in `built.diagnostics`.
  for (const auto &diagnostic : built.diagnostics) {
    if (diagnostic.code == "FILLET_BLEND_APPROXIMATED")
      saw_warning = true;
  }
  check(!saw_warning, where + "an analytic blend must not carry FILLET_BLEND_APPROXIMATED");
}

void check_contour_refused(const char *label, const TopoDS_Shape &body,
                           const std::vector<TopoDS_Edge> &edges, double radius,
                           const char *expected_code) {
  const kf::FilletBuildResult built = build_direct(body, edges, radius);
  const std::string where = std::string(label) + ": ";
  check(!built.ok, where + "expected a refusal, but the build was accepted");
  const std::string code =
      built.diagnostics.empty() ? built.error_code : built.diagnostics.front().code;
  check(code == expected_code,
       where + "expected refusal code " + expected_code + ", got " + code);
}

// ---------------------------------------------------------------- the cases
// (geometry construction copied verbatim from the WP-G scratchpad probe —
// this file supersedes it as the tracked, asserting ctest.)

// The shaft+boss tee, shared by case 1 and case 6.
TopoDS_Shape cylinder_tee() {
  const TopoDS_Shape shaft =
      BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(-30, 0, 0), gp_Dir(1, 0, 0)), 20.0,
                               60.0)
          .Shape();
  const TopoDS_Shape boss =
      BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 10.0,
                               40.0)
          .Shape();
  return BRepAlgoAPI_Fuse(shaft, boss).Shape();
}

std::vector<TopoDS_Edge> tee_seam_edges(const TopoDS_Shape &body) {
  const EdgeFaces map = edge_faces(body);
  std::vector<TopoDS_Edge> picked;
  for (const TopoDS_Edge &edge : ft::all_edges(body)) {
    const std::vector<TopoDS_Face> faces = neighbours(map, edge);
    if (faces.size() != 2)
      continue;
    if ((is_cylinder(faces[0], 20.0) && is_cylinder(faces[1], 10.0)) ||
        (is_cylinder(faces[1], 20.0) && is_cylinder(faces[0], 10.0)))
      picked.push_back(edge);
  }
  return picked;
}

// The shaft's own END RIM (cylinder/plane at x = +-30): an ANALYTIC contour on
// the same body as the approximated seam.
std::vector<TopoDS_Edge> tee_end_rim_edges(const TopoDS_Shape &body) {
  const EdgeFaces map = edge_faces(body);
  std::vector<TopoDS_Edge> picked;
  for (const TopoDS_Edge &edge : ft::all_edges(body)) {
    const std::vector<TopoDS_Face> faces = neighbours(map, edge);
    if (faces.size() != 2 || faces[0].IsSame(faces[1]))
      continue;
    const bool cylinder_plane =
        (is_cylinder(faces[0], 20.0) && !is_cylinder(faces[1], 20.0) &&
         BRepAdaptor_Surface(faces[1]).GetType() == GeomAbs_Plane) ||
        (is_cylinder(faces[1], 20.0) && !is_cylinder(faces[0], 20.0) &&
         BRepAdaptor_Surface(faces[0]).GetType() == GeomAbs_Plane);
    if (!cylinder_plane)
      continue;
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    if (first.IsNull() || std::abs(BRep_Tool::Pnt(first).X() - 30.0) > 1.0e-6)
      continue;
    picked.push_back(edge);
  }
  return picked;
}

// Case 6 — SCHEMA §7.3 "Classification, measurement and budget are all PER
// CONTOUR". One op over BOTH the approximated tee seam and the analytic end
// rim. The rim must be judged at the EXACT budget, not at the seam's 0.02 mm
// one; the observable proof is that exactly ONE contour needed the adaptive
// budget, and that the rim ALONE publishes with no warning at all.
void case6_mixed_classes_in_one_op() {
  const TopoDS_Shape body = cylinder_tee();
  const std::vector<TopoDS_Edge> seam = tee_seam_edges(body);
  const std::vector<TopoDS_Edge> rim = tee_end_rim_edges(body);
  check(!seam.empty() && !rim.empty(),
        "case6: fixture needs both the tee seam and the shaft end rim");
  if (seam.empty() || rim.empty())
    return;

  // The analytic contour ALONE: exact budgets, no warning, whatever else is on
  // the body.
  const kf::FilletBuildResult rim_only = build_direct(body, rim, 2.0);
  check(rim_only.ok, std::string("case6 rim-alone: must publish, got ") +
                         rim_only.error_code + ":" + rim_only.message);
  if (rim_only.ok) {
    check(rim_only.fillet_evidence.surface_class == kf::BlendSurfaceClass::Analytic,
          "case6 rim-alone: the end rim is an ANALYTIC blend");
    check(rim_only.diagnostics.empty(),
          "case6 rim-alone: an analytic blend carries no diagnostic");
  }

  std::vector<TopoDS_Edge> both = seam;
  both.insert(both.end(), rim.begin(), rim.end());
  const session::CandidateResult candidate = run_op_path(body, both, 2.0);
  check(candidate.status == session::CandidateResult::Status::Ok,
        std::string("case6 seam+rim: must publish, got ") +
            status_name(candidate.status) + "/" + candidate.error_code +
            (candidate.error_message.empty() ? "" : (":" + candidate.error_message)));
  if (candidate.status != session::CandidateResult::Status::Ok)
    return;
  const nlohmann::json *warning =
      find_diagnostic(candidate, "FILLET_BLEND_APPROXIMATED");
  check(warning != nullptr,
        "case6 seam+rim: the approximated seam still makes the step warn");
  if (!warning)
    return;
  const nlohmann::json evidence =
      warning->value("evidence", nlohmann::json::object());
  check(evidence.value("approximatedContours", -1) == 1,
        "case6 seam+rim: exactly ONE contour needed the adaptive budget — the "
        "analytic rim was judged at the exact budget, not the seam's; got " +
            evidence.value("approximatedContours", nlohmann::json(-1)).dump());
}

// Ø40 shaft along X fused with a Ø20 boss along +Z: a cylinder/cylinder T
// intersection. The fillet on that seam is an APPROXIMATED (ChFi3d B-spline)
// blend on the shipped kernel.
void case1_cylinder_tee() {
  const TopoDS_Shape shaft =
      BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(-30, 0, 0), gp_Dir(1, 0, 0)), 20.0,
                               60.0)
          .Shape();
  const TopoDS_Shape boss =
      BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 10.0,
                               40.0)
          .Shape();
  const TopoDS_Shape body = BRepAlgoAPI_Fuse(shaft, boss).Shape();
  const EdgeFaces map = edge_faces(body);
  std::vector<TopoDS_Edge> picked;
  for (const TopoDS_Edge &edge : ft::all_edges(body)) {
    const std::vector<TopoDS_Face> faces = neighbours(map, edge);
    if (faces.size() != 2)
      continue;
    if ((is_cylinder(faces[0], 20.0) && is_cylinder(faces[1], 10.0)) ||
        (is_cylinder(faces[1], 20.0) && is_cylinder(faces[0], 10.0)))
      picked.push_back(edge);
  }
  // MEASURED on OCCT 8.0.1 (SCHEMA §14): residual 1.97e-3 mm against the
  // 0.02 mm budget, G1 error 1.24e-5 rad against 2.0e-4.
  check_approximated_accepted("case1 cyl-cyl-tee", body, picked, 2.0, 1.97e-3,
                              1.24e-5);
}

// Ø20 cylinder cut by a plane 30° off XY through mid-height: an oblique
// elliptical rim, also an APPROXIMATED blend.
void case2_oblique_ellipse() {
  const TopoDS_Shape cylinder =
      BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 10.0,
                               30.0)
          .Shape();
  const double tilt = 30.0 * M_PI / 180.0;
  const gp_Pln plane(gp_Pnt(0, 0, 15),
                     gp_Dir(std::sin(tilt), 0.0, std::cos(tilt)));
  const TopoDS_Face cut_face =
      BRepBuilderAPI_MakeFace(plane, -100.0, 100.0, -100.0, 100.0).Face();
  const TopoDS_Shape half =
      BRepPrimAPI_MakeHalfSpace(cut_face, gp_Pnt(0, 0, 200)).Solid();
  const TopoDS_Shape body = BRepAlgoAPI_Cut(cylinder, half).Shape();
  const EdgeFaces map = edge_faces(body);
  std::vector<TopoDS_Edge> picked;
  for (const TopoDS_Edge &edge : ft::all_edges(body)) {
    const std::vector<TopoDS_Face> faces = neighbours(map, edge);
    if (faces.size() != 2)
      continue;
    if ((is_cylinder(faces[0], 10.0) && is_plane(faces[1], false)) ||
        (is_cylinder(faces[1], 10.0) && is_plane(faces[0], false)))
      picked.push_back(edge);
  }
  // MEASURED on OCCT 8.0.1 (SCHEMA §14): residual 2.87e-3 mm against the
  // 0.01 mm budget, G1 error 4.94e-6 rad against 2.54e-4.
  check_approximated_accepted("case2 oblique-elliptical-rim", body, picked, 1.0,
                              2.87e-3, 4.94e-6);
}

// Frustum r1=10 r2=5 h=10 fused onto a 40x40x10 box top: the cone/plane base
// rim fillet is ANALYTIC.
void case3_cone_rim() {
  const TopoDS_Shape box =
      BRepPrimAPI_MakeBox(gp_Pnt(-20, -20, 0), 40.0, 40.0, 10.0).Shape();
  const TopoDS_Shape cone =
      BRepPrimAPI_MakeCone(gp_Ax2(gp_Pnt(0, 0, 10), gp_Dir(0, 0, 1)), 10.0, 5.0,
                           10.0)
          .Shape();
  const TopoDS_Shape body = BRepAlgoAPI_Fuse(box, cone).Shape();
  const EdgeFaces map = edge_faces(body);
  std::vector<TopoDS_Edge> picked;
  for (const TopoDS_Edge &edge : ft::all_edges(body)) {
    const std::vector<TopoDS_Face> faces = neighbours(map, edge);
    if (faces.size() != 2)
      continue;
    if ((is_cone(faces[0]) && is_plane(faces[1], true)) ||
        (is_cone(faces[1]) && is_plane(faces[0], true)))
      picked.push_back(edge);
  }
  check_analytic_accepted_op_path("case3 cone-base-rim", body, picked, 1.0);
}

// 40x40x5 plate, two crossing 4x10 ribs; one rib top edge into the
// valence-5 crossing vertex. ANALYTIC (plane/plane) blend.
void case4_valence4_rib() {
  const TopoDS_Shape plate =
      BRepPrimAPI_MakeBox(gp_Pnt(-20, -20, 0), 40.0, 40.0, 5.0).Shape();
  const TopoDS_Shape rib_x =
      BRepPrimAPI_MakeBox(gp_Pnt(-20, -2, 5), 40.0, 4.0, 10.0).Shape();
  const TopoDS_Shape rib_y =
      BRepPrimAPI_MakeBox(gp_Pnt(-2, -20, 5), 4.0, 40.0, 10.0).Shape();
  const TopoDS_Shape body =
      BRepAlgoAPI_Fuse(BRepAlgoAPI_Fuse(plate, rib_x).Shape(), rib_y).Shape();
  const EdgeFaces map = edge_faces(body);

  TopoDS_Edge best;
  int best_valence = 0;
  for (const TopoDS_Edge &edge : ft::all_edges(body)) {
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    if (first.IsNull() || last.IsNull())
      continue;
    const gp_Pnt a = BRep_Tool::Pnt(first);
    const gp_Pnt b = BRep_Tool::Pnt(last);
    // Top long edge of the X rib: z = 15, |y| = 2, running along X.
    if (std::abs(a.Z() - 15.0) > 1.0e-6 || std::abs(b.Z() - 15.0) > 1.0e-6)
      continue;
    if (std::abs(std::abs(a.Y()) - 2.0) > 1.0e-6 ||
        std::abs(std::abs(b.Y()) - 2.0) > 1.0e-6)
      continue;
    if (std::abs(a.X() - b.X()) < 1.0e-6)
      continue;
    const std::vector<TopoDS_Face> faces = neighbours(map, edge);
    if (faces.size() != 2)
      continue;
    BRepAdaptor_Surface left(faces[0]);
    BRepAdaptor_Surface right(faces[1]);
    if (left.GetType() != GeomAbs_Plane || right.GetType() != GeomAbs_Plane)
      continue;
    const double dihedral = left.Plane().Axis().Direction().Angle(
        right.Plane().Axis().Direction());
    if (dihedral < 1.0e-6) // coplanar fuse seam, not a real convex edge
      continue;
    const gp_Pnt corner = std::abs(a.X()) < std::abs(b.X()) ? a : b;
    if (std::abs(std::abs(corner.X()) - 2.0) > 1.0e-6)
      continue;
    const int count = valence(map, corner);
    if (count > best_valence) {
      best_valence = count;
      best = edge;
    }
  }
  std::vector<TopoDS_Edge> picked;
  if (!best.IsNull())
    picked.push_back(best);
  check_analytic_accepted_op_path("case4 valence5-rib-corner", body, picked, 1.0);
}

// Ø20x20 boss on a 40x40x5 plate: top circular edge (ANALYTIC, driven direct
// per the brief) and the periodic seam edge itself (case 5b, must stay
// FILLET_CONTOUR_INVALID).
void case5_cylinder_seam() {
  const TopoDS_Shape plate =
      BRepPrimAPI_MakeBox(gp_Pnt(-20, -20, 0), 40.0, 40.0, 5.0).Shape();
  const TopoDS_Shape boss =
      BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, 5), gp_Dir(0, 0, 1)), 10.0,
                               20.0)
          .Shape();
  const TopoDS_Shape body = BRepAlgoAPI_Fuse(plate, boss).Shape();
  const EdgeFaces map = edge_faces(body);
  std::vector<TopoDS_Edge> top;
  std::vector<TopoDS_Edge> seam;
  for (const TopoDS_Edge &edge : ft::all_edges(body)) {
    const std::vector<TopoDS_Face> faces = neighbours(map, edge);
    if (faces.size() != 2)
      continue;
    if (faces[0].IsSame(faces[1])) {
      if (is_cylinder(faces[0], 10.0))
        seam.push_back(edge);
      continue;
    }
    const bool cylinder_plane =
        (is_cylinder(faces[0], 10.0) && is_plane(faces[1], true)) ||
        (is_cylinder(faces[1], 10.0) && is_plane(faces[0], true));
    if (!cylinder_plane)
      continue;
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    if (first.IsNull() || std::abs(BRep_Tool::Pnt(first).Z() - 25.0) > 1.0e-6)
      continue;
    top.push_back(edge);
  }
  check_analytic_accepted_direct("case5 cyl-top-edge-near-seam", body, top, 1.0);
  check_contour_refused("case5b cyl-seam-edge-itself", body, seam, 1.0,
                        "FILLET_CONTOUR_INVALID");
}

} // namespace

int main() {
  struct Case {
    const char *name;
    void (*run)();
  };
  const Case cases[] = {{"1", &case1_cylinder_tee},
                        {"2", &case2_oblique_ellipse},
                        {"3", &case3_cone_rim},
                        {"4", &case4_valence4_rib},
                        {"5", &case5_cylinder_seam},
                        {"6", &case6_mixed_classes_in_one_op}};
  for (const Case &entry : cases) {
    try {
      entry.run();
    } catch (const Standard_Failure &failure) {
      const char *message = failure.GetMessageString();
      ++failures;
      std::fprintf(stderr, "FAIL: case %s setup threw: %s\n", entry.name,
                  message ? message : "OCCT");
    }
  }
  if (failures > 0)
    std::fprintf(stderr, "%d assertion(s) failed\n", failures);
  return failures == 0 ? 0 : 1;
}
