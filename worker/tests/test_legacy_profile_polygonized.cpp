// test_legacy_profile_polygonized.cpp — kernel audit 2026-11-11 finding 10.
// A LEGACY region (Extrude with no `regionIdentityVersion`) whose loop mixes lines
// and arcs is built by FaceBuilder's polygon fallback: each arc becomes chords.
// That is frozen legacy geometry and stays exactly as it was (a stadium of two
// lines and two R5 arcs extrudes to 28 planar faces, ~0.32 % under the analytic
// volume), but the builder's "polygon wire" warning was dropped by the caller, so
// the approximation was invisible. It now rides the step as the warning
// PROFILE_CURVES_POLYGONIZED naming the curved WIRE entity ids.
//
// Controls: a legacy rectangle (chords ARE its edges) and a legacy lone circle
// (built exactly) carry no such warning; the same stadium through a V3 regionId is
// exact (6 faces) and carries none. No framework.
#include <cmath>
#include <cstdio>
#include <set>
#include <string>
#include <vector>

#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

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
constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr double kPi = 3.14159265358979323846;

json line(const char* id, double x0, double y0, double x1, double y1) {
    return {{"id", id}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y1}}};
}
json stadium() {
    return json::array({line("l0", 0, -5, 20, -5),
                        {{"id", "a1"}, {"type", "Arc"}, {"center", {20.0, 0.0}}, {"radius", 5.0},
                         {"startAngle", -kPi / 2}, {"endAngle", kPi / 2}},
                        line("l1", 20, 5, 0, 5),
                        {{"id", "a2"}, {"type", "Arc"}, {"center", {0.0, 0.0}}, {"radius", 5.0},
                         {"startAngle", kPi / 2}, {"endAngle", 3 * kPi / 2}}});
}
json sketch_params(const json& ents) {
    return {{"sketchId", "s"}, {"plane", {{"kind", "XY"}}}, {"entities", ents},
            {"constraints", json::array()}};
}

std::string v3_region_id(const json& params) {
    onecad::wire::TranslateResult tr = onecad::wire::translate(params);
    if (!tr.ok || !tr.sketch->solve().success) return {};
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

struct Run {
    std::string stopped;
    json diagnostics = json::array();  // the extrude step's planStep diagnostics
    double volume = -1.0;
    int faces = -1;
};

Run run(const json& ents, bool v3) {
    Session s;
    s.open("doc", 0, 3, "determinism");
    const json sketch = sketch_params(ents);
    json extrude = {{"sketchId", "s"}, {"distance", 10.0}, {"extrudeMode", "Blind"},
                    {"booleanMode", "NewBody"}};
    if (v3) {
        extrude["regionId"] = v3_region_id(sketch);
        extrude["regionIdentityVersion"] = 3;
    }
    const json ops = json::array({json{{"opType", "Sketch"}, {"opId", "op0"}, {"stepIndex", 0},
                                       {"params", sketch}},
                                  json{{"opType", "Extrude"}, {"opId", "op1"}, {"stepIndex", 1},
                                       {"params", extrude}}});
    Run out;
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [&out](Envelope& ev) {
        if (ev.event_name == "planStep" && ev.result.value("stepIndex", -1) == 1 &&
            ev.result.contains("diagnostics")) {
            out.diagnostics = ev.result["diagnostics"];
        }
    }};
    const json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                       {"expectedBaseHash", kEmpty}, {"prefixHashes", json::array({"h0", "h1"})},
                       {"targetStep", 1}, {"ops", ops}};
    const Envelope r = onecad::session::handle_execute_plan(s, Envelope::request(1, "ExecutePlan", args), ctx);
    out.stopped = r.result.value("stoppedReason", "?");
    onecad::session::handle_accept_prepared(
        s, Envelope::request(1, "AcceptPrepared", json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    const onecad::session::BodyStore bodies = s.bodies_copy();
    if (const auto* rec = bodies.get("body_op1")) {
        out.volume = onecad::session::shape_volume(rec->geom);
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(rec->geom, TopAbs_FACE, faces);
        out.faces = faces.Extent();
    }
    return out;
}

const json* find_code(const json& diagnostics, const std::string& code) {
    for (const json& d : diagnostics) {
        if (d.value("code", "") == code) return &d;
    }
    return nullptr;
}

}  // namespace

int main() {
    try {
        const double exact = (200.0 + kPi * 25.0) * 10.0;
        {
            const Run r = run(stadium(), false);
            check(r.stopped == "completed", "legacy stadium: completed");
            // Frozen legacy geometry, unchanged: the chorded section, not the analytic one.
            check(r.faces == 28, "legacy stadium: 28 planar faces (got " + std::to_string(r.faces) + ")");
            check(std::abs(r.volume - 2776.457135) < 1e-5,
                  "legacy stadium: volume unchanged (got " + std::to_string(r.volume) + ")");
            const json* d = find_code(r.diagnostics, "PROFILE_CURVES_POLYGONIZED");
            check(d != nullptr, "legacy stadium: PROFILE_CURVES_POLYGONIZED warning on the step");
            if (d != nullptr) {
                check(d->value("severity", "") == "warning" && d->value("stage", "") == "profile",
                      "legacy stadium: severity warning, stage profile");
                std::set<std::string> ids;
                for (const json& id : (*d)["evidence"]["entityIds"]) ids.insert(id.get<std::string>());
                check(ids == std::set<std::string>{"a1", "a2"},
                      "legacy stadium: evidence names exactly the two arcs (wire ids)");
            }
        }
        {
            const Run r = run(json::array({line("e1", 0, 0, 10, 0), line("e2", 10, 0, 10, 10),
                                           line("e3", 10, 10, 0, 10), line("e4", 0, 10, 0, 0)}),
                              false);
            check(r.stopped == "completed" && std::abs(r.volume - 1000.0) < 1e-9,
                  "legacy rectangle: exact box");
            check(find_code(r.diagnostics, "PROFILE_CURVES_POLYGONIZED") == nullptr,
                  "legacy rectangle: no polygonized warning (its chords are its edges)");
        }
        {
            const Run r = run(json::array({{{"id", "c"}, {"type", "Circle"}, {"center", {0.0, 0.0}},
                                            {"radius", 5.0}}}),
                              false);
            check(r.stopped == "completed" && std::abs(r.volume - kPi * 250.0) < 1e-9,
                  "legacy circle: exact cylinder");
            check(find_code(r.diagnostics, "PROFILE_CURVES_POLYGONIZED") == nullptr,
                  "legacy circle: no polygonized warning");
        }
        {
            const Run r = run(stadium(), true);
            check(r.stopped == "completed" && r.faces == 6 && std::abs(r.volume - exact) < 1e-9 * exact,
                  "V3 stadium: exact, 6 faces");
            check(find_code(r.diagnostics, "PROFILE_CURVES_POLYGONIZED") == nullptr,
                  "V3 stadium: no polygonized warning");
        }
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: exception %s\n", e.what());
        ++g_failures;
    }
    if (g_failures == 0) std::fprintf(stderr, "test_legacy_profile_polygonized: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
