#include "session/AnalyzeEdgeOpRange.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <iterator>
#include <map>
#include <optional>
#include <string>
#include <thread>
#include <vector>

#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>

#include "kernel/fillet/EdgeContour.h"
#include "kernel/fillet/FilletRangeAnalyzer.h"
#include "session/EdgePicks.h"
#include "util/Log.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;
namespace kf = onecad::kernel::fillet;

namespace {

// Both caps exist so one pathological body cannot hand the frontend an
// unbounded list to reason about. Eight is enough to describe a shape with three
// islands and still fit a tooltip.
constexpr std::size_t kMaxFeasibleIntervals = 8;
constexpr std::size_t kMaxLimitingEntities = 8;

// How often the watchdog looks at the clock and at the caller's cancel token.
// Nothing waits on this but the join at the end of the analysis, so a coarse
// value costs at most one poll of latency on a search measured in hundreds of ms.
constexpr std::chrono::milliseconds kDeadlinePoll{5};

Envelope fail(const Envelope &req, const char *code, const std::string &message) {
  return Envelope::error_response(
      req.id, protocol::ErrorInfo{code, message, false});
}

std::string get_str(const json &value, const char *key) {
  return value.is_object() && value.contains(key) && value[key].is_string()
             ? value[key].get<std::string>()
             : std::string{};
}

json topo_keys(const std::vector<int> &ordinals) {
  json keys = json::array();
  for (const int ordinal : ordinals)
    keys.push_back("e:" + std::to_string(ordinal));
  return keys;
}

json refusal(const char *code, const std::string &message,
             const std::vector<int> &ordinals = {}) {
  return {{"code", code}, {"message", message}, {"edges", topo_keys(ordinals)}};
}

// The answer shape when NOTHING was probed: a closure refusal. Every bound is
// null rather than 0.0 — 0.0 is a radius a caller could act on, and none of
// these were measured. `searchedRange` is {0,0} for the same reason: no range
// was searched, and echoing the REQUEST here would claim a search that never ran.
json refused_result(std::uint64_t snapshot, const std::string &mode,
                    const std::string &target, json edges, json refusal_value) {
  return {{"snapshotId", snapshot},
          {"mode", mode},
          {"targetBodyId", target},
          {"edges", std::move(edges)},
          {"searchedRange", {{"min", 0.0}, {"max", 0.0}}},
          {"lowerBound", nullptr},
          {"bestKnownMax", nullptr},
          {"provenUpperBound", nullptr},
          {"feasibleIntervals", json::array()},
          {"intervalsTruncated", false},
          {"limitingEntities", json::array()},
          {"confidence", "none"},
          {"monotonicObserved", true},
          {"probesUsed", 0},
          {"budgetExhausted", false},
          {"stoppedReason", "converged"},
          {"refusal", std::move(refusal_value)}};
}

// ── The wall-clock stop ─────────────────────────────────────────────────────
//
// `FilletRangeAnalyzer` caps probe COUNT, never elapsed time — deliberately, so
// a range answer cannot change between two runs of the same document. That is
// the right guarantee for the kernel and the wrong one for an interactive verb,
// because probe COST is geometry: 96 builds is 0.2 s on a box and could be
// seconds on a dense import.
//
// So the deadline lives HERE, at the verb, and reaches the search through the
// only door the kernel opens: the cancel token it already polls between probes.
// This watch ORs two sources onto one token and remembers which one fired,
// because they mean opposite things — the caller's cancel yields `CANCELLED`
// with no result, the deadline yields a normal, truthful, looser answer.
class ProbeDeadline {
public:
  ProbeDeadline(const onecad::CancelToken &upstream, onecad::CancelToken &local,
                std::chrono::milliseconds budget) {
    const std::chrono::steady_clock::time_point deadline =
        std::chrono::steady_clock::now() + budget;
    watch_ = std::thread([this, &upstream, &local, deadline] {
      while (!done_.load(std::memory_order_relaxed)) {
        if (upstream.cancelled()) {
          local.cancel();
          return;
        }
        if (std::chrono::steady_clock::now() >= deadline) {
          expired_.store(true, std::memory_order_relaxed);
          local.cancel();
          return;
        }
        std::this_thread::sleep_for(kDeadlinePoll);
      }
    });
  }

  ~ProbeDeadline() { stop(); }

  ProbeDeadline(const ProbeDeadline &) = delete;
  ProbeDeadline &operator=(const ProbeDeadline &) = delete;

