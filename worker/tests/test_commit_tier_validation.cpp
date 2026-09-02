// test_commit_tier_validation.cpp — kernel-hardening WP-E red-first probes.
//   1. A revolve profile that CROSSES its axis is refused by name
//      (REVOLVE_PROFILE_CROSSES_AXIS) before any kernel work — the swept solid
//      passes BRepCheck face by face and has positive volume, so nothing downstream
//      would catch the figure-8.
//   2. A profile TOUCHING the axis (an edge on it) is legal: the sweep is a cylinder
//      with degenerate pole edges, volume π·r²·h, published at commit.
//   3. A profile strictly on one side revolves to the analytic washer volume.
// No framework: exit code == failure count.
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
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.6f want %.6f tol %.6f)\n", msg.c_str(), got, want, tol);
        ++g_failures;
    }
}
constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr double kPi = 3.14159265358979323846;

json rect(const std::string& p, double x0, double y0, double x1, double y1) {
    return json::array({{{"id", p + "1"}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y0}}},
                        {{"id", p + "2"}, {"type", "Line"}, {"p0", {x1, y0}}, {"p1", {x1, y1}}},
                        {{"id", p + "3"}, {"type", "Line"}, {"p0", {x1, y1}}, {"p1", {x0, y1}}},
                        {{"id", p + "4"}, {"type", "Line"}, {"p0", {x0, y1}}, {"p1", {x0, y0}}}});
}

struct Run {
    Envelope resp;
};

// Sketch on XZ (u = y, v = z): a rectangle u∈[u0,u1] × v∈[0,10] plus a construction
// axis line at u = 0; Revolve 360° NewBody about that line.
Run revolve_plan(Session& s, double u0, double u1) {
    s.open("doc", 0, 3, "determinism");
    json ents = rect("r", u0, 0, u1, 10);
    ents.push_back(json{{"id", "axis"}, {"type", "Line"}, {"p0", {0.0, -5.0}}, {"p1", {0.0, 15.0}},
                        {"construction", true}});
    json ops = json::array({
        json{{"opType", "Sketch"}, {"opId", "op0"}, {"stepIndex", 0},
             {"params", {{"sketchId", "sk"}, {"plane", {{"kind", "XZ"}}}, {"entities", ents},
                         {"constraints", json::array()}}}},
        json{{"opType", "Revolve"}, {"opId", "op1"}, {"stepIndex", 1},
             {"params", {{"sketchId", "sk"}, {"angleDeg", 360.0}, {"booleanMode", "NewBody"},
                         {"axis", {{"kind", "sketchLine"}, {"sketchId", "sk"}, {"lineId", "axis"}}}}}},
    });
    Run run;
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty}, {"prefixHashes", json::array({"h0", "h1"})},
                 {"targetStep", 1}, {"ops", ops}};
    run.resp = onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    return run;
}

// Sketch on XZ: a full circle centred (cu, cv) of radius r plus the construction
// axis at u = 0; Revolve 360° NewBody. A circle has ONE vertex (its seam), so this
// is the profile the vertex-only classifier could not see.
Run revolve_circle_plan(Session& s, double cu, double cv, double r) {
    s.open("doc", 0, 3, "determinism");
    json ents = json::array({json{{"id", "c"}, {"type", "Circle"}, {"center", {cu, cv}}, {"radius", r}}});
    ents.push_back(json{{"id", "axis"}, {"type", "Line"}, {"p0", {0.0, -5.0}}, {"p1", {0.0, 25.0}},
                        {"construction", true}});
    json ops = json::array({
        json{{"opType", "Sketch"}, {"opId", "op0"}, {"stepIndex", 0},
             {"params", {{"sketchId", "sk"}, {"plane", {{"kind", "XZ"}}}, {"entities", ents},
                         {"constraints", json::array()}}}},
        json{{"opType", "Revolve"}, {"opId", "op1"}, {"stepIndex", 1},
             {"params", {{"sketchId", "sk"}, {"angleDeg", 360.0}, {"booleanMode", "NewBody"},
                         {"axis", {{"kind", "sketchLine"}, {"sketchId", "sk"}, {"lineId", "axis"}}}}}},
    });
    Run run;
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty}, {"prefixHashes", json::array({"h0", "h1"})},
                 {"targetStep", 1}, {"ops", ops}};
    run.resp = onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    return run;
}

