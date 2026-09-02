// test_revolve_boolean_modes.cpp — MODEL-HARDEN review follow-up: pins the OTHER
// branches of `publish_boolean_result` reached from RevolveOp's boolean tail
// (commit 0278ac1 swapped RevolveOp's Add/Cut/Intersect path onto the SAME
// OpCommon.cpp publish_boolean_result as Extrude/Boolean). test_revolve_split.cpp
// already covers the MULTI-solid Cut split (`body_<opId>:<k>`); this file covers
// the single-solid "modify in place, target id preserved" branch under three
// different real-world shapes of that branch:
//   1. Add,  tool a proper subset of the target        -> volume unchanged.
//   2. Cut,  tool disjoint from the target (removes 0)  -> volume unchanged.
//   3. Intersect, reusing the split-test geometry        -> exact z-slab volume.
// (3)'s header explains why a genuinely multi-solid Add/Intersect is NOT reachable
// with clean (non-circular-arc) arithmetic for a box + concentric-washer pair, and
// why Intersect is pinned single-solid instead (task's documented fallback).
//
// Frame mapping (verified against worker Sketch.h SketchPlane + WireSketch toWorld,
// identical to test_revolve_split.cpp):
//   XY plane: toWorld(u,v) = (-v,  u, 0)   (xAxis=(0,1,0), yAxis=(-1,0,0))
//   XZ plane: toWorld(u,v) = ( 0,  u, v)   (xAxis=(0,1,0), yAxis=(0,0,1))
//
// No framework: exit code == failure count. (Mirrors test_revolve_split.cpp.)
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "session/ShapeMetrics.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::Session;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

json rect(const std::string& p, double x0, double y0, double x1, double y1) {
    return json::array({{{"id", p + "1"}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y0}}},
                        {{"id", p + "2"}, {"type", "Line"}, {"p0", {x1, y0}}, {"p1", {x1, y1}}},
                        {{"id", p + "3"}, {"type", "Line"}, {"p0", {x1, y1}}, {"p1", {x0, y1}}},
                        {{"id", p + "4"}, {"type", "Line"}, {"p0", {x0, y1}}, {"p1", {x0, y0}}}});
}
json sketch_op(const std::string& op_id, int step, const std::string& sid,
               const std::string& plane_kind, const json& ents) {
    return json{{"opType", "Sketch"}, {"opId", op_id}, {"stepIndex", step},
                {"params", {{"sketchId", sid}, {"plane", {{"kind", plane_kind}}},
                            {"entities", ents}, {"constraints", json::array()}}}};
}

// Run: Sketch(box) -> Extrude(box, NewBody, Blind +Z box_h) -> Sketch(tool profile +
// a construction axis line) -> Revolve(360deg, booleanMode, target=body_op1).
// `body_events` collects the Revolve step's bodyEvents. (Session holds a mutex ⇒
// non-movable, passed by reference.) Generalizes test_revolve_split.cpp's run_plan
// so each case below only states its box/axis/profile numbers + booleanMode.
Envelope run_plan(Session& s, std::vector<json>& body_events,
                  double box_u0, double box_v0, double box_u1, double box_v1, double box_h,
                  double axis_u, double axis_v0, double axis_v1,
                  double prof_u0, double prof_v0, double prof_u1, double prof_v1,
                  const std::string& boolean_mode) {
    s.open("doc", 0, 3, "determinism");

    json tool_ents = rect("t", prof_u0, prof_v0, prof_u1, prof_v1);
    tool_ents.push_back(json{{"id", "taxis"}, {"type", "Line"},
                             {"p0", {axis_u, axis_v0}}, {"p1", {axis_u, axis_v1}}});

    json ops = json::array({
        sketch_op("op0", 0, "sk_a", "XY", rect("a", box_u0, box_v0, box_u1, box_v1)),
        json{{"opType", "Extrude"}, {"opId", "op1"}, {"stepIndex", 1},
             {"params", {{"sketchId", "sk_a"}, {"distance", box_h},
                         {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}},
        sketch_op("op2", 2, "sk_t", "XZ", tool_ents),
        json{{"opType", "Revolve"}, {"opId", "op3"}, {"stepIndex", 3},
             {"params", {{"sketchId", "sk_t"}, {"angleDeg", 360.0}, {"booleanMode", boolean_mode},
                         {"targetBodyId", "body_op1"},
                         {"axis", {{"kind", "sketchLine"}, {"sketchId", "sk_t"},
                                   {"lineId", "taxis"}}}}}},
    });

    CancelToken tok;
    auto capture = [&body_events](Envelope& ev) {
        if (ev.event_name == "planStep" && ev.result.contains("bodyEvents")) {
            body_events.clear();
            for (const auto& be : ev.result["bodyEvents"]) body_events.push_back(be);
        }
    };
    HandlerContext ctx{tok, [](int) {}, capture};
    json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({"a", "b", "c", "d"})},
                 {"targetStep", 3}, {"ops", ops}};
    Envelope prepared =
        onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    return prepared;
}

