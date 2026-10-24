#include <algorithm>
#include <cmath>
#include <cstdio>
#include <limits>

#include <TopExp.hxx>
#include <NCollection_IndexedMap.hxx>
#include <TopTools_ShapeMapHasher.hxx>

#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "ops/LoftOp.h"
#include "protocol/Envelope.h"
#include "session/HistoryHash.h"
#include "session/PlanExecutor.h"
#include "session/PreviewOp.h"
#include "session/ShapeMetrics.h"
#include "sketch/WireSketch.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace session = onecad::session;
using onecad::protocol::Envelope;

namespace {
int failures = 0;
void check(bool condition, const char* message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        ++failures;
    }
}

json rectangle(double size, bool reverse = false) {
    json entities = json::array();
    const std::array<std::array<double, 2>, 4> points = {{{0, 0}, {size, 0}, {size, size}, {0, size}}};
    for (int index = 0; index < 4; ++index) {
        auto start = points[index];
        auto end = points[(index + 1) % 4];
        if (reverse) std::swap(start, end);
        entities.push_back({{"id", "e" + std::to_string(index)}, {"type", "Line"}, {"p0", start}, {"p1", end}});
    }
    if (reverse) std::reverse(entities.begin(), entities.end());
    return entities;
}

json sketch(double z, double size = 10.0, bool reverse = false) {
    return {{"plane", {{"kind", "custom"}, {"origin", {0, 0, z}}, {"xAxis", {1, 0, 0}},
                       {"yAxis", {0, 1, 0}}, {"normal", {0, 0, 1}}}},
            {"entities", rectangle(size, reverse)}, {"constraints", json::array()}};
}

json region_ref(const std::string& id, const json& sketch_params, int version = 2) {
    auto translated = onecad::wire::translate(sketch_params);
    check(translated.ok, "fixture sketch translates");
    translated.sketch->solve();
    onecad::core::loop::LoopDetector detector;
    auto config = onecad::core::loop::makeRegionDetectionConfig();
    config.exactAnalyticFragments = true;
    if (version == 3) config.curveRefinementPolicy = onecad::core::loop::CurveRefinementPolicy::V3PhysicalProximity;
    detector.setConfig(config);
    const auto detected = detector.detect(*translated.sketch);
    const auto table = onecad::core::loop::buildRegionTable(detected, [&](const auto& edge) {
        return translated.index.internal_edge_to_wire.at(edge);
    }, onecad::core::sketch::constants::COINCIDENCE_TOLERANCE,
       version == 3 ? onecad::core::loop::RegionIdentityVersion::V3 : onecad::core::loop::RegionIdentityVersion::V2);
    check(table.success && !table.regions.empty(), "fixture has region");
    return {{"sketchId", id}, {"regionId", table.regions.front().id}, {"regionIdentityVersion", version}};
}

struct Fixture {
    session::BodyStore bodies;
    onecad::elementmap::ElementMapPartition partition;
    std::vector<std::pair<std::string, json>> sketches;
    std::string last;
    onecad::CancelToken cancel;
    json profiles = json::array();

    void add(const std::string& id, json params) {
        profiles.push_back(region_ref(id, params));
        sketches.emplace_back(id, std::move(params));
    }
    json op(bool ruled = false) const {
        return {{"opType", "Loft"}, {"opId", "loft"}, {"params", {
            {"profiles", profiles}, {"isSolid", true}, {"isRuled", ruled}, {"booleanMode", "NewBody"}}}};
    }
    ops::OpOutcome run(const json& operation) {
        ops::OpContext context{bodies, &sketches, partition, &last, false, json::object(), &cancel};
        return ops::execute_loft(context, operation, "loft");
    }
};

void check_shape(const Fixture& fixture, const ops::OpOutcome& out, double volume) {
    check(out.status == ops::OpOutcome::Status::Ok, out.error_message.c_str());
    const auto* body = fixture.bodies.get("body_loft");
    check(body != nullptr, "standard body id created");
    if (!body) return;
    check(std::abs(session::shape_volume(body->geom) - volume) < volume * 1.0e-7, "analytic loft volume");
    check(out.body_events.size() == 1 && out.body_events[0].kind == "created", "created event");
    check(out.topology_history.size() == 1, "one created body history");
    if (out.topology_history.empty()) return;
    const auto& history = out.topology_history[0];
    check(history.coverage == session::HistoryCoverage::Proven, "birth history proven");
    check(history.survivors.empty() && history.successors.empty(), "no fabricated source correspondence");
    std::size_t count = 0;
    for (auto kind : {TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX}) {
        NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> shapes;
        TopExp::MapShapes(body->geom, kind, shapes);
        count += shapes.Extent();
    }
    check(history.births.size() == count, "every referenceable result shape certified birth");
}

