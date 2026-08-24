// test_wp6_ops.cpp — W-WP6 new-op numerics (scope B): Fillet / Chamfer / Revolve,
// in-process via the op executors (real OCCT). Covers geometry results, the radius
// guard, and the "edge ref no longer resolves ⇒ NeedsRepair, never a wrong bind".
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepGProp.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <GProp_GProps.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/FilletChamferOp.h"
#include "ops/OpTypes.h"
#include "ops/RevolveOp.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
using onecad::session::BodyStore;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.4f want %.4f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

TopoDS_Shape edge_by_center(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(shape, TopAbs_EDGE, edges);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= edges.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(edges(i));
        const double dx = d.center.X() - cx, dy = d.center.Y() - cy, dz = d.center.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) { best_d2 = d2; best = edges(i); }
    }
    return best;
}

std::size_t face_count(const TopoDS_Shape& s) {
    return onecad::session::compute_shape_metrics(s).face_count;
}

// An edge input ref (semantic ref) carrying the edge's frozen descriptor + anchor.
json edge_input(const std::string& body_id, const std::string& elem_id, const TopoDS_Shape& edge,
                double ax, double ay, double az) {
    return json{{"primary", {{"bodyId", body_id}, {"elementId", elem_id}, {"kind", "edge"}}},
                {"intent", {{"kind", "edge"},
                            {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                               em::ElementMapPartition::describe(edge))}}},
                {"anchor", {{"worldPoint", {ax, ay, az}}}}};
}

// A minimal OpContext over `bodies` (no sketches).
struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, em::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

// --- Fillet a single vertical box edge → +1 rolled face, small volume loss. ---
void test_fillet_edge() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();  // vol 1000
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;

    const TopoDS_Shape edge = edge_by_center(box, 0, 0, 5);  // vertical edge at (0,0)
    json op = {{"opType", "Fillet"}, {"opId", "opf"},
               {"inputs", json::array({edge_input("body_1", "el_e", edge, 0, 0, 5)})},
               {"params", {{"mode", "Fillet"}, {"radius", 1.0}, {"edgeIds", json::array({"e:x"})}}}};

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_fillet(ctx, op, "opf");
    check(oc.status == ops::OpOutcome::Status::Ok, "fillet: Ok");
    check(oc.needs_repair.empty(), "fillet: no NeedsRepair (edge resolves)");
    check(oc.body_events.size() == 1 && oc.body_events[0].kind == "modified", "fillet: body modified");
    const double v = onecad::session::shape_volume(bodies.get("body_1")->geom);
    check(v < 1000.0 && v > 990.0, "fillet: small volume loss (rounded corner)");
    check(face_count(bodies.get("body_1")->geom) == 7, "fillet: 6 + 1 rolled face = 7");
}

// --- Chamfer the same edge → +1 flat face, larger volume loss than fillet. ---
void test_chamfer_edge() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;

    const TopoDS_Shape edge = edge_by_center(box, 0, 0, 5);
    json op = {{"opType", "Chamfer"}, {"opId", "opc"},
               {"inputs", json::array({edge_input("body_1", "el_e", edge, 0, 0, 5)})},
               {"params", {{"mode", "Chamfer"}, {"radius", 1.0}, {"edgeIds", json::array({"e:x"})}}}};

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_chamfer(ctx, op, "opc");
    check(oc.status == ops::OpOutcome::Status::Ok, "chamfer: Ok");
    check(oc.needs_repair.empty(), "chamfer: no NeedsRepair");
    const double v = onecad::session::shape_volume(bodies.get("body_1")->geom);
    // Chamfer removes a right-triangle prism ~0.5·1·1·10 = 5 mm^3.
    check_near(v, 995.0, 1.0, "chamfer: volume ~995 (flat cut)");
    check(face_count(bodies.get("body_1")->geom) == 7, "chamfer: 6 + 1 flat face = 7");
}

// ── two-distance chamfer (SCHEMA §7.3, 2026-08-03 — WP-C T2a) ────────────────

// The result of running an asymmetric chamfer on the (0,0) vertical edge of a
// 10mm box: volume + centroid. The CENTROID is the signature that discriminates
// the reference-face choice — the removed wedge is asymmetric about the x==y
// diagonal, so putting `radius` on the other adjacent face mirrors it.
struct ChamferRun {
    double volume = 0.0;
    gp_Pnt centre;
    std::size_t faces = 0;
    bool ok = false;
    std::string error_code;
    std::string error_message;
};

