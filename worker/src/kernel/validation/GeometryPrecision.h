// GeometryPrecision.h — OneCAD's owned answer to "what is geometrically
// distinguishable here?".
//
// WHY THIS EXISTS. The kernel supports a model range of roughly 0.01 mm to
// 10 000 mm — six orders of magnitude — and its geometric thresholds were
// scattered as ~40 absolute literals across `ops/` and `kernel/`. Each one was
// individually defensible; collectively there was no single place that answered
// the question above, and no way to reason about a 0.01 mm feature inside a 10 m
// model. This type is that place.
//
// ── THE MIGRATION CONTRACT — read before changing any number here ────────────
// v1 is defined to reproduce today's values BIT-FOR-BIT. That is not timidity:
// those literals define the behaviour of 133 ctest binaries, 1265 Rust tests and
// two pinned kernelbench digest baselines, so a migration that changes numbers
// and moves call sites at the same time cannot tell a rename from a regression.
// The rule is therefore:
//
//   * A call site moving to this context must observe NO numeric change. The
//     identity is asserted in `test_geometry_precision.cpp` BEFORE the literal
//     is deleted.
//   * A number that should change gets its own commit, its own red-first test,
//     and its own gate row. One behaviour change at a time, never batched with
//     renames.
//   * `kGeometryPrecisionVersion` is bumped by a behaviour change, never by a
//     rename.
//
// ── WHAT DOES NOT BELONG HERE ───────────────────────────────────────────────
// Not every small constant is a precision budget, and absorbing one that is not
// would be a regression dressed as cleanup. Deliberately excluded, each for a
// stated reason:
//
//   * ElementMap quantization (`ElementMap.h`) and the rank-key quantization in
//     `OpCommon.cpp` — identity-critical. Changing them changes element hashes.
//   * `kThroughAllFallback` (`ExtrudeOp.cpp`) and `kFaceOvershoot`
//     (`HoleTool.h`) — synthetic tool EXTENTS, not resolutions. Scaling them
//     changes the tool solid and therefore the result.
//   * `kTangentAngleTol`, `kBuildToleranceFloor` (`OffsetFaceOp.h`) and
//     `kToNextContactEpsilon` (`ExtrudeOp.cpp`) — each documents, in its own
//     comment, a non-precision reason to be a fixed number.
//   * Squared-length degeneracy guards on normalized direction vectors
//     (Mirror/Transform/Component) — dimensionless.
//
// ── UNITS ───────────────────────────────────────────────────────────────────
// Lengths are mm and angles are radians, always. There is no document unit to
// consult: the wire is fixed at mm (SCHEMA §2), and the unit a user SEES is a
// frontend presentation concern that must never reach geometry.
#pragma once

#include <TopoDS_Shape.hxx>

#include "nlohmann/json.hpp"

namespace onecad::kernel::validation {

// Bumped only by a behaviour change. See the migration contract above.
inline constexpr int kGeometryPrecisionVersion = 1;

// The smallest geometric change OneCAD is willing to claim it performed, mm.
// This is a PRODUCT decision, not an OCCT one: it is two orders above
// `Precision::Confusion()` because a change the kernel can only just represent
// is not a change it can honestly prove. Absorbed from the `kMinValue` family
// (Extrude, Fillet/Chamfer, Hole, Shell) and `kMinimumFeatureChange`.
inline constexpr double kAuthoringResolutionMm = 1.0e-3;

// Relative conditioning cost of working far from the origin. Copied verbatim
// from `BlendEvidence.cpp`, which already used exactly this term and is the
// precedent this whole type generalizes: a measurement taken at 1 km cannot be
// held to the same absolute tolerance as one taken at the origin.
//
// Deliberately tiny. At the largest supported coordinate (1e6 mm) it
// contributes 1e-8 mm, still an order UNDER `Precision::Confusion()`, so it
// cannot move any existing threshold anywhere in the supported range. That is
// what makes adding it to a "pure rename" safe.
inline constexpr double kConditioning = 1.0e-14;

// Floor for semantic measurement, mm/rad. Semantic tolerance answers "was the
// requested modelling change achieved?" and is deliberately INDEPENDENT of
// OCCT's construction tolerance — a kernel that built sloppily still has to
// answer that question strictly. Absorbed from OffsetFace's `kSemanticLengthTol`
// / `kSemanticAngularTol`.
inline constexpr double kSemanticLengthFloorMm = 1.0e-6;
inline constexpr double kSemanticAngularFloorRad = 1.0e-7;

// An edge or face narrower than this many multiples of its own tolerance cannot
// be distinguished from a degenerate one. Two points closer than 2·tol are the
// same point as far as OCCT is concerned (`Precision.hxx`, `IsEqual`), so 2 is
// the smallest factor that is not simply asserting the tolerance back.
inline constexpr double kMicroFactor = 2.0;

// The precision budget for ONE operation on ONE shape. Cheap to construct
// (a bounding box and three tolerance queries) and meant to be constructed at
// the point of use rather than threaded: see `precision_of`.
struct GeometryPrecisionContext {
  int version = kGeometryPrecisionVersion;

