// W0b — `referenceLocked` end-to-end in the worker (SCHEMA §7.3).
//
// The flag marks geometry PROJECTED from the sketch's host face. It is the
// deliberate opposite of `construction` on the two axes that matter:
//
//   | flag             | bounds regions | editable | solved |
//   | construction     | NO             | yes      | yes    |
//   | referenceLocked  | YES            | no       | pinned |
//
// The oracle port already carried the edit guards but nothing ever SET the flag,
// so all of it was dead code and a constraint touching locked geometry was
// vetoed outright — which made the whole point of a projected boundary (snapping
// a profile to it) unreachable. `WireSketch` now reads the flag, the veto is
// gone, and `Sketch::referenceLockPins` freezes the locked entity's parameters
// in PlaneGCS under the INTERNAL tag 0. These are the pins for that chain.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "nlohmann/json.hpp"
#include "sketch/Sketch.h"
#include "sketch/SketchArc.h"
#include "sketch/SketchCircle.h"
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

json sketch_of(json entities, json constraints = json::array()) {
    return {
        {"sketchId", "reference-locked"},
        {"plane", {{"kind", "XY"}}},
        {"entities", std::move(entities)},
        {"constraints", std::move(constraints)},
    };
}

// Four closed Line entities with INLINE p0/p1 — the synthesized endpoint Points
// must inherit the parent's flags.
void push_rect(json& entities, unsigned int base, double x0, double y0, double w, double h,
               const char* flag) {
    const double x1 = x0 + w;
    const double y1 = y0 + h;
    const double corner[4][2] = {{x0, y0}, {x1, y0}, {x1, y1}, {x0, y1}};
    for (unsigned int i = 0; i < 4; ++i) {
        const double* a = corner[i];
        const double* b = corner[(i + 1) % 4];
        json e{{"id", uuid(base + i)},
               {"type", "Line"},
               {"p0", {a[0], a[1]}},
               {"p1", {b[0], b[1]}}};
        if (flag) e[flag] = true;
        entities.push_back(std::move(e));
    }
}

// Same pipeline `SketchRegions` and plan-time profile derivation run.
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

// (a) The flag survives the wire — on the entity AND on its synthesized children.
void test_wire_carries_the_flag_and_children_inherit_it() {
    json entities = json::array({
        {{"id", uuid(10)}, {"type", "Circle"}, {"center", {0.0, 0.0}}, {"radius", 5.0},
         {"referenceLocked", true}},
        {{"id", uuid(11)}, {"type", "Line"}, {"p0", {20.0, 0.0}}, {"p1", {30.0, 0.0}}},
        {{"id", uuid(12)}, {"type", "Arc"}, {"center", {0.0, 40.0}}, {"radius", 6.0},
         {"startAngle", 0.0}, {"endAngle", M_PI / 2}, {"referenceLocked", true}},
    });
    onecad::wire::TranslateResult t = onecad::wire::translate(sketch_of(entities));
    check(t.ok, "mixed locked/free sketch translates: " + t.error);
    if (!t.ok) {
        return;
    }

    const auto locked = [&](const sk::EntityID& id) {
        return !id.empty() && t.sketch->isEntityReferenceLocked(id);
    };
    check(locked(t.index.wire_to_internal[uuid(10)]), "the locked circle is locked");
    check(locked(t.index.resolve_point(uuid(10), "center")),
          "the circle's synthesized center INHERITS the lock");
    check(locked(t.index.wire_to_internal[uuid(12)]), "the locked arc is locked");
    check(locked(t.index.resolve_point(uuid(12), "center")),
          "the arc's synthesized center inherits the lock");
    check(locked(t.index.resolve_point(uuid(12), "start")) &&
              locked(t.index.resolve_point(uuid(12), "end")),
          "the arc's minted `.start`/`.end` endpoint points inherit the lock");

    // …and an absent flag still means false (the default every sketch relies on).
    check(!locked(t.index.wire_to_internal[uuid(11)]), "the free line is NOT locked");
    check(!locked(t.index.resolve_point(uuid(11), "p0")),
          "the free line's endpoints are NOT locked");
}

