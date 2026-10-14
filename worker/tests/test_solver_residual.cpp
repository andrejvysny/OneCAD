// WP-B probe — honest solve. RED-FIRST measurement, no production fix.
//
// P-B4 `test_contradictory_distances_are_not_a_successful_solve`
//   Two points carrying TWO Distance constraints on the SAME pair (10 and 20) is
//   an unsatisfiable system: no placement makes both true. `ConstraintSolver`
//   reports `result.success = (Success || Converged)` (ConstraintSolver.cpp:434),
//   and PlaneGCS returns `Converged` when the STEP fell below the convergence
//   criterion even though the ERROR is still above `smallF` (GCS.cpp:2078-2085) —
//   i.e. a local minimum of an inconsistent system. A solve that lands 5 mm away
//   from both dimensions must not be reported as a success, because every
//   downstream consumer (`build_profile_face`, `SolverLane::on_regions`) gates on
//   exactly this flag and then models the geometry it produced.
//
//   The control is the same pair with ONE Distance 10, already satisfied: that
//   IS a success and must stay one.
//
// WP-B T4.4 — `maxResidual`, REPORTING ONLY (SCHEMA §7.4). The measurement the
//   probe used to do by hand now lives in `maxConstraintResidual` and is assigned
//   to `SolverResult::residual` / `sk::SolveResult::residual` by the EXACT solve.
//   The verdicts above are unchanged BY CONSTRUCTION and re-asserted here: the
//   contradictory pair still reports `Diverged` / `success=false`, now carrying a
//   measured 10 mm; the control still succeeds, now carrying a measured ~0. A
//   constraint whose entities cannot be read is SKIPPED, never `+infinity` — a
//   non-finite value is a wire failure (SCHEMA §4), so that is a hard invariant
//   and gets its own case.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <string>

#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "protocol/SolverLane.h"
#include "session/SketchStore.h"
#include "sketch/Sketch.h"
#include "sketch/SketchPoint.h"
#include "sketch/WireSketch.h"
#include "sketch/constraints/Constraints.h"
#include "sketch/solver/ConstraintSolver.h"

using nlohmann::json;
using onecad::protocol::Envelope;
namespace sk = onecad::core::sketch;
namespace cs = onecad::core::sketch::constraints;

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

const char* status_name(sk::SolverResult::Status status) {
    switch (status) {
        case sk::SolverResult::Status::Uninitialized: return "Uninitialized";
        case sk::SolverResult::Status::Success: return "Success";
        case sk::SolverResult::Status::PartialSuccess: return "PartialSuccess";
        case sk::SolverResult::Status::MaxIterations: return "MaxIterations";
        case sk::SolverResult::Status::Timeout: return "Timeout";
        case sk::SolverResult::Status::Diverged: return "Diverged";
        case sk::SolverResult::Status::Redundant: return "Redundant";
        case sk::SolverResult::Status::Overconstrained: return "Overconstrained";
        case sk::SolverResult::Status::Underconstrained: return "Underconstrained";
        case sk::SolverResult::Status::InvalidInput: return "InvalidInput";
        case sk::SolverResult::Status::InternalError: return "InternalError";
    }
    return "?";
}

void report(const char* label, const sk::SolverResult& result, const sk::Sketch& sketch,
            const sk::EntityID& a, const sk::EntityID& b) {
    const auto* pa = sketch.getEntityAs<sk::SketchPoint>(a);
    const auto* pb = sketch.getEntityAs<sk::SketchPoint>(b);
    const double separation =
        (pa && pb) ? std::hypot(pb->x() - pa->x(), pb->y() - pa->y()) : -1.0;
    std::fprintf(stderr,
                 "%s: success=%d status=%s iterations=%d residual=%.9f conflicting=%zu "
                 "redundant=%zu error='%s'\n",
                 label, result.success ? 1 : 0, status_name(result.status), result.iterations,
                 result.residual, result.conflictingConstraints.size(),
                 result.redundantConstraints.size(), result.errorMessage.c_str());
    if (pa && pb) {
        std::fprintf(stderr, "  A=(%.9f, %.9f) B=(%.9f, %.9f) |AB|=%.9f\n", pa->x(), pa->y(),
                     pb->x(), pb->y(), separation);
    }
}

