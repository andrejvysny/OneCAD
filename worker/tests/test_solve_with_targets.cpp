// SP-2 W1 — the generalized drag solver entry (`Sketch::solveWithTargets`) and
// the entity→owned-points map (`Sketch::entityPointIds`) the SCHEMA §7.4 gesture
// kinds are built on.
//
// The pre-SP-2 entries can express exactly one gesture: "one point to one
// position" (`solveWithDrag`) or "N points rigidly, or reject"
// (`solveWithGroupDrag`). Neither can express a `radius` drag (moves NO point),
// an `arcEnd` reshape (needs the sibling endpoint FREE, which
// `beginPointDrag`'s pin-everything strategy forbids), or an `entityBody`
// translate (needs NOTHING pinned so coupled geometry can follow). This is the
// superset: explicit point targets + explicit radius targets + an explicit pin
// set, with the targets ADVISORY (temporary tag(−1) drives, so a committed
// constraint always outranks them).
//
// The per-kind behaviour matrix (which pin set each kind uses, the degenerate
// guards) belongs to the lane and is covered by W2's test_sketch_drag_kinds.
#include <cmath>
#include <cstdio>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "nlohmann/json.hpp"
#include "sketch/Sketch.h"
#include "sketch/SketchArc.h"
#include "sketch/SketchCircle.h"
#include "sketch/SketchPoint.h"
#include "sketch/WireSketch.h"

using nlohmann::json;
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

json sketch_of(json entities, json constraints) {
    return {
        {"sketchId", "solve-with-targets"},
        {"plane", {{"kind", "XY"}}},
        {"entities", std::move(entities)},
        {"constraints", std::move(constraints)},
    };
}

json point_entity(unsigned int id, double x, double y) {
    return {{"id", uuid(id)}, {"type", "Point"}, {"at", {x, y}}};
}

json line_entity(unsigned int id, unsigned int p0, unsigned int p1) {
    return {{"id", uuid(id)}, {"type", "Line"}, {"p0Ref", uuid(p0)}, {"p1Ref", uuid(p1)}};
}

json circle_entity(unsigned int id, double cx, double cy, double r) {
    return {{"id", uuid(id)}, {"type", "Circle"}, {"center", {cx, cy}}, {"radius", r}};
}

json unary_constraint(unsigned int id, const char* type, unsigned int entity) {
    return {{"id", uuid(id)}, {"type", type}, {"entities", {uuid(entity)}}};
}

// Position of an internal point, by wire handle.
sk::Vec2d at(const onecad::wire::TranslateResult& tr, unsigned int wire_id,
             const std::string& role = {}) {
    const sk::EntityID internal = tr.index.resolve_point(uuid(wire_id), role);
    const auto* point = tr.sketch->getEntityAs<sk::SketchPoint>(internal);
    if (!point) {
        check(false, "point handle " + uuid(wire_id) + "." + role + " resolves");
        return {};
    }
    return sk::Vec2d{point->position().X(), point->position().Y()};
}

bool near(double a, double b, double tol = 1e-6) { return std::abs(a - b) < tol; }

bool near_point(const sk::Vec2d& p, double x, double y, double tol = 1e-6) {
    return near(p.x, x, tol) && near(p.y, y, tol);
}

using PointTargets = std::unordered_map<sk::EntityID, sk::Vec2d>;
using RadiusTargets = std::unordered_map<sk::EntityID, double>;
using PinSet = std::unordered_set<sk::EntityID>;

// (a) The no-constraint fast path for PURE point targets: with nothing to
//     propagate through, every target IS the answer, exactly.
void test_free_point_targets_land_exactly() {
    onecad::wire::TranslateResult tr = onecad::wire::translate(sketch_of(
        json::array({point_entity(100, 0, 0), point_entity(101, 10, 0), line_entity(200, 100, 101)}),
        json::array()));
    check(tr.ok, "free line translates: " + tr.error);
    if (!tr.ok) return;

    PointTargets targets;
    targets[tr.index.resolve_point(uuid(100), "")] = sk::Vec2d{1.0, 2.0};
    targets[tr.index.resolve_point(uuid(101), "")] = sk::Vec2d{11.0, 12.0};

    const sk::SolveResult r = tr.sketch->solveWithTargets(targets, {}, {});
    check(r.success, "free-line point targets succeed: " + r.errorMessage);
    check(near_point(at(tr, 100), 1.0, 2.0), "free point target 1 lands exactly");
    check(near_point(at(tr, 101), 11.0, 12.0), "free point target 2 lands exactly");
}

