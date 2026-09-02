// test_extrude_boolean_modes.cpp — kernel-hardening WP-C/WP-D red-first probes.
// Pins the boolean RESULT POLICY reached from ExtrudeOp (and, for the shared branch,
// BooleanOp) and the CONSTRUCTION of Symmetric / two-direction / ThroughAll prisms:
//   1. Add whose tool does not touch the target REFUSES by name; the target keeps its
//      id, its geometry and its partition (never a split into body_<op>:0/:1).
//   2. Cut / Intersect that leave no material REFUSE by name; the target is untouched
//      (aligned with BooleanOp's BOOLEAN_EMPTY_RESULT — never a silent `deleted`).
//   3. Symmetric and two-direction Blind/Blind prisms are ONE prism: 6 faces / 12
//      edges / 8 vertices for a rectangle, never a fuse of two halves with a
//      mid-plane seam.
//   4. ThroughAll with NewBody (no reference body) refuses by name; ThroughAll with
//      Add ends EXACTLY at the target's far extent (no overshoot material).
//   5. A two-direction extrude with a negative leg refuses by name.
//   6. A Cut that leaves two solids touching along ONE EDGE is a legitimate split
//      (manifold-ness is a per-solid property), not PUBLICATION_OPEN_MANIFOLD.
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepBndLib.hxx>
#include <Bnd_Box.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

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
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.6f want %.6f tol %.6f)\n", msg.c_str(), got, want, tol);
        ++g_failures;
    }
}
constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

json rect(const std::string& p, double x0, double y0, double x1, double y1) {
    return json::array({{{"id", p + "1"}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y0}}},
                        {{"id", p + "2"}, {"type", "Line"}, {"p0", {x1, y0}}, {"p1", {x1, y1}}},
                        {{"id", p + "3"}, {"type", "Line"}, {"p0", {x1, y1}}, {"p1", {x0, y1}}},
                        {{"id", p + "4"}, {"type", "Line"}, {"p0", {x0, y1}}, {"p1", {x0, y0}}}});
}
json tri(const std::string& p, double x0, double y0, double x1, double y1, double x2, double y2) {
    return json::array({{{"id", p + "1"}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y1}}},
                        {{"id", p + "2"}, {"type", "Line"}, {"p0", {x1, y1}}, {"p1", {x2, y2}}},
                        {{"id", p + "3"}, {"type", "Line"}, {"p0", {x2, y2}}, {"p1", {x0, y0}}}});
}
json sketch_op(const std::string& op_id, int step, const std::string& sid,
               const std::string& plane_kind, const json& ents, double offset = 0.0) {
    // The worker's XY frame is toWorld(u,v) = (-v, u, offset): xAxis (0,1,0), yAxis
    // (-1,0,0), normal (0,0,1). An offset plane is the same frame lifted along Z.
    json plane = {{"kind", plane_kind}};
    if (offset != 0.0) {
        plane = json{{"kind", "custom"}, {"origin", {0.0, 0.0, offset}}, {"xAxis", {0.0, 1.0, 0.0}},
                     {"yAxis", {-1.0, 0.0, 0.0}}, {"normal", {0.0, 0.0, 1.0}}};
    }
    return json{{"opType", "Sketch"}, {"opId", op_id}, {"stepIndex", step},
                {"params", {{"sketchId", sid}, {"plane", plane},
                            {"entities", ents}, {"constraints", json::array()}}}};
}
json body_input(const std::string& bid) {
    return json::array({json{{"primary", {{"bodyId", bid}, {"elementId", bid}, {"kind", "body"}}}}});
}
json extrude_op(const std::string& op_id, int step, const std::string& sid, json params,
                const std::string& target = "") {
    params["sketchId"] = sid;
    json op = {{"opType", "Extrude"}, {"opId", op_id}, {"stepIndex", step}, {"params", params}};
    if (!target.empty()) {
        op["inputs"] = body_input(target);
        op["params"]["targetBodyId"] = target;
    }
    return op;
}

struct Run {
    Envelope resp;
    std::vector<json> body_events;  // last planStep's bodyEvents
};