void successful_sections() {
    for (bool ruled : {false, true}) {
        Fixture taper;
        taper.add("a", sketch(0, 10));
        taper.add("b", sketch(20, 5, true));
        check_shape(taper, taper.run(taper.op(ruled)), 20.0 * (100 + 25 + 50) / 3.0);
        Fixture three;
        three.add("a", sketch(0));
        three.add("b", sketch(10, 10, true));
        three.add("c", sketch(20));
        check_shape(three, three.run(three.op(ruled)), 2000);
        Fixture reversed;
        reversed.add("a", sketch(20));
        reversed.add("b", sketch(0, 10, true));
        check_shape(reversed, reversed.run(reversed.op(ruled)), 2000);
    }
}

void circular_sections() {
    for (bool ruled : {false, true}) {
        Fixture fixture;
        for (int index = 0; index < 3; ++index) {
            auto params = sketch(index * 10);
            params["entities"] = json::array({{{"id", "circle"}, {"type", "Circle"},
                                               {"center", {0, 0}}, {"radius", 5}}});
            fixture.add(std::to_string(index), params);
        }
        check_shape(fixture, fixture.run(fixture.op(ruled)), M_PI * 25 * 20);
    }
}

void refused(Fixture& fixture, const json& operation, const char* text) {
    const auto out = fixture.run(operation);
    check(out.status != ops::OpOutcome::Status::Ok, "invalid section refused");
    check(out.error_message.find(text) != std::string::npos, out.error_message.c_str());
    check(fixture.bodies.all().empty(), "refused candidate publishes no body");
    check(out.body_events.empty() && out.topology_history.empty(), "refused candidate has no births/events");
}

void malformed_parameters() {
    Fixture fixture;
    fixture.add("a", sketch(0));
    fixture.add("b", sketch(10));
    for (const std::string mode : {"Add", "Cut", "Intersect", "surprise"}) {
        auto op = fixture.op(); op["params"]["booleanMode"] = mode;
        refused(fixture, op, "only NewBody");
    }
    for (const std::string field : {"isSolid", "isRuled"}) {
        auto op = fixture.op(); op["params"][field] = "true";
        refused(fixture, op, "boolean");
    }
    auto op = fixture.op(); op["params"]["isSolid"] = false;
    refused(fixture, op, "only solid");
    for (const std::string field : {"sections", "guideRails", "guides", "rails", "spine", "startCondition",
                                    "endCondition", "continuity", "points"}) {
        op = fixture.op(); op["params"][field] = json::array();
        refused(fixture, op, ("unsupported " + field).c_str());
    }
    op = fixture.op(); op["params"]["profiles"][1]["regionId"] = "";
    refused(fixture, op, "requires sketchId and regionId");
    op = fixture.op(); op["params"]["profiles"][1] = fixture.profiles[0];
    refused(fixture, op, "duplicate");
    op = fixture.op(); op["params"]["profiles"][1]["regionIdentityVersion"] = 9;
    refused(fixture, op, "regionIdentityVersion");
    op = fixture.op(); op["params"]["profiles"][1]["regionAnchor"] = {0, "bad"};
    refused(fixture, op, "regionAnchor");
    op = fixture.op(); op["params"]["profiles"][1]["regionId"] = "missing";
    refused(fixture, op, "region");
    op = fixture.op(); op["params"]["profiles"] = json::array({fixture.profiles[0]});
    refused(fixture, op, "2..32");
    op = fixture.op(); op["params"]["profiles"] = json::array();
    for (int i = 0; i < 33; ++i) op["params"]["profiles"].push_back(fixture.profiles[0]);
    refused(fixture, op, "2..32");
}

