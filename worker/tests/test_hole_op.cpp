// test_hole_op.cpp — WP-C T3 machined-hole numerics (SCHEMA §7.3 `Hole`),
// in-process via `ops::execute_hole` on real OCCT. Every expectation is EXACT
// analytic arithmetic against the EXACT BRep (GProp volume), never a mesh:
//
//   simple through-all  10000 − π·r²·t
//   simple blind        10000 − π·r²·depth
//   counterbore         10000 − (π·r²·(depth − cbDepth) + π·cbR²·cbDepth)
//   countersink         10000 − (π·r²·depth + frustum − π·r²·csDepth),
//                       frustum = (π·csDepth/3)(csR² + csR·r + r²),
//                       csDepth = (csD − d)/2 / tan(csAngle/2)
//
// plus the recoverable guards (non-planar seat, point off the plane, point off the
// face boundary, cb/cs invariants) and bit-equal determinism.
//
// NOT named test_hole_*.cpp by accident: `test_hole_extrude.cpp` is an EXTRUDE
// test (a profile with a hole in it) and is unrelated.
//
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/HoleOp.h"
#include "ops/HoleTool.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "session/Signatures.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
using onecad::session::BodyStore;

namespace {

constexpr double kPi = 3.14159265358979323846;
// The box every case is drilled into: 20×20×25, volume 10000, top face at z=25.
constexpr double kBoxVol = 10000.0;

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}
// Relative comparison — the analytic expectations are exact, so the only slack is
// OCCT's own GProp quadrature over the cylindrical/conical walls.
void check_rel(double got, double want, double rel, const std::string& msg) {
    const double tol = std::abs(want) * rel;
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.9f want %.9f, tol %.2g)\n", msg.c_str(), got, want,
                     tol);
        ++g_failures;
    }
}

double vol(const TopoDS_Shape& s) { return onecad::session::shape_volume(s); }

int solid_count(const TopoDS_Shape& shape) {
    TopTools_IndexedMapOfShape solids;
    TopExp::MapShapes(shape, TopAbs_SOLID, solids);
    return solids.Extent();
}

TopoDS_Shape box_20_20_25() { return BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape(); }

// The FACE whose descriptor centre is nearest (cx,cy,cz) — test_m6a_ops.cpp:49.
TopoDS_Shape face_by_center(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(faces(i));
        const double dx = d.center.X() - cx, dy = d.center.Y() - cy, dz = d.center.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) {
            best_d2 = d2;
            best = faces(i);
        }
    }
    return best;
}

// A face input ref carrying the face's frozen descriptor + a world-point anchor.
json face_input(const std::string& body_id, const std::string& elem_id, const TopoDS_Shape& face,
                double ax, double ay, double az) {
    return json{{"primary", {{"bodyId", body_id}, {"elementId", elem_id}, {"kind", "face"}}},
                {"intent",
                 {{"kind", "face"},
                  {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                     em::ElementMapPartition::describe(face))}}},
                {"anchor", {{"worldPoint", {ax, ay, az}}}}};
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, em::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

// A Hole op on the box's TOP face, centred at (10,10,25), with `extra` merged into
// params. The host body ref rides inputs[0] exactly as the wire lowers it.
json hole_op(const TopoDS_Shape& host, const json& extra) {
    const TopoDS_Shape top = face_by_center(host, 10, 10, 25);
    json params = {{"targetBodyId", "body_1"}, {"point", {10.0, 10.0, 25.0}},
                   {"holeType", "simple"},     {"diameter", 6.0},
                   {"depth", nullptr}};
    params.update(extra);
    return json{{"opType", "Hole"},
                {"opId", "ophl"},
                {"inputs", json::array({json{{"primary",
                                              {{"bodyId", "body_1"},
                                               {"elementId", "body_1"},
                                               {"kind", "body"}}}},
                                        face_input("body_1", "el_top", top, 10, 10, 25)})},
                {"params", params}};
}

ops::OpOutcome run_hole_on_host(BodyStore& bodies,
                                    em::ElementMapPartition& part,
                                    const TopoDS_Shape& host,
                                    const json& extra) {
    bodies.create("body_1", "op0", host);
    const json op = hole_op(host, extra);
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    return ops::execute_hole(ctx, op, "ophl");
}

