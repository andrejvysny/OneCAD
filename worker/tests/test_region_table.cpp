// Canonical selectable planar-cell table: identity, nesting, strict lookup.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <numbers>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include <BRepGProp.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <GProp_GProps.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>

#include "loop/FaceBuilder.h"
#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "nlohmann/json.hpp"
#include "ops/OpCommon.h"
#include "sketch/RegionId.h"
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

double signed_area(const std::vector<sk::Vec2d>& polygon) {
    double area = 0.0;
    for (std::size_t i = 0; i < polygon.size(); ++i) {
        const sk::Vec2d& a = polygon[i];
        const sk::Vec2d& b = polygon[(i + 1) % polygon.size()];
        area += a.x * b.y - b.x * a.y;
    }
    return area * 0.5;
}

loop::Loop make_loop(std::vector<std::string> edges,
                     std::vector<sk::Vec2d> polygon) {
    loop::Loop result;
    result.wire.edges = std::move(edges);
    result.wire.forward.assign(result.wire.edges.size(), true);
    result.wire.startPoint = "closed";
    result.wire.endPoint = "closed";
    result.polygon = std::move(polygon);
    result.signedArea = signed_area(result.polygon);
    return result;
}

loop::LoopDetectionResult detection(loop::Loop outer,
                                    std::vector<loop::Loop> holes = {}) {
    loop::LoopDetectionResult result;
    result.faces.push_back(loop::Face{std::move(outer), std::move(holes)});
    return result;
}

loop::RegionTable table_from(const loop::LoopDetectionResult& result) {
    return loop::buildRegionTable(
        result, [](const sk::EntityID& id) { return id; },
        sk::constants::COINCIDENCE_TOLERANCE);
}

loop::RegionTable table_from(const json& sketch) {
    onecad::wire::TranslateResult translated = onecad::wire::translate(sketch);
    check(translated.ok, "wire sketch translates");
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
    return loop::buildRegionTable(
        detected, map_edge, sk::constants::COINCIDENCE_TOLERANCE);
}

const loop::RegionDefinition* region_with_outer(
    const loop::RegionTable& table, const std::string& edge) {
    for (const loop::RegionDefinition& region : table.regions) {
        if (std::find(region.outerWireEdges.begin(), region.outerWireEdges.end(), edge) !=
            region.outerWireEdges.end()) {
            return &region;
        }
    }
    return nullptr;
}

void reverse_loop(loop::Loop& value) {
    std::reverse(value.wire.edges.begin(), value.wire.edges.end());
    std::reverse(value.wire.forward.begin(), value.wire.forward.end());
    for (std::size_t i = 0; i < value.wire.forward.size(); ++i) {
        value.wire.forward[i] = !value.wire.forward[i];
    }
    std::reverse(value.polygon.begin(), value.polygon.end());
    value.signedArea = -value.signedArea;
}

void test_identity_determinism() {
    const std::string a = uuid(1);
    const std::string b = uuid(2);
    loop::Loop fragmented = make_loop(
        {a + "#seg0_p0", a + "#seg0_p1", b + "#seg0_p0"},
        {{0, 0}, {4, 0}, {0, 4}});
    const loop::RegionTable base = table_from(detection(fragmented));
    check(base.success && base.regions.size() == 1, "fragmented table builds");

    loop::Loop rotated = fragmented;
    std::rotate(rotated.wire.edges.begin(), rotated.wire.edges.begin() + 1,
                rotated.wire.edges.end());
    std::rotate(rotated.wire.forward.begin(), rotated.wire.forward.begin() + 1,
                rotated.wire.forward.end());
    std::rotate(rotated.polygon.begin(), rotated.polygon.begin() + 1,
                rotated.polygon.end());
    const loop::RegionTable rotated_table = table_from(detection(rotated));
    check(rotated_table.success && rotated_table.regions[0].id == base.regions[0].id,
          "loop start rotation preserves canonical id");

    loop::Loop reversed = fragmented;
    reverse_loop(reversed);
    const loop::RegionTable reversed_table = table_from(detection(reversed));
    check(reversed_table.success && reversed_table.regions[0].id == base.regions[0].id,
          "reversed traversal preserves canonical id after orientation normalization");
}

