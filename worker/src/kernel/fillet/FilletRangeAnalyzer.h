// FilletRangeAnalyzer.h — what radii can this edge selection actually take?
//
// WHY THIS EXISTS. Nothing in the stack can answer that question today. The
// frontend clamps a fillet radius at a 0.1 mm floor and has NO upper bound at
// all (`src/tools/preview/filletRadius.ts`), so the only way a user learns that
// 6 mm does not fit on a 10 mm box is to arm the op and watch it refuse. The
// textbook answer ("half the smallest adjacent face width") is not a bound this
// kernel is willing to publish: it is an estimate about an idealized corner, and
// the real feasibility frontier belongs to OCCT's blend walker, not to a formula.
//
// So the oracle here is a REAL BUILD. One probe is one `FilletBuilder` (or one
// equal-leg `BRepFilletAPI_MakeChamfer`) at one value, classified by the same
// TierB publication decision production uses. Nothing is inferred; every number
// this type reports is backed by a build that actually ran.
//
// ── THE ALGORITHM IS A PORT, NOT AN INVENTION ───────────────────────────────
// The staged search is `onecad-kernelbench`'s `search.rs::explore()`, ported:
// sample-growth -> sweep-brackets -> subdivide-transitions -> probe-offsets, over
// a sorted map of value -> classification, with `Transition` and
// `monotonic_observed` computed the same way. That crate stays untouched and
// remains the independent cross-check oracle for what this reports.
//
// The staging matters because feasibility is NOT known to be monotonic. The
// kernelbench suite already carries a `HiddenIsland` executor — a success island
// sitting above a failure region — precisely because "everything below the max
// works" is an assumption, not a fact. Bisection alone would find one boundary
// and lie about the rest; the bracket sweep is what makes an island observable.
// `transitions` is therefore reported in full, and `monotonic_observed` says out
// loud whether the simple story held.
//
// ── DETERMINISM ─────────────────────────────────────────────────────────────
// Same body + same edges + same kind + same budget => identical probe SEQUENCE
// and identical bounds, bit for bit. There is no randomness, and the budget caps
// probe COUNT, never wall time. This is what lets a range answer be cached
// against a snapshot and re-derived later.
#pragma once

#include <map>
#include <optional>
#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>

#include "kernel/fillet/EdgeContour.h"    // EdgeOpMode
#include "kernel/fillet/FilletAnalyzer.h" // ResolvedEdge
#include "util/Cancel.h"

namespace onecad::kernel::fillet {

// Verdict for ONE probe.
//
// `Invalid` is kept apart from `Refusal` on purpose. Both bound the feasible
// range identically — neither is a value we would let a user pick — but they say
// different things about the kernel. A `Refusal` is OCCT and OneCAD agreeing,
// through evidence, that the value does not fit. An `Invalid` is a probe that
// escaped its own guards, which is a defect report, not a geometry fact. Folding
// the second into the first would hide it.
enum class ProbeClassification { Success, Refusal, Invalid };

const char *probe_classification_name(ProbeClassification classification);

// A pair of ADJACENT probes that disagree. Nothing is claimed about the interior
// of `[lower, upper]` — only that the frontier is somewhere inside it.
struct RangeTransition {
  double lower = 0.0;
  double upper = 0.0;
  ProbeClassification from = ProbeClassification::Success;
  ProbeClassification to = ProbeClassification::Refusal;
};

// MEASURED, not chosen. On a 10 mm box with one vertical edge (Release,
// macOS 14 / OCCT 8.0.1) one `FilletBuilder` probe costs 3.94 ms when it
// succeeds and 1.30 ms when it refuses, and a complete analysis of that fixture
// runs 71 probes in 160 ms — 2.25 ms per probe over the real mix. 96 is the
// smallest round cap comfortably above the 71 a complete search needs, and it
// bounds a worst case of 96 x 3.94 ms = 0.38 s, inside the ~1 s an interactive
// analysis is allowed. `test_fillet_range` re-measures this on every run (the
// `COST` line) and reds if one probe ever exceeds 40 ms, which is the point at
// which this arithmetic stops holding and the number must be re-derived rather
// than quietly exceeded.
inline constexpr int kDefaultProbeBudget = 96;

struct ProbeBudget {
  // A cap on the number of BUILDS, never on elapsed time. A wall-clock cap would
  // make the result depend on machine load, and a range answer that changes
  // between two runs of the same document is not an answer.
  int max_probes = kDefaultProbeBudget;
};

// What the builder NAMED when it refused at the value that bounds the feasible
// range. Empty is a legitimate outcome: OCCT does not always attribute a blend
// failure to a contour, and inventing an attribution would be worse than none.
struct LimitingEvidence {
  double probe_value = 0.0;
  // The `FilletAnalyzer` diagnostic code (`FILLET_WALKING_FAILED`,
  // `FILLET_TWISTED_SURFACE`, ...) or, for chamfer, the publication reason code.
  std::string diagnostic_code;
  // OCCT contour index the failure was attributed to, 0 when unattributed.
  int contour_index = 0;
  // Snapshot-scoped `e:<ordinal>` keys of every edge on that contour — the
  // topology the blend actually could not walk.
  std::vector<std::string> edge_topo_keys;
  // ElementIds of the REQUESTED edges that seeded that contour. Rust-minted and
  // therefore the only durable half of this evidence.
  std::vector<std::string> element_ids;