double vol_of(Session& s, const std::string& bid) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get(bid);
    if (!rec) return -1.0;
    return onecad::session::shape_volume(rec->geom);
}

// Common shape of the single-solid `publish_boolean_result` branch: exactly one
// "modified" event naming the ORIGINAL target id, no "created"/"deleted" (no split).
void check_single_modify(const std::vector<json>& body_events, const std::string& target_id,
                         const std::string& tag) {
    int created = 0, deleted = 0, modified = 0;
    bool target_modified = false;
    for (const json& be : body_events) {
        const std::string kind = be.value("kind", "");
        const std::string bid = be.value("bodyId", "");
        if (kind == "created") ++created;
        if (kind == "deleted") ++deleted;
        if (kind == "modified") { ++modified; if (bid == target_id) target_modified = true; }
    }
    check(modified == 1 && created == 0 && deleted == 0,
          tag + ": single-solid result -> exactly one modified event, no split");
    check(target_modified, tag + ": modified event names the ORIGINAL target id " + target_id);
}

// --- Case 1: Add, tool a proper interior subset of the target -> volume unchanged. ---
void test_add_tool_fully_inside() {
    // Box (XY, rect u∈[-10,40] v∈[-10,10], Blind +Z 30):
    //   world x=-v∈[-10,10], y=u∈[-10,40], z∈[0,30]  →  vol 20·50·30 = 30000.
    //
    // Tool (XZ sketch, 360° revolve): axis line u=0  →  world axis at (x=0,y=0)
    // parallel to Z (through the world origin, since XZ toWorld(0,v)=(0,0,v)).
    // Profile rect u∈[2,8], v∈[5,25]  →  world y∈[2,8], z∈[5,25], x=0. Radial
    // distance from the axis r(x,y) = √(x²+y²); the profile spans r=|y| ∈ [2,8]  →
    // a 360° washer (annular disc): inner r=2, outer r=8, z-slab z∈[5,25].
    //
    // Washer bounding box: x,y ∈ [-8,8], z ∈ [5,25]. Margins to the target box on
    // every side are strictly positive (x: 2, y: 2 near / 32 far, z: 5 / 5) ⇒ the
    // washer is a PROPER INTERIOR SUBSET of the target — no shared boundary.
    //
    // Add(target, tool) with tool ⊂ int(target) contributes NO new volume (the tool
    // is already inside solid material) ⇒ result volume == target volume == 30000
    // EXACTLY, and (being fully enclosed, no new external boundary) the boolean
    // collapses to a single solid — the "modify in place" branch, never the split
    // branch.
    Session s;
    std::vector<json> body_events;
    run_plan(s, body_events,
             /*box u0,v0,u1,v1,h*/ -10, -10, 40, 10, 30.0,
             /*axis u,v0,v1*/ 0, 0, 20,
             /*profile u0,v0,u1,v1*/ 2, 5, 8, 25,
             "Add");

    check_single_modify(body_events, "body_op1", "revolve-add");

    const onecad::session::BodyStore bodies = s.bodies_copy();
    check(bodies.contains("body_op1"), "revolve-add: target id survives publish_boolean_result");
    check(!bodies.contains("body_op3:0"), "revolve-add: no split children minted");

    const double v = vol_of(s, "body_op1");
    check(std::abs(v - 30000.0) < 1e-3,
          "revolve-add: fused volume == target volume 30000 (tool fully inside, adds nothing)");
}

