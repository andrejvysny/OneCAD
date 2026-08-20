// test_extrude_toface_tilted.cpp — WP5: exact Extrude `ToFace` termination on a
// TILTED planar target.
//
// Until this work package a tilted target was refused by name, because the only
// thing the executor could build was a constant-distance flat cap — geometry the
// user did not ask for. The construction now is build-long-then-trim: a prism
// long enough to cross the target plane everywhere, cut back by that plane's
// half-space, so the terminating cap IS the plane. That makes the height vary
// linearly across the profile, which is what "up to that face" means.
//
// ── WHY EVERY EXPECTED VOLUME HERE IS ANALYTIC, NOT RECORDED ────────────────
// The solid is a prism of cross-section A whose height varies AFFINELY over the
// profile, so its volume is exactly
//
//     V = ∫∫_profile h(x,y) dA = A · h(centroid)
//
// (the mean of an affine function over a region is its value at the centroid).
// With the profile the S×S square x∈[−S,0], y∈[0,S] at z=0, the extrude
// direction +Z, and a target plane through (0,0,z0) whose normal is
// (sinθ, 0, cosθ):
//
//     h(p) = ((T0 − p)·n)/(d·n) = (−p_x·sinθ + z0·cosθ)/cosθ = z0 − p_x·tanθ
//
// so h runs from z0 (at p_x = 0) to z0 + S·tanθ (at p_x = −S), the centroid sits
// at p_x = −S/2, and
//
//     V = S² · (z0 + (S/2)·tanθ).
//
// Nothing here is a golden number: change θ, z0 or S and the closed form still
// predicts the answer. That is the point — a recorded volume proves only that
// the code still does what it did.
//
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/ExtrudeOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/HistoryHash.h"
#include "session/PlanExecutor.h"
#include "session/PreviewOp.h"
#include "session/ScratchJob.h"
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
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

// Relative comparison, because the scale fixtures span 0.01 mm and 10 m: a fixed
// absolute tolerance is either meaningless at one end or vacuous at the other.
// Always PRINTS the measurement — the numbers are the deliverable.
void check_exact(double got, double want, double relative, const std::string& msg) {
    const double error = std::abs(got - want);
    const double budget = std::max(1.0e-12, std::abs(want) * relative);
    std::fprintf(stderr, "  %-46s got %.17g want %.17g  relErr %.3g\n", msg.c_str(), got, want,
                 want == 0.0 ? error : error / std::abs(want));
    if (error > budget) {
        std::fprintf(stderr, "FAIL: %s (got %.17g want %.17g, budget %.3g)\n", msg.c_str(), got,
                     want, budget);
        ++g_failures;
    }
}

// ── fixtures ────────────────────────────────────────────────────────────────

// Four Line entities forming the closed S×S square. World map (u,v)->(-v,u,0),
// so the loop covers world x∈[-S,0], y∈[0,S], z=0 (area S²).
json square_sketch(const std::string& sid, double s) {
    json sk;
    sk["sketchId"] = sid;
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = json::array({
        json{{"id", "e1"}, {"type", "Line"}, {"p0", {0.0, 0.0}}, {"p1", {s, 0.0}}},
        json{{"id", "e2"}, {"type", "Line"}, {"p0", {s, 0.0}}, {"p1", {s, s}}},
        json{{"id", "e3"}, {"type", "Line"}, {"p0", {s, s}}, {"p1", {0.0, s}}},
        json{{"id", "e4"}, {"type", "Line"}, {"p0", {0.0, s}}, {"p1", {0.0, 0.0}}},
    });
    sk["constraints"] = json::array();
    return sk;
}

gp_Dir tilt_normal(double theta) { return gp_Dir(std::sin(theta), 0.0, std::cos(theta)); }

