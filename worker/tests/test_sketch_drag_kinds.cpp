// SP-2 W2 — the SCHEMA §7.4 gesture target KINDS on the solver lane.
//
// Everything here is driven through the LANE (SketchUpsert / BeginGesture /
// SolveDrag / EndGesture via Dispatcher::dispatch_once), because the behaviour
// under test IS the lane's: which points each kind pins, which offset it derives
// from `grab` and when, the degenerate guards it owns (the MIN_GEOMETRY_SIZE
// radius floor and the MIN_ARC_SWEEP refusal), and the additive `curves`
// channel. `Sketch::solveWithTargets` itself — the generalized solver entry all
// three new kinds share — is covered one layer down in test_solve_with_targets.
//
// The pin sets are the point of the whole design, so each case names the one it
// exercises: entityBody pins NOTHING (coupled geometry must follow), arcEnd pins
// everything but the arc's two endpoints (the sibling must float or the arc
// rules over-determine), radius pins the center only (the curve must not slide).
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "protocol/SolverLane.h"
#include "session/SketchStore.h"
#include "sketch/Sketch.h"
#include "sketch/SketchPoint.h"
#include "sketch/WireSketch.h"

using nlohmann::json;
namespace sk = onecad::core::sketch;
using onecad::protocol::Envelope;

namespace {
int g_failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

bool near(double a, double b, double tol = 1e-6) { return std::abs(a - b) < tol; }

// --- wire builders ----------------------------------------------------------

json point_entity(const char* id, double x, double y) {
    return {{"id", id}, {"type", "Point"}, {"at", {x, y}}};
}

json line_ref(const char* id, const char* p0, const char* p1) {
    return {{"id", id}, {"type", "Line"}, {"p0Ref", p0}, {"p1Ref", p1}};
}

json line_inline(const char* id, double x0, double y0, double x1, double y1) {
    return {{"id", id}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y1}}};
}

json circle_entity(const char* id, double cx, double cy, double r) {
    return {{"id", id}, {"type", "Circle"}, {"center", {cx, cy}}, {"radius", r}};
}

json arc_entity(const char* id, double cx, double cy, double r, double sa, double ea) {
    return {{"id", id}, {"type", "Arc"}, {"center", {cx, cy}},
            {"radius", r}, {"startAngle", sa}, {"endAngle", ea}};
}

json unary(const char* id, const char* type, const char* entity) {
    return {{"id", id}, {"type", type}, {"entities", {entity}}};
}

json binary(const char* id, const char* type, const char* a, const char* b) {
    return {{"id", id}, {"type", type}, {"entities", {a, b}}};
}

// --- the lane under test ----------------------------------------------------

// A SolverLane wired to its own store and dispatcher, driven synchronously.
// `dispatch_once` is the same execute() path the solver thread runs, minus the
// threading — the verbs are exactly what a real worker would run.
struct Lane {
    onecad::session::SketchStore store;
    onecad::protocol::Dispatcher dispatcher;
    onecad::protocol::SolverLane lane;
    std::uint64_t next_id = 1;

    Lane() : lane(store) { lane.register_verbs(dispatcher); }

    Envelope call(const char* verb, json args) {
        return dispatcher.dispatch_once(Envelope::request(next_id++, verb, std::move(args)));
    }

    // Upsert + assert it took; returns the sketch revision.
    std::uint64_t upsert(const char* sketch_id, json entities, json constraints,
                         const std::string& label) {
        Envelope r = call("SketchUpsert", {{"sketchId", sketch_id},
                                           {"plane", {{"kind", "XY"}}},
                                           {"entities", std::move(entities)},
                                           {"constraints", std::move(constraints)}});
        check(r.ok.value_or(false), label + ": SketchUpsert ok");
        return r.result.value("sketchRevision", std::uint64_t{0});
    }

    Envelope begin(const char* sketch_id, std::uint64_t revision, std::uint64_t gesture,
                   json drag) {
        return call("BeginGesture", {{"sketchId", sketch_id},
                                     {"sketchRevision", revision},
                                     {"gestureId", gesture},
                                     {"drag", std::move(drag)}});
    }

    Envelope drag(std::uint64_t gesture, std::uint64_t seq, double x, double y) {
        return call("SolveDrag",
                    {{"gestureId", gesture}, {"seq", seq}, {"target", json::array({x, y})}});
    }

    Envelope end(std::uint64_t gesture) { return call("EndGesture", {{"gestureId", gesture}}); }