  // ── Measured inputs ────────────────────────────────────────────────────────
  // Bounding-box diagonal of the shape, mm. The MODEL scale.
  double scale_diagonal = 0.0;
  // Largest |corner| of the bounding box, mm. The CONDITIONING input — how far
  // from the origin the arithmetic is being done, which is a different question
  // from how big the model is.
  double coordinate_magnitude = 0.0;
  // Largest B-Rep tolerance carried by the INPUT, mm. How much uncertainty the
  // geometry arrived with, which bounds what can honestly be claimed about it.
  double input_tolerance = 0.0;
  // Magnitude of the requested change (distance, radius, thickness), mm, or 0
  // when the operation has none. The FEATURE scale, which can be orders below
  // the model scale.
  double feature_magnitude = 0.0;

  // ── Derived budgets ────────────────────────────────────────────────────────

  // Smallest change worth claiming, mm. v1: a constant.
  //
  // It does NOT scale with `scale_diagonal` in v1, on purpose. Making it scale
  // would move a refusal boundary in about fourteen operations simultaneously,
  // which is precisely the batched behaviour change the migration contract
  // forbids. It is a candidate for v2, with its own gate.
  double authoring_resolution() const;

  // Smallest length this shape can be reasoned about at, mm — the floor under
  // any construction-tolerance decision.
  double linear_resolution() const;

  // Semantic measurement budgets: did the requested change actually happen?
  double semantic_length() const;
  double semantic_angular() const;

  // Largest output B-Rep tolerance an operation may publish, mm.
  //
  // `base_tolerance` is explicit rather than implicitly `input_tolerance`
  // because the operations disagree about the right base, and that disagreement
  // is real: Extrude and Fillet grow from the input shape's tolerance, while
  // OffsetFace grows from the CONSTRUCTION tolerance it handed OCCT. Hiding
  // that behind a default would silently change one of them.
  double tolerance_ceiling(double base_tolerance, double growth, double epsilon) const;

  // Smallest publishable volume, mm³ — `authoring_resolution()` cubed, i.e. the
  // smallest cube whose side is a change worth claiming. Absorbed from
  // OffsetFace's `kMinVolume`, whose value this now derives rather than asserts.
  double minimum_volume() const;

  // Below these an edge/face is indistinguishable from degenerate, mm.
  //
  // Both are ABSOLUTE and tolerance-relative, never a ratio to the bounding-box
  // diagonal. A ratio to the global diagonal is scale-INcorrect in the way that
  // matters most here: it judges a 0.01 mm feature by the size of the 10 m part
  // it sits on. See the micro-topology notes in `ShapeAudit.cpp`.
  double micro_edge_length() const;
  double sliver_face_width() const;

  nlohmann::json to_json() const;
};

// Measure a shape. Never throws: a null shape, or one whose bounding box is
// void, yields an origin-scale context with zeroed inputs, which makes every
// derived budget fall back to its floor. That is the conservative direction —
// a missing measurement tightens the budget rather than loosening it.
GeometryPrecisionContext precision_of(const TopoDS_Shape &shape);
GeometryPrecisionContext precision_of(const TopoDS_Shape &shape, double feature_magnitude);

} // namespace onecad::kernel::validation
