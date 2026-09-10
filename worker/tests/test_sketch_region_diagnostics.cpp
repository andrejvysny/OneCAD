// WP-S1: a refused sketch profile must say WHICH entities are at fault.
// SCHEMA §7.4 `SketchRegions` `diagnostics` — the refusal reason rides
// `detail.diagnostics[].reasonCode` with `evidence`, and the terminal
// `error.code` stays a closed §8 value. These pin the three layers the evidence
// crosses: LoopDetector (internal ids) → RegionTable (remap or omit) →
// SolverLane (the wire JSON).
#include <cstdio>
#include <string>
#include <vector>

#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "protocol/SolverLane.h"
#include "session/SketchStore.h"
#include "sketch/Sketch.h"
#include "sketch/WireSketch.h"

using nlohmann::json;
using onecad::protocol::Envelope;
namespace loop = onecad::core::loop;
namespace sk = onecad::core::sketch;

namespace {
int g_failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

std::string uuid(unsigned int value) {
    char out[37];
    std::snprintf(out, sizeof(out), "00000000-0000-0000-0000-%012u", value);
    return out;
}

json line(unsigned int id, double x0, double y0, double x1, double y1) {
    return {{"id", uuid(id)}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y1}}};
}

// Two collinear segments overlapping over [10,20], plus a closing chain.
json overlapping_profile() {
    return {
        {"sketchId", "wps1-overlap"},
        {"plane", {{"kind", "XY"}}},
        {"entities", json::array({line(511, 0, 0, 20, 0), line(512, 10, 0, 40, 0),
                                  line(513, 40, 0, 40, 20), line(514, 40, 20, 0, 20),
                                  line(515, 0, 20, 0, 0)})},
        {"constraints", json::array()},
    };
}

// A closed rectangle plus one entity shorter than the node-merge tolerance: the
// region publishes and the dropped entity is reported as an advisory.
json profile_with_degenerate_entity() {
    return {
        {"sketchId", "wps1-degenerate"},
        {"plane", {{"kind", "XY"}}},
        {"entities", json::array({line(521, 0, 0, 40, 0), line(522, 40, 0, 40, 20),
                                  line(523, 40, 20, 0, 20), line(524, 0, 20, 0, 0),
                                  line(525, 10, 10, 10, 10.0000001)})},
        {"constraints", json::array()},
    };
}

loop::LoopDetectionResult detect(const json& sketch, const loop::LoopDetectorConfig& config,
                                 onecad::wire::TranslateResult& translated) {
    translated = onecad::wire::translate(sketch);
    check(translated.ok, "wire sketch translates: " + translated.error);
    if (!translated.ok) return {};
    const sk::SolveResult solve = translated.sketch->solve();
    check(solve.success, "wire sketch solves");
    if (!solve.success) return {};
    loop::LoopDetector detector(config);
    return detector.detect(*translated.sketch);
}

loop::WireEdgeMapper wire_mapper(const onecad::wire::TranslateResult& translated) {
    return [&translated](const sk::EntityID& internal) -> std::string {
        const auto it = translated.index.internal_edge_to_wire.find(internal);
        return it == translated.index.internal_edge_to_wire.end() ? internal : it->second;
    };
}

// --- the lane under test ----------------------------------------------------

struct Lane {
    onecad::session::SketchStore store;
    onecad::protocol::Dispatcher dispatcher;
    onecad::protocol::SolverLane lane;
    std::uint64_t next_id = 1;

    Lane() : lane(store) { lane.register_verbs(dispatcher); }

    Envelope call(const char* verb, json args) {
        return dispatcher.dispatch_once(Envelope::request(next_id++, verb, std::move(args)));
    }

    // Upsert the sketch verbatim, then ask for its regions.
    Envelope regions(const json& sketch) {
        const Envelope up = call("SketchUpsert", sketch);
        check(up.ok.value_or(false), "SketchUpsert ok");
        return call("SketchRegions", {{"sketchId", sketch.at("sketchId")}});
    }
};

// --- 1. the limit-exceeded evidence shape -----------------------------------

