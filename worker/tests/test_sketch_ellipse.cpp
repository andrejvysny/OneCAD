// Ellipse on the solver lane (SCHEMA §7.3/§7.4). `WireSketch::translate` used to
// reject `type: "Ellipse"` outright ("unsupported entity type"), which closed BOTH
// lanes at once — SketchUpsert/SketchRegions and the shared plan-time profile
// translate in ops/OpCommon.cpp. Everything downstream already handled ellipses
// (LoopDetector tessellation, FaceBuilder's true Geom_Ellipse edge, the naive-DOF
// fallback for solver-unsupported entities); only the wire branch was missing.
// These are the pins for that branch.
#include <cmath>
#include <cstdio>
#include <numbers>
#include <string>
#include <vector>

#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "nlohmann/json.hpp"
#include "sketch/Sketch.h"
#include "sketch/SketchEllipse.h"
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

json ellipse_entity(const std::string& id, double cx, double cy, double major, double minor,
                    double rotation, bool construction = false) {
    return {
        {"id", id},          {"type", "Ellipse"},   {"center", {cx, cy}},
        {"majorR", major},   {"minorR", minor},     {"rotation", rotation},
        {"construction", construction},
    };
}

json sketch_of(json entities, json constraints = json::array()) {
    return {
        {"sketchId", "ellipse"},
        {"plane", {{"kind", "XY"}}},
        {"entities", std::move(entities)},
        {"constraints", std::move(constraints)},
    };
}

// Same pipeline SketchRegions and plan-time profile derivation run.
loop::RegionTable table_from(const json& sketch) {
    onecad::wire::TranslateResult translated = onecad::wire::translate(sketch);
    check(translated.ok, "wire sketch translates: " + translated.error);
    if (!translated.ok) {
        return {};
    }
    const sk::SolveResult solve = translated.sketch->solve();
    check(solve.success, "wire sketch solves: " + solve.errorMessage);
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
    return loop::buildRegionTable(detected, map_edge, sk::constants::COINCIDENCE_TOLERANCE);
}

// (a) The wire branch mints the center Point itself and registers `<id>.center`
//     as its PRIMARY handle — exactly the Circle/Arc contract, so a re-entering
//     frontend can address (and drag) the center.
void test_ellipse_translates_and_owns_its_center_point() {
    onecad::wire::TranslateResult tr =
        onecad::wire::translate(sketch_of(json::array({ellipse_entity("el", 0, 0, 6, 3, 0.25)})));
    check(tr.ok, "a lone ellipse translates: " + tr.error);
    if (!tr.ok) {
        return;
    }
    check(tr.sketch->getAllEntities().size() == 2,
          "one wire ellipse materializes 2 entities (center Point + Ellipse), got " +
              std::to_string(tr.sketch->getAllEntities().size()));

    const sk::EntityID center = tr.index.resolve_point("el", "center");
    check(!center.empty(), "`el.center` resolves to the minted center point");
    check(tr.index.handle_for(center) == "el.center",
          "`el.center` is the PRIMARY handle of the minted point (got '" +
              tr.index.handle_for(center) + "')");

    const auto ellipse_it = tr.index.wire_to_internal.find("el");
    check(ellipse_it != tr.index.wire_to_internal.end(), "the ellipse itself is id-mapped");
    if (ellipse_it == tr.index.wire_to_internal.end()) {
        return;
    }
    const auto* el = tr.sketch->getEntityAs<sk::SketchEllipse>(ellipse_it->second);
    check(el != nullptr, "the mapped entity really is a SketchEllipse");
    if (!el) {
        return;
    }
    check(std::abs(el->majorRadius() - 6.0) < 1e-9 && std::abs(el->minorRadius() - 3.0) < 1e-9 &&
              std::abs(el->rotation() - 0.25) < 1e-9,
          "majorR/minorR/rotation arrive verbatim (already normalized: major >= minor)");
    check(el->centerPointId() == center, "the ellipse points at the handle-registered center");
    check(!el->isConstruction(), "construction defaults false");
}

