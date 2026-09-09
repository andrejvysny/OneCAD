#include "kernel/fillet/FilletRangeAnalyzer.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <utility>

#include <BRepBuilderAPI_MakeShape.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepGProp.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>

#include "kernel/fillet/FilletBuilder.h"
#include "kernel/fillet/FilletSemanticChecks.h"
#include "kernel/validation/GeometryPrecision.h"
#include "kernel/validation/ShapeAudit.h"
#include "util/Log.h"

namespace onecad::kernel::fillet {

namespace {

namespace validation = onecad::kernel::validation;

// ── The ported search constants ──────────────────────────────────────────────
// `search.rs` reads these off a case-authored `SearchSpec`. There is no case
// file here, so each one is either derived from the body (the bracket) or fixed
// with its reason stated.

// Geometric ladder ratio for the sample-growth stage. `search.rs` cases use 2.0
// over a bracket spanning less than one decade; the bracket derived below spans
// FOUR (1e-3 mm to a body diagonal), where 2.0 costs 15 samples and then 98
// sweep points — the whole budget before the search has started. 4.0 covers the
// same bracket in 9.
//
// It is also exactly representable, which is load-bearing: every probe value in
// every stage is then produced by IEEE-754 +, -, *, / and `sqrt` alone, with no
// libm transcendental anywhere. The probe SEQUENCE is therefore identical on
// macOS and Linux, which is what lets the bounds below be pinned as literals.
constexpr double kGrowthFactor = 4.0;

// `for step in 1..8` — seven interior points per bracket. Ported verbatim. This
// stage is what makes a success island above a failure region observable; a
// bisection-only search would find one frontier and report a clean interval that
// is not true.
constexpr int kSweepDivisions = 8;

// Ported verbatim. Absolute precision is NOT ported: `search.rs` uses 1e-6 mm,
// which for a radius means spending probes to separate two values the product
// has already declared identical. The absolute floor here is
// `authoring_resolution()` (1e-3 mm) — see `precision_`.
constexpr double kRelativePrecision = 1.0e-6;

// Ported verbatim, including the two entries that will normally be skipped: at a
// 5 mm centre, 1e-6 and 1e-8 relative are 5e-6 and 5e-8 mm, far below anything
// this kernel claims to resolve, so `probe_offsets` drops an offset whose step
// lands under `precision_` rather than burning six builds per transition on it.
constexpr std::array<double, 4> kRelativeOffsets = {1.0e-2, 1.0e-4, 1.0e-6,
                                                    1.0e-8};

std::vector<std::string> contour_element_ids(const FilletAnalysis &analysis,
                                             int contour_index,
                                             const std::vector<ResolvedEdge> &edges) {
  std::vector<std::string> ids;
  for (const FilletContour &contour : analysis.contours) {
    if (contour.index != contour_index)
      continue;
    for (std::size_t index : contour.requested_edges) {
      if (index < edges.size() && !edges[index].element_id.empty())
        ids.push_back(edges[index].element_id);
    }
  }
  return ids;
}

// What the fillet builder NAMED. `failure_diagnostics` orders its output by
// OCCT's faulty-contour list, so the front entry is a deterministic choice.
LimitingEvidence fillet_evidence(double value, const FilletBuildResult &built,
                                 const std::vector<ResolvedEdge> &edges) {
  LimitingEvidence evidence;
  evidence.probe_value = value;
  if (built.diagnostics.empty())
    return evidence;
  const diagnostics::OperationDiagnostic &diagnostic = built.diagnostics.front();
  evidence.diagnostic_code = diagnostic.code;
  const nlohmann::json::const_iterator contour = diagnostic.evidence.find("contour");
  if (contour == diagnostic.evidence.end() || !contour->is_object())
    return evidence;
  evidence.contour_index = contour->value("index", 0);
  const nlohmann::json::const_iterator keys = contour->find("edgeTopoKeys");
  if (keys != contour->end() && keys->is_array()) {
    for (const nlohmann::json &key : *keys) {
      if (key.is_string())
        evidence.edge_topo_keys.push_back(key.get<std::string>());
    }
  }
  evidence.element_ids =
      contour_element_ids(built.analysis, evidence.contour_index, edges);
  return evidence;
}

// SCHEMA §7.6 remnant floor (WP-G). A blend that BUILDS and passes Tier B can
// still leave a face or an edge below the authoring resolution — a 0.5 µm
// sliver the publication policy's micro-topology counters do not catch at this
// tier. Steering `bestKnownMax` onto such a radius offers the user a value that
// produces garbage, so the probe is a NON-success (finding fillet-chamfer-2).
//
// Faces are measured by AREA against `res²` and edges by LENGTH against `res`,
// exactly as the rule is written, and ONLY over the faces the probe generated
// or modified (see `touched_faces`). Degenerate edges are excluded: they carry
// zero length by construction and are legal topology (a sphere pole), so
// counting them would call every spherical result a remnant.
struct RemnantFinding {
  bool hit = false;
  std::string kind;
  double value = 0.0;
};

// A remnant probe has no OCCT diagnostic to quote — OCCT was HAPPY with it, and
// the refusal is entirely OneCAD's resolution floor. Its attribution is
// therefore the REQUESTED closure itself: those are the edges whose blend left
// the sub-resolution leftover, and without them a remnant-bounded run reports
// `limitingEntities: []` and tells the user nothing about which edge to change.
LimitingEvidence remnant_evidence(double value, const char *code,
                                  const std::vector<ResolvedEdge> &edges) {
  LimitingEvidence evidence;
  evidence.probe_value = value;
  evidence.diagnostic_code = code;
  for (const ResolvedEdge &edge : edges) {
    if (!edge.topo_key.empty())
      evidence.edge_topo_keys.push_back(edge.topo_key);
    if (!edge.element_id.empty())
      evidence.element_ids.push_back(edge.element_id);
  }
  return evidence;
}

// The faces this probe GENERATED or MODIFIED, filtered to the output. Scanning
// the whole result instead is a defect, not a shortcut: a body that already
// carries a sub-resolution edge — an imported sliver, a near-coincident
// boolean's leftover 25 mm away — would make EVERY probe a non-success and the
// verb would report `confidence: none` on geometry whose range is perfectly
// well defined (measured: a box fused to a box 5e-4 mm taller, 58 probes, 0
// successes). A pre-existing sliver is the INPUT's property; only what the
// blend itself left behind can bound the blend's range.
std::vector<TopoDS_Face> touched_faces(BRepBuilderAPI_MakeShape &builder,
                                       const TopoDS_Shape &input,
                                       const TopoDS_Shape &output,
                                       const std::vector<ResolvedEdge> &edges) {
  std::vector<TopoDS_Face> faces;
  if (input.IsNull() || output.IsNull())
    return faces;
  TopTools_IndexedMapOfShape output_faces;
  TopExp::MapShapes(output, TopAbs_FACE, output_faces);
  const auto keep = [&](const TopoDS_Shape &candidate) {
    if (candidate.IsNull() || candidate.ShapeType() != TopAbs_FACE ||
        output_faces.FindIndex(candidate) == 0)
      return;
    for (const TopoDS_Face &known : faces) {
      if (known.IsSame(candidate))
        return;
    }
    faces.push_back(TopoDS::Face(candidate));
  };
  TopTools_IndexedDataMapOfShapeListOfShape input_edge_faces;
  TopExp::MapShapesAndAncestors(input, TopAbs_EDGE, TopAbs_FACE,
                                input_edge_faces);
  for (const ResolvedEdge &requested : edges) {
    for (const TopoDS_Shape &generated : builder.Generated(requested.edge))
      keep(generated);
    const int index = input_edge_faces.FindIndex(requested.edge);
    if (index == 0)
      continue;
    for (const TopoDS_Shape &support : input_edge_faces(index)) {
      for (const TopoDS_Shape &modified : builder.Modified(support))
        keep(modified);
      keep(support);
    }
  }
  return faces;
}

RemnantFinding find_remnant(const std::vector<TopoDS_Face> &faces,
                            double resolution) {
  RemnantFinding out;
  if (faces.empty() || !(resolution > 0.0))
    return out;
  const double area_floor = resolution * resolution;
  double smallest_area = -1.0;
  for (const TopoDS_Face &face : faces) {
    GProp_GProps properties;
    BRepGProp::SurfaceProperties(face, properties);
    const double area = properties.Mass();
    if (area < area_floor && (smallest_area < 0.0 || area < smallest_area))
      smallest_area = area;
  }
  if (smallest_area >= 0.0)
    return {true, "face", smallest_area};

  TopTools_IndexedMapOfShape edges;
  for (const TopoDS_Face &face : faces)
    TopExp::MapShapes(face, TopAbs_EDGE, edges);
  double smallest_length = -1.0;
  for (int i = 1; i <= edges.Extent(); ++i) {
    const TopoDS_Edge edge = TopoDS::Edge(edges(i));
    if (BRep_Tool::Degenerated(edge))
      continue;
    GProp_GProps properties;
    BRepGProp::LinearProperties(edge, properties);
    const double length = properties.Mass();
    if (length < resolution &&
        (smallest_length < 0.0 || length < smallest_length))
      smallest_length = length;
  }
  if (smallest_length >= 0.0)
    return {true, "edge", smallest_length};
  return out;
}

} // namespace

const char *probe_classification_name(ProbeClassification classification) {
  switch (classification) {
  case ProbeClassification::Success:
    return "success";
  case ProbeClassification::Refusal:
    return "refusal";
  case ProbeClassification::Invalid:
    return "invalid";
  }
  return "invalid";
}

FilletRangeAnalyzer::FilletRangeAnalyzer(TopoDS_Shape body,
                                         std::vector<ResolvedEdge> edges,
                                         EdgeOpMode mode, ProbeBudget budget)
    : body_(std::move(body)), edges_(std::move(edges)), mode_(mode),
      budget_(budget) {}

FilletRangeResult FilletRangeAnalyzer::analyze(const onecad::CancelToken *cancel) {
  cancel_ = cancel;
  values_.clear();
  order_.clear();
  budget_exhausted_ = false;
  cancelled_ = false;
  seed();
  WLOG_DEBUG("fillet-range: start mode=%s edges=%zu bracket=[%g,%g] budget=%d",
             mode_ == EdgeOpMode::Fillet ? "fillet" : "chamfer", edges_.size(),
             lower_seed_, upper_seed_, budget_.max_probes);
  sample_growth();
  sweep_brackets();
  subdivide_transitions();
  probe_offsets();
  const FilletRangeResult result = summarize();
  WLOG_INFO("fillet-range: probes=%d max=%g upper=%g monotonic=%d exhausted=%d "
            "cancelled=%d",
            result.probes_used,
            result.has_best_known_max ? result.best_known_max : 0.0,
            result.has_proven_upper_bound ? result.proven_upper_bound : 0.0,
            result.monotonic_observed ? 1 : 0, result.budget_exhausted ? 1 : 0,
            result.cancelled ? 1 : 0);
  return result;
}

// The starting bracket.
//
// LOWER is `authoring_resolution()` — the smallest change OneCAD is willing to
// claim it performed, and therefore the smallest value it would let a user ask
// for. There is nothing below it worth probing.
//
// UPPER is the body's bounding-box diagonal, already measured by
// `precision_of`, so it costs no extra OCCT call. It is a deliberate
// OVER-estimate and is NOT claimed to be a true upper bound on feasibility: a
// near-flat dihedral can take a blend far larger than the part. It only has to
// be a bracket the sweep can work inside, and being generous is the safe
// direction — a bracket that stops below the real frontier reports "no refusal
// found" and tells the user nothing.
void FilletRangeAnalyzer::seed() {
  const validation::GeometryPrecisionContext precision =
      validation::precision_of(body_);
  precision_ = precision.authoring_resolution();
  lower_seed_ = precision_;
  upper_seed_ = std::max(precision.scale_diagonal, lower_seed_ * kGrowthFactor);
}

bool FilletRangeAnalyzer::observe(double value) {
  if (cancelled_ || budget_exhausted_)
    return false;
  if (values_.find(value) != values_.end())
    return true;
  if (cancel_ != nullptr && cancel_->cancelled()) {
    cancelled_ = true;
    return false;
  }
  if (static_cast<int>(values_.size()) >= budget_.max_probes) {
    budget_exhausted_ = true;
    return false;
  }
  std::optional<Observation> observation = probe(value);
  if (!observation) {
    cancelled_ = true;
    return false;
  }
  order_.push_back({value, observation->classification});
  values_.emplace(value, std::move(*observation));
  return true;
}

std::optional<FilletRangeAnalyzer::Observation>
FilletRangeAnalyzer::probe(double value) {
  return mode_ == EdgeOpMode::Fillet ? probe_fillet(value) : probe_chamfer(value);
}

// One probe = one real fillet. `FilletBuildResult` is already a clean, evidenced
// verdict — `FilletBuilder` catches OCCT's failures inside its own guards and
// converts them to diagnostics — so anything it returns is a `Refusal`, and
// `Invalid` is reserved for a throw that escaped those guards, which would be a
// defect in the builder rather than a fact about the geometry.
std::optional<FilletRangeAnalyzer::Observation>
FilletRangeAnalyzer::probe_fillet(double value) {
  try {
    FilletBuilder builder(body_, edges_, value);
    const FilletBuildResult built = builder.build(cancel_);
    if (built.cancelled)
      return std::nullopt;
    if (built.ok) {
      const RemnantFinding remnant = find_remnant(
          touched_faces(builder.history(), body_, built.shape, edges_),
          precision_);
      if (!remnant.hit)
        return Observation{ProbeClassification::Success, {}, false, std::string(), 0.0};
      return Observation{ProbeClassification::Refusal,
                         remnant_evidence(value, "FILLET_REMNANT_FLOOR", edges_),
                         true, remnant.kind, remnant.value};
    }
    return Observation{ProbeClassification::Refusal,
                       fillet_evidence(value, built, edges_), false,
                       std::string(), 0.0};
  } catch (...) {
    LimitingEvidence evidence;
    evidence.probe_value = value;
    evidence.diagnostic_code = "FILLET_PROBE_THREW";
    return Observation{ProbeClassification::Invalid, std::move(evidence), false,
                       std::string(), 0.0};
  }
}

// The equal-leg chamfer oracle.
//
// It reproduces `FilletChamferOp`'s equal-leg branch — same `Add(d, d, edge,
// faces.First())`, same `IsDone()` gate, same TierB `single_solid_policy`
// publication decision — rather than calling it. `ops/` may depend on `kernel/`
// and never the reverse, and the production path lives in an anonymous namespace
// inside the op with an `OpContext` this analyzer has no business constructing.
// The consequence is real and worth stating: if that branch changes, this must
// change with it, or the range reported here stops matching the op that will
// consume the value.
std::optional<FilletRangeAnalyzer::Observation>
FilletRangeAnalyzer::probe_chamfer(double value) {
  LimitingEvidence evidence;
  evidence.probe_value = value;
  if (!valid_constant_radius(value)) {
    evidence.diagnostic_code = "CHAMFER_DISTANCE_INVALID";
    return Observation{ProbeClassification::Refusal, std::move(evidence), false,
                       std::string(), 0.0};
  }
  try {
    TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
    TopExp::MapShapesAndAncestors(body_, TopAbs_EDGE, TopAbs_FACE, edge_faces);
    BRepFilletAPI_MakeChamfer chamfer(body_);
    std::size_t added = 0;
    for (const ResolvedEdge &requested : edges_) {
      const int index = edge_faces.FindIndex(requested.edge);
      if (index == 0 || edge_faces(index).IsEmpty())
        continue;
      chamfer.Add(value, value, requested.edge,
                  TopoDS::Face(edge_faces(index).First()));
      ++added;
    }
    if (added == 0) {
      evidence.diagnostic_code = "CHAMFER_NO_VALID_EDGES";
      return Observation{ProbeClassification::Refusal, std::move(evidence), false,
                       std::string(), 0.0};
    }
    chamfer.Build();
    if (!chamfer.IsDone()) {
      evidence.diagnostic_code = "CHAMFER_BUILD_FAILED";
      return Observation{ProbeClassification::Refusal, std::move(evidence), false,
                       std::string(), 0.0};
    }
    const validation::PublicationDecision decision =
        validation::evaluate_publication_policy(
            validation::collect_shape_evidence(chamfer.Shape(),
                                               validation::PublicationTier::TierB),
            validation::single_solid_policy("Chamfer",
                                            validation::PublicationTier::TierB));
    if (!decision.publishable()) {
      evidence.diagnostic_code = decision.reason_code.empty()
                                     ? std::string("CHAMFER_NOT_PUBLISHABLE")
                                     : decision.reason_code;
      return Observation{ProbeClassification::Refusal, std::move(evidence), false,
                       std::string(), 0.0};
    }
    const RemnantFinding remnant = find_remnant(
        touched_faces(chamfer, body_, chamfer.Shape(), edges_), precision_);
    if (remnant.hit) {
      return Observation{ProbeClassification::Refusal,
                         remnant_evidence(value, "CHAMFER_REMNANT_FLOOR", edges_),
                         true, remnant.kind, remnant.value};
    }
    return Observation{ProbeClassification::Success, {}, false, std::string(), 0.0};
  } catch (...) {
    evidence.diagnostic_code = "CHAMFER_PROBE_THREW";
    return Observation{ProbeClassification::Invalid, std::move(evidence), false,
                       std::string(), 0.0};
  }
}

// Stage 1 — `sample_growth`. A geometric ladder from the lower seed to the upper
// seed, capped at the seed exactly so the endpoint carries no accumulated
// rounding.
void FilletRangeAnalyzer::sample_growth() {
  double value = lower_seed_;
  while (true) {
    if (!observe(value))
      return;
    if (value >= upper_seed_)
      return;
    value = std::min(value * kGrowthFactor, upper_seed_);
  }
}

// Stage 2 — `sweep_brackets`. Seven interior points in every ladder bracket. A
// bracket whose step lands under `precision_` is skipped whole: those probes
// would separate values the product treats as the same value.
void FilletRangeAnalyzer::sweep_brackets() {
  std::vector<double> keys;
  keys.reserve(values_.size());
  for (const auto &entry : values_)
    keys.push_back(entry.first);
  for (std::size_t i = 1; i < keys.size(); ++i) {
    const double lower = keys[i - 1];
    const double upper = keys[i];
    const double span = upper - lower;
    if (span / kSweepDivisions < precision_)
      continue;
    for (int step = 1; step < kSweepDivisions; ++step) {
      if (!observe(lower + span * step / kSweepDivisions))
        return;
    }
  }
}

// Stage 3 — `subdivide_transitions`. Bisect the widest disagreeing bracket until
// every one of them is under precision, holding back exactly the budget stage 4
// will need.
void FilletRangeAnalyzer::subdivide_transitions() {
  while (const std::optional<std::pair<double, double>> interval = widest_interval()) {
    const std::size_t reserve = transitions().size() * kRelativeOffsets.size() * 2;
    if (values_.size() + reserve >= static_cast<std::size_t>(budget_.max_probes))
      return;
    if (!observe((interval->first + interval->second) * 0.5))
      return;
  }
}

std::optional<std::pair<double, double>>
FilletRangeAnalyzer::widest_interval() const {
  std::optional<std::pair<double, double>> best;
  double best_width = 0.0;
  for (auto it = values_.begin(); it != values_.end(); ++it) {
    auto next = std::next(it);
    if (next == values_.end())
      break;
    if (it->second.classification == next->second.classification)
      continue;
    const double width = next->first - it->first;
    const double precision =
        std::max(precision_, kRelativePrecision * std::abs(next->first));
    if (width <= precision || width <= best_width)
      continue;
    best_width = width;
    best = std::make_pair(it->first, next->first);
  }
  return best;
}

// Stage 4 — `probe_offsets`. Relative offsets either side of each frontier, to
// catch a boundary that is not where bisection left it.
void FilletRangeAnalyzer::probe_offsets() {
  for (const RangeTransition &transition : transitions()) {
    const double center = (transition.lower + transition.upper) * 0.5;
    for (double offset : kRelativeOffsets) {
      if (center * offset < precision_)
        continue;
      for (double sign : {-1.0, 1.0}) {
        const double value = center * (1.0 + sign * offset);
        if (value <= 0.0 || value > upper_seed_)
          continue;
        if (!observe(value))
          return;
      }
    }
  }
}

std::vector<RangeTransition> FilletRangeAnalyzer::transitions() const {
  std::vector<RangeTransition> out;
  for (auto it = values_.begin(); it != values_.end(); ++it) {
    auto next = std::next(it);
    if (next == values_.end())
      break;
    if (it->second.classification == next->second.classification)
      continue;
    out.push_back({it->first, next->first, it->second.classification,
                   next->second.classification});
  }
  return out;
}

FilletRangeResult FilletRangeAnalyzer::summarize() const {
  FilletRangeResult result;
  result.mode = mode_;
  result.lower_seed = lower_seed_;
  result.upper_seed = upper_seed_;
  result.probes = order_;
  result.probes_used = static_cast<int>(order_.size());
  result.budget_exhausted = budget_exhausted_;
  result.cancelled = cancelled_;

  bool failure_seen = false;
  for (const auto &[value, observation] : values_) {
    switch (observation.classification) {
    case ProbeClassification::Success:
      ++result.success_probes;
      break;
    case ProbeClassification::Refusal:
      ++result.refusal_probes;
      break;
    case ProbeClassification::Invalid:
      ++result.invalid_probes;
      break;
    }
    if (observation.classification == ProbeClassification::Success) {
      if (!result.has_lower_bound) {
        result.has_lower_bound = true;
        result.lower_bound = value;
      }
      result.has_best_known_max = true;
      result.best_known_max = value;
      if (!failure_seen) {
        result.has_contiguous_success_max = true;
        result.contiguous_success_max = value;
      }
    } else {
      failure_seen = true;
    }
  }

  for (const auto &[value, observation] : values_) {
    if (observation.classification == ProbeClassification::Success)
      continue;
    if (result.has_best_known_max && value <= result.best_known_max)
      continue;
    result.has_proven_upper_bound = true;
    result.proven_upper_bound = value;
    result.limiting = observation.evidence;
    break;
  }

  // `values_` is sorted ascending, so the first remnant probe is the one at the
  // smallest radius — the value at which clean geometry stopped being available.
  for (const auto &[value, observation] : values_) {
    if (!observation.remnant)
      continue;
    result.remnant_floor_hit = true;
    result.remnant_floor_kind = observation.remnant_kind;
    result.remnant_floor_value = observation.remnant_value;
    result.remnant_floor_radius = value;
    break;
  }

  result.transitions = transitions();
  // `monotonic_observed` is taken over the THREE-state classification, exactly as
  // `search.rs` does: a Success -> Refusal -> Invalid run counts two changes and
  // reports false. That is the honest reading — an Invalid among the refusals is
  // not the clean "works below a threshold" story either.
  int changes = 0;
  bool have_previous = false;
  ProbeClassification previous = ProbeClassification::Success;
  for (const auto &[value, observation] : values_) {
    (void)value;
    if (have_previous && previous != observation.classification)
      ++changes;
    previous = observation.classification;
    have_previous = true;
  }
  result.monotonic_observed = changes <= 1;
  return result;
}

} // namespace onecad::kernel::fillet
