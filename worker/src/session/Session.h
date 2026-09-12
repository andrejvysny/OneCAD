// Session.h — the worker's per-document session (W-WP4).
//
// SUPERSEDES the pre-W-WP4 `protocol/WorkerSession.h` placeholder. One session
// per document (V1). Owns:
//   * the head fencing tokens {documentRevision, workerEpoch, snapshotId} + the
//     `historyPrefixHash` (SCHEMA §7.1/§7.2), and stamps every worker frame via
//     `head_stamp()` (the Dispatcher's stamp source);
//   * the live `BodyStore` (published bodies — stub geometry, W-WP5 → TopoDS);
//   * the session-owned, mutex-guarded `SketchStore` (shared with the solver lane
//     — see SketchStore.h for the cross-lane handoff);
//   * exactly one optional `ScratchJob` (the prepared-but-unpublished plan state);
//   * the committed op-line prefix backing `historyPrefixHash` (see HistoryHash.h);
//   * an ElementMap-partition placeholder (real partitions land in W-WP5).
//
// ── Locking model (solver lane ↔ kernel lane) ────────────────────────────────
// `Session::mu_` guards the head + bodies + scratch + committed prefix. It is
// held only BRIEFLY: fence-and-clone, store-prepared, accept, discard, and each
// `head_stamp()` read. Plan OP EXECUTION runs on the kernel lane WITHOUT the lock
// (on the scratch clone), so a slow/`__slow` plan never blocks the solver lane's
// frame stamping — the solver stays responsive (test_concurrent_lanes).
//
// The `SketchStore` carries its OWN mutex (it is touched by both lanes
// independently of the head), so it is NOT guarded by `mu_`; the solver lane
// writes committed sketches, the kernel lane reads snapshots, with no head-lock
// contention. Live PlaneGCS solve state stays lane-local in SolverLane.
#pragma once

#include <cstdint>
#include <map>
#include <mutex>
#include <optional>
#include <string>
#include <vector>

#include "ops/GearOp.h"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"
#include "session/ScratchJob.h"
#include "session/SketchStore.h"

namespace onecad::session {

// The full head reported by GetWorkerHead / OpenSession (SCHEMA §7.1).
struct WorkerHead {
    std::uint64_t document_revision = 0;
    std::uint64_t worker_epoch = 0;
    std::uint64_t snapshot_id = 0;
    std::string history_prefix_hash;
    bool has_scratch = false;
    // WP-H: a RestoreCheckpoint result is parked in the restored-base slot,
    // waiting for the ExecutePlan that names it in `baseCheckpoint` (§7.2).
    bool has_restored_base = false;
};

// SCHEMA §7.2 `baseCheckpoint` — a plan's claim on the restored-base slot.
struct BaseCheckpointRef {
    std::uint64_t step_index = 0;
    std::string checkpoint_id;
};

// Outcome of fence-and-clone at ExecutePlan entry.
struct FenceOutcome {
    enum class Status { Ok, IdempotentPrepared, Error };
    Status status = Status::Error;
    protocol::ErrorInfo error;                       // when Error
    nlohmann::json idempotent_result;                // when IdempotentPrepared
    BodyStore cloned_bodies;                         // when Ok
    elementmap::ElementMapPartition cloned_partition;  // when Ok
    ResolvedInputEvidenceLedger cloned_input_evidence;  // when Ok
    TopologyOwnerLedger cloned_topology_owners;          // when Ok
    std::map<std::string, ops::GearBodyInfo> cloned_gear_bodies;  // when Ok (WP-I)
    std::uint64_t prepared_snapshot_id = 0;          // when Ok
};

// Outcome of AcceptPrepared.
struct AcceptOutcome {
    bool ok = false;
    protocol::ErrorInfo error;      // when !ok
    std::uint64_t snapshot_id = 0;
    std::uint64_t document_revision = 0;
};

// One in-session checkpoint (SCHEMA §7.7): the head state at a step, retained so a
// later incremental regen can restore it WITHOUT the geometry crossing the wire again
// (the transport carries no request-binary). Persistence to the .onecad container is
// Rust-side (SaveCheckpoint also serializes these to the resp for durability); on a
// worker restart the map is empty ⇒ RestoreCheckpoint reports restored=false ⇒ Rust
// replays from 0 (Invariant 7 — the cache degrades to replay, never a wrong result).
struct CheckpointState {
    BodyStore bodies;
    elementmap::ElementMapPartition partition;
    ResolvedInputEvidenceLedger input_evidence;
    TopologyOwnerLedger topology_owners;
    std::string history_prefix_hash;
    // SCHEMA §7.3 gear referenceability (WP-I): the plan-derived gear-body map
    // of the head this checkpoint froze. It rides the checkpoint for the same
    // reason the partition does — a restore that dropped it would let a tooth
    // face become bindable until the next publish.
    std::map<std::string, ops::GearBodyInfo> gear_bodies;
};

// Outcome of RestoreCheckpoint.
struct RestoreOutcome {
    bool restored = false;          // false ⇒ no such in-session checkpoint (replay-from-0)
    bool drift_detected = false;    // stored hash != expected (staleness)
    std::uint64_t snapshot_id = 0;  // WP-H: the UNCHANGED head (a restore never moves it)
    std::string stored_hash;        // the checkpoint's history-prefix hash
};

// The restored-base slot (SCHEMA §7.7 / §7.2, kernel-hardening WP-H): a
// checkpoint a `RestoreCheckpoint` parked for the plan that names it. It is
// distinct from the head AND from the scratch, which is the whole point — a
// restore no longer rolls the head back under an unfenced reader.
//
// Lifetime, in one place (§7.1): set by `restore_checkpoint`; SURVIVES
// `DiscardPrepared` and a failed prepare; dropped at `accept_prepared` of the
// plan that used it; dropped by any plan WITHOUT `baseCheckpoint` before that
// plan is fenced; cleared by `open()` / `reset()`.
struct RestoredBase {
    CheckpointState state;
    std::string checkpoint_id;   // as the RESTORE request named it (Rust-owned id)
    std::uint64_t step = 0;      // the checkpoint's step index
    std::string history_prefix_hash;  // == state.history_prefix_hash; the fence value
};

// Immutable published-state copy for snapshot-fenced read-only handlers. All
// fields are captured under one `Session::mu_` acquisition.
struct PublishedStateSnapshot {
    std::uint64_t snapshot_id = 0;
    BodyStore bodies;
    elementmap::ElementMapPartition partition;
};

struct ElementBindingInput {
    std::string body_id;
    std::string topo_key;
    std::string element_id;
    std::string kind;
    nlohmann::json anchor;
};

struct BoundElement {
    std::string body_id;
    std::string topo_key;
    std::string element_id;
    std::string kind;
};

struct BindElementsOutcome {
    bool ok = false;
    protocol::ErrorInfo error;
    std::vector<BoundElement> bound;
};

class Session {
public:
    Session() = default;

