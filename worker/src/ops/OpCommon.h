// OpCommon.h — shared helpers for the real OCCT op executors (W-WP5).
//
// Ports the reusable pieces of OneCAD-CPP RegenerationEngine.cpp:
//   * profile build: sketch params → LoopDetector/FaceBuilder → TopoDS_Face
//     (RegenerationEngine.cpp:1639-1667 buildFaceFromSketchRegion);
//   * planar face plane+normal (RegenerationEngine.cpp:201-219
//     planarFacePlaneAndNormal);
//   * checked boolean (Fuse/Cut/Common with IsDone + validity + cancellation)
//     (RegenerationEngine.cpp:144-199 checkedBooleanResult);
//   * scalar reader (bare number OR {value, expr?}, SCHEMA §7.3).
#pragma once

#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include <BRepBuilderAPI_MakeShape.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>

#include "modeling/BooleanMode.h"
#include "kernel/validation/ShapeAudit.h"
#include "nlohmann/json.hpp"
#include "ops/OpTypes.h"  // OpContext / OpOutcome / session::BodyEvent
#include "util/Cancel.h"

namespace onecad::ops {

// A dimensional param: bare number OR {value, expr?} object (SCHEMA §7.3).
double read_scalar(const nlohmann::json& params, const char* key, double dflt);

// As `read_scalar`, but malformed PRESENT data is an error instead of a default.
// Absence keeps legacy defaults until an operation versions its authoring contract.
bool read_scalar_strict(const nlohmann::json& params, const char* key, double dflt,
                        double& value_out, std::string& error_out);

// params.<key> as a string, or `dflt`.
std::string read_str(const nlohmann::json& o, const char* key, const std::string& dflt = "");

// As `json::value`, but malformed PRESENT boolean data is never silently coerced.
bool read_bool_strict(const nlohmann::json& params, const char* key, bool dflt,
                      bool& value_out, std::string& error_out);

// A typed array of ids. Absence yields an empty vector and `true` (the caller owns
// the "is absence legal here?" question); a PRESENT non-array, a non-string element,
// or an empty-string element is an error. Filtering a malformed element out instead
// would silently SHORTEN the array and shift every positional `inputs[]` pairing —
// the worker is an independent trust boundary and must refuse, never repair.
bool read_string_array_strict(const nlohmann::json& params, const char* key,
                              std::vector<std::string>& value_out,
                              std::string& error_out);

// Collect policy-tier evidence and classify one operation result. Callers own
// lifecycle changes only after this returns `Publishable` or `LifecycleOnly`.
kernel::validation::PublicationDecision publication_decision(
    const TopoDS_Shape& shape, const kernel::validation::PublicationPolicy& policy);

// Preview may use Tier A evidence for responsiveness, while commit/gate execution
// must retain the authoritative tier requested by the operation. Structural Body,
// BRep, volume and tolerance checks remain mandatory at every tier.
kernel::validation::PublicationTier result_validation_tier(
    const OpContext& ctx, kernel::validation::PublicationTier authoritative);

// Mutating operations must refuse an invalid modeling input before invoking OCCT.
// Import remains separate because its advisory/healing policy is versioned.
std::optional<OpOutcome> validate_modeling_input(const TopoDS_Shape& shape,
                                                 const std::string& operation,
                                                 const std::string& role);

// Body-aware trust boundary: quarantined imported geometry remains visible and
// exportable, but cannot enter any modeling operation.
std::optional<OpOutcome> validate_modeling_body(
    const session::BodyRecord& body, const std::string& operation,
    const std::string& role);

// Operation-local semantic-ref ownership preflight. The generic ladder deliberately
// accepts refs against their own named bodies; these operations instead require
// their typed sub-element refs to belong to one operated body. Returns §9-shaped
// NeedsRepair state for persisted malformed records, before any resolver fallback.
std::vector<nlohmann::json> operation_ref_ownership_repairs(
    const nlohmann::json& op, const std::string& op_id);

// Build one selectable planar-cell face from a solved Sketch op. Publication and
// lookup use the same RegionTable. Version 2 requires one exact canonical id;
// absent keeps the legacy detector and its documented first-region fallback.
std::optional<TopoDS_Face> build_profile_face(const nlohmann::json& sketch_params,
                                              const std::string& region_id,
                                              std::optional<int> region_identity_version,
                                              std::string& err);

// Compatibility overload for direct V1 callers and fixtures.
std::optional<TopoDS_Face> build_profile_face(const nlohmann::json& sketch_params,
                                              const std::string& region_id, std::string& err);

// Plane + outward normal of a planar face (normal reversed for REVERSED faces).
// false when the face is null / non-planar.
bool planar_face_plane_normal(const TopoDS_Face& face, gp_Pln& plane_out, gp_Dir& normal_out);

// Result of a checked boolean: the shape (null on failure) + the §8 error code to
// surface. `hist_out` receives the builder so the caller can apply OCCT history to
// the ElementMap partition (SCHEMA §10 ladder level 1 — builder kept alive).
struct BooleanResult {
    TopoDS_Shape shape;           // null ⇒ failed / cancelled
    std::string error_code;       // "" on success; OP_FAILED / GEOMETRY_INVALID / CANCELLED
    std::string error_message;
};

// Fuse/Cut/Common of target ⊕ tool, honoring determinism (SetRunParallel) +
// occtOptions (fuzzyValue/useOBB) + the cancel token (via CancelProgress). The
// builder is heap-owned and returned in `builder_out` (kept alive for history).
// Mirrors RegenerationEngine.cpp checkedBooleanResult semantics (IsDone → fail,
// invalid shape → fail), plus cancellation.
BooleanResult checked_boolean(const TopoDS_Shape& target, const TopoDS_Shape& tool,
                              app::BooleanMode mode, bool parallel,
                              const nlohmann::json& occt_options, const onecad::CancelToken* cancel,
                              std::shared_ptr<BRepBuilderAPI_MakeShape>& builder_out);

// One solid of an N-body result, paired with the quantized geometric key its
// ordinal was assigned by (VF-B6 identity-tripwire evidence).
struct RankedSolid {
    TopoDS_Shape shape;
    session::RankKey key;
};

// The solids of `shape` in DETERMINISTIC order, each carrying its sort key —
// the ordinal a split's child ids (`body_<opId>:<k>`) are numbered by (SCHEMA §2,
// D1). Ordered by a quantized geometric key: (volume, centroid x, y, z, face count)
// at 1e-6 quantization, so a symmetric bisection (equal volumes) is disambiguated
// by centroid. NEVER unordered TopExp iteration. Empty when `shape` carries no
// solid.
//
// The key is RETAINED rather than recomputed inside the comparator so that
// `publish_boolean_result` can publish it (SCHEMA §7.2 `bodyEvents[].rankKey`):
// the ordinal is a GEOMETRIC rank, not lineage, so Rust needs the rank key to
// notice a parametric edit flipping which solid `:<k>` names.
std::vector<RankedSolid> ranked_solids(const TopoDS_Shape& shape);

// [`ranked_solids`] without the keys — the shape-only form the callers that just
// need the deterministic order (StepRead, Extrude/Hole emptiness probes) use.
std::vector<TopoDS_Shape> ordered_solids(const TopoDS_Shape& shape);

enum class BooleanPublishResult { Published, Empty };

// Publish a boolean / boolean-mode-Cut result into the scratch as the successor of
// `target_id`. A SINGLE-solid result MODIFIES `target_id` in place (BodyId preserved
// — corpus invariant — + OCCT history applied to its partition). A MULTI-solid
// result SPLITS: `target_id` is Deleted and each solid becomes a deterministic child
// `body_<opId>:<k>` (Created, ordered by `ordered_solids`; SCHEMA §2 split minting,
// D1). Fills `out` (body_events, body_ids, delta). `builder` may be null (no history
// applied). On a split, the parent's referenced-element partition entries are dropped
// (a rebuildable ID-on-demand cache; a later ref re-mints against a child or
// NeedsRepairs) — no confident 1:1 child assignment exists.
//
// An empty result is deliberately NOT published. The caller chooses the operation's
// lifecycle policy before mutating any body: standalone Boolean and Revolve refuse,
// while Extrude retains its separately specified deletion behavior.
BooleanPublishResult publish_boolean_result(OpContext& ctx, const std::string& op_id,
                                            const std::string& target_id,
                                            const TopoDS_Shape& result,
                                            BRepBuilderAPI_MakeShape* builder,
                                            OpOutcome& out);

}  // namespace onecad::ops