    Envelope end_at(std::uint64_t gesture, double x, double y) {
        return call("EndGesture", {{"gestureId", gesture},
                                   {"commit", {{"finalTarget", json::array({x, y})}}}});
    }

    // The committed wire, after EndGesture wrote the solved pose back.
    json stored(const char* sketch_id) {
        auto s = store.snapshot(sketch_id);
        return s ? s->wire_args : json::object();
    }
};

// The stored wire entity with this id ({} when absent).
json entity_of(const json& wire, const char* id) {
    if (!wire.contains("entities")) return json::object();
    for (const json& e : wire["entities"]) {
        if (e.value("id", std::string{}) == id) return e;
    }
    return json::object();
}

// Committed coordinates of a Point entity.
bool point_at(const json& wire, const char* id, double& x, double& y) {
    const json e = entity_of(wire, id);
    if (!e.contains("at") || !e["at"].is_array() || e["at"].size() < 2) return false;
    x = e["at"][0].get<double>();
    y = e["at"][1].get<double>();
    return true;
}

// A reported position/curve member, or NaN when the key is absent.
double member(const json& obj, const std::string& key, const std::string& sub = {}) {
    if (!obj.contains(key)) return std::nan("");
    const json& v = obj.at(key);
    if (sub.empty()) return v.is_number() ? v.get<double>() : std::nan("");
    return v.is_object() && v.contains(sub) ? v.at(sub).get<double>() : std::nan("");
}

double reported_x(const json& positions, const char* handle) {
    if (!positions.contains(handle)) return std::nan("");
    return positions.at(handle)[0].get<double>();
}

double reported_y(const json& positions, const char* handle) {
    if (!positions.contains(handle)) return std::nan("");
    return positions.at(handle)[1].get<double>();
}

// ── (1) entityBody, nothing pinned, nothing to propagate through ─────────────
// The delta lands EXACTLY on every owned point: `target − grab` applied to each
// point's BeginGesture pose. This is the shape of the whole kind — the line is
// moved by dragging its BODY, not one of its handles.
void test_entity_body_translates_a_free_line_exactly() {
    Lane lane;
    const std::uint64_t rev = lane.upsert(
        "fl", json::array({point_entity("p1", 0, 0), point_entity("p2", 10, 0),
                           line_ref("l1", "p1", "p2")}),
        json::array(), "free line");

    Envelope b = lane.begin("fl", rev, 1,
                            {{"kind", "entityBody"}, {"entity", "l1"}, {"grab", {5.0, 0.0}}});
    check(b.ok.value_or(false), "entityBody BeginGesture on a free line is ready");

    Envelope d = lane.drag(1, 1, 8.0, 4.0);  // delta (3, 4)
    check(d.ok.value_or(false) && d.result.value("status", std::string{}) == "success",
          "free-line body drag succeeds");
    const json& pos = d.result["positions"];
    check(near(reported_x(pos, "p1"), 3.0) && near(reported_y(pos, "p1"), 4.0),
          "p1 moved by exactly the grab delta");
    check(near(reported_x(pos, "p2"), 13.0) && near(reported_y(pos, "p2"), 4.0),
          "p2 moved by exactly the grab delta");
    check(d.result["curves"].is_object() && d.result["curves"].empty(),
          "a line drag reports an EMPTY curves object (nothing curved changed)");

    Envelope e = lane.end_at(1, 8.0, 4.0);
    check(e.ok.value_or(false), "EndGesture commits");
    double x = 0, y = 0;
    check(point_at(lane.stored("fl"), "p2", x, y) && near(x, 13.0) && near(y, 4.0),
          "the committed wire carries the translated endpoint");
}

