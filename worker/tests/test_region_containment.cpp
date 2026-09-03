// WP-B probes — robust region detection. RED-FIRST measurement, no production fix.
//
// P-B2 `test_inner_circle_is_a_hole_of_the_chorded_disk`
//   A circle R=100 split by a chord at y=50 publishes two bounded cells. A small
//   r=1 circle sitting 0.8 mm inside the rim of the LARGE cell, at 262.5° — the
//   angular MIDPOINT of a sample segment, where the chord sagitta is maximal —
//   must be that cell's hole. Containment is decided against the SAMPLED polygon,
//   so this pins whether nesting survives a large-radius arc. The test also emits
//   a clearance sweep at two rim angles (a sample vertex, and a segment midpoint);
//   that sweep is measurement only and asserts nothing, so it cannot change the
//   verdict.
//
// P-B3 `test_hundred_line_profile_still_detects_its_region`
//   A closed 100-line zig-zag is an ordinary profile. Exact analytic refinement
//   enumerates C(n,2) curve pairs and refuses above `maxPlanarizedCurvePairs`
//   (4096, LoopDetector.cpp:1119-1128); C(100,2) = 4950. This pins whether a
//   perfectly legal profile is refused for its entity COUNT.
//
// P-B5 `test_degenerate_entity_is_warned_not_refused`
//   One zero-length line beside an otherwise ordinary rect+circle sketch. Exact
//   refinement cannot give it analytic provenance, and under V3 that refused the
//   WHOLE graph ("profile refinement could not preserve analytic provenance").
//   This pins that a single degenerate entity is dropped with a warning instead.
#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <limits>
#include <numbers>
#include <optional>
#include <string>
#include <unordered_set>
#include <vector>

#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "nlohmann/json.hpp"
#include "ops/OpCommon.h"
#include "sketch/Sketch.h"
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

json line_entity(unsigned int id, double x0, double y0, double x1, double y1) {
    return {{"id", uuid(id)},
            {"type", "Line"},
            {"p0", json::array({x0, y0})},
            {"p1", json::array({x1, y1})}};
}

json circle_entity(unsigned int id, double cx, double cy, double r) {
    return {{"id", uuid(id)},
            {"type", "Circle"},
            {"center", json::array({cx, cy})},
            {"radius", r}};
}

json sketch_of(const std::string& name, json entities) {
    return {
        {"sketchId", name},
        {"plane", {{"kind", "XY"}}},
        {"entities", std::move(entities)},
        {"constraints", json::array()},
    };
}

// The V3 publication path verbatim: `makeRegionDetectionConfig` (which already
// sets `exactAnalyticFragments`) plus the physical-proximity refinement policy,
// exactly as `SolverLane::on_regions` and `ops::build_profile_face` configure it.
loop::RegionTable v3_table(const json& sketch,
                           loop::LoopDetectionResult* detection_out = nullptr) {
    onecad::wire::TranslateResult translated = onecad::wire::translate(sketch);
    check(translated.ok, "wire sketch translates: " + translated.error);
    if (!translated.ok) return {};
    const sk::SolveResult solve = translated.sketch->solve();
    check(solve.success, "wire sketch solves: " + solve.errorMessage);
    if (!solve.success) return {};

    loop::LoopDetectorConfig config = loop::makeRegionDetectionConfig();
    config.curveRefinementPolicy = loop::CurveRefinementPolicy::V3PhysicalProximity;
    loop::LoopDetector detector;
    detector.setConfig(config);
    const loop::LoopDetectionResult detected = detector.detect(*translated.sketch);
    if (detection_out) *detection_out = detected;
    const auto map_edge = [&](const sk::EntityID& internal) {
        const auto it = translated.index.internal_edge_to_wire.find(internal);
        return it == translated.index.internal_edge_to_wire.end() ? internal : it->second;
    };
    return loop::buildRegionTable(detected, map_edge, sk::constants::COINCIDENCE_TOLERANCE,
                                  loop::RegionIdentityVersion::V3);
}

