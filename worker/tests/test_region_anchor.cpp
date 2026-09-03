// test_region_anchor.cpp — kernel-hardening WP-B: `regionAnchor` rebinds a profile
// whose stored `regionId` no longer names any cell (SCHEMA §7.3 "Region anchor").
//
// A V3 regionId hashes the cell's bounding fragments INCLUDING their intersection
// parameters, so moving a crossing re-mints every id in the sketch and a dependent
// extrude used to fail `regionId … matched no selectable region` on every such
// edit. These probes pin the four outcomes the resolution ladder must produce:
//   1. fresh id + anchor        → exact id wins, NO warning;
//   2. STALE id + interior anchor → the unique containing cell is bound, the step
//      is Ok and carries `REGION_REBOUND_BY_ANCHOR` with {from, to, anchor};
//   3. STALE id + anchor outside every cell → the unchanged refusal, nothing bound;
//   4. malformed `regionAnchor`  → a named OP_FAILED at the params boundary.
// Plus the two paths the anchor must NEVER reach (SCHEMA §7.3: it is a fallback
// for a STALE id and nothing else):
//   5. an EMPTY regionId keeps the V1 first-region fallback, anchor ignored;
//   6. an AMBIGUOUS legacy id still refuses — the anchor must not break a tie the
//      id itself could not.
// No framework: exit code == failure count.
#include <array>
#include <cmath>
#include <cstdio>
#include <optional>
#include <string>
#include <vector>

#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>

#include "loop/FaceBuilder.h"
#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include <TopoDS_Face.hxx>

#include "nlohmann/json.hpp"
#include "ops/OpCommon.h"
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
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.9f want %.9f tol %g)\n", msg.c_str(), got, want, tol);
        ++g_failures;
    }
}
constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr double kPi = 3.14159265358979323846;

// A 40×20 rectangle with a circle of `radius` centred ON its right edge at (40, 10).
// The circle crosses the edge, so the table holds three bounded cells: the rectangle
// minus the inner half-disc (largest), the inner half-disc, and the outer half-disc.
// Changing `radius` moves every crossing and therefore re-mints every V3 id.
json sketch_params(double radius) {
    json entities = json::array({
        json{{"id", "l1"}, {"type", "Line"}, {"p0", {0.0, 0.0}}, {"p1", {40.0, 0.0}}},
        json{{"id", "l2"}, {"type", "Line"}, {"p0", {40.0, 0.0}}, {"p1", {40.0, 20.0}}},
        json{{"id", "l3"}, {"type", "Line"}, {"p0", {40.0, 20.0}}, {"p1", {0.0, 20.0}}},
        json{{"id", "l4"}, {"type", "Line"}, {"p0", {0.0, 20.0}}, {"p1", {0.0, 0.0}}},
        json{{"id", "c1"}, {"type", "Circle"}, {"center", {40.0, 10.0}}, {"radius", radius}},
    });
    return json{{"sketchId", "sk"},
                {"plane", {{"kind", "XY"}}},
                {"entities", std::move(entities)},
                {"constraints", json::array()}};
}

// The V3 publication table verbatim — `makeRegionDetectionConfig` plus the
// physical-proximity refinement policy, exactly as `SolverLane::on_regions` and
// `ops::build_profile_face` configure it for `regionIdentityVersion: 3`.
loop::RegionTable v3_table(const json& params, sk::Sketch*& sketch_out,
                           onecad::wire::TranslateResult& keep_alive) {
    keep_alive = onecad::wire::translate(params);
    check(keep_alive.ok, "wire sketch translates: " + keep_alive.error);
    if (!keep_alive.ok) return {};
    const sk::SolveResult solve = keep_alive.sketch->solve();
    check(solve.success, "wire sketch solves: " + solve.errorMessage);
    if (!solve.success) return {};
    sketch_out = keep_alive.sketch.get();

    loop::LoopDetectorConfig config = loop::makeRegionDetectionConfig();
    config.curveRefinementPolicy = loop::CurveRefinementPolicy::V3PhysicalProximity;
    loop::LoopDetector detector;
    detector.setConfig(config);
    const loop::LoopDetectionResult detected = detector.detect(*keep_alive.sketch);
    const auto map_edge = [&](const sk::EntityID& internal) {
        const auto it = keep_alive.index.internal_edge_to_wire.find(internal);
        return it == keep_alive.index.internal_edge_to_wire.end() ? internal : it->second;
    };
    return loop::buildRegionTable(detected, map_edge, sk::constants::COINCIDENCE_TOLERANCE,
                                  loop::RegionIdentityVersion::V3);
}