// ── (2) entityBody on a welded rect: the coupling FOLLOWS ────────────────────
// Dragging the bottom edge sideways drags the two top corners with it, because
// the verticals weld their x together — that only works because the kind pins
// NOTHING. The horizontals/verticals themselves still hold exactly.
void test_entity_body_carries_the_welded_corners() {
    Lane lane;
    const std::uint64_t rev = lane.upsert(
        "rect",
        json::array({point_entity("p1", 0, 0), point_entity("p2", 10, 0),
                     point_entity("p3", 10, 10), point_entity("p4", 0, 10),
                     line_ref("l1", "p1", "p2"), line_ref("l2", "p2", "p3"),
                     line_ref("l3", "p3", "p4"), line_ref("l4", "p4", "p1")}),
        json::array({unary("cH1", "Horizontal", "l1"), unary("cH3", "Horizontal", "l3"),
                     unary("cV2", "Vertical", "l2"), unary("cV4", "Vertical", "l4")}),
        "welded rect");

    Envelope b = lane.begin("rect", rev, 2,
                            {{"kind", "entityBody"}, {"entity", "l1"}, {"grab", {5.0, 0.0}}});
    check(b.ok.value_or(false), "entityBody BeginGesture on the bottom edge is ready");

    Envelope d = lane.drag(2, 1, 9.0, 0.0);  // delta (4, 0)
    check(d.ok.value_or(false) && d.result.value("status", std::string{}) == "success",
          "the coupled body drag succeeds (advisory targets, committed constraints win)");
    lane.end_at(2, 9.0, 0.0);

    const json wire = lane.stored("rect");
    double x1 = 0, y1 = 0, x2 = 0, y2 = 0, x3 = 0, y3 = 0, x4 = 0, y4 = 0;
    check(point_at(wire, "p1", x1, y1) && point_at(wire, "p2", x2, y2) &&
              point_at(wire, "p3", x3, y3) && point_at(wire, "p4", x4, y4),
          "every rect corner is committed");
    std::fprintf(stderr, "  rect: p1(%.4f,%.4f) p2(%.4f,%.4f) p3(%.4f,%.4f) p4(%.4f,%.4f)\n", x1,
                 y1, x2, y2, x3, y3, x4, y4);
    check(near(x1, 4.0, 1e-4) && near(y1, 0.0, 1e-4), "the dragged edge's p1 reached its target");
    check(near(x2, 14.0, 1e-4) && near(y2, 0.0, 1e-4), "the dragged edge's p2 reached its target");
    check(near(x4, x1, 1e-4), "the left Vertical carried the top-left corner along");
    check(near(x3, x2, 1e-4), "the right Vertical carried the top-right corner along");
    check(near(y1, y2, 1e-6) && near(y3, y4, 1e-6), "both Horizontals still hold");
    check(near(y3, 10.0, 1e-3) && near(y4, 10.0, 1e-3),
          "the untouched top edge stayed where it was");
}

// ── (3) entityBody vs a committed constraint: the SOLVER wins ────────────────
// The targets are advisory tag(−1) drives. A Fixed endpoint simply does not
// move, the rest of the body follows the cursor, and that is `success` — an
// entity lagging the cursor because a constraint holds it is the CORRECT answer
// (§7.4), not a partial solve.
void test_entity_body_yields_to_a_fixed_endpoint() {
    Lane lane;
    const std::uint64_t rev = lane.upsert(
        "fx", json::array({point_entity("p1", 0, 0), point_entity("p2", 10, 0),
                           line_ref("l1", "p1", "p2")}),
        json::array({unary("cF", "Fixed", "p1")}), "fixed-end line");

    Envelope b = lane.begin("fx", rev, 3,
                            {{"kind", "entityBody"}, {"entity", "l1"}, {"grab", {5.0, 0.0}}});
    check(b.ok.value_or(false), "entityBody BeginGesture on the fixed-end line is ready");

    Envelope d = lane.drag(3, 1, 5.0, 5.0);  // delta (0, 5)
    check(d.ok.value_or(false), "the constrained body drag responds ok");
    check(d.result.value("status", std::string{}) == "success",
          "a constraint that holds a point is SUCCESS, not partial (got " +
              d.result.value("status", std::string{}) + ")");
    const json& pos = d.result["positions"];
    check(!pos.contains("p1"), "the Fixed endpoint never moved (so it is not reported)");
    check(near(reported_x(pos, "p2"), 10.0, 1e-4) && near(reported_y(pos, "p2"), 5.0, 1e-4),
          "the free endpoint still followed the cursor");
    lane.end(3);
}