// A block whose SEATED face lies on the plane through (0,0,z0) with normal
// (sinθ,0,cosθ), covering world x∈[-2s+dx, 2s+dx] and y∈[-s/2, 3s/2] — i.e. the
// s×s profile footprint with a margin all round when `dx` is 0.
//
// `dx` slides the block along its own in-plane X direction, so the PLANE is
// identical for every `dx` and only the trim moves — which is exactly what a
// partial-coverage probe needs: same termination geometry, less bounded face.
//
// Thickness 4s, not a thin plate: the seated face and the block's opposite face
// have the same area and antiparallel normals, so a thin block makes them a
// near-tie for the resolution ladder, which then answers NeedsRepair (correctly
// — Invariant 2). A fixture that has to be repaired proves nothing about
// termination, so the two faces are placed well apart.
// `below` grows the block along −n instead of +n. The seated face and its plane
// are identical either way (the shift is purely along the local Y, which lies IN
// the plane); it exists so a target BEHIND the profile can still be a block that
// sits away from the profile rather than one growing through it.
TopoDS_Shape tilted_plate(double theta, double z0, double s, double dx = 0.0,
                          bool below = false) {
    const double c = std::cos(theta), t = std::tan(theta);
    const gp_Dir n = tilt_normal(theta);
    const gp_Dir u(c, 0.0, -std::sin(theta));  // in-plane, toward +X
    const double qx = -2.0 * s + dx;
    const double qz = z0 + (2.0 * s - dx) * t;
    // local Y = localZ × localX, so reversing the normal reverses it too: the
    // world-y footprint stays [−s/2, 3s/2] only if the origin moves to its far end.
    const gp_Pnt q(qx, below ? 1.5 * s : -0.5 * s, qz);
    const gp_Ax2 frame(q, below ? n.Reversed() : n, u);
    return BRepPrimAPI_MakeBox(frame, 4.0 * s / c, 2.0 * s, 4.0 * s).Shape();
}

double centroid_z(const TopoDS_Shape& solid) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(solid, props);
    return props.CentreOfMass().Z();
}

double centroid_x(const TopoDS_Shape& solid) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(solid, props);
    return props.CentreOfMass().X();
}

// The planar face of `shape` whose support plane passes through `point` with
// normal (anti)parallel to `n` — i.e. the plate's seated face, named by the
// geometry the test constructed rather than by an explorer ordinal.
TopoDS_Shape face_on_plane(const TopoDS_Shape& shape, const gp_Pnt& point, const gp_Dir& n) {
    for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
        const TopoDS_Face face = TopoDS::Face(it.Current());
        BRepAdaptor_Surface surface(face, true);
        if (surface.GetType() != GeomAbs_Plane) continue;
        const gp_Pln plane = surface.Plane();
        if (!plane.Axis().Direction().IsParallel(n, 1.0e-9)) continue;
        if (std::abs(plane.Distance(point)) > 1.0e-9) continue;
        return face;
    }
    return {};
}

// The lateral (cylindrical) face of `shape`.
TopoDS_Shape curved_face(const TopoDS_Shape& shape) {
    for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
        BRepAdaptor_Surface surface(TopoDS::Face(it.Current()), true);
        if (surface.GetType() == GeomAbs_Cylinder) return it.Current();
    }
    return {};
}

json semantic_face_ref(const std::string& body_id, const std::string& element_id,
                       const TopoDS_Shape& face) {
    const km::ElementDescriptor d = em::ElementMapPartition::describe(face);
    return {{"primary", {{"bodyId", body_id}, {"elementId", element_id}, {"kind", "face"}}},
            {"intent",
             {{"kind", "face"}, {"descriptor", em::ElementMapPartition::descriptor_to_json(d)}}},
            {"anchor", {{"worldPoint", {d.center.X(), d.center.Y(), d.center.Z()}}}}};
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& b, em::ElementMapPartition& p) {
        return ops::OpContext{b, &sketches, p, &last_sketch, false, json::object(), &cancel};
    }
};

json extrude_to_face(const json& target_face, const std::string& boolean_mode = "NewBody",
                     const std::string& target_body = {}) {
    json params = {{"sketchId", "sk1"},
                   {"extrudeMode", "ToFace"},
                   {"booleanMode", boolean_mode},
                   {"targetFace", target_face}};
    if (!target_body.empty()) params["targetBodyId"] = target_body;
    return json{{"opType", "Extrude"}, {"opId", "ope"}, {"params", std::move(params)}};
}

