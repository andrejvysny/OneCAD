// test_analyze_edge_op_range.cpp — WP4 verb half. `FilletRangeAnalyzer` already
// proves the kernel measures the frontier honestly (`test_fillet_range`); this
// proves the VERB reports what was measured, in the shape SCHEMA §7.6 promises,
// and that its projection cannot be talked into an answer the probes do not
// support.
//
// Two lanes, deliberately:
//   * the HANDLER, driven in-process against a seeded session, for the cases a
//     real body produces (the box frontier, chamfer, budget exhaustion,
//     refusals, the read-only guarantee);
//   * `derive_edge_op_range` directly, for the cases NO fixture geometry
//     produces — a feasible island above a refusal region, and an interval list
//     long enough to overflow the cap. Those exist in the code because
//     feasibility is not known to be monotonic; if the only way to test them
//     were to find geometry that exhibits them, they would go untested.

#include <cmath>
#include <cstdio>
#include <map>
#include <string>
#include <utility>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/FilletRangeAnalyzer.h"
#include "protocol/Envelope.h"
#include "session/AnalyzeEdgeOpRange.h"
#include "session/HistoryHash.h"
#include "session/Session.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::protocol::Envelope;
using onecad::session::Session;
namespace kf = onecad::kernel::fillet;
namespace ft = onecad::tests::fillet;
namespace ses = onecad::session;

