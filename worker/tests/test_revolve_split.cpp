// test_revolve_split.cpp — MODEL-HARDEN W3: a boolean-mode Revolve Cut whose result
// is a multi-solid compound must mint deterministic split children `body_<opId>:<k>`
// (SCHEMA §2/§7.2, D1) — EXACT parity with a boolean-mode Extrude Cut / a Boolean op.
// Proves the RevolveOp `publish_boolean_result` fan-out (was a direct in-place
// `bodies.create`, which kept two disconnected pieces under one body id).
//
// Frame mapping (verified against worker Sketch.h SketchPlane + WireSketch toWorld):
//   XY plane: toWorld(u,v) = (-v,  u, 0)   (xAxis=(0,1,0), yAxis=(-1,0,0))
//   XZ plane: toWorld(u,v) = ( 0,  u, v)   (xAxis=(0,1,0), yAxis=(0,0,1))
//
// Box A (XY, rect u∈[0,40] v∈[0,20], Blind +Z 25):
//   world x∈[-20,0], y∈[0,40], z∈[0,25]  →  vol 20·40·25 = 20000.
//
// Tool (XZ sketch, 360° revolve):
//   axis line u=-10  →  world axis through (0,-10,·) parallel to world Z.
//   profile rect u∈[-9,60], v∈[10,14]  →  world x=0, y∈[-9,60], z∈[10,14].
//   Radial distance from the axis r(x,y) = √(x² + (y+10)²); the profile spans
//   r=|y+10| ∈ [1,70]  →  a 360° washer (annular disc): inner r=1, outer r=70,
//   z-slab z∈[10,14].
//
// Box footprint (x∈[-20,0], y∈[0,40]) vs the axis at (0,-10):
//   min r = 10 (at (0,0)); max r = √(20²+50²) = √2900 ≈ 53.85 (at (-20,40)).
//   [10, 53.85] ⊂ [1, 70]  →  the disc fully spans the box cross-section over the
//   whole z-slab, so Cut(A, disc) removes z∈[10,14] across the ENTIRE section and
//   splits A into two exact box children:
//     bottom  z∈[0,10]  →  20·40·10 = 8000
//     top     z∈[14,25] →  20·40·11 = 8800
//   Distinct volumes ⇒ deterministic `ordered_solids` ordering (volume ascending):
//     :0 = 8000 (bottom),  :1 = 8800 (top).
//
// No framework: exit code == failure count. (Mirrors test_wp6_split.cpp.)
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

// Run the revolve-split plan into `s`; `body_events` collects the revolve step's
// bodyEvents. (Session holds a mutex ⇒ non-movable, passed by reference.)
void run_plan(Session& s, std::vector<json>& body_events) {
    s.open("doc", 0, 3, "determinism");

    // The revolve tool sketch: rect profile + a separate construction axis line at
    // u=-10 (outside the box footprint; -10 < -9 so it never touches the profile).
    json tool_ents = rect("t", -9, 10, 60, 14);
    tool_ents.push_back(json{{"id", "taxis"}, {"type", "Line"},
                             {"p0", {-10, 0}}, {"p1", {-10, 20}}});

    json ops = json::array({
        sketch_op("op0", 0, "sk_a", "XY", rect("a", 0, 0, 40, 20)),
        json{{"opType", "Extrude"}, {"opId", "op1"}, {"stepIndex", 1},
             {"params", {{"sketchId", "sk_a"}, {"distance", 25.0},
                         {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}},
        sketch_op("op2", 2, "sk_t", "XZ", tool_ents),
        json{{"opType", "Revolve"}, {"opId", "op3"}, {"stepIndex", 3},
             {"params", {{"sketchId", "sk_t"}, {"angleDeg", 360.0}, {"booleanMode", "Cut"},
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
    onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
}

double vol_of(Session& s, const std::string& bid) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get(bid);
    if (!rec) return -1.0;
    return onecad::session::shape_volume(rec->geom);
}
}  // namespace

int main() {
    Session s1;
    std::vector<json> body_events;
    run_plan(s1, body_events);

    // The revolve step's bodyEvents: parent Deleted + two children Created. Unlike a
    // standalone Boolean op there is NO separate tool body (the revolve builds its own
    // tool), so exactly ONE Deleted (the parent box).
    int created = 0, deleted = 0;
    bool has_c0 = false, has_c1 = false, parent_deleted = false;
    for (const json& be : body_events) {
        const std::string kind = be.value("kind", "");
        const std::string bid = be.value("bodyId", "");
        if (kind == "created") ++created;
        if (kind == "deleted") ++deleted;
        if (kind == "created" && bid == "body_op3:0") has_c0 = true;
        if (kind == "created" && bid == "body_op3:1") has_c1 = true;
        if (kind == "deleted" && bid == "body_op1") parent_deleted = true;
    }
    check(created == 2, "revolve-split: two children Created");
    check(has_c0 && has_c1, "revolve-split: children are body_op3:0 + body_op3:1 (deterministic)");
    check(parent_deleted, "revolve-split: parent body_op1 Deleted");
    check(deleted == 1, "revolve-split: exactly the parent Deleted (revolve tool is internal)");

    // The parent is gone; only the two children survive at head.
    const onecad::session::BodyStore bodies = s1.bodies_copy();
    check(!bodies.contains("body_op1"), "revolve-split: parent body removed from head");
    check(bodies.contains("body_op3:0") && bodies.contains("body_op3:1"),
          "revolve-split: both children present at head");

    // Volumes are exact box arithmetic and ORDERED by volume (ascending): :0 is the
    // bottom slab (8000), :1 the top slab (8800). Distinct ⇒ no centroid tiebreak.
    const double v0 = vol_of(s1, "body_op3:0");
    const double v1 = vol_of(s1, "body_op3:1");
    check(std::abs(v0 - 8000.0) < 1e-3, "revolve-split: child :0 volume == 8000 (bottom slab z∈[0,10])");
    check(std::abs(v1 - 8800.0) < 1e-3, "revolve-split: child :1 volume == 8800 (top slab z∈[14,25])");

    // Ids + volumes stable across a replay (pure function of opId + ordinal).
    Session s2;
    std::vector<json> body_events2;
    run_plan(s2, body_events2);
    const onecad::session::BodyStore bodies2 = s2.bodies_copy();
    check(bodies2.contains("body_op3:0") && bodies2.contains("body_op3:1"),
          "revolve-split: child ids identical across a replay");
    check(std::abs(vol_of(s2, "body_op3:0") - 8000.0) < 1e-3,
          "revolve-split: replay child :0 volume stable");
    check(std::abs(vol_of(s2, "body_op3:1") - 8800.0) < 1e-3,
          "revolve-split: replay child :1 volume stable");

    if (g_failures == 0) std::fprintf(stderr, "revolve_split: OK\n");
    // An exit status that is a multiple of 256 reports PASS to the shell, so the raw
    // failure count must never be returned directly.
    return g_failures == 0 ? 0 : 1;
}