// The V3 id of the largest-area cell, measured on the BUILT faces (the same
// FaceBuilder the op uses), so "largest" means the same thing here and there.
std::string largest_cell_id(double radius, double& area_out) {
    onecad::wire::TranslateResult translated;
    sk::Sketch* sketch = nullptr;
    const loop::RegionTable table = v3_table(sketch_params(radius), sketch, translated);
    check(table.success && !table.regions.empty(),
          "region table built for radius " + std::to_string(radius));
    if (!table.success || sketch == nullptr) return {};
    const loop::FaceBuilder builder;
    std::string best_id;
    double best_area = -1.0;
    for (const loop::RegionDefinition& region : table.regions) {
        loop::Face face;
        face.outerLoop = region.outerLoop;
        face.innerLoops = region.holes;
        const loop::FaceBuildResult built = builder.buildFace(face, *sketch);
        if (!built.success || built.face.IsNull()) continue;
        GProp_GProps props;
        BRepGProp::SurfaceProperties(built.face, props);
        if (props.Mass() > best_area) { best_area = props.Mass(); best_id = region.id; }
    }
    area_out = best_area;
    return best_id;
}

// Two overlapping circles publish three bounded cells whose LEGACY outer ids
// collide, which is the only way to reach the ambiguity branch (a V3 id is a
// content hash and cannot collide). Mirrors `test_region_table.cpp`.
json overlapping_circles() {
    return json{{"sketchId", "overlap"},
                {"plane", {{"kind", "XY"}}},
                {"entities", json::array({
                     json{{"id", "o1"}, {"type", "Circle"}, {"center", {-5, 0}}, {"radius", 10}},
                     json{{"id", "o2"}, {"type", "Circle"}, {"center", {5, 0}}, {"radius", 10}}})},
                {"constraints", json::array()}};
}

double face_area(const TopoDS_Face& face) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(face, props);
    return props.Mass();
}

struct Run {
    Envelope resp;
    std::vector<Envelope> events;
};

// Sketch (step 0) + Blind Extrude 10 mm NewBody of `region_id` (step 1), then
// AcceptPrepared. `anchor` is attached verbatim so a malformed value reaches the
// worker's params boundary unmodified.
Run extrude_plan_over(Session& s, json sketch, const std::string& region_id,
                      const std::optional<json>& anchor);

Run extrude_plan(Session& s, double radius, const std::string& region_id,
                 const std::optional<json>& anchor) {
    return extrude_plan_over(s, sketch_params(radius), region_id, anchor);
}

Run extrude_plan_over(Session& s, json sketch, const std::string& region_id,
                      const std::optional<json>& anchor) {
    s.open("doc", 0, 3, "determinism");
    json extrude_params = {{"sketchId", "sk"},
                           {"regionId", region_id},
                           {"regionIdentityVersion", 3},
                           {"distance", 10.0},
                           {"extrudeMode", "Blind"},
                           {"booleanMode", "NewBody"}};
    if (anchor) extrude_params["regionAnchor"] = *anchor;
    json ops = json::array({
        json{{"opType", "Sketch"}, {"opId", "op0"}, {"stepIndex", 0},
             {"params", std::move(sketch)}},
        json{{"opType", "Extrude"}, {"opId", "op1"}, {"stepIndex", 1},
             {"params", std::move(extrude_params)}},
    });
    Run run;
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [&run](Envelope& frame) { run.events.push_back(frame); }};
    json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty}, {"prefixHashes", json::array({"h0", "h1"})},
                 {"targetStep", 1}, {"ops", ops}};
    run.resp = onecad::session::handle_execute_plan(
        s, Envelope::request(1, "ExecutePlan", args), ctx);
    onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared",
                             json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    return run;
}

std::string stopped(const Run& r) { return r.resp.result.value("stoppedReason", std::string("?")); }

