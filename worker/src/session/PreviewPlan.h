// PreviewPlan.h — `PreviewPlan` (SCHEMA §7.6): a non-publishing, full-replay
// dry run of an ExecutePlan-shaped op list (ONECAD-369 edit preview).
//
// The edit-preview lane. Re-editing an existing feature changes it AND every
// downstream feature, so a one-op `PreviewOp` over the head would apply the edit
// on top of the feature's own old output. This verb instead replays the whole
// candidate plan from an EMPTY base, exactly as a from-0 `ExecutePlan` would, and
// returns meshes for the bodies whose geometry differs from the head.
//
// **What makes it safe.**
//   * It touches NO session slot: no `fence_and_clone` (that reserves a snapshot
//     id and drops a parked restored base), no `store_prepared`, no accept or
//     discard. The scratch is a stack local, so a cancel, an error or a crash
//     strands nothing, and a held prepared scratch is never disturbed.
//   * It is fenced READ-ONLY: `workerEpoch` and `expectedSnapshotId` must both
//     equal the head, else `STALE_PREVIEW` (recoverable — the caller re-issues).
//   * Steps run at Tier A (`PreviewInteractive`), the same tier as `PreviewOp`.
//
// **Mesh scope.** A final body is meshed when it is absent from the head or its
// B-rep (plus health and face colours) differs from the head body with the same
// id. Everything else the caller keeps showing from the head.
#pragma once

#include "protocol/Dispatcher.h"  // HandlerContext
#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::session {

protocol::Envelope handle_preview_plan(Session& session, const protocol::Envelope& req,
                                       protocol::HandlerContext& ctx);

}  // namespace onecad::session
