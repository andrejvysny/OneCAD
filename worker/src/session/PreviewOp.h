// PreviewOp.h — `PreviewOp` (SCHEMA §7.6): run ONE op against a THROWAWAY copy
// of the session head and return the resulting bodies' MESH1.
//
// This is the drag-time preview lane. Before it existed, the "exact" preview mesh
// the frontend showed during an extrude drag was synthesized in JavaScript
// (`mockMeshes.makeExtrudeBodyMesh`) — the same function the mock client uses — so
// a Cut preview never actually subtracted and Fillet/Shell/Revolve had no preview
// at all. The commit then produced something the preview had not shown.
//
// **What makes it safe.**
//   * It runs on the KERNEL lane. OCCT work is single-writer here by design; a
//     third OCCT thread would break that. (The solver lane is PlaneGCS and its
//     latest-wins coalescing is hard-keyed to `SolveDrag`, so it is not available
//     for this payload either.)
//   * It does NOT call `fence_and_clone` — that takes the fencing path and bumps
//     `snapshot_counter_`. It uses the fencing-free `bodies_copy()` /
//     `partition_copy()`, the same read-only pair the identity verbs use.
//   * It NEVER calls `store_prepared` / `AcceptPrepared` / `DiscardPrepared`. The
//     session's head bodies, partition, history hash, snapshot id, document
//     revision and epoch are all untouched, and no scratch is left behind. A
//     preview is invisible to fencing and to the plan.
//
// **Sketch profiles.** The shared candidate executor resolves a profile ONLY from
// `OpContext::sketches`, which a real plan fills from its own preceding `Sketch`
// op. A preview has no plan, so the caller names the sketch and the handler
// pre-seeds it from the session's committed `SketchStore` — the one piece of
// genuinely new plumbing here.

#pragma once

#include "protocol/Envelope.h"
#include "session/Session.h"
#include "util/Cancel.h"

namespace onecad::session {

/// Handles a `PreviewOp` request. Always returns a terminal envelope; an op that
/// fails is reported as an error, never as a partial mutation (there is nothing
/// to mutate — the copy is dropped either way).
protocol::Envelope handle_preview_op(Session& session, const protocol::Envelope& req,
                                     const onecad::CancelToken& cancel);

// Synchronous test/helper overload. Production must pass HandlerContext::cancel.
protocol::Envelope handle_preview_op(Session& session, const protocol::Envelope& req);

}  // namespace onecad::session