void test_contradictory_distances_are_not_a_successful_solve() {
    sk::Sketch sketch;
    const sk::EntityID a = sketch.addPoint(0.0, 0.0);
    const sk::EntityID b = sketch.addPoint(10.0, 0.0);
    auto* pa = sketch.getEntityAs<sk::SketchPoint>(a);
    auto* pb = sketch.getEntityAs<sk::SketchPoint>(b);
    check(pa != nullptr && pb != nullptr, "both points exist");
    if (!pa || !pb) return;

    cs::DistanceConstraint ten(a, b, 10.0);
    cs::DistanceConstraint twenty(a, b, 20.0);

    sk::ConstraintSolver solver;
    solver.addPoint(pa);
    solver.addPoint(pb);
    check(solver.addConstraint(&ten), "Distance 10 registers with the solver");
    check(solver.addConstraint(&twenty), "Distance 20 registers with the solver");

    const sk::SolverResult result = solver.solve(&sketch);
    report("P-B4 contradictory Distance(10) + Distance(20)", result, sketch, a, b);

    // The hand-rolled reference the probe used before `SolverResult::residual`
    // was ever assigned — kept as the independent oracle the field must equal.
    const double errorTen = ten.getError(sketch);
    const double errorTwenty = twenty.getError(sketch);
    const double maxResidual = std::max(errorTen, errorTwenty);
    std::fprintf(stderr,
                 "  |AB|−10 residual = %.9f, |AB|−20 residual = %.9f, max = %.9f\n", errorTen,
                 errorTwenty, maxResidual);

    check(maxResidual > 1e-4,
          "sanity: an unsatisfiable pair really does leave a residual above tolerance");
    check(!result.success,
          "a solve that leaves a dimension unsatisfied must NOT report success");

    // WP-B T4.4: the measurement is now the solver's, and it changes NO decision.
    check(std::isfinite(result.residual), "maxResidual: the measurement is finite");
    check(std::abs(result.residual - 10.0) < 1e-9,
          "maxResidual: the unsatisfied Distance 20 is 10 mm off the 10 mm the "
          "restored pose holds");
    check(std::abs(result.residual - maxResidual) < 1e-12,
          "maxResidual: the solver's number IS the per-constraint maximum");
    check(result.status == sk::SolverResult::Status::Diverged,
          "maxResidual is reporting only: the status is still Diverged");
    check(!result.success, "maxResidual is reporting only: success is still false");
    check(result.conflictingConstraints.size() == 2,
          "maxResidual is reporting only: both constraints are still blamed");
}

// SCHEMA §7.4: "A constraint whose entities cannot be read yields no residual and
// is skipped, never `+infinity`." `getError` returns +inf for a missing entity
// (Constraints.cpp:470 and friends), and a non-finite number is rejected on the
// wire (SCHEMA §4) — so letting one through would take down the frame, not just
// mis-report a value.
//
// Asserted on `maxConstraintResidual` DIRECTLY, because that is the only level
// where the rule can bite: `ConstraintSolver::addConstraint` refuses a constraint
// it cannot translate, so the solver's own set is already free of them. The
// EndGesture wire site measures over `Sketch::getAllConstraints()`, which is NOT
// filtered that way — that is the path this case stands in for.
void test_an_unreadable_constraint_is_skipped_not_infinite() {
    sk::Sketch sketch;
    const sk::EntityID a = sketch.addPoint(0.0, 0.0);
    const sk::EntityID b = sketch.addPoint(10.0, 0.0);

    // `ghost` names an entity that is in no sketch at all; `twenty` is readable
    // and 10 mm short of its 20 mm target at the authored pose.
    cs::DistanceConstraint twenty(a, b, 20.0);
    cs::DistanceConstraint ghost(a, uuid(999), 5.0);
    check(!std::isfinite(ghost.getError(sketch)),
          "unreadable: the ghost constraint really does report +infinity");

    const std::vector<sk::SketchConstraint*> mixed{&twenty, &ghost};
    const double measured = sk::maxConstraintResidual(sketch, mixed);
    std::fprintf(stderr, "P-B4 unreadable-constraint residual = %.9f\n", measured);
    check(std::isfinite(measured),
          "unreadable: a constraint that cannot be read must not make the residual "
          "non-finite — that value is refused on the wire");
    check(std::abs(measured - 10.0) < 1e-9,
          "unreadable: the readable violation still sets the maximum");
    check(sk::maxConstraintResidual(sketch, std::vector<sk::SketchConstraint*>{&ghost}) == 0.0,
          "unreadable: a wholly unreadable set measures 0.0, not infinity");
}

