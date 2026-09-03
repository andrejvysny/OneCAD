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

#include <array>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include <BRepAlgoAPI_BooleanOperation.hxx>
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
// `cancel` (optional) is handed to the Tier B self-interference pass; a caller
// that passes it must treat `ctx.cancel->cancelled()` after the decision as
// CANCELLED (the evidence then carries an audit error, never a verdict).
kernel::validation::PublicationDecision publication_decision(
    const TopoDS_Shape& shape, const kernel::validation::PublicationPolicy& policy,
    const onecad::CancelToken* cancel = nullptr);

// Turn a refused `PublicationDecision` into the operation's failure outcome.
// Sets the §8 top-level code/message from the decision AND attaches the full
// §7.2 diagnostic — `{severity, code, message, stage, reasonCode, evidence}` —
// so `candidate_diagnostics` forwards it verbatim instead of synthesizing a
// bare one that drops the reason. `message_override` (empty ⇒ use the
// decision's) exists for the one caller that qualifies the text with its body
// id. Only call this on a decision that is neither Publishable nor
// LifecycleOnly.
OpOutcome publication_refusal(const kernel::validation::PublicationDecision& decision,
                              const char* stage,
                              const std::string& message_override = {});

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

// `params.regionAnchor` as a finite `[u, v]` in sketch UV (SCHEMA §7.3 "Region
// anchor"). Absence yields `nullopt` + `true` — the field is optional, and a
// producer without the region fill omits it. A PRESENT value that is not a
// two-element array of finite numbers is an error, never a silent drop: the
// anchor decides what the op may bind to, so a malformed one must refuse rather
// than fall through to the stale-id refusal and look like a different defect.
bool read_region_anchor(const nlohmann::json& params,
                        std::optional<std::array<double, 2>>& value_out,
                        std::string& error_out);

// Build one selectable planar-cell face from a solved Sketch op. Publication and
// lookup use the same RegionTable. Version 2 requires one exact canonical id;
// absent keeps the legacy detector and its documented first-region fallback.
//
// `region_anchor` (SCHEMA §7.3 "Region anchor", kernel-hardening WP-B) is
// consulted ONLY when the exact `region_id` lookup matched NO cell — never to
// break an ambiguous match, and never before the id. When it resolves to exactly
// one containing cell, that cell is bound and a `REGION_REBOUND_BY_ANCHOR`
// `warning` is appended to `diagnostics_out` (when non-null); zero or two or more
// containing cells fall through to the unchanged refusal. The stored params are
// never rewritten, so the resolution repeats deterministically on every regen.
// Both trailing parameters are optional and default to the pre-WP-B behavior, so
// an existing 4-argument call is unchanged in meaning as well as in shape.
std::optional<TopoDS_Face> build_profile_face(
    const nlohmann::json& sketch_params, const std::string& region_id,
    std::optional<int> region_identity_version, std::string& err,
    std::vector<nlohmann::json>* diagnostics_out = nullptr,
    const std::optional<std::array<double, 2>>& region_anchor = std::nullopt);

// Compatibility overload for direct V1 callers and fixtures.
std::optional<TopoDS_Face> build_profile_face(const nlohmann::json& sketch_params,
                                              const std::string& region_id, std::string& err);

// Fold the profile advisories (`SKETCH_ENTITY_DEGENERATE`, `REGION_REBOUND_BY_ANCHOR`)
// onto the op's outcome. They ride an Ok step's `planStep` diagnostics AND a
// Failed/Unsupported step's `perStepResults[].diagnostics` — a dropped
// degenerate entity is often exactly why the profile then has no closed region.
// Advisories are placed FIRST: `PlanExecutor::execute_ops` derives the step
// message from the LAST diagnostic, and reserves the last slot for the failure
// (advisory_limit 63 + 1). A Cancelled outcome carries nothing.
void attach_profile_diagnostics(OpOutcome& out, std::vector<nlohmann::json>& diagnostics);

// Plane + outward normal of a planar face (normal reversed for REVERSED faces).
// false when the face is null / non-planar.
bool planar_face_plane_normal(const TopoDS_Face& face, gp_Pln& plane_out, gp_Dir& normal_out);

// Stage `algo` as `arg ∘ tool` WITHOUT building it (WP-E). Every auxiliary
// boolean that is not a `checked_boolean` goes through here so it shares the
// two properties the head depends on: `SetNonDestructive(true)` — the BOP must
// never raise tolerances on argument TShapes it shares with the live head or the
// scratch (see `checked_boolean`) — and single-threaded execution (Invariant 5).
// The two-shape OCCT constructors build in the constructor, BEFORE any option
// can be set, which is why callers must default-construct and then `Build()`.
void stage_boolean(BRepAlgoAPI_BooleanOperation& algo, const TopoDS_Shape& arg,
                   const TopoDS_Shape& tool);

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

// The boolean RESULT POLICY shared by Boolean / Extrude / Revolve (kernel-hardening
// WP-C). Runs on the raw `checked_boolean` result BEFORE publication:
//   * Add: the result must hold exactly as many solids as the target did. OCCT's
//     Fuse of two DISJOINT solids "succeeds" as a compound of both; publishing that
//     as a D1 split would DELETE the target's BodyId and every element tracked on
//     it, so it is a named refusal (`add_disjoint_code`) with the target intact.
//   * Cut / Intersect: an empty result is a named refusal (`empty_code`), never a
//     silent `deleted` lifecycle event. A multi-solid Cut stays the D1 split.
// Returns the failure to hand back, or nullopt when the result may be published.
std::optional<OpOutcome> boolean_result_policy(app::BooleanMode mode, const TopoDS_Shape& target,
                                                const TopoDS_Shape& result, const char* op_name,
                                                const std::string& target_id,
                                                const char* add_disjoint_code,
                                                const char* empty_code,
                                                nlohmann::json evidence_extra = nlohmann::json::object());

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