std::unordered_set<std::string> region_ids(const loop::RegionTable& table) {
    std::unordered_set<std::string> ids;
    for (const loop::RegionDefinition& region : table.regions) ids.insert(region.id);
    return ids;
}

double polygon_area(const std::vector<sk::Vec2d>& polygon) {
    double twice = 0.0;
    for (std::size_t i = 0; i < polygon.size(); ++i) {
        const sk::Vec2d& a = polygon[i];
        const sk::Vec2d& b = polygon[(i + 1) % polygon.size()];
        twice += (a.x * b.y) - (b.x * a.y);
    }
    return std::abs(twice) * 0.5;
}

// Smallest distance from the origin to the midpoint of a polygon edge that
// approximates the ARC: both endpoints on the rim of radius `radius`, and shorter
// than `radius` so the authored chord (which also has both endpoints on the rim,
// but spans 1.73·radius) is excluded. The samples themselves lie on the analytic
// curve, so the sagitta only shows up between them, and it is that inward dip that
// decides a near-rim containment test.
double min_arc_edge_midpoint_radius(const std::vector<sk::Vec2d>& polygon, double radius) {
    double smallest = std::numeric_limits<double>::infinity();
    for (std::size_t i = 0; i < polygon.size(); ++i) {
        const sk::Vec2d& a = polygon[i];
        const sk::Vec2d& b = polygon[(i + 1) % polygon.size()];
        if (std::abs(std::hypot(a.x, a.y) - radius) > 1e-6) continue;
        if (std::abs(std::hypot(b.x, b.y) - radius) > 1e-6) continue;
        if (std::hypot(b.x - a.x, b.y - a.y) >= radius) continue;
        const double mx = (a.x + b.x) * 0.5;
        const double my = (a.y + b.y) * 0.5;
        smallest = std::min(smallest, std::hypot(mx, my));
    }
    return smallest;
}

void dump_table(const char* label, const loop::RegionTable& table) {
    std::fprintf(stderr, "%s: success=%d error='%s' regions=%zu\n", label,
                 table.success ? 1 : 0, table.errorMessage.c_str(), table.regions.size());
    for (std::size_t i = 0; i < table.regions.size(); ++i) {
        const loop::RegionDefinition& region = table.regions[i];
        std::fprintf(stderr,
                     "  [%zu] id=%s outerEdges=%zu outerPolygon=%zu outerArea=%.6f holes=%zu\n",
                     i, region.id.c_str(), region.outerWireEdges.size(),
                     region.outerLoop.polygon.size(), region.outerLoop.area(),
                     region.holes.size());
        for (std::size_t h = 0; h < region.holes.size(); ++h) {
            std::fprintf(stderr, "        hole[%zu] polygon=%zu area=%.6f\n", h,
                         region.holes[h].polygon.size(), region.holes[h].area());
        }
    }
}

// ── P-B2 ─────────────────────────────────────────────────────────────────────
//
// R=100 disk, chord y=50 with both endpoints exactly on the rim
// (x = ±√(100² − 50²) = ±86.60254037844386), plus a r=1 circle placed near the
// rim of the LARGE cell of the two the chord makes.
//
// The ASSERTING placement is the one the clearance sweep below measures as the
// defect: centre at 262.5°, the angular MIDPOINT of a 16-segment sample of the
// large cell's 240° arc, where the chord sagitta is maximal, with 0.8 mm between
// the small circle's rim and the disk's. A 16-segment polygon dips 0.856 mm
// inward there and loses the hole; a chord-tolerance sample count keeps the dip
// under 0.01 mm and keeps it.
constexpr double kInnerAngleDegrees = 262.5;
constexpr double kInnerRadius = 1.0;
constexpr double kInnerClearance = 0.8;

double inner_angle_radians() {
    return kInnerAngleDegrees * std::numbers::pi_v<double> / 180.0;
}

json chorded_disk_with_inner_circle(double innerCx, double innerCy) {
    const double halfChord = std::sqrt((100.0 * 100.0) - (50.0 * 50.0));
    return sketch_of("chorded-disk",
                     json::array({
                         circle_entity(201, 0.0, 0.0, 100.0),
                         line_entity(202, -halfChord, 50.0, halfChord, 50.0),
                         circle_entity(203, innerCx, innerCy, kInnerRadius),
                     }));
}