// (b) A radius target on a constraint-free circle takes the same fast path —
//     no solver round-trip, and the radius is set verbatim. `radius` is the
//     kind that moves NO point, so `positions` alone could never report it.
void test_free_circle_radius_target_is_exact() {
    onecad::wire::TranslateResult tr = onecad::wire::translate(
        sketch_of(json::array({circle_entity(300, 0, 0, 5.0)}), json::array()));
    check(tr.ok, "free circle translates: " + tr.error);
    if (!tr.ok) return;

    const sk::EntityID circle = tr.index.wire_to_internal.at(uuid(300));
    RadiusTargets radii;
    radii[circle] = 12.0;

    const sk::SolveResult r = tr.sketch->solveWithTargets({}, radii, {});
    check(r.success, "free-circle radius target succeeds: " + r.errorMessage);
    const auto* c = tr.sketch->getEntityAs<sk::SketchCircle>(circle);
    check(c && near(c->radius(), 12.0), "the free circle radius is set verbatim");
    check(near_point(at(tr, 300, "center"), 0.0, 0.0), "the free circle center did not move");
}

// (c) The same target through PlaneGCS. One unrelated user constraint is enough
//     to disqualify the fast path, so this is the real `addConstraintCircleRadius`
//     tag(−1) drive — and the pinned center proves a pin and a radius target
//     coexist in one step.
void test_constrained_circle_radius_target_solves() {
    onecad::wire::TranslateResult tr = onecad::wire::translate(sketch_of(
        json::array({circle_entity(300, 0, 0, 5.0), point_entity(100, 50, 0),
                     point_entity(101, 60, 0), line_entity(200, 100, 101)}),
        json::array({unary_constraint(400, "Horizontal", 200)})));
    check(tr.ok, "circle + constrained line translates: " + tr.error);
    if (!tr.ok) return;

    const sk::EntityID circle = tr.index.wire_to_internal.at(uuid(300));
    const std::vector<sk::EntityID> owned = tr.sketch->entityPointIds(circle);
    check(owned.size() == 1, "a Circle owns exactly its center point");
    if (owned.empty()) return;

    RadiusTargets radii;
    radii[circle] = 20.0;
    const sk::SolveResult r = tr.sketch->solveWithTargets({}, radii, PinSet{owned[0]});
    check(r.success, "solver-path radius target succeeds: " + r.errorMessage);

    const auto* c = tr.sketch->getEntityAs<sk::SketchCircle>(circle);
    check(c && near(c->radius(), 20.0, 1e-4),
          "PlaneGCS drove the radius to the target (got " +
              std::to_string(c ? c->radius() : 0.0) + ")");
    check(near_point(at(tr, 300, "center"), 0.0, 0.0, 1e-6), "the pinned center held");
}

// (d) Pins hold, targets are reached, and the coupling still propagates —
//     the three things a per-kind pin set exists to buy. A constrained rect,
//     left edge pinned, top-right corner dragged: the right edge's Vertical
//     carries the bottom-right corner along, and neither pinned corner moves.
void test_pinned_points_hold_while_the_coupling_propagates() {
    json entities = json::array({point_entity(100, 0, 0), point_entity(101, 10, 0),
                                 point_entity(102, 10, 10), point_entity(103, 0, 10),
                                 line_entity(200, 100, 101), line_entity(201, 101, 102),
                                 line_entity(202, 102, 103), line_entity(203, 103, 100)});
    json constraints = json::array({unary_constraint(300, "Horizontal", 200),
                                    unary_constraint(301, "Horizontal", 202),
                                    unary_constraint(302, "Vertical", 201),
                                    unary_constraint(303, "Vertical", 203)});
    onecad::wire::TranslateResult tr = onecad::wire::translate(sketch_of(entities, constraints));
    check(tr.ok, "constrained rect translates: " + tr.error);
    if (!tr.ok) return;
    tr.sketch->solve();

    PointTargets targets;
    targets[tr.index.resolve_point(uuid(102), "")] = sk::Vec2d{25.0, 10.0};
    const PinSet pins{tr.index.resolve_point(uuid(100), ""),
                      tr.index.resolve_point(uuid(103), "")};

    const sk::SolveResult r = tr.sketch->solveWithTargets(targets, {}, pins);
    check(r.success, "the pinned-rect drag succeeds: " + r.errorMessage);

    check(near_point(at(tr, 100), 0.0, 0.0, 1e-6), "pinned bottom-left held");
    check(near_point(at(tr, 103), 0.0, 10.0, 1e-6), "pinned top-left held");
    check(near_point(at(tr, 102), 25.0, 10.0, 1e-4), "the dragged corner reached its target");
    const sk::Vec2d br = at(tr, 101);
    std::fprintf(stderr, "  rect: bottom-right -> (%.4f, %.4f)\n", br.x, br.y);
    check(near(br.x, 25.0, 1e-4), "Vertical carried the bottom-right corner to x = 25");
    check(near(br.y, 0.0, 1e-4), "Horizontal kept the bottom-right corner on the pinned edge");
}