void test_simple_legacy_and_holes() {
    const std::vector<std::string> outer_ids = {uuid(11), uuid(12), uuid(13), uuid(14)};
    const loop::Loop outer =
        make_loop(outer_ids, {{0, 0}, {20, 0}, {20, 20}, {0, 20}});
    const loop::RegionTable simple = table_from(detection(outer));
    const std::string legacy = onecad::region::derive_region_id(
        outer_ids, onecad::region::Winding::Ccw);
    check(simple.success && simple.regions.size() == 1, "simple table builds");
    check(simple.regions[0].id == legacy, "simple unsplit hole-free id stays legacy");

    loop::Loop hole_a = make_loop(
        {uuid(21), uuid(22), uuid(23), uuid(24)},
        {{2, 2}, {6, 2}, {6, 6}, {2, 6}});
    loop::Loop hole_b = make_loop(
        {uuid(31), uuid(32), uuid(33), uuid(34)},
        {{12, 12}, {18, 12}, {18, 18}, {12, 18}});
    const loop::RegionTable holes_ab = table_from(detection(outer, {hole_a, hole_b}));
    const loop::RegionTable holes_ba = table_from(detection(outer, {hole_b, hole_a}));
    const loop::RegionDefinition* outer_ab = region_with_outer(holes_ab, outer_ids.front());
    const loop::RegionDefinition* outer_ba = region_with_outer(holes_ba, outer_ids.front());
    check(outer_ab && outer_ba, "outer cell survives nested-hole expansion");
    if (outer_ab && outer_ba) {
        check(outer_ab->id != legacy, "adding a hole changes the material-cell id");
        check(outer_ab->id == outer_ba->id, "hole ordering does not change the id");
        check(outer_ab->id == "r_71729d5ae63bdc8c",
              "material-cell byte stream stays fixture-locked");
    }
}

json nested_sketch() {
    const std::string circle = uuid(105);
    return {
        {"sketchId", "nested"},
        {"plane", {{"kind", "XY"}}},
        {"entities",
         json::array({
             {{"id", uuid(101)}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {40, 0}}},
             {{"id", uuid(102)}, {"type", "Line"}, {"p0", {40, 0}}, {"p1", {40, 20}}},
             {{"id", uuid(103)}, {"type", "Line"}, {"p0", {40, 20}}, {"p1", {0, 20}}},
             {{"id", uuid(104)}, {"type", "Line"}, {"p0", {0, 20}}, {"p1", {0, 0}}},
             {{"id", circle}, {"type", "Circle"}, {"center", {20, 10}}, {"radius", 5}},
         })},
        {"constraints", json::array()},
    };
}

double face_area(const TopoDS_Face& face) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(face, props);
    return props.Mass();
}

struct CurveCensus {
    int lines = 0;
    int circles = 0;
    int ellipses = 0;
    int other = 0;
};

CurveCensus curve_census(const TopoDS_Face& face) {
    CurveCensus census;
    for (TopExp_Explorer explorer(face, TopAbs_EDGE); explorer.More(); explorer.Next()) {
        const BRepAdaptor_Curve curve(TopoDS::Edge(explorer.Current()));
        switch (curve.GetType()) {
            case GeomAbs_Line:
                ++census.lines;
                break;
            case GeomAbs_Circle:
                ++census.circles;
                break;
            case GeomAbs_Ellipse:
                ++census.ellipses;
                break;
            default:
                ++census.other;
                break;
        }
    }
    return census;
}

void test_nested_cells_and_profile_lookup() {
    const json sketch = nested_sketch();
    const loop::RegionTable table = table_from(sketch);
    check(table.success && table.regions.size() == 2,
          "rectangle plus contained circle publishes annulus and disk");
    if (!table.success || table.regions.size() != 2) {
        return;
    }

    const loop::RegionDefinition* annulus = nullptr;
    const loop::RegionDefinition* disk = nullptr;
    for (const loop::RegionDefinition& region : table.regions) {
        if (region.holes.empty()) {
            disk = &region;
        } else {
            annulus = &region;
        }
    }
    check(annulus && disk, "nested cells retain independent material boundaries");
    if (!annulus || !disk) {
        return;
    }
    check(annulus->id != disk->id, "nested cells have unique ids");

    std::string error;
    const auto annulus_face =
        onecad::ops::build_profile_face(sketch, annulus->id, 2, error);
    check(annulus_face.has_value(), "annulus canonical id resolves");
    error.clear();
    const auto disk_face = onecad::ops::build_profile_face(sketch, disk->id, 2, error);
    check(disk_face.has_value(), "disk canonical id resolves");
    if (annulus_face && disk_face) {
        const double circle_area = std::numbers::pi_v<double> * 25.0;
        check(std::abs(face_area(*annulus_face) - (800.0 - circle_area)) < 0.1,
              "annulus face subtracts its required hole");
        check(std::abs(face_area(*disk_face) - circle_area) < 0.1,
              "inner disk face remains independently selectable");
    }

    error.clear();
    const auto legacy_face =
        onecad::ops::build_profile_face(sketch, annulus->legacyId, error);
    check(legacy_face.has_value(), "unique legacy outer id resolves compatibly");
    error.clear();
    check(!onecad::ops::build_profile_face(sketch, "r_0000000000000000", error),
          "stale explicit id fails");
    check(error.find("available: [") != std::string::npos,
          "stale-id error lists canonical available ids");
}