// The asserting fixture, at the sweep's measured failure point.
json chorded_disk_with_inner_circle() {
    const double centreRadius = 100.0 - kInnerClearance - kInnerRadius;
    return chorded_disk_with_inner_circle(centreRadius * std::cos(inner_angle_radians()),
                                          centreRadius * std::sin(inner_angle_radians()));
}

/// The largest cell's hole count, or `-1` when the table refused.
long largest_cell_holes(const loop::RegionTable& table) {
    if (!table.success) return -1;
    long holes = -1;
    double largestArea = -1.0;
    for (const loop::RegionDefinition& region : table.regions) {
        if (region.outerLoop.area() > largestArea) {
            largestArea = region.outerLoop.area();
            holes = static_cast<long>(region.holes.size());
        }
    }
    return holes;
}

// Pure MEASUREMENT, no verdict: how much rim clearance the sampled polygon
// actually costs, at two rim positions. 270° is where a fragment SAMPLE lands
// (the polygon touches the true rim there, so no sagitta at all); 262.5° is the
// angular MIDPOINT of a sample segment, where the 0.856 mm sagitta is maximal.
void measure_containment_clearance_limit() {
    std::fprintf(stderr,
                 "P-B2 clearance sweep (largest-cell hole count; -1 = table refused):\n");
    for (const double degrees : {270.0, 262.5}) {
        const double radians = degrees * std::numbers::pi_v<double> / 180.0;
        for (const double clearance : {2.0, 1.5, 1.0, 0.9, 0.8, 0.7, 0.5, 0.25}) {
            const double centreRadius = 100.0 - clearance - 1.0;
            const loop::RegionTable table = v3_table(chorded_disk_with_inner_circle(
                centreRadius * std::cos(radians), centreRadius * std::sin(radians)));
            std::fprintf(stderr,
                         "  angle %6.2f° clearance %.2f mm: success=%d regions=%zu "
                         "largestCellHoles=%ld\n",
                         degrees, clearance, table.success ? 1 : 0, table.regions.size(),
                         largest_cell_holes(table));
        }
    }
}

void test_inner_circle_is_a_hole_of_the_chorded_disk() {
    const json sketch = chorded_disk_with_inner_circle();
    const loop::RegionTable table = v3_table(sketch);
    dump_table("P-B2 chorded disk + inner circle", table);
    check(table.success, "chorded disk with an inner circle builds a region table: " +
                             table.errorMessage);
    if (!table.success) return;

    // Analytic expectation: the chord's endpoints sit at ±30°, so it subtends a
    // 120° arc. The small cell is that circular segment, R²/2·(θ − sinθ) =
    // 5000·(2.0944 − 0.8660) = 6141.848; the large cell is the rest of the disk,
    // 31415.927 − 6141.848 = 25274.078, less the π hole. Both regions carry
    // SAMPLED polygons, so their measured areas sit just under those figures.
    check(table.regions.size() == 3,
          "chord + contained circle publishes segment, large cell and inner disk");

    const loop::RegionDefinition* large = nullptr;
    for (const loop::RegionDefinition& region : table.regions) {
        if (!large || region.outerLoop.area() > large->outerLoop.area()) large = &region;
    }
    if (!large) {
        check(false, "the table published no region at all");
        return;
    }

    const double sagittaRadius = min_arc_edge_midpoint_radius(large->outerLoop.polygon, 100.0);
    // The point on the small circle nearest the disk's rim: the containment test
    // that decides the nesting turns on exactly this point.
    const double rimProbeRadius = 100.0 - kInnerClearance;
    const sk::Vec2d rimProbe{rimProbeRadius * std::cos(inner_angle_radians()),
                             rimProbeRadius * std::sin(inner_angle_radians())};
    const bool rimInside = large->outerLoop.contains(rimProbe);
    std::fprintf(stderr,
                 "  large cell: outerPolygon=%zu polygonArea=%.6f (analytic 25274.078) "
                 "minArcEdgeMidRadius=%.6f (sagitta %.6f mm vs %.3f mm clearance) "
                 "containsNearRimPointOfInnerCircle(%.4f,%.4f)=%d holes=%zu\n",
                 large->outerLoop.polygon.size(), polygon_area(large->outerLoop.polygon),
                 sagittaRadius, 100.0 - sagittaRadius, kInnerClearance, rimProbe.x,
                 rimProbe.y, rimInside ? 1 : 0, large->holes.size());

    check(large->holes.size() == 1,
          "the r=1 circle 0.8 mm inside the rim is the large cell's hole");
    if (large->holes.size() == 1) {
        // The hole loop is the circle's own SAMPLED polygon (32 segments), whose
        // area is 16·sin(11.25°) = 3.1214, hence the band rather than an equality.
        const double holeArea = large->holes.front().area();
        check(std::abs(holeArea - std::numbers::pi_v<double>) / std::numbers::pi_v<double> < 0.01,
              "…and that hole is the unit circle (sampled area within 1% of π)");
    }

    measure_containment_clearance_limit();
}