  bool empty() const {
    return diagnostic_code.empty() && edge_topo_keys.empty() &&
           element_ids.empty();
  }
};

struct ProbeSample {
  double value = 0.0;
  ProbeClassification classification = ProbeClassification::Refusal;
};

struct FilletRangeResult {
  EdgeOpMode mode = EdgeOpMode::Fillet;

  // The starting bracket the search was seeded with, reported so a reader can
  // tell "no refusal exists below the diagonal" from "the search never looked".
  double lower_seed = 0.0;
  double upper_seed = 0.0;

  // Smallest value PROVEN feasible — the smallest probe that succeeded. This is
  // the floor a slider may offer. When the smallest probe (`lower_seed`, i.e.
  // `authoring_resolution()`) succeeds, this equals it.
  bool has_lower_bound = false;
  double lower_bound = 0.0;

  // Largest value below which EVERY probe succeeded — the top of the contiguous
  // feasible run starting at `lower_bound`. This is the honest "safe range"
  // number, and it is deliberately separate from `best_known_max`: when the two
  // differ, feasibility is not a single interval and `transitions` says where it
  // breaks.
  bool has_contiguous_success_max = false;
  double contiguous_success_max = 0.0;

  // Largest value that was actually built and published. Never an extrapolation.
  bool has_best_known_max = false;
  double best_known_max = 0.0;

  // Smallest probed NON-success strictly above `best_known_max`. The pair
  // (`best_known_max`, `proven_upper_bound`) is the executable definition of the
  // frontier: the first builds, the second does not.
  bool has_proven_upper_bound = false;
  double proven_upper_bound = 0.0;

  // Every adjacent disagreement, in ascending value order. Reported in full so
  // an island above `contiguous_success_max` cannot be silently averaged away.
  std::vector<RangeTransition> transitions;
  // False means the observed data does NOT fit "succeeds below a threshold,
  // refuses above it". A caller that needs a single interval must treat a false
  // here as a reason to distrust one, not as noise.
  bool monotonic_observed = true;

  // Probes in VISIT order. The determinism claim is about this sequence, not
  // merely about the sorted set it collapses to.
  std::vector<ProbeSample> probes;
  int probes_used = 0;
  int success_probes = 0;
  int refusal_probes = 0;
  int invalid_probes = 0;

  // The search stopped early because the budget ran out. Bounds are still
  // ordered and still true; they are simply looser than a full run's.
  bool budget_exhausted = false;
  // The token was set. Partial results, flagged — never an exception.
  bool cancelled = false;

  LimitingEvidence limiting;
};

// One analysis over one body and one resolved edge selection. Cheap to
// construct; all the cost is in `analyze`.
class FilletRangeAnalyzer {
public:
  FilletRangeAnalyzer(TopoDS_Shape body, std::vector<ResolvedEdge> edges,
                      EdgeOpMode mode, ProbeBudget budget = {});

  // Runs the staged search. `cancel` is polled between probes and handed to
  // `FilletBuilder` so a long OCCT build can abort mid-probe. Never throws.
  FilletRangeResult analyze(const onecad::CancelToken *cancel = nullptr);

private:
  struct Observation {
    ProbeClassification classification = ProbeClassification::Refusal;
    LimitingEvidence evidence;
  };

  // Returns false when the search must stop (budget, cancellation).
  bool observe(double value);
  std::optional<Observation> probe(double value);
  std::optional<Observation> probe_fillet(double value);
  std::optional<Observation> probe_chamfer(double value);

  void seed();
  void sample_growth();
  void sweep_brackets();
  void subdivide_transitions();
  void probe_offsets();

  std::vector<RangeTransition> transitions() const;
  std::optional<std::pair<double, double>> widest_interval() const;
  FilletRangeResult summarize() const;

  TopoDS_Shape body_;
  std::vector<ResolvedEdge> edges_;
  EdgeOpMode mode_ = EdgeOpMode::Fillet;
  ProbeBudget budget_;

  const onecad::CancelToken *cancel_ = nullptr;
  std::map<double, Observation> values_;
  std::vector<ProbeSample> order_;
  double lower_seed_ = 0.0;
  double upper_seed_ = 0.0;
  // `GeometryPrecisionContext::authoring_resolution()` for this body: the
  // smallest change OneCAD claims it can perform, and therefore the width below
  // which refining a bracket buys nothing a user could act on.
  double precision_ = 0.0;
  bool budget_exhausted_ = false;
  bool cancelled_ = false;
};

} // namespace onecad::kernel::fillet
