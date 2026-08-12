// PlanExecutor.h — the ExecutePlan / AcceptPrepared / DiscardPrepared verbs
// (kernel lane). W-WP5: REAL OCCT ops (Extrude + Boolean, src/ops/*), ElementMap
// V2 partition deltas (src/elementmap/*), OCCT-history rebinding, opaque head
// token (prefixHashes[]), and an inline tessellate artifact (src/tess/*).
//
// The transaction machinery this proves (SCHEMA §7.2):
//   * fence the plan (D4: workerEpoch matches head AND expectedBaseHash == head
//     historyPrefixHash) → else PROTOCOL_ERROR with reconciliation detail. The
//     plan's documentRevision is a Rust-owned advisory stamp (an edit counter) —
//     NEVER fenced; it is adopted as the head documentRevision at AcceptPrepared;
//   * execute ops sequentially into a SCRATCH clone (never the live session),
//     streaming one `planStep` event per executed step (bodyEvents,
//     elementMapDelta, needsRepair, three §12 signatures, diagnostics);
//   * stop at the first failure / NeedsRepair, preparing snapshot `m−1`;
//   * end with a terminal `PlanPrepared`; publish on AcceptPrepared (atomic swap)
//     or drop on DiscardPrepared / cancel / failure.
//
// TEST HOOKS (compiled always; harmless — a Rust core never authors these opIds):
//   * opId contains "__crash"       → std::abort() mid-plan (chaos drill).
//   * opId contains "__fail"        → the step fails (OP_FAILED); stop, prepare
//                                     ≤ m−1, PlanPrepared stoppedReason "opFailed".
//   * opId contains "__needsrepair" → emit a §9-shaped NeedsRepair for the step;
//                                     stop, prepare m−1, stoppedReason "needsRepair".
//   * opId contains "__slow"        → sleep ~500 ms in 10 ms slices, polling the
//                                     cancel token (proves solver-lane liveness +
//                                     cooperative cancellation).
#pragma once

#include "ops/OpTypes.h"
#include "protocol/Dispatcher.h"  // HandlerContext
#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::session {

// Result of executing one candidate step against scratch state. Failed,
// cancelled, and NeedsRepair candidates are rolled back before this is returned.
struct CandidateResult {
    enum class Status { Ok, Failed, Unsupported, NeedsRepair, Cancelled };
    Status status = Status::Ok;
    std::string error_code;
    std::string error_message;
    std::vector<BodyEvent> body_events;
    std::vector<std::string> body_ids;
    elementmap::ElementMapDelta delta;
    std::vector<nlohmann::json> needs_repair;
    std::vector<RefBinding> ref_bindings;
    // Advisory `{severity, code, message}` entries the op surfaced, merged into
    // the step's `planStep.diagnostics[]` (SCHEMA §7.2). Survives rollback: a
    // diagnostic explains what the attempt SAW, so it is still worth reporting.
    std::vector<nlohmann::json> diagnostics;
    // Component Library P3 WP-3.1: set ONLY when `PlaceComponent`'s `mate`
    // reseated (`ops::OpOutcome::mate_placement`, see its own doc comment).
    // Echoed as `planStep.matePlacement` (SCHEMA §7.2) so Rust can persist
    // the reseat as a derived, no-undo writeback.
    std::optional<nlohmann::json> mate_placement;
};

// Execute one complete candidate step: predecessor input resolution, operation,
// NeedsRepair handling, and rollback. ExecutePlan and PreviewOp both use this.
CandidateResult execute_candidate_op(ScratchJob& job, const nlohmann::json& op,
                                     const std::string& op_id,
                                     std::string& last_sketch_id,
                                     const onecad::CancelToken& cancel);

// Stable diagnostic projection shared by ExecutePlan and PreviewOp. Op findings
// retain order; the terminal failure diagnostic is last.
nlohmann::json candidate_diagnostics(const CandidateResult& candidate);

// ExecutePlan (kernel lane): fence → execute into scratch (streaming planStep
// events via ctx.emit) → terminal PlanPrepared / PROTOCOL_ERROR / CANCELLED.
protocol::Envelope handle_execute_plan(Session& session, const protocol::Envelope& req,
                                       protocol::HandlerContext& ctx);

// AcceptPrepared (kernel lane): re-fence + atomic publish.
protocol::Envelope handle_accept_prepared(Session& session, const protocol::Envelope& req);

// DiscardPrepared (kernel lane): drop the scratch.
protocol::Envelope handle_discard_prepared(Session& session, const protocol::Envelope& req);

}  // namespace onecad::session