void test_limit_exceeded_carries_named_ceiling() {
    loop::LoopDetectorConfig config = loop::makeRegionDetectionConfig();
    config.curveRefinementPolicy = loop::CurveRefinementPolicy::V3PhysicalProximity;
    config.maxPlanarizedSources = 1;
    onecad::wire::TranslateResult translated;
    const loop::LoopDetectionResult detected =
        detect(overlapping_profile(), config, translated);
    check(!detected.success, "the analytic-source ceiling refuses");
    if (detected.success) return;
    // The human sentence is unchanged: this adds evidence, it does not reword.
    check(detected.errorMessage == "profile refinement exceeds analytic-source limit",
          "the source-ceiling message is byte-stable");
    check(detected.refusal.reason == loop::ProfileRefusalReason::LimitExceeded,
          "the source ceiling reports SKETCH_PROFILE_LIMIT_EXCEEDED");
    check(detected.refusal.limitName == "maxPlanarizedSources" &&
              detected.refusal.limit == 1 && detected.refusal.measured == 2,
          "limit evidence names the ceiling and the count that broke it");
    // A ceiling is a property of the whole profile, not of one entity.
    check(detected.refusal.entityIds.empty(), "a ceiling refusal names no entity");

    const loop::RegionTable table = loop::buildRegionTable(
        detected, wire_mapper(translated), sk::constants::COINCIDENCE_TOLERANCE,
        loop::RegionIdentityVersion::V3);
    check(!table.success, "the region table forwards the ceiling refusal");
    check(table.refusal.reason == loop::ProfileRefusalReason::LimitExceeded &&
              table.refusal.limitName == "maxPlanarizedSources" &&
              table.refusal.limit == 1 && table.refusal.measured == 2,
          "limit evidence survives the region table");
}

void test_fragment_ceiling_carries_its_own_name() {
    loop::LoopDetectorConfig config = loop::makeRegionDetectionConfig();
    config.curveRefinementPolicy = loop::CurveRefinementPolicy::V3PhysicalProximity;
    config.maxPlanarizedFragments = 1;
    onecad::wire::TranslateResult translated;
    const loop::LoopDetectionResult detected =
        detect(profile_with_degenerate_entity(), config, translated);
    check(!detected.success, "the fragment ceiling refuses");
    if (detected.success) return;
    check(detected.refusal.reason == loop::ProfileRefusalReason::LimitExceeded &&
              detected.refusal.limitName == "maxPlanarizedFragments" &&
              detected.refusal.limit == 1 && detected.refusal.measured == 2,
          "the fragment ceiling names itself, not the source ceiling");
}

// --- 2. an id that cannot be mapped is omitted, never published raw ----------

void test_unmappable_ids_are_omitted_not_published_raw() {
    loop::LoopDetectorConfig config = loop::makeRegionDetectionConfig();
    config.curveRefinementPolicy = loop::CurveRefinementPolicy::V3PhysicalProximity;
    onecad::wire::TranslateResult translated;
    const loop::LoopDetectionResult detected =
        detect(overlapping_profile(), config, translated);
    check(!detected.success, "the overlapping profile refuses");
    if (detected.success) return;
    check(detected.refusal.entityIds.size() == 2, "the detector names both internal sources");

    // An id the mapper returns unchanged was never in the wire id space. A raw
    // internal id would look right and resolve to nothing for every consumer,
    // which is worse than saying nothing.
    const loop::RegionTable unmapped = loop::buildRegionTable(
        detected, [](const sk::EntityID& id) { return id; },
        sk::constants::COINCIDENCE_TOLERANCE, loop::RegionIdentityVersion::V3);
    check(!unmapped.success, "the unmapped table still refuses");
    check(unmapped.refusal.reason == loop::ProfileRefusalReason::OverlappingCurves,
          "the reason survives even when the ids cannot be mapped");
    check(unmapped.refusal.entityIds.empty(),
          "an unmappable id is omitted rather than published raw");

    // The real mapper resolves both, ascending by source index.
    const loop::RegionTable mapped = loop::buildRegionTable(
        detected, wire_mapper(translated), sk::constants::COINCIDENCE_TOLERANCE,
        loop::RegionIdentityVersion::V3);
    const std::vector<sk::EntityID> expected{uuid(511), uuid(512)};
    check(mapped.refusal.entityIds == expected,
          "a mappable pair publishes both wire ids in source order");
}