// The single diagnostic code an outcome carries ("" when it carries none).
std::string diagnostic_code(const ops::OpOutcome& outcome) {
    if (outcome.diagnostics.size() != 1) return "<" + std::to_string(outcome.diagnostics.size()) +
                                                 " diagnostics>";
    return outcome.diagnostics[0].value("code", std::string{});
}

void report_refusal(const char* label, const ops::OpOutcome& outcome) {
    std::fprintf(stderr, "  %-30s code=%s stage=%s msg=\"%s\"\n", label,
                 diagnostic_code(outcome).c_str(),
                 outcome.diagnostics.size() == 1
                     ? outcome.diagnostics[0].value("stage", std::string{}).c_str()
                     : "-",
                 outcome.error_message.c_str());
}

// ── the exact-volume battery ────────────────────────────────────────────────

// One NewBody tilted ToFace at scale `s`, tilt `theta`, target plane at z0 = 2s.
// Returns the produced volume, or a negative sentinel on refusal.
double run_tilted(double theta, double s, ops::OpOutcome& outcome_out,
                  em::ElementMapPartition& part, BodyStore& bodies) {
    const double z0 = 2.0 * s;
    const TopoDS_Shape plate = tilted_plate(theta, z0, s);
    bodies.create("body_plate", "op_plate", plate);
    const TopoDS_Shape target = face_on_plane(plate, gp_Pnt(0, 0, z0), tilt_normal(theta));
    if (target.IsNull()) return -1.0;

    Ctx c;
    c.sketches.push_back({"sk1", square_sketch("sk1", s)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    outcome_out = ops::execute_extrude(
        ctx, extrude_to_face(semantic_face_ref("body_plate", "el_tf", target)), "ope");
    if (outcome_out.status != ops::OpOutcome::Status::Ok || !bodies.contains("body_ope")) {
        return -2.0;
    }
    return onecad::session::shape_volume(bodies.get("body_ope")->geom);
}

void tilted_case(double degrees, double s, const std::string& label) {
    const double theta = degrees * M_PI / 180.0;
    BodyStore bodies;
    em::ElementMapPartition part;
    ops::OpOutcome outcome;
    const double got = run_tilted(theta, s, outcome, part, bodies);
    if (got < 0.0) {
        std::fprintf(stderr, "FAIL: %s did not produce a body (status=%d msg=%s nr=%s)\n",
                     label.c_str(), static_cast<int>(outcome.status),
                     outcome.error_message.c_str(),
                     outcome.needs_repair.empty() ? "-" : outcome.needs_repair[0].dump().c_str());
        ++g_failures;
        return;
    }
    // V = A · h(centroid) = s² · (z0 + (s/2)·tanθ), z0 = 2s.
    const double want = s * s * (2.0 * s + 0.5 * s * std::tan(theta));
    check_exact(got, want, 1.0e-12, label);
    check(outcome.needs_repair.empty(), label + ": no NeedsRepair");
    check(outcome.diagnostics.empty(), label + ": no diagnostics on success");
    check(outcome.body_events.size() == 1 && outcome.body_events[0].kind == "created" &&
              outcome.body_events[0].body_id == "body_ope",
          label + ": worker-minted body_<opId> created");
    check(outcome.delta.added.empty() && outcome.delta.relabeled.empty() &&
              outcome.delta.removed.empty(),
          label + ": NewBody carries an empty element-map delta");
}

// ── refusals ────────────────────────────────────────────────────────────────

ops::OpOutcome refuse_with(const TopoDS_Shape& target_body, const TopoDS_Shape& target_face,
                           double s) {
    BodyStore bodies;
    bodies.create("body_target", "op_t", target_body);
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", square_sketch("sk1", s)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome outcome = ops::execute_extrude(
        ctx, extrude_to_face(semantic_face_ref("body_target", "el_tf", target_face)), "ope");
    check(!bodies.contains("body_ope"), "refusal publishes no body");
    return outcome;
}

void expect_refusal(const char* label, const ops::OpOutcome& outcome, const char* code) {
    report_refusal(label, outcome);
    check(outcome.status == ops::OpOutcome::Status::Failed, std::string(label) + ": refused");
    check(outcome.error_code == "OP_FAILED", std::string(label) + ": OP_FAILED");
    check(diagnostic_code(outcome) == code, std::string(label) + ": code " + code);
}

}  // namespace