// (b) A referenced (not synthesized) Point keeps its OWN flag: a free parent
//     must never clear a locked point back to false.
void test_referenced_points_keep_their_own_flag() {
    json entities = json::array({
        {{"id", uuid(20)}, {"type", "Point"}, {"at", {0.0, 0.0}}, {"referenceLocked", true}},
        {{"id", uuid(21)}, {"type", "Point"}, {"at", {10.0, 0.0}}},
        // A FREE line over one locked and one free endpoint.
        {{"id", uuid(22)}, {"type", "Line"}, {"p0Ref", uuid(20)}, {"p1Ref", uuid(21)}},
    });
    onecad::wire::TranslateResult t = onecad::wire::translate(sketch_of(entities));
    check(t.ok, "shared-point sketch translates: " + t.error);
    if (!t.ok) {
        return;
    }
    check(t.sketch->isEntityReferenceLocked(t.index.wire_to_internal[uuid(20)]),
          "the locked point stays locked under a free line");
    check(!t.sketch->isEntityReferenceLocked(t.index.wire_to_internal[uuid(21)]),
          "the free point stays free");
    check(!t.sketch->isEntityReferenceLocked(t.index.wire_to_internal[uuid(22)]),
          "the line itself stays free");
}

// (c) THE contrast with `construction`: a locked rectangle DOES bound a region.
//     Red-first control: the same rectangle drawn as construction bounds nothing.
void test_locked_geometry_still_bounds_a_region() {
    json construction = json::array();
    push_rect(construction, 30, 0, 0, 20, 20, "construction");
    const loop::RegionTable none = table_from(sketch_of(construction));
    check(none.success && none.regions.empty(),
          "control: a construction rectangle publishes NO region (got " +
              std::to_string(none.regions.size()) + ")");

    json baseline = json::array();
    push_rect(baseline, 40, 0, 0, 20, 20, nullptr);
    const loop::RegionTable plain = table_from(sketch_of(baseline));
    check(plain.success && plain.regions.size() == 1,
          "control: a plain rectangle publishes exactly one region");

    json locked = json::array();
    push_rect(locked, 40, 0, 0, 20, 20, "referenceLocked");
    const loop::RegionTable table = table_from(sketch_of(locked));
    check(table.success && table.regions.size() == 1,
          "a reference-locked rectangle publishes ONE region (got " +
              std::to_string(table.regions.size()) + ") — locked geometry is not construction");
    if (table.regions.size() != 1 || plain.regions.size() != 1) {
        return;
    }
    check(table.regions[0].id == plain.regions[0].id,
          "…and its region id is byte-identical to the unlocked rectangle's (" +
              table.regions[0].id + " vs " + plain.regions[0].id + ")");
}

// (d) A constraint MAY reference locked geometry, and the solve moves only the
//     FREE side: a Distance from a free point to a locked circle's center.
void test_constraint_against_a_locked_circle_moves_only_the_free_point() {
    json entities = json::array({
        {{"id", uuid(50)}, {"type", "Circle"}, {"center", {0.0, 0.0}}, {"radius", 5.0},
         {"referenceLocked", true}},
        {{"id", uuid(51)}, {"type", "Point"}, {"at", {10.0, 0.0}}},
    });
    json constraints = json::array({{
        {"id", uuid(52)},
        {"type", "Distance"},
        {"entities", {uuid(51), uuid(50)}},
        {"positions", {"", "Center"}},
        {"value", 25.0},
    }});
    onecad::wire::TranslateResult t =
        onecad::wire::translate(sketch_of(entities, constraints));
    check(t.ok, "a constraint against locked geometry is ACCEPTED: " + t.error);
    if (!t.ok) {
        return;
    }

    const sk::EntityID centerId = t.index.resolve_point(uuid(50), "center");
    const sk::EntityID freeId = t.index.wire_to_internal[uuid(51)];
    const auto* circle = t.sketch->getEntityAs<sk::SketchCircle>(t.index.wire_to_internal[uuid(50)]);
    check(circle != nullptr, "the locked circle exists");
    if (!circle) {
        return;
    }

    const sk::SolveResult solve = t.sketch->solve();
    check(solve.success, "the sketch solves with a pinned locked circle: " + solve.errorMessage);
    check(!t.sketch->hasRedundantConstraints(),
          "the tag-0 pins are NOT diagnosed as redundant constraints");

    const auto* center = t.sketch->getEntityAs<sk::SketchPoint>(centerId);
    const auto* freePoint = t.sketch->getEntityAs<sk::SketchPoint>(freeId);
    check(center && freePoint, "both points survive the solve");
    if (!center || !freePoint) {
        return;
    }
    check(std::abs(center->position().X()) < 1e-9 && std::abs(center->position().Y()) < 1e-9,
          "the LOCKED circle's center did not move");
    check(std::abs(circle->radius() - 5.0) < 1e-9, "the LOCKED circle's radius did not move");
    check(std::abs(center->position().Distance(freePoint->position()) - 25.0) < 1e-6,
          "the FREE point moved to satisfy the dimension (d=" +
              std::to_string(center->position().Distance(freePoint->position())) + ")");
}