// Run one chamfer on the (0,0) vertical edge of a 10 mm box. `mode_params` carries
// `radius` plus whichever mode field is under test (none / `distance2` / `angleDeg`),
// so a case reads as "which SHAPE of params did I hand it". The measurements are
// taken unconditionally: on a refusal they are the UNTOUCHED box, which is what
// "recoverable, session intact" means.
ChamferRun run_chamfer(const json& mode_params, const std::string& op_id) {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();  // [0,10]^3
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;

    const TopoDS_Shape edge = edge_by_center(box, 0, 0, 5);
    json params = json{{"mode", "Chamfer"}, {"edgeIds", json::array({"e:x"})}};
    params.update(mode_params);
    json op = {{"opType", "Chamfer"}, {"opId", op_id},
               {"inputs", json::array({edge_input("body_1", "el_e", edge, 0, 0, 5)})},
               {"params", params}};

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_chamfer(ctx, op, op_id);
    ChamferRun r;
    r.ok = oc.status == ops::OpOutcome::Status::Ok && oc.needs_repair.empty();
    r.error_code = oc.error_code;
    r.error_message = oc.error_message;
    const TopoDS_Shape& out = bodies.get("body_1")->geom;
    GProp_GProps props;
    BRepGProp::VolumeProperties(out, props);
    r.volume = props.Mass();
    r.centre = props.CentreOfMass();
    r.faces = face_count(out);
    return r;
}

ChamferRun run_two_distance_chamfer(double d1, double d2) {
    return run_chamfer(json{{"radius", d1}, {"distance2", d2}}, "opc2");
}

// Whether the SCHEMA §7.3 reference face (the adjacent face with the SMALLER
// resolved face ordinal — the same 1-based `TopExp::MapShapes` index a TopoKey
// "f:N" is built from) of the box's (0,0) vertical edge is the x==0 plane.
// Recomputed HERE from OCCT rather than assumed, so the expectation below states
// the RULE and not one build's happenstance face order.
bool reference_face_is_x0(const TopoDS_Shape& box, const TopoDS_Shape& edge) {
    TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
    TopExp::MapShapesAndAncestors(box, TopAbs_EDGE, TopAbs_FACE, edge_faces);
    TopTools_IndexedMapOfShape face_map;
    TopExp::MapShapes(box, TopAbs_FACE, face_map);
    const TopTools_ListOfShape& faces = edge_faces(edge_faces.FindIndex(edge));
    TopoDS_Shape best;
    int best_ordinal = 0;
    for (TopTools_ListOfShape::Iterator it(faces); it.More(); it.Next()) {
        const int ord = face_map.FindIndex(it.Value());
        if (ord > 0 && (best_ordinal == 0 || ord < best_ordinal)) {
            best_ordinal = ord;
            best = it.Value();
        }
    }
    // The x==0 face's centre is (0,5,5); the y==0 face's is (5,0,5).
    return em::ElementMapPartition::describe(best).center.X() < 1e-9;
}

// --- Asymmetric legs remove the EXACT analytic wedge d1·d2/2·L, with `radius`
//     landing on the deterministic reference face. ---
void test_chamfer_two_distance() {
    const double d1 = 1.0, d2 = 2.5, len = 10.0;
    const ChamferRun r = run_two_distance_chamfer(d1, d2);
    check(r.ok, "chamfer2: Ok, no NeedsRepair");
    if (!r.ok) return;

    // Removed = right-triangle prism of legs d1,d2 swept the edge length.
    const double removed = 0.5 * d1 * d2 * len;
    check_near(r.volume, 1000.0 - removed, 1e-6, "chamfer2: volume == 1000 - d1*d2/2*L");
    check(r.faces == 7, "chamfer2: 6 + 1 flat face = 7");
    // …and it is neither equal-leg reading of the params (d=1 ⇒ 995, d=2.5 ⇒ 968.75).
    check(std::abs(r.volume - 995.0) > 1.0 && std::abs(r.volume - 968.75) > 1.0,
          "chamfer2: distance2 actually reached the kernel (not an equal-leg cut)");

    // ORIENTATION: `Add(Dis1, Dis2, E, F)` measures Dis1 ON F, so on the box's
    // (0,0) vertical edge the cut meets the reference face at `d1` from the edge
    // and the other adjacent face at `d2`. The removed prism's cross-section is
    // therefore the triangle (0,0)-(0,d1)-(d2,0) when the reference face is x==0
    // (centroid (d2/3, d1/3)), and its mirror when it is y==0.
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const bool ref_x0 = reference_face_is_x0(box, edge_by_center(box, 0, 0, 5));
    const double rem_cx = (ref_x0 ? d2 : d1) / 3.0;
    const double rem_cy = (ref_x0 ? d1 : d2) / 3.0;
    const double kept = 1000.0 - removed;
    check_near(r.centre.X(), (1000.0 * 5.0 - removed * rem_cx) / kept, 1e-6,
               "chamfer2: centroid X matches `radius` on the smaller face ordinal");
    check_near(r.centre.Y(), (1000.0 * 5.0 - removed * rem_cy) / kept, 1e-6,
               "chamfer2: centroid Y matches `distance2` on the other face");
    std::fprintf(stderr, "chamfer2: reference face is %s, centroid (%.6f, %.6f)\n",
                 ref_x0 ? "x==0" : "y==0", r.centre.X(), r.centre.Y());
}