json overlapping_circles() {
    return {
        {"sketchId", "overlap"},
        {"plane", {{"kind", "XY"}}},
        {"entities",
         json::array({
             {{"id", uuid(201)}, {"type", "Circle"}, {"center", {-5, 0}}, {"radius", 10}},
             {{"id", uuid(202)}, {"type", "Circle"}, {"center", {5, 0}}, {"radius", 10}},
         })},
        {"constraints", json::array()},
    };
}

void test_overlapping_cells_are_unique_and_bounded() {
    const json sketch = overlapping_circles();
    const loop::RegionTable table = table_from(sketch);
    check(table.success && table.regions.size() == 3,
          "two overlapping circles publish three bounded cells");
    if (!table.success || table.regions.size() != 3) {
        return;
    }

    std::unordered_set<std::string> canonical;
    std::unordered_map<std::string, int> legacy_counts;
    std::vector<double> areas;
    for (const loop::RegionDefinition& region : table.regions) {
        canonical.insert(region.id);
        ++legacy_counts[region.legacyId];
        std::string error;
        const auto face = onecad::ops::build_profile_face(sketch, region.id, 2, error);
        check(face.has_value(), "each overlapping-cell id resolves its bounded face");
        if (face) {
            areas.push_back(face_area(*face));
            const CurveCensus census = curve_census(*face);
            check(census.circles > 0 && census.lines == 0 && census.ellipses == 0 &&
                      census.other == 0,
                  "overlapping cells retain exact circular BRep edges");
        }
    }
    check(canonical.size() == 3, "overlapping cells have three unique canonical ids");
    const loop::RegionTable replay = table_from(sketch);
    std::unordered_set<std::string> replayIds;
    for (const loop::RegionDefinition& region : replay.regions) {
        replayIds.insert(region.id);
    }
    check(replay.success && replayIds == canonical,
          "fragment identity is deterministic across exact re-detection");
    std::sort(areas.begin(), areas.end());
    check(areas.size() == 3 && areas.front() > 100.0 && areas.back() < 210.0,
          "fragment faces are bounded lens/crescents, never whole circles");

    std::string ambiguous;
    for (const auto& [legacy, count] : legacy_counts) {
        if (count > 1) {
            ambiguous = legacy;
            break;
        }
    }
    check(!ambiguous.empty(), "overlap reproduces a legacy outer-id collision");
    if (!ambiguous.empty()) {
        std::string error;
        check(!onecad::ops::build_profile_face(sketch, ambiguous, error),
              "ambiguous legacy id never picks the first cell");
        check(error.find("ambiguous") != std::string::npos,
              "legacy collision returns an explicit ambiguity");
        for (const std::string& id : canonical) {
            check(error.find(id) != std::string::npos,
                  "ambiguity error lists every canonical available id");
        }
    }
}

json circle_with_chord() {
    return {
        {"sketchId", "circle-chord"},
        {"plane", {{"kind", "XY"}}},
        {"entities",
         json::array({
             {{"id", uuid(301)}, {"type", "Circle"}, {"center", {0, 0}}, {"radius", 5}},
             {{"id", uuid(302)}, {"type", "Line"}, {"p0", {-10, 0}}, {"p1", {10, 0}}},
         })},
        {"constraints", json::array()},
    };
}

