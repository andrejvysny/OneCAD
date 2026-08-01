// W0b — arc endpoints are REAL solver points.
//
// Before this work an `Arc`'s start/end were derived from center + radius +
// angles and existed nowhere in the solver, so:
//   * a `Coincident` addressing `<arc>.start` / `<arc>.end` failed wire
//     translation with "Coincident: unresolved point handle", and
//   * SCHEMA §7.3's positions example + §7.4's `"pointId": "e3.start"` SolveDrag
//     example described something no implementation could do.
// `WireSketch` now mints a `SketchPoint` per arc endpoint, registers the
// `<id>.start` / `<id>.end` handles, and `ConstraintSolver::addArc` couples them
// to center+radius+angle with PlaneGCS `addConstraintArcRules` under the
// INTERNAL tag 0 (never in `gcsTagToConstraint_`, so it can never be blamed as a
// user constraint). These are the pins.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "nlohmann/json.hpp"
#include "sketch/Sketch.h"
#include "sketch/SketchArc.h"
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
        {"sketchId", "arc-endpoints"},
        {"plane", {{"kind", "XY"}}},
        {"entities", std::move(entities)},
        {"constraints", std::move(constraints)},
    };
}

// ── The slot, worker-authored ────────────────────────────────────────────────
//
// Centerline p0=(-50,0) → p1=(50,0), radius 20. Mirrors `slotDrafts`
// (src/tools/sketch/toolMachine.ts) exactly: walls [+n] and [−n] as Line
// entities over shared `Point` entities, caps as ANGLE-form arcs (the shape the
// frontend marshaller emits), then the 4 wall↔cap welds + 4 Tangent + 1 Equal.
//
//   ids 100..103  wall endpoints a0 a1 b0 b1     (a = +n side, b = −n side)
//   ids 200,201   walls   (wall0 = a0→a1, wall1 = b0→b1)
//   ids 300,301   caps    (cap@p1 = b1→a1, cap@p0 = a0→b0)
constexpr double kR = 20.0;
constexpr double kX = 50.0;

json slot_entities() {
    json entities = json::array();
    const double pts[4][2] = {{-kX, kR}, {kX, kR}, {-kX, -kR}, {kX, -kR}};  // a0 a1 b0 b1
    for (unsigned int i = 0; i < 4; ++i) {
        entities.push_back({{"id", uuid(100 + i)}, {"type", "Point"}, {"at", {pts[i][0], pts[i][1]}}});
    }
    entities.push_back({{"id", uuid(200)}, {"type", "Line"},
                        {"p0Ref", uuid(100)}, {"p1Ref", uuid(101)}});  // wall0: a0→a1
    entities.push_back({{"id", uuid(201)}, {"type", "Line"},
                        {"p0Ref", uuid(102)}, {"p1Ref", uuid(103)}});  // wall1: b0→b1
    // cap@p1: start b1 (−pi/2) sweeping CCW to end a1 (+pi/2).
    entities.push_back({{"id", uuid(300)}, {"type", "Arc"}, {"center", {kX, 0.0}},
                        {"radius", kR}, {"startAngle", -M_PI / 2}, {"endAngle", M_PI / 2}});
    // cap@p0: start a0 (+pi/2) sweeping CCW to end b0 (−pi/2 ≡ 3pi/2).
    entities.push_back({{"id", uuid(301)}, {"type", "Arc"}, {"center", {-kX, 0.0}},
                        {"radius", kR}, {"startAngle", M_PI / 2}, {"endAngle", -M_PI / 2}});
    return entities;
}

json tangent(unsigned int id, unsigned int a, unsigned int b) {
    return {{"id", uuid(id)}, {"type", "Tangent"}, {"entities", {uuid(a), uuid(b)}}};
}

// The 4 wall↔cap welds, positions carried per slot (SCHEMA §7.3 `positions`).
json slot_welds() {
    json out = json::array();
    const struct {
        unsigned int id, point, arc;
        const char *point_pos, *arc_pos;
    } welds[4] = {
        {400, 101, 300, "", "End"},    // wall0.End (a1) ↔ cap@p1.End
        {401, 103, 300, "", "Start"},  // wall1.End (b1) ↔ cap@p1.Start
        {402, 100, 301, "", "Start"},  // wall0.Start (a0) ↔ cap@p0.Start
        {403, 102, 301, "", "End"},    // wall1.Start (b0) ↔ cap@p0.End
    };
    for (const auto& w : welds) {
        out.push_back({{"id", uuid(w.id)},
                       {"type", "Coincident"},
                       {"entities", {uuid(w.point), uuid(w.arc)}},
                       {"positions", {w.point_pos, w.arc_pos}}});
    }
    return out;
}

