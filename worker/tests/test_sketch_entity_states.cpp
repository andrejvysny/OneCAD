// SCHEMA §7.4 `entityStates` — the PER-ENTITY constrained state.
//
// The whole point of the field is that it is NOT a whole-sketch echo: a sketch
// reporting `state: "UnderConstrained"` with `dof: 3` may still have a line that
// is completely pinned down, and the frontend colours THAT line differently from
// the free circle sitting next to it. Every case below is written so that a
// whole-sketch answer, or an answer read off `dof`, would fail it.
//
// Two rules carry most of the risk and each has its own case:
//   * OWNERSHIP UNION. PlaneGCS binds parameters, not entities. A Line owns no
//     parameter at all, an Arc owns three but is pinned down by NINE (its
//     center/start/end points carry the other six, and the tag-0 arc rules
//     couple them). So "is this entity constrained?" is a union over the entity
//     AND every point it owns — proven here from BOTH directions on a circle
//     (own param free / owned point free) and on an arc.
//   * OMISSION. A sketch carrying an ellipse has no PlaneGCS diagnosis at all
//     (the naive-DOF fallback), so the whole map is omitted rather than guessed.
//     The NDJSON harness matches subsets and cannot assert a key is ABSENT, so
//     the omission is pinned HERE — `protocol/fixtures/sketch_entity_states.ndjson`
//     exercises the same round and asserts everything except the absence.
//
// Everything is driven through the LANE (Dispatcher::dispatch_once), because the
// wire keyspace is part of the contract: the map is keyed by WIRE entity id, and
// an inline-minted point (a line endpoint, a circle center) has no wire id and
// must never appear.
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <string>
#include <unordered_map>
#include <vector>

#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "protocol/SolverLane.h"
#include "session/SketchStore.h"
#include "sketch/Sketch.h"
#include "sketch/WireSketch.h"

using nlohmann::json;
namespace sk = onecad::core::sketch;
namespace wire = onecad::wire;
using onecad::protocol::Envelope;