// The same contradiction as a consumer actually meets it: authored on the wire,
// translated, and solved through `Sketch::solve()` — the exact call
// `build_profile_face` and `SolverLane::on_regions` gate on.
void test_the_production_wire_path_reports_the_same_verdict() {
    const json sketch = {
        {"sketchId", "contradictory-distance"},
        {"plane", {{"kind", "XY"}}},
        {"entities", json::array({
                         {{"id", uuid(1)}, {"type", "Point"}, {"at", json::array({0.0, 0.0})}},
                         {{"id", uuid(2)}, {"type", "Point"}, {"at", json::array({10.0, 0.0})}},
                     })},
        {"constraints", json::array({
                            {{"id", uuid(50)},
                             {"type", "Distance"},
                             {"entities", json::array({uuid(1), uuid(2)})},
                             {"value", 10.0}},
                            {{"id", uuid(51)},
                             {"type", "Distance"},
                             {"entities", json::array({uuid(1), uuid(2)})},
                             {"value", 20.0}},
                        })},
    };

    onecad::wire::TranslateResult translated = onecad::wire::translate(sketch);
    check(translated.ok, "the contradictory sketch translates: " + translated.error);
    if (!translated.ok) return;

    const sk::SolveResult solve = translated.sketch->solve();
    const auto* pa = translated.sketch->getEntityAs<sk::SketchPoint>(
        translated.index.wire_to_internal.at(uuid(1)));
    const auto* pb = translated.sketch->getEntityAs<sk::SketchPoint>(
        translated.index.wire_to_internal.at(uuid(2)));
    std::fprintf(stderr,
                 "P-B4 production wire path: success=%d residual=%.9f conflicting=%zu "
                 "error='%s'\n",
                 solve.success ? 1 : 0, solve.residual, solve.conflictingConstraints.size(),
                 solve.errorMessage.c_str());
    if (pa && pb) {
        std::fprintf(stderr, "  A=(%.9f, %.9f) B=(%.9f, %.9f) |AB|=%.9f\n", pa->x(), pa->y(),
                     pb->x(), pb->y(), std::hypot(pb->x() - pa->x(), pb->y() - pa->y()));
    }

    check(!solve.success,
          "the production wire path must not call an unsatisfiable sketch solved");
    // The value `SolverLane::on_upsert` puts on the wire as §7.4 `maxResidual`.
    check(std::isfinite(solve.residual) && std::abs(solve.residual - 10.0) < 1e-9,
          "the wire path reports the same measured 10 mm residual");
}