json equal_caps() {
    return json::array({
        {{"id", uuid(504)}, {"type", "Equal"}, {"entities", {uuid(300), uuid(301)}}},
    });
}

json concat(std::initializer_list<json> parts) {
    json out = json::array();
    for (const json& part : parts) {
        for (const json& c : part) out.push_back(c);
    }
    return out;
}

// The set `slotTool` authors: 4 wall↔cap welds + Equal. (It used to author
// 4 entity-level Tangents instead — see
// test_endpoint_weld_makes_entity_tangent_redundant for why those had to go.)
json slot_constraints(bool with_welds) {
    return with_welds ? concat({slot_welds(), equal_caps()}) : equal_caps();
}

// (a) An arc-endpoint Coincident TRANSLATES (it used to fail "unresolved point
//     handle") and the welded slot solves clean. DOF arithmetic: 26 params
//     (8 wall endpoints + 4 minted centers + 6 arc scalars + 8 minted endpoints)
//     − 17 equations (8 ArcRules + 8 Coincident + 1 Equal) = 9. The remaining 9
//     are c0(2) c1(2) r(1) and the 4 cap angles — the welds pin the walls TO the
//     caps, they do not make the walls parallel.
void test_welded_slot_solves_clean() {
    onecad::wire::TranslateResult tr =
        onecad::wire::translate(sketch_of(slot_entities(), slot_constraints(true)));
    check(tr.ok, "arc-endpoint Coincident translates: " + tr.error);
    if (!tr.ok) return;

    const sk::SolveResult solved = tr.sketch->solve();
    check(solved.success, "the welded slot solves");
    const int dof = tr.sketch->getDegreesOfFreedom();
    check(dof == 9, "welded slot DOF is 9 (got " + std::to_string(dof) + ")");
    check(tr.sketch->getConflictingConstraints().empty(),
          "welded slot reports NO conflicting constraints");
    // The whole point of tag 0: the four ArcRules pairs are structurally
    // required, never blamed. If they leaked into the diagnosis the sketch would
    // read as carrying redundant constraints, `upsert_state` would call it
    // OverConstrained, and every drag would report "redundant" instead of
    // "success" (SCHEMA §7.4 status precedence).
    check(!tr.sketch->hasRedundantConstraints(),
          "ArcRules are INTERNAL (tag 0) — the welded slot is redundancy-free");
    check(!tr.sketch->isOverConstrained(), "the welded slot is not over-constrained");
    // The naive path (what an ellipse-bearing sketch would fall back to) must
    // agree, which is what the −4-per-endpoint-bearing-arc adjustment buys.
    const int naive = tr.sketch->naiveDegreesOfFreedom();
    check(naive == 9, "naive DOF agrees with the diagnosis (got " + std::to_string(naive) + ")");
}

// (b) The welds are what removes the DOF: exactly −8 (2 per Coincident).
void test_welds_remove_exactly_eight_dof() {
    onecad::wire::TranslateResult bare =
        onecad::wire::translate(sketch_of(slot_entities(), slot_constraints(false)));
    onecad::wire::TranslateResult welded =
        onecad::wire::translate(sketch_of(slot_entities(), slot_constraints(true)));
    check(bare.ok && welded.ok, "both slot variants translate");
    if (!bare.ok || !welded.ok) return;
    bare.sketch->solve();
    welded.sketch->solve();
    const int before = bare.sketch->getDegreesOfFreedom();
    const int after = welded.sketch->getDegreesOfFreedom();
    std::fprintf(stderr, "  slot dof: %d (no welds) -> %d (4 welds)\n", before, after);
    check(before - after == 8,
          "the 4 arc-endpoint Coincidents remove exactly 8 DOF (got " +
              std::to_string(before - after) + ")");
}