namespace {

int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

using Probes = std::map<double, kf::ProbeClassification>;
constexpr kf::ProbeClassification kOk = kf::ProbeClassification::Success;
constexpr kf::ProbeClassification kNo = kf::ProbeClassification::Refusal;

void seed_head(Session &session,
               const std::vector<std::pair<std::string, TopoDS_Shape>> &seeds) {
  session.open("doc", 1, 1, "determinism");
  auto fence =
      session.fence_and_clone(1, 1, 1, onecad::session::kEmptyPrefixHash);
  onecad::session::ScratchJob job;
  job.job_id = 1;
  job.plan_document_revision = 1;
  job.bodies = std::move(fence.cloned_bodies);
  job.partition = std::move(fence.cloned_partition);
  job.prepared_snapshot_id = fence.prepared_snapshot_id;
  for (const auto &[id, shape] : seeds)
    job.bodies.create(id, "op_seed", shape);
  session.store_prepared(std::move(job));
  session.accept_prepared(1, 1, 1);
}

Envelope request(json args) {
  Envelope req;
  req.id = 41;
  req.verb = "AnalyzeEdgeOpRange";
  req.args = std::move(args);
  return req;
}

json pick(const std::string &body, const std::string &topo) {
  return {{"bodyId", body}, {"topoKey", topo}};
}

// The vertical box edge every kernel fixture uses, addressed the way the wire
// does — by snapshot-scoped ordinal, resolved off the published body.
std::string vertical_topo_key(const TopoDS_Shape &body) {
  return onecad::elementmap::ElementMapPartition::topokey_for_shape(
      body, ft::vertical_edges(body).front(),
      onecad::kernel::elementmap::ElementKind::Edge);
}

Envelope analyze(Session &session, const std::string &mode,
                 const std::string &topo, json extra = json::object()) {
  json args = {{"snapshotId", std::uint64_t{1}},
               {"mode", mode},
               {"pickedEdges", json::array({pick("body_1", topo)})},
               {"chainTangentEdges", true}};
  for (const auto &[key, value] : extra.items())
    args[key] = value;
  return onecad::session::handle_analyze_edge_op_range(session, request(args),
                                                       onecad::CancelToken{});
}

void report(const char *label, const json &r) {
  std::fprintf(stderr,
               "ANSWER %-22s lower=%s best=%s upper=%s intervals=%zu "
               "confidence=%s monotonic=%d probes=%d stopped=%s\n",
               label, r["lowerBound"].dump().c_str(),
               r["bestKnownMax"].dump().c_str(),
               r["provenUpperBound"].dump().c_str(),
               r["feasibleIntervals"].size(),
               r.value("confidence", "").c_str(),
               r.value("monotonicObserved", false) ? 1 : 0,
               r.value("probesUsed", -1),
               r.value("stoppedReason", "").c_str());
}

// THE normative invariant (SCHEMA §7.6): `lowerBound <= bestKnownMax <
// provenUpperBound` whenever all three are non-null, plus the two identities
// that tie the interval list to the scalar bounds a consumer clamps with.
void check_invariant(const char *label, const json &r) {
  const std::string prefix = std::string(label) + ": ";
  const json &lower = r.at("lowerBound");
  const json &best = r.at("bestKnownMax");
  const json &upper = r.at("provenUpperBound");
  if (!lower.is_null() && !best.is_null())
    check(lower.get<double>() <= best.get<double>(),
          prefix + "lowerBound <= bestKnownMax");
  if (!best.is_null() && !upper.is_null())
    check(best.get<double>() < upper.get<double>(),
          prefix + "bestKnownMax < provenUpperBound");
  const json &intervals = r.at("feasibleIntervals");
  check(intervals.size() <= 8, prefix + "at most eight intervals");
  if (intervals.empty()) {
    check(lower.is_null() && best.is_null(),
          prefix + "no intervals means no feasible bound was claimed");
    return;
  }
  check(!lower.is_null() &&
            intervals.front()["lower"].get<double>() == lower.get<double>(),
        prefix + "intervals[0].lower == lowerBound");
  check(!best.is_null() &&
            intervals.back()["upper"].get<double>() == best.get<double>(),
        prefix + "intervals.back().upper == bestKnownMax");
  double previous = -1.0;
  for (const json &interval : intervals) {
    const double lo = interval["lower"].get<double>();
    const double hi = interval["upper"].get<double>();
    check(lo <= hi, prefix + "each interval is ordered");
    check(lo > previous, prefix + "intervals ascend and are disjoint");
    previous = hi;
  }
}

bool inside_intervals(const json &intervals, double value) {
  for (const json &interval : intervals) {
    if (value >= interval["lower"].get<double>() &&
        value <= interval["upper"].get<double>())
      return true;
  }
  return false;
}

// ── The canonical box, through the verb ─────────────────────────────────────
void test_box_edge_answer() {
  const TopoDS_Shape body = ft::box();
  Session session;
  seed_head(session, {{"body_1", body}});
  const Envelope resp = analyze(session, "Fillet", vertical_topo_key(body));
  check(resp.ok.value_or(false), "box: the verb answers ok");
  const json &r = resp.result;
  report("box/fillet", r);
  check_invariant("box/fillet", r);

  check(r["snapshotId"].get<std::uint64_t>() == 1, "box: snapshotId is echoed");
  check(r["mode"] == "Fillet", "box: mode is echoed");
  check(r["targetBodyId"] == "body_1", "box: the target body is named");
  check(r["edges"].size() == 1 && r["edges"][0].is_string(),
        "box: one closure edge, reported as a TopoKey");
  check(r["refusal"].is_null(), "box: an answer carries no refusal");
  check(!r["budgetExhausted"].get<bool>() &&
            r["stoppedReason"] == "converged",
        "box: the default budget completes the search");
  check(r["monotonicObserved"].get<bool>(),
        "box: one frontier, so the simple story holds");
  check(!r["intervalsTruncated"].get<bool>(),
        "box: one interval does not overflow the cap");
  check(r["confidence"] == "bracketed",
        "box: a complete monotonic bracket is `bracketed`");

  // R2 is the product's default fillet radius. The whole point of this verb is
  // that a value the user is about to be handed is inside the feasible set.
  check(inside_intervals(r["feasibleIntervals"], 2.0),
        "box: the default 2 mm radius lies inside a feasible interval");
  // 12 mm exceeds the 10 mm face the blend rolls onto. Being outside is the
  // measurement `test_fillet_range` pins at 9.99925 / 10.0; here it is the
  // CLAIM the frontend clamps with.
  check(!inside_intervals(r["feasibleIntervals"], 12.0),
        "box: 12 mm is outside every feasible interval");
  check(r["searchedRange"]["min"].get<double>() == 0.001,
        "box: the searched floor is the authoring resolution");
  check(r["searchedRange"]["max"].get<double>() >= 10.0,
        "box: the searched ceiling brackets the frontier");
  check(r["limitingEntities"].is_array(),
        "box: limiting entities are reported as a list");
  for (const json &entity : r["limitingEntities"]) {
    check(entity.value("kind", "") == "edge" &&
              !entity.value("topoKey", "").empty(),
          "box: every limiting entity names an edge TopoKey");
  }
}

// ── The engine field this verb was told to reconcile against ────────────────
// RULING 3: `provenUpperBound` must be the smallest infeasible probe ABOVE
// `bestKnownMax`, not the smallest refusal overall. The verb derives it from the
// probe sequence rather than reading the kernel's field; this asserts the two
// AGREE on a real body, which is what makes the derivation a cross-check rather
// than a second opinion.
void test_derivation_matches_engine() {
  const TopoDS_Shape body = ft::box();
  const std::vector<kf::ResolvedEdge> edges = {
      ft::resolved(body, ft::vertical_edges(body).front())};
  const kf::FilletRangeResult engine =
      kf::FilletRangeAnalyzer(body, edges, kf::EdgeOpMode::Fillet).analyze();
  Probes probes;
  for (const kf::ProbeSample &sample : engine.probes)
    probes.emplace(sample.value, sample.classification);
  const ses::EdgeOpRangeAnswer derived =
      ses::derive_edge_op_range(probes, engine.budget_exhausted, false);

  std::fprintf(stderr,
               "RECONCILE engine lower=%.17g best=%.17g upper=%.17g | derived "
               "lower=%.17g best=%.17g upper=%.17g\n",
               engine.lower_bound, engine.best_known_max,
               engine.proven_upper_bound, derived.lower_bound,
               derived.best_known_max, derived.proven_upper_bound);
  check(derived.has_lower_bound == engine.has_lower_bound &&
            derived.lower_bound == engine.lower_bound,
        "reconcile: lowerBound matches the kernel's smallest feasible probe");
  check(derived.has_best_known_max == engine.has_best_known_max &&
            derived.best_known_max == engine.best_known_max,
        "reconcile: bestKnownMax matches the kernel's largest feasible probe");
  check(derived.has_proven_upper_bound == engine.has_proven_upper_bound &&
            derived.proven_upper_bound == engine.proven_upper_bound,
        "reconcile: provenUpperBound matches the kernel's field on a monotonic body");
  check(derived.monotonic_observed == engine.monotonic_observed,
        "reconcile: the monotonicity verdict matches");
}

// ── An island: the case no fixture geometry produces ────────────────────────
void test_island_projection() {
  // Feasible [1,2], a refusal region, feasible again at [8,9]. `bestKnownMax` is
  // 9 and the ONLY honest upper bound is 10 — the refusal at 4 is not an upper
  // bound on anything, because the kernel built 8 and 9 above it.
  const Probes probes = {{1.0, kOk}, {2.0, kOk}, {4.0, kNo},  {6.0, kNo},
                         {8.0, kOk}, {9.0, kOk}, {10.0, kNo}};
  const ses::EdgeOpRangeAnswer answer =
      ses::derive_edge_op_range(probes, false, false);

  check(!answer.monotonic_observed,
        "island: two classification changes is not the simple story");
  check(answer.confidence == "nonMonotonic",
        "island: an observed island outranks every other confidence rung");
  check(answer.has_lower_bound && answer.lower_bound == 1.0,
        "island: lowerBound is the smallest feasible probe");
  check(answer.has_best_known_max && answer.best_known_max == 9.0,
        "island: bestKnownMax is the largest feasible probe, inside the island");
  check(answer.has_proven_upper_bound && answer.proven_upper_bound == 10.0,
        "island: provenUpperBound is the first refusal ABOVE bestKnownMax, "
        "not the refusal at 4");
  check(answer.lower_bound <= answer.best_known_max &&
            answer.best_known_max < answer.proven_upper_bound,
        "island: the ordering invariant survives an island");
  check(answer.feasible_intervals.size() == 2,
        "island: two maximal feasible runs");
  check(answer.feasible_intervals[0].lower == 1.0 &&
            answer.feasible_intervals[0].upper == 2.0 &&
            answer.feasible_intervals[1].lower == 8.0 &&
            answer.feasible_intervals[1].upper == 9.0,
        "island: interval endpoints are probed values, never the frontier");
  check(!answer.intervals_truncated, "island: two intervals do not truncate");
  // The reason the intervals are the answer here: a consumer that clamped to
  // `bestKnownMax` alone would happily offer 5, which was measured to FAIL.
  check(!answer.feasible_intervals.empty() &&
            answer.feasible_intervals[0].upper < 5.0 &&
            answer.feasible_intervals[1].lower > 5.0,
        "island: 5 falls in the gap the intervals expose");
}

// ── Overflow: first and last are load-bearing ───────────────────────────────
void test_interval_truncation() {
  // Ten feasible runs. The first and last carry `lowerBound` and
  // `bestKnownMax`, which are the floor and ceiling a consumer clamps with, so
  // they must survive truncation whatever their width.
  Probes probes;
  for (int i = 0; i < 10; ++i) {
    const double base = 1.0 + i * 10.0;
    // Interior runs get wider with i, so a naive "keep the widest 8" would drop
    // the first two — including the one carrying `lowerBound`.
    const double width = static_cast<double>(i);
    probes.emplace(base, kOk);
    probes.emplace(base + width, kOk);
    probes.emplace(base + width + 1.0, kNo);
  }
  const ses::EdgeOpRangeAnswer answer =
      ses::derive_edge_op_range(probes, false, false);

  check(answer.intervals_truncated, "truncate: ten runs overflow the cap of 8");
  check(answer.feasible_intervals.size() == 8, "truncate: exactly eight kept");
  check(answer.feasible_intervals.front().lower == answer.lower_bound,
        "truncate: the first interval still carries lowerBound");
  check(answer.feasible_intervals.back().upper == answer.best_known_max,
        "truncate: the last interval still carries bestKnownMax");
  double previous = -1.0;
  for (const ses::EdgeOpRangeInterval &interval : answer.feasible_intervals) {
    check(interval.lower > previous, "truncate: the kept intervals ascend");
    previous = interval.upper;
  }
}

// ── The three remaining confidence rungs ────────────────────────────────────
void test_confidence_ladder() {
  check(ses::derive_edge_op_range({{1.0, kNo}, {2.0, kNo}}, false, false)
                .confidence == "none",
        "confidence: nothing built is `none`");
  check(ses::derive_edge_op_range({{1.0, kOk}, {2.0, kOk}}, false, false)
                .confidence == "lowerOnly",
        "confidence: a floor with no refusal above it is `lowerOnly`");
  check(ses::derive_edge_op_range({{1.0, kOk}, {2.0, kNo}}, true, false)
                .confidence == "coarse",
        "confidence: a bracket from a truncated search is `coarse`");
  check(ses::derive_edge_op_range({{1.0, kOk}, {2.0, kNo}}, false, true)
                .confidence == "coarse",
        "confidence: a deadline stop is `coarse` for the same reason");
  check(ses::derive_edge_op_range({{1.0, kOk}, {2.0, kNo}}, false, false)
                .confidence == "bracketed",
        "confidence: a complete monotonic bracket is `bracketed`");
  // `none` outranks everything, including an island: with nothing built there is
  // no interval to honour and no floor to enforce.
  check(ses::derive_edge_op_range({{1.0, kNo}}, true, true).confidence == "none",
        "confidence: the ladder is total and `none` wins when nothing built");
}

// ── Chamfer (RULING 2) ──────────────────────────────────────────────────────
void test_chamfer_mode() {
  const TopoDS_Shape body = ft::box();
  Session session;
  seed_head(session, {{"body_1", body}});
  const Envelope resp = analyze(session, "Chamfer", vertical_topo_key(body));
  check(resp.ok.value_or(false), "chamfer: the verb answers ok");
  const json &r = resp.result;
  report("box/chamfer", r);
  check_invariant("box/chamfer", r);
  check(r["mode"] == "Chamfer", "chamfer: the mode is echoed, not normalized");
  check(!r["lowerBound"].is_null() && !r["bestKnownMax"].is_null(),
        "chamfer: the equal-leg oracle establishes real bounds");
  check(r["bestKnownMax"].get<double>() > 1.0 &&
            r["bestKnownMax"].get<double>() <= 10.0,
        "chamfer: the distance frontier sits inside the 10 mm face it cuts");
  check(inside_intervals(r["feasibleIntervals"], 2.0),
        "chamfer: the default 2 mm distance is feasible");
}

// ── The budget clamp and the stop it produces ───────────────────────────────
void test_budget_clamp_and_stop() {
  const TopoDS_Shape body = ft::box();
  Session session;
  seed_head(session, {{"body_1", body}});
  const std::string topo = vertical_topo_key(body);

  // Below the floor: clamped UP to 8. A budget of 3 cannot finish the growth
  // ladder, so honouring it literally would buy a useless answer rather than a
  // cheaper one.
  const json &small = analyze(session, "Fillet", topo, {{"probeBudget", 3}}).result;
  report("box/budget-3", small);
  check_invariant("box/budget-3", small);
  check(small["probesUsed"].get<int>() == 8,
        "budget: 3 is clamped up to the floor of 8");
  check(small["budgetExhausted"].get<bool>() &&
            small["stoppedReason"] == "budgetExhausted",
        "budget: exhaustion is reported as a normal stop, not an error");
  check(small["refusal"].is_null(),
        "budget: a truncated search is still an answer");
  check(!small["lowerBound"].is_null(),
        "budget: a truncated search still reports what it proved");

  // Above the ceiling: clamped DOWN to 128. The box needs ~71 probes, so the
  // clamp is observable only as "no more than the ceiling".
  const json &big = analyze(session, "Fillet", topo, {{"probeBudget", 9999}}).result;
  check(big["probesUsed"].get<int>() <= 128,
        "budget: 9999 is clamped down to the ceiling of 128");
  check(!big["budgetExhausted"].get<bool>(),
        "budget: the box's full search fits inside the clamped ceiling");

  // The default is what an absent field means, not zero.
  const json &fallback = analyze(session, "Fillet", topo).result;
  check(fallback["probesUsed"].get<int>() == big["probesUsed"].get<int>(),
        "budget: an absent probeBudget completes the same search");
}

// ── A narrowed window ───────────────────────────────────────────────────────
void test_requested_range_window() {
  const TopoDS_Shape body = ft::box();
  Session session;
  seed_head(session, {{"body_1", body}});
  const std::string topo = vertical_topo_key(body);
  const json extra = {{"range", {{"min", 1.0}, {"max", 3.0}}}};
  const json &r = analyze(session, "Fillet", topo, extra).result;
  report("box/window-1-3", r);
  check_invariant("box/window-1-3", r);

  check(r["searchedRange"]["min"].get<double>() == 1.0 &&
            r["searchedRange"]["max"].get<double>() == 3.0,
        "window: the effective range is echoed");
  check(!r["bestKnownMax"].is_null() &&
            r["bestKnownMax"].get<double>() <= 3.0,
        "window: nothing above the window is reported");
  check(!r["lowerBound"].is_null() && r["lowerBound"].get<double>() >= 1.0,
        "window: nothing below the window is reported");
  check(r["provenUpperBound"].is_null(),
        "window: the box refuses nothing under 3 mm, so no ceiling is claimed");
  check(r["confidence"] == "lowerOnly",
        "window: a floor with no refusal above it is `lowerOnly`");
  check(r["limitingEntities"].empty(),
        "window: no bounding refusal means no attribution is invented");
}

// ── Refusals are answers; errors are errors ─────────────────────────────────
void test_refusals_and_fence() {
  const TopoDS_Shape first = ft::box();
  const TopoDS_Shape second =
      BRepPrimAPI_MakeBox(gp_Pnt(20, 0, 0), 10, 10, 10).Shape();
  Session session;
  seed_head(session, {{"body_1", first}, {"body_2", second}});

  const Envelope stale = onecad::session::handle_analyze_edge_op_range(
      session,
      request({{"snapshotId", std::uint64_t{99}},
               {"mode", "Fillet"},
               {"pickedEdges", json::array({pick("body_1", "e:1")})},
               {"chainTangentEdges", true}}),
      onecad::CancelToken{});
  check(!stale.ok.value_or(true) && stale.error &&
            stale.error->code == "STALE_PREVIEW",
        "fence: a stale snapshot is refused, never answered");

  const Envelope cross = onecad::session::handle_analyze_edge_op_range(
      session,
      request({{"snapshotId", std::uint64_t{1}},
               {"mode", "Fillet"},
               {"pickedEdges", json::array({pick("body_1", "e:1"),
                                            pick("body_2", "e:1")})},
               {"chainTangentEdges", true}}),
      onecad::CancelToken{});
  check(cross.ok.value_or(false) &&
            cross.result["refusal"].value("code", "") == "crossBody",
        "refusal: a cross-body pick is an ok:true answer");
  check(cross.result["probesUsed"].get<int>() == 0 &&
            cross.result["lowerBound"].is_null() &&
            cross.result["bestKnownMax"].is_null() &&
            cross.result["provenUpperBound"].is_null() &&
            cross.result["feasibleIntervals"].empty(),
        "refusal: a refusal ran no builds and claims no bounds");
  check(cross.result["confidence"] == "none",
        "refusal: a refusal carries no confidence to act on");

  const Envelope bad_mode = onecad::session::handle_analyze_edge_op_range(
      session,
      request({{"snapshotId", std::uint64_t{1}},
               {"mode", "Draft"},
               {"pickedEdges", json::array({pick("body_1", "e:1")})}}),
      onecad::CancelToken{});
  check(!bad_mode.ok.value_or(true) && bad_mode.error &&
            bad_mode.error->code == "PROTOCOL_ERROR",
        "taxonomy: an unknown mode is a protocol error, not a refusal");

  const Envelope unresolved = onecad::session::handle_analyze_edge_op_range(
      session,
      request({{"snapshotId", std::uint64_t{1}},
               {"mode", "Fillet"},
               {"pickedEdges", json::array({pick("body_1", "e:9999")})}}),
      onecad::CancelToken{});
  check(!unresolved.ok.value_or(true) && unresolved.error &&
            unresolved.error->code == "REF_UNRESOLVED",
        "taxonomy: an edge that does not resolve is REF_UNRESOLVED");
}

// ── Cancellation ────────────────────────────────────────────────────────────
void test_cancellation() {
  const TopoDS_Shape body = ft::box();
  Session session;
  seed_head(session, {{"body_1", body}});
  onecad::CancelToken token;
  token.cancel();
  const Envelope resp = onecad::session::handle_analyze_edge_op_range(
      session,
      request({{"snapshotId", std::uint64_t{1}},
               {"mode", "Fillet"},
               {"pickedEdges",
                json::array({pick("body_1", vertical_topo_key(body))})},
               {"chainTangentEdges", true}}),
      token);
  check(!resp.ok.value_or(true) && resp.error &&
            resp.error->code == "CANCELLED",
        "cancel: a cancelled analysis is an error, not a partial answer");
  check(resp.result.is_null() || resp.result.empty(),
        "cancel: no partial result rides a cancellation");
}

// ── The read-only guarantee ─────────────────────────────────────────────────
void test_read_only() {
  const TopoDS_Shape body = ft::box();
  Session session;
  seed_head(session, {{"body_1", body}});
  const onecad::session::WorkerHead before = session.head();
  const std::size_t partition_before = session.partition_copy().size();
  const Envelope first = analyze(session, "Fillet", vertical_topo_key(body));
  const onecad::session::WorkerHead after = session.head();

  check(before.document_revision == after.document_revision &&
            before.worker_epoch == after.worker_epoch &&
            before.snapshot_id == after.snapshot_id &&
            before.has_scratch == after.has_scratch &&
            before.history_prefix_hash == after.history_prefix_hash,
        "read-only: the worker head is byte-identical before and after");
  check(!after.has_scratch, "read-only: no scratch was created");
  check(session.partition_copy().size() == partition_before,
        "read-only: nothing was minted into the element partition");

  // Determinism: `converged` stops are byte-identical. (A `deadline` stop is
  // explicitly exempt — it depends on machine load by construction.)
  const Envelope second = analyze(session, "Fillet", vertical_topo_key(body));
  check(first.result == second.result,
        "determinism: two converged runs produce the identical answer");
}

} // namespace

int main() {
  test_box_edge_answer();
  test_derivation_matches_engine();
  test_island_projection();
  test_interval_truncation();
  test_confidence_ladder();
  test_chamfer_mode();
  test_budget_clamp_and_stop();
  test_requested_range_window();
  test_refusals_and_fence();
  test_cancellation();
  test_read_only();
  return failures;
}