Run run_plan(Session& s, const json& ops) {
    s.open("doc", 0, 3, "determinism");
    Run run;
    CancelToken tok;
    auto capture = [&run](Envelope& ev) {
        if (ev.event_name == "planStep" && ev.result.contains("bodyEvents")) {
            run.body_events.clear();
            for (const auto& be : ev.result["bodyEvents"]) run.body_events.push_back(be);
        }
    };
    HandlerContext ctx{tok, [](int) {}, capture};
    json hashes = json::array();
    for (std::size_t i = 0; i < ops.size(); ++i) hashes.push_back("h" + std::to_string(i));
    json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty}, {"prefixHashes", hashes},
                 {"targetStep", ops.size() - 1}, {"ops", ops}};
    run.resp = onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    return run;
}

std::string stopped(const Run& r) { return r.resp.result.value("stoppedReason", std::string("?")); }

// Every diagnostic code carried by perStepResults (a failed step emits no planStep).
std::vector<std::string> step_codes(const Run& r) {
    std::vector<std::string> out;
    if (!r.resp.result.contains("perStepResults")) return out;
    for (const json& step : r.resp.result["perStepResults"]) {
        if (!step.contains("diagnostics")) continue;
        for (const json& d : step["diagnostics"]) out.push_back(d.value("code", ""));
    }
    return out;
}
bool has_code(const Run& r, const std::string& code) {
    for (const std::string& c : step_codes(r)) if (c == code) return true;
    return false;
}
std::string codes_joined(const Run& r) {
    std::string s;
    for (const std::string& c : step_codes(r)) { if (!s.empty()) s += ","; s += c; }
    return s.empty() ? "<none>" : s;
}

struct Topo { int faces = 0, edges = 0, vertices = 0, solids = 0; };
Topo topo_of(Session& s, const std::string& bid) {
    Topo t;
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get(bid);
    if (!rec) return t;
    TopTools_IndexedMapOfShape m;
    TopExp::MapShapes(rec->geom, TopAbs_FACE, m); t.faces = m.Extent(); m.Clear();
    TopExp::MapShapes(rec->geom, TopAbs_EDGE, m); t.edges = m.Extent(); m.Clear();
    TopExp::MapShapes(rec->geom, TopAbs_VERTEX, m); t.vertices = m.Extent(); m.Clear();
    TopExp::MapShapes(rec->geom, TopAbs_SOLID, m); t.solids = m.Extent();
    return t;
}
double vol_of(Session& s, const std::string& bid) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get(bid);
    return rec ? onecad::session::shape_volume(rec->geom) : -1.0;
}
double zmax_of(Session& s, const std::string& bid) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get(bid);
    if (!rec) return NAN;
    Bnd_Box box;
    BRepBndLib::Add(rec->geom, box);
    Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
    box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    return zmax;
}
int events_of(const Run& r, const std::string& kind) {
    int n = 0;
    for (const json& be : r.body_events) if (be.value("kind", "") == kind) ++n;
    return n;
}

// Base body used by the boolean cases: 20×20×10 box body_op1 (volume 4000).
json base_ops() {
    return json::array({sketch_op("op0", 0, "sk_a", "XY", rect("a", 0, 0, 20, 20)),
                        extrude_op("op1", 1, "sk_a",
                                   {{"distance", 10.0}, {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}})});
}

// --- 1. Add with a 5 mm gap: refuse, target intact, no split. --------------------
void test_add_disjoint_refuses() {
    Session s;
    json ops = base_ops();
    // 4×4 boss on a plane 15 mm above the box, extruded 10 mm UP (away): 5 mm gap.
    ops.push_back(sketch_op("op2", 2, "sk_b", "XY", rect("b", 8, 8, 12, 12), 15.0));
    ops.push_back(extrude_op("op3", 3, "sk_b",
                             {{"distance", 10.0}, {"extrudeMode", "Blind"}, {"booleanMode", "Add"}},
                             "body_op1"));
    Run r = run_plan(s, ops);
    check(stopped(r) == "opFailed", "disjoint Add: step refused (stoppedReason=" + stopped(r) + ")");
    check(has_code(r, "EXTRUDE_ADD_DISJOINT"),
          "disjoint Add: diagnostic EXTRUDE_ADD_DISJOINT (got " + codes_joined(r) + ")");
    check(s.bodies_copy().contains("body_op1"), "disjoint Add: target id survives");
    check(s.bodies_copy().size() == 1, "disjoint Add: exactly one body (no split children)");
    check_near(vol_of(s, "body_op1"), 4000.0, 1e-6, "disjoint Add: target geometry untouched");
}