// (e) Points AND radii in one solve, both deltas past the 100 mm substep
//     threshold, so the interpolation has to cover BOTH channels: a radius that
//     jumped its whole delta on step 1 while the points crept would hand
//     PlaneGCS a discontinuity it cannot follow.
void test_mixed_point_and_radius_targets_substep_together() {
    onecad::wire::TranslateResult tr = onecad::wire::translate(
        sketch_of(json::array({point_entity(100, 0, 0), point_entity(101, 0, 50),
                               line_entity(200, 100, 101), circle_entity(300, 100, 0, 5.0)}),
                  json::array({unary_constraint(400, "Vertical", 200)})));
    check(tr.ok, "mixed-target sketch translates: " + tr.error);
    if (!tr.ok) return;
    tr.sketch->solve();

    const sk::EntityID circle = tr.index.wire_to_internal.at(uuid(300));
    PointTargets targets;
    targets[tr.index.resolve_point(uuid(101), "")] = sk::Vec2d{0.0, 200.0};  // Δ 150 mm
    RadiusTargets radii;
    radii[circle] = 130.0;  // Δ 125 mm
    const PinSet pins{tr.index.resolve_point(uuid(100), "")};

    const sk::SolveResult r = tr.sketch->solveWithTargets(targets, radii, pins);
    check(r.success, "the mixed substepped drag succeeds: " + r.errorMessage);
    check(near_point(at(tr, 100), 0.0, 0.0, 1e-6), "the pinned line start held");
    check(near_point(at(tr, 101), 0.0, 200.0, 1e-4), "the point target survived substepping");
    const auto* c = tr.sketch->getEntityAs<sk::SketchCircle>(circle);
    check(c && near(c->radius(), 130.0, 1e-4),
          "the radius target survived substepping (got " +
              std::to_string(c ? c->radius() : 0.0) + ")");
}

// (f) A reference-locked entity is immovable by contract (SCHEMA §7.3), so
//     driving one is refused BEFORE the solver is touched — a silent no-op the
//     caller reads as a successful drag is the failure mode to avoid.
void test_reference_locked_targets_are_refused() {
    json entities = json::array({circle_entity(300, 0, 0, 5.0), point_entity(100, 20, 0)});
    entities[0]["referenceLocked"] = true;
    entities[1]["referenceLocked"] = true;
    onecad::wire::TranslateResult tr = onecad::wire::translate(sketch_of(entities, json::array()));
    check(tr.ok, "locked sketch translates: " + tr.error);
    if (!tr.ok) return;

    const sk::EntityID circle = tr.index.wire_to_internal.at(uuid(300));
    RadiusTargets radii;
    radii[circle] = 20.0;
    const sk::SolveResult radiusResult = tr.sketch->solveWithTargets({}, radii, {});
    check(!radiusResult.success, "a locked circle refuses a radius target");
    check(radiusResult.errorMessage == "Entity is locked",
          "the locked radius refusal says so: " + radiusResult.errorMessage);
    const auto* c = tr.sketch->getEntityAs<sk::SketchCircle>(circle);
    check(c && near(c->radius(), 5.0), "the refused radius drag changed nothing");

    PointTargets targets;
    targets[tr.index.resolve_point(uuid(100), "")] = sk::Vec2d{30.0, 30.0};
    const sk::SolveResult pointResult = tr.sketch->solveWithTargets(targets, {}, {});
    check(!pointResult.success, "a locked point refuses a point target");
    check(pointResult.errorMessage == "Entity is locked",
          "the locked point refusal says so: " + pointResult.errorMessage);
    check(near_point(at(tr, 100), 20.0, 0.0), "the refused point drag changed nothing");
}