// Ok-step diagnostics ride the `planStep` EVENT (`perStepResults` carries them only
// for a FAILED step), so a successful warning is only visible here.
std::optional<json> step_diagnostic(const Run& r, std::uint64_t step, const std::string& code) {
    for (const Envelope& e : r.events) {
        if (!e.event_name || *e.event_name != "planStep") continue;
        if (!e.step_index || *e.step_index != step) continue;
        if (!e.result.contains("diagnostics")) continue;
        for (const json& d : e.result["diagnostics"]) {
            if (d.value("code", "") == code) return d;
        }
    }
    return std::nullopt;
}

bool any_diagnostic(const Run& r, const std::string& code) {
    for (const Envelope& e : r.events) {
        if (!e.result.contains("diagnostics")) continue;
        for (const json& d : e.result["diagnostics"]) {
            if (d.value("code", "") == code) return true;
        }
    }
    if (!r.resp.result.contains("perStepResults")) return false;
    for (const json& step : r.resp.result["perStepResults"]) {
        if (!step.contains("diagnostics")) continue;
        for (const json& d : step["diagnostics"]) {
            if (d.value("code", "") == code) return true;
        }
    }
    return false;
}

std::string step_message(const Run& r, std::uint64_t step) {
    if (!r.resp.result.contains("perStepResults")) return {};
    for (const json& s : r.resp.result["perStepResults"]) {
        if (s.value("stepIndex", std::uint64_t{0}) == step) return s.value("message", "");
    }
    return {};
}

double vol_of(Session& s, const std::string& bid) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get(bid);
    return rec ? onecad::session::shape_volume(rec->geom) : -1.0;
}

double prism_volume(double radius) { return (800.0 - kPi * radius * radius * 0.5) * 10.0; }

std::string g_id_r6;
std::string g_id_r7;

void measure_ids() {
    double area6 = 0.0;
    double area7 = 0.0;
    g_id_r6 = largest_cell_id(6.0, area6);
    g_id_r7 = largest_cell_id(7.0, area7);
    std::fprintf(stderr, "region_anchor: r=6 largest cell id=%s area=%.9f\n", g_id_r6.c_str(), area6);
    std::fprintf(stderr, "region_anchor: r=7 largest cell id=%s area=%.9f\n", g_id_r7.c_str(), area7);
    check(!g_id_r6.empty() && !g_id_r7.empty(), "both largest-cell ids resolved");
    check(g_id_r6 != g_id_r7, "a radius edit re-mints the V3 id (that is the premise)");
    check_near(area6, 800.0 - kPi * 18.0, 1e-6, "r=6 largest cell area 800 - 18π");
    check_near(area7, 800.0 - kPi * 24.5, 1e-6, "r=7 largest cell area 800 - 24.5π");
}

// 1. The exact id still matches: the anchor is never consulted and no warning fires.
void test_fresh_id_wins_over_anchor() {
    Session s;
    Run r = extrude_plan(s, 6.0, g_id_r6, json::array({10.0, 10.0}));
    check(stopped(r) == "completed", "fresh id: completed (stoppedReason=" + stopped(r) + ")");
    const double volume = vol_of(s, "body_op1");
    std::fprintf(stderr, "region_anchor: fresh id volume=%.9f\n", volume);
    check_near(volume, prism_volume(6.0), 1e-6, "fresh id: (800 - 18π)·10");
    check(!any_diagnostic(r, "REGION_REBOUND_BY_ANCHOR"),
          "fresh id: NO REGION_REBOUND_BY_ANCHOR warning");
}

