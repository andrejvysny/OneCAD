// Construction geometry (SCHEMA §7.3 `entities[].construction`) is reference-only:
// it is materialized in the solver like any other entity, but it NEVER bounds a
// region. `WireSketch` used to hardcode construction=false, so the flag Rust has
// always emitted was dropped on the floor and `LoopDetector`'s construction
// filters were dead code. These are the pins for that chain.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "nlohmann/json.hpp"
#include "sketch/Sketch.h"
#include "sketch/SketchPoint.h"
#include "sketch/WireSketch.h"

using nlohmann::json;
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

// Four closed Line entities (inline p0/p1 — the synthesized endpoint Points must
// inherit the parent's flag), ids `base+0..3`.
void push_rect(json& entities, unsigned int base, double x0, double y0, double w,
               double h, bool construction) {
    const double x1 = x0 + w;
    const double y1 = y0 + h;
    const double corner[4][2] = {{x0, y0}, {x1, y0}, {x1, y1}, {x0, y1}};
    for (unsigned int i = 0; i < 4; ++i) {
        const double* a = corner[i];
        const double* b = corner[(i + 1) % 4];
        entities.push_back({
            {"id", uuid(base + i)},
            {"type", "Line"},
            {"p0", {a[0], a[1]}},
            {"p1", {b[0], b[1]}},
            {"construction", construction},
        });
    }
}

json sketch_of(json entities, json constraints = json::array()) {
    return {
        {"sketchId", "construction"},
        {"plane", {{"kind", "XY"}}},
        {"entities", std::move(entities)},
        {"constraints", std::move(constraints)},
    };
}

// Same pipeline `SketchRegions` and plan-time profile derivation run.
loop::RegionTable table_from(const json& sketch) {
    onecad::wire::TranslateResult translated = onecad::wire::translate(sketch);
    check(translated.ok, "wire sketch translates: " + translated.error);
    if (!translated.ok) {
        return {};
    }
    const sk::SolveResult solve = translated.sketch->solve();
    check(solve.success, "wire sketch solves");
    if (!solve.success) {
        return {};
    }
    loop::LoopDetector detector;
    detector.setConfig(loop::makeRegionDetectionConfig());
    const loop::LoopDetectionResult detected = detector.detect(*translated.sketch);
    const auto map_edge = [&](const sk::EntityID& internal) {
        const auto it = translated.index.internal_edge_to_wire.find(internal);
        return it == translated.index.internal_edge_to_wire.end() ? internal : it->second;
    };
    return loop::buildRegionTable(detected, map_edge,
                                  sk::constants::COINCIDENCE_TOLERANCE);
}

// (a) A closed rectangle drawn entirely in construction geometry bounds NOTHING.
void test_construction_rect_publishes_no_region() {
    json entities = json::array();
    push_rect(entities, 10, 0, 0, 20, 20, /*construction=*/true);
    const loop::RegionTable table = table_from(sketch_of(entities));
    check(table.success, "all-construction sketch still builds a region table");
    check(table.regions.empty(),
          "a closed rectangle of construction lines publishes NO region (got " +
              std::to_string(table.regions.size()) + ")");
}

// (b) Excluding construction geometry leaves the remaining region ids
//     byte-identical — signatures derive only from loop edge wire-ids.
void test_construction_does_not_perturb_remaining_region_ids() {
    json real_only = json::array();
    push_rect(real_only, 30, 40, 0, 20, 20, /*construction=*/false);
    const loop::RegionTable baseline = table_from(sketch_of(real_only));
    check(baseline.success && baseline.regions.size() == 1,
          "the real rectangle alone publishes exactly one region");
    if (!baseline.success || baseline.regions.size() != 1) {
        return;
    }

    json mixed = json::array();
    push_rect(mixed, 10, 0, 0, 20, 20, /*construction=*/true);
    push_rect(mixed, 30, 40, 0, 20, 20, /*construction=*/false);
    const loop::RegionTable table = table_from(sketch_of(mixed));
    check(table.success && table.regions.size() == 1,
          "construction rectangle + real rectangle publishes only the real cell (got " +
              std::to_string(table.regions.size()) + ")");
    if (!table.success || table.regions.size() != 1) {
        return;
    }
    check(table.regions[0].id == baseline.regions[0].id,
          "the surviving region id is byte-identical to the real rectangle alone (" +
              table.regions[0].id + " vs " + baseline.regions[0].id + ")");
    check(table.regions[0].legacyId == baseline.regions[0].legacyId,
          "the surviving legacy id is unchanged too");
}

// (c) Construction geometry is still SOLVED: a dimension on a construction line
//     removes a DOF and actually moves the geometry.
void test_constraint_on_construction_geometry_still_solves() {
    const std::string line = uuid(50);
    json entities = json::array({{
        {"id", line},
        {"type", "Line"},
        {"p0", {0.0, 0.0}},
        {"p1", {10.0, 0.0}},
        {"construction", true},
    }});

    onecad::wire::TranslateResult free_sketch = onecad::wire::translate(sketch_of(entities));
    check(free_sketch.ok, "free construction line translates");
    if (!free_sketch.ok) {
        return;
    }
    const int free_dof = free_sketch.sketch->getDegreesOfFreedom();

    json constraints = json::array({{
        {"id", uuid(51)},
        {"type", "Distance"},
        {"entities", json::array({line})},
        {"value", 25.0},
    }});
    onecad::wire::TranslateResult dimensioned =
        onecad::wire::translate(sketch_of(entities, constraints));
    check(dimensioned.ok, "dimensioned construction line translates: " + dimensioned.error);
    if (!dimensioned.ok) {
        return;
    }
    const sk::SolveResult solve = dimensioned.sketch->solve();
    check(solve.success, "a dimension on construction geometry solves");
    check(dimensioned.sketch->getDegreesOfFreedom() == free_dof - 1,
          "the dimension removes exactly one DOF from the construction line (" +
              std::to_string(dimensioned.sketch->getDegreesOfFreedom()) + " vs " +
              std::to_string(free_dof) + ")");

    const sk::EntityID p0 = dimensioned.index.resolve_point(line, "p0");
    const sk::EntityID p1 = dimensioned.index.resolve_point(line, "p1");
    const auto* start = dimensioned.sketch->getEntityAs<sk::SketchPoint>(p0);
    const auto* end = dimensioned.sketch->getEntityAs<sk::SketchPoint>(p1);
    check(start && end, "construction endpoints stay addressable after the solve");
    if (start && end) {
        const double length = start->position().Distance(end->position());
        check(std::abs(length - 25.0) < 1e-6,
              "the construction line really moved to the dimensioned length (" +
                  std::to_string(length) + ")");
    }

    // …and it is still invisible to region detection.
    const loop::RegionTable table = table_from(sketch_of(entities, constraints));
    check(table.success && table.regions.empty(),
          "a solved construction line publishes no region");
}
}  // namespace

int main() {
    test_construction_rect_publishes_no_region();
    test_construction_does_not_perturb_remaining_region_ids();
    test_constraint_on_construction_geometry_still_solves();
    if (g_failures == 0) {
        std::fprintf(stderr, "sketch_construction: OK\n");
    }
    return g_failures;
}