// --- Case 2: Cut, tool entirely disjoint from the target -> removes nothing. ---
void test_cut_removes_nothing() {
    // Same target box as test_add_tool_fully_inside: world x∈[-10,10], y∈[-10,40],
    // z∈[0,30], vol 30000. Farthest box corner from the axis (0,0) is (±10,40) at
    // r = √(10² + 40²) = √1700 ≈ 41.23.
    //
    // Tool profile u∈[50,60], v∈[5,25]  →  washer inner r=50, outer r=60. Since
    // 50 > 41.23, the washer's material (r ∈ [50,60]) never reaches ANY point of the
    // box footprint, at any z ⇒ the tool is entirely disjoint from the target.
    //
    // A boolean Cut of disjoint operands removes nothing: OCCT returns the target
    // shape unchanged (still a single solid) ⇒ modified event, target id preserved,
    // volume EXACTLY 30000, no children minted, no delete.
    Session s;
    std::vector<json> body_events;
    run_plan(s, body_events,
             -10, -10, 40, 10, 30.0,
             0, 0, 20,
             50, 5, 60, 25,
             "Cut");

    check_single_modify(body_events, "body_op1", "revolve-cut-miss");

    const onecad::session::BodyStore bodies = s.bodies_copy();
    check(bodies.contains("body_op1"), "revolve-cut-miss: target id survives");
    check(!bodies.contains("body_op3:0"), "revolve-cut-miss: no split children minted");

    const double v = vol_of(s, "body_op1");
    check(std::abs(v - 30000.0) < 1e-3,
          "revolve-cut-miss: volume unchanged at 30000 (disjoint tool removes nothing)");
}

// --- Case 3: Intersect, reusing test_revolve_split.cpp's box+washer geometry. ---
void test_intersect_single_solid_zslab() {
    // Reuses the EXACT box+washer numbers from test_revolve_split.cpp's Cut case
    // (see that file's header for the full derivation) with booleanMode=Intersect:
    //   Box A (XY, u∈[0,40] v∈[0,20], Blind +Z 25): world x∈[-20,0], y∈[0,40],
    //     z∈[0,25]  →  vol 20000.
    //   Tool (XZ, axis u=-10, profile u∈[-9,60] v∈[10,14]): 360° washer, inner r=1,
    //     outer r=70, z-slab z∈[10,14].
    //   The box footprint's radial range from the axis is [10, √2900≈53.85] ⊂
    //   [1,70] — the washer's ANNULAR (circular) boundary never crosses the box
    //   footprint at all; the only surface that clips the box is the washer's FLAT
    //   z-slab pair (z=10, z=14). That is exactly why the Cut variant produces two
    //   clean box prisms (8000/8800, test_revolve_split.cpp) instead of a
    //   circular-arc-bounded remainder: a flat plane, not a cylinder, does the
    //   cutting.
    //
    //   The SAME "footprint fully inside the radial band" property makes
    //   Intersect(A, washer) a pure z-slab clip: the kept region is exactly the part
    //   of A with z∈[10,14] (A's own unclipped x/y extents) — a SINGLE connected
    //   solid, vol = 20·40·(14-10) = 3200 EXACTLY.
    //
    //   Why this suite pins Intersect single-solid rather than a genuine multi-solid
    //   Add/Intersect (the task's documented "else" fallback): a revolve tool is
    //   rotationally symmetric, so the ONLY way an Intersect/Add against a convex
    //   box target can itself produce >1 disjoint solid is for the tool's circular
    //   (inner-radius) boundary to pass THROUGH the box footprint — but every exact
    //   volume in this file (and in test_revolve_split.cpp) depends on the box
    //   footprint staying entirely on one side of that boundary (either fully inside
    //   the radial band, as here, or fully outside it, as in test_cut_removes_
    //   nothing), specifically so the cut/intersect surface stays a FLAT plane and
    //   the resulting volume is exact prism arithmetic, not a circular-segment
    //   integral. A flat-plane clip of a single convex box can remove a slab (one
    //   piece kept, Intersect) or bisect it into two prisms (both pieces kept, Cut)
    //   — it can never leave Intersect/Add with two disjoint pieces, since Add only
    //   ever ADDS material (monotone, no matter how the tool is shaped, if the
    //   target was one solid it stays one solid or grows) and this Intersect keeps
    //   exactly the single slab between the two flat cuts. Cut is therefore the only
    //   reachable multi-solid case for this geometry family; Intersect is pinned
    //   single-solid here instead, per plan.
    Session s;
    std::vector<json> body_events;
    run_plan(s, body_events,
             0, 0, 40, 20, 25.0,
             -10, 0, 20,
             -9, 10, 60, 14,
             "Intersect");

    check_single_modify(body_events, "body_op1", "revolve-intersect");

    const onecad::session::BodyStore bodies = s.bodies_copy();
    check(bodies.contains("body_op1"), "revolve-intersect: target id survives");
    check(!bodies.contains("body_op3:0"), "revolve-intersect: no split children minted");

    const double v = vol_of(s, "body_op1");
    check(std::abs(v - 3200.0) < 1e-3,
          "revolve-intersect: volume == 3200 (20·40·4 z-slab, exact — radial band non-binding)");
}

