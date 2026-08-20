// AnalyzeEdgeOpRange.h — the §7.6 verb behind "what radius will this edge take?"
//
// Read-only and snapshot-FENCED, like `PrepareEdgeOp` and for the same reason:
// the answer is about to gate a value the user commits, so a stale head must be
// refused (`STALE_PREVIEW`) rather than answered against different geometry.
//
// It mints nothing, creates no scratch, publishes no snapshot and carries no bin
// tail. Every number it reports came out of a REAL build run by
// `FilletRangeAnalyzer` at the AUTHORITATIVE publication tier — never the TierA
// preview downgrade, because a maximum that only the cheap tier accepts converts
// this guard into a new surprise refusal at commit.
//
// COST FENCE. Two independent stops, whichever trips first:
//   * `kDefaultProbeBudget` builds (client may move it inside
//     [`kMinProbeBudget`, `kMaxProbeBudget`]), and
//   * `kAnalyzeEdgeOpDeadlineMs` of wall time, checked BETWEEN probes.
// Both end the search NORMALLY — a truncated answer is still true, just looser,
// and `stoppedReason` says which stop ended it. Only a §3.5 `cancel` produces an
// error, and then no partial result is published at all.
#pragma once

#include <map>
#include <string>
#include <vector>

#include "kernel/fillet/FilletRangeAnalyzer.h"
#include "protocol/Envelope.h"
#include "session/Session.h"
#include "util/Cancel.h"

namespace onecad::session {

// ── The cost fence, MEASURED ────────────────────────────────────────────────
// `test_fillet_range`'s `COST` line measures one probe at 2.25 ms over the real
// success/refusal mix on a 10 mm box (Release, OCCT 8.0.1), and a COMPLETE
// analysis of that fixture at 71 probes. 96 is therefore the smallest round cap
// above what a full search needs; 128 is the ceiling a client may ask for
// (~0.5 s of builds) and 8 the floor below which the staged search cannot even
// finish its growth ladder, so a smaller number would buy a useless answer
// rather than a cheaper one.
inline constexpr int kMinProbeBudget = 8;
inline constexpr int kDefaultProbeBudget = 96;
inline constexpr int kMaxProbeBudget = 128;

// The wall-clock stop, in ms. It exists because a probe's COST is geometry, not
// arithmetic: the budget bounds the number of builds, and on a part where one
// build costs 50 ms rather than 2 ms, 96 of them is five seconds of an
// interactive gesture. Checked between probes (see the caveat below).
inline constexpr int kAnalyzeEdgeOpDeadlineMs = 1500;

// A run of adjacent probes that ALL built. Both endpoints are values that were
// actually probed — an interval is never widened to the neighbouring refusal,
// because the frontier between the two was never observed.
struct EdgeOpRangeInterval {
  double lower = 0.0;
  double upper = 0.0;
};

// The whole measured half of the wire answer, derived from ONE ascending probe
// sequence. Keeping it in one derivation is what makes
// `lower_bound <= best_known_max < proven_upper_bound` structural rather than
// asserted: all three are read off the same sequence, in one pass, by position.
struct EdgeOpRangeAnswer {
  bool has_lower_bound = false;
  double lower_bound = 0.0;
  bool has_best_known_max = false;
  double best_known_max = 0.0;
  bool has_proven_upper_bound = false;
  double proven_upper_bound = 0.0;
  bool monotonic_observed = true;
  std::vector<EdgeOpRangeInterval> feasible_intervals;
  bool intervals_truncated = false;
  // "none" | "nonMonotonic" | "lowerOnly" | "bracketed" | "coarse".
  std::string confidence;
};

// Exposed rather than kept private because the cases this must get right — a
// feasible ISLAND above a refusal region, and an interval list long enough to
// overflow — do not occur on any geometry in the fixture set. Handing it a
// sequence is the only way to test the behaviour that exists for them.
EdgeOpRangeAnswer derive_edge_op_range(
    const std::map<double, kernel::fillet::ProbeClassification> &probes,
    bool budget_exhausted, bool deadline);

// `cancel` is the §3.5 token the Dispatcher registered for this request id.
//
// CANCELLATION IS NOT INSTANT FOR CHAMFER. A fillet probe hands the token to
// `FilletBuilder`, so a long OCCT blend aborts mid-build. `BRepFilletAPI_MakeChamfer`
// has no `UserBreak` hook, so a chamfer probe can only be stopped BETWEEN
// builds; the residual exposure is exactly one uninterruptible chamfer build,
// which is the same exposure the existing chamfer commit and preview paths carry.
protocol::Envelope handle_analyze_edge_op_range(Session &session,
                                                const protocol::Envelope &req,
                                                const onecad::CancelToken &cancel);

} // namespace onecad::session