// ── (4) arcEnd reshapes: radius + angle follow, the center does NOT ──────────
// Pin set = every point except the arc's two endpoints. The center is IN it (a
// reshape must not translate the arc); the sibling endpoint is OUT (it has to
// slide along the new circle, and pinning it would over-determine the four
// internal arc rules). `positions` alone cannot describe this — the radius and
// the end angle are the result, which is exactly what `curves` carries.
void test_arc_end_reshapes_radius_and_angle() {
    Lane lane;
    const double kHalfPi = std::acos(-1.0) / 2.0;
    const std::uint64_t rev = lane.upsert(
        "ae", json::array({arc_entity("a1", 0, 0, 20.0, 0.0, kHalfPi)}), json::array(), "bare arc");

    Envelope b = lane.begin("ae", rev, 4, {{"kind", "arcEnd"}, {"entity", "a1"}, {"role", "end"}});
    check(b.ok.value_or(false), "arcEnd BeginGesture resolves the end handle");

    Envelope d = lane.drag(4, 1, -20.0, 20.0);  // 135°, |t| = 28.284
    check(d.ok.value_or(false) && d.result.value("status", std::string{}) == "success",
          "the arc-end reshape succeeds");
    const json& pos = d.result["positions"];
    const json& curves = d.result["curves"];
    check(!pos.contains("a1.center"), "the pinned center did not move");
    check(pos.contains("a1.start"), "the FREE sibling endpoint moved (it is not pinned)");
    check(curves.contains("a1"), "the reshape reports the arc through `curves`");
    const double radius = member(curves, "a1", "radius");
    const double end_angle = member(curves, "a1", "endAngle");
    std::fprintf(stderr, "  arcEnd: radius %.6f endAngle %.6f\n", radius, end_angle);
    check(near(radius, std::hypot(20.0, 20.0), 1e-4), "the radius grew to the cursor's distance");
    check(near(end_angle, std::atan2(20.0, -20.0), 1e-4),
          "the end angle swept to the cursor's bearing (135°)");
    // The sibling slides ALONG the new circle rather than sweeping around it:
    // nothing drives its angle, so the solve leaves it within a degree of where
    // it started. (The "unchanged members are omitted" half of the incremental
    // discipline is pinned by the held-radius case below, where `curves` is
    // empty outright.)
    check(std::abs(member(curves, "a1", "startAngle")) < 0.05 ||
              !curves["a1"].contains("startAngle"),
          "the start angle stayed put — the drag reshaped, it did not rotate the arc");

    // The arc rule is the invariant: both endpoints stay on the same circle,
    // whatever the drag did to the radius.
    const double sx = reported_x(pos, "a1.start");
    const double sy = reported_y(pos, "a1.start");
    check(near(std::hypot(sx, sy), radius, 1e-4),
          "the sibling endpoint slid ALONG the reshaped circle");
    check(near(std::hypot(reported_x(pos, "a1.end"), reported_y(pos, "a1.end")), radius, 1e-4),
          "the dragged endpoint is on the reshaped circle too");
    lane.end(4);
}

// ── (5) the arc sweep floor: REFUSED, partial, gesture stays OPEN ────────────
// Dragging the end onto the start collapses the sweep to nothing, and a
// collapsed arc cannot be recovered by dragging further — so the pose is never
// entered. The step restores what it last reported and answers `partial` with
// the gesture still live, which the next step proves by succeeding.
void test_arc_end_refuses_to_collapse_the_sweep() {
    Lane lane;
    const double kHalfPi = std::acos(-1.0) / 2.0;
    // The start endpoint is Fixed, so the sibling cannot rotate out of the way:
    // dragging the end onto it collapses the sweep to nothing, deterministically.
    json fixed_start = unary("cFs", "Fixed", "a1");
    fixed_start["positions"] = json::array({"Start"});
    const std::uint64_t rev =
        lane.upsert("sw", json::array({arc_entity("a1", 0, 0, 20.0, 0.0, kHalfPi)}),
                    json::array({fixed_start}), "sweep arc");

    Envelope b = lane.begin("sw", rev, 5, {{"kind", "arcEnd"}, {"entity", "a1"}, {"role", "end"}});
    check(b.ok.value_or(false), "arcEnd BeginGesture is ready");

    Envelope collapse = lane.drag(5, 1, 20.0, 0.0);  // onto the START point
    check(collapse.ok.value_or(false), "the collapsing step still answers (recoverable)");
    check(collapse.result.value("status", std::string{}) == "partial",
          "a collapsed sweep is REFUSED as partial (got " +
              collapse.result.value("status", std::string{}) + ")");
    check(collapse.result["positions"].empty(),
          "the refused step restored the pose, so it reports NO position change");
    check(collapse.result["curves"].empty(),
          "the refused step restored the curves, so it reports NO curve change");

    Envelope recover = lane.drag(5, 2, 0.0, 30.0);
    check(recover.ok.value_or(false) && recover.result.value("status", std::string{}) == "success",
          "the gesture stayed OPEN — the next step drags normally");
    check(recover.result["curves"].contains("a1"), "the recovering step reshapes the arc");

    Envelope e = lane.end(5);
    check(e.ok.value_or(false), "EndGesture after a refused step still commits");
    // The committed arc is the RECOVERED one, not the collapsed one: its sweep is
    // back above the floor (the Fixed start holds radius 20, so the recovering
    // drag reopens the arc rather than growing it).
    const json arc = entity_of(lane.stored("sw"), "a1");
    const double sweep = std::abs(arc.value("endAngle", 0.0) - arc.value("startAngle", 0.0));
    std::fprintf(stderr, "  collapse: committed sweep %.6f radius %.4f\n", sweep,
                 arc.value("radius", 0.0));
    check(sweep > 1e-3, "the committed arc still has a real sweep (got " +
                            std::to_string(sweep) + ")");
    // The Fixed start is still exactly where it was — the recovering drag
    // reshaped around it (the center is only SOFTLY pinned, so radius is free to
    // grow as the center drifts; what is hard is the constraint).
    const double cx = arc["center"][0].get<double>();
    const double cy = arc["center"][1].get<double>();
    const double r = arc.value("radius", 0.0);
    const double sa = arc.value("startAngle", 0.0);
    check(near(cx + (r * std::cos(sa)), 20.0, 1e-3) && near(cy + (r * std::sin(sa)), 0.0, 1e-3),
          "the Fixed start endpoint never moved");
}