// --- Case 4: a disjoint Intersect produces zero solids and must refuse. ---
void test_intersect_disjoint_refuses() {
    // The target and radial washer are exactly the disjoint Cut fixture above; an
    // Intersect would be a valid-but-empty OCCT compound. It must not replace the
    // target with an unmeshable empty body or consume any existing body.
    Session s;
    std::vector<json> body_events;
    const Envelope prepared = run_plan(s, body_events,
                                       -10, -10, 40, 10, 30.0,
                                       0, 0, 20,
                                       50, 5, 60, 25,
                                       "Intersect");

    check(prepared.ok == true, "revolve-empty: recoverable operation failure still prepares prefix");
    check(prepared.result.value("stoppedReason", "") == "opFailed",
          "revolve-empty: stops at the refused revolve step");
    const json& steps = prepared.result["perStepResults"];
    check(steps.size() == 4 && steps[3].value("status", "") == "opFailed",
          "revolve-empty: failed step is reported as opFailed");
    check(steps.size() == 4 &&
              steps[3].value("message", "") == "Revolve Intersect: the result would be empty",
          "revolve-empty: refusal reason reaches the plan result");
    // WP-C: the empty result is a NAMED refusal (never a silent `deleted`).
    bool named = false;
    if (steps.size() == 4 && steps[3].contains("diagnostics")) {
        for (const json& d : steps[3]["diagnostics"]) named = named || d.value("code", "") == "REVOLVE_EMPTY_RESULT";
    }
    check(named, "revolve-empty: diagnostic REVOLVE_EMPTY_RESULT rides the failed step");

    const onecad::session::BodyStore bodies = s.bodies_copy();
    check(bodies.contains("body_op1") && bodies.all().size() == 1,
          "revolve-empty: prefix target remains the only published body");
    check(std::abs(vol_of(s, "body_op1") - 30000.0) < 1e-3,
          "revolve-empty: target geometry is unchanged");
}

// --- Case 5: an unknown booleanMode is a MALFORMED RECORD, never a NewBody. ---
void test_unknown_boolean_mode_refuses() {
    // The lenient mapping silently minted a fresh body here: a user (or a producer
    // bug) asking for "Subtract" would get a floating revolved solid instead of a cut,
    // with the target left untouched and no diagnostic anywhere.
    Session s;
    std::vector<json> body_events;
    const Envelope prepared = run_plan(s, body_events,
                                       -10, -10, 40, 10, 30.0,
                                       0, 0, 20,
                                       2, 5, 8, 25,
                                       "Subtract");

    check(prepared.ok == true, "revolve-mode: recoverable failure still prepares the prefix");
    check(prepared.result.value("stoppedReason", "") == "opFailed",
          "revolve-mode: stops at the refused revolve step");
    const json& steps = prepared.result["perStepResults"];
    check(steps.size() == 4 && steps[3].value("status", "") == "opFailed",
          "revolve-mode: failed step is reported as opFailed");
    check(steps.size() == 4 &&
              steps[3].value("message", "").find("unknown boolean mode 'Subtract'") !=
                  std::string::npos,
          "revolve-mode: refusal names the offending token");

    const onecad::session::BodyStore bodies = s.bodies_copy();
    check(bodies.all().size() == 1 && bodies.contains("body_op1"),
          "revolve-mode: no body minted by an unknown mode");
}

