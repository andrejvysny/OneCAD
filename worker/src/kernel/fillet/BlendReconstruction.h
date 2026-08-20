// BlendReconstruction.h — SUPPRESS a recognized blend, then REBUILD it.
//
// This is the engine underneath OffsetFace DirectModeler V1 and it is deliberately
// NOT wired to any operation. V1's pipeline is suppress -> offset -> reblend; this
// module owns the first and third steps, so the middle one can be dropped in
// without the reconstruction machinery ever having been proved by an operation
// that also moves geometry. The proof that matters is the IDENTITY ROUND TRIP:
// suppress a blend and rebuild it at the same radius with NOTHING in between, and
// the body must come back. `test_blend_reconstruction.cpp` asserts exactly that,
// to 1e-9 relative volume with an identical face count. If that is red, every
// downstream result is a coincidence.
//
// ── WHY A DESTRUCTIVE PATH NEEDS THIS MUCH PROOF ────────────────────────────
// `BlendRecognizer` infers "constant section radius" from a finite sample set, and
// `AnalyticBlend.h` is explicit that an exact tangency certificate still does not
// prove a face IS a blend — a user-authored quarter-round rib certifies exactly
// like the fillet it resembles. Suppression DESTROYS the face it was handed. The
// safety argument is therefore not "we recognized it correctly"; it is that a
// false positive is only destructive if the RECONSTRUCTION DIFFERS FROM THE
// ORIGINAL. Rebuilding at the original measured radius on the same support pair
// and refusing unless the result measures as that blend makes a mis-recognized rib
// come back as the same rib — the honest outcome — and makes anything that cannot
// be rebuilt a refusal instead of a silent loss.
//
// This module carries three of WP3's five false-positive defense layers:
//
//   layer 3  TRIM/EXTENT — `certify_blend`. Both boundary edges are lines parallel
//            to the blend axis, of equal length L, and (postcondition 7 below) the
//            sharp edge suppression exposes is a NEW edge of that same length L.
//            This is what kills the partially-trimmed round, where defeaturing
//            yields a LONGER sharp edge and a naive rebuild would silently round
//            an edge the user never rounded.
//   layer 4  THE EIGHT SUPPRESSION POSTCONDITIONS — `suppress_blends`.
//   layer 5  REPRODUCTION — `reblend`.
//
// Layers 1 (`analytic_cylinder_blend`) and 2 (the raised `BlendSamplingBudget`)
// live in `AnalyticBlend.h` and `BlendEvidence.h`; `certify_blend` and `reblend`
// invoke both. `kReconstructionBudget` is the raised budget this module owns per
// `BlendSamplingBudget`'s header: a path that will destroy the face it is
// measuring pays for 33 edge samples and a 17x17 grid rather than the frozen
// {9, 5} every Fillet result and both kernelbench baselines were measured at.
//
// ── FAIL-CLOSED, WITH A NAME ────────────────────────────────────────────────
// Every result struct carries `ok` and a `reason`. `reason` is empty exactly when
// `ok` is true and is a SCREAMING_SNAKE code plus measured detail otherwise; there
// are no partial results, so a failed `SuppressionResult` carries no shape and a
// failed `ReblendResult` publishes nothing. The codes are the vocabulary WP3-C5
// maps onto the `OFFSET_FACE_SUPPRESSION_*` / `_REBLEND_*` refusal families.
#pragma once

#include <string>
#include <vector>

#include <BRepTools_History.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>

#include "kernel/fillet/AnalyticBlend.h"
#include "kernel/fillet/BlendEvidence.h"
#include "kernel/fillet/BlendRecognizer.h"