// (e) The arc case: a Coincident welding a free point to a LOCKED arc's `.start`
//     handle drags the free point, never the arc. Angles + radius + center are
//     pinned, so the arc-rules coupling holds the endpoints in place too.
void test_constraint_on_a_locked_arc_endpoint_leaves_the_arc_still() {
    constexpr double kR = 8.0;
    json entities = json::array({
        {{"id", uuid(60)}, {"type", "Arc"}, {"center", {0.0, 0.0}}, {"radius", kR},
         {"startAngle", 0.0}, {"endAngle", M_PI}, {"referenceLocked", true}},
        {{"id", uuid(61)}, {"type", "Point"}, {"at", {30.0, 30.0}}},
    });
    json constraints = json::array({{
        {"id", uuid(62)},
        {"type", "Coincident"},
        {"entities", {uuid(61), uuid(60)}},
        {"positions", {"", "Start"}},
    }});
    onecad::wire::TranslateResult t =
        onecad::wire::translate(sketch_of(entities, constraints));
    check(t.ok, "a Coincident onto a locked arc endpoint is ACCEPTED: " + t.error);
    if (!t.ok) {
        return;
    }

    const auto* arc = t.sketch->getEntityAs<sk::SketchArc>(t.index.wire_to_internal[uuid(60)]);
    check(arc != nullptr, "the locked arc exists");
    if (!arc) {
        return;
    }
    // A HALF-circle on purpose: pinning `.start`/`.end` positions instead of the
    // arc's own angles would make exactly this configuration singular.
    const sk::SolveResult solve = t.sketch->solve();
    check(solve.success, "the sketch solves: " + solve.errorMessage);
    check(!t.sketch->hasRedundantConstraints(),
          "a locked 180-degree arc is NOT diagnosed as redundant");

    const auto* center = t.sketch->getEntityAs<sk::SketchPoint>(arc->centerPointId());
    const auto* start = t.sketch->getEntityAs<sk::SketchPoint>(arc->startPointId());
    const auto* end = t.sketch->getEntityAs<sk::SketchPoint>(arc->endPointId());
    const auto* freePoint =
        t.sketch->getEntityAs<sk::SketchPoint>(t.index.wire_to_internal[uuid(61)]);
    check(center && start && end && freePoint, "arc points survive the solve");
    if (!center || !start || !end || !freePoint) {
        return;
    }
    check(std::abs(center->position().X()) < 1e-9 && std::abs(center->position().Y()) < 1e-9,
          "the locked arc's center did not move");
    check(std::abs(arc->radius() - kR) < 1e-9, "the locked arc's radius did not move");
    check(std::abs(start->position().X() - kR) < 1e-6 &&
              std::abs(start->position().Y()) < 1e-6,
          "the locked arc's `.start` endpoint did not move");
    check(std::abs(end->position().X() + kR) < 1e-6 && std::abs(end->position().Y()) < 1e-6,
          "the locked arc's `.end` endpoint did not move");
    check(freePoint->position().Distance(start->position()) < 1e-6,
          "the FREE point moved onto the arc's start handle");
}