// ── (6) radius tracks the cursor, and the grab offset never drifts ───────────
// Pin set = the center only. The offset `|grab − center| − radius` is derived
// ONCE at BeginGesture: grabbing the curve 2 mm outside its rim must keep that
// 2 mm gap for the whole gesture instead of snapping the rim to the cursor.
void test_radius_tracks_the_cursor_with_a_stable_grab_offset() {
    Lane lane;
    std::uint64_t rev = lane.upsert("rd", json::array({circle_entity("c1", 0, 0, 5.0)}),
                                    json::array(), "free circle");

    // Grab exactly ON the rim ⇒ offset 0 ⇒ the radius IS the cursor distance.
    Envelope b = lane.begin("rd", rev, 6,
                            {{"kind", "radius"}, {"entity", "c1"}, {"grab", {5.0, 0.0}}});
    check(b.ok.value_or(false), "radius BeginGesture on a free circle is ready");
    Envelope d = lane.drag(6, 1, 12.0, 0.0);
    check(d.ok.value_or(false) && d.result.value("status", std::string{}) == "success",
          "the free radius drag succeeds");
    check(d.result["positions"].empty(),
          "a radius drag moves NO point — which is exactly why `curves` had to exist");
    check(near(member(d.result["curves"], "c1", "radius"), 12.0, 1e-6),
          "the radius tracked the cursor distance");
    Envelope e = lane.end_at(6, 12.0, 0.0);
    check(near(member(e.result["curves"], "c1", "radius"), 12.0, 1e-6),
          "EndGesture reports the curve delta since BeginGesture");
    rev = e.result.value("sketchRevision", std::uint64_t{0});
    check(near(entity_of(lane.stored("rd"), "c1").value("radius", 0.0), 12.0, 1e-6),
          "the committed wire carries the new radius");

    // Grab 2 mm OUTSIDE the rim ⇒ offset 2 ⇒ the rim trails the cursor by 2.
    Envelope b2 = lane.begin("rd", rev, 7,
                             {{"kind", "radius"}, {"entity", "c1"}, {"grab", {14.0, 0.0}}});
    check(b2.ok.value_or(false), "the offset-grab BeginGesture is ready");
    Envelope d2 = lane.drag(7, 1, 20.0, 0.0);
    check(near(member(d2.result["curves"], "c1", "radius"), 18.0, 1e-6),
          "the grab offset held (radius = |cursor| − 2, not |cursor|)");
    Envelope d3 = lane.drag(7, 2, 30.0, 0.0);
    check(near(member(d3.result["curves"], "c1", "radius"), 28.0, 1e-6),
          "and it is still the SAME offset a step later (derived once, no drift)");
    lane.end(7);
}