void test_line_circle_planarization_keeps_exact_edges() {
    const json sketch = circle_with_chord();
    const loop::RegionTable table = table_from(sketch);
    check(table.success && table.regions.size() == 2,
          "a chord through a circle publishes two bounded half-disks");
    if (!table.success || table.regions.size() != 2) {
        return;
    }

    double totalArea = 0.0;
    for (const loop::RegionDefinition& region : table.regions) {
        std::string error;
        const auto face = onecad::ops::build_profile_face(sketch, region.id, 2, error);
        check(face.has_value(), "line-circle canonical cell resolves: " + error);
        if (!face) {
            continue;
        }
        totalArea += face_area(*face);
        const CurveCensus census = curve_census(*face);
        check(census.lines == 1 && census.circles > 0 && census.ellipses == 0 &&
                  census.other == 0,
              "line-circle BRep contains only exact line and circular fragments");
    }
    check(std::abs(totalArea - std::numbers::pi_v<double> * 25.0) < 0.01,
          "line-circle cells conserve analytic disk area");
}

json crossing_arc_circles() {
    const double pi = std::numbers::pi_v<double>;
    return {
        {"sketchId", "crossing-arcs"},
        {"plane", {{"kind", "XY"}}},
        {"entities",
         json::array({
             {{"id", uuid(311)}, {"type", "Arc"}, {"center", {-3, 0}},
              {"radius", 5}, {"startAngle", 0.0}, {"endAngle", pi}},
             {{"id", uuid(312)}, {"type", "Arc"}, {"center", {-3, 0}},
              {"radius", 5}, {"startAngle", pi}, {"endAngle", 0.0}},
             {{"id", uuid(313)}, {"type", "Arc"}, {"center", {3, 0}},
              {"radius", 5}, {"startAngle", 0.0}, {"endAngle", pi}},
             {{"id", uuid(314)}, {"type", "Arc"}, {"center", {3, 0}},
              {"radius", 5}, {"startAngle", pi}, {"endAngle", 0.0}},
         })},
        {"constraints", json::array()},
    };
}

void test_crossing_arcs_publish_three_exact_cells() {
    const json sketch = crossing_arc_circles();
    const loop::RegionTable table = table_from(sketch);
    check(table.success && table.regions.size() == 3,
          "two circles represented by crossing arcs publish lens and crescents");
    if (!table.success || table.regions.size() != 3) {
        return;
    }

    for (const loop::RegionDefinition& region : table.regions) {
        std::string error;
        const auto face = onecad::ops::build_profile_face(sketch, region.id, 2, error);
        check(face.has_value(), "crossing-arc canonical cell resolves: " + error);
        if (!face) {
            continue;
        }
        const CurveCensus census = curve_census(*face);
        check(census.circles > 0 && census.lines == 0 && census.ellipses == 0 &&
                  census.other == 0,
              "crossing arcs stay circular BRep fragments");
    }
}

json rotated_ellipse_with_chord() {
    return {
        {"sketchId", "ellipse-chord"},
        {"plane", {{"kind", "XY"}}},
        {"entities",
         json::array({
             {{"id", uuid(321)}, {"type", "Ellipse"}, {"center", {0, 0}},
              {"majorR", 6}, {"minorR", 3}, {"rotation", 0.4}},
             {{"id", uuid(322)}, {"type", "Line"}, {"p0", {0, -10}}, {"p1", {0, 10}}},
         })},
        {"constraints", json::array()},
    };
}

void test_rotated_ellipse_line_planarization_keeps_exact_edges() {
    const json sketch = rotated_ellipse_with_chord();
    const loop::RegionTable table = table_from(sketch);
    check(table.success && table.regions.size() == 2,
          "a chord through a rotated ellipse publishes two bounded cells");
    if (!table.success || table.regions.size() != 2) {
        return;
    }

    double totalArea = 0.0;
    for (const loop::RegionDefinition& region : table.regions) {
        std::string error;
        const auto face = onecad::ops::build_profile_face(sketch, region.id, 2, error);
        check(face.has_value(), "ellipse-line canonical cell resolves: " + error);
        if (!face) {
            continue;
        }
        totalArea += face_area(*face);
        const CurveCensus census = curve_census(*face);
        check(census.ellipses > 0 && census.lines == 1 && census.circles == 0 &&
                  census.other == 0,
              "ellipse-line BRep contains only exact ellipse and line fragments");
    }
    check(std::abs(totalArea - std::numbers::pi_v<double> * 18.0) < 0.01,
          "ellipse-line cells conserve analytic ellipse area");
}

json tangent_circles() {
    return {
        {"sketchId", "tangent-circles"},
        {"plane", {{"kind", "XY"}}},
        {"entities",
         json::array({
             {{"id", uuid(331)}, {"type", "Circle"}, {"center", {-5, 0}}, {"radius", 5}},
             {{"id", uuid(332)}, {"type", "Circle"}, {"center", {5, 0}}, {"radius", 5}},
         })},
        {"constraints", json::array()},
    };
}