// --- The reference-face rule is a pure function of the predecessor shape: the
//     same document replayed twice produces an IDENTICAL geometry signature. ---
void test_chamfer_two_distance_reference_face_is_deterministic() {
    const ChamferRun a = run_two_distance_chamfer(1.0, 2.5);
    const ChamferRun b = run_two_distance_chamfer(1.0, 2.5);
    check(a.ok && b.ok, "chamfer2 replay: both runs Ok");
    if (!(a.ok && b.ok)) return;
    // Volume alone cannot see a flipped reference face (d1*d2/2 is symmetric) —
    // the centroid can, so equality of BOTH is the determinism proof.
    check(a.volume == b.volume, "chamfer2 replay: identical volume");
    check(a.centre.X() == b.centre.X() && a.centre.Y() == b.centre.Y() &&
              a.centre.Z() == b.centre.Z(),
          "chamfer2 replay: identical centroid (same reference face both times)");
    check(a.faces == b.faces, "chamfer2 replay: identical face count");

    // …and swapping the two legs is a DIFFERENT solid, which is what makes the
    // equality above a real assertion rather than a tautology.
    const ChamferRun swapped = run_two_distance_chamfer(2.5, 1.0);
    check(swapped.ok, "chamfer2 swapped: Ok");
    check_near(swapped.volume, a.volume, 1e-6, "chamfer2 swapped: same removed volume");
    check(std::abs(swapped.centre.X() - a.centre.X()) > 1e-6,
          "chamfer2 swapped: the legs are ORIENTED (centroid moves)");
}

// ── distance-angle chamfer (SCHEMA §7.3, 2026-08-24 — WP6 item 1) ────────────

// --- `radius` is the leg ON the reference face and `angleDeg` is the angle
//     between the chamfer face and THAT face, so the far leg is radius·tan(A)
//     and the removed prism is exactly d·(d·tanA)/2 swept the edge length. ---
void test_chamfer_angle_distance() {
    const double d1 = 1.0, angle_deg = 60.0, len = 10.0;
    const double far_leg = d1 * std::tan(angle_deg * M_PI / 180.0);  // 1.7320508…
    const ChamferRun r = run_chamfer(json{{"radius", d1}, {"angleDeg", angle_deg}}, "opca");
    check(r.ok, "chamfer-angle: Ok, no NeedsRepair (got '" + r.error_message + "')");
    if (!r.ok) return;

    const double removed = 0.5 * d1 * far_leg * len;
    check_near(r.volume, 1000.0 - removed, 1e-6,
               "chamfer-angle: volume == 1000 - d*(d*tanA)/2*L");
    check(r.faces == 7, "chamfer-angle: 6 + 1 flat face = 7");
    // …and not the equal-leg reading of `radius` (995) — the angle reached OCCT.
    check(std::abs(r.volume - 995.0) > 1.0,
          "chamfer-angle: angleDeg actually reached the kernel (not an equal-leg cut)");

    // ORIENTATION: the DISTANCE leg lies on the recomputed smaller-ordinal face and
    // the tan-derived leg on the other one, so the removed prism's cross-section is
    // the triangle (0,0)-(0,d1)-(far,0) when the reference face is x==0, and its
    // mirror when it is y==0. Measuring the distance on the OTHER face swaps these.
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const bool ref_x0 = reference_face_is_x0(box, edge_by_center(box, 0, 0, 5));
    const double rem_cx = (ref_x0 ? far_leg : d1) / 3.0;
    const double rem_cy = (ref_x0 ? d1 : far_leg) / 3.0;
    const double kept = 1000.0 - removed;
    check_near(r.centre.X(), (1000.0 * 5.0 - removed * rem_cx) / kept, 1e-6,
               "chamfer-angle: centroid X matches `radius` on the smaller face ordinal");
    check_near(r.centre.Y(), (1000.0 * 5.0 - removed * rem_cy) / kept, 1e-6,
               "chamfer-angle: centroid Y matches the tan-derived leg on the other face");
    std::fprintf(stderr, "chamfer-angle: reference face is %s, centroid (%.6f, %.6f)\n",
                 ref_x0 ? "x==0" : "y==0", r.centre.X(), r.centre.Y());
}