// ── (7) the radius floor: saturates at MIN_GEOMETRY_SIZE, then recovers ──────
// Dragging across the center would ask for a zero (then negative) radius. The
// lane clamps before the solver ever sees it, so the gesture stays live instead
// of dying on a degenerate circle.
void test_radius_clamps_at_the_center_and_recovers() {
    Lane lane;
    const std::uint64_t rev = lane.upsert("cl", json::array({circle_entity("c1", 0, 0, 5.0)}),
                                          json::array(), "clamp circle");
    Envelope b = lane.begin("cl", rev, 8,
                            {{"kind", "radius"}, {"entity", "c1"}, {"grab", {5.0, 0.0}}});
    check(b.ok.value_or(false), "radius BeginGesture is ready");

    Envelope onto = lane.drag(8, 1, 0.0, 0.0);  // straight onto the center
    check(onto.ok.value_or(false) && onto.result.value("status", std::string{}) == "success",
          "dragging onto the center is a normal success, not a failure");
    check(near(member(onto.result["curves"], "c1", "radius"), sk::constants::MIN_GEOMETRY_SIZE,
               1e-9),
          "the radius saturated at MIN_GEOMETRY_SIZE");

    Envelope across = lane.drag(8, 2, -3.0, -4.0);  // 5 mm past the center
    check(near(member(across.result["curves"], "c1", "radius"), 5.0, 1e-6),
          "crossing the center is measured as DISTANCE — never a negative radius");

    Envelope out = lane.drag(8, 3, 9.0, 0.0);
    check(near(member(out.result["curves"], "c1", "radius"), 9.0, 1e-6),
          "and the radius recovers on the way back out");
    lane.end(8);
}

// ── (8) radius vs a committed Radius dimension: held, and NOT an error ───────
// The drive is a tag(−1) auxiliary; the dimension is a real constraint and
// outranks it. The right answer is `success` with nothing changed — an empty
// `curves` object, not a hard failure the UI would have to explain.
void test_radius_drag_on_a_dimensioned_circle_is_held() {
    Lane lane;
    json constraints = json::array({unary("cF", "Fixed", "c1"), {{"id", "cR"},
                                                                 {"type", "Radius"},
                                                                 {"entities", {"c1"}},
                                                                 {"value", 5.0}}});
    constraints[0]["positions"] = json::array({"Center"});
    const std::uint64_t rev = lane.upsert("dim", json::array({circle_entity("c1", 0, 0, 5.0)}),
                                          std::move(constraints), "dimensioned circle");

    Envelope b = lane.begin("dim", rev, 9,
                            {{"kind", "radius"}, {"entity", "c1"}, {"grab", {5.0, 0.0}}});
    check(b.ok.value_or(false), "radius BeginGesture on a dimensioned circle is ready");

    Envelope d = lane.drag(9, 1, 12.0, 0.0);
    check(d.ok.value_or(false), "the held radius drag responds ok");
    const std::string status = d.result.value("status", std::string{});
    check(status == "success" || status == "redundant",
          "a committed dimension holding the radius is not an ERROR (got " + status + ")");
    check(d.result["curves"].empty(), "nothing changed, so `curves` reports nothing");
    lane.end(9);
    check(near(entity_of(lane.stored("dim"), "c1").value("radius", 0.0), 5.0, 1e-6),
          "the dimensioned radius is exactly what it was");
}

// ── (9) radius propagates through a Tangent — into the LINE ──────────────────
// Pin set = the center ONLY, so everything the curve is coupled to stays free to
// follow. Growing a circle that a Horizontal line is tangent to pushes the LINE
// away; `positions` carries that half of the answer and `curves` the other half.
void test_radius_drag_moves_a_tangent_line() {
    Lane lane;
    json entities = json::array({circle_entity("c1", 0, 0, 5.0), point_entity("p1", -20, -5),
                                 point_entity("p2", 20, -5), line_ref("l1", "p1", "p2")});
    json constraints = json::array({unary("cH", "Horizontal", "l1"),
                                    binary("cT", "Tangent", "l1", "c1"),
                                    unary("cF", "Fixed", "c1")});
    constraints[2]["positions"] = json::array({"Center"});
    const std::uint64_t rev = lane.upsert("tg", std::move(entities), std::move(constraints),
                                          "tangent line + circle");

    Envelope b = lane.begin("tg", rev, 10,
                            {{"kind", "radius"}, {"entity", "c1"}, {"grab", {5.0, 0.0}}});
    check(b.ok.value_or(false), "radius BeginGesture on the tangent circle is ready");

    Envelope d = lane.drag(10, 1, 10.0, 0.0);
    check(d.ok.value_or(false), "the tangent-coupled radius drag responds ok");
    check(near(member(d.result["curves"], "c1", "radius"), 10.0, 1e-4),
          "the radius reached the cursor (got " +
              std::to_string(member(d.result["curves"], "c1", "radius")) + ")");
    lane.end_at(10, 10.0, 0.0);

    const json wire = lane.stored("tg");
    double x1 = 0, y1 = 0, x2 = 0, y2 = 0, cx = 0, cy = 0;
    check(point_at(wire, "p1", x1, y1) && point_at(wire, "p2", x2, y2), "the line is committed");
    const json circle = entity_of(wire, "c1");
    cx = circle["center"][0].get<double>();
    cy = circle["center"][1].get<double>();
    std::fprintf(stderr, "  tangent: line y %.4f / %.4f, center (%.4f,%.4f) r %.4f\n", y1, y2, cx,
                 cy, circle.value("radius", 0.0));
    check(near(cx, 0.0, 1e-6) && near(cy, 0.0, 1e-6), "the Fixed center did not move");
    check(near(y1, y2, 1e-6), "the line is still Horizontal");
    check(near(std::abs(y1), 10.0, 1e-3),
          "the TANGENT LINE moved to the new radius (a point drag could never report this)");
}