void test_tangent_contacts_do_not_create_degenerate_fragments() {
    const json sketch = tangent_circles();
    const loop::RegionTable table = table_from(sketch);
    check(table.success && table.regions.size() == 2,
          "point tangency keeps two non-degenerate circular regions");
    if (!table.success || table.regions.size() != 2) {
        return;
    }
    for (const loop::RegionDefinition& region : table.regions) {
        std::string error;
        const auto face = onecad::ops::build_profile_face(sketch, region.id, 2, error);
        check(face.has_value(), "tangent circle canonical cell resolves: " + error);
        if (!face) {
            continue;
        }
        const CurveCensus census = curve_census(*face);
        check(census.circles > 0 && census.lines == 0 && census.ellipses == 0 &&
                  census.other == 0,
              "tangent circle remains an exact circular BRep edge");
    }
}

void test_analytic_refinement_honors_cancellation() {
    const json sketch = overlapping_circles();
    onecad::wire::TranslateResult translated = onecad::wire::translate(sketch);
    check(translated.ok, "cancellation wire sketch translates");
    if (!translated.ok) {
        return;
    }
    const sk::SolveResult solve = translated.sketch->solve();
    check(solve.success, "cancellation wire sketch solves");
    if (!solve.success) {
        return;
    }
    loop::LoopDetectorConfig config = loop::makeRegionDetectionConfig();
    config.isCancelled = [] { return true; };
    loop::LoopDetector detector(config);
    const loop::LoopDetectionResult detected = detector.detect(*translated.sketch);
    check(!detected.success, "analytic refinement cancellation refuses publication");
    check(detected.errorMessage == "profile refinement cancelled",
          "analytic refinement cancellation diagnostic remains stable");
}

json coincident_circles() {
    return {
        {"sketchId", "coincident-circles"},
        {"plane", {{"kind", "XY"}}},
        {"entities",
         json::array({
             {{"id", uuid(341)}, {"type", "Circle"}, {"center", {0, 0}}, {"radius", 5}},
             {{"id", uuid(342)}, {"type", "Circle"}, {"center", {0, 0}}, {"radius", 5}},
         })},
        {"constraints", json::array()},
    };
}

void test_coincident_analytic_curves_refuse_stably() {
    const json sketch = coincident_circles();
    onecad::wire::TranslateResult translated = onecad::wire::translate(sketch);
    check(translated.ok, "coincident-circle wire sketch translates");
    if (!translated.ok) {
        return;
    }
    const sk::SolveResult solve = translated.sketch->solve();
    check(solve.success, "coincident-circle wire sketch solves");
    if (!solve.success) {
        return;
    }
    loop::LoopDetector detector;
    detector.setConfig(loop::makeRegionDetectionConfig());
    const loop::LoopDetectionResult detected = detector.detect(*translated.sketch);
    check(!detected.success, "coincident analytic circles are refused");
    check(detected.errorMessage == "profile has overlapping or coincident analytic curves",
          "coincident analytic refusal remains stable");
}

void test_required_hole_failure_is_fatal() {
    sk::Sketch sketch;
    loop::Face face;
    face.outerLoop = make_loop({}, {{0, 0}, {10, 0}, {10, 10}, {0, 10}});
    face.innerLoops.push_back(make_loop({}, {{2, 2}, {4, 2}}));
    const loop::FaceBuildResult built = loop::FaceBuilder().buildFace(face, sketch);
    check(!built.success, "required hole wire failure rejects the face");
    check(built.errorMessage.find("Failed to build hole wire") != std::string::npos,
          "required-hole failure remains diagnostic");
}
}  // namespace

int main() {
    test_identity_determinism();
    test_simple_legacy_and_holes();
    test_nested_cells_and_profile_lookup();
    test_overlapping_cells_are_unique_and_bounded();
    test_line_circle_planarization_keeps_exact_edges();
    test_crossing_arcs_publish_three_exact_cells();
    test_rotated_ellipse_line_planarization_keeps_exact_edges();
    test_tangent_contacts_do_not_create_degenerate_fragments();
    test_analytic_refinement_honors_cancellation();
    test_coincident_analytic_curves_refuse_stably();
    test_required_hole_failure_is_fatal();
    if (g_failures == 0) {
        std::fprintf(stderr, "region_table: OK\n");
    }
    return g_failures;
}