namespace onecad::kernel::fillet {

// The sampling density this module measures at, in both directions of the round
// trip. 33 samples per boundary edge and a 17x17 UV grid: 289 grid points against
// the frozen default's 25. `reblend` additionally requires >= 64 accepted samples
// and exactly 2 boundaries, so a blend whose trimmed domain is too small to answer
// densely is refused rather than passed on thin evidence.
inline constexpr BlendSamplingBudget kReconstructionBudget{33, 17};
inline constexpr int kMinimumReconstructionSamples = 64;

// One blend, CERTIFIED: layers 1-3 have all passed on it and it may be handed to
// `suppress_blends`. Produced by `certify_blend`; the struct is plain data so a
// test can also forge one, which is how the postconditions below get exercised.
//
// `radius` is the ANALYTIC cylinder radius, never the sampled mean. The face is a
// proven `GeomAbs_Cylinder` by the time this exists, so `BRepAdaptor_Surface::
// Cylinder().Radius()` is an exact statement where the sampled mean is an
// inference, and the rebuild is driven from the exact one.
struct RecognizedBlend {
  TopoDS_Face face;
  TopoDS_Face support_a;
  TopoDS_Face support_b;
  double radius = 0.0;
  BlendConvexity convexity = BlendConvexity::Unknown;
  // Layer 3. The common length of the two straight boundary edges the blend
  // shares with its supports — the extent suppression must expose and no more.
  double boundary_length = 0.0;
};

struct BlendCertification {
  bool ok = false;
  std::string reason;
  RecognizedBlend blend;
  // The raised-budget measurement taken on the ORIGINAL face, kept so a caller
  // can report what the refusal measured.
  BlendEvidence evidence;
  // The SAMPLED recognizer's verdict, published as the enum rather than left to be
  // inferred from `reason`'s `BLEND_NOT_RECOGNIZED` prefix. The three values are NOT
  // interchangeable to a caller that must decide what to do with the face:
  //
  //   Recognized  the sampled layer agrees it is a blend (later layers may still
  //               refuse it, in which case `ok` is false with a later code)
  //   NotBlend    MEASURED not a blend — non-constant section radius, or not G1
  //               tangent to its supports
  //   Ambiguous   CANNOT TELL — more than two G1 supports (every vertex-patch
  //               region of a fully-rounded body), an unmeasurable junction, or
  //               too little trimmed-domain evidence
  //
  // Collapsing `Ambiguous` into `NotBlend` would let a caller claim a face was
  // proven ordinary when the recognizer only declined to answer.
  BlendRecognitionStatus recognition_status = BlendRecognitionStatus::NotBlend;
};

// Layers 1-3 for ONE face of `body`. Runs `recognize_blend_at` (sampled
// recognition + `analytic_cylinder_blend` + convexity), re-measures at
// `kReconstructionBudget`, and proves the trim/extent property. Refusals:
//
//   BLEND_NOT_RECOGNIZED          sampled recognition refused (its reason appended)
//   BLEND_NOT_PROVED              analytic tangency is not `Proved`
//   BLEND_CONVEXITY_UNKNOWN       the face is not an orientable cylinder
//   BLEND_EVIDENCE_INSUFFICIENT   raised budget: too few samples, or not 2 boundaries
//   BLEND_BOUNDARY_NOT_SIMPLE     a support does not share exactly one edge
//   BLEND_BOUNDARY_NOT_LINEAR     a boundary edge is not a line parallel to the axis
//   BLEND_BOUNDARY_LENGTH_MISMATCH  the two boundary edges differ in length
BlendCertification certify_blend(const TopoDS_Face &face,
                                 const TopoDS_Shape &body);

// What suppression exposed for ONE blend, and everything `reblend` needs to prove
// it rebuilt THAT blend rather than something else that fits.
struct SuppressedBlendSeed {
  // The sharp edge suppression exposed: new, shared by the two support images,
  // and of the original blend's boundary length.
  TopoDS_Edge edge;
  // The two supports' images in the suppressed body. `reblend` proves the new
  // blend face is bounded by exactly these.
  TopoDS_Face support_a;
  TopoDS_Face support_b;
  // Carried from `RecognizedBlend`, so the reproduction proof compares against the
  // ORIGINAL blend rather than against whatever the rebuild happened to produce.
  double boundary_length = 0.0;
  // The radius the suppressed blend actually had. `reblend` takes its radius as a
  // parameter (one operation, one radius) and holds it to THIS, because nothing in
  // the eight suppression postconditions looks at radius: two rounds of different
  // radii and the same convexity pass all eight, and a rebuild at one radius would
  // otherwise return the other one as a different feature with `ok = true`.
  double radius = 0.0;
  BlendConvexity convexity = BlendConvexity::Unknown;
};

struct SuppressionResult {
  bool ok = false;
  std::string reason;
  TopoDS_Shape suppressed_shape;
  // Ingested from `BRepAlgoAPI_Defeaturing` over the input body's sub-shapes, in
  // the form `elementmap::ComposedHistory::add_stage` consumes.
  occ::handle<BRepTools_History> history;
  // Index-aligned with the input blends, one per blend.
  std::vector<SuppressedBlendSeed> seed_edges;
  // Measured, for the tolerance characterization and for refusal evidence.
  double maximum_tolerance = 0.0;
  double input_tolerance = 0.0;
  double volume = 0.0;
  double input_volume = 0.0;
};

// Suppress every blend in `blends` from `body` with one `BRepAlgoAPI_Defeaturing`
// pass (`SetToFillHistory(true)`, `SetRunParallel(Standard_False)` — the result of
// a destructive geometric decision must not depend on thread scheduling).
//
// EIGHT POSTCONDITIONS, each with its own reason. Nothing is returned unless all
// eight hold; there is no "succeeded with warnings".
//
//   1 SUPPRESSION_NOT_DONE          the kernel did not complete, or returned null
//   2 SUPPRESSION_NOT_PUBLISHABLE   the result is not one valid closed solid, or its
//                                   B-Rep tolerance exceeds the growth ceiling
//   3 SUPPRESSION_FACE_COUNT        face_count != input_face_count - |blends|; a
//                                   split support or an invented face is a
//                                   different operation than the one requested
//   4 SUPPRESSION_ANCESTRY_NOT_UNIQUE  some surviving face has other than exactly
//                                   one input-face ancestor (a merge or an orphan)
//   5 SUPPRESSION_BLEND_SURVIVED    a blend face is not reported deleted
//   6 SUPPRESSION_SUPPORT_SURFACE_DRIFT  a retained support's SURFACE moved. The
//                                   supports must be re-TRIMMED and not re-fitted:
//                                   same plane, same cylinder, within semantic
//                                   length. A support whose surface moved means the
//                                   kernel healed rather than defeatured
//   7 SUPPRESSION_SEED_EDGE_*       not exactly one NEW, length-matched sharp edge
//                                   shared by the two support images (layer 3)
//   8 SUPPRESSION_VOLUME_DIRECTION  the volume did not move the way the blend's
//                                   convexity demands (derivation in the .cpp)
//
// Plus `SUPPRESSION_TOLERANCE_CAP` when the result exceeds
// `kRoundTripToleranceCapMm`, which is a tighter bound than the generic growth
// budget postcondition 2 evaluates.
//
// Input validation refuses `SUPPRESSION_INPUT_INVALID` for an empty list, a null
// or foreign face, a non-positive radius, an unknown convexity, a mixed-convexity
// set, or a MIXED-RADIUS set. The last two are the same hole: postcondition 8 has
// no single expected direction for mixed convexity, and NONE of the eight reads a
// radius at all, so a mixed-radius set passes every one of them and is then
// rebuilt by a single-radius fillet that silently re-cuts one blend to the other's
// radius. Both are refused before any geometry is destroyed.
SuppressionResult suppress_blends(const TopoDS_Shape &body,
                                  const std::vector<RecognizedBlend> &blends);

struct ReblendResult {
  bool ok = false;
  std::string reason;
  TopoDS_Shape shape;
  occ::handle<BRepTools_History> history;
  // Index-aligned with `seeds`, and always the RESULT BODY's own instances so an
  // orientation-sensitive query (`blend_convexity`, an outward normal) reads the
  // side the body assigns rather than the side the history recorded.
  std::vector<TopoDS_Face> blend_faces;
  double maximum_tolerance = 0.0;
  double input_tolerance = 0.0;
  double volume = 0.0;
  // The raised-budget measurement over every rebuilt face at once.
  BlendEvidence evidence;
};

// Rebuild every seed at `radius` through `FilletBuilder`, which brings its own
// contour analysis, radius-law validation, partial-result probe, TierB publication
// decision and measured radius/tangency verification.
//
// THE REPRODUCTION PROOF (layer 5) is what runs after it, per seed:
//
//   REBLEND_FAILED                 FilletBuilder refused (its message appended)
//   REBLEND_NOT_GENERATED          the seed did not generate exactly one face that
//                                  is present in the result
//   REBLEND_FACE_NOT_UNIQUE        two seeds generated the same face
//   REBLEND_NOT_CYLINDER           the new face is not an exact `GeomAbs_Cylinder`
//   REBLEND_RADIUS                 its analytic radius is outside
//                                  `fillet_section_radius_limit(radius, ...)`
//   REBLEND_NOT_RECOGNIZED         `recognize_blend_at` does not recognize it on
//                                  the RESULT body
//   REBLEND_SUPPORTS_MISMATCH      it is not bounded by the two expected support
//                                  images
//   REBLEND_NOT_PROVED             `analytic_cylinder_blend` against those two
//                                  images is not `Proved`
//   REBLEND_CONVEXITY              the rebuilt blend's material side is not the
//                                  suppressed one's (see below)
//   REBLEND_BOUNDARY_NOT_SIMPLE    a support shares other than one edge with it
//   REBLEND_BOUNDARY_LENGTH        a new boundary edge's length differs from the
//                                  original blend's
//   REBLEND_EVIDENCE               the raised-budget measurement is thin (< 64
//                                  samples or != 2 boundaries per blend) or exceeds
//                                  the profile / tangency limits
//   REBLEND_TOLERANCE_CAP          the rebuilt body exceeds `kRoundTripToleranceCapMm`
//   REBLEND_INPUT_INVALID          a null body, an empty seed set, a seed with a
//                                  null edge or support, or a seed whose own radius
//                                  is not the radius being rebuilt
//
// WHY `REBLEND_CONVEXITY` EXISTS even though the identity round trip cannot reach
// it. A round and a fillet of the same radius on the same support pair are tangent
// in exactly the same way, so every other rung above passes on either — the analytic
// proof is deliberately blind to the material side (`AnalyticBlend.h`). With no
// offset between the two halves the sign cannot change, so this gate is dead code
// for C4 and load-bearing for C5, where an offset moves a support across the blend
// axis and can invert a corner from convex to concave. Adding it after the pipeline
// is wired would mean adding a destructive-path safety gate with no red-first test;
// adding it now means it is proven by a forged seed before anything depends on it.
//
// Any miss publishes nothing. In V1 that is what makes a mis-recognized face safe:
// it is either reconstructed identically or the whole operation refuses.
ReblendResult reblend(const TopoDS_Shape &suppressed_shape,
                      const std::vector<SuppressedBlendSeed> &seeds,
                      double radius);

// ── Tolerance characterization ──────────────────────────────────────────────
// Both kernel stages inflate B-Rep tolerance, and a cap that is raised whenever a
// case exceeds it is not a cap. `kRoundTripToleranceCapMm` is ENFORCED by both
// stages (`SUPPRESSION_TOLERANCE_CAP` / `REBLEND_TOLERANCE_CAP`), not merely
// asserted by the test: the generic publication growth budget those stages also
// evaluate lands three orders looser and would never fire. This struct is the
// measurement side, so a caller can report what a refusal measured.
struct RoundTripTolerance {
  double input = 0.0;
  double after_suppress = 0.0;
  double after_reblend = 0.0;