// 2. The stored id was re-minted by the radius edit; the anchor still lands in one
//    cell, so the step binds it and says so.
void test_stale_id_rebinds_by_anchor() {
    Session s;
    Run r = extrude_plan(s, 7.0, g_id_r6, json::array({10.0, 10.0}));
    check(stopped(r) == "completed", "stale id + anchor: completed (stoppedReason=" +
                                         stopped(r) + ", message='" + step_message(r, 1) + "')");
    const double volume = vol_of(s, "body_op1");
    std::fprintf(stderr, "region_anchor: rebound volume=%.9f\n", volume);
    check_near(volume, prism_volume(7.0), 1e-6, "stale id + anchor: (800 - 24.5π)·10");
    const std::optional<json> diag = step_diagnostic(r, 1, "REGION_REBOUND_BY_ANCHOR");
    check(diag.has_value(), "stale id + anchor: planStep 1 carries REGION_REBOUND_BY_ANCHOR");
    if (!diag) return;
    std::fprintf(stderr, "region_anchor: diagnostic=%s\n", diag->dump().c_str());
    check(diag->value("severity", "") == "warning", "rebind diagnostic severity=warning");
    check(diag->value("stage", "") == "profile", "rebind diagnostic stage=profile");
    const json region = diag->value("evidence", json::object()).value("region", json::object());
    check(region.value("from", "") == g_id_r6, "rebind evidence.from == the stale id");
    check(region.value("to", "") == g_id_r7, "rebind evidence.to == the new largest cell id");
    check(region.value("anchor", json::array()) == json::array({10.0, 10.0}),
          "rebind evidence.anchor == the authored [u, v]");
}

// 3. An anchor outside every cell answers nothing, so the refusal stands unchanged.
void test_anchor_outside_every_cell_refuses() {
    Session s;
    Run r = extrude_plan(s, 7.0, g_id_r6, json::array({60.0, 60.0}));
    check(stopped(r) == "opFailed", "anchor outside: refused (stoppedReason=" + stopped(r) + ")");
    const std::string message = step_message(r, 1);
    std::fprintf(stderr, "region_anchor: outside-anchor refusal='%s'\n", message.c_str());
    check(message.find("matched no selectable region") != std::string::npos,
          "anchor outside: unchanged refusal reason");
    check(message.find("available: [") != std::string::npos,
          "anchor outside: refusal still names the available ids");
    check(!any_diagnostic(r, "REGION_REBOUND_BY_ANCHOR"),
          "anchor outside: NO rebind warning");
    check(s.bodies_copy().size() == 0, "anchor outside: nothing published");
}

// 4. A present-but-malformed anchor is a named boundary refusal, never a silent drop
//    that would surface as the stale-id failure and look like a different defect.
void test_malformed_anchor_refuses() {
    Session s;
    Run r = extrude_plan(s, 6.0, g_id_r6, json::array({10.0}));
    check(stopped(r) == "opFailed", "malformed anchor: refused (stoppedReason=" + stopped(r) + ")");
    const std::string message = step_message(r, 1);
    std::fprintf(stderr, "region_anchor: malformed-anchor refusal='%s'\n", message.c_str());
    check(message == "Extrude: regionAnchor must be [u, v]",
          "malformed anchor: named refusal message");
    check(s.bodies_copy().size() == 0, "malformed anchor: nothing published");
}

// 5. An EMPTY regionId is the V1 legacy first-region fallback. The anchor is scoped
//    to a STALE id, so it must not hijack that fallback even when it names a cell.
void test_empty_region_id_ignores_anchor() {
    const json sketch = sketch_params(6.0);
    std::string plain_error;
    const auto plain = onecad::ops::build_profile_face(sketch, "", std::nullopt, plain_error);
    check(plain.has_value(), "empty regionId: V1 fallback still builds: " + plain_error);
    if (!plain) return;

    // (44, 10) is inside the circle and OUTSIDE the rectangle, i.e. the small outer
    // half-disc cell — so if the anchor were consulted the area would collapse.
    std::vector<json> diagnostics;
    std::string anchored_error;
    const auto anchored = onecad::ops::build_profile_face(
        sketch, "", std::nullopt, anchored_error, &diagnostics,
        std::optional<std::array<double, 2>>{{44.0, 10.0}});
    check(anchored.has_value(), "empty regionId + anchor still builds: " + anchored_error);
    if (!anchored) return;
    const double plain_area = face_area(*plain);
    const double anchored_area = face_area(*anchored);
    std::fprintf(stderr, "region_anchor: V1 fallback area=%.9f with-anchor area=%.9f\n",
                 plain_area, anchored_area);
    check_near(anchored_area, plain_area, 1e-9,
               "empty regionId: the anchor changes nothing (V1 first-region fallback)");
    check(plain_area > 500.0,
          "empty regionId: the fallback cell is NOT the small cell the anchor names "
          "(so the check is not vacuous)");
    check(diagnostics.empty(), "empty regionId: no diagnostic emitted");
}