void refused_geometry() {
    Fixture coincident;
    coincident.add("a", sketch(0)); coincident.add("b", sketch(0, 5));
    refused(coincident, coincident.op(), "coincident");
    Fixture unordered;
    unordered.add("a", sketch(0)); unordered.add("b", sketch(20)); unordered.add("c", sketch(10));
    refused(unordered, unordered.op(), "strictly monotone");
    Fixture fixture;
    fixture.add("a", sketch(0)); fixture.add("b", sketch(10));
    fixture.sketches[1].second["plane"] = {{"kind", "XZ"}};
    refused(fixture, fixture.op(), "parallel");
    fixture.sketches[1].second = sketch(10);
    fixture.sketches[1].second["entities"][0]["p0"][0] = std::numeric_limits<double>::infinity();
    refused(fixture, fixture.op(), "nonfinite");
    fixture.sketches[1].second = sketch(10);
    fixture.sketches[1].second["entities"].erase(3);
    refused(fixture, fixture.op(), "closed region");
    fixture.sketches[1].second["entities"] = json::array({{{"id", "point"}, {"type", "Point"}, {"at", {0, 0}}}});
    refused(fixture, fixture.op(), "closed region");
    fixture.sketches[1].second = sketch(10, 0);
    refused(fixture, fixture.op(), "region");
}

void holes_refused() {
    Fixture fixture;
    fixture.add("a", sketch(0));
    auto hole = sketch(10);
    hole["entities"].push_back({{"id", "hole"}, {"type", "Circle"}, {"center", {5, 5}}, {"radius", 2}});
    fixture.add("b", hole);
    refused(fixture, fixture.op(), "holes");
}