// (b) DOF: the solver lane cannot represent an ellipse (SolverAdapter skips it —
//     legacy parity, the oracle solver has zero ellipse refs), so an ellipse-
//     bearing sketch falls back to NAIVE counting: ellipse 3 + center point 2.
void test_ellipse_dof_uses_the_naive_fallback() {
    onecad::wire::TranslateResult free_sketch =
        onecad::wire::translate(sketch_of(json::array({ellipse_entity("el", 0, 0, 6, 3, 0.0)})));
    check(free_sketch.ok, "free ellipse translates: " + free_sketch.error);
    if (!free_sketch.ok) {
        return;
    }
    check(free_sketch.sketch->hasSolverUnsupportedEntities(),
          "an ellipse marks the sketch solver-unsupported (naive DOF path)");
    check(free_sketch.sketch->getDegreesOfFreedom() == 5,
          "lone ellipse dof == 5 (3 shape + 2 center), got " +
              std::to_string(free_sketch.sketch->getDegreesOfFreedom()));

    json constraints = json::array({{
        {"id", "cF"},
        {"type", "Fixed"},
        {"entities", json::array({"el"})},
        {"positions", json::array({"center"})},
    }});
    onecad::wire::TranslateResult fixed = onecad::wire::translate(
        sketch_of(json::array({ellipse_entity("el", 0, 0, 6, 3, 0.0)}), constraints));
    check(fixed.ok, "Fixed on the ellipse center translates: " + fixed.error);
    if (!fixed.ok) {
        return;
    }
    const sk::SolveResult solve = fixed.sketch->solve();
    check(solve.success, "an ellipse sketch with a Fixed center still solves: " + solve.errorMessage);
    check(fixed.sketch->getDegreesOfFreedom() == 3,
          "Fixed center removes both center DOF => naive dof 3, got " +
              std::to_string(fixed.sketch->getDegreesOfFreedom()));
    check(!fixed.sketch->hasRedundantConstraints(),
          "redundancy is a PlaneGCS notion — unreported on the naive path");
}

// (c) Corpus oracle: corpus/cases/i_multiregion_loop_detection.json case
//     `regions_ellipse` — center (0,0), rx 6, ry 3, rotation 0.25 => ONE region,
//     no holes, area > 50 (provenance OneCAD-CPP tests/prototypes/
//     proto_loop_detector.cpp:80-94, asserts area() > 50.0). pi*6*3 == 56.549 is
//     the analytic value; the loop polygon is a tessellation so it slightly
//     UNDER-estimates — 1% is comfortably inside the segment count
//     `computeCircleSegments` picks for r=6.
void test_ellipse_region_matches_the_corpus_oracle() {
    const loop::RegionTable table =
        table_from(sketch_of(json::array({ellipse_entity("el", 0, 0, 6, 3, 0.25)})));
    check(table.success, "an ellipse builds a region table: " + table.errorMessage);
    check(table.regions.size() == 1,
          "a standalone closed ellipse publishes exactly ONE region, got " +
              std::to_string(table.regions.size()));
    if (table.regions.size() != 1) {
        return;
    }
    check(table.regions[0].holes.empty(), "the ellipse cell has no holes");
    check(table.regions[0].outerWireEdges == std::vector<std::string>{"el"},
          "the cell's only boundary edge is the wire ellipse id");
    const double area = table.regions[0].outerLoop.area();
    const double analytic = std::numbers::pi_v<double> * 6.0 * 3.0;  // 56.549
    check(area > 50.0, "corpus oracle: area() > 50.0, got " + std::to_string(area));
    check(std::abs(area - analytic) / analytic < 0.01,
          "area is within 1% of pi*a*b (" + std::to_string(area) + " vs " +
              std::to_string(analytic) + ")");
}

// (d) `Sketch::addEllipse` NORMALIZES (swaps major/minor and rotates by pi/2 when
//     minor > major). `apply_solved_positions` MUST echo the normalized values,
//     otherwise every upsert stores parameters the worker does not actually hold
//     and each subsequent diff re-dirties the sketch.
void test_apply_solved_positions_echoes_normalized_params() {
    // Deliberately inverted: minorR (6) > majorR (3).
    json args = sketch_of(json::array({ellipse_entity("el", 1.0, 2.0, 3.0, 6.0, 0.0)}));
    onecad::wire::TranslateResult tr = onecad::wire::translate(args);
    check(tr.ok, "an inverted-radii ellipse translates: " + tr.error);
    if (!tr.ok) {
        return;
    }
    const sk::SolveResult solve = tr.sketch->solve();
    check(solve.success, "it solves");
    onecad::wire::apply_solved_positions(args, *tr.sketch, tr.index);

    const json& e = args["entities"][0];
    check(e.contains("majorR") && std::abs(e["majorR"].get<double>() - 6.0) < 1e-9,
          "echoed majorR is the NORMALIZED 6 (swapped), got " + e.value("majorR", json{}).dump());
    check(e.contains("minorR") && std::abs(e["minorR"].get<double>() - 3.0) < 1e-9,
          "echoed minorR is the NORMALIZED 3 (swapped), got " + e.value("minorR", json{}).dump());
    check(e.contains("rotation") &&
              std::abs(e["rotation"].get<double>() - std::numbers::pi_v<double> / 2.0) < 1e-9,
          "echoed rotation gained pi/2 with the swap, got " + e.value("rotation", json{}).dump());
    check(e.contains("center") && std::abs(e["center"][0].get<double>() - 1.0) < 1e-9 &&
              std::abs(e["center"][1].get<double>() - 2.0) < 1e-9,
          "the center is echoed from the solved point, like Circle");
}