// Run one Hole against a fresh 20×20×25 box; returns the outcome and leaves the
// result in `bodies`.
ops::OpOutcome run_hole(BodyStore& bodies, em::ElementMapPartition& part,
                        const json& extra) {
    return run_hole_on_host(bodies, part, box_20_20_25(), extra);
}

// ── 1. Simple THROUGH-ALL: removed = π·r²·t, t = the full 25 mm thickness ──────
void test_simple_through_all() {
    BodyStore bodies;
    em::ElementMapPartition part;
    const ops::OpOutcome oc = run_hole(bodies, part, json{{"depth", nullptr}});
    check(oc.status == ops::OpOutcome::Status::Ok, "through-all: Ok");
    check(oc.needs_repair.empty(), "through-all: no NeedsRepair (top face resolves)");
    // SCHEMA §7.3: lineage = modified on targetBodyId, nothing minted.
    check(oc.body_events.size() == 1 && oc.body_events[0].kind == "modified" &&
              oc.body_events[0].body_id == "body_1",
          "through-all: MODIFIED host, id preserved, nothing minted");
    check(oc.body_ids.size() == 1 && oc.body_ids[0] == "body_1", "through-all: one output body");
    const double want = kBoxVol - kPi * 3.0 * 3.0 * 25.0;
    check_rel(vol(bodies.get("body_1")->geom), want, 1e-6, "through-all: 10000 − π·3²·25");
}

// ── 2. Simple BLIND: removed = π·r²·depth (the overshoot is outside the solid) ─
void test_simple_blind() {
    BodyStore bodies;
    em::ElementMapPartition part;
    const ops::OpOutcome oc = run_hole(bodies, part, json{{"depth", 10.0}});
    check(oc.status == ops::OpOutcome::Status::Ok, "blind: Ok");
    const double want = kBoxVol - kPi * 3.0 * 3.0 * 10.0;
    check_rel(vol(bodies.get("body_1")->geom), want, 1e-6, "blind: 10000 − π·3²·10 (exact depth)");
    // A blind hole leaves a bottom: strictly more material than the through hole.
    check(vol(bodies.get("body_1")->geom) > kBoxVol - kPi * 9.0 * 25.0,
          "blind: leaves more material than through-all");
}

// ── 3. COUNTERBORE: π·r²·(depth − cbDepth) + π·cbR²·cbDepth ───────────────────
void test_counterbore() {
    BodyStore bodies;
    em::ElementMapPartition part;
    const ops::OpOutcome oc =
        run_hole(bodies, part,
                 json{{"holeType", "counterbore"},
                      {"depth", 20.0},
                      {"cbDiameter", 10.0},
                      {"cbDepth", 5.0}});
    check(oc.status == ops::OpOutcome::Status::Ok, "counterbore: Ok");
    const double removed = kPi * 3.0 * 3.0 * (20.0 - 5.0) + kPi * 5.0 * 5.0 * 5.0;
    check_rel(vol(bodies.get("body_1")->geom), kBoxVol - removed, 1e-6,
              "counterbore: 10000 − (π·3²·15 + π·5²·5)");
}

// ── 4. COUNTERSINK: the cone volume is analytic, and the derived cone DEPTH is
//      what proves csAngleDeg actually reached the kernel. 90° ⇒ tan(45°) = 1 ⇒
//      csDepth = (6 − 3) = 3 mm. ───────────────────────────────────────────────
void test_countersink() {
    ops::HoleDims dims;
    dims.diameter = 6.0;
    dims.cs_diameter = 12.0;
    dims.cs_angle_deg = 90.0;
    check_rel(ops::countersink_depth(dims), 3.0, 1e-12, "countersink: csDepth = (csR−r)/tan(45°)");
    // A shallower included angle digs DEEPER for the same diameters.
    dims.cs_angle_deg = 82.0;
    check(ops::countersink_depth(dims) > 3.0, "countersink: 82° sinks deeper than 90°");

    BodyStore bodies;
    em::ElementMapPartition part;
    const ops::OpOutcome oc = run_hole(bodies, part,
                                       json{{"holeType", "countersink"},
                                            {"depth", 20.0},
                                            {"csDiameter", 12.0},
                                            {"csAngleDeg", 90.0}});
    check(oc.status == ops::OpOutcome::Status::Ok, "countersink: Ok");
    const double r = 3.0, cs_r = 6.0, cs_depth = 3.0;
    const double frustum = (kPi * cs_depth / 3.0) * (cs_r * cs_r + cs_r * r + r * r);
    // Union of the drill and the cone: the cone's first `cs_depth` overlaps the drill.
    const double removed = kPi * r * r * 20.0 + frustum - kPi * r * r * cs_depth;
    check_rel(vol(bodies.get("body_1")->geom), kBoxVol - removed, 1e-6,
              "countersink: 10000 − (π·3²·20 + frustum − π·3²·3)");
}