  // Multiple of the input tolerance the round trip ended at.
  double inflation() const;
  // What the round trip ADDED, mm. This is the quantity the cap bounds.
  double contribution() const;
  bool within_cap() const;
};

// WHAT IT BOUNDS: what ONE STAGE ADDS, not what the body ends at. That distinction
// is measured, not stylistic. `box(10)` rounded R2 arrives at
// `Precision::Confusion()` = 1.0e-7 and the concave L-prism at 1.5e-7, but the
// boss-in-wall fixture arrives at 1.0e-6 because the boolean that built it put it
// there — and its suppression is geometrically exact, adding 0.0 mm. An ABSOLUTE
// 1.0e-6 ceiling refused that body over a rounding hair in the eighth digit, which
// is a cap punishing a body for how it arrived rather than for what this module did
// to it. Bounding the contribution is the honest form.
//
// MEASURED, OCCT 8.0.1, contribution per stage:
//   convex box (1.0e-7 in) — suppress +5.0e-8, reblend +0.0, ends at 1.5e-7
//   concave L-prism (1.5e-7 in) — suppress +0.0, reblend +0.0, ends at 1.5e-7
//   boss in wall (1.0e-6 in) — suppress +0.0 (then refused on face count)
// So the observed worst contribution is 5.0e-8 and the plane-supported round trips
// end at an absolute 1.5e-7, which `test_blend_reconstruction.cpp` also pins.
//
// The cap is 1.0e-6 — 20x headroom over the worst measured contribution, and the
// same order as `kSemanticLengthFloorMm`, so a stage that spends the whole budget
// still leaves geometry measurable to the tolerance its own postconditions are
// asserted at. It is ENFORCED by both stages, and it is a CEILING, not a target: a
// case that exceeds it is a defect to characterize, never a number to raise. WP3-C5
// adds the offset stage and characterizes that stage separately rather than raising
// this one.
inline constexpr double kRoundTripToleranceCapMm = 1.0e-6;

RoundTripTolerance roundtrip_tolerance_cap(const TopoDS_Shape &input,
                                           const SuppressionResult &suppressed,
                                           const ReblendResult &reblended);

} // namespace onecad::kernel::fillet