// --- Case 6: an axis sketch whose constraints do not solve must refuse. ---
void test_unsolved_axis_sketch_refuses() {
    // The axis lives in its OWN sketch, so the profile builder (which does check its
    // solve) cannot shadow the axis path. Two contradictory Distance constraints on
    // the SAME endpoint pair translate cleanly but cannot be satisfied, so `solve()`
    // reports failure. Consuming the endpoints anyway would build the revolution
    // around stale seed positions — and a wrong-but-nonzero axis sails past the
    // degenerate-length guard.
    Session s;
    s.open("doc", 0, 3, "determinism");

    json axis_ents = json::array({json{{"id", "ax"}, {"type", "Line"},
                                       {"p0", {0.0, 0.0}}, {"p1", {0.0, 20.0}}}});
    json axis_cons = json::array({
        json{{"id", "d1"}, {"type", "Distance"}, {"entities", json::array({"ax", "ax"})},
             {"positions", json::array({"Start", "End"})}, {"value", 10.0}},
        json{{"id", "d2"}, {"type", "Distance"}, {"entities", json::array({"ax", "ax"})},
             {"positions", json::array({"Start", "End"})}, {"value", 30.0}},
    });

    json ops = json::array({
        sketch_op("op0", 0, "sk_a", "XY", rect("a", -10, -10, 40, 10)),
        json{{"opType", "Extrude"}, {"opId", "op1"}, {"stepIndex", 1},
             {"params", {{"sketchId", "sk_a"}, {"distance", 30.0},
                         {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}},
        sketch_op("op2", 2, "sk_t", "XZ", rect("t", 2, 5, 8, 25)),
        json{{"opType", "Sketch"}, {"opId", "op3"}, {"stepIndex", 3},
             {"params", {{"sketchId", "sk_axis"}, {"plane", {{"kind", "XZ"}}},
                         {"entities", axis_ents}, {"constraints", axis_cons}}}},
        json{{"opType", "Revolve"}, {"opId", "op4"}, {"stepIndex", 4},
             {"params", {{"sketchId", "sk_t"}, {"angleDeg", 360.0},
                         {"booleanMode", "Cut"}, {"targetBodyId", "body_op1"},
                         {"axis", {{"kind", "sketchLine"}, {"sketchId", "sk_axis"},
                                   {"lineId", "ax"}}}}}},
    });

    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({"a", "b", "c", "d", "e"})},
                 {"targetStep", 4}, {"ops", ops}};
    const Envelope prepared =
        onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);

    check(prepared.ok == true, "revolve-axis: recoverable failure still prepares the prefix");
    check(prepared.result.value("stoppedReason", "") == "opFailed",
          "revolve-axis: stops at the refused revolve step");
    const json& steps = prepared.result["perStepResults"];
    const std::string message = steps.size() == 5 ? steps[4].value("message", "") : "";
    check(steps.size() == 5 && steps[4].value("status", "") == "opFailed",
          "revolve-axis: failed step is reported as opFailed (got '" + message + "')");
    check(message.find("axis sketch solve failed") != std::string::npos,
          "revolve-axis: refusal names the unsolved axis sketch (got '" + message + "')");
}

}  // namespace

int main() {
    test_add_tool_fully_inside();
    test_cut_removes_nothing();
    test_intersect_single_solid_zslab();
    test_intersect_disjoint_refuses();
    test_unknown_boolean_mode_refuses();
    test_unsolved_axis_sketch_refuses();
    if (g_failures == 0) std::fprintf(stderr, "revolve_boolean_modes: OK\n");
    // An exit status that is a multiple of 256 reports PASS to the shell, so the raw
    // failure count must never be returned directly.
    return g_failures == 0 ? 0 : 1;
}