// ── (10) reference-locked geometry refuses EVERY kind ────────────────────────
// Locked geometry is immovable by contract (§7.3). Each kind must refuse rather
// than "solve" into a no-op the caller would read as a successful drag.
void test_locked_geometry_refuses_every_kind() {
    Lane lane;
    json entities = json::array({line_inline("l1", 0, 0, 10, 0), circle_entity("c1", 40, 0, 5.0),
                                 arc_entity("a1", 0, 40, 6.0, 0.0, std::acos(-1.0) / 2.0)});
    for (json& e : entities) e["referenceLocked"] = true;
    const std::uint64_t rev = lane.upsert("lk", std::move(entities), json::array(), "locked");

    struct Case {
        const char* label;
        json drag;
    } cases[] = {
        {"point", {{"kind", "point"}, {"entity", "l1"}, {"role", "p0"}}},
        {"arcEnd", {{"kind", "arcEnd"}, {"entity", "a1"}, {"role", "end"}}},
        {"radius", {{"kind", "radius"}, {"entity", "c1"}, {"grab", {45.0, 0.0}}}},
        {"entityBody", {{"kind", "entityBody"}, {"entity", "l1"}, {"grab", {5.0, 0.0}}}},
    };

    std::uint64_t gesture = 20;
    std::uint64_t revision = rev;
    for (const Case& c : cases) {
        Envelope b = lane.begin("lk", revision, gesture, c.drag);
        check(b.ok.value_or(false),
              std::string("locked ") + c.label + ": BeginGesture still RESOLVES the target");
        Envelope d = lane.drag(gesture, 1, 30.0, 30.0);
        check(d.ok.value_or(false),
              std::string("locked ") + c.label + ": the refused drag is recoverable");
        check(d.result.value("status", std::string{}) == "partial",
              std::string("locked ") + c.label + ": refused as partial (got " +
                  d.result.value("status", std::string{}) + ")");
        check(d.result["positions"].empty() && d.result["curves"].empty(),
              std::string("locked ") + c.label + ": the refused drag changed NOTHING");
        revision = lane.end(gesture).result.value("sketchRevision", revision);
        ++gesture;
    }

    const json wire = lane.stored("lk");
    check(near(entity_of(wire, "c1").value("radius", 0.0), 5.0, 1e-9),
          "the locked circle's radius survived every gesture");
    check(near(entity_of(wire, "a1").value("radius", 0.0), 6.0, 1e-9),
          "the locked arc's radius survived every gesture");
}