// (f) The edit guards, now reachable: removal, split and drag are refused, and
//     a translate REFUSES outright instead of silently shearing the sketch.
void test_edit_guards_refuse_locked_geometry() {
    sk::Sketch sketch;
    const sk::EntityID a = sketch.addPoint(0.0, 0.0);
    const sk::EntityID b = sketch.addPoint(40.0, 0.0);
    const sk::EntityID line = sketch.addLine(a, b);
    const sk::EntityID freePoint = sketch.addPoint(10.0, 10.0);
    check(sketch.setEntityReferenceLocked(line, true), "the line locks");
    check(sketch.setEntityReferenceLocked(a, true) && sketch.setEntityReferenceLocked(b, true),
          "its endpoints lock");

    check(!sketch.removeEntity(line), "removing a locked line is refused");
    check(!sketch.removeEntity(a), "removing a point a locked line depends on is refused");
    check(sketch.splitLineAt(line, sk::Vec2d{20.0, 0.0}).first.empty(),
          "splitting a locked line is refused");
    check(!sketch.solveWithDrag(a, sk::Vec2d{5.0, 5.0}).success,
          "dragging a locked point is refused");

    const gp_Pnt2d before = sketch.getEntityAs<sk::SketchPoint>(freePoint)->position();
    check(!sketch.translateSketch(1.0, 2.0),
          "translating a sketch containing locked geometry is REFUSED (not silently partial)");
    const gp_Pnt2d after = sketch.getEntityAs<sk::SketchPoint>(freePoint)->position();
    check(before.Distance(after) < 1e-12,
          "…and the refusal moved NOTHING, not even the free point");

    // The free point is still fully editable.
    check(sketch.removeEntity(freePoint), "a free point is still removable");
}

// (g) A `Fixed` on a locked point is NOT double-pinned — the pin set defers to
//     it, so PlaneGCS never sees two copies of the same equation.
void test_a_fixed_constraint_suppresses_the_duplicate_pin() {
    sk::Sketch sketch;
    const sk::EntityID center = sketch.addPoint(0.0, 0.0);
    const sk::EntityID circle = sketch.addCircle(center, 5.0);
    check(sketch.setEntityReferenceLocked(circle, true), "the circle locks");
    check(sketch.setEntityReferenceLocked(center, true), "its center locks");

    check(sketch.referenceLockPins().points.size() == 1,
          "without a Fixed, the locked circle pins its center");
    check(!sketch.addFixed(center).empty(), "a Fixed on locked geometry is accepted");
    check(sketch.referenceLockPins().points.empty(),
          "with a Fixed present, the center is no longer pinned twice");
    check(sketch.referenceLockPins().radii.size() == 1,
          "the radius pin remains — no Fixed constraint can reach it");

    const sk::SolveResult solve = sketch.solve();
    check(solve.success, "the doubly-held sketch solves: " + solve.errorMessage);
    check(!sketch.hasRedundantConstraints(),
          "Fixed + pins together are NOT redundant");
    check(sketch.getDegreesOfFreedom() == 0,
          "a fully locked circle has zero DOF (got " +
              std::to_string(sketch.getDegreesOfFreedom()) + ")");
}
}  // namespace

int main() {
    test_wire_carries_the_flag_and_children_inherit_it();
    test_referenced_points_keep_their_own_flag();
    test_locked_geometry_still_bounds_a_region();
    test_constraint_against_a_locked_circle_moves_only_the_free_point();
    test_constraint_on_a_locked_arc_endpoint_leaves_the_arc_still();
    test_edit_guards_refuse_locked_geometry();
    test_a_fixed_constraint_suppresses_the_duplicate_pin();
    if (g_failures == 0) {
        std::fprintf(stderr, "sketch_reference_locked: OK\n");
    }
    return g_failures;
}