// --- On a 90° dihedral, `angleDeg: 45` IS the equal-leg chamfer of the same
//     distance. That identity is what makes the reference-face rule checkable
//     from a document: it pins the angle's zero without a second convention. ---
void test_chamfer_angle_45_is_equal_leg() {
    const ChamferRun equal_leg = run_chamfer(json{{"radius", 1.5}}, "opce");
    const ChamferRun at45 = run_chamfer(json{{"radius", 1.5}, {"angleDeg", 45.0}}, "opc45");
    check(equal_leg.ok && at45.ok, "chamfer-angle 45: both runs Ok");
    if (!(equal_leg.ok && at45.ok)) return;
    check_near(at45.volume, equal_leg.volume, 1e-9,
               "chamfer-angle 45: volume equal to the plain equal-leg chamfer of the same d");
    check_near(at45.centre.X(), equal_leg.centre.X(), 1e-9,
               "chamfer-angle 45: identical centroid X");
    check_near(at45.centre.Y(), equal_leg.centre.Y(), 1e-9,
               "chamfer-angle 45: identical centroid Y");
    check(at45.faces == equal_leg.faces, "chamfer-angle 45: identical face count");

    // …and 30° is a DIFFERENT solid, which is what makes the equality above an
    // assertion about the angle rather than about the code path being dead.
    const ChamferRun at30 = run_chamfer(json{{"radius", 1.5}, {"angleDeg", 30.0}}, "opc30");
    check(at30.ok && std::abs(at30.volume - equal_leg.volume) > 1e-3,
          "chamfer-angle 45: a different angle is a different solid");
}

// --- The refusals. Each names the offending field, each is a recoverable
//     OP_FAILED, and none of them touches the body. ---
void test_chamfer_angle_refusals() {
    const ChamferRun both =
        run_chamfer(json{{"radius", 1.0}, {"distance2", 2.5}, {"angleDeg", 45.0}}, "opcx1");
    check(!both.ok && both.error_code == "OP_FAILED" &&
              both.error_message.find("mutually exclusive") != std::string::npos,
          "chamfer-angle: distance2 + angleDeg refused BY NAME (got '" + both.error_message + "')");
    check_near(both.volume, 1000.0, 1e-9, "chamfer-angle: both-fields refusal left the box intact");

    const ChamferRun zero = run_chamfer(json{{"radius", 1.0}, {"angleDeg", 0.0}}, "opcx2");
    check(!zero.ok && zero.error_code == "OP_FAILED" &&
              zero.error_message.find("angleDeg") != std::string::npos,
          "chamfer-angle: 0° refused by name (got '" + zero.error_message + "')");
    check_near(zero.volume, 1000.0, 1e-9, "chamfer-angle: 0° refusal left the box intact");

    const ChamferRun flat = run_chamfer(json{{"radius", 1.0}, {"angleDeg", 180.0}}, "opcx3");
    check(!flat.ok && flat.error_code == "OP_FAILED" &&
              flat.error_message.find("angleDeg") != std::string::npos,
          "chamfer-angle: 180° refused by name (got '" + flat.error_message + "')");
    check_near(flat.volume, 1000.0, 1e-9, "chamfer-angle: 180° refusal left the box intact");

    // The static bound is DELIBERATELY loose — an obtuse dihedral legitimately
    // takes more than 90°, so the validator must not own that ceiling. 80° is far
    // past equal-leg and clears the guard; whether OCCT can build a given angle on
    // a given edge is the Build/IsDone net's business, not the validator's.
    const ChamferRun obtuse = run_chamfer(json{{"radius", 1.0}, {"angleDeg", 80.0}}, "opcx4");
    check(obtuse.error_message.find("angleDeg must be within") == std::string::npos,
          "chamfer-angle: 80° clears the static bound (got '" + obtuse.error_message + "')");

    // A PRESENT but malformed angle is an error, never a default. `read_scalar`
    // would have silently substituted 0.0 and refused for the wrong reason.
    const ChamferRun malformed =
        run_chamfer(json{{"radius", 1.0}, {"angleDeg", "45"}}, "opcx5");
    check(!malformed.ok && malformed.error_code == "OP_FAILED" &&
              malformed.error_message.find("angleDeg must be a finite scalar") != std::string::npos,
          "chamfer-angle: malformed angleDeg refused by name (got '" + malformed.error_message +
              "')");
}