// ── (11) the `point` kind is byte-identical to the pre-SP-2 lane ─────────────
// The whole amendment is additive, so the legacy call — a bare `drag.pointId`,
// no kind — must still be `beginPointDrag` + `solveWithDrag` and nothing else.
// This runs the lane against a hand-rolled replay of exactly those calls and
// compares EVERY point bit for bit (==, not near).
void test_point_kind_is_byte_identical_to_solve_with_drag() {
    Lane lane;
    json entities = json::array({point_entity("p1", 0, 0), point_entity("p2", 10, 0),
                                 point_entity("p3", 10, 10), point_entity("p4", 0, 10),
                                 line_ref("l1", "p1", "p2"), line_ref("l2", "p2", "p3"),
                                 line_ref("l3", "p3", "p4"), line_ref("l4", "p4", "p1")});
    json constraints = json::array({unary("cH1", "Horizontal", "l1"),
                                    unary("cH3", "Horizontal", "l3"),
                                    unary("cV2", "Vertical", "l2"),
                                    unary("cV4", "Vertical", "l4")});
    const std::uint64_t rev =
        lane.upsert("bc", entities, constraints, "back-compat rect");

    // The reference: the exact call sequence on_begin/on_drag used before SP-2,
    // over the SAME wire the lane holds (the store carries the post-upsert pose).
    onecad::wire::TranslateResult tr = onecad::wire::translate(lane.stored("bc"));
    check(tr.ok, "the stored wire re-translates: " + tr.error);
    if (!tr.ok) return;
    const sk::EntityID dragged = tr.index.resolve_point("p3", "");
    tr.sketch->solve();
    tr.sketch->beginPointDrag(dragged);
    const sk::SolveResult ref = tr.sketch->solveWithDrag(dragged, sk::Vec2d{25.0, 14.0});
    check(ref.success, "the reference drag succeeds");

    // The lane, through the LEGACY form: no kind, just a pointId.
    Envelope b = lane.begin("bc", rev, 30, {{"pointId", "p3"}});
    check(b.ok.value_or(false), "the legacy pointId BeginGesture is ready");
    Envelope d = lane.drag(30, 1, 25.0, 14.0);
    check(d.ok.value_or(false) && d.result.value("status", std::string{}) == "success",
          "the legacy drag succeeds");

    const json& pos = d.result["positions"];
    int compared = 0;
    for (const auto& e : tr.sketch->getAllEntities()) {
        if (!e || e->type() != sk::EntityType::Point) continue;
        const auto* p = dynamic_cast<const sk::SketchPoint*>(e.get());
        if (!p) continue;
        const std::string handle = tr.index.handle_for(p->id());
        if (handle.empty() || !pos.contains(handle)) continue;
        ++compared;
        check(pos.at(handle)[0].get<double>() == p->position().X() &&
                  pos.at(handle)[1].get<double>() == p->position().Y(),
              "lane point '" + handle + "' is BIT-identical to solveWithDrag");
    }
    check(compared >= 3, "the comparison actually covered the moved corners (got " +
                             std::to_string(compared) + ")");
    check(d.result["curves"].empty(), "a pure point drag on straight geometry curves nothing");
    tr.sketch->endPointDrag();
    lane.end(30);
}

// ── unknown kind: OP_FAILED, never a silent degrade to `point` ───────────────
void test_unknown_kind_fails_the_request() {
    Lane lane;
    const std::uint64_t rev = lane.upsert("uk", json::array({point_entity("p1", 0, 0)}),
                                          json::array(), "unknown-kind sketch");
    Envelope b = lane.begin("uk", rev, 40,
                            {{"kind", "wiggle"}, {"entity", "p1"}, {"pointId", "p1"}});
    check(!b.ok.value_or(false), "an unknown kind FAILS (it must never degrade to `point`)");
    check(b.error && b.error->code == "OP_FAILED",
          "and it fails as OP_FAILED (§7.4)");

    // Unresolvable targets are REF_UNRESOLVED, not OP_FAILED.
    Envelope arc_on_point = lane.begin("uk", rev, 41,
                                       {{"kind", "arcEnd"}, {"entity", "p1"}, {"role", "end"}});
    check(!arc_on_point.ok.value_or(false) && arc_on_point.error &&
              arc_on_point.error->code == "REF_UNRESOLVED",
          "arcEnd on a non-arc is REF_UNRESOLVED");
    Envelope radius_on_point =
        lane.begin("uk", rev, 42, {{"kind", "radius"}, {"entity", "p1"}});
    check(!radius_on_point.ok.value_or(false) && radius_on_point.error &&
              radius_on_point.error->code == "REF_UNRESOLVED",
          "radius on a non-curve is REF_UNRESOLVED");
}

}  // namespace

int main() {
    test_entity_body_translates_a_free_line_exactly();
    test_entity_body_carries_the_welded_corners();
    test_entity_body_yields_to_a_fixed_endpoint();
    test_arc_end_reshapes_radius_and_angle();
    test_arc_end_refuses_to_collapse_the_sweep();
    test_radius_tracks_the_cursor_with_a_stable_grab_offset();
    test_radius_clamps_at_the_center_and_recovers();
    test_radius_drag_on_a_dimensioned_circle_is_held();
    test_radius_drag_moves_a_tangent_line();
    test_locked_geometry_refuses_every_kind();
    test_point_kind_is_byte_identical_to_solve_with_drag();
    test_unknown_kind_fails_the_request();

    if (g_failures > 0) {
        std::fprintf(stderr, "%d check(s) failed\n", g_failures);
        return 1;
    }
    std::fprintf(stderr, "all drag-kind checks passed\n");
    return 0;
}