// --- 3. the wire JSON, both paths -------------------------------------------

void test_refusal_wire_shape() {
    Lane lane;
    const Envelope resp = lane.regions(overlapping_profile());
    check(!resp.ok.value_or(true), "SketchRegions refuses the overlapping profile");
    check(resp.error.has_value(), "the refusal carries an error");
    if (!resp.error) return;
    // §8's taxonomy is closed: an unknown top-level code fails the whole frame.
    check(resp.error->code == "OP_FAILED", "the terminal code stays OP_FAILED");
    check(resp.error->message ==
              "SketchRegions: profile has overlapping or coincident analytic curves",
          "the terminal message is unchanged");
    check(resp.error->detail.has_value(), "the refusal carries detail");
    if (!resp.error->detail) return;
    const json& detail = *resp.error->detail;
    check(detail.contains("diagnostics") && detail["diagnostics"].is_array() &&
              detail["diagnostics"].size() == 1,
          "detail carries exactly the one refusal diagnostic");
    if (!detail.contains("diagnostics") || detail["diagnostics"].size() != 1) return;
    const json& entry = detail["diagnostics"][0];
    // All three §7.2-required fields, or the Rust reader drops the entry whole.
    check(entry.value("severity", "") == "error", "severity is error");
    check(entry.value("code", "") == "OP_FAILED", "code equals the terminal code");
    check(entry.value("message", "") == resp.error->message, "message is the producer sentence");
    check(entry.value("stage", "") == "profile", "stage is profile");
    check(entry.value("reasonCode", "") == "SKETCH_PROFILE_OVERLAPPING_CURVES",
          "reasonCode routes the refusal");
    const json expected_ids = json::array({uuid(511), uuid(512)});
    check(entry.contains("evidence") && entry["evidence"].value("entityIds", json()) ==
                                            expected_ids,
          "evidence names both wire entity ids");
}

void test_success_wire_shape_publishes_advisories() {
    Lane lane;
    const Envelope resp = lane.regions(profile_with_degenerate_entity());
    check(resp.ok.value_or(false), "SketchRegions publishes the rectangle");
    if (!resp.ok.value_or(false)) return;
    check(resp.result.value("regions", json::array()).size() == 1,
          "the dropped entity does not cost the region");
    check(resp.result.contains("diagnostics") && resp.result["diagnostics"].is_array() &&
              resp.result["diagnostics"].size() == 1,
          "the success result carries the advisory");
    if (!resp.result.contains("diagnostics") || resp.result["diagnostics"].size() != 1) return;
    const json& entry = resp.result["diagnostics"][0];
    check(entry.value("severity", "") == "warning", "the advisory is a warning");
    check(entry.value("code", "") == "SKETCH_ENTITY_DEGENERATE", "the advisory code is named");
    check(entry.value("stage", "") == "profile", "the advisory stage is profile");
    check(!entry.value("message", std::string()).empty(), "the advisory carries a message");
    check(entry.contains("evidence") &&
              entry["evidence"].value("entityId", "") == uuid(525),
          "the advisory names the dropped wire entity (singular entityId)");
}

void test_clean_profile_omits_diagnostics() {
    Lane lane;
    json clean = profile_with_degenerate_entity();
    clean["sketchId"] = "wps1-clean";
    clean["entities"].erase(clean["entities"].begin() + 4);
    const Envelope resp = lane.regions(clean);
    check(resp.ok.value_or(false), "SketchRegions publishes the clean rectangle");
    // Absent means "nothing to report" — an empty array is never emitted.
    check(!resp.result.contains("diagnostics"),
          "a clean profile omits diagnostics entirely");
}

}  // namespace

int main() {
    test_limit_exceeded_carries_named_ceiling();
    test_fragment_ceiling_carries_its_own_name();
    test_unmappable_ids_are_omitted_not_published_raw();
    test_refusal_wire_shape();
    test_success_wire_shape_publishes_advisories();
    test_clean_profile_omits_diagnostics();
    if (g_failures == 0) {
        std::fprintf(stderr, "sketch_region_diagnostics: OK\n");
    }
    return g_failures;
}