    // --- lifecycle (SCHEMA §7.1) ---
    // OpenSession: adopt the request's fencing tokens; reset geometry + history.
    void open(std::string document_id, std::uint64_t document_revision,
              std::uint64_t worker_epoch, std::string mode);
    // CloseSession: drop the open flag (state left as last-seen; a fresh open resets).
    void close();
    // ResetSession: drop ALL session + scratch state, increment workerEpoch, keep
    // the process alive. Returns the new epoch (SCHEMA §7.1).
    std::uint64_t reset();

    bool is_open() const;

    // --- head ---
    // The §3 frame stamp (documentRevision/workerEpoch/snapshotId); seq is filled
    // by the Dispatcher. Thread-safe; the Dispatcher's stamp source.
    protocol::Stamp head_stamp() const;
    // The full head (incl. historyPrefixHash + hasScratch) for GetWorkerHead.
    WorkerHead head() const;

    // The session-owned sketch store (self-locked; shared with the solver lane).
    SketchStore& sketches() { return sketches_; }

    // --- ExecutePlan transaction machinery ---
    // Validate fencing + reserve a prepared snapshot id + clone the base bodies /
    // committed prefix. Called at ExecutePlan entry (kernel lane) BEFORE the
    // lock-free op execution. Fencing is workerEpoch + expectedBaseHash ONLY (D4):
    // documentRevision is a Rust-owned advisory stamp and never rejects a plan.
    // D5: a from-0 plan (no base checkpoint AND expectedBaseHash == kEmptyPrefixHash)
    // is ALWAYS base-valid — the head-hash comparison is SKIPPED and the scratch is
    // cloned from an EMPTY base (full replay + wholesale publish at accept), so
    // sequential regens keep working after the head token advances. Incremental plans
    // (expectedBaseHash != the empty anchor) keep the strict head-hash fence.
    //
    // WP-H adds the FOURTH fence case: a plan carrying §7.2 `baseCheckpoint`
    // (`base` non-null) fences `expected_base_hash` against the RESTORED BASE's
    // hash instead of the head's and clones the scratch from that base. A plan
    // WITHOUT it drops any pending restored base before it is fenced.
    FenceOutcome fence_and_clone(std::uint64_t job_id, std::uint64_t document_revision,
                                 std::uint64_t worker_epoch,
                                 const std::string& expected_base_hash,
                                 const BaseCheckpointRef* base = nullptr);

    // Install the finished scratch as the (single) prepared job. `mu_`-guarded.
    void store_prepared(ScratchJob job);