// --- 2. Cut consuming the whole target / empty Intersect: refuse, target intact. --
void test_empty_result_refuses() {
    {   // Cut: a 40×40 profile at z=-5 extruded 30 up swallows the whole box.
        Session s;
        json ops = base_ops();
        ops.push_back(sketch_op("op2", 2, "sk_c", "XY", rect("c", -10, -10, 30, 30), -5.0));
        ops.push_back(extrude_op("op3", 3, "sk_c",
                                 {{"distance", 30.0}, {"extrudeMode", "Blind"}, {"booleanMode", "Cut"}},
                                 "body_op1"));
        Run r = run_plan(s, ops);
        check(stopped(r) == "opFailed", "full-consumption Cut: refused (stoppedReason=" + stopped(r) + ")");
        check(has_code(r, "EXTRUDE_EMPTY_RESULT"),
              "full-consumption Cut: diagnostic EXTRUDE_EMPTY_RESULT (got " + codes_joined(r) + ")");
        check(s.bodies_copy().contains("body_op1"), "full-consumption Cut: target NOT deleted");
        check_near(vol_of(s, "body_op1"), 4000.0, 1e-6, "full-consumption Cut: target untouched");
    }
    {   // Intersect with a tool above the box (z 15..20): empty common.
        Session s;
        json ops = base_ops();
        ops.push_back(sketch_op("op2", 2, "sk_i", "XY", rect("i", 5, 5, 15, 15), 15.0));
        ops.push_back(extrude_op("op3", 3, "sk_i",
                                 {{"distance", 5.0}, {"extrudeMode", "Blind"}, {"booleanMode", "Intersect"}},
                                 "body_op1"));
        Run r = run_plan(s, ops);
        check(stopped(r) == "opFailed", "empty Intersect: refused (stoppedReason=" + stopped(r) + ")");
        check(has_code(r, "EXTRUDE_EMPTY_RESULT"),
              "empty Intersect: diagnostic EXTRUDE_EMPTY_RESULT (got " + codes_joined(r) + ")");
        check(s.bodies_copy().contains("body_op1"), "empty Intersect: target NOT deleted");
    }
}

// --- 3. Symmetric / two-direction: one prism, box topology. -----------------------
void test_symmetric_and_two_direction_topology() {
    {
        Session s;
        json ops = json::array({sketch_op("op0", 0, "sk", "XY", rect("s", 0, 0, 40, 20)),
                                extrude_op("op1", 1, "sk",
                                           {{"distance", 30.0}, {"extrudeMode", "Symmetric"},
                                            {"booleanMode", "NewBody"}})});
        Run r = run_plan(s, ops);
        check(stopped(r) == "completed", "symmetric: completed");
        const Topo t = topo_of(s, "body_op1");
        check(t.faces == 6 && t.edges == 12 && t.vertices == 8,
              "symmetric 40x20x30: 6/12/8 topology (got " + std::to_string(t.faces) + "/" +
                  std::to_string(t.edges) + "/" + std::to_string(t.vertices) + ")");
        check_near(vol_of(s, "body_op1"), 24000.0, 1e-6, "symmetric: volume 24000");
        check_near(zmax_of(s, "body_op1"), 15.0, 1e-6, "symmetric: spans +15 (total = distance)");
    }
    {
        Session s;
        json ops = json::array({sketch_op("op0", 0, "sk", "XY", rect("s", 0, 0, 10, 10)),
                                extrude_op("op1", 1, "sk",
                                           {{"distance", 5.0}, {"extrudeMode", "Blind"},
                                            {"booleanMode", "NewBody"}, {"twoDirections", true},
                                            {"extrudeMode2", "Blind"}, {"distance2", 3.0}})});
        Run r = run_plan(s, ops);
        check(stopped(r) == "completed", "two-direction: completed");
        const Topo t = topo_of(s, "body_op1");
        check(t.faces == 6 && t.edges == 12 && t.vertices == 8,
              "two-direction 10x10 (+5/-3): 6/12/8 topology (got " + std::to_string(t.faces) + "/" +
                  std::to_string(t.edges) + "/" + std::to_string(t.vertices) + ")");
        check_near(vol_of(s, "body_op1"), 800.0, 1e-6, "two-direction: volume 800");
    }
}