// (g) `entityPointIds` in handle order. The order is contractual: SCHEMA §7.4
//     anchors a grab-less `entityBody` gesture on the FIRST id.
void test_entity_point_ids_are_in_handle_order() {
    json entities = json::array({
        point_entity(100, 0, 0),
        point_entity(101, 10, 0),
        line_entity(200, 100, 101),
        circle_entity(300, 40, 0, 5.0),
        {{"id", uuid(400)}, {"type", "Arc"}, {"center", {0.0, 40.0}},
         {"radius", 6.0}, {"startAngle", 0.0}, {"endAngle", M_PI / 2}},
        {{"id", uuid(500)}, {"type", "Ellipse"}, {"center", {80.0, 0.0}},
         {"majorR", 10.0}, {"minorR", 4.0}, {"rotation", 0.0}},
    });
    onecad::wire::TranslateResult tr = onecad::wire::translate(sketch_of(entities, json::array()));
    check(tr.ok, "all-entity sketch translates: " + tr.error);
    if (!tr.ok) return;

    const auto owned = [&](unsigned int wire_id) {
        return tr.sketch->entityPointIds(tr.index.wire_to_internal.at(uuid(wire_id)));
    };

    const std::vector<sk::EntityID> line = owned(200);
    check(line.size() == 2, "a Line owns 2 points");
    if (line.size() == 2) {
        check(line[0] == tr.index.resolve_point(uuid(200), "p0"), "Line[0] is p0");
        check(line[1] == tr.index.resolve_point(uuid(200), "p1"), "Line[1] is p1");
    }

    const std::vector<sk::EntityID> circle = owned(300);
    check(circle.size() == 1 && circle[0] == tr.index.resolve_point(uuid(300), "center"),
          "a Circle owns its center only");

    const std::vector<sk::EntityID> arc = owned(400);
    check(arc.size() == 3, "an endpoint-bearing Arc owns 3 points (got " +
                               std::to_string(arc.size()) + ")");
    if (arc.size() == 3) {
        check(arc[0] == tr.index.resolve_point(uuid(400), "center"), "Arc[0] is the center");
        check(arc[1] == tr.index.resolve_point(uuid(400), "start"), "Arc[1] is the start");
        check(arc[2] == tr.index.resolve_point(uuid(400), "end"), "Arc[2] is the end");
    }

    const std::vector<sk::EntityID> ellipse = owned(500);
    check(ellipse.size() == 1 && ellipse[0] == tr.index.resolve_point(uuid(500), "center"),
          "an Ellipse owns its center only (it is not solver-registered)");

    const sk::EntityID standalone = tr.index.resolve_point(uuid(100), "");
    const std::vector<sk::EntityID> point = tr.sketch->entityPointIds(standalone);
    check(point.size() == 1 && point[0] == standalone, "a Point owns itself");

    check(tr.sketch->entityPointIds("no-such-entity").empty(),
          "an unknown entity owns no points");
}

// (h) An empty request is a caller bug, not a no-op success.
void test_empty_targets_are_rejected() {
    onecad::wire::TranslateResult tr = onecad::wire::translate(
        sketch_of(json::array({circle_entity(300, 0, 0, 5.0)}), json::array()));
    check(tr.ok, "empty-target sketch translates: " + tr.error);
    if (!tr.ok) return;
    const sk::SolveResult r = tr.sketch->solveWithTargets({}, {}, {});
    check(!r.success && r.errorMessage == "No drag targets",
          "a target-less solve is refused: " + r.errorMessage);
}

}  // namespace

int main() {
    test_free_point_targets_land_exactly();
    test_free_circle_radius_target_is_exact();
    test_constrained_circle_radius_target_solves();
    test_pinned_points_hold_while_the_coupling_propagates();
    test_mixed_point_and_radius_targets_substep_together();
    test_reference_locked_targets_are_refused();
    test_entity_point_ids_are_in_handle_order();
    test_empty_targets_are_rejected();

    if (g_failures > 0) {
        std::fprintf(stderr, "%d check(s) failed\n", g_failures);
        return 1;
    }
    std::fprintf(stderr, "all solveWithTargets checks passed\n");
    return 0;
}