  // Joins the watch, then reports whether the DEADLINE (not the caller) tripped
  // the token. Must be joined before the answer is read: the flag is written on
  // the watch thread.
  bool expired() {
    stop();
    return expired_.load(std::memory_order_relaxed);
  }

private:
  void stop() {
    done_.store(true, std::memory_order_relaxed);
    if (watch_.joinable())
      watch_.join();
  }

  std::atomic<bool> done_{false};
  std::atomic<bool> expired_{false};
  std::thread watch_;
};

// Maximal runs of adjacent feasible probes.
std::vector<EdgeOpRangeInterval>
project_intervals(const std::map<double, kf::ProbeClassification> &window) {
  std::vector<EdgeOpRangeInterval> intervals;
  bool open = false;
  for (const auto &[value, classification] : window) {
    if (classification == kf::ProbeClassification::Success) {
      if (!open) {
        intervals.push_back({value, value});
        open = true;
      } else {
        intervals.back().upper = value;
      }
    } else {
      open = false;
    }
  }
  return intervals;
}

// Overflow policy. The FIRST and LAST intervals are kept unconditionally,
// because they carry `lowerBound` and `bestKnownMax` — the floor and the ceiling
// a consumer clamps to. Dropping either to make room for a wider interior island
// would break the two identities the contract states, and would hand the UI a
// ceiling above a value the kernel proved it can build. The remaining slots go
// to the widest of the interior intervals, restored to ascending order.
bool truncate_intervals(std::vector<EdgeOpRangeInterval> &intervals) {
  if (intervals.size() <= kMaxFeasibleIntervals)
    return false;
  const EdgeOpRangeInterval first = intervals.front();
  const EdgeOpRangeInterval last = intervals.back();
  std::vector<EdgeOpRangeInterval> interior(intervals.begin() + 1,
                                            intervals.end() - 1);
  std::stable_sort(
      interior.begin(), interior.end(),
      [](const EdgeOpRangeInterval &a, const EdgeOpRangeInterval &b) {
        return (a.upper - a.lower) > (b.upper - b.lower);
      });
  interior.resize(kMaxFeasibleIntervals - 2);
  std::sort(interior.begin(), interior.end(),
            [](const EdgeOpRangeInterval &a, const EdgeOpRangeInterval &b) {
              return a.lower < b.lower;
            });
  intervals.clear();
  intervals.push_back(first);
  intervals.insert(intervals.end(), interior.begin(), interior.end());
  intervals.push_back(last);
  return true;
}

// The confidence ladder, in order, and TOTAL: every result lands on exactly one
// rung. Each rung names what a consumer may do with the numbers, and the order
// encodes which caution wins when two apply at once.
const char *confidence_of(const EdgeOpRangeAnswer &answer, bool budget_exhausted,
                          bool deadline) {
  // Nothing built. There is no floor, so there is nothing to clamp to.
  if (!answer.has_lower_bound)
    return "none";
  // An island was observed. `bestKnownMax` alone would license a value inside
  // the gap below it, so the INTERVALS are the answer and a single ceiling is
  // not. This outranks truncation: an observed island is a fact about the
  // geometry, while a truncated search is a fact about the budget.
  if (!answer.monotonic_observed)
    return "nonMonotonic";
  // A floor, but nothing above the best success ever refused — the ceiling is
  // unproven, so only the floor may be enforced.
  if (!answer.has_proven_upper_bound)
    return "lowerOnly";
  // A bracket exists but the search stopped early, so the real frontier may sit
  // well below `provenUpperBound`. The ceiling to enforce is `bestKnownMax`, the
  // largest value actually built.
  if (budget_exhausted || deadline)
    return "coarse";
  // A complete, monotonic search with both sides of the frontier probed.
  return "bracketed";
}

double clamp_to(double value, double low, double high) {
  return std::min(std::max(value, low), high);
}

} // namespace

