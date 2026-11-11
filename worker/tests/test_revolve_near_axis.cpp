// test_revolve_near_axis.cpp — kernel audit 2026-11-11 finding 1 / review SR-01.
// A Revolve profile whose edge runs parallel to the axis at ~Precision::Confusion()
// made BRepPrimAPI_MakeRevol index an empty parallel-lines extrema result: the
// worker died with SIGSEGV (gap ±1e-7 mm, 90° and 360°, legacy and V3 regions).
// Neighbouring gaps were inconsistent: 2e-7..1e-6 GEOMETRY_INVALID, 1e-5..5e-4 a
// published nanometre bore, −1e-6..−1e-3 a generic "Revolve operation failed".
//
// Contract pinned here (SCHEMA §7.3 Revolve):
//   * on the axis within Confusion → unchanged (exact-axis volume, no refusal);
//   * off-axis closest approach below the 1e-3 mm authoring resolution →
//     REVOLVE_PROFILE_NEAR_AXIS, never a crash, never a snap;
//   * crossing the axis by any amount beyond Confusion → REVOLVE_PROFILE_CROSSES_AXIS;
//   * closest approach ≥ resolution → the analytic annular volume;
//   * an ARC whose interior minimum (not an endpoint) sits in the band is refused;
//   * a refusal leaves the predecessor geometry intact.
//
// Frame: XZ sketch, toWorld(u, v) = (0, u, v); the axis line u = 0 is world Z.
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "session/ShapeMetrics.h"
#include "sketch/WireSketch.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::Session;
namespace loop = onecad::core::loop;
namespace sk = onecad::core::sketch;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_near(double got, double want, double rel, const std::string& msg) {
    if (!(std::abs(got - want) <= rel * std::abs(want))) {
        std::fprintf(stderr, "FAIL: %s (got %.12g want %.12g rel %g)\n", msg.c_str(), got, want, rel);
        ++g_failures;
    }
}
constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr double kPi = 3.14159265358979323846;

json axis_line() {
    return json{{"id", "axis"}, {"type", "Line"}, {"p0", {0.0, -5.0}}, {"p1", {0.0, 15.0}},
                {"construction", true}};
}
json rect_profile(double u0, double u1) {
    return json::array({
        json{{"id", "r1"}, {"type", "Line"}, {"p0", {u0, 0.0}}, {"p1", {u1, 0.0}}},
        json{{"id", "r2"}, {"type", "Line"}, {"p0", {u1, 0.0}}, {"p1", {u1, 10.0}}},
        json{{"id", "r3"}, {"type", "Line"}, {"p0", {u1, 10.0}}, {"p1", {u0, 10.0}}},
        json{{"id", "r4"}, {"type", "Line"}, {"p0", {u0, 10.0}}, {"p1", {u0, 0.0}}},
        axis_line()});
}
// Half disc of radius 5 centred (5 + g, 5), bulging TOWARD the axis: the arc's
// closest approach g is at its INTERIOR point (g, 5); both endpoints sit at 5 + g.
json half_disc_profile(double g) {
    return json::array({
        json{{"id", "a1"}, {"type", "Arc"}, {"center", {5.0 + g, 5.0}}, {"radius", 5.0},
             {"startAngle", kPi / 2.0}, {"endAngle", 3.0 * kPi / 2.0}},
        json{{"id", "l1"}, {"type", "Line"}, {"p0", {5.0 + g, 0.0}}, {"p1", {5.0 + g, 10.0}}},
        axis_line()});
}
json sketch_params(const json& ents) {
    return json{{"sketchId", "sk"}, {"plane", {{"kind", "XZ"}}}, {"entities", ents},
                {"constraints", json::array()}};
}

// The V3 id of the sketch's single bounded cell, configured exactly as
// `ops::build_profile_face` does for `regionIdentityVersion: 3`.
std::string v3_region_id(const json& params) {
    onecad::wire::TranslateResult tr = onecad::wire::translate(params);
    if (!tr.ok) return {};
    if (!tr.sketch->solve().success) return {};
    loop::LoopDetectorConfig config = loop::makeRegionDetectionConfig();
    config.curveRefinementPolicy = loop::CurveRefinementPolicy::V3PhysicalProximity;
    loop::LoopDetector detector;
    detector.setConfig(config);
    const loop::LoopDetectionResult detected = detector.detect(*tr.sketch);
    const auto map_edge = [&](const sk::EntityID& internal) {
        const auto it = tr.index.internal_edge_to_wire.find(internal);
        return it == tr.index.internal_edge_to_wire.end() ? internal : it->second;
    };
    const loop::RegionTable table = loop::buildRegionTable(
        detected, map_edge, sk::constants::COINCIDENCE_TOLERANCE, loop::RegionIdentityVersion::V3);
    return table.success && table.regions.size() == 1 ? table.regions[0].id : std::string{};
}