    // AcceptPrepared: publish the prepared scratch atomically (swap bodies +
    // partition in, advance snapshotId, adopt the opaque head token, and ADOPT the
    // plan's documentRevision as the head — D4). Re-fences workerEpoch ONLY (a
    // restart between prepare/accept bumps the epoch). (Sketches materialized by the
    // plan are intra-plan only — the solver lane owns sketch authoring — so they are
    // not republished here.)
    AcceptOutcome accept_prepared(std::uint64_t job_id, std::uint64_t document_revision,
                                  std::uint64_t worker_epoch);

    // Legacy independently locked copies. New element-identity handlers use
    // `published_state_at` so the snapshot fence and both stores are one read.
    BodyStore bodies_copy() const;
    elementmap::ElementMapPartition partition_copy() const;
    TopologyOwnerLedger topology_owners_copy() const;
    // SCHEMA §7.3 gear referenceability (WP-I): the live head's gear-body map.
    std::map<std::string, ops::GearBodyInfo> gear_bodies_copy() const;
    std::uint64_t current_snapshot_id() const;

    // Atomically fence an optional snapshot claim and copy bodies + partition.
    // A missing claim reads the current head for legacy callers.
    std::optional<PublishedStateSnapshot> published_state_at(
        std::optional<std::uint64_t> expected_snapshot_id,
        std::uint64_t* head_snapshot_id = nullptr) const;

    // Validate the complete Rust-owned binding batch against one unchanged head,
    // then publish its partition update atomically under the same lock.
    BindElementsOutcome bind_element_ids(std::uint64_t expected_snapshot_id,
                                         const std::vector<ElementBindingInput>& bindings);

    // DiscardPrepared / cancel / failure: drop the scratch (best-effort). Returns
    // whether a scratch was dropped.
    bool discard_prepared(std::uint64_t job_id);

    bool has_scratch() const;

    // --- Checkpoints (SCHEMA §7.7) ---
    // Save the current head as an in-session checkpoint at `step`. Returns a copy of
    // the saved state (bodies + partition + hash) so the caller can serialize it for
    // Rust-side persistence. A later save at the same step supersedes.
    CheckpointState save_checkpoint(std::uint64_t step);
    // Install the in-session checkpoint at `step` into the RESTORED-BASE SLOT
    // (WP-H). The head is untouched — `snapshot_id`, `history_prefix_hash` and the
    // published bodies do not move, and `out.snapshot_id` echoes the unchanged
    // head — so an unfenced reader between the restore and the plan still sees the
    // head. `restored=false` when absent (⇒ Rust replays from 0);
    // `drift_detected=true` when the stored hash != `expected_hash` (staleness);
    // in both of those cases the slot is left exactly as it was.
    RestoreOutcome restore_checkpoint(std::uint64_t step, const std::string& expected_hash,
                                      const std::string& checkpoint_id = std::string());

    bool has_restored_base() const;

private:
    mutable std::mutex mu_;
    bool open_ = false;
    std::string document_id_;
    std::uint64_t document_revision_ = 0;
    std::uint64_t worker_epoch_ = 0;
    std::uint64_t snapshot_id_ = 0;
    std::string history_prefix_hash_;  // == kEmptyPrefixHash after open()
    std::string mode_ = "determinism";

    BodyStore bodies_;                          // live published bodies (real TopoDS_Shape)
    elementmap::ElementMapPartition partition_; // live published element-map partition
    ResolvedInputEvidenceLedger input_evidence_;
    TopologyOwnerLedger topology_owners_;
    // SCHEMA §7.3 gear referenceability (WP-I): body id → gear info for every
    // live gear body, rebuilt at each AcceptPrepared from the ACCEPTED plan.
    // `BindElementIds` consults it to refuse a tooth face by name.
    std::map<std::string, ops::GearBodyInfo> gear_bodies_;
    SketchStore sketches_;                      // self-locked, shared with solver lane
    std::optional<ScratchJob> scratch_;         // the single prepared job
    std::uint64_t snapshot_counter_ = 0;        // monotonic prepared-snapshot ids
    std::map<std::uint64_t, CheckpointState> checkpoints_;  // step → retained head (§7.7)
    // WP-H restored-base slot (§7.2/§7.7). Guarded by `mu_` like the head; read by
    // the status thread through `head()`/`has_restored_base()` (a bool, no shape).
    std::optional<RestoredBase> restored_base_;
};

// The OPTIONAL reader fence shared by `Tessellate` and the §7.8 writers
// (SCHEMA §7.6, kernel-hardening WP-H). Returns an error `resp` when
// `req.args.snapshotId` is present and does not equal the head's; nullopt when
// it is absent (the pre-WP-H live-head path, byte-identical) or matches.
//
// Callers MUST apply it before opening a file or building a mesh: §7.6 promises
// a stale read leaves the head untouched and writes NOTHING.
std::optional<protocol::Envelope> stale_snapshot_fence(const Session& session,
                                                       const protocol::Envelope& req,
                                                       const char* verb);

}  // namespace onecad::session