// --- 4. ThroughAll: NewBody refuses; Add ends exactly at the far face. -------------
void test_through_all_policy() {
    {
        Session s;
        json ops = json::array({sketch_op("op0", 0, "sk", "XY", rect("s", 0, 0, 10, 10)),
                                extrude_op("op1", 1, "sk",
                                           {{"distance", 1.0}, {"extrudeMode", "ThroughAll"},
                                            {"booleanMode", "NewBody"}})});
        Run r = run_plan(s, ops);
        check(stopped(r) == "opFailed", "ThroughAll NewBody: refused (stoppedReason=" + stopped(r) + ")");
        check(has_code(r, "EXTRUDE_THROUGH_ALL_NO_TARGET"),
              "ThroughAll NewBody: diagnostic EXTRUDE_THROUGH_ALL_NO_TARGET (got " + codes_joined(r) + ")");
        check(s.bodies_copy().size() == 0, "ThroughAll NewBody: nothing published");
    }
    {   // Add: 4×4 profile on the box's own base plane, ThroughAll up → exactly the box.
        Session s;
        json ops = base_ops();
        ops.push_back(sketch_op("op2", 2, "sk_p", "XY", rect("p", 8, 8, 12, 12)));
        ops.push_back(extrude_op("op3", 3, "sk_p",
                                 {{"distance", 1.0}, {"extrudeMode", "ThroughAll"}, {"booleanMode", "Add"}},
                                 "body_op1"));
        Run r = run_plan(s, ops);
        check(stopped(r) == "completed", "ThroughAll Add: completed (stoppedReason=" + stopped(r) + ")");
        check_near(vol_of(s, "body_op1"), 4000.0, 1e-6, "ThroughAll Add: no overshoot material");
        check_near(zmax_of(s, "body_op1"), 10.0, 1e-6, "ThroughAll Add: ends exactly at the far face");
    }
}

// --- 5. Two-direction negative leg refuses. --------------------------------------
void test_two_direction_negative_leg_refuses() {
    Session s;
    json ops = json::array({sketch_op("op0", 0, "sk", "XY", rect("s", 0, 0, 10, 10)),
                            extrude_op("op1", 1, "sk",
                                       {{"distance", -10.0}, {"extrudeMode", "Blind"},
                                        {"booleanMode", "NewBody"}, {"twoDirections", true},
                                        {"extrudeMode2", "Blind"}, {"distance2", 5.0}})});
    Run r = run_plan(s, ops);
    check(stopped(r) == "opFailed", "two-direction negative leg: refused (stoppedReason=" + stopped(r) + ")");
    check(has_code(r, "EXTRUDE_TWO_DIRECTION_NEGATIVE_LEG"),
          "two-direction negative leg: diagnostic (got " + codes_joined(r) + ")");
}

// --- 6. Cut bisecting to one shared EDGE is a two-child split. --------------------
void test_edge_touching_split_publishes() {
    // Box [0,20]^3. Tool profile on XZ (u=y, v=z): a V with apex at (y=10, z=0) and
    // top corners at (0,20),(20,20); Symmetric 100 along X cuts a V-groove that
    // reaches the bottom face exactly, leaving two prisms sharing the bottom edge.
    Session s;
    json ops = json::array({sketch_op("op0", 0, "sk_a", "XY", rect("a", 0, 0, 20, 20)),
                            extrude_op("op1", 1, "sk_a",
                                       {{"distance", 20.0}, {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}),
                            sketch_op("op2", 2, "sk_v", "XZ", tri("v", -1, 20, 21, 20, 10, 0)),
                            extrude_op("op3", 3, "sk_v",
                                       {{"distance", 100.0}, {"extrudeMode", "Symmetric"}, {"booleanMode", "Cut"}},
                                       "body_op1")});
    Run r = run_plan(s, ops);
    check(stopped(r) == "completed", "edge-touching split: completed (stoppedReason=" + stopped(r) +
                                         ", codes " + codes_joined(r) + ")");
    check(events_of(r, "created") == 2 && events_of(r, "deleted") == 1,
          "edge-touching split: two created children + parent deleted");
    check(s.bodies_copy().contains("body_op3:0") && s.bodies_copy().contains("body_op3:1"),
          "edge-touching split: children body_op3:0 / :1 published");
}

}  // namespace

int main() {
    try {
        test_add_disjoint_refuses();
        test_empty_result_refuses();
        test_symmetric_and_two_direction_topology();
        test_through_all_policy();
        test_two_direction_negative_leg_refuses();
        test_edge_touching_split_publishes();
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: exception %s\n", e.what());
        ++g_failures;
    }
    if (g_failures == 0) std::fprintf(stderr, "test_extrude_boolean_modes: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