struct Opts {
    double angle = 360.0;
    bool v3 = false;
    bool with_box = false;          // a predecessor NewBody box (body_op1) before the revolve
    std::string mode = "NewBody";   // Cut targets the box
};

// [Sketch(box) → Extrude] → Sketch(profile + axis) → Revolve. Accepts the prepared prefix.
Envelope run(Session& s, const json& ents, const Opts& o) {
    s.open("doc", 0, 3, "determinism");
    json ops = json::array();
    int step = 0;
    if (o.with_box) {
        json box = json::array({
            json{{"id", "b1"}, {"type", "Line"}, {"p0", {-20.0, -20.0}}, {"p1", {20.0, -20.0}}},
            json{{"id", "b2"}, {"type", "Line"}, {"p0", {20.0, -20.0}}, {"p1", {20.0, 20.0}}},
            json{{"id", "b3"}, {"type", "Line"}, {"p0", {20.0, 20.0}}, {"p1", {-20.0, 20.0}}},
            json{{"id", "b4"}, {"type", "Line"}, {"p0", {-20.0, 20.0}}, {"p1", {-20.0, -20.0}}}});
        ops.push_back(json{{"opType", "Sketch"}, {"opId", "op0"}, {"stepIndex", step++},
                           {"params", {{"sketchId", "sk_box"}, {"plane", {{"kind", "XY"}}},
                                       {"entities", box}, {"constraints", json::array()}}}});
        ops.push_back(json{{"opType", "Extrude"}, {"opId", "op1"}, {"stepIndex", step++},
                           {"params", {{"sketchId", "sk_box"}, {"distance", 5.0},
                                       {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}});
    }
    const json sketch = sketch_params(ents);
    ops.push_back(json{{"opType", "Sketch"}, {"opId", "op" + std::to_string(step)},
                       {"stepIndex", step}, {"params", sketch}});
    ++step;
    json params = {{"sketchId", "sk"}, {"angleDeg", o.angle}, {"booleanMode", o.mode},
                   {"axis", {{"kind", "sketchLine"}, {"sketchId", "sk"}, {"lineId", "axis"}}}};
    if (o.mode != "NewBody") params["targetBodyId"] = "body_op1";
    if (o.v3) {
        params["regionId"] = v3_region_id(sketch);
        params["regionIdentityVersion"] = 3;
        check(!params["regionId"].get<std::string>().empty(), "V3 region id derived");
    }
    ops.push_back(json{{"opType", "Revolve"}, {"opId", "op" + std::to_string(step)},
                       {"stepIndex", step}, {"params", params}});
    json hashes = json::array();
    for (int i = 0; i <= step; ++i) hashes.push_back("h" + std::to_string(i));
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty}, {"prefixHashes", hashes},
                 {"targetStep", step}, {"ops", ops}};
    Envelope resp =
        onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    return resp;
}

std::string stopped(const Envelope& r) { return r.result.value("stoppedReason", std::string("?")); }
// Every diagnostic code of the plan's steps, joined (for messages and lookups).
std::string codes(const Envelope& r) {
    std::string out;
    if (!r.result.contains("perStepResults")) return out;
    for (const json& step : r.result["perStepResults"]) {
        if (!step.contains("diagnostics")) continue;
        for (const json& d : step["diagnostics"]) out += d.value("code", "") + " ";
    }
    return out;
}
bool has_code(const Envelope& r, const std::string& code) {
    return codes(r).find(code + " ") != std::string::npos;
}
double vol_of(Session& s, const std::string& bid) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get(bid);
    return rec ? onecad::session::shape_volume(rec->geom) : -1.0;
}
std::string tag(const char* what, double gap, const Opts& o) {
    char buf[160];
    std::snprintf(buf, sizeof buf, "%s gap=%g angle=%g %s", what, gap, o.angle, o.v3 ? "V3" : "legacy");
    return buf;
}

void expect_refused(double gap, const Opts& o, const std::string& code) {
    Session s;
    const Envelope r = run(s, rect_profile(gap, 10.0), o);
    check(stopped(r) == "opFailed", tag("refused", gap, o) + " (stoppedReason=" + stopped(r) + ")");
    check(has_code(r, code), tag("named", gap, o) + ": want " + code + " got " + codes(r));
    check(s.bodies_copy().size() == 0, tag("nothing published", gap, o));
}