EdgeOpRangeAnswer
derive_edge_op_range(const std::map<double, kf::ProbeClassification> &probes,
                     bool budget_exhausted, bool deadline) {
  EdgeOpRangeAnswer out;
  for (const auto &[value, classification] : probes) {
    if (classification != kf::ProbeClassification::Success)
      continue;
    if (!out.has_lower_bound) {
      out.has_lower_bound = true;
      out.lower_bound = value;
    }
    out.has_best_known_max = true;
    out.best_known_max = value;
  }
  // The smallest infeasible probe ABOVE the best feasible one — NOT the smallest
  // refusal overall. With an island present those are different values, and the
  // smaller one is not an upper bound on anything: the kernel built something
  // above it.
  for (const auto &[value, classification] : probes) {
    if (classification == kf::ProbeClassification::Success)
      continue;
    if (out.has_best_known_max && value <= out.best_known_max)
      continue;
    out.has_proven_upper_bound = true;
    out.proven_upper_bound = value;
    break;
  }
  // Taken over the THREE-state classification, exactly as the kernel does: an
  // `Invalid` among the refusals is not the clean "works below a threshold"
  // story either, and saying so is the honest reading.
  int changes = 0;
  bool have_previous = false;
  kf::ProbeClassification previous = kf::ProbeClassification::Success;
  for (const auto &[value, classification] : probes) {
    (void)value;
    if (have_previous && previous != classification)
      ++changes;
    previous = classification;
    have_previous = true;
  }
  out.monotonic_observed = changes <= 1;
  out.feasible_intervals = project_intervals(probes);
  out.intervals_truncated = truncate_intervals(out.feasible_intervals);
  out.confidence = confidence_of(out, budget_exhausted, deadline);
  return out;
}