// --- A Fillet carrying angleDeg is refused BY NAME. Unlike `distance2` (which
//     predates the field-identity rule and is ignored), a Chamfer-only ANGLE on a
//     Fillet record can only come from a core defect, and answering it with a
//     plain fillet would hide that. ---
void test_fillet_rejects_angle_deg() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    const TopoDS_Shape edge = edge_by_center(box, 0, 0, 5);
    json op = {{"opType", "Fillet"}, {"opId", "opfa"},
               {"inputs", json::array({edge_input("body_1", "el_e", edge, 0, 0, 5)})},
               {"params", {{"mode", "Fillet"}, {"radius", 1.0}, {"angleDeg", 45.0},
                           {"edgeIds", json::array({"e:x"})}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_fillet(ctx, op, "opfa");
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED" &&
              oc.error_message.find("angleDeg") != std::string::npos,
          "fillet+angleDeg: refused by name (got '" + oc.error_message + "')");
    check_near(onecad::session::shape_volume(bodies.get("body_1")->geom), 1000.0, 1e-6,
               "fillet+angleDeg: body untouched");
}

// --- distance2 below kMinValue ⇒ recoverable OP_FAILED (never a silent cut). ---
void test_chamfer_distance2_too_small() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    const TopoDS_Shape edge = edge_by_center(box, 0, 0, 5);
    json op = {{"opType", "Chamfer"}, {"opId", "opc3"},
               {"inputs", json::array({edge_input("body_1", "el_e", edge, 0, 0, 5)})},
               {"params", {{"mode", "Chamfer"}, {"radius", 1.0}, {"distance2", 0.0},
                           {"edgeIds", json::array({"e:x"})}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_chamfer(ctx, op, "opc3");
    check(oc.status == ops::OpOutcome::Status::Failed, "chamfer2: distance2 too small → OP_FAILED");
}

// --- A Fillet IGNORES distance2 (Chamfer-only; Rust rejects such a record, so
//     honouring it here would only paper over a core defect). ---
void test_fillet_ignores_distance2() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    const TopoDS_Shape edge = edge_by_center(box, 0, 0, 5);
    json op = {{"opType", "Fillet"}, {"opId", "opf2"},
               {"inputs", json::array({edge_input("body_1", "el_e", edge, 0, 0, 5)})},
               {"params", {{"mode", "Fillet"}, {"radius", 1.0}, {"distance2", 2.5},
                           {"edgeIds", json::array({"e:x"})}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_fillet(ctx, op, "opf2");
    check(oc.status == ops::OpOutcome::Status::Ok, "fillet+distance2: Ok");
    const double v = onecad::session::shape_volume(bodies.get("body_1")->geom);
    // (1 - pi/4)*r^2*L = 2.146 for r=1, L=10 — the r=1 fillet, unchanged.
    check_near(v, 1000.0 - (1.0 - M_PI / 4.0) * 10.0, 1e-6,
               "fillet+distance2: the plain r=1 fillet, distance2 ignored");
}

// --- Radius below kMinValue (1e-3) → recoverable OP_FAILED. ---
void test_fillet_radius_too_small() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    const TopoDS_Shape edge = edge_by_center(box, 0, 0, 5);
    json op = {{"opType", "Fillet"}, {"opId", "opf"},
               {"inputs", json::array({edge_input("body_1", "el_e", edge, 0, 0, 5)})},
               {"params", {{"mode", "Fillet"}, {"radius", 0.0}, {"edgeIds", json::array({"e:x"})}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_fillet(ctx, op, "opf");
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
          "fillet: radius too small → OP_FAILED (recoverable)");
}

// --- The authoring-resolution boundary. ---
// The radius floor is `GeometryPrecisionContext::authoring_resolution()`. The r=0
// case above is refused by ANY plausible threshold, so it cannot prove the guard is
// still wired to the context. These STRADDLE the boundary and flip the moment the
// context moves. The upper case asserts the GUARD, not the build: OCCT is entitled
// to refuse a 1.05 µm blend for its own reasons, but not with this message.
void test_fillet_authoring_resolution_boundary() {
    const std::string needle = "radius too small";
    const auto fillet_by = [&](double radius) {
        const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
        BodyStore bodies;
        bodies.create("body_1", "op0", box);
        em::ElementMapPartition part;
        const TopoDS_Shape edge = edge_by_center(box, 0, 0, 5);
        json op = {{"opType", "Fillet"}, {"opId", "opfb"},
                   {"inputs", json::array({edge_input("body_1", "el_e", edge, 0, 0, 5)})},
                   {"params", {{"mode", "Fillet"}, {"radius", radius},
                               {"edgeIds", json::array({"e:x"})}}}};
        Ctx c;
        ops::OpContext ctx = c.make(bodies, part);
        return ops::execute_fillet(ctx, op, "opfb");
    };

    const ops::OpOutcome below = fillet_by(9.9e-4);
    check(below.status == ops::OpOutcome::Status::Failed &&
              below.error_message.find(needle) != std::string::npos,
          "fillet authoring boundary: r=9.9e-4 mm is refused by name, got: " +
              below.error_message);

    const ops::OpOutcome above = fillet_by(1.05e-3);
    check(above.error_message.find(needle) == std::string::npos,
          "fillet authoring boundary: r=1.05e-3 mm clears the guard, got: " +
              above.error_message);
}

// --- Edge ref that does NOT resolve (symmetric tie) ⇒ NeedsRepair, body untouched. ---
void test_fillet_ambiguous_edge_needs_repair() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;

    // Intent = a vertical edge; anchor at the body centre → all 4 vertical edges tie.
    const TopoDS_Shape edge = edge_by_center(box, 0, 0, 5);
    json op = {{"opType", "Fillet"}, {"opId", "opf"},
               {"inputs", json::array({edge_input("body_1", "el_e", edge, 5, 5, 5)})},
               {"params", {{"mode", "Fillet"}, {"radius", 1.0}, {"edgeIds", json::array({"e:x"})}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_fillet(ctx, op, "opf");
    check(oc.status == ops::OpOutcome::Status::Ok, "fillet ambiguous: not an Err (state, not error)");
    check(!oc.needs_repair.empty(), "fillet ambiguous: NeedsRepair emitted");
    check(oc.body_events.empty(), "fillet ambiguous: body NOT modified (never a wrong bind)");
    check_near(onecad::session::shape_volume(bodies.get("body_1")->geom), 1000.0, 1e-6,
               "fillet ambiguous: body geometry unchanged");
}

// --- Revolve a rectangle offset from a sketch-line axis (Pappus volume). ---
void test_revolve_pappus() {
    // Sketch (XY plane, world map (u,v)->(-v,u,0)): rectangle u∈[10,20],v∈[0,10]
    // → world x∈[-10,0], y∈[10,20], z=0 (area 100, centroid y=15). Axis line along
    // u=0 → world through origin along X. Revolve 360° ⇒ V = 2π·15·100 = 9424.78.
    json sk;
    sk["sketchId"] = "sk1";
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = json::array({
        json{{"id", "r1"}, {"type", "Line"}, {"p0", {10, 0}}, {"p1", {20, 0}}},
        json{{"id", "r2"}, {"type", "Line"}, {"p0", {20, 0}}, {"p1", {20, 10}}},
        json{{"id", "r3"}, {"type", "Line"}, {"p0", {20, 10}}, {"p1", {10, 10}}},
        json{{"id", "r4"}, {"type", "Line"}, {"p0", {10, 10}}, {"p1", {10, 0}}},
        json{{"id", "axis"}, {"type", "Line"}, {"p0", {0, -5}}, {"p1", {0, 15}}},
    });
    sk["constraints"] = json::array();

    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", sk});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);

    json op = {{"opType", "Revolve"}, {"opId", "opr"},
               {"params", {{"sketchId", "sk1"}, {"angleDeg", 360.0}, {"booleanMode", "NewBody"},
                           {"axis", {{"kind", "sketchLine"}, {"sketchId", "sk1"}, {"lineId", "axis"}}}}}};
    ops::OpOutcome oc = ops::execute_revolve(ctx, op, "opr");
    check(oc.status == ops::OpOutcome::Status::Ok, "revolve: Ok");
    check(oc.body_events.size() == 1 && oc.body_events[0].kind == "created", "revolve: NewBody created");
    check(bodies.contains("body_opr"), "revolve: NewBody id body_opr (D1)");
    if (bodies.contains("body_opr")) {
        check_near(onecad::session::shape_volume(bodies.get("body_opr")->geom), 9424.78, 30.0,
                   "revolve: Pappus volume 2π·15·100");
    }
}

// A typed body-edge axis resolves its semantic companion, never the legacy
// snapshot ordinal. `edgeId` is deliberately nonsense: changing it must not
// change the result once `edgeRef` is present.
void test_revolve_typed_edge_axis_wins() {
    json sk;
    sk["sketchId"] = "sk1";
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = json::array({
        json{{"id", "r1"}, {"type", "Line"}, {"p0", {10, 0}}, {"p1", {20, 0}}},
        json{{"id", "r2"}, {"type", "Line"}, {"p0", {20, 0}}, {"p1", {20, 10}}},
        json{{"id", "r3"}, {"type", "Line"}, {"p0", {20, 10}}, {"p1", {10, 10}}},
        json{{"id", "r4"}, {"type", "Line"}, {"p0", {10, 10}}, {"p1", {10, 0}}},
    });
    sk["constraints"] = json::array();

    const TopoDS_Shape axis_shape =
        BRepBuilderAPI_MakeEdge(gp_Pnt(-20, 0, 0), gp_Pnt(20, 0, 0)).Shape();
    const TopoDS_Shape axis_edge = edge_by_center(axis_shape, 0, 0, 0);
    BodyStore bodies;
    bodies.create("body_axis", "axis_source", axis_shape);
    em::ElementMapPartition part;
    part.mint("body_axis", "el_axis", km::ElementKind::Edge, axis_edge, axis_shape,
              json{{"worldPoint", {0.0, 0.0, 0.0}}});

    Ctx c;
    c.sketches.push_back({"sk1", sk});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    const json typed = edge_input("body_axis", "el_axis", axis_edge, 0, 0, 0);
    json op = {{"opType", "Revolve"}, {"opId", "typed_axis"}, {"inputs", json::array({typed})},
               {"params", {{"sketchId", "sk1"}, {"angleDeg", 360.0}, {"booleanMode", "NewBody"},
                           {"axis", {{"kind", "edge"}, {"bodyId", "body_axis"},
                                     {"edgeId", "e:999"}, {"edgeRef", typed}}}}}};
    const ops::OpOutcome out = ops::execute_revolve(ctx, op, "typed_axis");
    check(out.status == ops::OpOutcome::Status::Ok, "revolve typed axis: Ok");
    check(out.needs_repair.empty(), "revolve typed axis: no NeedsRepair");
    check(bodies.contains("body_typed_axis"), "revolve typed axis: typed edge won over legacy ordinal");
}

// The four-line rectangle profile every revolve case below revolves; kept out of
// the individual tests so a case reads as "which AXIS did I hand it".
json revolve_profile_sketch() {
    json sk;
    sk["sketchId"] = "sk1";
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = json::array({
        json{{"id", "r1"}, {"type", "Line"}, {"p0", {10, 0}}, {"p1", {20, 0}}},
        json{{"id", "r2"}, {"type", "Line"}, {"p0", {20, 0}}, {"p1", {20, 10}}},
        json{{"id", "r3"}, {"type", "Line"}, {"p0", {20, 10}}, {"p1", {10, 10}}},
        json{{"id", "r4"}, {"type", "Line"}, {"p0", {10, 10}}, {"p1", {10, 0}}},
    });
    sk["constraints"] = json::array();
    return sk;
}

// WP1.5 test 2 — a CURVED axis edge is refused, never silently straightened into
// the line through its endpoints. The refusal is `OP_FAILED` and NOT NeedsRepair:
// the edge resolved perfectly well, it is simply not a valid axis, so there is
// nothing for a repair dialog to rebind (`RevolveOp.cpp:121-124`).
void test_revolve_typed_axis_curved_edge_refuses() {
    const TopoDS_Shape cyl = BRepPrimAPI_MakeCylinder(5.0, 10.0).Shape();
    const TopoDS_Shape circle = edge_by_center(cyl, 0, 0, 0);  // the base circle
    BodyStore bodies;
    bodies.create("body_axis", "axis_source", cyl);
    em::ElementMapPartition part;
    part.mint("body_axis", "el_axis", km::ElementKind::Edge, circle, cyl,
              json{{"worldPoint", {0.0, 0.0, 0.0}}});

    Ctx c;
    c.sketches.push_back({"sk1", revolve_profile_sketch()});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    const json typed = edge_input("body_axis", "el_axis", circle, 0, 0, 0);
    json op = {{"opType", "Revolve"}, {"opId", "curved_axis"}, {"inputs", json::array({typed})},
               {"params", {{"sketchId", "sk1"}, {"angleDeg", 360.0}, {"booleanMode", "NewBody"},
                           {"axis", {{"kind", "edge"}, {"bodyId", "body_axis"},
                                     {"edgeId", "e:1"}, {"edgeRef", typed}}}}}};
    const ops::OpOutcome out = ops::execute_revolve(ctx, op, "curved_axis");
    check(out.status == ops::OpOutcome::Status::Failed && out.error_code == "OP_FAILED",
          "revolve curved axis: OP_FAILED");
    check(out.error_message == "Revolve: axis edge must be a straight line",
          "revolve curved axis: names the reason (got '" + out.error_message + "')");
    check(out.needs_repair.empty(),
          "revolve curved axis: a resolvable-but-invalid edge is NOT NeedsRepair");
    check(!bodies.contains("body_curved_axis"), "revolve curved axis: no body published");
}

// WP1.5 test 4 — two bodies, and the typed axis must stay on the one it NAMES.
// Both ownership gates are exercised: a ref whose `primary.bodyId` disagrees with
// the axis `bodyId` (`RevolveOp.cpp:147`), and an elementId that agrees on paper
// but is bound to the OTHER body in the partition (`:167`). Either one binding the
// second body's edge would be a silent cross-body axis swap.
void test_revolve_typed_axis_two_body_ambiguity() {
    // Two parallel straight edges, one per body — geometrically interchangeable,
    // so only the identity gates can tell them apart.
    const TopoDS_Shape shape_a =
        BRepBuilderAPI_MakeEdge(gp_Pnt(-20, 0, 0), gp_Pnt(20, 0, 0)).Shape();
    const TopoDS_Shape shape_b =
        BRepBuilderAPI_MakeEdge(gp_Pnt(-20, 0, 5), gp_Pnt(20, 0, 5)).Shape();
    const TopoDS_Shape edge_a = edge_by_center(shape_a, 0, 0, 0);
    const TopoDS_Shape edge_b = edge_by_center(shape_b, 0, 0, 5);

    BodyStore bodies;
    bodies.create("body_a", "op_a", shape_a);
    bodies.create("body_b", "op_b", shape_b);
    em::ElementMapPartition part;
    part.mint("body_a", "el_a", km::ElementKind::Edge, edge_a, shape_a,
              json{{"worldPoint", {0.0, 0.0, 0.0}}});
    part.mint("body_b", "el_b", km::ElementKind::Edge, edge_b, shape_b,
              json{{"worldPoint", {0.0, 0.0, 5.0}}});

    Ctx c;
    c.sketches.push_back({"sk1", revolve_profile_sketch()});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);

    // (a) the ref names body_b while the axis names body_a.
    const json ref_b = edge_input("body_b", "el_b", edge_b, 0, 0, 5);
    json op_cross = {{"opType", "Revolve"}, {"opId", "cross_body"}, {"inputs", json::array({ref_b})},
                     {"params", {{"sketchId", "sk1"}, {"angleDeg", 360.0}, {"booleanMode", "NewBody"},
                                 {"axis", {{"kind", "edge"}, {"bodyId", "body_a"},
                                           {"edgeId", "e:1"}, {"edgeRef", ref_b}}}}}};
    const ops::OpOutcome cross = ops::execute_revolve(ctx, op_cross, "cross_body");
    check(cross.status == ops::OpOutcome::Status::Failed && cross.error_code == "OP_FAILED",
          "revolve two-body: mismatched ref body → OP_FAILED");
    check(cross.error_message == "Revolve: typed axis edge belongs to a different body",
          "revolve two-body: names the ownership refusal (got '" + cross.error_message + "')");
    check(!bodies.contains("body_cross_body"), "revolve two-body: nothing published on refusal");

    // (b) the ref agrees with the axis on `bodyId`, but its elementId is bound to
    // body_b in the partition — the paper trail is consistent, the binding is not.
    json ref_forged = edge_input("body_a", "el_b", edge_b, 0, 0, 5);
    json op_forged = {{"opType", "Revolve"}, {"opId", "forged"}, {"inputs", json::array({ref_forged})},
                      {"params", {{"sketchId", "sk1"}, {"angleDeg", 360.0}, {"booleanMode", "NewBody"},
                                  {"axis", {{"kind", "edge"}, {"bodyId", "body_a"},
                                            {"edgeId", "e:1"}, {"edgeRef", ref_forged}}}}}};
    const ops::OpOutcome forged = ops::execute_revolve(ctx, op_forged, "forged");
    check(forged.status == ops::OpOutcome::Status::Failed && forged.error_code == "OP_FAILED",
          "revolve two-body: foreign partition binding → OP_FAILED");
    check(forged.error_message == "Revolve: typed axis edge binding is foreign or not an edge",
          "revolve two-body: names the foreign binding (got '" + forged.error_message + "')");
    check(!bodies.contains("body_forged"), "revolve two-body: nothing published on foreign binding");
}

// --- Revolve angle too small → OP_FAILED. ---
void test_revolve_angle_too_small() {
    json sk;
    sk["sketchId"] = "sk1";
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = json::array({
        json{{"id", "r1"}, {"type", "Line"}, {"p0", {10, 0}}, {"p1", {20, 0}}},
        json{{"id", "r2"}, {"type", "Line"}, {"p0", {20, 0}}, {"p1", {20, 10}}},
        json{{"id", "r3"}, {"type", "Line"}, {"p0", {20, 10}}, {"p1", {10, 10}}},
        json{{"id", "r4"}, {"type", "Line"}, {"p0", {10, 10}}, {"p1", {10, 0}}},
        json{{"id", "axis"}, {"type", "Line"}, {"p0", {0, -5}}, {"p1", {0, 15}}},
    });
    sk["constraints"] = json::array();
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", sk});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Revolve"}, {"opId", "opr"},
               {"params", {{"sketchId", "sk1"}, {"angleDeg", 0.0}, {"booleanMode", "NewBody"},
                           {"axis", {{"kind", "sketchLine"}, {"sketchId", "sk1"}, {"lineId", "axis"}}}}}};
    ops::OpOutcome oc = ops::execute_revolve(ctx, op, "opr");
    check(oc.status == ops::OpOutcome::Status::Failed, "revolve: angle too small → OP_FAILED");
}

}  // namespace

int main() {
    test_fillet_edge();
    test_chamfer_edge();
    test_chamfer_two_distance();
    test_chamfer_two_distance_reference_face_is_deterministic();
    test_chamfer_angle_distance();
    test_chamfer_angle_45_is_equal_leg();
    test_chamfer_angle_refusals();
    test_fillet_rejects_angle_deg();
    test_chamfer_distance2_too_small();
    test_fillet_ignores_distance2();
    test_fillet_radius_too_small();
    test_fillet_authoring_resolution_boundary();
    test_fillet_ambiguous_edge_needs_repair();
    test_revolve_pappus();
    test_revolve_typed_edge_axis_wins();
    test_revolve_typed_axis_curved_edge_refuses();
    test_revolve_typed_axis_two_body_ambiguity();
    test_revolve_angle_too_small();
    if (g_failures == 0) std::fprintf(stderr, "wp6_ops: OK\n");
    return g_failures;
}