int main() {
    const double kQuarter = 45.0 * M_PI / 180.0;

    // ── 1. Exact volumes across the tilt range ──────────────────────────────
    std::fprintf(stderr, "-- exact volumes (V = s²·(2s + (s/2)·tanθ)) --\n");
    tilted_case(45.0, 10.0, "45deg s=10");
    tilted_case(75.0, 10.0, "75deg s=10");
    tilted_case(5.0, 10.0, "5deg s=10");

    // ── 2. The same construction at both ends of the supported model range ──
    std::fprintf(stderr, "-- scale (0.01 mm and 10 m) --\n");
    tilted_case(45.0, 0.01, "45deg s=0.01mm");
    tilted_case(45.0, 10000.0, "45deg s=10m");

    // ── 3. Boolean Cut lowers the tilted tool like any other ────────────────
    //
    // Stock x∈[-20,10], y∈[-10,20], z∈[0,40] = 30·30·40 = 36000 mm³. The tilted
    // tool (2500 mm³, the 45°/s=10 case above) sits wholly inside it laterally
    // and reaches z ∈ [20,30] < 40, so the pocket is open at z=0 and the result
    // is one solid of 36000 − 2500 = 33500 mm³.
    {
        std::fprintf(stderr, "-- boolean Cut --\n");
        const TopoDS_Shape plate = tilted_plate(kQuarter, 20.0, 10.0);
        const TopoDS_Shape target = face_on_plane(plate, gp_Pnt(0, 0, 20), tilt_normal(kQuarter));
        check(!target.IsNull(), "cut: tilted target face found");
        BodyStore bodies;
        bodies.create("body_plate", "op_plate", plate);
        bodies.create("body_stock", "op_stock",
                      BRepPrimAPI_MakeBox(gp_Pnt(-20, -10, 0), 30.0, 30.0, 40.0).Shape());
        em::ElementMapPartition part;
        Ctx c;
        c.sketches.push_back({"sk1", square_sketch("sk1", 10.0)});
        c.last_sketch = "sk1";
        ops::OpContext ctx = c.make(bodies, part);
        const ops::OpOutcome outcome = ops::execute_extrude(
            ctx,
            extrude_to_face(semantic_face_ref("body_plate", "el_tf", target), "Cut", "body_stock"),
            "ope");
        check(outcome.status == ops::OpOutcome::Status::Ok, "cut: Ok");
        check(outcome.body_events.size() == 1 && outcome.body_events[0].kind == "modified" &&
                  outcome.body_events[0].body_id == "body_stock",
              "cut: stock modified in place (BodyId preserved)");
        check(!bodies.contains("body_ope"), "cut: no new body minted");
        if (bodies.contains("body_stock")) {
            check_exact(onecad::session::shape_volume(bodies.get("body_stock")->geom), 33500.0,
                        1.0e-12, "cut: 36000 - 2500");
        }
    }

    // ── 3b. The BACKWARD branch: a target BEHIND the profile ────────────────
    //
    // `h` is signed. When the whole profile terminates on the far side of the
    // sketch plane the resolver takes `sign = −1`, the prism is built along −Z,
    // and the kept half-space is the one on the OTHER side of the target plane
    // from the forward case. Nothing else in the suite exercises that branch.
    //
    // WHY IT MATTERS THAT THIS IS TESTED AT ALL. Flipping the kept side here
    // does not produce an empty trim — it produces the slab between the target
    // plane and the prism's far end, whose cap is still the full oblique section
    // on the target plane and is still inside the bounded face. Both publication
    // proofs pass. The prism runs to −(30 + 0.05·30 + 0.001) = −31.501, so the
    // wrong solid would be 10·∫(31.501 − (30+x))dx over x∈[−10,0] = 650.1 mm³,
    // shipped silently. Volume is the only thing that catches it.
    //
    // Plane through (0,0,−30) at 45°, so h(p) = −30 − p_x runs −30 → −20 over
    // p_x ∈ [−10,0]: wholly behind, mean depth 25, V = 100·25 = 2500 mm³ — the
    // mirror of the forward case. The centroid follows from the same integrals:
    //   x̄ = ∫x(30+x)dx / ∫(30+x)dx = −1166.66… / 250 = −14/3
    //   z̄ = −∫((30+x)²/2)dx / 250 = −3166.66… / 250 = −38/3
    {
        std::fprintf(stderr, "-- backward branch (target behind the profile) --\n");
        BodyStore bodies;
        bodies.create("body_plate", "op_plate",
                      tilted_plate(kQuarter, -30.0, 10.0, /*dx=*/0.0, /*below=*/true));
        const TopoDS_Shape target = face_on_plane(bodies.get("body_plate")->geom,
                                                  gp_Pnt(0, 0, -30), tilt_normal(kQuarter));
        check(!target.IsNull(), "backward: seated face found");
        em::ElementMapPartition part;
        Ctx c;
        c.sketches.push_back({"sk1", square_sketch("sk1", 10.0)});
        c.last_sketch = "sk1";
        ops::OpContext ctx = c.make(bodies, part);
        const ops::OpOutcome outcome = ops::execute_extrude(
            ctx, extrude_to_face(semantic_face_ref("body_plate", "el_tf", target)), "ope");
        check(outcome.status == ops::OpOutcome::Status::Ok, "backward: Ok");
        check(bodies.contains("body_ope"), "backward: NewBody created");
        if (bodies.contains("body_ope")) {
            const TopoDS_Shape solid = bodies.get("body_ope")->geom;
            check_exact(onecad::session::shape_volume(solid), 2500.0, 1.0e-12,
                        "backward: A·mean|h| = 100·25");
            check_exact(centroid_x(solid), -14.0 / 3.0, 1.0e-12, "backward: centroid x = -14/3");
            check_exact(centroid_z(solid), -38.0 / 3.0, 1.0e-12, "backward: centroid z = -38/3");
        }
    }

    // ── 3c. TWO DIRECTIONS, both tilted ─────────────────────────────────────
    //
    // `trim2` is a separate half-space applied to the second prism before the
    // fuse, and it is resolved against `dir2 = −Z`, so its sign convention is
    // the mirror of direction 1's. Untested, a tilted `targetFace2` could trim
    // against direction 1's kept side and no other case would notice.
    //
    // Up: plane through (0,0,20), h = 20 − p_x ∈ [20,30], V = 2500, z̄ = +38/3.
    // Down: plane through (0,0,−30) measured along −Z, h = 30 + p_x ∈ [20,30],
    // V = 2500, z̄ = −38/3. Fused (the two halves meet exactly on the sketch
    // plane and share no interior): V = 5000 and the centroid z is exactly 0,
    // because the two equal volumes sit at ±38/3.
    {
        std::fprintf(stderr, "-- two directions, both tilted --\n");
        BodyStore bodies;
        bodies.create("body_up", "op_up", tilted_plate(kQuarter, 20.0, 10.0));
        bodies.create("body_down", "op_down",
                      tilted_plate(kQuarter, -30.0, 10.0, /*dx=*/0.0, /*below=*/true));
        const gp_Dir n = tilt_normal(kQuarter);
        const TopoDS_Shape up =
            face_on_plane(bodies.get("body_up")->geom, gp_Pnt(0, 0, 20), n);
        const TopoDS_Shape down =
            face_on_plane(bodies.get("body_down")->geom, gp_Pnt(0, 0, -30), n);
        check(!up.IsNull() && !down.IsNull(), "twoDir: both seated faces found");
        em::ElementMapPartition part;
        Ctx c;
        c.sketches.push_back({"sk1", square_sketch("sk1", 10.0)});
        c.last_sketch = "sk1";
        ops::OpContext ctx = c.make(bodies, part);
        json op = {{"opType", "Extrude"},
                   {"opId", "ope"},
                   {"params",
                    {{"sketchId", "sk1"},
                     {"extrudeMode", "ToFace"},
                     {"extrudeMode2", "ToFace"},
                     {"twoDirections", true},
                     {"booleanMode", "NewBody"},
                     {"targetFace", semantic_face_ref("body_up", "el_up", up)},
                     {"targetFace2", semantic_face_ref("body_down", "el_down", down)}}}};
        const ops::OpOutcome outcome = ops::execute_extrude(ctx, op, "ope");
        check(outcome.status == ops::OpOutcome::Status::Ok, "twoDir: Ok");
        check(outcome.needs_repair.empty(), "twoDir: both targets resolved");
        check(bodies.contains("body_ope"), "twoDir: NewBody created");
        if (bodies.contains("body_ope")) {
            const TopoDS_Shape solid = bodies.get("body_ope")->geom;
            check_exact(onecad::session::shape_volume(solid), 5000.0, 1.0e-12,
                        "twoDir: 2500 up + 2500 down");
            // Exactly 0 by symmetry, so this is an ABSOLUTE budget: a relative
            // one against 0 would assert nothing.
            const double cz = centroid_z(solid);
            std::fprintf(stderr, "  %-46s got %.17g want 0\n", "twoDir: centroid z", cz);
            check(std::abs(cz) < 1.0e-9, "twoDir: centroid z is 0 (equal halves at ±38/3)");
        }
    }

    // ── 4. Refusals, by stable code ─────────────────────────────────────────
    std::fprintf(stderr, "-- refusals --\n");
    {
        // A cylinder's lateral face is not planar. Axis along +Y so the curved
        // face genuinely sits over the profile footprint.
        const TopoDS_Shape cylinder =
            BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(-5, -10, 30), gp_Dir(0, 1, 0)), 10.0, 30.0)
                .Shape();
        const TopoDS_Shape lateral = curved_face(cylinder);
        check(!lateral.IsNull(), "curved: cylindrical face found");
        expect_refusal("curved target", refuse_with(cylinder, lateral, 10.0),
                       "EXTRUDE_TO_FACE_TARGET_NOT_PLANAR");
    }
    {
        // A hole through the plate under the profile footprint. The cap is a full
        // oblique section; the BOUNDED face has a void in the middle of it.
        const TopoDS_Shape plate = tilted_plate(kQuarter, 20.0, 10.0);
        const gp_Dir n = tilt_normal(kQuarter);
        // Where the profile centroid's ray meets the plane: h = z0 − p_x·tanθ.
        const gp_Pnt hit(-5.0, 5.0, 20.0 + 5.0);
        const gp_Pnt bore_start = hit.Translated(gp_Vec(n) * -40.0);
        const TopoDS_Shape bore =
            BRepPrimAPI_MakeCylinder(gp_Ax2(bore_start, n), 2.0, 120.0).Shape();
        BRepAlgoAPI_Cut holed(plate, bore);
        holed.Build();
        check(holed.IsDone(), "holed: plate bored");
        const TopoDS_Shape target = face_on_plane(holed.Shape(), gp_Pnt(0, 0, 20), n);
        check(!target.IsNull(), "holed: seated face found");
        expect_refusal("holed target", refuse_with(holed.Shape(), target, 10.0),
                       "EXTRUDE_TO_FACE_NOT_COVERED");
    }
    {
        // Same PLANE, block slid 15 mm along its own in-plane X: the bounded face
        // now starts at world x = −5, so half the profile footprint hangs off it.
        const TopoDS_Shape plate = tilted_plate(kQuarter, 20.0, 10.0, /*dx=*/15.0);
        const TopoDS_Shape target = face_on_plane(plate, gp_Pnt(0, 0, 20), tilt_normal(kQuarter));
        check(!target.IsNull(), "partial: seated face found");
        expect_refusal("partial overlap", refuse_with(plate, target, 10.0),
                       "EXTRUDE_TO_FACE_NOT_COVERED");
    }
    {
        // A face whose plane CONTAINS the extrude direction: the ray never meets
        // it, so there is no termination at any distance.
        const TopoDS_Shape wall =
            BRepPrimAPI_MakeBox(gp_Pnt(-40, -10, -10), 20.0, 40.0, 60.0).Shape();
        const TopoDS_Shape target = face_on_plane(wall, gp_Pnt(-20, 0, 0), gp_Dir(1, 0, 0));
        check(!target.IsNull(), "parallel: wall face found");
        expect_refusal("plane parallel to dir", refuse_with(wall, target, 10.0),
                       "EXTRUDE_TO_FACE_DEGENERATE");
    }
    {
        // The plane crosses the profile: h runs −5 → +5, so half the footprint
        // would have to extrude backwards. Ambiguous, never a guess.
        const TopoDS_Shape plate = tilted_plate(kQuarter, -5.0, 10.0);
        const TopoDS_Shape target = face_on_plane(plate, gp_Pnt(0, 0, -5), tilt_normal(kQuarter));
        check(!target.IsNull(), "straddle: seated face found");
        const ops::OpOutcome outcome = refuse_with(plate, target, 10.0);
        expect_refusal("plane straddles profile", outcome, "EXTRUDE_TO_FACE_DEGENERATE");
        if (outcome.diagnostics.size() == 1) {
            const json ev = outcome.diagnostics[0].value("evidence", json::object());
            check(ev.contains("toFace") && ev["toFace"].contains("minHeight") &&
                      ev["toFace"].contains("maxHeight"),
                  "straddle: evidence names the height range");
        }
    }

    // ── 5. Determinism: same input, same solid, same TopoKeys ───────────────
    {
        std::fprintf(stderr, "-- determinism --\n");
        double volumes[2] = {0.0, 0.0};
        std::string cap_keys[2];
        std::string body_ids[2];
        for (int run = 0; run < 2; ++run) {
            BodyStore bodies;
            em::ElementMapPartition part;
            ops::OpOutcome outcome;
            volumes[run] = run_tilted(kQuarter, 10.0, outcome, part, bodies);
            if (!bodies.contains("body_ope")) continue;
            body_ids[run] = outcome.body_events.empty() ? "" : outcome.body_events[0].body_id;
            const TopoDS_Shape solid = bodies.get("body_ope")->geom;
            const TopoDS_Shape cap = face_on_plane(solid, gp_Pnt(0, 0, 20), tilt_normal(kQuarter));
            cap_keys[run] = cap.IsNull() ? "" : em::ElementMapPartition::topokey_for_shape(
                                                    solid, cap, km::ElementKind::Face);
        }
        std::fprintf(stderr, "  run A cap TopoKey=%s  run B cap TopoKey=%s\n",
                     cap_keys[0].c_str(), cap_keys[1].c_str());
        check(volumes[0] == volumes[1], "determinism: bit-identical volume");
        check(!cap_keys[0].empty() && cap_keys[0] == cap_keys[1],
              "determinism: the cap face keeps its TopoKey across runs");
        check(body_ids[0] == "body_ope" && body_ids[0] == body_ids[1],
              "determinism: worker-minted BodyId is body_<opId>");
    }

    // ── 6. PREVIEW == COMMIT ────────────────────────────────────────────────
    //
    // PreviewOp runs the SAME candidate executor against a throwaway head copy,
    // so an exact tilted termination must preview exactly. Asserted twice: the
    // real verb ships a mesh, and the preview lane's executor (the only thing
    // that differs is `ValidationMode`) produces the identical solid.
    {
        std::fprintf(stderr, "-- preview parity --\n");
        onecad::session::Session session;
        session.open("doc", 1, 1, "normal");

        onecad::session::FenceOutcome fenced = session.fence_and_clone(
            1, 1, 1, onecad::session::kEmptyPrefixHash);
        onecad::session::ScratchJob seed;
        seed.job_id = 1;
        seed.plan_document_revision = 1;
        seed.bodies = std::move(fenced.cloned_bodies);
        seed.partition = std::move(fenced.cloned_partition);
        seed.prepared_snapshot_id = fenced.prepared_snapshot_id;
        seed.bodies.create("body_plate", "op_plate", tilted_plate(kQuarter, 20.0, 10.0));
        const TopoDS_Shape target =
            face_on_plane(seed.bodies.get("body_plate")->geom, gp_Pnt(0, 0, 20),
                          tilt_normal(kQuarter));
        check(!target.IsNull(), "preview: tilted target face found");
        session.store_prepared(std::move(seed));
        session.accept_prepared(1, 1, 1);

        json sketch_args = square_sketch("sk1", 10.0);
        sketch_args.erase("sketchId");
        session.sketches().upsert("sk1", sketch_args);

        const json op = extrude_to_face(semantic_face_ref("body_plate", "el_tf", target));

        onecad::protocol::Envelope req;
        req.id = 7;
        req.args = json{{"op", op}, {"sketchId", "sk1"}, {"lod", "coarse"}};
        const onecad::protocol::Envelope resp =
            onecad::session::handle_preview_op(session, req);
        check(!resp.error.has_value(), "preview: verb succeeded");
        if (resp.error) {
            std::fprintf(stderr, "  preview error: %s / %s\n", resp.error->code.c_str(),
                         resp.error->message.c_str());
        } else {
            check(resp.result["meshes"].size() == 1, "preview: one mesh shipped");
            check(resp.result["bodyEvents"].size() == 1 &&
                      resp.result["bodyEvents"][0]["kind"] == "created",
                  "preview: created event");
            check(!resp.out_bin.empty(), "preview: MESH1 bytes shipped");
        }

        // The preview lane's own executor, so the candidate SOLID is measurable.
        onecad::session::ScratchJob preview_job;
        preview_job.bodies = session.bodies_copy();
        preview_job.partition = session.partition_copy();
        json seeded = sketch_args;
        seeded["sketchId"] = "sk1";
        preview_job.sketches.emplace_back("sk1", seeded);
        std::string last = "sk1";
        const onecad::CancelToken no_cancel;
        const onecad::session::CandidateResult preview_candidate =
            onecad::session::execute_candidate_op(preview_job, op, "ope", last, no_cancel,
                                                  ops::ValidationMode::PreviewInteractive);
        check(preview_candidate.status == onecad::session::CandidateResult::Status::Ok,
              "preview: candidate Ok");
        const double preview_volume =
            preview_job.bodies.contains("body_ope")
                ? onecad::session::shape_volume(preview_job.bodies.get("body_ope")->geom)
                : -1.0;

        onecad::session::ScratchJob commit_job;
        commit_job.bodies = session.bodies_copy();
        commit_job.partition = session.partition_copy();
        commit_job.sketches.emplace_back("sk1", seeded);
        last = "sk1";
        const onecad::session::CandidateResult commit_candidate =
            onecad::session::execute_candidate_op(commit_job, op, "ope", last, no_cancel,
                                                  ops::ValidationMode::CommitAuthoritative);
        check(commit_candidate.status == onecad::session::CandidateResult::Status::Ok,
              "commit: candidate Ok");
        const double commit_volume =
            commit_job.bodies.contains("body_ope")
                ? onecad::session::shape_volume(commit_job.bodies.get("body_ope")->geom)
                : -2.0;

        std::fprintf(stderr, "  preview %.17g  commit %.17g\n", preview_volume, commit_volume);
        check(preview_volume == commit_volume, "preview volume == commit volume, bit for bit");
        check_exact(commit_volume, 2500.0, 1.0e-12, "commit: 45deg s=10");
    }

    if (g_failures == 0) std::fprintf(stderr, "extrude_toface_tilted: all checks passed\n");
    return g_failures;
}
