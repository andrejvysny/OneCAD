// Checkpoint.h — the SaveCheckpoint / RestoreCheckpoint verbs (SCHEMA §7.7).
//
// A checkpoint is an atomic artifact set for a step: per-body BREP blobs (BinTools) +
// the ElementMap partition + the 3 signatures + the historyPrefixHash. SaveCheckpoint
// serializes the current session head into the resp binary tail (inline — the artifacts
// are small in V1; the §7.7 streamId/bulk shape is a documented divergence) AND retains
// the head in-session (Session::save_checkpoint) so RestoreCheckpoint can seed a base
// WITHOUT the geometry crossing the wire again (the OCW1 request path carries no
// binary). RestoreCheckpoint installs the retained step state into the RESTORED-BASE
// SLOT — NOT the head (kernel-hardening WP-H, SCHEMA §7.7): the head's snapshotId,
// historyPrefixHash and bodies do not move, so an unfenced reader between the restore
// and the plan that names the base in §7.2 `baseCheckpoint` still sees the head. Fenced
// on workerEpoch; an ABSENT checkpoint (e.g. after a worker restart) reports
// `restored:false` so Rust replays from 0 (Invariant 7 — the cache degrades to replay,
// never a wrong result). Rust persists the SaveCheckpoint bytes into the .onecad
// container for durability.
#pragma once

#include "protocol/Envelope.h"
#include "session/Session.h"

namespace onecad::io {

protocol::Envelope handle_save_checkpoint(session::Session& session, const protocol::Envelope& req);
protocol::Envelope handle_restore_checkpoint(session::Session& session,
                                             const protocol::Envelope& req);

}  // namespace onecad::io