// (b2) WHY the slot tool dropped its 4 entity-level Tangents when it gained the
//      welds — a load-bearing negative result, pinned so it cannot regress
//      silently.
//
//      An entity-level Tangent(line, arc) is `distance(arcCenter, line) == rad`.
//      Once the wall's endpoint is WELDED to the arc's endpoint the wall already
//      passes through a point at distance `rad` from the center, so that distance
//      is at its MAXIMUM — the constraint sits on the boundary `dist ≤ rad` and
//      its gradient vanishes on the manifold of the other constraints. It is
//      first-order dependent: PlaneGCS diagnoses all four as redundant, DOF does
//      not drop, and `upsert_state` would render every slot "OverConstrained".
//      (Same reason FreeCAD models endpoint tangency as its own constraint.)
void test_endpoint_weld_makes_entity_tangent_redundant() {
    json tangents = json::array({tangent(500, 200, 300), tangent(501, 200, 301),
                                 tangent(502, 201, 300), tangent(503, 201, 301)});
    onecad::wire::TranslateResult welded_only =
        onecad::wire::translate(sketch_of(slot_entities(), slot_constraints(true)));
    onecad::wire::TranslateResult with_tangents = onecad::wire::translate(
        sketch_of(slot_entities(), concat({slot_welds(), tangents, equal_caps()})));
    check(welded_only.ok && with_tangents.ok, "both tangency variants translate");
    if (!welded_only.ok || !with_tangents.ok) return;
    welded_only.sketch->solve();
    with_tangents.sketch->solve();
    check(!welded_only.sketch->hasRedundantConstraints(), "welds + Equal: redundancy-free");
    check(with_tangents.sketch->hasRedundantConstraints(),
          "welds + entity-level Tangents: PlaneGCS reports the Tangents redundant");
    check(with_tangents.sketch->getDegreesOfFreedom() ==
              welded_only.sketch->getDegreesOfFreedom(),
          "the 4 Tangents remove NO further DOF once the endpoints are welded");
}

// One arc at the origin, radius kR, sweeping 0 → pi/2.
json bare_arc(unsigned int id) {
    return json::array({{{"id", uuid(id)},
                         {"type", "Arc"},
                         {"center", {0.0, 0.0}},
                         {"radius", kR},
                         {"startAngle", 0.0},
                         {"endAngle", M_PI / 2}}});
}

// Drag `<arc>.start` toward `target`; returns the solved endpoint + center.
struct DragOutcome {
    bool ok = false;
    double x = 0, y = 0, cx = 0, cy = 0, radius = 0;
};

DragOutcome drag_arc_start(const json& entities, const json& constraints, unsigned int arc_id,
                           double tx, double ty, const std::string& label) {
    DragOutcome out;
    onecad::wire::TranslateResult tr = onecad::wire::translate(sketch_of(entities, constraints));
    check(tr.ok, label + ": translates: " + tr.error);
    if (!tr.ok) return out;

    const sk::EntityID start = tr.index.resolve_point(uuid(arc_id), "start");
    const sk::EntityID center = tr.index.resolve_point(uuid(arc_id), "center");
    check(!start.empty(), label + ": `<id>.start` resolves to a real point handle");
    check(!tr.index.resolve_point(uuid(arc_id), "end").empty(),
          label + ": `<id>.end` resolves to a real point handle");
    if (start.empty() || center.empty()) return out;

    tr.sketch->solve();
    tr.sketch->beginPointDrag(start);
    const sk::SolveResult r = tr.sketch->solveWithDrag(start, sk::Vec2d{tx, ty});
    check(r.success, label + ": the drag succeeds");
    tr.sketch->endPointDrag();

    const auto* sp = tr.sketch->getEntityAs<sk::SketchPoint>(start);
    const auto* cp = tr.sketch->getEntityAs<sk::SketchPoint>(center);
    const auto* arc = tr.sketch->getEntityAs<sk::SketchArc>(tr.index.wire_to_internal.at(uuid(arc_id)));
    if (!sp || !cp || !arc) {
        check(false, label + ": endpoint / center / arc are live entities");
        return out;
    }
    out = {r.success, sp->position().X(), sp->position().Y(),
           cp->position().X(), cp->position().Y(), arc->radius()};
    std::fprintf(stderr, "  %s: .start -> (%.4f, %.4f) center (%.4f, %.4f) r %.4f\n",
                 label.c_str(), out.x, out.y, out.cx, out.cy, out.radius);
    return out;
}

// (c) `<id>.start` is a REAL draggable handle (SCHEMA §7.4's SolveDrag example
//     `"pointId": "e3.start"`). Dragging it goes through PlaneGCS and the arc
//     rule holds throughout: the endpoint can never leave its own circle. Note
//     `beginPointDrag`'s pins are SOFT (they enter the auxiliary, negative-tag
//     subsystem), so a bare arc is free to trade radius against drag error —
//     what is invariant is the coupling, not the radius.
void test_arc_start_drag_keeps_the_endpoint_on_its_circle() {
    const DragOutcome d = drag_arc_start(bare_arc(700), json::array(), 700, 4.0, 4.0, "bare arc");
    if (!d.ok) return;
    const double dist = std::hypot(d.x - d.cx, d.y - d.cy);
    check(std::abs(dist - d.radius) < 1e-6,
          "the dragged endpoint stays ON its arc (|p-c| = " + std::to_string(dist) +
              ", radius = " + std::to_string(d.radius) + ")");
    check(d.y > 1.0, "the endpoint swept toward the drag target");
}