void expect_revolved(double gap, const Opts& o, double want_volume, double rel) {
    Session s;
    const Envelope r = run(s, rect_profile(gap, 10.0), o);
    check(stopped(r) == "completed", tag("completed", gap, o) + " (stoppedReason=" + stopped(r) +
                                         " codes=" + codes(r) + ")");
    check_near(vol_of(s, "body_op1"), want_volume, rel, tag("volume", gap, o));
}

void test_gap_sweep(const Opts& o) {
    const double sector = std::abs(o.angle) / 360.0;
    const double exact_axis = kPi * 100.0 * 10.0 * sector;
    // Exactly on the axis: the analytic cylinder sector (unchanged behaviour).
    expect_revolved(0.0, o, exact_axis, 1e-12);
    // Within Confusion of the axis: OCCT treats the edge as on-axis (unchanged).
    for (const double g : {1e-12, -1e-12, 1e-9, -1e-9, 1e-8, -1e-8}) {
        expect_revolved(g, o, exact_axis, 1e-8);
    }
    // Off-axis but below the authoring resolution: named refusal on both sides.
    for (const double g : {1e-7, 1e-6, 1e-5, 1e-4, 5e-4}) {
        expect_refused(g, o, "REVOLVE_PROFILE_NEAR_AXIS");
        expect_refused(-g, o, "REVOLVE_PROFILE_CROSSES_AXIS");
    }
    expect_refused(-1e-3, o, "REVOLVE_PROFILE_CROSSES_AXIS");
    expect_refused(-1e-2, o, "REVOLVE_PROFILE_CROSSES_AXIS");
    // At and beyond the resolution: the analytic annular sector π(r₂² − r₁²)·h·θ.
    for (const double g : {1e-3, 1e-2}) {
        expect_revolved(g, o, kPi * (100.0 - g * g) * 10.0 * sector, 1e-12);
    }
}

// The arc's minimum is INTERIOR; its endpoints are 5 mm away.
void test_arc_interior_minimum(bool v3) {
    Opts o;
    o.v3 = v3;
    {
        Session s;
        const Envelope r = run(s, half_disc_profile(5e-4), o);
        check(stopped(r) == "opFailed" && has_code(r, "REVOLVE_PROFILE_NEAR_AXIS"),
              std::string("arc interior minimum 5e-4 refused by name (") + (v3 ? "V3" : "legacy") +
                  ", codes=" + codes(r) + ")");
        check(s.bodies_copy().size() == 0, "arc interior minimum: nothing published");
    }
    if (!v3) return;  // the legacy path polygonizes arcs (audit finding 10); volume only on V3
    Session s;
    const double g = 0.01;
    const Envelope r = run(s, half_disc_profile(g), o);
    check(stopped(r) == "completed", "arc at 0.01 mm revolves (codes=" + codes(r) + ")");
    // Pappus: A = πR²/2, centroid 4R/(3π) from the flat side toward the axis.
    const double area = kPi * 25.0 / 2.0;
    const double rc = (5.0 + g) - 4.0 * 5.0 / (3.0 * kPi);
    check_near(vol_of(s, "body_op1"), area * 2.0 * kPi * rc, 1e-9, "arc at 0.01 mm: Pappus volume");
}

// A refused near-axis revolve leaves the predecessor body untouched, NewBody and Cut.
void test_predecessor_intact() {
    const double box_volume = 40.0 * 40.0 * 5.0;
    for (const char* mode : {"NewBody", "Cut"}) {
        Opts o;
        o.with_box = true;
        o.mode = mode;
        Session s;
        const Envelope r = run(s, rect_profile(1e-7, 10.0), o);
        check(stopped(r) == "opFailed" && has_code(r, "REVOLVE_PROFILE_NEAR_AXIS"),
              std::string("predecessor case refused by name: ") + mode);
        check(r.result.value("lastValidStep", -1) == 2,
              std::string("predecessor prefix kept: ") + mode);
        check(s.bodies_copy().size() == 1, std::string("only the box is published: ") + mode);
        check_near(vol_of(s, "body_op1"), box_volume, 1e-12, std::string("box volume intact: ") + mode);
    }
}

}  // namespace

int main() {
    try {
        for (const double angle : {360.0, 90.0, -90.0}) {
            for (const bool v3 : {false, true}) {
                Opts o;
                o.angle = angle;
                o.v3 = v3;
                test_gap_sweep(o);
            }
        }
        test_arc_interior_minimum(false);
        test_arc_interior_minimum(true);
        test_predecessor_intact();
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: exception %s\n", e.what());
        ++g_failures;
    }
    if (g_failures == 0) std::fprintf(stderr, "test_revolve_near_axis: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