void changing_edges() {
    for (bool ruled : {false, true}) {
        for (bool circle : {false, true}) {
            Fixture fixture;
            fixture.add("a", sketch(0));
            auto params = sketch(10);
            params["entities"] = circle
                ? json::array({{{"id", "circle"}, {"type", "Circle"}, {"center", {5, 5}}, {"radius", 5}}})
                : json::array({{{"id", "t1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {10, 0}}},
                               {{"id", "t2"}, {"type", "Line"}, {"p0", {10, 0}}, {"p1", {5, 10}}},
                               {{"id", "t3"}, {"type", "Line"}, {"p0", {5, 10}}, {"p1", {0, 0}}}});
            fixture.add("b", params);
            const auto out = fixture.run(fixture.op(ruled));
            check(out.status == ops::OpOutcome::Status::Ok, out.error_message.c_str());
            const auto* body = fixture.bodies.get("body_loft");
            check(body != nullptr, "changing edge counts produce qualified solid");
            if (body) check(session::shape_volume(body->geom) > 0, "changing edges positive solid volume");
        }
    }
}

void profile_identity() {
    Fixture exact;
    exact.add("a", sketch(0)); exact.add("b", sketch(10));
    for (std::size_t index = 0; index < exact.sketches.size(); ++index) {
        exact.profiles[index] = region_ref(exact.sketches[index].first, exact.sketches[index].second, 3);
    }
    check_shape(exact, exact.run(exact.op()), 1000);
    Fixture rebound;
    rebound.add("a", sketch(0)); rebound.add("b", sketch(10));
    auto operation = rebound.op();
    operation["params"]["profiles"][1]["regionId"] = "stale";
    operation["params"]["profiles"][1]["regionAnchor"] = {5, 5};
    const auto outcome = rebound.run(operation);
    check_shape(rebound, outcome, 1000);
    check(std::any_of(outcome.diagnostics.begin(), outcome.diagnostics.end(), [](const auto& diagnostic) {
        return diagnostic.value("code", std::string{}) == "REGION_REBOUND_BY_ANCHOR";
    }), "anchor rebind publishes shared warning");
}

json plan_args(const Fixture& fixture) {
    json operations = json::array();
    int index = 0;
    for (const auto& [id, params] : fixture.sketches) {
        auto sketch_params = params; sketch_params["sketchId"] = id;
        operations.push_back({{"opType", "Sketch"}, {"opId", "sketch" + std::to_string(index)},
                              {"stepIndex", index++}, {"params", sketch_params}});
    }
    auto op = fixture.op(); op["stepIndex"] = index;
    operations.push_back(op);
    return {{"jobId", 1}, {"documentRevision", 1}, {"workerEpoch", 3},
            {"expectedBaseHash", session::kEmptyPrefixHash}, {"prefixHashes", {"a", "b", "c"}},
            {"targetStep", index}, {"ops", operations}};
}

void transaction_fences() {
    Fixture fixture;
    fixture.add("a", sketch(0)); fixture.add("b", sketch(10));
    session::Session state;
    state.open("loft", 0, 3, "determinism");
    onecad::CancelToken cancel;
    onecad::protocol::HandlerContext context{cancel, [](int) {}, [](Envelope&) {}, {}};
    const auto prepared = session::handle_execute_plan(state,
        Envelope::request(1, "ExecutePlan", plan_args(fixture)), context);
    check(prepared.ok.value_or(false) && prepared.result["lastValidStep"] == 2, "Loft plan prepared");
    check(state.bodies_copy().all().empty(), "prepared Loft invisible at head");
    auto response = session::handle_accept_prepared(state, Envelope::request(2, "AcceptPrepared",
        {{"jobId", 1}, {"documentRevision", 1}, {"workerEpoch", 4}}));
    check(response.error.has_value(), "stale epoch cannot accept Loft");
    response = session::handle_accept_prepared(state, Envelope::request(3, "AcceptPrepared",
        {{"jobId", 1}, {"documentRevision", 1}, {"workerEpoch", 3}}));
    check(response.ok.value_or(false), "valid Loft accepted");
    check(state.bodies_copy().contains("body_loft"), "accepted Loft at head");
    auto stale = plan_args(fixture); stale["jobId"] = 2; stale["expectedBaseHash"] = "obsolete";
    response = session::handle_execute_plan(state, Envelope::request(4, "ExecutePlan", stale), context);
    check(response.error.has_value() && response.error->code == "PROTOCOL_ERROR", "stale base hash fenced");
    check(state.bodies_copy().all().size() == 1, "stale plan preserves head");
}

void cancellation() {
    Fixture fixture;
    fixture.add("a", sketch(0)); fixture.add("b", sketch(10));
    fixture.cancel.cancel();
    auto out = fixture.run(fixture.op());
    check(out.status == ops::OpOutcome::Status::Cancelled, "cancelled Loft reports cancelled");
    check(fixture.bodies.all().empty(), "cancelled Loft never publishes");
}

void previews_preserve_head() {
    Fixture fixture;
    fixture.add("a", sketch(0)); fixture.add("b", sketch(10));
    session::Session state;
    state.open("loft-preview", 0, 3, "determinism");
    for (const auto& [id, params] : fixture.sketches) state.sketches().upsert(id, params);
    for (int attempt = 0; attempt < 2; ++attempt) {
        const auto response = session::handle_preview_op(state,
            Envelope::request(1, "PreviewOp", {{"op", fixture.op()}, {"expectedSnapshotId", std::uint64_t{0}}}));
        if (response.error) std::fprintf(stderr, "preview error: %s: %s\n", response.error->code.c_str(), response.error->message.c_str());
        check(response.ok.value_or(false), "Loft preview seeds both profile sketches");
        if (response.ok.value_or(false)) {
            check(response.result["changedBodies"] == json::array({"body_loft"}), "Loft preview body");
            check(response.result["meshes"].size() == 1, "Loft preview checked mesh");
        }
        check(state.current_snapshot_id() == 0 && state.bodies_copy().all().empty(), "preview leaves head untouched");
    }
    auto invalid = fixture.op(); invalid["params"]["profiles"][1] = fixture.profiles[0];
    auto response = session::handle_preview_op(state, Envelope::request(2, "PreviewOp", {{"op", invalid}}));
    check(response.error.has_value() && response.error->code == "OP_FAILED", "invalid Loft preview refused");
    invalid = fixture.op(); invalid["params"]["profiles"][1]["sketchId"] = "missing";
    response = session::handle_preview_op(state, Envelope::request(3, "PreviewOp", {{"op", invalid}}));
    check(response.error.has_value() && response.error->code == "REF_UNRESOLVED", "missing preview sketch refuses");
    response = session::handle_preview_op(state,
        Envelope::request(4, "PreviewOp", {{"op", fixture.op()}, {"expectedSnapshotId", std::uint64_t{1}}}));
    check(response.error.has_value() && response.error->code == "STALE_PREVIEW", "stale Loft preview fenced");
    onecad::CancelToken cancelled; cancelled.cancel();
    response = session::handle_preview_op(state, Envelope::request(5, "PreviewOp", {{"op", fixture.op()}}), cancelled);
    check(response.error.has_value() && response.error->code == "CANCELLED", "cancelled Loft preview fenced");
    check(state.current_snapshot_id() == 0 && state.bodies_copy().all().empty(), "all refused previews preserve head");
}
}  // namespace

int main() {
    successful_sections(); circular_sections(); malformed_parameters();
    refused_geometry(); holes_refused(); changing_edges();
    profile_identity(); transaction_fences(); cancellation(); previews_preserve_head();
    if (failures == 0) std::fprintf(stderr, "loft: all checks passed\n");
    return failures == 0 ? 0 : 1;
}