// SCHEMA §7.4 `maxResidual` on `EndGesture`, through the LANE — specifically the
// `commit.finalTarget` branch, which is the one that can silently report a
// measured-looking 0.0.
//
// `on_end` has two ways to reach its final pose: a plain exact `Sketch::solve()`,
// which measures, and `run_step()` with the committed pointer-up target, which is
// a DRAG entry and by policy measures nothing. Sourcing the field from that
// result would put `0.0` — "unmeasured" — on the wire for exactly half of all
// pointer-ups, indistinguishable from "nothing is violated". `on_end` therefore
// measures from the SKETCH, and this case is what holds it there: the
// contradictory pair is 10 mm out and must say so on the finalTarget branch too.
//
// The NUMBER lives here rather than in `protocol/fixtures/sketch_solve_residual.ndjson`
// for the same reason the coordinates do: a nonzero residual is not a literal the
// cross-track contract pins.
void test_conflicting_begin_is_refused_without_store_mutation() {
    onecad::session::SketchStore store;
    onecad::protocol::Dispatcher dispatcher;
    onecad::protocol::SolverLane lane(store);
    lane.register_verbs(dispatcher);

    const json entities = json::array({
        {{"id", "q1"}, {"type", "Point"}, {"at", json::array({0.0, 0.0})}},
        {{"id", "q2"}, {"type", "Point"}, {"at", json::array({10.0, 0.0})}},
    });
    const json constraints = json::array({
        {{"id", "d10"}, {"type", "Distance"}, {"entities", json::array({"q1", "q2"})},
         {"value", 10.0}},
        {{"id", "d20"}, {"type", "Distance"}, {"entities", json::array({"q1", "q2"})},
         {"value", 20.0}},
    });

    const Envelope up = dispatcher.dispatch_once(Envelope::request(
        1, "SketchUpsert",
        {{"sketchId", "cx"}, {"plane", {{"kind", "XY"}}}, {"entities", entities},
         {"constraints", constraints}}));
    check(up.ok.value_or(false), "lane: the contradictory SketchUpsert answers ok");
    check(up.result.contains("maxResidual"), "lane: SketchUpsert carries maxResidual");
    check(up.result.value("maxResidual", -1.0) == 10.0,
          "lane: SketchUpsert measures the 10 mm the unsatisfied dimension is out by");
    check(up.result.value("state", std::string{}) == "Conflicting",
          "lane: reporting only — the state PlaneGCS reported is unchanged");
    check(!up.result.contains("positions") && !up.result.contains("curves"),
          "failed exact upsert must not publish geometry corrections");

    const Envelope begun = dispatcher.dispatch_once(Envelope::request(
        2, "BeginGesture",
        {{"sketchId", "cx"}, {"sketchRevision", 1}, {"gestureId", 73},
         {"drag", {{"pointId", "q2"}}}}));
    check(!begun.ok.value_or(true), "lane: BeginGesture refuses conflicting exact truth");
    check(begun.error.has_value() && begun.error->code == "OP_FAILED",
          "lane: conflicting Begin returns OP_FAILED");
    check(begun.error.has_value() && begun.error->detail.has_value() &&
          begun.error->detail->contains("diagnostics") &&
          !begun.error->detail->at("diagnostics").empty(),
          "lane: conflicting Begin preserves typed refusal diagnostics");
    check(store.snapshot("cx")->revision == 1,
          "lane: refused Begin leaves retained sketch revision unchanged");
    check(store.snapshot("cx")->wire_args.at("entities") == entities,
          "lane: refused Begin leaves retained request pose unchanged");
}