// ── P-B3 ─────────────────────────────────────────────────────────────────────
//
// 97 zig-zag segments between (0,0) and (97,5) plus three closing lines
// ((97,5)→(97,−5)→(0,−5)→(0,0)) — exactly 100 Line entities forming one simple
// closed polygon.
json hundred_line_zigzag() {
    json entities = json::array();
    unsigned int id = 401;
    const auto tooth = [](int k) {
        return sk::Vec2d{static_cast<double>(k), (k % 2) == 0 ? 0.0 : 5.0};
    };
    for (int k = 0; k < 97; ++k) {
        const sk::Vec2d a = tooth(k);
        const sk::Vec2d b = tooth(k + 1);
        entities.push_back(line_entity(id++, a.x, a.y, b.x, b.y));
    }
    entities.push_back(line_entity(id++, 97.0, 5.0, 97.0, -5.0));
    entities.push_back(line_entity(id++, 97.0, -5.0, 0.0, -5.0));
    entities.push_back(line_entity(id++, 0.0, -5.0, 0.0, 0.0));
    return sketch_of("zigzag-100", std::move(entities));
}

void test_hundred_line_profile_still_detects_its_region() {
    const json sketch = hundred_line_zigzag();
    check(sketch["entities"].size() == 100, "the zig-zag fixture is exactly 100 lines");

    loop::LoopDetectionResult detection;
    const auto started = std::chrono::steady_clock::now();
    const loop::RegionTable table = v3_table(sketch, &detection);
    const auto elapsed = std::chrono::duration_cast<std::chrono::microseconds>(
        std::chrono::steady_clock::now() - started);
    // C(100,2) = 4950 enumerated pairs against a 4096 ceiling; the bbox cull is
    // what has to bring the SURVIVING count under it.
    std::fprintf(stderr,
                 "P-B3 100-line zig-zag: %.3f ms, curve pairs after cull = %d "
                 "(enumerated C(100,2) = 4950, limit 4096)\n",
                 static_cast<double>(elapsed.count()) / 1000.0,
                 detection.planarizedCurvePairs);
    dump_table("P-B3 100-line zig-zag", table);

    // The production-facing message, through the same entry point an Extrude uses.
    std::string profileError;
    const auto face = onecad::ops::build_profile_face(sketch, "r_deadbeefdeadbeef", 3,
                                                      profileError);
    std::fprintf(stderr, "P-B3 build_profile_face: face=%d err='%s'\n", face.has_value() ? 1 : 0,
                 profileError.c_str());

    check(table.success,
          "a closed 100-line profile builds a region table: " + table.errorMessage);
    if (!table.success) return;
    check(table.regions.size() == 1, "the closed zig-zag is exactly one selectable cell");
    if (table.regions.size() == 1) {
        // 97 teeth of unit pitch over y ∈ [0,5] plus the 5 mm apron below:
        // the apron is 97×5 = 485 and the saw-tooth band adds 97 half-cells of
        // 1×5/2, i.e. 242.5 — 727.5 in total.
        const double area = table.regions.front().outerLoop.area();
        std::fprintf(stderr, "  zig-zag cell area = %.6f (analytic 727.5)\n", area);
        check(std::abs(area - 727.5) < 0.5, "…with the zig-zag's analytic area");
    }
}