// (e) Construction parity: a construction ellipse is still materialized (it carries
//     DOF) but bounds nothing, and its synthesized center Point inherits the flag.
void test_construction_ellipse_bounds_nothing() {
    onecad::wire::TranslateResult tr = onecad::wire::translate(
        sketch_of(json::array({ellipse_entity("el", 0, 0, 6, 3, 0.0, /*construction=*/true)})));
    check(tr.ok, "a construction ellipse translates: " + tr.error);
    if (!tr.ok) {
        return;
    }
    const sk::EntityID center = tr.index.resolve_point("el", "center");
    const auto* cp = tr.sketch->getEntityAs<sk::SketchPoint>(center);
    check(cp && cp->isConstruction(), "the synthesized center Point INHERITS construction");
    check(tr.sketch->getDegreesOfFreedom() == 5,
          "a construction ellipse still carries its 5 DOF");

    const loop::RegionTable table = table_from(
        sketch_of(json::array({ellipse_entity("el", 0, 0, 6, 3, 0.0, /*construction=*/true)})));
    check(table.success, "a construction-only sketch still builds a table");
    check(table.regions.empty(), "a construction ellipse publishes NO region, got " +
                                     std::to_string(table.regions.size()));
}

// (f) Region ids are derived from loop edge wire-ids only, so adding an ellipse
//     cell must not perturb an unrelated cell's id (RegionTable is type-agnostic).
void test_ellipse_does_not_perturb_unrelated_region_ids() {
    const auto rect = [](double x0, double y0, double w, double h) {
        json entities = json::array();
        const double x1 = x0 + w;
        const double y1 = y0 + h;
        const double corner[4][2] = {{x0, y0}, {x1, y0}, {x1, y1}, {x0, y1}};
        for (int i = 0; i < 4; ++i) {
            const double* a = corner[i];
            const double* b = corner[(i + 1) % 4];
            entities.push_back({{"id", "r" + std::to_string(i)},
                                {"type", "Line"},
                                {"p0", {a[0], a[1]}},
                                {"p1", {b[0], b[1]}}});
        }
        return entities;
    };

    const loop::RegionTable baseline = table_from(sketch_of(rect(40, 0, 20, 20)));
    check(baseline.success && baseline.regions.size() == 1, "the rectangle alone is one region");
    if (!baseline.success || baseline.regions.size() != 1) {
        return;
    }

    json mixed = rect(40, 0, 20, 20);
    mixed.push_back(ellipse_entity("el", 0, 0, 6, 3, 0.25));
    const loop::RegionTable table = table_from(sketch_of(mixed));
    check(table.success && table.regions.size() == 2,
          "rectangle + disjoint ellipse publishes TWO cells, got " +
              std::to_string(table.regions.size()));
    if (!table.success || table.regions.size() != 2) {
        return;
    }
    bool found = false;
    for (const auto& r : table.regions) {
        if (r.id == baseline.regions[0].id) {
            found = true;
        }
    }
    check(found, "the rectangle's region id is byte-identical with the ellipse present");
}
}  // namespace

int main() {
    test_ellipse_translates_and_owns_its_center_point();
    test_ellipse_dof_uses_the_naive_fallback();
    test_ellipse_region_matches_the_corpus_oracle();
    test_apply_solved_positions_echoes_normalized_params();
    test_construction_ellipse_bounds_nothing();
    test_ellipse_does_not_perturb_unrelated_region_ids();
    if (g_failures == 0) {
        std::fprintf(stderr, "sketch_ellipse: OK\n");
    }
    return g_failures;
}