// ── 5. POINT OFF FACE — both fences, both recoverable and NAMED ───────────────
void test_point_fences() {
    {  // (a) 5 mm off the face PLANE ⇒ the 1e-3 re-projection fence.
        BodyStore bodies;
        em::ElementMapPartition part;
        const ops::OpOutcome oc =
            run_hole(bodies, part, json{{"point", {10.0, 10.0, 30.0}}, {"depth", 10.0}});
        check(oc.status == ops::OpOutcome::Status::Failed, "point off plane: Failed");
        check(oc.error_code == "OP_FAILED", "point off plane: RECOVERABLE OP_FAILED");
        check(oc.error_message.find("off the resolved face plane") != std::string::npos,
              "point off plane: names the reason (got: " + oc.error_message + ")");
        check_rel(vol(bodies.get("body_1")->geom), kBoxVol, 1e-12,
                  "point off plane: host untouched");
    }
    {  // (b) ON the plane but OUTSIDE the face's boundary.
        BodyStore bodies;
        em::ElementMapPartition part;
        const ops::OpOutcome oc =
            run_hole(bodies, part, json{{"point", {50.0, 50.0, 25.0}}, {"depth", 10.0}});
        check(oc.status == ops::OpOutcome::Status::Failed, "point off face: Failed");
        check(oc.error_code == "OP_FAILED", "point off face: RECOVERABLE OP_FAILED");
        check(oc.error_message.find("outside the resolved face boundary") != std::string::npos,
              "point off face: names the reason (got: " + oc.error_message + ")");
    }
    {  // (c) A point 1e-4 off the plane is WITHIN the fence and drills normally.
        BodyStore bodies;
        em::ElementMapPartition part;
        const ops::OpOutcome oc =
            run_hole(bodies, part, json{{"point", {10.0, 10.0, 25.0001}}, {"depth", 10.0}});
        check(oc.status == ops::OpOutcome::Status::Ok, "point within fence: Ok");
        check_rel(vol(bodies.get("body_1")->geom), kBoxVol - kPi * 9.0 * 10.0, 1e-5,
                  "point within fence: re-projected onto the face, depth still exact");
    }
}