// ── P-B5 ─────────────────────────────────────────────────────────────────────
//
// A 40×20 rectangle plus a r=8 circle centred on its right edge publishes three
// bounded cells: the rectangle minus the circle's left lens, the lens itself, and
// the circle's right half outside the rectangle. Adding ONE zero-length line
// somewhere else on the plane must drop that entity with a warning and leave the
// other three cells exactly as they were — not refuse the whole graph.
json rect_with_straddling_circle(bool withDegenerateLine) {
    json entities = json::array({
        line_entity(501, 0.0, 0.0, 40.0, 0.0),
        line_entity(502, 40.0, 0.0, 40.0, 20.0),
        line_entity(503, 40.0, 20.0, 0.0, 20.0),
        line_entity(504, 0.0, 20.0, 0.0, 0.0),
        circle_entity(505, 40.0, 10.0, 8.0),
    });
    if (withDegenerateLine) {
        entities.push_back(line_entity(506, 60.0, 60.0, 60.0, 60.0));
    }
    return sketch_of("rect-straddling-circle", std::move(entities));
}

void test_degenerate_entity_is_warned_not_refused() {
    const loop::RegionTable clean = v3_table(rect_with_straddling_circle(false));
    dump_table("P-B5 rect + straddling circle", clean);
    check(clean.success, "the rect + straddling circle builds a region table: " +
                             clean.errorMessage);
    check(clean.regions.size() == 3, "…as three bounded cells");
    check(clean.warnings.empty(), "…with no warnings");

    const loop::RegionTable withDegenerate = v3_table(rect_with_straddling_circle(true));
    dump_table("P-B5 + one zero-length line", withDegenerate);
    for (const loop::DetectionWarning& warning : withDegenerate.warnings) {
        std::fprintf(stderr, "  warning: entityId=%s text='%s'\n",
                     warning.entityId.c_str(), warning.text().c_str());
    }

    check(withDegenerate.success,
          "a zero-length line does not refuse the whole graph: " +
              withDegenerate.errorMessage);
    check(withDegenerate.regions.size() == 3,
          "…the same three cells are still published");
    check(withDegenerate.warnings.size() == 1, "…and it reports exactly one warning");
    if (withDegenerate.warnings.size() == 1) {
        const loop::DetectionWarning& warning = withDegenerate.warnings.front();
        // The warning must name the entity in the CALLER's id space (the wire
        // uuid), not the detector's internal id, or it cannot be acted on.
        check(warning.entityId == uuid(506),
              "…naming the degenerate entity by its wire id (got '" +
                  warning.entityId + "')");
        check(warning.text().find(uuid(506)) != std::string::npos &&
                  warning.text().find("degenerate") != std::string::npos,
              "…in a message that names the entity and says it was ignored");
    }
    check(region_ids(clean) == region_ids(withDegenerate),
          "…and the surviving cells keep their identities");

    // The same drop must reach an operation as a `warning` diagnostic rather than
    // failing the step: select the largest cell (the rectangle minus the lens).
    const loop::RegionDefinition* largest = nullptr;
    for (const loop::RegionDefinition& region : withDegenerate.regions) {
        if (!largest || region.outerLoop.area() > largest->outerLoop.area()) largest = &region;
    }
    if (!largest) return;
    std::string profileError;
    std::vector<json> diagnostics;
    const auto face = onecad::ops::build_profile_face(
        rect_with_straddling_circle(true), largest->id, 3, profileError, &diagnostics);
    std::fprintf(stderr, "P-B5 build_profile_face: face=%d err='%s' diagnostics=%s\n",
                 face.has_value() ? 1 : 0, profileError.c_str(), json(diagnostics).dump().c_str());
    check(face.has_value(), "build_profile_face still builds the profile: " + profileError);
    check(diagnostics.size() == 1, "…and forwards one diagnostic");
    if (diagnostics.size() == 1) {
        check(diagnostics[0].value("severity", "") == "warning" &&
                  diagnostics[0].value("code", "") == "SKETCH_ENTITY_DEGENERATE" &&
                  diagnostics[0]["evidence"].value("entityId", "") == uuid(506),
              "…shaped as a §7.2 warning naming the entity");
    }
}
// P-B6: a hole edge split so its last piece is 0.0005 mm long. The graph merges
// nodes at 1e-6 mm, so that piece is a real edge with two distinct nodes;
// dropping it (the authoring-resolution threshold did) opened the hole loop and
// deleted the hole from the region — an extrude then published 8000 mm³ where
// the user drew 7000 (adversarial review 2026-09-03). Must NOT be dropped.
json rect_with_split_hole_edge() {
    return sketch_of("rect-split-hole-edge",
                     json::array({
                         line_entity(601, 0.0, 0.0, 40.0, 0.0),
                         line_entity(602, 40.0, 0.0, 40.0, 20.0),
                         line_entity(603, 40.0, 20.0, 0.0, 20.0),
                         line_entity(604, 0.0, 20.0, 0.0, 0.0),
                         line_entity(605, 10.0, 5.0, 19.9995, 5.0),
                         line_entity(606, 19.9995, 5.0, 20.0, 5.0),
                         line_entity(607, 20.0, 5.0, 20.0, 15.0),
                         line_entity(608, 20.0, 15.0, 10.0, 15.0),
                         line_entity(609, 10.0, 15.0, 10.0, 5.0),
                     }));
}