std::string stopped(const Run& r) { return r.resp.result.value("stoppedReason", std::string("?")); }
bool has_code(const Run& r, const std::string& code) {
    if (!r.resp.result.contains("perStepResults")) return false;
    for (const json& step : r.resp.result["perStepResults"]) {
        if (!step.contains("diagnostics")) continue;
        for (const json& d : step["diagnostics"]) if (d.value("code", "") == code) return true;
    }
    return false;
}
double vol_of(Session& s, const std::string& bid) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get(bid);
    return rec ? onecad::session::shape_volume(rec->geom) : -1.0;
}

void test_profile_crossing_axis_refuses() {
    Session s;
    Run r = revolve_plan(s, -3.0, 7.0);
    check(stopped(r) == "opFailed", "crossing profile: refused (stoppedReason=" + stopped(r) + ")");
    check(has_code(r, "REVOLVE_PROFILE_CROSSES_AXIS"),
          "crossing profile: diagnostic REVOLVE_PROFILE_CROSSES_AXIS");
    check(s.bodies_copy().size() == 0, "crossing profile: nothing published");
}

void test_profile_touching_axis_revolves() {
    Session s;
    Run r = revolve_plan(s, 0.0, 10.0);
    check(stopped(r) == "completed", "touching profile: completed (stoppedReason=" + stopped(r) + ")");
    check_near(vol_of(s, "body_op1"), kPi * 100.0 * 10.0, 1e-6, "touching profile: cylinder volume π·r²·h");
}

void test_profile_one_side_revolves() {
    Session s;
    Run r = revolve_plan(s, 2.0, 7.0);
    check(stopped(r) == "completed", "one-side profile: completed (stoppedReason=" + stopped(r) + ")");
    check_near(vol_of(s, "body_op1"), kPi * (49.0 - 4.0) * 10.0, 1e-6, "one-side profile: washer volume");
}

void test_circle_profile_crossing_axis_refuses() {
    Session s;
    Run r = revolve_circle_plan(s, 2.0, 10.0, 5.0);  // u ∈ [-3, 7]
    check(stopped(r) == "opFailed", "crossing circle: refused (stoppedReason=" + stopped(r) + ")");
    check(has_code(r, "REVOLVE_PROFILE_CROSSES_AXIS"),
          "crossing circle: diagnostic REVOLVE_PROFILE_CROSSES_AXIS (not a generic build failure)");
    check(s.bodies_copy().size() == 0, "crossing circle: nothing published");
}

void test_circle_profile_one_side_revolves() {
    Session s;
    Run r = revolve_circle_plan(s, 6.0, 10.0, 5.0);  // u ∈ [1, 11]
    check(stopped(r) == "completed", "one-side circle: completed (stoppedReason=" + stopped(r) + ")");
    // Torus: V = 2·π²·R·r² with R = 6, r = 5.
    check_near(vol_of(s, "body_op1"), 2.0 * kPi * kPi * 6.0 * 25.0, 1e-6, "one-side circle: torus volume");
}

}  // namespace

int main() {
    try {
        test_profile_crossing_axis_refuses();
        test_profile_touching_axis_revolves();
        test_profile_one_side_revolves();
    test_circle_profile_crossing_axis_refuses();
    test_circle_profile_one_side_revolves();
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: exception %s\n", e.what());
        ++g_failures;
    }
    if (g_failures == 0) std::fprintf(stderr, "test_commit_tier_validation: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