// ── 6. NON-PLANAR seat ⇒ recoverable OP_FAILED naming the reason ──────────────
void test_non_planar_face() {
    const TopoDS_Shape cyl = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", cyl);
    em::ElementMapPartition part;
    const TopoDS_Shape lateral = face_by_center(cyl, 0, 0, 10);  // the curved wall
    const json op = {
        {"opType", "Hole"},
        {"opId", "ophl"},
        {"inputs", json::array({face_input("body_1", "el_side", lateral, 10, 0, 10)})},
        {"params",
         {{"targetBodyId", "body_1"}, {"point", {10.0, 0.0, 10.0}}, {"holeType", "simple"},
          {"diameter", 4.0}, {"depth", 5.0}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_hole(ctx, op, "ophl");
    check(oc.status == ops::OpOutcome::Status::Failed, "non-planar: Failed");
    check(oc.error_code == "OP_FAILED", "non-planar: RECOVERABLE OP_FAILED");
    check(oc.error_message.find("not planar") != std::string::npos,
          "non-planar: names the reason (got: " + oc.error_message + ")");
}

// ── 7. cb/cs INVARIANTS are re-checked worker-side (independent trust boundary) ─
void test_conditional_invariants() {
    struct Case {
        json extra;
        const char* needle;
        const char* what;
    };
    const std::vector<Case> cases = {
        {json{{"holeType", "counterbore"}, {"depth", 10.0}}, "requires cbDiameter and cbDepth",
         "counterbore without cb*"},
        {json{{"holeType", "counterbore"},
              {"depth", 10.0},
              {"cbDiameter", 6.0},
              {"cbDepth", 2.0}},
         "cbDiameter must exceed diameter", "cbDiameter == diameter"},
        {json{{"holeType", "countersink"}, {"depth", 10.0}}, "requires csDiameter and csAngleDeg",
         "countersink without cs*"},
        {json{{"holeType", "countersink"},
              {"depth", 10.0},
              {"csDiameter", 12.0},
              {"csAngleDeg", 95.0}},
         "csAngleDeg must be one of", "non-standard included angle"},
        {json{{"diameter", 0.0}, {"depth", 10.0}}, "diameter too small", "zero diameter"},
        {json{{"depth", 0.0}}, "depth too small", "zero depth"},
        {json{{"holeType", "spotface"}, {"depth", 10.0}}, "holeType must be", "unknown holeType"},
    };
    for (const Case& c : cases) {
        BodyStore bodies;
        em::ElementMapPartition part;
        const ops::OpOutcome oc = run_hole(bodies, part, c.extra);
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
              std::string("invariant (") + c.what + "): recoverable OP_FAILED");
        check(oc.error_message.find(c.needle) != std::string::npos,
              std::string("invariant (") + c.what + "): names the reason (got: " +
                  oc.error_message + ")");
        check_rel(vol(bodies.get("body_1")->geom), kBoxVol, 1e-12,
                  std::string("invariant (") + c.what + "): host untouched");
    }
}

// ── The authoring-resolution boundary ─────────────────────────────────────────
// Hole's dimension floor is `GeometryPrecisionContext::authoring_resolution()`. The
// `zero depth` invariant above is refused by ANY plausible threshold, so it cannot
// prove the guard is still wired to the context. This one STRADDLES the boundary and
// flips the moment the context moves.
void test_authoring_resolution_boundary() {
    const std::string needle = "Hole depth too small";
    {
        BodyStore bodies;
        em::ElementMapPartition part;
        const ops::OpOutcome below = run_hole(bodies, part, json{{"depth", 9.9e-4}});
        check(below.status == ops::OpOutcome::Status::Failed &&
                  below.error_message.find(needle) != std::string::npos,
              "authoring boundary: depth 9.9e-4 mm is refused by name, got: " +
                  below.error_message);
    }
    {
        BodyStore bodies;
        em::ElementMapPartition part;
        const ops::OpOutcome above = run_hole(bodies, part, json{{"depth", 1.05e-3}});
        check(above.error_message.find(needle) == std::string::npos,
              "authoring boundary: depth 1.05e-3 mm clears the guard, got: " +
                  above.error_message);
    }
}

TopoDS_Shape bridge_host() {
    const TopoDS_Shape left =
        BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 8.0, 20.0, 25.0).Shape();
    const TopoDS_Shape bridge =
        BRepPrimAPI_MakeBox(gp_Pnt(8, 8, 0), 4.0, 4.0, 25.0).Shape();
    const TopoDS_Shape right =
        BRepPrimAPI_MakeBox(gp_Pnt(12, 0, 0), 8.0, 20.0, 25.0).Shape();
    BRepAlgoAPI_Fuse first(left, bridge);
    first.Build();
    BRepAlgoAPI_Fuse second(first.Shape(), right);
    second.Build();
    return second.Shape();
}

void test_result_policy_versions() {
    const TopoDS_Shape host = bridge_host();
    check(!host.IsNull() && solid_count(host) == 1,
          "result policy: connected bridge host starts as one solid");
    {
        BodyStore bodies;
        em::ElementMapPartition part;
        const ops::OpOutcome outcome = run_hole_on_host(
            bodies, part, host, json{{"depth", nullptr}});
        check(outcome.status == ops::OpOutcome::Status::Ok,
              "result policy legacy: split-host residual remains replayable");
        check(bodies.contains("body_1") &&
                  solid_count(bodies.get("body_1")->geom) == 2,
              "result policy legacy: one Body may retain two residual solids");
    }
    {
        BodyStore bodies;
        em::ElementMapPartition part;
        const ops::OpOutcome outcome = run_hole_on_host(
            bodies, part, host,
            json{{"depth", nullptr}, {"resultPolicyVersion", 2}});
        check(outcome.status == ops::OpOutcome::Status::Failed &&
                  outcome.error_code == "OP_FAILED" &&
                  !outcome.diagnostics.empty() &&
                  outcome.diagnostics.front().value("code", "") ==
                      "HOLE_DISJOINT_RESULT",
              "result policy V2: disconnected result is named-refused");
        check(outcome.body_events.empty() && bodies.contains("body_1") &&
                  bodies.get("body_1")->geom.IsSame(host),
              "result policy V2: refusal leaves predecessor untouched");
    }
    {
        BodyStore bodies;
        em::ElementMapPartition part;
        const ops::OpOutcome outcome = run_hole_on_host(
            bodies, part, host, json{{"resultPolicyVersion", 3}});
        check(outcome.status == ops::OpOutcome::Status::Failed &&
                  outcome.error_code == "OP_FAILED" &&
                  !outcome.diagnostics.empty() &&
                  outcome.diagnostics.front().value("code", "") ==
                      "UNSUPPORTED_HOLE_RESULT_POLICY_VERSION",
              "result policy future: unsupported version refuses by name");
    }
}

void test_quarantined_host_is_not_modelable() {
    const TopoDS_Shape host = box_20_20_25();
    BodyStore bodies;
    bodies.create("body_1", "op0", host);
    bodies.quarantine("body_1", "fixture failed import validation");
    em::ElementMapPartition part;
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome outcome =
        ops::execute_hole(ctx, hole_op(host, json{{"resultPolicyVersion", 2}}),
                          "ophl");
    check(outcome.status == ops::OpOutcome::Status::Failed &&
              outcome.error_code == "OP_FAILED" &&
              !outcome.diagnostics.empty() &&
              outcome.diagnostics.front().value("code", "") ==
                  "QUARANTINED_MODELING_INPUT",
          "quarantine: visible imported geometry cannot enter Hole modeling");
    check(bodies.get("body_1")->geom.IsSame(host),
          "quarantine: rejected modeling leaves imported geometry untouched");
}

// ── 8. DETERMINISM: two independent runs are BIT-EQUAL (same geometry signature).
//      This is also what makes preview == commit: `PlanExecutor::execute_candidate_op`
//      is the one path both lanes take, so an identical predecessor snapshot plus
//      identical params can only produce an identical result. ────────────────────
void test_determinism() {
    const json extra{{"holeType", "counterbore"},
                     {"depth", 18.0},
                     {"cbDiameter", 11.0},
                     {"cbDepth", 4.0}};
    std::string sig[2];
    double volume[2] = {0.0, 0.0};
    for (int i = 0; i < 2; ++i) {
        BodyStore bodies;
        em::ElementMapPartition part;
        const ops::OpOutcome oc = run_hole(bodies, part, extra);
        check(oc.status == ops::OpOutcome::Status::Ok, "determinism: Ok");
        sig[i] = onecad::session::geometry_signature(bodies);
        volume[i] = vol(bodies.get("body_1")->geom);
    }
    check(sig[0] == sig[1], "determinism: geometry signature bit-equal across runs");
    check(volume[0] == volume[1], "determinism: volume bit-equal across runs");
}

}  // namespace

int main() {
    test_simple_through_all();
    test_simple_blind();
    test_counterbore();
    test_countersink();
    test_point_fences();
    test_non_planar_face();
    test_conditional_invariants();
    test_authoring_resolution_boundary();
    test_result_policy_versions();
    test_quarantined_host_is_not_modelable();
    test_determinism();
    if (g_failures == 0) std::fprintf(stderr, "test_hole_op: all checks passed\n");
    return g_failures;
}