void test_sub_resolution_edge_is_kept_and_the_hole_survives() {
    const loop::RegionTable table = v3_table(rect_with_split_hole_edge());
    dump_table("P-B6 rect with a 0.0005 mm hole-edge piece", table);
    for (const loop::DetectionWarning& warning : table.warnings) {
        std::fprintf(stderr, "  warning: %s\n", warning.text().c_str());
    }
    check(table.success, "the split-hole sketch builds a region table: " + table.errorMessage);
    check(table.warnings.empty(), "…a 0.0005 mm edge is real boundary, not degenerate (no warning)");
    check(table.regions.size() == 2, "…two cells: the ring and the hole");
    check(largest_cell_holes(table) == 1, "…and the ring KEEPS its hole");
}

// P-B7: a 0.0005 mm chamfer segment on a rectangle corner closes the loop; it
// must not be dropped either (that turned a rectangle into "no closed region").
void test_sub_resolution_chamfer_still_closes_the_rectangle() {
    const loop::RegionTable table = v3_table(sketch_of(
        "rect-micro-chamfer",
        json::array({
            line_entity(701, 0.0, 0.0, 40.0, 0.0),
            line_entity(702, 40.0, 0.0, 40.0, 20.0),
            line_entity(703, 40.0, 20.0, 0.0005, 20.0),
            line_entity(704, 0.0005, 20.0, 0.0, 19.9995),
            line_entity(705, 0.0, 19.9995, 0.0, 0.0),
        })));
    dump_table("P-B7 rect with a 0.0005 mm chamfer", table);
    check(table.success, "the chamfered rectangle builds a region table: " + table.errorMessage);
    check(table.warnings.empty(), "…with no degenerate warning");
    check(table.regions.size() == 1, "…and exactly one cell");
}
}  // namespace

int main() {
    test_inner_circle_is_a_hole_of_the_chorded_disk();
    test_hundred_line_profile_still_detects_its_region();
    test_degenerate_entity_is_warned_not_refused();
    test_sub_resolution_edge_is_kept_and_the_hole_survives();
    test_sub_resolution_chamfer_still_closes_the_rectangle();
    if (g_failures == 0) {
        std::fprintf(stderr, "region_containment: OK\n");
    }
    return g_failures;
}