Envelope handle_analyze_edge_op_range(Session &session, const Envelope &req,
                                      const onecad::CancelToken &cancel) {
  const json &args = req.args;
  if (!args.contains("snapshotId") || !args["snapshotId"].is_number_unsigned())
    return fail(req, "PROTOCOL_ERROR",
                "AnalyzeEdgeOpRange: snapshotId is required");
  const std::uint64_t requested = args["snapshotId"].get<std::uint64_t>();
  const std::optional<PublishedStateSnapshot> published =
      session.published_state_at(requested);
  if (!published)
    return fail(req, "STALE_PREVIEW", "AnalyzeEdgeOpRange: stale snapshot");
  const std::string mode = get_str(args, "mode");
  if (mode != "Fillet" && mode != "Chamfer")
    return fail(req, "PROTOCOL_ERROR", "AnalyzeEdgeOpRange: unknown mode");
  if (!args.contains("pickedEdges") || !args["pickedEdges"].is_array() ||
      args["pickedEdges"].empty())
    return fail(req, "PROTOCOL_ERROR",
                "AnalyzeEdgeOpRange: pickedEdges is required");

  const ResolvedEdgePicks picks = resolve_edge_picks(
      published->bodies, published->partition, args["pickedEdges"]);
  if (picks.unresolved)
    return fail(req, "REF_UNRESOLVED",
                "AnalyzeEdgeOpRange: edge did not resolve");
  if (picks.cross_body)
    return Envelope::ok_response(
        req.id,
        refused_result(published->snapshot_id, mode, "", json::array(),
                       refusal("crossBody",
                               "edge picks span multiple bodies")));

  const bool chain = !args.contains("chainTangentEdges") ||
                     !args["chainTangentEdges"].is_boolean() ||
                     args["chainTangentEdges"].get<bool>();
  const kf::EdgeOpMode op_mode =
      mode == "Fillet" ? kf::EdgeOpMode::Fillet : kf::EdgeOpMode::Chamfer;
  const TopoDS_Shape body = published->bodies.get(picks.body_id)->geom;

  // The analyzed closure is the SAME closure `PrepareEdgeOp` freezes, computed
  // by the same call. A bound measured on a different edge set than the one the
  // commit uses would be worse than no bound at all.
  TopTools_IndexedMapOfShape edge_map;
  TopExp::MapShapes(body, TopAbs_EDGE, edge_map);
  std::vector<TopoDS_Edge> seeds;
  for (const int ordinal : picks.ordinals)
    seeds.push_back(TopoDS::Edge(edge_map(ordinal)));
  const kf::EdgeContourResult contours =
      kf::analyze_edge_contours(body, seeds, op_mode);
  if (!contours.ok)
    return Envelope::ok_response(
        req.id, refused_result(published->snapshot_id, mode, picks.body_id,
                               json::array(),
                               refusal("unsupportedEdge", contours.message,
                                       picks.ordinals)));
  std::vector<int> extra;
  std::set_difference(contours.closure_ordinals.begin(),
                      contours.closure_ordinals.end(), picks.ordinals.begin(),
                      picks.ordinals.end(), std::back_inserter(extra));
  if (!chain && !extra.empty())
    return Envelope::ok_response(
        req.id,
        refused_result(
            published->snapshot_id, mode, picks.body_id, json::array(),
            refusal("chainMismatch",
                    "chainTangentEdges:false cannot hold tangent edges fixed",
                    extra)));

  std::vector<kf::ResolvedEdge> resolved;
  for (const int ordinal : contours.closure_ordinals) {
    kf::ResolvedEdge entry;
    entry.topo_key = "e:" + std::to_string(ordinal);
    entry.body_id = picks.body_id;
    entry.edge = TopoDS::Edge(edge_map(ordinal));
    resolved.push_back(entry);
  }

  kf::ProbeBudget budget;
  budget.max_probes = kDefaultProbeBudget;
  if (args.contains("probeBudget") && args["probeBudget"].is_number_integer()) {
    budget.max_probes =
        std::min(std::max(args["probeBudget"].get<int>(), kMinProbeBudget),
                 kMaxProbeBudget);
  }

  onecad::CancelToken stop;
  kf::FilletRangeResult result;
  bool deadline = false;
  {
    ProbeDeadline watch(cancel, stop,
                        std::chrono::milliseconds(kAnalyzeEdgeOpDeadlineMs));
    result = kf::FilletRangeAnalyzer(body, resolved, op_mode, budget).analyze(&stop);
    deadline = watch.expired();
  }
  // The caller's cancel wins over the deadline, and produces NO partial answer:
  // a cancelled analysis is one the caller stopped caring about (SCHEMA §3.5).
  if (result.cancelled && cancel.cancelled())
    return fail(req, "CANCELLED", "AnalyzeEdgeOpRange: cancelled");

  // The window the answer is ABOUT. A request `range` narrows it; it is clamped
  // into the kernel's own body-derived bracket, because nothing outside that
  // bracket was ever probed and this verb reports measurements only.
  double window_min = result.lower_seed;
  double window_max = result.upper_seed;
  const json::const_iterator range = args.find("range");
  if (range != args.end() && range->is_object()) {
    if (range->contains("min") && (*range)["min"].is_number())
      window_min = clamp_to((*range)["min"].get<double>(), result.lower_seed,
                            result.upper_seed);
    if (range->contains("max") && (*range)["max"].is_number())
      window_max = clamp_to((*range)["max"].get<double>(), window_min,
                            result.upper_seed);
  }

  std::map<double, kf::ProbeClassification> window;
  for (const kf::ProbeSample &sample : result.probes) {
    if (sample.value < window_min || sample.value > window_max)
      continue;
    window.emplace(sample.value, sample.classification);
  }
  const EdgeOpRangeAnswer answer =
      derive_edge_op_range(window, result.budget_exhausted, deadline);

  // Attribution is only reported for the refusal the kernel actually described.
  // A narrowed window can select a DIFFERENT bounding refusal, and the kernel
  // carries diagnostics for one; reporting the other one's entities there would
  // be an inference, which this verb does not make.
  json limiting = json::array();
  if (answer.has_proven_upper_bound && result.has_proven_upper_bound &&
      answer.proven_upper_bound == result.proven_upper_bound) {
    for (const std::string &key : result.limiting.edge_topo_keys) {
      if (limiting.size() >= kMaxLimitingEntities)
        break;
      limiting.push_back({{"topoKey", key}, {"kind", "edge"}});
    }
  }

  json intervals = json::array();
  for (const EdgeOpRangeInterval &interval : answer.feasible_intervals)
    intervals.push_back({{"lower", interval.lower}, {"upper", interval.upper}});

  const char *stopped = deadline                  ? "deadline"
                        : result.budget_exhausted ? "budgetExhausted"
                                                  : "converged";
  WLOG_INFO("analyze-edge-op-range: mode=%s edges=%zu probes=%d stopped=%s "
            "confidence=%s",
            mode.c_str(), resolved.size(), result.probes_used, stopped,
            answer.confidence.c_str());

  return Envelope::ok_response(
      req.id,
      json{{"snapshotId", published->snapshot_id},
           {"mode", mode},
           {"targetBodyId", picks.body_id},
           {"edges", topo_keys(contours.closure_ordinals)},
           {"searchedRange", {{"min", window_min}, {"max", window_max}}},
           {"lowerBound",
            answer.has_lower_bound ? json(answer.lower_bound) : json(nullptr)},
           {"bestKnownMax", answer.has_best_known_max
                                ? json(answer.best_known_max)
                                : json(nullptr)},
           {"provenUpperBound", answer.has_proven_upper_bound
                                    ? json(answer.proven_upper_bound)
                                    : json(nullptr)},
           {"feasibleIntervals", std::move(intervals)},
           {"intervalsTruncated", answer.intervals_truncated},
           {"limitingEntities", std::move(limiting)},
           {"confidence", answer.confidence},
           {"monotonicObserved", answer.monotonic_observed},
           {"probesUsed", result.probes_used},
           {"budgetExhausted", result.budget_exhausted},
           {"stoppedReason", stopped},
           {"refusal", nullptr}});
}

} // namespace onecad::session