// 6. An AMBIGUOUS id is a refusal, and stays one. Resolving it by anchor would be a
//    guess dressed up as evidence — exactly the mis-bind the ladder forbids.
void test_ambiguous_id_never_reaches_anchor() {
    const json sketch = overlapping_circles();
    onecad::wire::TranslateResult translated;
    sk::Sketch* solved = nullptr;
    const loop::RegionTable table = v3_table(sketch, solved, translated);
    check(table.success && table.regions.size() == 3,
          "overlapping circles publish three bounded cells");
    if (!table.success) return;

    std::string ambiguous;
    for (const loop::RegionDefinition& a : table.regions) {
        int count = 0;
        for (const loop::RegionDefinition& b : table.regions) {
            if (a.legacyId == b.legacyId) ++count;
        }
        if (count > 1) { ambiguous = a.legacyId; break; }
    }
    check(!ambiguous.empty(), "overlap reproduces a legacy outer-id collision");
    if (ambiguous.empty()) return;

    // (0, 0) is strictly inside the lens cell, so an anchor-first implementation
    // WOULD find exactly one containing cell here. It must be refused anyway.
    std::vector<json> diagnostics;
    std::string error;
    const auto face = onecad::ops::build_profile_face(
        sketch, ambiguous, std::nullopt, error, &diagnostics,
        std::optional<std::array<double, 2>>{{0.0, 0.0}});
    std::fprintf(stderr, "region_anchor: ambiguous-id refusal='%s'\n", error.c_str());
    check(!face.has_value(), "ambiguous legacy id + anchor: still refused");
    check(error.find("ambiguous") != std::string::npos,
          "ambiguous legacy id + anchor: refusal still says ambiguous");
    for (const json& d : diagnostics) {
        check(d.value("code", "") != "REGION_REBOUND_BY_ANCHOR",
              "ambiguous legacy id + anchor: no rebind warning");
    }
}

}  // namespace

// H3 (adversarial review 2026-09-03): a profile that FAILS after a degenerate
// entity was dropped must still carry the SKETCH_ENTITY_DEGENERATE advisory —
// it is the reason the profile has no closed region — with the failure last.
void test_failed_profile_keeps_the_degenerate_advisory() {
    Session s;
    json entities = json::array({
        json{{"id", "l1"}, {"type", "Line"}, {"p0", {0.0, 0.0}}, {"p1", {40.0, 0.0}}},
        json{{"id", "l2"}, {"type", "Line"}, {"p0", {40.0, 0.0}}, {"p1", {40.0, 20.0}}},
        // The top edge collapsed to a point: zero-length TO THE GRAPH, dropped.
        json{{"id", "l3"}, {"type", "Line"}, {"p0", {40.0, 20.0}}, {"p1", {40.0, 20.0}}},
        json{{"id", "l4"}, {"type", "Line"}, {"p0", {0.0, 20.0}}, {"p1", {0.0, 0.0}}},
    });
    json sketch{{"sketchId", "sk"}, {"plane", {{"kind", "XY"}}},
                {"entities", std::move(entities)}, {"constraints", json::array()}};
    Run r = extrude_plan_over(s, sketch, "r_0000000000000000", std::nullopt);
    check(stopped(r) == "opFailed", "open profile: refused (stoppedReason=" + stopped(r) + ")");
    const std::string message = step_message(r, 1);
    std::fprintf(stderr, "region_anchor: failed-profile message='%s'\n", message.c_str());
    check(message.find("no closed region") != std::string::npos ||
              message.find("matched no selectable region") != std::string::npos,
          "…the step message is the failure text, not the advisory");
    check(any_diagnostic(r, "SKETCH_ENTITY_DEGENERATE"),
          "…and the SKETCH_ENTITY_DEGENERATE advisory survives the failure");
}

int main() {
    try {
        measure_ids();
        test_fresh_id_wins_over_anchor();
        test_stale_id_rebinds_by_anchor();
        test_anchor_outside_every_cell_refuses();
        test_malformed_anchor_refuses();
        test_empty_region_id_ignores_anchor();
        test_ambiguous_id_never_reaches_anchor();
    test_failed_profile_keeps_the_degenerate_advisory();
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: exception %s\n", e.what());
        ++g_failures;
    }
    if (g_failures == 0) std::fprintf(stderr, "test_region_anchor: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