void test_numerically_successful_near_dimensions_are_residually_conflicting() {
    sk::Sketch sketch;
    const auto a = sketch.addPoint(0.0, 0.0);
    const auto b = sketch.addPoint(10.0, 0.0);
    cs::DistanceConstraint ten(a, b, 10.0);
    cs::DistanceConstraint near(a, b, 10.001);
    sk::SolverConfig config;
    config.detectRedundant = false;  // Preserve numerical termination status for this probe.
    sk::ConstraintSolver solver(config);
    solver.addPoint(sketch.getEntityAs<sk::SketchPoint>(a));
    solver.addPoint(sketch.getEntityAs<sk::SketchPoint>(b));
    check(solver.addConstraint(&ten) && solver.addConstraint(&near), "near dimensions register");
    const auto numerical = solver.solve(&sketch);
    report("ONECAD-14 numerical Distance(10)+Distance(10.001)", numerical, sketch, a, b);
    check(numerical.success, "near contradiction numerically succeeds (not a wire-only claim)");
    check(numerical.status == sk::SolverResult::Status::Success ||
          numerical.status == sk::SolverResult::Status::PartialSuccess,
          "actual termination is Success or Converged, without redundant status override");
    check(numerical.residual > 1e-4, "numerically successful pose violates physical length budget");
    const json args = {{"sketchId", "near-dimensions"}, {"plane", {{"kind", "XY"}}},
        {"entities", json::array({{{"id", "a"}, {"type", "Point"}, {"at", {0,0}}},
            {{"id", "b"}, {"type", "Point"}, {"at", {10,0}}}})},
        {"constraints", json::array({{{"id", "ten"}, {"type", "Distance"}, {"entities", {"a","b"}}, {"value",10}},
            {{"id", "near"}, {"type", "Distance"}, {"entities", {"a","b"}}, {"value",10.001}}})}};
    onecad::session::SketchStore store;
    onecad::protocol::Dispatcher dispatcher;
    onecad::protocol::SolverLane lane(store);
    lane.register_verbs(dispatcher);
    const auto reply = dispatcher.dispatch_once(Envelope::request(1, "SketchUpsert", args));
    check(reply.ok.value_or(false), "finite residual-violating pose remains publishable");
    std::fprintf(stderr, "ONECAD-14 near-dimensions reply: %s\n", reply.result.dump().c_str());
    check(reply.result.value("state", std::string{}) == "Conflicting",
          "numerical success does not hide an unsatisfied retained pose");
    check(reply.result.contains("reasons") && reply.result.at("reasons").value("near", std::string{}) == "residualViolation",
          "unsatisfied retained dimension has named residualViolation reason");
}

void test_a_satisfiable_distance_still_succeeds() {
    sk::Sketch sketch;
    const sk::EntityID a = sketch.addPoint(0.0, 0.0);
    const sk::EntityID b = sketch.addPoint(10.0, 0.0);
    auto* pa = sketch.getEntityAs<sk::SketchPoint>(a);
    auto* pb = sketch.getEntityAs<sk::SketchPoint>(b);
    if (!pa || !pb) {
        check(false, "control: both points exist");
        return;
    }

    cs::DistanceConstraint ten(a, b, 10.0);
    sk::ConstraintSolver solver;
    solver.addPoint(pa);
    solver.addPoint(pb);
    check(solver.addConstraint(&ten), "control: Distance 10 registers with the solver");

    const sk::SolverResult result = solver.solve(&sketch);
    report("P-B4 control single Distance(10)", result, sketch, a, b);

    check(result.success, "control: a satisfiable single dimension solves");
    check(ten.getError(sketch) < 1e-9, "control: …to a zero residual");
    // WP-B T4.4: 0.0 here means MEASURED-and-satisfied, not unmeasured.
    check(std::isfinite(result.residual) && result.residual <= 1e-4,
          "control: maxResidual is measured and inside the 1e-4 mm solver tolerance");
    check(std::abs(pa->x()) < 1e-9 && std::abs(pa->y()) < 1e-9 &&
              std::abs(pb->x() - 10.0) < 1e-9 && std::abs(pb->y()) < 1e-9,
          "control: an already-satisfied system leaves both points where they were");
}
}  // namespace

int main() {
    test_contradictory_distances_are_not_a_successful_solve();
    test_an_unreadable_constraint_is_skipped_not_infinite();
    test_the_production_wire_path_reports_the_same_verdict();
    test_conflicting_begin_is_refused_without_store_mutation();
    test_a_satisfiable_distance_still_succeeds();
    test_numerically_successful_near_dimensions_are_residually_conflicting();
    if (g_failures == 0) {
        std::fprintf(stderr, "solver_residual: OK\n");
    }
    return g_failures;
}