namespace {
int g_failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

// The state reported for one wire entity, or a marker for the two ways it can be
// missing — so a failure says WHICH.
std::string state_of(const json& result, const char* wire_id) {
    if (!result.contains("entityStates")) return "<no-map>";
    const json& m = result["entityStates"];
    if (!m.contains(wire_id)) return "<absent>";
    return m[wire_id].get<std::string>();
}

void expect_state(const json& result, const char* wire_id, const char* want,
                  const std::string& label) {
    const std::string got = state_of(result, wire_id);
    check(got == want, label + ": " + wire_id + " expected " + want + ", got " + got);
}

// --- wire builders ----------------------------------------------------------

json point_entity(const char* id, double x, double y) {
    return {{"id", id}, {"type", "Point"}, {"at", {x, y}}};
}

json line_ref(const char* id, const char* p0, const char* p1) {
    return {{"id", id}, {"type", "Line"}, {"p0Ref", p0}, {"p1Ref", p1}};
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

// A `Fixed` on a synthesized child point (`C.center`, `a.start`, …): the role
// rides in `positions`, exactly as SCHEMA §7.3 spells it.
json fixed_at(const char* id, const char* entity, const char* role) {
    return {{"id", id}, {"type", "Fixed"}, {"entities", {entity}}, {"positions", {role}}};
}

json valued(const char* id, const char* type, const char* entity, double value) {
    return {{"id", id}, {"type", type}, {"entities", {entity}}, {"value", value}};
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

    Envelope upsert(const char* sketch_id, json entities, json constraints) {
        return call("SketchUpsert", {{"sketchId", sketch_id},
                                     {"plane", {{"kind", "XY"}}},
                                     {"entities", std::move(entities)},
                                     {"constraints", std::move(constraints)}});
    }
};

// The round-1 sketch, shared by the lane case and the gesture case: a line that
// is completely pinned down sitting next to a circle that is completely free.
json round1_entities() {
    return json::array({point_entity("p1", 0, 0), point_entity("p2", 10, 0),
                        line_ref("L", "p1", "p2"), circle_entity("C", 20, 20, 5)});
}

json round1_constraints() {
    return json::array({unary("cF", "Fixed", "p1"), unary("cH", "Horizontal", "L"),
                        valued("cD", "Distance", "L", 10.0)});
}

// ---------------------------------------------------------------------------
// THE case: a fully-constrained line and a free circle in ONE under-constrained
// sketch. If `entityStates` were derived from `dof` or from `state`, every key
// here would carry the same token.
// ---------------------------------------------------------------------------
void test_line_pinned_circle_free() {
    Lane lane;
    Envelope r = lane.upsert("s", round1_entities(), round1_constraints());
    check(r.ok.value_or(false), "round1: SketchUpsert ok");

    check(r.result.value("dof", -1) == 3, "round1: dof is the circle's 3 free params");
    check(r.result.value("state", std::string{}) == "UnderConstrained",
          "round1: whole-sketch state is UnderConstrained");

    expect_state(r.result, "p1", "fullyConstrained", "round1");
    expect_state(r.result, "p2", "fullyConstrained", "round1");
    expect_state(r.result, "L", "fullyConstrained", "round1");
    expect_state(r.result, "C", "underConstrained", "round1");

    // The KEYSPACE is the wire's. `C`'s center is an inline-minted point with no
    // wire id; leaking an internal UUID here would be a new, undeclared id space.
    check(r.result.contains("entityStates"), "round1: map present");
    check(r.result["entityStates"].size() == 4,
          "round1: exactly the four WIRE entities, no minted points — got " +
              std::to_string(r.result["entityStates"].size()));

    std::fprintf(stderr, "round1 result: %s\n", r.result.dump().c_str());
}

// ---------------------------------------------------------------------------
// A sketch with NO constraint at all — the state every sketch starts in.
// PlaneGCS returns from diagnose() BEFORE identifying dependent parameters when
// not one driving constraint reaches the Jacobian, so the dependent list is
// empty for a reason that has nothing to do with the sketch being constrained.
// Read literally, the freshest possible sketch would light up entirely "fully
// constrained" while reporting dof 7.
// ---------------------------------------------------------------------------
void test_unconstrained_sketch_is_not_fully_constrained() {
    Lane lane;
    Envelope r = lane.upsert("free", round1_entities(), json::array());
    check(r.ok.value_or(false), "free: ok");
    check(r.result.value("dof", -1) == 7, "free: dof 7 (two points + circle)");
    expect_state(r.result, "p1", "underConstrained", "free");
    expect_state(r.result, "p2", "underConstrained", "free");
    expect_state(r.result, "L", "underConstrained", "free");
    expect_state(r.result, "C", "underConstrained", "free");
}

// ---------------------------------------------------------------------------
// OWNERSHIP UNION, proven from both directions on a circle. A circle owns ONE
// parameter (its radius) and ONE point (its center). Pin either alone and the
// answer must still be `underConstrained` — an implementation reading only the
// entity's own parameters fails the first half, one reading only the owned
// points fails the second.
// ---------------------------------------------------------------------------
void test_circle_needs_both_center_and_radius() {
    {
        Lane lane;  // center pinned, radius free
        Envelope r = lane.upsert(
            "sc", json::array({circle_entity("C", 0, 0, 5)}),
            json::array({fixed_at("cf", "C", "center")}));
        check(r.ok.value_or(false), "circle/center: ok");
        check(r.result.value("dof", -1) == 1, "circle/center: dof 1 (radius)");
        expect_state(r.result, "C", "underConstrained", "circle/center");
    }
    {
        Lane lane;  // radius pinned, center free
        Envelope r = lane.upsert("sr", json::array({circle_entity("C", 0, 0, 5)}),
                                 json::array({valued("cr", "Radius", "C", 5.0)}));
        check(r.ok.value_or(false), "circle/radius: ok");
        check(r.result.value("dof", -1) == 2, "circle/radius: dof 2 (center)");
        expect_state(r.result, "C", "underConstrained", "circle/radius");
    }
    {
        Lane lane;  // both
        Envelope r = lane.upsert(
            "sb", json::array({circle_entity("C", 0, 0, 5)}),
            json::array({fixed_at("cf", "C", "center"), valued("cr", "Radius", "C", 5.0)}));
        check(r.ok.value_or(false), "circle/both: ok");
        check(r.result.value("dof", -1) == 0, "circle/both: dof 0");
        expect_state(r.result, "C", "fullyConstrained", "circle/both");
    }
}

// ---------------------------------------------------------------------------
// THE ARC TRAP. An arc owns NINE parameters: center x/y, radius, startAngle,
// endAngle, and the x/y of the two endpoint points the tag-0 arc rules couple to
// them. Take the union over fewer than all nine and an arc mis-reports.
// ---------------------------------------------------------------------------
void test_arc_answers_over_all_nine_parameters() {
    // Quarter arc of radius 5 about the origin: start (5,0), end (0,5).
    const auto entities = [] {
        return json::array({arc_entity("A", 0, 0, 5, 0.0, M_PI / 2)});
    };
    {
        // Center + BOTH endpoints held. Every one of the nine parameters is then
        // determined (radius and both angles only through the arc rules — the
        // three the arc owns DIRECTLY are named by no constraint at all).
        Lane lane;
        Envelope r = lane.upsert("sa", entities(),
                                 json::array({fixed_at("f0", "A", "center"),
                                              fixed_at("f1", "A", "start"),
                                              fixed_at("f2", "A", "end")}));
        check(r.ok.value_or(false), "arc/full: ok");
        check(r.result.value("dof", -1) == 0, "arc/full: dof 0");
        expect_state(r.result, "A", "fullyConstrained", "arc/full");
    }
    {
        // Drop the END pin: the end point and `endAngle` are free, everything
        // else is pinned. An implementation that stopped at the arc's own three
        // parameters plus its center would still catch `endAngle` — but one that
        // forgot the endpoint POINTS entirely would not, so the sibling case
        // below removes the angle from the picture.
        Lane lane;
        Envelope r = lane.upsert(
            "sa2", entities(),
            json::array({fixed_at("f0", "A", "center"), fixed_at("f1", "A", "start")}));
        check(r.ok.value_or(false), "arc/openEnd: ok");
        check(r.result.value("dof", -1) == 1, "arc/openEnd: dof 1 (the end sweeps)");
        expect_state(r.result, "A", "underConstrained", "arc/openEnd");
    }
    {
        // THE ISOLATING CASE for the union. Radius pinned by a dimension, both
        // angles pinned by axis alignments against the center (start due east,
        // end due north) — so ALL THREE parameters the arc owns directly are
        // determined. Nothing pins the center, so the whole arc still slides
        // freely in the plane: the remaining freedom lives ONLY in owned points.
        // An implementation reading the arc's own parameters and stopping there
        // calls this fully constrained.
        Lane lane;
        json h = {{"id", "ha"},
                  {"type", "HorizontalPoints"},
                  {"entities", {"A", "A"}},
                  {"positions", {"center", "start"}}};
        json v = {{"id", "va"},
                  {"type", "VerticalPoints"},
                  {"entities", {"A", "A"}},
                  {"positions", {"center", "end"}}};
        Envelope r =
            lane.upsert("sa3", entities(), json::array({valued("ra", "Radius", "A", 5.0), h, v}));
        check(r.ok.value_or(false), "arc/freeCenter: ok");
        check(r.result.value("dof", -1) == 2, "arc/freeCenter: dof 2 (the center translates)");
        expect_state(r.result, "A", "underConstrained", "arc/freeCenter");
    }
}

// ---------------------------------------------------------------------------
// A shared free point makes BOTH owning lines under-constrained. Neither line is
// pinned down, and saying otherwise about either one would be a lie about the
// same degree of freedom (this matches FreeCAD).
// ---------------------------------------------------------------------------
void test_shared_free_point_frees_both_owners() {
    Lane lane;
    Envelope r = lane.upsert(
        "sh",
        json::array({point_entity("a", 0, 0), point_entity("m", 10, 0),
                     point_entity("b", 20, 0), line_ref("L1", "a", "m"),
                     line_ref("L2", "m", "b")}),
        json::array({unary("fa", "Fixed", "a"), unary("fb", "Fixed", "b")}));
    check(r.ok.value_or(false), "shared: ok");
    check(r.result.value("dof", -1) == 2, "shared: dof 2 (the middle point)");

    expect_state(r.result, "a", "fullyConstrained", "shared");
    expect_state(r.result, "b", "fullyConstrained", "shared");
    expect_state(r.result, "m", "underConstrained", "shared");
    expect_state(r.result, "L1", "underConstrained", "shared");
    expect_state(r.result, "L2", "underConstrained", "shared");
}

// ---------------------------------------------------------------------------
// Reference-locked geometry mirrors a model face: nothing may move it. The pins
// that enforce that are tag-0 solver constraints carried by no `constraints[]`
// entry, so a locked entity is `fullyConstrained` even though the wire names not
// one constraint against it.
// ---------------------------------------------------------------------------
void test_reference_locked_reports_fully_constrained() {
    Lane lane;
    json locked = line_ref("LK", "q0", "q1");
    locked["referenceLocked"] = true;
    json q0 = point_entity("q0", 0, 0);
    json q1 = point_entity("q1", 10, 0);
    q0["referenceLocked"] = true;
    q1["referenceLocked"] = true;

    Envelope r = lane.upsert("rl",
                             json::array({q0, q1, locked, circle_entity("C", 40, 40, 3)}),
                             json::array());
    check(r.ok.value_or(false), "locked: ok");
    expect_state(r.result, "LK", "fullyConstrained", "locked");
    expect_state(r.result, "q0", "fullyConstrained", "locked");
    expect_state(r.result, "q1", "fullyConstrained", "locked");
    // The free circle in the same sketch is the negative control: the pins bind
    // the locked geometry only.
    expect_state(r.result, "C", "underConstrained", "locked");
}

// ---------------------------------------------------------------------------
// Conflict outranks both other tokens, and reaches an entity through a point it
// owns: a wire `Fixed`/`HorizontalDistance` names POINTS, so a line caught in
// the contradiction is only reachable through its endpoints. Deliberately
// over-attributing — `conflicting[]` stays authoritative for WHICH constraints.
// ---------------------------------------------------------------------------
void test_conflict_projects_onto_named_entities() {
    Lane lane;
    json hd = {{"id", "hd"},
               {"type", "HorizontalDistance"},
               {"entities", {"p1", "p2"}},
               {"value", 25.0}};
    Envelope r = lane.upsert(
        "cf",
        json::array({point_entity("p1", 0, 0), point_entity("p2", 10, 0),
                     line_ref("L", "p1", "p2"), circle_entity("C", 50, 50, 4)}),
        json::array({unary("f1", "Fixed", "p1"), unary("f2", "Fixed", "p2"), hd}));
    check(r.ok.value_or(false), "conflict: ok");
    check(r.result.value("state", std::string{}) == "Conflicting", "conflict: state");
    check(!r.result["conflicting"].empty(), "conflict: conflicting[] non-empty");

    expect_state(r.result, "p1", "conflicting", "conflict");
    expect_state(r.result, "p2", "conflicting", "conflict");
    expect_state(r.result, "L", "conflicting", "conflict");
    // An entity no conflicting constraint names keeps its ordinary state — the
    // projection is targeted, not a sketch-wide repaint.
    expect_state(r.result, "C", "underConstrained", "conflict");
}

// ---------------------------------------------------------------------------
// ELLIPSE ⇒ the WHOLE map is omitted. An ellipse is not registered with PlaneGCS
// at all, so the sketch reports `dof` from a naive static count and there is no
// diagnosis to answer a per-entity question from. Reporting the entities that DO
// have an answer would be worse than silence: the sketch they live in was never
// diagnosed. (The NDJSON matcher is subset-only and cannot express this.)
// ---------------------------------------------------------------------------
void test_ellipse_omits_the_whole_map() {
    Lane lane;
    Envelope r = lane.upsert(
        "el",
        json::array({point_entity("p1", 0, 0), point_entity("p2", 10, 0),
                     line_ref("L", "p1", "p2"),
                     {{"id", "E"},
                      {"type", "Ellipse"},
                      {"center", {30, 30}},
                      {"majorR", 8.0},
                      {"minorR", 4.0}}}),
        json::array({unary("cF", "Fixed", "p1"), unary("cH", "Horizontal", "L"),
                     valued("cD", "Distance", "L", 10.0)}));
    check(r.ok.value_or(false), "ellipse: ok");
    check(!r.result.contains("entityStates"),
          "ellipse: entityStates OMITTED (naive-DOF fallback has no diagnosis)");
    // The rest of the response is unchanged — omission costs nothing else.
    check(r.result.contains("dof") && r.result.contains("state"),
          "ellipse: dof/state still reported");
}

// ---------------------------------------------------------------------------
// `pDependentParameters` ACCUMULATES across diagnose() calls (vendored GCS.cpp
// clears the conflict lists but not that one), and one SketchUpsert diagnoses
// several times. Repeating the diagnosis must not change the answer.
//
// What this proves: the read is idempotent under repeated diagnosis, which is
// the property a future count-based or index-based reading would break. It does
// NOT prove the pointer-level dedupe is load-bearing on its own — the result is
// a SET of entity ids, so a duplicated pointer collapses there anyway. The
// dedupe bounds the work at the read site and documents the hazard.
// ---------------------------------------------------------------------------
void test_repeated_diagnosis_is_idempotent() {
    wire::TranslateResult tr = wire::translate(
        {{"plane", {{"kind", "XY"}}},
         {"entities", round1_entities()},
         {"constraints", round1_constraints()}});
    check(tr.ok, "idempotent: translate ok");
    if (!tr.ok) return;

    tr.sketch->solve();
    std::vector<std::unordered_map<sk::EntityID, sk::EntityConstrainedState>> rounds;
    for (int i = 0; i < 5; ++i) {
        tr.sketch->getDegreesOfFreedom();
        tr.sketch->hasRedundantConstraints();  // a second diagnose() every round
        auto states = tr.sketch->entityConstrainedStates();
        check(states.has_value(), "idempotent: round " + std::to_string(i) + " has a map");
        if (!states) return;
        rounds.push_back(*states);
    }
    for (std::size_t i = 1; i < rounds.size(); ++i) {
        check(rounds[i] == rounds[0],
              "idempotent: round " + std::to_string(i) + " differs from round 0");
    }
    // Non-vacuity: the map must actually distinguish entities, or "identical
    // across rounds" would be satisfied by an empty map five times over.
    // 4 wire entities + the circle's inline-minted center point. The lane keys
    // the wire map off `wire_to_internal`, so the minted point never ships — but
    // the sketch-level answer does cover it.
    check(rounds[0].size() == 5, "idempotent: 5 internal entities, got " +
                                     std::to_string(rounds[0].size()));
}

// ---------------------------------------------------------------------------
// GESTURE-FIXED. BeginGesture and EndGesture carry the map; SolveDrag never
// does. The drag's tag(−1) drives are excluded from the diagnosis Jacobian, and
// every drag step ends by INVALIDATING the diagnosis the map is derived from —
// so a per-step re-derivation would read an empty dependent set and report a
// half-dragged sketch as fully constrained. A drag adds no constraint, so the
// BeginGesture answer is still the answer at pointer-up.
// ---------------------------------------------------------------------------
void test_gesture_carries_the_map_at_both_ends_only() {
    Lane lane;
    Envelope up = lane.upsert("g", round1_entities(), round1_constraints());
    check(up.ok.value_or(false), "gesture: upsert ok");
    const std::uint64_t rev = up.result.value("sketchRevision", std::uint64_t{0});

    Envelope begin = lane.call("BeginGesture", {{"sketchId", "g"},
                                                {"sketchRevision", rev},
                                                {"gestureId", 7},
                                                {"drag", {{"pointId", "C.center"}}}});
    check(begin.ok.value_or(false), "gesture: BeginGesture ok");
    check(begin.result.contains("entityStates"), "gesture: BeginGesture carries the map");
    expect_state(begin.result, "L", "fullyConstrained", "gesture/begin");
    expect_state(begin.result, "C", "underConstrained", "gesture/begin");

    Envelope d1 = lane.call(
        "SolveDrag", {{"gestureId", 7}, {"seq", 1}, {"target", json::array({25.0, 20.0})}});
    check(d1.ok.value_or(false), "gesture: SolveDrag 1 ok");
    check(!d1.result.contains("entityStates"), "gesture: SolveDrag carries NO map");
    Envelope d2 = lane.call(
        "SolveDrag", {{"gestureId", 7}, {"seq", 2}, {"target", json::array({30.0, 20.0})}});
    check(d2.ok.value_or(false), "gesture: SolveDrag 2 ok");
    check(!d2.result.contains("entityStates"), "gesture: SolveDrag 2 carries NO map");

    Envelope end = lane.call("EndGesture", {{"gestureId", 7}});
    check(end.ok.value_or(false), "gesture: EndGesture ok");
    check(end.result.contains("entityStates"), "gesture: EndGesture carries the map");
    expect_state(end.result, "L", "fullyConstrained", "gesture/end");
    expect_state(end.result, "C", "underConstrained", "gesture/end");
    check(end.result["entityStates"] == begin.result["entityStates"],
          "gesture: the map is gesture-fixed (identical at both ends)");
}

}  // namespace

int main() {
    test_line_pinned_circle_free();
    test_unconstrained_sketch_is_not_fully_constrained();
    test_circle_needs_both_center_and_radius();
    test_arc_answers_over_all_nine_parameters();
    test_shared_free_point_frees_both_owners();
    test_reference_locked_reports_fully_constrained();
    test_conflict_projects_onto_named_entities();
    test_ellipse_omits_the_whole_map();
    test_repeated_diagnosis_is_idempotent();
    test_gesture_carries_the_map_at_both_ends_only();

    if (g_failures > 0) {
        std::fprintf(stderr, "%d check(s) failed\n", g_failures);
        return 1;
    }
    std::fprintf(stderr, "all entityStates checks passed\n");
    return 0;
}
