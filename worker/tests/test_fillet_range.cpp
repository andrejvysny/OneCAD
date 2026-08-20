// test_fillet_range.cpp — WP4 kernel half. What radii does the kernel ACTUALLY
// accept on a given edge selection, and does `FilletRangeAnalyzer` report that
// honestly?
//
// Every number pinned below was MEASURED by this test on OCCT 8.0.1 and is
// printed on every run (the `PIN` lines) so a drift is readable, not just red.
// The pins compare with a 1e-9 mm tolerance rather than bit-equality: the probe
// sequence is bit-portable by construction (see `kGrowthFactor` in
// FilletRangeAnalyzer.cpp — no libm transcendental touches a probe value), but
// OCCT's own frontier is allowed to move by less than a nanometre without that
// being a regression. A drift large enough to matter to a user is orders above
// this tolerance.

#include <chrono>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Tool.hxx>
#include <TopExp.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <gp_Pnt.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/FilletBuilder.h"
#include "kernel/fillet/FilletRangeAnalyzer.h"
#include "kernel/validation/GeometryPrecision.h"

namespace ft = onecad::tests::fillet;
namespace kf = onecad::kernel::fillet;
namespace kv = onecad::kernel::validation;

namespace {

int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

// A pinned literal is only meaningful next to the number that was measured.
void pin(double measured, double expected, double tolerance,
         const std::string &label) {
  std::fprintf(stderr, "PIN  %-46s measured=%.17g pinned=%.17g\n", label.c_str(),
               measured, expected);
  check(std::abs(measured - expected) <= tolerance,
        label + ": measured " + std::to_string(measured) + " vs pinned " +
            std::to_string(expected));
}

void report(const char *label, const kf::FilletRangeResult &result) {
  std::fprintf(
      stderr,
      "RANGE %-24s probes=%d (ok=%d refuse=%d invalid=%d) bracket=[%g,%g] "
      "lower=%.17g contiguousMax=%.17g bestMax=%.17g provenUpper=%.17g "
      "transitions=%zu monotonic=%d exhausted=%d cancelled=%d\n",
      label, result.probes_used, result.success_probes, result.refusal_probes,
      result.invalid_probes, result.lower_seed, result.upper_seed,
      result.lower_bound, result.contiguous_success_max, result.best_known_max,
      result.proven_upper_bound, result.transitions.size(),
      result.monotonic_observed ? 1 : 0, result.budget_exhausted ? 1 : 0,
      result.cancelled ? 1 : 0);
  for (const kf::RangeTransition &transition : result.transitions) {
    std::fprintf(stderr, "      transition [%.17g -> %.17g] %s -> %s\n",
                 transition.lower, transition.upper,
                 kf::probe_classification_name(transition.from),
                 kf::probe_classification_name(transition.to));
  }
  if (!result.limiting.empty()) {
    std::string keys;
    for (const std::string &key : result.limiting.edge_topo_keys)
      keys += (keys.empty() ? "" : ",") + key;
    std::string ids;
    for (const std::string &id : result.limiting.element_ids)
      ids += (ids.empty() ? "" : ",") + id;
    std::fprintf(stderr,
                 "      limiting at %.17g code=%s contour=%d edges=[%s] "
                 "elements=[%s]\n",
                 result.limiting.probe_value,
                 result.limiting.diagnostic_code.c_str(),
                 result.limiting.contour_index, keys.c_str(), ids.c_str());
  }
}

// The bound ordering that has to hold on EVERY result, complete or partial.
void check_ordering(const char *label, const kf::FilletRangeResult &result) {
  const std::string prefix = std::string(label) + ": ";
  if (result.has_lower_bound && result.has_best_known_max)
    check(result.lower_bound <= result.best_known_max,
          prefix + "lower_bound <= best_known_max");
  if (result.has_contiguous_success_max && result.has_best_known_max)
    check(result.contiguous_success_max <= result.best_known_max,
          prefix + "contiguous_success_max <= best_known_max");
  if (result.has_best_known_max && result.has_proven_upper_bound)
    check(result.best_known_max < result.proven_upper_bound,
          prefix + "best_known_max < proven_upper_bound");
  check(result.probes_used ==
            result.success_probes + result.refusal_probes + result.invalid_probes,
        prefix + "probe counts partition the probe list");
  check(result.probes_used == static_cast<int>(result.probes.size()),
        prefix + "probes_used matches the recorded sequence");
}

TopoDS_Edge edge_between(const TopoDS_Shape &shape, const gp_Pnt &a,
                         const gp_Pnt &b) {
  for (const TopoDS_Edge &edge : ft::all_edges(shape)) {
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    if (first.IsNull() || last.IsNull())
      continue;
    const gp_Pnt p = BRep_Tool::Pnt(first);
    const gp_Pnt q = BRep_Tool::Pnt(last);
    if ((p.Distance(a) < 1.0e-6 && q.Distance(b) < 1.0e-6) ||
        (p.Distance(b) < 1.0e-6 && q.Distance(a) < 1.0e-6))
      return edge;
  }
  return {};
}

// A 10 mm box with a 1 mm-wide rib standing on top, inset from every box edge so
// the rib's four vertical edges are clean protrusion edges owned entirely by the
// rib. The rib spans x in [4.5, 5.5], y in [2, 8], z in [10, 13]: the limiting
// face at a rib vertical edge is 1 mm wide, an order below anything on the box.
TopoDS_Shape ribbed_box() {
  const TopoDS_Shape base = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
  const TopoDS_Shape rib =
      BRepPrimAPI_MakeBox(gp_Pnt(4.5, 2.0, 10.0), 1.0, 6.0, 3.0).Shape();
  BRepAlgoAPI_Fuse fuse(base, rib);
  fuse.Build();
  return fuse.Shape();
}

double milliseconds(std::chrono::steady_clock::time_point start,
                    std::chrono::steady_clock::time_point stop) {
  return std::chrono::duration<double, std::milli>(stop - start).count();
}

// ── The measurement the default probe budget is derived from ────────────────
// A budget is a claim about cost. This measures the cost, prints it, and asserts
// only the order of magnitude the claim rests on: if one probe ever costs more
// than 40 ms, `kDefaultProbeBudget` no longer keeps a full analysis near a
// second and has to be re-derived rather than quietly exceeded.
void test_probe_cost() {
  const TopoDS_Shape body = ft::box();
  const std::vector<kf::ResolvedEdge> edges = {
      ft::resolved(body, ft::vertical_edges(body).front())};

  double success_ms = 0.0;
  double refusal_ms = 0.0;
  const int repeats = 3;
  for (int i = 0; i < repeats; ++i) {
    const auto t0 = std::chrono::steady_clock::now();
    const kf::FilletBuildResult ok = kf::FilletBuilder(body, edges, 2.0).build();
    const auto t1 = std::chrono::steady_clock::now();
    const kf::FilletBuildResult bad =
        kf::FilletBuilder(body, edges, 40.0).build();
    const auto t2 = std::chrono::steady_clock::now();
    check(ok.ok, "probe cost: the 2 mm reference probe succeeds");
    check(!bad.ok, "probe cost: the 40 mm reference probe refuses");
    success_ms += milliseconds(t0, t1) / repeats;
    refusal_ms += milliseconds(t1, t2) / repeats;
  }

  const auto t0 = std::chrono::steady_clock::now();
  const kf::FilletRangeResult result =
      kf::FilletRangeAnalyzer(body, edges, kf::EdgeOpMode::Fillet).analyze();
  const auto t1 = std::chrono::steady_clock::now();
  const double analysis_ms = milliseconds(t0, t1);

  std::fprintf(stderr,
               "COST probe success=%.2f ms  refusal=%.2f ms  |  full analysis "
               "%.1f ms over %d probes (%.2f ms/probe, budget %d)\n",
               success_ms, refusal_ms, analysis_ms, result.probes_used,
               result.probes_used > 0 ? analysis_ms / result.probes_used : 0.0,
               kf::kDefaultProbeBudget);
  check(success_ms < 40.0 && refusal_ms < 40.0,
        "probe cost: one build stays under the 40 ms the default budget assumes");
  check(result.probes_used <= kf::kDefaultProbeBudget,
        "probe cost: a complete analysis fits the default budget");
}

// ── Fixture 1: 10 mm box, one vertical edge ─────────────────────────────────
void test_single_box_edge() {
  const TopoDS_Shape body = ft::box();
  const std::vector<kf::ResolvedEdge> edges = {
      ft::resolved(body, ft::vertical_edges(body).front())};
  const kf::FilletRangeResult result =
      kf::FilletRangeAnalyzer(body, edges, kf::EdgeOpMode::Fillet).analyze();
  report("box/one-edge", result);
  check_ordering("box/one-edge", result);

  check(!result.cancelled && !result.budget_exhausted,
        "box/one-edge: a complete search, neither cancelled nor truncated");
  check(result.has_lower_bound && result.has_best_known_max &&
            result.has_proven_upper_bound,
        "box/one-edge: all three bounds are established");

  // The floor is the authoring resolution itself: the smallest value OneCAD
  // claims it can perform is probed first and builds.
  check(result.probes.front().value == kv::GeometryPrecisionContext{}.authoring_resolution(),
        "box/one-edge: the first probe is authoring_resolution()");
  check(result.probes.front().classification == kf::ProbeClassification::Success,
        "box/one-edge: the smallest probe succeeds");
  pin(result.lower_bound, kv::GeometryPrecisionContext{}.authoring_resolution(),
      0.0, "box/one-edge lower_bound == authoring_resolution");

  // The geometry's own bracket. A vertical box edge blends against two 10 mm
  // faces, so a fillet cannot exceed the point where it consumes one of them;
  // half the shorter adjacent span (5 mm) is the textbook estimate and 10 mm,
  // the width itself, is the hard ceiling. The kernel's real frontier must sit
  // inside that. The `+1e-9` slack is float noise, not licence: the frontier
  // values are bisection midpoints, so 10 mm arrives as 10.000000000000002.
  check(result.best_known_max >= 4.0,
        "box/one-edge: best_known_max >= 4.0, the geometry's lower expectation");
  check(result.proven_upper_bound <= 10.0 + 1.0e-9,
        "box/one-edge: proven_upper_bound <= 10.0, the adjacent face width");

  // MEASURED (OCCT 8.0.1, Release), then pinned.
  //
  // DERIVATION. The textbook "half the shorter adjacent span" estimate says
  // 5 mm and is WRONG here, which is exactly why this analyzer runs builds
  // instead of formulas. One vertical edge of a 10x10x10 box blends against two
  // 10 mm-wide faces and rolls R onto EACH of them independently; nothing on the
  // far side is competing for that material, so the blend stays valid until R
  // consumes a whole adjacent face at R = 10. The frontier is therefore the face
  // WIDTH, and the search brackets it to [9.99925, 10.0] — a 7.5e-4 mm gap,
  // deliberately just under `authoring_resolution()` (1e-3 mm), which is where
  // `widest_interval` stops refining. 9.99925 is the last bisection midpoint
  // below 10 and 10.0 is the first refusal above it.
  pin(result.best_known_max, 9.99925, 1.0e-9, "box/one-edge best_known_max");
  pin(result.proven_upper_bound, 10.0, 1.0e-9,
      "box/one-edge proven_upper_bound");
  pin(result.contiguous_success_max, 9.99925, 1.0e-9,
      "box/one-edge contiguous_success_max");

  // The definition, made executable: the reported max builds and the reported
  // proven upper bound does not — asserted against `FilletBuilder` directly, not
  // through the analyzer that produced them.
  check(kf::FilletBuilder(body, edges, result.best_known_max).build().ok,
        "box/one-edge cross-check: FilletBuilder succeeds at best_known_max");
  check(!kf::FilletBuilder(body, edges, result.proven_upper_bound).build().ok,
        "box/one-edge cross-check: FilletBuilder refuses at proven_upper_bound");

  check(!result.limiting.empty(),
        "box/one-edge: the bounding refusal named something");
}

// ── Fixture 2: all four vertical edges ──────────────────────────────────────
// Adjacent blends share the faces they roll onto, so the feasible maximum has to
// shrink relative to one edge alone. That shrink is the assertion; the value is
// the pin.
void test_four_box_edges() {
  const TopoDS_Shape body = ft::box();
  const std::vector<TopoDS_Edge> vertical = ft::vertical_edges(body);
  check(vertical.size() == 4, "box/four-edges: the box has four vertical edges");
  const kf::FilletRangeResult result =
      kf::FilletRangeAnalyzer(body, ft::resolved(body, vertical),
                              kf::EdgeOpMode::Fillet)
          .analyze();
  report("box/four-edges", result);
  check_ordering("box/four-edges", result);

  const kf::FilletRangeResult single =
      kf::FilletRangeAnalyzer(
          body, {ft::resolved(body, vertical.front())}, kf::EdgeOpMode::Fillet)
          .analyze();
  check(result.best_known_max < single.best_known_max,
        "box/four-edges: four interacting fillets cap lower than one alone");

  // DERIVATION. With all four vertical edges blended, each 10 mm face is shared
  // by two fillets rolling in from opposite ends, so each may claim only half of
  // it: the frontier halves from 10 mm to 5 mm. That is the interaction the
  // single-edge fixture cannot show, and it is the reason a per-edge formula
  // cannot answer this question for a selection. Bracketed to
  // [4.99975, 5.0005], again a 7.5e-4 mm gap at the refinement floor.
  pin(result.best_known_max, 4.99975, 1.0e-9, "box/four-edges best_known_max");
  pin(result.proven_upper_bound, 5.0005, 1.0e-9,
      "box/four-edges proven_upper_bound");
}

// ── Fixture 3: an edge whose maximum is tiny ────────────────────────────────
void test_rib_edge() {
  const TopoDS_Shape body = ribbed_box();
  const TopoDS_Edge edge =
      edge_between(body, gp_Pnt(5.5, 2.0, 10.0), gp_Pnt(5.5, 2.0, 13.0));
  check(!edge.IsNull(), "rib: the 1 mm rib's vertical edge is present");
  if (edge.IsNull())
    return;
  const kf::FilletRangeResult result =
      kf::FilletRangeAnalyzer(body, {ft::resolved(body, edge)},
                              kf::EdgeOpMode::Fillet)
          .analyze();
  report("rib/narrow-edge", result);
  check_ordering("rib/narrow-edge", result);

  check(result.best_known_max < 1.0,
        "rib/narrow-edge: the 1 mm rib width caps the blend under 1 mm");
  // DERIVATION. The limiting adjacent face is the rib's 1 mm-wide end face
  // (x in [4.5, 5.5] at y = 2); the blend rolls across it and consumes it whole
  // at R = 1. The frontier is the rib WIDTH, an order below anything the box
  // itself allows and two orders above the authoring floor — which is the case
  // a fixed UI clamp gets wrong in both directions. Bracketed to
  // [0.99925, 1.0].
  pin(result.best_known_max, 0.99925, 1.0e-9, "rib/narrow-edge best_known_max");
  pin(result.proven_upper_bound, 1.0, 1.0e-9,
      "rib/narrow-edge proven_upper_bound");
}

// ── Fixture 4: equal-leg chamfer on the box edge ────────────────────────────
void test_chamfer_edge() {
  const TopoDS_Shape body = ft::box();
  const std::vector<kf::ResolvedEdge> edges = {
      ft::resolved(body, ft::vertical_edges(body).front())};
  const kf::FilletRangeResult result =
      kf::FilletRangeAnalyzer(body, edges, kf::EdgeOpMode::Chamfer).analyze();
  report("box/chamfer", result);
  check_ordering("box/chamfer", result);

  check(result.mode == kf::EdgeOpMode::Chamfer,
        "box/chamfer: the result reports the mode it was asked for");
  // DERIVATION. An equal-leg chamfer of distance d on the same edge cuts d back
  // along each adjacent 10 mm face, so it too survives until it consumes a whole
  // face at d = 10 and lands on the same [9.99925, 10.0] bracket as the fillet.
  // The agreement is a property of THIS fixture (two equal, orthogonal, 10 mm
  // faces), not a general rule — the two oracles are different builders and the
  // rib fixture is where they would be expected to part.
  pin(result.best_known_max, 9.99925, 1.0e-9, "box/chamfer best_known_max");
  pin(result.proven_upper_bound, 10.0, 1.0e-9, "box/chamfer proven_upper_bound");
}

// ── Budget exhaustion ───────────────────────────────────────────────────────
void test_budget_exhaustion() {
  const TopoDS_Shape body = ft::box();
  const std::vector<kf::ResolvedEdge> edges = {
      ft::resolved(body, ft::vertical_edges(body).front())};
  kf::ProbeBudget budget;
  budget.max_probes = 3;
  const kf::FilletRangeResult result =
      kf::FilletRangeAnalyzer(body, edges, kf::EdgeOpMode::Fillet, budget).analyze();
  report("box/budget-3", result);
  check_ordering("box/budget-3", result);

  check(result.budget_exhausted, "budget: a budget of 3 is exhausted");
  check(result.probes_used == 3, "budget: exactly three probes ran");
  check(!result.cancelled, "budget: exhaustion is not cancellation");
  // The partial answer still has to be an answer: what it reports is true, just
  // looser than a full run's.
  check(result.has_lower_bound && result.has_best_known_max,
        "budget: a truncated search still establishes what it proved");
}

// ── Cancellation ────────────────────────────────────────────────────────────
void test_cancellation() {
  const TopoDS_Shape body = ft::box();
  const std::vector<kf::ResolvedEdge> edges = {
      ft::resolved(body, ft::vertical_edges(body).front())};
  onecad::CancelToken token;
  token.cancel();
  const kf::FilletRangeResult result =
      kf::FilletRangeAnalyzer(body, edges, kf::EdgeOpMode::Fillet).analyze(&token);
  report("box/cancelled", result);
  check_ordering("box/cancelled", result);

  check(result.cancelled, "cancel: a pre-cancelled token is reported");
  check(result.probes_used == 0,
        "cancel: a pre-cancelled analysis runs no builds at all");
  check(!result.has_best_known_max && !result.has_proven_upper_bound,
        "cancel: nothing is claimed that was never probed");
}

// ── Determinism ─────────────────────────────────────────────────────────────
void test_determinism() {
  const TopoDS_Shape body = ft::box();
  const std::vector<kf::ResolvedEdge> edges = {
      ft::resolved(body, ft::vertical_edges(body).front())};
  const kf::FilletRangeResult first =
      kf::FilletRangeAnalyzer(body, edges, kf::EdgeOpMode::Fillet).analyze();
  const kf::FilletRangeResult second =
      kf::FilletRangeAnalyzer(body, edges, kf::EdgeOpMode::Fillet).analyze();

  check(first.probes_used == second.probes_used,
        "determinism: the two runs used the same probe count");
  bool identical_sequence = first.probes.size() == second.probes.size();
  for (std::size_t i = 0; identical_sequence && i < first.probes.size(); ++i) {
    identical_sequence = first.probes[i].value == second.probes[i].value &&
                         first.probes[i].classification ==
                             second.probes[i].classification;
  }
  check(identical_sequence,
        "determinism: the probe SEQUENCE is identical, value and verdict");
  check(first.lower_bound == second.lower_bound &&
            first.best_known_max == second.best_known_max &&
            first.proven_upper_bound == second.proven_upper_bound &&
            first.contiguous_success_max == second.contiguous_success_max,
        "determinism: every bound is bit-equal across runs");
  check(first.transitions.size() == second.transitions.size() &&
            first.monotonic_observed == second.monotonic_observed,
        "determinism: the transition list and monotonicity verdict match");
}

// ── Non-monotonic honesty ───────────────────────────────────────────────────
// Nothing here is constructed to be non-monotonic. The point is that the
// analyzer REPORTS what it saw: whatever these fixtures do, the verdict is
// printed and pinned, so a future fixture that develops an island is visible
// rather than averaged into a clean interval.
void test_monotonicity_report() {
  const TopoDS_Shape body = ft::box();
  const std::vector<kf::ResolvedEdge> edges = {
      ft::resolved(body, ft::vertical_edges(body).front())};
  const kf::FilletRangeResult result =
      kf::FilletRangeAnalyzer(body, edges, kf::EdgeOpMode::Fillet).analyze();
  std::fprintf(stderr, "PIN  %-46s measured=%d\n",
               "box/one-edge monotonic_observed",
               result.monotonic_observed ? 1 : 0);
  check(result.monotonic_observed,
        "monotonicity: the box edge shows a single frontier, as measured");
  check(result.transitions.size() == 1,
        "monotonicity: exactly one adjacent disagreement, as measured");
  check(result.contiguous_success_max == result.best_known_max,
        "monotonicity: with one frontier there is no island above it");
}

} // namespace

int main() {
  test_probe_cost();
  test_single_box_edge();
  test_four_box_edges();
  test_rib_edge();
  test_chamfer_edge();
  test_budget_exhaustion();
  test_cancellation();
  test_determinism();
  test_monotonicity_report();
  return failures;
}