// (c2) With the radius + center actually PINNED by user constraints, the drag is
//      the textbook arc-endpoint drag: the endpoint sweeps along a fixed circle
//      instead of following the cursor inward.
void test_arc_start_drag_preserves_a_constrained_radius() {
    json constraints = json::array({
        {{"id", uuid(710)}, {"type", "Fixed"}, {"entities", {uuid(700)}}, {"positions", {"Center"}}},
        {{"id", uuid(711)}, {"type", "Radius"}, {"entities", {uuid(700)}}, {"value", kR}},
    });
    const DragOutcome d =
        drag_arc_start(bare_arc(700), constraints, 700, 4.0, 4.0, "pinned arc");
    if (!d.ok) return;
    check(std::abs(d.cx) < 1e-6 && std::abs(d.cy) < 1e-6, "the Fixed center did not move");
    check(std::abs(d.radius - kR) < 1e-6, "the Radius constraint held");
    const double dist = std::hypot(d.x - d.cx, d.y - d.cy);
    check(std::abs(dist - kR) < 1e-6,
          "the dragged endpoint swept along the fixed circle (|p-c| = " + std::to_string(dist) +
              ", expected " + std::to_string(kR) + ")");
    check(d.y > 1.0, "the endpoint swept toward the drag target");
}

// (d) A construction arc's minted endpoints inherit the flag (SCHEMA §7.3: a
//     child point synthesized for an inline-coordinate parent inherits it), so
//     region exclusion still sees a wholly-construction arc.
void test_construction_arc_endpoints_inherit_the_flag() {
    json entities = json::array();
    entities.push_back({{"id", uuid(800)}, {"type", "Arc"}, {"center", {0.0, 0.0}},
                        {"radius", kR}, {"startAngle", 0.0}, {"endAngle", M_PI / 2},
                        {"construction", true}});
    onecad::wire::TranslateResult tr =
        onecad::wire::translate(sketch_of(entities, json::array()));
    check(tr.ok, "construction arc translates: " + tr.error);
    if (!tr.ok) return;
    for (const char* role : {"start", "end", "center"}) {
        const sk::EntityID pid = tr.index.resolve_point(uuid(800), role);
        const auto* p = pid.empty() ? nullptr : tr.sketch->getEntityAs<sk::SketchPoint>(pid);
        check(p != nullptr && p->isConstruction(),
              std::string("construction arc's `.") + role + "` point inherits the flag");
    }
}

// (e) The angle-form echo: a welded cap that ROTATES under a solve must write
//     back the angles it settled on, or the stored wire re-dirties every diff.
void test_angle_form_arc_echoes_solved_angles() {
    json args = sketch_of(slot_entities(), slot_constraints(true));
    onecad::wire::TranslateResult tr = onecad::wire::translate(args);
    check(tr.ok, "slot translates for the echo check");
    if (!tr.ok) return;
    tr.sketch->solve();
    onecad::wire::apply_solved_positions(args, *tr.sketch, tr.index);
    for (const json& e : args["entities"]) {
        if (e.value("type", std::string{}) != "Arc") continue;
        check(e.contains("startAngle") && e.contains("endAngle"),
              "an angle-form arc keeps its angle fields after write-back");
        check(e["startAngle"].is_number() && e["endAngle"].is_number(),
              "the echoed angles are numbers");
        check(e.contains("radius") && e["radius"].is_number(), "the echoed radius is a number");
    }
}

}  // namespace

int main() {
    test_welded_slot_solves_clean();
    test_welds_remove_exactly_eight_dof();
    test_endpoint_weld_makes_entity_tangent_redundant();
    test_arc_start_drag_keeps_the_endpoint_on_its_circle();
    test_arc_start_drag_preserves_a_constrained_radius();
    test_construction_arc_endpoints_inherit_the_flag();
    test_angle_form_arc_echoes_solved_angles();

    if (g_failures > 0) {
        std::fprintf(stderr, "%d check(s) failed\n", g_failures);
        return 1;
    }
    std::fprintf(stderr, "all arc-endpoint checks passed\n");
    return 0;
}
