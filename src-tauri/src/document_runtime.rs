//! The per-document runtime — the app's **single writer**, Tauri-free.
//!
//! V1 = one open document. [`DocumentRuntime`] owns the authoritative
//! [`DocumentSession`] (edits + undo/redo), a mirror [`RegenSession`] (the geometry
//! outputs the regen [`RegenExecutor`] writes), the fencing tokens, the LRU
//! [`MeshCache`], and the latest [`ModelSnapshot`]. Every method here is a plain
//! (async) function the thin `#[tauri::command]` wrappers delegate to, so the app
//! logic is testable without a running webview (plan quality bar).
//!
//! ## Single-writer regen (driver seam) — fencing live (R-WP11)
//!
//! The app layer runs the executor in three phases so a slow worker never blocks
//! edits and revision fencing goes **live**:
//!
//! * **phase 1 (locked)** — [`begin_regen`](DocumentRuntime::begin_regen) compiles
//!   the plan, wraps the backend in an [`AdoptingEngine`] (D1 body-id adoption),
//!   captures the [`FencingCell`] tokens, and **clones** the [`RegenSession`] so the
//!   executor drives on a copy;
//! * **phase 2 (unlocked)** — [`PreparedRegen::drive`] runs
//!   [`RegenExecutor::run`](onecad_core::regen::RegenExecutor::run) over the cloned
//!   scratch with the runtime lock **released**. Its
//!   [`RevisionGate`](onecad_core::regen::RevisionGate) reads the live
//!   [`FencingCell`], so an edit that lands during worker IO advances the revision
//!   and the executor supersedes the stale prepare at accept time;
//! * **phase 3 (locked)** — [`finish_regen`](DocumentRuntime::finish_regen) commits
//!   the driven snapshot into the live session **iff** the tokens are unchanged
//!   (else reports `Superseded`), preserving single-writer for the mutation.
//!
//! [`run_regen`](DocumentRuntime::run_regen) keeps the old inline (lock-held)
//! variant for direct callers/tests. The
//! [`RegenScheduler`](onecad_core::regen::RegenScheduler) drives phase 1→3 through
//! its [`RegenDriver`](onecad_core::regen::RegenDriver) seam (wired in
//! `crate::run`); debounce/coalesce/preview-priority live in the scheduler.
//!
//! The runtime holds the backend behind `Arc<dyn `[`GeometryEngine`]`>` +
//! `Arc<dyn `[`MeshProvider`]`>`; production wires the real `WorkerManager`, with
//! [`PendingBackend`](crate::worker::PendingBackend) the no-worker fallback.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tracing::Instrument;
use uuid::Uuid;

use onecad_core::document::body::{BodyLifecycleEvent, BodyRegistry};
use onecad_core::document::element_index::ElementEntry;
use onecad_core::document::modules::{ModuleId, ModuleState};
use onecad_core::document::record::{
    ExtrudeMode, ImportSourceCodec, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{AnchorIntent, ElementKind, ElementRef, IntentQuery, PrimaryRef};
use onecad_core::document::repair::RepairItem;
use onecad_core::document::variables::Variable;
use onecad_core::document::Document;
use onecad_core::edit::session::{empty_outcome, merge_outcome, DocumentSession};
use onecad_core::edit::{CommandOutcome, EditCommand, SketchEditOp, UndoOutcome};
use onecad_core::error::DomainError;
use onecad_core::history::{DependencyGraph, StepState, Timeline};
use onecad_core::ids::{
    BodyId, DocumentId, DocumentRevision, ElementId, EntityId, JobId, RecordId, SketchId,
    SnapshotId, TopoKey, WorkerEpoch,
};
use onecad_core::io::container::{
    CacheRead, ContainerCaches, ContainerReader, ContainerWriter, LoadedContainer,
    MeshCache as MeshCacheBlob, SaveMeta, CHECKPOINTS_DIR, PREVIEW_PATH,
};
use onecad_core::io::imports::{ImportBlob, ImportBlobs, MAX_IMPORT_BLOB_BYTES};
use onecad_core::io::IoError;
use onecad_core::math::Vec2;
use onecad_core::regen::{
    mint_element_ids, AcquireRequest, BindElementIdsRequest, CancelToken, CheckpointArtifacts,
    CheckpointStore, ElementBinding, EngineError, GeometryEngine, InMemoryCheckpointStore, Lod,
    MeshKey, MeshSink, ModelSnapshot, Outcome, Pick, PlanArtifacts, PlanContext, PlanRequest,
    PolicyVersions, RefResolution, RegenExecutor, RegenPlan, RegenPlanner, RegenRequest,
    RegenSession, ResolveOutcome, ResolveRef, ResolveRequest, SnapshotPublisher, TessellateSpec,
    UnresolvedVariable,
};
use onecad_core::sketch::{CurveParams, Sketch, SketchAttachment, WorldPlane};

use crate::document_runtime::solver_lane::{Claim, LaneOwner, SolverLaneClaims};
use crate::dto::{
    default_label, feature_kind, feature_status, feature_status_message, feature_value,
    needs_repair_item_dto, op_type_name, BodyDto, BodyMeshRef, DatumDto, DocStatus, DocumentChange,
    DocumentProjection, FailedStep, FeatureDependenciesDto, FeatureDto, FinishSketchDto,
    NeedsRepairItemDto, PromotedElementDto, SketchDto, SketchHostFaceDto, SketchSessionDto,
    SketchSolveStatus, SketchStatus, SketchUpsertDto, VariableDto, GEOMETRY_SOURCE_CACHED,
    GEOMETRY_SOURCE_LIVE, GEOMETRY_SOURCE_NONE,
};
use crate::error::ApiError;
use crate::imports::{ImportWorkspace, PreparedImport};
use crate::mesh_cache::MeshCache;
use crate::worker::{lod_str, wire, AdoptingEngine, EngineClock, MeshProvider, SolverEngine};

/// The `(documentRevision, workerEpoch)` fencing tokens behind an `Arc` so the
/// regen driver's [`RevisionGate`](onecad_core::regen::RevisionGate) can read them
/// **lock-free** while slow worker IO is in flight (R-WP11).
///
/// This is what makes fencing **live**: the live regen path releases the runtime
/// lock across the worker call and drives the executor on a **cloned** scratch
/// session, so an edit that lands during the IO can acquire the lock and
/// [`bump_revision`](FencingCell::bump_revision) here — the executor's gate then
/// observes the change at accept time and supersedes the stale prepare (SCHEMA §7.2
/// fencing). Single-writer for the document is preserved: only the runtime-lock
/// holder ever mutates these tokens; reads are lock-free.
#[derive(Debug)]
pub struct FencingCell {
    revision: AtomicU64,
    epoch: AtomicU64,
}

impl FencingCell {
    fn new(epoch: u64) -> Self {
        Self {
            revision: AtomicU64::new(0),
            epoch: AtomicU64::new(epoch),
        }
    }

    /// The current `(revision, epoch)` — the executor's gate reads this.
    #[must_use]
    pub fn get(&self) -> (DocumentRevision, WorkerEpoch) {
        (
            DocumentRevision(self.revision.load(Ordering::SeqCst)),
            WorkerEpoch(self.epoch.load(Ordering::SeqCst)),
        )
    }

    fn revision(&self) -> DocumentRevision {
        DocumentRevision(self.revision.load(Ordering::SeqCst))
    }

    fn bump_revision(&self) {
        self.revision.fetch_add(1, Ordering::SeqCst);
    }

    fn set_epoch(&self, epoch: u64) {
        self.epoch.store(epoch, Ordering::SeqCst);
    }
}

/// The wall-clock split of one regen — **measurement only**. Nothing branches on
/// these numbers, no budget is asserted, and they are backend/test-facing (no DTO
/// carries them to the frontend).
///
/// * [`planner_ms`](Self::planner_ms) — phase 1: compiling the plan under the
///   runtime lock ([`begin_regen`](DocumentRuntime::begin_regen)).
/// * [`worker_ms`](Self::worker_ms) — phase 2: the worker round trips
///   ([`EngineClock::exec_ns`]).
/// * [`mesh_ms`](Self::mesh_ms) — phase 2: the `ExecutePlan` post-ops window, where
///   the worker tessellates the prepared bodies and inlines their MESH1 blobs
///   ([`EngineClock::mesh_ns`]).
///
/// Phase 1 is disjoint from phase 2, and `worker_ms + mesh_ms` is a lower bound on
/// the drive — the remainder is the executor's own Rust-side fold and publish.
/// Each is truncated to whole milliseconds, so a sub-millisecond phase reads `0`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegenTimings {
    /// Plan compilation (phase 1, locked).
    pub planner_ms: u64,
    /// Worker round trips during the drive (phase 2, unlocked).
    pub worker_ms: u64,
    /// The worker's inline tessellation window during the drive (phase 2).
    pub mesh_ms: u64,
}

/// What one regen produced, for event emission. `outcome` is the executor's
/// terminal; `changed`/`removed` drive the pull-model `document-changed` event.
#[derive(Debug)]
pub struct RegenReport {
    /// The executor terminal (Published / Superseded / EngineFailed / Cancelled / NoOp).
    pub outcome: Outcome,
    /// The document revision the regen was fenced against.
    pub revision: u64,
    /// The fencing `documentRevision` captured at [`begin_regen`](DocumentRuntime::begin_regen)
    /// — the revision this regen was PREPARED for (commit-correlation provenance,
    /// Codex MAJOR-3). Unlike [`revision`](Self::revision) (the CURRENT revision at
    /// finish, which a later edit may have advanced), this pins which edit the regen
    /// covers, so the frontend awaiter resolves the exact commit under rapid edits: a
    /// `superseded` report carries `source_revision < revision` and is ignored, while
    /// a later from-0 publish covering the commit resolves it.
    pub source_revision: u64,
    /// The published snapshot id (`0` when nothing was published). Shared by all
    /// bodies/maps/meshes of this regen (Invariant 4); the frontend forwards it to
    /// `promoteSelection` so picks resolve against the exact snapshot.
    pub snapshot_id: u64,
    /// Bodies present after the regen, with their generation-pinned mesh keys.
    pub changed: Vec<(BodyId, MeshKey)>,
    /// Bodies that were present before but are gone now.
    pub removed: Vec<BodyId>,
    /// The post-regen NeedsRepair set (empty ⇒ no repairs / repairs cleared). Lean
    /// per-item summaries for the `needs-repair` event; the panel fetches the full
    /// candidate evidence via `resolveRefs`. Populated only for a **published** regen
    /// (a superseded/failed/no-op regen leaves the live repair state unchanged).
    pub needs_repair: Vec<NeedsRepairItemDto>,
    /// Timeline records left in `StepState::Error` by a PUBLISHED regen (MODEL-HARDEN
    /// finding 1). Empty on a clean publish and on every non-published outcome. Threaded
    /// into `regen-finished` `failedSteps` so a from-0 regen that publishes sibling
    /// bodies but fails the newly-committed op is not mistaken for a commit success.
    pub failed_steps: Vec<FailedStep>,
    /// Structured diagnostics retained with their regen terminal.
    pub diagnostics: Vec<onecad_core::regen::Diagnostic>,
    /// Per-record-id, the bodies each op CREATED or MODIFIED in this PUBLISHED regen
    /// (`document-changed` wire form). Empty on non-published outcomes. Threaded into
    /// `regen-finished` `affectedBodies` for precise per-commit correlation.
    pub affected_bodies: BTreeMap<String, Vec<String>>,
    /// Where this regen's wall clock went (measurement only — see [`RegenTimings`]).
    /// Reported on every terminal, including the ones that publish nothing.
    pub timings: RegenTimings,
}

impl RegenReport {
    /// Whether this regen published a new snapshot (drives the `document-changed` +
    /// `needs-repair` emission gate).
    #[must_use]
    pub fn published(&self) -> bool {
        matches!(self.outcome, Outcome::Published(_))
    }

    /// The regen terminal as the `regen-finished` `outcome` token (`published` |
    /// `superseded` | `failed` | `cancelled` | `noop`).
    #[must_use]
    pub fn outcome_str(&self) -> &'static str {
        match self.outcome {
            Outcome::Published(_) => "published",
            Outcome::Superseded => "superseded",
            Outcome::EngineFailed(_) => "failed",
            Outcome::Cancelled => "cancelled",
            Outcome::NoOp => "noop",
        }
    }

    /// The human-facing failure message for a hard-failed regen (SCHEMA §8 — the
    /// worker's error), or `None` for any other terminal. Threaded into the
    /// `regen-finished` `message` so a correlated apply surfaces WHY it failed.
    #[must_use]
    pub fn failure_message(&self) -> Option<String> {
        match &self.outcome {
            Outcome::EngineFailed(e) => Some(e.to_string()),
            _ => None,
        }
    }

    /// The `document-changed` payload, or `None` when nothing was published.
    #[must_use]
    pub fn document_change(&self) -> Option<DocumentChange> {
        if !matches!(self.outcome, Outcome::Published(_)) {
            return None;
        }
        Some(DocumentChange {
            revision: self.revision,
            snapshot_id: self.snapshot_id,
            changed_bodies: self
                .changed
                .iter()
                .map(|(body, key)| BodyMeshRef {
                    body_id: body.to_string(),
                    mesh_key: mesh_key_string(*key),
                })
                .collect(),
            removed_bodies: self.removed.iter().map(BodyId::to_string).collect(),
        })
    }
}

/// An in-flight sketch drag gesture (SCHEMA §7.4). The `before` sketch is the
/// pre-gesture memento; pointer-up commits **one** [`EditCommand::SketchDragGesture`]
/// so the whole drag is a single undo step (plan "Solver lane in V1").
struct ActiveGesture {
    gesture_id: u64,
    sketch_id: SketchId,
    drag_point: EntityId,
    before: Sketch,
    /// Next `SolveDrag` seq (monotonic; latest-wins).
    next_seq: u64,
    /// The solver-lane claim for `sketch_id` (SP-0 D3). Acquired in
    /// [`begin_gesture`](DocumentRuntime::begin_gesture) BEFORE the upsert that
    /// opens the worker gesture, and released by `Drop` when this record is taken
    /// (end / cancel / `finish_sketch`'s dangling-gesture clear) — so there is no
    /// release site to forget. See [`solver_lane`].
    _claim: Claim,
}

/// The pre-session state captured on [`DocumentRuntime::enter_sketch`] so
/// finish/cancel can collapse every in-session granular edit into ONE net
/// undoable command (Codex-review B1 squash). `prior` is the exact sketch to
/// restore on undo of the squashed command; `undo_watermark` is the undo depth at
/// entry — the steps committed since is what gets squashed.
struct SketchSession {
    sketch_id: SketchId,
    prior: Sketch,
    undo_watermark: usize,
    /// The undo-stack eviction count at enter (finding 3). If it moves before
    /// finish/cancel, the depth-based `undo_watermark` no longer addresses the
    /// session's steps (the stack bottom shifted out past the cap), so the squash is
    /// refused wholesale — the granular steps stay (safe, noisier stack).
    evicted_at_enter: u64,
}

/// Immutable input for a read-only sketch-region query. Prepared under the
/// runtime lock, then driven without it so solver/worker IO cannot block edits.
pub struct PreparedSketchRegions {
    sketch: Sketch,
    solver: Arc<dyn SolverEngine>,
    /// The solver-lane claim for this sketch (SP-0 D3), taken under the runtime
    /// lock in [`DocumentRuntime::prepare_sketch_regions`] and released by `Drop`
    /// when [`drive`](Self::drive) consumes `self`. Holding it ACROSS the unlocked
    /// drive is the whole point: it is what a `beginGesture` landing mid-drive
    /// collides with. See [`solver_lane`].
    _claim: Claim,
}

impl PreparedSketchRegions {
    /// Syncs the authoritative sketch into the rebuildable solver cache and
    /// derives its closed regions. Document/session/undo state is untouched.
    pub async fn drive(self) -> Result<FinishSketchDto, EngineError> {
        self.solver.sketch_upsert(&self.sketch).await?;
        self.solver
            .sketch_regions(&self.sketch.id.to_string())
            .await
    }
}

/// What one [`DocumentRuntime::undo`] / [`redo`](DocumentRuntime::redo) did
/// (HISTORY-HARDEN H8).
///
/// Its existence means a step WAS reverted (the methods return `Option<RevertReport>`).
/// Checkpoint eviction already happened inside the runtime; the only thing left for
/// the caller is to enqueue [`regen`](Self::regen) if it is `Some`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevertReport {
    /// The regen to schedule for this revert — always a
    /// [`RegenRequest::RevertToEnd`] carrying an already-clamped floor, so it can
    /// never claim SCHEMA §7.2 `editedFrom` (see
    /// [`DocumentRuntime::revert_report`]). `None` ⇒ the revert moved nothing the
    /// timeline regenerates, so scheduling anything would be pure waste.
    pub regen: Option<RegenRequest>,
}

/// An admitted checkpoint mint (SCHEMA §7.7): the head step to save, plus the
/// fencing tokens the artifacts must still match when they come back.
/// [`DocumentRuntime::prepare_checkpoint`] issues one; only
/// [`DocumentRuntime::adopt_checkpoint`] consumes it.
#[derive(Debug, Clone, Copy)]
pub struct CheckpointTicket {
    step: usize,
    fencing: (DocumentRevision, WorkerEpoch),
}

impl CheckpointTicket {
    /// The head step the admitted checkpoint describes — the `SaveCheckpoint`
    /// argument. Exposed so the engine round-trip can run **off** the runtime lock
    /// between [`prepare_checkpoint`](DocumentRuntime::prepare_checkpoint) and
    /// [`adopt_checkpoint`](DocumentRuntime::adopt_checkpoint).
    #[must_use]
    pub fn step(&self) -> usize {
        self.step
    }
}

/// Everything one container write needs, detached from the [`DocumentRuntime`]
/// (VF-B7 / plan F1a).
///
/// A save is two halves: a cheap `&self` snapshot under the single-writer lock
/// ([`DocumentRuntime::build_save_payload`]) and an expensive, lock-free write
/// ([`DocumentRuntime::write_payload`]) — JSON serialization plus deflate of every
/// referenced import blob. Holding the runtime lock across the second half stalled
/// every edit for the duration of the write; a `SavePayload` is what lets the lock
/// be released in between. It is `Send + 'static`, so it can be moved onto a
/// `spawn_blocking` thread.
pub struct SavePayload {
    doc: Document,
    caches: ContainerCaches,
    imports: ImportBlobs,
    meta: SaveMeta,
}

/// Which "open paints immediately" caches a save embeds in the container.
///
/// The distinction is **explicit save vs autosave**, not a tuning knob. An
/// explicit save is a user-intended checkpoint of the work and is worth spending
/// container bytes on so the next open can paint last-saved geometry before the
/// from-0 regen finishes. An autosave fires on a timer, competes with live
/// editing, and its container is only ever read by crash recovery — which regens
/// from 0 anyway. So autosave writes [`SaveCaches::none`] and the two lanes never
/// argue about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveCaches {
    /// Embed the head snapshot's coarse meshes (`meshes/<bodyId>.coarse.mesh`),
    /// bounded by [`MAX_SAVED_MESH_BYTES`].
    pub meshes: bool,
    /// A PNG viewport capture to store as `preview.png`.
    ///
    /// `None` **carries forward** whatever preview the document already has: a
    /// save from a window that produced no capture (headless, a backgrounded
    /// webview, an older frontend) must not erase the thumbnail the start screen
    /// is showing. Only a `Some` replaces it.
    pub preview_png: Option<Vec<u8>>,
}

impl SaveCaches {
    /// Persist nothing (the autosave lane).
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// The explicit-save lane with no viewport capture: meshes on, preview carried
    /// forward.
    #[must_use]
    pub fn explicit() -> Self {
        Self {
            meshes: true,
            preview_png: None,
        }
    }

    /// Whether this lane writes ANY cache section. False only for
    /// [`none`](Self::none) — which is what keeps an autosave container free of a
    /// `preview.png` it would otherwise inherit through the carry-forward rule.
    #[must_use]
    pub fn persists_caches(&self) -> bool {
        self.meshes || self.preview_png.is_some()
    }
}

/// Budget for the mesh caches ONE save embeds (64 MiB). A document dense enough to
/// blow through this is one where the container write, not the regen, becomes the
/// thing the user waits on — and the caches are pure acceleration (Invariant 7), so
/// truncating is always sound. The bodies that fit still paint at open; the rest
/// arrive with the regen.
pub const MAX_SAVED_MESH_BYTES: usize = 64 * 1024 * 1024;

/// Budget for the mesh caches ONE open loads back (256 MiB) — matches
/// [`crate::mesh_cache::DEFAULT_BYTE_CAPACITY`], since these blobs occupy the same
/// role (resident MESH1 bytes) until the first publish retires them.
pub const MAX_LOADED_MESH_BYTES: usize = 256 * 1024 * 1024;

/// Tessellation tier for committed document geometry. Drag-time preview defaults
/// to coarse in the preview command; it must not lower the fidelity of a
/// published body or its persisted open-paint cache.
const DISPLAY_LOD: Lod = Lod::Fine;

/// How many TRANSPORT failures the DI-4 re-bind pass may retry across before it
/// goes terminal for the session (see
/// [`DocumentRuntime::note_rebind_transport_failure`]). A ladder ANSWER is
/// terminal on the first pass regardless — this bounds only the worker-error
/// path, where retrying is correct but must not become a per-publish tax.
const MAX_REBIND_TRANSPORT_FAILURES: u8 = 3;

/// One entry of the promotion cache (see [`DocumentRuntime::promoted`]).
///
/// The `descriptor` is the worker's opaque evidence for the element as it stood in
/// the snapshot the id was minted under — **evidence, never identity** (Invariant
/// 2). It is what makes the cross-generation reuse rung sound: byte-equality of
/// two descriptors is the only cheap proof that a `TopoKey` still names the same
/// geometry after a regen.
#[derive(Debug, Clone)]
struct PromotionEntry {
    element_id: ElementId,
    kind: onecad_core::document::refs::ElementKind,
    descriptor: Option<serde_json::Value>,
}

/// The per-document runtime (V1 single writer).
pub struct DocumentRuntime {
    session: DocumentSession,
    /// The regen mirror. Its timeline holds the **EFFECTIVE** records — every
    /// expression-driven `Scalar` already resolved against the document's
    /// variable table (WP-VE.1, `onecad_core::regen::variables`) — while
    /// [`session`](Self::session) keeps the stored ones verbatim. The mirror is
    /// what `begin_regen` plans (and hashes) from, what the executor's
    /// replay-from-0 fallback re-plans from, and what the wire lowering
    /// serializes; it is never saved.
    regen: RegenSession,
    /// Steps whose `Scalar` expression could not be resolved, lowest first
    /// (`onecad_core::regen::variables`). Recomputed on every mirror rebuild, so
    /// it is a pure function of (records, variables). The lowest entry is a hard
    /// execution ceiling for [`begin_regen`](Self::begin_regen): nothing at or
    /// after a broken binding may run on a stale cached value.
    unresolved_variables: BTreeMap<usize, UnresolvedVariable>,
    /// The lock-free fencing tokens (revision + worker epoch). See [`FencingCell`].
    fencing: Arc<FencingCell>,
    /// Identity of THIS runtime object, minted per construction and never reused.
    ///
    /// The `(revision, epoch)` pair fences edits WITHIN one document; it cannot fence
    /// ACROSS documents, because every fresh runtime restarts from the same origin
    /// (revision 0). Swapping the runtime slot (`new_document` / `open_document` /
    /// import / recovery) does not cancel an in-flight regen, so without this id a
    /// job prepared on the previous document can land on the new one with tokens that
    /// collide exactly, and phase 3 would commit the wrong document's geometry.
    /// [`finish_regen`](Self::finish_regen) discards any regen not minted here.
    instance: Uuid,
    title: String,
    path: Option<PathBuf>,
    dirty: bool,
    read_only: bool,
    mesh_cache: MeshCache,
    /// MESH1 blobs read back from the container at **open** — the last-saved
    /// geometry, keyed WITHOUT a generation because it belongs to no live snapshot.
    ///
    /// This is the whole point of the feature: a reopened document paints its
    /// bodies immediately instead of staring at an empty viewport for the whole
    /// from-0 worker regen. It is served only in the pre-publish window
    /// ([`get_mesh`](Self::get_mesh)) and dropped at the first
    /// [`commit_snapshot`](Self::commit_snapshot) — once real geometry exists,
    /// serving saved bytes could show a body the current timeline no longer
    /// produces.
    cached_meshes: HashMap<(BodyId, Lod), Arc<Vec<u8>>>,
    /// The document's `preview.png`, loaded at open and rewritten by an explicit
    /// save that carries a capture. Held so a capture-less save can carry it
    /// forward instead of erasing the start screen's thumbnail.
    preview_png: Option<Arc<Vec<u8>>>,
    latest_snapshot: Option<Arc<ModelSnapshot>>,
    /// The worker epoch [`latest_snapshot`](Self::latest_snapshot) was produced under.
    /// Only that worker still holds the head geometry, so it is the only one a
    /// checkpoint of the head may be asked from (VF-M2).
    head_epoch: WorkerEpoch,
    publisher: Arc<SnapshotPublisher>,
    engine: Arc<dyn GeometryEngine>,
    meshes: Arc<dyn MeshProvider>,
    solver: Arc<dyn SolverEngine>,
    occt_fingerprint: String,
    job_seq: u64,
    /// Last solver-lane `(dof, status)` per sketch — real projection dof/status
    /// (replaces the `dof:0`/`Ok` placeholders). Empty until a sketch is solved.
    sketch_solve: BTreeMap<SketchId, (u32, SketchSolveStatus)>,
    /// The active drag gesture, if the pointer is down mid-drag.
    active_gesture: Option<ActiveGesture>,
    /// Per-sketch solver-lane claims (SP-0 D3) — the RAII fence between a drag
    /// gesture and a region query on the SAME sketch. See [`solver_lane`].
    solver_lane: Arc<SolverLaneClaims>,
    /// The open sketch-edit session watermark (B1 squash), set on `enter_sketch`
    /// and consumed on `finish_sketch`/`cancel_sketch`. `None` outside a session.
    sketch_session: Option<SketchSession>,
    /// Monotonic gesture id allocator (SCHEMA §7.4 `gestureId`).
    gesture_seq: u64,
    /// Rust-owned promotion cache `(snapshot, body, topoKey) → `[`PromotionEntry`]
    /// so re-picking the same element **within one snapshot** returns the same id
    /// (Invariant 1). The worker only echoes ids it already holds; Rust owns id
    /// identity, so this map upholds the invariant across `AcquireElementIds` calls.
    ///
    /// **The `SnapshotId` in the key is the VF-M4 fix.** A `TopoKey` is a 1-based
    /// ordinal into `TopExp::MapShapes` — snapshot-scoped evidence, never identity
    /// (SCHEMA §9). Keyed on `(body, topoKey)` alone the cache survived regen and
    /// handed back a WRONG id (not merely a missing one) as soon as an edit
    /// renumbered the ordinals: one face got two ids, or worse, two faces shared
    /// one. Pruned to two generations by [`prune_promoted`](Self::prune_promoted).
    promoted: HashMap<(SnapshotId, BodyId, TopoKey), PromotionEntry>,
    /// Whether the one-shot DI-4 re-bind of PERSISTED element ids has already run
    /// this session (see [`rebind_persisted_elements`](Self::rebind_persisted_elements)).
    /// Ids minted in THIS session are bound at promotion time, so the pass has
    /// nothing to do after its first attempt — and retrying it on every published
    /// regen would put a worker round trip on a path that is otherwise free.
    rebind_attempted: bool,
    /// TRANSPORT failures of the re-bind pass this session. A worker error is not
    /// a ladder answer, so it re-arms the pass for the next published regen — but
    /// only up to [`MAX_REBIND_TRANSPORT_FAILURES`], so a persistently failing
    /// worker cannot put a doomed round trip on every publish. A pass the ladder
    /// actually ANSWERED (even all-refusals) stays terminal regardless.
    rebind_transport_failures: u8,
    /// The regen checkpoint cache (SCHEMA §7.7). Populated by
    /// [`take_checkpoint_at_head`](Self::take_checkpoint_at_head) (policy: on explicit
    /// `save_document` only — the cheapest sound policy). [`begin_regen`](Self::begin_regen)
    /// hands its metadata to the planner so a post-checkpoint edit regens incrementally
    /// (RestoreCheckpoint) instead of from 0. A **disposable cache**: an incompatible
    /// or unavailable checkpoint degrades to replay, never a wrong result (Invariant 7).
    ///
    /// **In-session only** (V2 policy): never persisted, never loaded from a container.
    /// The worker's restore map lives inside one worker session, so a checkpoint that
    /// outlives the process can only ever answer `restored:false` — persisting it grew
    /// the container and bought a guaranteed replay detour. Bounded + truncation-evicted
    /// by [`InMemoryCheckpointStore`].
    checkpoints: InMemoryCheckpointStore,
    /// Whether the most recent [`begin_regen`](Self::begin_regen) compiled a
    /// checkpoint-accelerated plan (observability for tests / diagnostics).
    last_regen_used_checkpoint: bool,
    /// The document's import source blobs, keyed by content hash — the carrier
    /// [`ContainerWriter::save_with_imports`] persists (only the digests a live
    /// `ImportStep` record references are actually written; see
    /// [`referenced_import_shas`](onecad_core::io::imports::referenced_import_shas)).
    /// Populated at open from the container and at import time from the just-read
    /// file. Held in memory because a save must be able to rewrite every blob even
    /// if its temp materialization was deleted underneath us.
    imports: ImportBlobs,
    /// The temp directory the blobs are materialized into for the worker. Owned
    /// here so the files outlive the unlocked `PreparedRegen::drive` window and a
    /// worker-restart replay, and are cleaned when the document closes (Drop).
    import_workspace: ImportWorkspace,
    /// STEP product names captured at import time, per `ImportStep` record, in
    /// ordinal order. Applied to the bodies that record mints once regen adopts
    /// them (see [`apply_import_body_names`](Self::apply_import_body_names)) — the
    /// ids are not known until then.
    import_names: HashMap<RecordId, Vec<String>>,
}

fn title_for_path(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "Document".to_string())
}

impl DocumentRuntime {
    /// A fresh blank document ("Untitled").
    #[must_use]
    pub fn new_blank(
        engine: Arc<dyn GeometryEngine>,
        meshes: Arc<dyn MeshProvider>,
        solver: Arc<dyn SolverEngine>,
    ) -> Self {
        let doc = Document::new(DocumentId::new());
        Self::from_document(
            doc,
            "Untitled".to_string(),
            None,
            false,
            engine,
            meshes,
            solver,
        )
    }

    /// Opens an existing `.onecad` container at `path`.
    ///
    /// # Errors
    /// [`IoError`] on a malformed / hostile / corrupt archive. A low-confidence
    /// migration opens **read-only** (not an error); reflected in [`read_only`].
    ///
    /// [`read_only`]: DocumentRuntime::is_read_only
    pub fn open(
        path: &Path,
        engine: Arc<dyn GeometryEngine>,
        meshes: Arc<dyn MeshProvider>,
        solver: Arc<dyn SolverEngine>,
    ) -> Result<Self, IoError> {
        let loaded = ContainerReader::open(path)?;
        let read_only = loaded.outcome.read_only;
        let doc = loaded.document().clone();
        let title = title_for_path(path);
        let mut rt = Self::from_document(
            doc,
            title,
            Some(path.to_path_buf()),
            read_only,
            engine,
            meshes,
            solver,
        );
        log_legacy_checkpoint_cache(&loaded);
        // Import sources are authoritative-for-a-record: without them the
        // `ImportStep` steps cannot replay. Loaded eagerly (unlike caches) because
        // regen starts immediately after open and the worker needs real files.
        rt.load_import_blobs(&loaded);
        // Last-saved geometry + thumbnail. Eager for the same reason: the whole
        // value is being resident BEFORE the open-regen's first publish.
        rt.load_open_caches(&loaded);
        Ok(rt)
    }

    fn from_document(
        mut doc: Document,
        title: String,
        path: Option<PathBuf>,
        read_only: bool,
        engine: Arc<dyn GeometryEngine>,
        meshes: Arc<dyn MeshProvider>,
        solver: Arc<dyn SolverEngine>,
    ) -> Self {
        // Legacy containers may carry sketches with no Sketch timeline record
        // (pre-fix interactive saves); without one every extrude commit fails
        // "profile sketch not found in plan". Backfill before anything reads
        // the timeline (regen mirror seed included).
        backfill_missing_sketch_records(&mut doc);
        // Seed the regen mirror from the (possibly persisted) geometry outputs so
        // the tree renders saved bodies immediately, before the first regen.
        // WP-VE.1: the mirror carries EFFECTIVE records — a loaded document's
        // expr-bound scalars are resolved here, so the very first regen already
        // builds the variable table's current numbers.
        let (effective_timeline, unresolved_variables) =
            onecad_core::regen::substituted_timeline(&doc.timeline, &doc.variables);
        log_unresolved_variables(&unresolved_variables, "open");
        let regen = RegenSession {
            bodies: seed_regen_bodies(&doc),
            timeline: effective_timeline,
            repair: doc.repair.clone(),
            elements: doc.elements.clone(),
        };
        // Repopulate the wire split-id interner from the persisted `split_of` BEFORE
        // any plan compiles (the cross-process fix): a fresh process starts with an
        // empty interner, so a downstream op that references a split child would
        // otherwise render a bare derived uuid the worker never minted (REF_UNRESOLVED)
        // — replay-from-0 compiles the whole plan up front, before the worker re-mints
        // the child.
        reintern_split_children(regen.bodies.bodies());
        let import_workspace = ImportWorkspace::new(doc.id);
        Self {
            session: DocumentSession::new(doc),
            regen,
            unresolved_variables,
            instance: Uuid::new_v4(),
            fencing: Arc::new(FencingCell::new(engine.current_epoch().0)),
            title,
            path,
            dirty: false,
            read_only,
            mesh_cache: MeshCache::new(),
            cached_meshes: HashMap::new(),
            preview_png: None,
            latest_snapshot: None,
            // No snapshot yet; `prepare_checkpoint` refuses before the first publish
            // regardless, and the first commit stamps the real value.
            head_epoch: WorkerEpoch(0),
            publisher: Arc::new(SnapshotPublisher::new()),
            engine,
            meshes,
            solver,
            occt_fingerprint: "pending-r-wp11".to_string(),
            job_seq: 0,
            sketch_solve: BTreeMap::new(),
            active_gesture: None,
            solver_lane: SolverLaneClaims::new(),
            sketch_session: None,
            gesture_seq: 0,
            promoted: HashMap::new(),
            rebind_attempted: false,
            rebind_transport_failures: 0,
            checkpoints: InMemoryCheckpointStore::new(),
            last_regen_used_checkpoint: false,
            imports: ImportBlobs::new(),
            import_workspace,
            import_names: HashMap::new(),
        }
    }

    // ── Accessors ────────────────────────────────────────────────────────────

    /// The document title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The document id (as a string).
    #[must_use]
    pub fn document_id(&self) -> String {
        self.session.document().id.to_string()
    }

    /// Current undo depth (committed steps). Used by tests / diagnostics; the B1
    /// sketch-session squash is asserted against it.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.session.undo_depth()
    }

    /// The current document revision.
    #[must_use]
    pub fn revision(&self) -> DocumentRevision {
        self.fencing.revision()
    }

    /// The lock-free fencing cell (revision + epoch). The live regen driver clones
    /// this `Arc` so its gate observes concurrent edits during worker IO (R-WP11).
    #[must_use]
    pub fn fencing(&self) -> Arc<FencingCell> {
        self.fencing.clone()
    }

    /// A worker (re)start bumped the epoch (SCHEMA §8 restart + replay): adopt the
    /// new epoch so subsequent plans fence against it, and mark the document dirty
    /// so the caller's replay recomputes geometry. Called by the WorkerManager's
    /// restart hook (under the runtime lock).
    pub fn on_worker_restart(&mut self, epoch: WorkerEpoch) {
        self.fencing.set_epoch(epoch.0);
        self.dirty = true;
    }

    /// Re-reads the engine's live epoch into the fencing cell.
    ///
    /// Construction already seeds it, but the runtime is built BEFORE it is published
    /// into the app's runtime slot (open/import do worker IO in between), and the
    /// restart hook can only reach a runtime that is already in the slot. Resampling
    /// under the slot lock closes that window: after this call either the hook saw the
    /// runtime, or this read saw the hook's epoch (VF-B4).
    pub fn adopt_current_epoch(&self) {
        self.fencing.set_epoch(self.engine.current_epoch().0);
    }

    /// The stored save path, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The body ids present at head (the regen mirror's current bodies), for STEP
    /// export (`export_step_file`). Read-only; visible-body filtering can wait.
    #[must_use]
    pub fn head_body_ids(&self) -> Vec<BodyId> {
        self.regen.bodies.bodies().iter().map(|b| b.id).collect()
    }

    /// Whether the document opened read-only (low-confidence migration).
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Whether there are unsaved changes.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The document's variable table in declaration order (WP-VE.2).
    ///
    /// Reads the STORED table (`session`), not the substituted regen mirror: the
    /// mirror holds effective records, never variables, and a variable's own
    /// `value` is already the last evaluated number (`sync_variable_values`).
    #[must_use]
    pub fn variables(&self) -> Vec<Variable> {
        self.session.document().variables.iter().cloned().collect()
    }

    /// One module's slice of the document, if it has any (ADR-0004).
    ///
    /// The payload is returned as-is: this layer has no schema for it and must
    /// not normalize what it cannot read.
    #[must_use]
    pub fn module_state(&self, module: &ModuleId) -> Option<ModuleState> {
        self.session.document().modules.get(module).cloned()
    }

    /// Every module this document carries state for, with its schema version.
    ///
    /// Enough to report "this project uses an addon you do not have installed"
    /// without decoding a single payload.
    #[must_use]
    pub fn document_modules(&self) -> Vec<(ModuleId, u32)> {
        self.session
            .document()
            .modules
            .iter()
            .map(|(id, state)| (id.clone(), state.schema_version))
            .collect()
    }

    /// Writes or clears one module's slice, through the ordinary transaction
    /// path — so a programmatic write is as undoable as a user edit
    /// (docs/ARCHITECTURE.md §9).
    ///
    /// # Errors
    /// [`DomainError`] when the document is read-only.
    pub fn set_module_state(
        &mut self,
        module: ModuleId,
        state: Option<ModuleState>,
    ) -> Result<(), DomainError> {
        self.apply(EditCommand::SetModuleState { module, state })
            .map(|_| ())
    }

    /// The scheduler-facing subscription to the latest published snapshot.
    #[must_use]
    pub fn subscribe_snapshots(&self) -> tokio::sync::watch::Receiver<Option<Arc<ModelSnapshot>>> {
        self.publisher.subscribe()
    }

    // ── Edits ────────────────────────────────────────────────────────────────

    /// Applies one [`EditCommand`], syncing the regen mirror and bumping the
    /// revision. Returns the [`CommandOutcome`] (its [`RegenHint`] drives the
    /// scheduler).
    ///
    /// [`RegenHint`]: onecad_core::edit::RegenHint
    ///
    /// # Errors
    /// [`DomainError`] on validation failure; the document is left unchanged.
    pub fn apply(&mut self, cmd: EditCommand) -> Result<CommandOutcome, DomainError> {
        if self.read_only {
            return Err(DomainError::ReadOnly);
        }
        self.reject_timeline_body_delete(&cmd)?;
        let cmd = self.hydrate_ref_intents(cmd);
        let is_rollback = matches!(cmd, EditCommand::SetRollback { .. });
        let outcome = self.session.apply(cmd)?;
        // Truncation eviction (SCHEMA §7.7): every checkpoint at or above the dirty
        // floor describes a prefix this edit just changed, so the planner would reject
        // it on every future plan — an orphan that only grows the cache. Metadata-only
        // edits carry no dirty span and touch nothing. `SetRollback` is EXEMPT: cursor
        // motion rewrites no record bytes, so every checkpoint stays hash-valid and
        // evicting them only makes the roll-forward replay from 0 (the same exemption
        // undo/redo get for free by not routing through this funnel).
        if !is_rollback {
            if let Some(dirty) = outcome.dirty {
                self.checkpoints.invalidate_from(dirty.from);
            }
        }
        self.after_mutation();
        Ok(outcome)
    }

    /// Rejects `DeleteBody` for a body the timeline produces. The Session cannot see
    /// the regen mirror, and since [`adopt_regen_bodies`] gives every regen body a
    /// document row, its `contains` check would let the delete "succeed" while the
    /// projection keeps listing the regen row — a silent no-op. Timeline geometry is
    /// deleted by deleting its producing feature.
    ///
    /// [`adopt_regen_bodies`]: onecad_core::edit::DocumentSession::adopt_regen_bodies
    fn reject_timeline_body_delete(&self, cmd: &EditCommand) -> Result<(), DomainError> {
        if let EditCommand::DeleteBody { body } = cmd {
            if self.regen.bodies.contains(*body) {
                return Err(DomainError::Validation(format!(
                    "body {body} is produced by the timeline — delete its producing feature instead"
                )));
            }
        }
        Ok(())
    }

    // ── intent hydration (HISTORY-HARDEN H5) ─────────────────────────────────

    /// Stamps the worker's frozen `intent.descriptor` evidence onto the element
    /// refs of an op being **minted or re-edited**, so the ladder scores real
    /// evidence (SCHEMA §10 descriptor features) instead of falling through to a
    /// bare-anchor nearest-centroid match at `total_weight 0.25`.
    ///
    /// Two rules, both load-bearing:
    ///
    /// * **Fill iff `intent.is_none()`.** An existing intent is the descriptor as
    ///   the element stood WHEN THE REF WAS AUTHORED (Invariant 2 — evidence, never
    ///   identity). Refreshing it would freeze post-move evidence onto a pre-move
    ///   ref, i.e. teach the ladder that the moved geometry was always the intended
    ///   one — the exact silent-rebind class this wave exists to kill.
    /// * **Mint / re-edit only.** This runs on the [`EditCommand`] funnel, never on
    ///   load. A document opened from disk keeps `intent: None` on every legacy ref,
    ///   so its `document.json` bytes and its golden `history_prefix_hash` are
    ///   unchanged (the WP-FIX W4 `host_face` no-backfill discipline).
    ///
    /// Evidence comes from the H4 promotion cache, the only place Rust holds a
    /// worker-authored descriptor per persistent [`ElementId`]. A ref whose element
    /// was never promoted through this runtime (a FE-authored id, a legacy record,
    /// a ref carrying no `primary`) is left exactly as authored — anchor-only, the
    /// pre-H5 behaviour, never a fabricated descriptor.
    fn hydrate_ref_intents(&self, cmd: EditCommand) -> EditCommand {
        match cmd {
            EditCommand::AddOperation {
                mut record,
                at_cursor,
            } => {
                if let Operation::Known(known) = &mut record.op {
                    self.stamp_intents(known);
                }
                EditCommand::AddOperation { record, at_cursor }
            }
            EditCommand::UpdateOperationParams { record, mut op } => {
                if let Operation::Known(known) = &mut op {
                    self.stamp_intents(known);
                }
                EditCommand::UpdateOperationParams { record, op }
            }
            other => other,
        }
    }

    /// Fills the `intent` of every unstamped element ref of `op` from the promotion
    /// cache. See [`hydrate_ref_intents`](Self::hydrate_ref_intents) for the rules.
    fn stamp_intents(&self, op: &mut KnownOperation) {
        for reference in op.element_refs_mut() {
            if reference.intent.is_some() {
                continue;
            }
            let Some(primary) = reference.primary.as_ref() else {
                continue;
            };
            let Some((kind, descriptor)) = self.promoted_evidence(&primary.element) else {
                continue;
            };
            reference.intent = Some(IntentQuery {
                // The descriptor policy axis the evidence was computed under — the
                // same `descriptorVersion` every plan is compiled with, so a future
                // bump invalidates stored intents through one knob (SCHEMA §6/§13).
                version: PolicyVersions::default().descriptor,
                kind,
                descriptor,
                extra: Default::default(),
            });
        }
    }

    /// The newest worker-authored `(kind, descriptor)` the promotion cache holds for
    /// `element`, or `None` when the element was never promoted here or the worker
    /// returned no descriptor for it.
    ///
    /// The cache is keyed by `(snapshot, body, topoKey)`, so the same persistent id
    /// can appear once per retained generation; the NEWEST generation's evidence is
    /// the one the ref is being authored against.
    fn promoted_evidence(&self, element: &ElementId) -> Option<(ElementKind, serde_json::Value)> {
        self.promoted
            .iter()
            .filter(|((_, _, _), entry)| &entry.element_id == element)
            .filter_map(|((snapshot, _, _), entry)| {
                entry.descriptor.clone().map(|d| (*snapshot, entry.kind, d))
            })
            .max_by_key(|(snapshot, _, _)| snapshot.0)
            .map(|(_, kind, descriptor)| (kind, descriptor))
    }

    /// Undoes the newest committed edit. `None` if nothing was undone (read-only,
    /// an open drag gesture, an open transaction, or an empty undo stack).
    ///
    /// See [`revert_report`](Self::revert_report) for what the returned
    /// [`RevertReport`] guarantees.
    pub fn undo(&mut self) -> Option<RevertReport> {
        if self.read_only || self.revert_blocked_by_gesture("undo") {
            return None;
        }
        let outcome = self.session.undo()?;
        let report = self.revert_report(outcome);
        self.after_mutation();
        self.drop_stale_sketch_session();
        Some(report)
    }

    /// Redoes the newest undone edit. `None` if nothing was redone.
    ///
    /// # Errors
    /// [`DomainError`] if a replayed command fails.
    pub fn redo(&mut self) -> Result<Option<RevertReport>, DomainError> {
        if self.read_only || self.revert_blocked_by_gesture("redo") {
            return Ok(None);
        }
        let Some(outcome) = self.session.redo()? else {
            return Ok(None);
        };
        let report = self.revert_report(outcome);
        self.after_mutation();
        self.drop_stale_sketch_session();
        Ok(Some(report))
    }

    /// Whether a revert must stand down because a drag gesture is open (SP-0 D3
    /// audit of HISTORY-HARDEN H8).
    ///
    /// H8 gave undo/redo a real dirty floor, checkpoint eviction and a
    /// `RevertToEnd`, but never looked at [`active_gesture`](Self::active_gesture).
    /// The regen half of that is harmless — replay never issues `SketchUpsert`, so
    /// it cannot touch the worker's solver store. The DOCUMENT half is not:
    /// `begin_gesture` snapshots a `before` memento and the runtime lock is
    /// released between begin and end, so ⌘Z mid-drag (nothing in
    /// `useShortcuts`/`SketchController` gates it on the pointer being down)
    /// interleaves like this:
    ///
    /// 1. `begin_gesture` captures `before` = sketch S₁;
    /// 2. `undo` reverts the last edit ⇒ the document holds S₀;
    /// 3. pointer-up: `end_gesture` builds `after` from **S₁** and commits
    ///    `SketchDragGesture { before: S₁, after }`, which writes S₁+drag straight
    ///    back into the document.
    ///
    /// The undo is silently un-done — a confident wrong answer, the exact class
    /// this stack exists to eliminate. Standing down is the same shape the
    /// `read_only` early-return already uses: `None` ⇒ the api layer emits an
    /// unchanged projection, schedules no regen and notes no mutation. It is also
    /// what every mainstream CAD does with an undo chord mid-drag.
    ///
    /// Refusing on ANY open gesture (not just one on the reverted sketch) is
    /// deliberate: the outcome only names a dirty floor, not the sketches an
    /// inverse restores, so "is this revert the drag's sketch?" is not answerable
    /// here without over-reaching into the edit layer. A gesture is a momentary
    /// pointer-down state; refusing for its duration costs nothing.
    fn revert_blocked_by_gesture(&self, what: &str) -> bool {
        let Some(g) = self.active_gesture.as_ref() else {
            return false;
        };
        tracing::warn!(
            sketch = %g.sketch_id,
            gesture = g.gesture_id,
            "{what}: refused — a drag gesture is open (its `before` memento would resurrect the reverted state)"
        );
        true
    }

    /// Turns a session-level [`UndoOutcome`] into the runtime-level [`RevertReport`]:
    /// evict the checkpoints the revert invalidated, then name the regen the caller
    /// must schedule (HISTORY-HARDEN H8).
    ///
    /// Two rules, both load-bearing and deliberately using DIFFERENT floors:
    ///
    /// * **Eviction takes the RAW floor.** Undo/redo used to bypass the truncation
    ///   eviction entirely (WP-FIX W2). That exemption rested on undo's request being
    ///   `ToEnd { from: 0 }`, whose checkpoint ceiling is `0.checked_sub(1) == None`:
    ///   a revert could never SELECT a checkpoint, so leaving stale ones around was
    ///   merely untidy. H8 threads a real floor, so a revert now selects one — and the
    ///   only thing standing between it and a checkpoint minted over the state the
    ///   revert just undid is the planner's stored-prefix-hash filter, i.e. a single
    ///   guard on a cache the revert is now allowed to read. It gets the same
    ///   truncation eviction every other mutation gets. The RAW floor is used (not the
    ///   clamped one) so an undo-of-append at index `i` drops only the checkpoints at
    ///   `≥ i`, keeping the still-valid one below it — a truncation, not a wipe.
    /// * **The regen request takes the CLAMPED floor** — `min(floor, applied − 1)`
    ///   over the cursor AFTER the inverse applied. Without the clamp an
    ///   undo-of-append yields `from == applied`, which the planner answers with an
    ///   empty plan ⇒ `NoOp` ⇒ the undone body STAYS on screen. `applied == 0`
    ///   saturates to `from: 0`, the planner's clear-publish path.
    ///
    /// The request is minted HERE, as a [`RegenRequest::RevertToEnd`], so no caller can
    /// accidentally hand the H6a edit-lane veto a `from > 0` that came from an undo.
    fn revert_report(&mut self, outcome: UndoOutcome) -> RevertReport {
        let Some(floor) = outcome.dirty_floor else {
            // Nothing the timeline regenerates moved (a sketch-visibility undo).
            return RevertReport { regen: None };
        };
        self.checkpoints.invalidate_from(floor);
        let applied = self.session.document().timeline.cursor();
        let from = floor.min(applied.saturating_sub(1));
        tracing::debug!(
            floor,
            from,
            applied,
            "revert: dirty floor threaded (checkpoints evicted from the raw floor)"
        );
        RevertReport {
            regen: Some(RegenRequest::RevertToEnd { from }),
        }
    }

    /// Drops an open sketch session whose enter watermark now EXCEEDS the (shrunk)
    /// undo depth (finding 2a). A document undo/redo that pops below the session's
    /// enter point invalidates the depth-based watermark: a later finish/cancel would
    /// squash a range that no longer maps to the session's steps. Dropping the session
    /// keeps the granular steps (safe: no squash, just a noisier stack). Called after
    /// every depth-shrinking runtime mutation (undo/redo).
    fn drop_stale_sketch_session(&mut self) {
        if let Some(s) = &self.sketch_session {
            if s.undo_watermark > self.session.undo_depth() {
                self.sketch_session = None;
            }
        }
    }

    /// After any structural mutation: re-mirror the timeline (all Dirty pending
    /// regen), bump the fencing revision, and mark unsaved.
    fn after_mutation(&mut self) {
        self.sync_regen_timeline();
        // Mirror the authoritative document's SEEDED repair gates (SCHEMA §7.3)
        // into the regen copy — that copy is what `begin_regen` reads for its
        // execution ceiling and what `save` persists. Only the seeded subset is
        // synced: worker-published repair items live solely on the regen side and
        // must not be clobbered by an edit.
        let seeded = self.session.document().repair.seeded_items();
        self.regen.repair.sync_seeded(seeded);
        // Re-register the regen bodies' document rows: `Inverse::RestoreBodies`
        // restores the WHOLE registry, so an undo erases rows adopted after the edit
        // it reverts. Insert-only — user-authored name/visible survive.
        self.session.adopt_regen_bodies(&self.regen.bodies);
        self.fencing.bump_revision();
        self.dirty = true;
    }

    /// Rebuilds the regen mirror timeline from the authoritative session timeline
    /// (records + cursor). `from_records` marks every step Dirty; the next regen
    /// recomputes states.
    ///
    /// WP-VE.1: the mirror is the document's **effective** form — every
    /// expression-driven `Scalar` resolved against the current variable table on
    /// this copy, never on the stored records. Rebuilding it here (rather than
    /// inside `begin_regen`) is what keeps ONE substituted record set behind the
    /// planner hash, the wire lowering, and the executor's replay-from-0
    /// re-plan. A step whose expression cannot resolve is stamped
    /// `StepState::Error` immediately — the state is a pure function of
    /// (records, variables), so it needs no worker round-trip to be true.
    fn sync_regen_timeline(&mut self) {
        let doc = self.session.document();
        let (mirror, unresolved) =
            onecad_core::regen::substituted_timeline(&doc.timeline, &doc.variables);
        log_unresolved_variables(&unresolved, "edit");
        self.regen.timeline = mirror;
        self.unresolved_variables = unresolved;
    }

    // ── Regen (the driver body) ──────────────────────────────────────────────

    /// Compiles and drives a regen plan to its terminal **inline** (holding the
    /// caller's `&mut self`). Kept for direct callers/tests; the live app path uses
    /// the lock-free [`begin_regen`](Self::begin_regen) → drive →
    /// [`finish_regen`](Self::finish_regen) split so a slow worker never blocks
    /// edits and fencing goes live.
    ///
    /// Because this variant holds the runtime lock for the whole run, the fencing
    /// gate cannot change during it (no edit can land) — sound, but fencing is
    /// inert here by construction.
    pub async fn run_regen(&mut self, request: RegenRequest, cancel: CancelToken) -> RegenReport {
        let Some(prepared) = self.begin_regen(request) else {
            return self.noop_report();
        };
        let driven = prepared.drive(cancel).await;
        self.finish_regen(driven)
    }

    /// A `NoOp` [`RegenReport`] stamped at the current revision — shared by the inline
    /// [`run_regen`](Self::run_regen) empty-plan path and the production driver's
    /// empty-plan branch (`lib.rs`), so a no-op-producing request STILL emits exactly
    /// one completion and the frontend awaiter's `regen-finished{noop}` anti-hang
    /// terminal fires in production (MODEL-HARDEN finding 4). No plan was compiled, so
    /// `source_revision` is the current revision — nothing older to correlate against.
    ///
    /// **`failed_steps` is NOT empty here** (W5 result truth). A broken `Scalar`
    /// binding gates the plan strictly below the offending step
    /// ([`begin_regen`](Self::begin_regen)), and a gate at step 0 leaves nothing
    /// legal to execute at all — which is exactly the single-feature document a
    /// user meets when they delete a variable an extrude is still bound to. That
    /// noop is a no-op only in the sense that no OP ran: a step really is in
    /// `StepState::Error`, and reporting silence made the frontend's correlation
    /// call the edit a success. The steps come from
    /// [`unresolved_variables`](Self::sync_regen_timeline), which is a pure
    /// function of (records, variables) and needs no worker round-trip — so a
    /// document with no broken binding still reports exactly nothing.
    #[must_use]
    pub fn noop_report(&self) -> RegenReport {
        let rev = self.fencing.revision().0;
        RegenReport {
            outcome: Outcome::NoOp,
            revision: rev,
            source_revision: rev,
            snapshot_id: 0,
            changed: Vec::new(),
            removed: Vec::new(),
            needs_repair: Vec::new(),
            failed_steps: self
                .unresolved_variables
                .values()
                .map(|u| FailedStep {
                    record_id: u.record_id.to_string(),
                    message: u.message.clone(),
                    diagnostics: Vec::new(),
                })
                .collect(),
            diagnostics: Vec::new(),
            affected_bodies: BTreeMap::new(),
            timings: RegenTimings::default(),
        }
    }

    /// Phase 1 (**locked**): compile the plan against the current timeline, capture
    /// the fencing tokens, and **clone** the regen session so the executor can drive
    /// lock-free on the copy. `None` for an empty plan. Enforces D1 body-id
    /// adoption via [`AdoptingEngine`].
    pub fn begin_regen(&mut self, request: RegenRequest) -> Option<PreparedRegen> {
        let planning_started = std::time::Instant::now();
        let ctx = PlanContext {
            policy_versions: PolicyVersions::default(),
            occt_fingerprint: self.occt_fingerprint.clone(),
        };
        let graph = DependencyGraph::new(); // linear timeline: order is authoritative.
                                            // Hand the checkpoint metadata to the planner (SCHEMA §7.7): a compatible
                                            // checkpoint at/below the dirty floor accelerates the base (incremental regen).
        let checkpoint_metas = self.checkpoints.list();
        // SCHEMA §7.3 `TransformBody` edit-safety gate: a seeded NeedsRepair step's
        // refs may only re-resolve through the repair flow, so the plan stops
        // strictly BELOW the lowest seeded step (publish ≤ m−1, Invariant 6).
        //
        // WP-VE.1 shares that ceiling: a step whose `Scalar` expression names a
        // missing variable MUST NOT execute on its stale cached value, so the plan
        // stops strictly below the lowest broken binding too. Whichever gate is
        // lower wins; the step itself already carries `StepState::Error` from
        // `sync_regen_timeline`, and everything after it stays Dirty — the same
        // shape as any other failed step.
        let gate = match (
            self.regen.repair.first_seeded_step(),
            self.unresolved_variables.keys().next().copied(),
        ) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let ceiling = match gate {
            None => None,
            // A gate on step 0 leaves nothing legal to execute at all.
            Some(0) => {
                tracing::info!(
                    "begin_regen: repair/variable gate at step 0 → nothing may execute (noop)"
                );
                return None;
            }
            Some(s) => Some(s - 1),
        };
        let plan = RegenPlanner::plan_with_ceiling(
            &self.regen.timeline,
            &graph,
            &checkpoint_metas,
            request,
            &ctx,
            ceiling,
        );
        if plan.is_empty() {
            self.last_regen_used_checkpoint = false;
            // TRUST F2: an empty op list is NOT always "nothing to do". When the request
            // covers the whole applied prefix and every op in it is suppressed (or nothing
            // is applied at all), the correct result is NO geometry — reporting NoOp here
            // left the last published bodies on screen AND in the saved container.
            if let Some(mut prepared) = self.prepare_clear_regen(&plan) {
                prepared.planner_ms = planning_started.elapsed().as_millis() as u64;
                tracing::info!("begin_regen: EMPTY plan → CLEAR publish (all ops suppressed)");
                return Some(prepared);
            }
            tracing::info!("begin_regen: EMPTY plan (noop)");
            return None;
        }
        let job = self.next_job_id();
        let (plan_rev, epoch) = self.fencing.get();
        let artifacts = PlanArtifacts {
            tessellate: Some(TessellateSpec {
                lod: DISPLAY_LOD,
                include_edges: true,
            }),
        };
        // `edited_from` comes from the REQUEST, not the plan (SCHEMA §7.2): only the
        // edit lane's `ToEnd { from > 0 }` claims an upstream content edit.
        let mut plan_req = plan
            .into_request(job, plan_rev, epoch, PolicyVersions::default(), artifacts)
            .with_edited_from(request.edited_from());
        // Attach the selected checkpoint's stored artifacts so the executor's
        // RestoreCheckpoint reconstructs the base from them (review F3). A missing
        // stored checkpoint leaves them `None` ⇒ the worker reports `restored:false`
        // ⇒ replay-from-0 (Invariant 7).
        self.last_regen_used_checkpoint = plan_req.base_checkpoint.is_some();
        if let Some(cp) = &plan_req.base_checkpoint {
            if let Some(stored) = self.checkpoints.load(cp.step_index) {
                plan_req.base_checkpoint_artifacts = Some(stored.artifacts);
            }
        }
        let base_hash_prefix = hash_prefix(plan_req.expected_base_hash.as_str());
        let step_count = plan_req.ops.len();
        tracing::info!(
            job = %job.0,
            rev = plan_rev.0,
            epoch = epoch.0,
            base = %base_hash_prefix,
            steps = step_count,
            checkpoint = self.last_regen_used_checkpoint,
            "begin_regen: steps=[{}]",
            plan_req
                .ops
                .iter()
                .map(|o| format!("{}:{}", o.operation.op_type(), o.record_id))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let prior: Vec<BodyId> = self
            .latest_snapshot
            .as_ref()
            .map(|s| s.bodies.iter().map(|b| b.body).collect())
            .unwrap_or_default();
        let executed: BTreeSet<RecordId> = plan_req.ops.iter().map(|o| o.record_id).collect();
        let clock = Arc::new(EngineClock::default());
        Some(PreparedRegen {
            work: PreparedWork::Plan {
                plan_req: Box::new(plan_req),
                // D1: worker-minted `created` ids must match an op in the plan the
                // engine is handed and be unique. Replay-from-0 base is empty, so
                // collisions are in-plan.
                engine: Box::new(
                    AdoptingEngine::new(self.engine.clone(), HashSet::new())
                        .with_clock(clock.clone()),
                ),
            },
            scratch: self.clone_regen_session(),
            fencing: self.fencing.clone(),
            publisher: self.publisher.clone(),
            instance: self.instance,
            expected: (plan_rev, epoch),
            lod: DISPLAY_LOD,
            prior,
            executed,
            job: Some(job),
            base_hash_prefix,
            step_count,
            mesh_sink: MeshSink::default(),
            planner_ms: planning_started.elapsed().as_millis() as u64,
            clock,
        })
    }

    /// Phase 1 for the **CLEAR** case (TRUST F2): a regen whose op list is empty
    /// *because every op it covers is suppressed* (or because nothing is applied at
    /// all) must still publish — as **no geometry** — instead of reporting `NoOp` and
    /// leaving the previous bodies on screen and in the saved container.
    ///
    /// `None` (⇒ keep the `NoOp` terminal) whenever the emptiness is the benign kind:
    /// any unsuppressed applied op inside the plan's target range — a request past the
    /// applied end ("nothing new to do") or a checkpoint-based plan whose restored base
    /// still holds the geometry — or a document with no geometry to clear at all.
    ///
    /// The publish is Rust-side, with **no worker round-trip**: there is nothing for the
    /// worker to compute, an ops-empty `ExecutePlan` is not part of the wire contract
    /// (SCHEMA §7.2 — and the executor short-circuits `start_step().is_none()` to
    /// `NoOp`), and the worker's stale head is harmless because the next non-empty regen
    /// from this state replays from an empty base (`start_step == 0`) or restores from
    /// immutable checkpoint artifacts. The result still travels the normal
    /// [`finish_regen`](Self::finish_regen) accept path, so revision/fencing, the
    /// `document-changed` delta and the projection all stay consistent.
    fn prepare_clear_regen(&self, plan: &RegenPlan) -> Option<PreparedRegen> {
        let target = self.fully_suppressed_target(plan)?;
        // Only meaningful if there IS geometry to drop. A never-regenerated blank
        // document keeps its `NoOp` terminal (no spurious empty publish).
        let mut prior: Vec<BodyId> = self.regen.bodies.bodies().iter().map(|b| b.id).collect();
        if let Some(snap) = &self.latest_snapshot {
            for b in &snap.bodies {
                if !prior.contains(&b.body) {
                    prior.push(b.body);
                }
            }
        }
        if prior.is_empty() {
            return None;
        }
        let (plan_rev, epoch) = self.fencing.get();
        Some(PreparedRegen {
            work: PreparedWork::Clear { target },
            scratch: self.clone_regen_session(),
            fencing: self.fencing.clone(),
            publisher: self.publisher.clone(),
            instance: self.instance,
            expected: (plan_rev, epoch),
            lod: DISPLAY_LOD,
            prior,
            // Nothing executed ⇒ no record's `outputs` is refreshed. Every op in range is
            // suppressed, and `sync_record_outputs` skips suppressed records anyway (their
            // last-known outputs are what an un-suppress cascade needs).
            executed: BTreeSet::new(),
            // No plan was compiled: there is no job id, no expected base hash and no
            // steps. The regen lane renders `job=clear` for this path.
            job: None,
            base_hash_prefix: String::new(),
            step_count: 0,
            // No worker round-trip ⇒ no inline artifacts. A CLEAR publishes NO
            // geometry, so there is nothing to cache either way.
            mesh_sink: MeshSink::default(),
            // Stamped by the caller once the whole phase-1 compile is done; no worker
            // runs on this path, so the clock stays at zero.
            planner_ms: 0,
            clock: Arc::new(EngineClock::default()),
        })
    }

    /// `Some(steps_covered)` iff every applied op in `[0, plan.target_step]` is
    /// suppressed — i.e. the geometry the (empty) `plan` describes is no geometry at
    /// all. `steps_covered` is the number of leading timeline steps the clear spans
    /// (`0` when nothing is applied).
    ///
    /// Decided off the **compiled plan**, not the raw request, because only the plan
    /// knows the target the planner actually settled on after clamping to the applied
    /// prefix and to the SCHEMA §7.3 repair ceiling. Keying on the request's `start`
    /// instead (the earlier `start != 0 ⇒ None` rule) refused the clear for every
    /// suffix request — including the roll-to-row whose whole applied range is
    /// suppressed, which then kept its stale bodies on screen.
    ///
    /// The guard that keeps a benign empty plan a `NoOp` is the suppressed-ness of the
    /// range itself: a request past the applied end, and a checkpoint-accelerated plan
    /// whose restored base still holds geometry, both cover at least one UNSUPPRESSED
    /// applied op in `[0, target]` and so return `None` here.
    fn fully_suppressed_target(&self, plan: &RegenPlan) -> Option<usize> {
        let applied = self.regen.timeline.cursor();
        if applied == 0 {
            return Some(0); // nothing applied ⇒ nothing to show.
        }
        let target = plan.target_step.min(applied - 1);
        let records = self.regen.timeline.records();
        records
            .get(0..=target)?
            .iter()
            .all(|rec| rec.suppressed)
            .then_some(target + 1)
    }

    /// Phase 3 (**locked**): commit a driven regen back into the live session.
    ///
    /// A `Published` snapshot commits **only if** the fencing tokens are unchanged
    /// since [`begin_regen`](Self::begin_regen) — i.e. no edit landed during the
    /// lock-free worker IO. If they advanced, the worker already accepted lock-free
    /// but the document moved on: the snapshot is stale, so it is **not** committed
    /// (the pending edit's regen reconverges) and the outcome is reported as
    /// `Superseded`. This upholds single-writer for the session mutation.
    ///
    /// A regen prepared on a DIFFERENT [`instance`](Self::instance) takes the same
    /// `Superseded` path: the fencing tuple is per-document, so it cannot rule out a
    /// job that outlived a runtime-slot swap (VF-B2).
    pub fn finish_regen(&mut self, driven: DrivenRegen) -> RegenReport {
        let DrivenRegen {
            outcome,
            scratch,
            prior,
            instance,
            expected,
            lod,
            executed,
            job,
            base_hash_prefix,
            step_count,
            mesh_sink,
            timings,
        } = driven;
        let job = JobLabel(job);
        // The revision this regen was PREPARED for (fenced at begin_regen). Threaded
        // into EVERY outcome — including Superseded/Failed — for commit correlation.
        let source_revision = expected.0 .0;
        let same_instance = instance == self.instance;
        if !same_instance {
            tracing::warn!(
                job = %job,
                srcRev = source_revision,
                "regen from a different runtime instance discarded (the document slot was \
                 swapped while the job was in flight)"
            );
        }
        if let Outcome::Published(snap) = &outcome {
            if same_instance && self.fencing.get() == expected {
                let snapshot_id = snap.id.0;
                let existing_body_metadata: HashSet<BodyId> = self
                    .session
                    .document()
                    .bodies
                    .bodies()
                    .iter()
                    .map(|meta| meta.id)
                    .collect();
                let (changed, removed) = self.commit_snapshot(scratch, snap, lod, &prior);
                // The worker already tessellated every one of these bodies while
                // preparing and shipped the MESH1 blobs in the plan terminal's tail.
                // Seed them now so the `document-changed` this commit is about to emit
                // resolves to cache HITS instead of one Tessellate round-trip per body.
                // INSIDE the commit guard on purpose: the same fencing check that gates
                // `commit_snapshot` gates the cache fill, so geometry the document did
                // not adopt can never be served for its generation.
                self.seed_mesh_cache(&mesh_sink);
                // Write the just-produced body provenance back onto the records so the
                // dependency graph gains its body edges (see `sync_record_outputs`).
                self.sync_record_outputs(&executed);
                // Component Library P3 WP-3.1: a reseated mate's `placement`
                // is derived data the same way `outputs` is — write it back
                // onto the document's own record right beside it.
                self.sync_mate_placements(&executed);
                // WP-VE.1: likewise for an expression-driven `Scalar` — its
                // `value` IS the last evaluated number, and this regen just
                // evaluated it.
                self.sync_variable_values(&executed);
                // Label freshly-imported bodies from their STEP product names BEFORE
                // the rows are adopted — `adopt_regen_bodies` is insert-only, so this
                // is the one moment the name can reach `document.bodies`.
                self.apply_import_body_names();
                // Give every just-published body a document metadata row, so the body
                // commands (rename / visibility) can address it at all.
                self.session.adopt_regen_bodies(&self.regen.bodies);
                self.inherit_v2_pattern_child_display_metadata(&existing_body_metadata);
                // Post-commit: the live repair state now reflects this regen. A lean
                // per-item set drives the `needs-repair` event (empty ⇒ repairs
                // cleared → banner drop).
                let needs_repair = self.needs_repair_items();
                // Finding 1: a from-0 regen can PUBLISH sibling bodies while leaving the
                // newly-committed op in Error (stale axis/region). Derive the per-record
                // failure + created/modified-body maps from the just-committed regen
                // mirror so the frontend correlates its own commit's recordId precisely.
                let failed_steps = failed_steps_of(&self.regen.timeline, &snap.diagnostics_by_step);
                let diagnostics = snap.diagnostics.clone();
                let affected_bodies = affected_bodies_of(&self.regen.timeline, &self.regen.bodies);
                let report = RegenReport {
                    outcome,
                    revision: self.fencing.revision().0,
                    source_revision,
                    snapshot_id,
                    changed,
                    removed,
                    needs_repair,
                    failed_steps,
                    diagnostics,
                    affected_bodies,
                    timings,
                };
                log_regen_outcome(job, &base_hash_prefix, step_count, &report);
                return report;
            }
            // Window race: worker accepted lock-free but the document advanced.
            tracing::warn!(
                job = %job,
                rev = self.fencing.revision().0,
                srcRev = source_revision,
                snapshot = snap.id.0,
                "regen: SUPERSEDED (window race — accepted lock-free, document advanced)"
            );
            let report = RegenReport {
                outcome: Outcome::Superseded,
                revision: self.fencing.revision().0,
                source_revision,
                snapshot_id: 0,
                changed: Vec::new(),
                removed: Vec::new(),
                needs_repair: Vec::new(),
                failed_steps: Vec::new(),
                diagnostics: Vec::new(),
                affected_bodies: BTreeMap::new(),
                timings,
            };
            log_regen_outcome(job, &base_hash_prefix, step_count, &report);
            return report;
        }
        // Finding 5: an `EngineFailed` outcome whose fencing tokens MOVED since
        // `begin_regen` is really a supersede — a later covering regen is already on the
        // way — so reporting `failed` here would send a spurious failure to a commit that
        // is about to be resolved by that covering regen. Downgrade it to `Superseded`
        // (a hard failure with UNCHANGED fencing is still reported as `failed`). Cancelled
        // and NoOp keep their own terminal. A failure that belongs to ANOTHER runtime
        // instance is likewise not this document's to report (VF-B2).
        let outcome = if matches!(outcome, Outcome::EngineFailed(_))
            && (!same_instance || self.fencing.get() != expected)
        {
            // The failure itself is the only place this error is ever visible:
            // the report about to be built reports `superseded` and DROPS it.
            if let Outcome::EngineFailed(original_error) = &outcome {
                tracing::warn!(
                    job = %job,
                    rev = self.fencing.revision().0,
                    srcRev = source_revision,
                    original_error = %original_error,
                    "regen: FAILED downgraded to superseded (fencing moved)"
                );
            }
            Outcome::Superseded
        } else {
            outcome
        };
        let diagnostics = match &outcome {
            Outcome::EngineFailed(EngineError::OpFailed { diagnostics, .. }) => diagnostics.clone(),
            _ => Vec::new(),
        };
        let report = RegenReport {
            outcome,
            revision: self.fencing.revision().0,
            source_revision,
            snapshot_id: 0,
            changed: Vec::new(),
            removed: Vec::new(),
            needs_repair: Vec::new(),
            failed_steps: Vec::new(),
            diagnostics,
            affected_bodies: BTreeMap::new(),
            timings,
        };
        log_regen_outcome(job, &base_hash_prefix, step_count, &report);
        report
    }

    /// The current document repair items (SCHEMA §9 state), for a test/repair-panel
    /// projection. Order-stable (sorted by `(step, refId)` in [`RepairState`]).
    ///
    /// [`RepairState`]: onecad_core::document::repair::RepairState
    /// `Some(recordId)` iff a new placement gesture on `body` may **fold into** an
    /// existing `TransformBody` record instead of appending a new one (SCHEMA §7.3
    /// cumulative-placement rule). Core-owned lineage query — see
    /// [`can_fold_transform`](onecad_core::document::transform::can_fold_transform)
    /// for the exact rule; the frontend has no lineage of its own.
    #[must_use]
    pub fn can_fold_transform(&self, body: BodyId) -> Option<RecordId> {
        onecad_core::document::transform::can_fold_transform(self.session.document(), body)
    }

    /// A feature's upstream/downstream transitive closures (H10 dependency view) —
    /// the same class of read-only lineage query as [`can_fold_transform`]: the
    /// frontend sees a flat history list, never edges between records. `None` when
    /// `record` is not a known timeline node. A SUPPRESSED record is still listed —
    /// suppression is a node flag on the graph, not removal from it.
    #[must_use]
    pub fn feature_dependencies(&self, record: RecordId) -> Option<FeatureDependenciesDto> {
        let graph = self.session.graph();
        if !graph.contains(record) {
            return None;
        }
        Some(FeatureDependenciesDto {
            upstream: graph
                .upstream(record)
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
            downstream: graph
                .downstream(record)
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
        })
    }

    #[must_use]
    pub fn repair_items(&self) -> &[RepairItem] {
        self.regen.repair.items()
    }

    /// A body's reconciled document metadata — the exact row
    /// [`merged_bodies`](Self::merged_bodies) would save, so `geom_stamp` /
    /// `split_of` read here are what a reopen restores.
    #[must_use]
    pub fn body_meta(&self, body: BodyId) -> Option<onecad_core::document::body::BodyMeta> {
        self.merged_bodies().get(body).cloned()
    }

    /// Lean per-item NeedsRepair summaries for the `needs-repair` event, resolving
    /// each item's timeline step to its op record id (`opId`) and the body that step
    /// OPERATES ON (`bodyId`).
    ///
    /// The body is the record's first derived input body. For every op whose inputs
    /// can go into NeedsRepair — Fillet/Chamfer (body recovered from the edge refs),
    /// Shell (`targetBodyId`), Hole (`targetBodyId`), Extrude ToFace (the boolean
    /// target when it has one) — that IS the operated body, because `derive_inputs`
    /// pushes it first. An op with NO input bodies (a NewBody extrude whose ToFace
    /// target broke) falls back to what the step PRODUCED, which is the body a
    /// candidate `TopoKey` for it would be promoted against.
    fn needs_repair_items(&self) -> Vec<NeedsRepairItemDto> {
        let records = self.regen.timeline.records();
        self.regen
            .repair
            .items()
            .iter()
            .map(|item| {
                let record = records.get(item.step_index);
                let op_id = record.map(|r| r.record_id.to_string()).unwrap_or_default();
                let body_id = record.and_then(|r| {
                    r.op.derive_inputs()
                        .bodies
                        .first()
                        .copied()
                        .or_else(|| r.outputs.first().copied())
                        .map(|b| b.to_string())
                });
                needs_repair_item_dto(op_id, body_id, item)
            })
            .collect()
    }

    /// Moves the driven scratch state into the live session and records the
    /// changed/removed bodies for the `document-changed` event.
    fn commit_snapshot(
        &mut self,
        scratch: RegenSession,
        snap: &Arc<ModelSnapshot>,
        lod: Lod,
        prior: &[BodyId],
    ) -> (Vec<(BodyId, MeshKey)>, Vec<BodyId>) {
        let _ = lod;
        // VF-B6 split-ordinal tripwire — decided BEFORE the mirror is overwritten,
        // because `self.regen.bodies` IS the previous regen's stamp set and the only
        // source that both survives save/reopen (it is seeded from the persisted
        // registry at open) and is refreshed by every commit. `document.bodies` is
        // adopted insert-only, so it cannot be trusted to hold the PREVIOUS regen's
        // stamps for a body it already knew about.
        let tripwire = ordinal_tripwire::evaluate(
            &self.regen.bodies,
            &scratch.bodies,
            self.session.document(),
        );
        let prev_snapshot = self.latest_snapshot.as_ref().map(|s| s.id);
        self.regen = scratch;
        self.latest_snapshot = Some(snap.clone());
        // First publish retires the container's last-saved geometry: real geometry
        // now exists, and these blobs describe a document state the timeline may
        // already have moved past.
        if !self.cached_meshes.is_empty() {
            tracing::debug!(
                entries = self.cached_meshes.len(),
                "regen: first publish — dropping the container's last-saved mesh cache"
            );
            self.cached_meshes.clear();
        }
        // VF-M4: the promotion cache is snapshot-keyed, so a commit retires every
        // generation but {this one, the one before it} — the window the
        // descriptor-pinned reuse rung reads. Ordered AFTER the tripwire evaluate
        // above, which must see the pre-swap mirror.
        self.prune_promoted(snap.id, prev_snapshot);
        // The caller has already fenced (tokens unchanged since begin_regen), so the
        // live epoch IS the one this geometry was computed under.
        self.head_epoch = self.fencing.get().1;
        // NOT `self.dirty = true` here. A genuine edit already dirties via
        // `after_mutation` at apply()/undo()/redo() time, before any regen
        // starts — setting it again here is redundant for that case. But
        // this path ALSO runs for edit-free regens (open_document's from-0
        // replay, a worker-restart replay), and unconditionally dirtying
        // there wrongly flags a freshly-opened, unmodified document as
        // unsaved.
        self.apply_ordinal_tripwire(tripwire);
        let changed: Vec<(BodyId, MeshKey)> =
            snap.bodies.iter().map(|b| (b.body, b.mesh_key)).collect();
        let current: HashSet<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
        let removed: Vec<BodyId> = prior
            .iter()
            .copied()
            .filter(|b| !current.contains(b))
            .collect();
        (changed, removed)
    }

    /// Plants / retires the VF-B6 ordinal-permutation gates a just-committed regen
    /// decided on (see `ordinal_tripwire`).
    ///
    /// **The seeds carry NO command inverse.** `commit_snapshot` is not an
    /// `EditCommand`, so the VF-M8 single-snapshot rule (every edit-lane repair
    /// mutation rides one `RestoreRepair` at the head of the command's inverse) does
    /// not apply and `DocumentSession::fold_repair_inverse` is not reachable from
    /// here. These gates are instead **self-healing state derived from geometry
    /// stamps**: each one records the ordinal→rank-key anchor it was raised against,
    /// so undoing the offending edit re-runs regen, the ordering matches the anchor
    /// again, and this very function clears the gate. An undo restores the geometry;
    /// the gate follows it. Nothing else may plant an `OrdinalPermutation` item.
    ///
    /// Both copies of the repair state are written: `document.repair` is
    /// authoritative (it is what a save persists and what `after_mutation` re-syncs
    /// from), and `regen.repair` is what `begin_regen` reads for its execution
    /// ceiling — so a gate planted here bites on the very next regen.
    ///
    /// **One-regen lag on the lift, by construction.** The ceiling is sampled in
    /// `begin_regen`, i.e. BEFORE the commit that clears a gate, so the regen that
    /// self-heals still stopped below the formerly-gated step; the next regen runs
    /// it. Until then the published snapshot's `repair_summary` (built by the
    /// executor from the scratch state, also before this point) still counts the
    /// gate, while `repair_items()` is already empty. Benign and self-correcting —
    /// but it is why the lift takes two regens, not one.
    fn apply_ordinal_tripwire(&mut self, tripwire: ordinal_tripwire::Tripwire) {
        if tripwire.is_empty() {
            return;
        }
        for op in &tripwire.healed {
            if self.session.clear_ordinal_gates(*op) {
                tracing::info!(op = %op, "regen: ordinal tripwire CLEARED (order matches the anchor)");
            }
        }
        if !tripwire.items.is_empty() {
            tracing::warn!(
                items = tripwire.items.len(),
                "regen: ordinal tripwire TRIPPED — an N-body op's children changed geometric \
                 rank; downstream refs seeded NeedsRepair rather than silently re-bound"
            );
            self.session.seed_regen_repair_gates(tripwire.items.clone());
        }
        ordinal_tripwire::apply(&mut self.regen.repair, &tripwire);
    }

    /// Writes the bodies each op produced/modified in the just-committed regen back
    /// onto its record's derived `outputs`, so the session's [`DependencyGraph`] gains
    /// its body-producer edges.
    ///
    /// Without this the graph only ever has sketch edges: `outputs` is minted nowhere
    /// else (records arrive from the frontend with it empty), so
    /// `SetOperationSuppression { cascade: true }` on an extrude could never reach the
    /// fillet standing on its body — the downstream closure was empty and the fillet
    /// would regen against a body that no longer exists. Derived data, so it takes no
    /// undo entry and does not bump the fencing revision.
    ///
    /// **Non-destructive outside `executed`** (TRUST F1). The write is scoped to the
    /// records this regen actually ran: a checkpoint-accelerated regen restores its base
    /// from the immutable artifacts, and that reconstructed registry carries an EMPTY
    /// lifecycle log — so [`produced_bodies_of`], which folds only that log, knows nothing
    /// about the pre-checkpoint prefix. Feeding it as the whole truth erased every earlier
    /// record's `outputs`, which killed the suppression cascade for the whole prefix, and
    /// the wipe persisted because `outputs` is serialized with the record. Suppressed
    /// records are skipped (see the session method's contract).
    ///
    /// [`DependencyGraph`]: onecad_core::history::DependencyGraph
    fn sync_record_outputs(&mut self, executed: &BTreeSet<RecordId>) {
        let produced = produced_bodies_of(&self.regen.timeline, &self.regen.bodies);
        self.session.sync_record_outputs(&produced, executed);
    }

    /// Component Library P3 WP-3.1 (spec §5.5): writes a reseated mate's
    /// `placement` from `self.regen.timeline` (already updated by the
    /// executor, see `RegenExecutor::run`) onto the document's own record —
    /// the same derived-writeback treatment `sync_record_outputs` gives
    /// `outputs`, right beside it.
    fn sync_mate_placements(&mut self, executed: &BTreeSet<RecordId>) {
        self.session
            .sync_mate_placements(&self.regen.timeline, executed);
    }

    /// WP-VE.1: writes each executed record's RESOLVED expression-driven scalar
    /// values from the regen mirror onto the document's own record — the same
    /// derived, no-undo writeback as [`sync_mate_placements`](Self::sync_mate_placements),
    /// so a document saved after a regen carries the variable table's current
    /// numbers rather than whatever the record was last hand-edited with.
    fn sync_variable_values(&mut self, executed: &BTreeSet<RecordId>) {
        self.session
            .sync_variable_values(&self.regen.timeline, executed);
    }

    fn inherit_v2_pattern_child_display_metadata(&mut self, existing: &HashSet<BodyId>) {
        let inherit: Vec<(BodyId, Vec<BodyId>)> = self
            .session
            .document()
            .timeline
            .records()
            .iter()
            .filter_map(|record| match &record.op {
                Operation::Known(KnownOperation::LinearPattern(params))
                    if params.result_policy_version == Some(2) && !params.fuse_result =>
                {
                    params
                        .source_body
                        .map(|source| (source, record.outputs.clone()))
                }
                Operation::Known(KnownOperation::CircularPattern(params))
                    if params.result_policy_version == Some(2) && !params.fuse_result =>
                {
                    params
                        .source_body
                        .map(|source| (source, record.outputs.clone()))
                }
                _ => None,
            })
            .collect();
        for (source, children) in inherit {
            for child in children {
                if child != source && !existing.contains(&child) {
                    self.session.inherit_body_display_metadata(source, child);
                }
            }
        }
    }

    /// The body registry the document should be *seen* and *saved* with:
    /// [`merge_body_metadata`] of the regen mirror and the authoritative document. The
    /// single reconciliation point — `save`, `write_autosave` and `projection` all read
    /// bodies through it, so what the tree shows is exactly what a reopen restores.
    fn merged_bodies(&self) -> BodyRegistry {
        merge_body_metadata(&self.regen.bodies, &self.session.document().bodies)
    }

    /// The body registry a **save** writes: [`merged_bodies`](Self::merged_bodies) plus
    /// the document-only rows (TRUST F4). Diverges from the projection deliberately —
    /// see [`merge_body_metadata_for_save`].
    fn saved_bodies(&self) -> BodyRegistry {
        merge_body_metadata_for_save(&self.regen.bodies, &self.session.document().bodies)
    }

    /// Deep-clones the regen session so the executor drives on a copy (lock-free).
    fn clone_regen_session(&self) -> RegenSession {
        RegenSession {
            bodies: self.regen.bodies.clone(),
            timeline: self.regen.timeline.clone(),
            repair: self.regen.repair.clone(),
            elements: self.regen.elements.clone(),
        }
    }

    fn next_job_id(&mut self) -> JobId {
        self.job_seq += 1;
        JobId(Uuid::from_u128(u128::from(self.job_seq)))
    }

    // ── Mesh pull ────────────────────────────────────────────────────────────

    /// Fetches a body's MESH1 blob (pull model), caching it. `generation` pins the
    /// snapshot; `None` ⇒ the latest snapshot's generation. `None` on miss (no
    /// document geometry, a stale generation, or a provider failure).
    ///
    /// Bytes are returned behind an `Arc` so the command hands the webview a
    /// zero-copy [`tauri::ipc::Response`].
    ///
    /// ## The pre-publish window
    ///
    /// Before the first snapshot this serves [`cached_meshes`](Self::cached_meshes)
    /// — the geometry the last explicit save embedded in the container — so a
    /// reopened document paints instantly rather than after the from-0 regen. Two
    /// bounds keep that honest:
    ///
    /// * **Only until the first publish.** The moment a snapshot exists the cache is
    ///   gone (`commit_snapshot` clears it) and this never consults it again.
    /// * **Only for an unpinned generation.** `None`/`0` means "whatever is current";
    ///   the snapshot publisher mints generations from **1**, so no caller can ever
    ///   have asked for these bytes by generation. A request that names a specific
    ///   generation is asking for a snapshot that does not exist here, and gets a
    ///   miss rather than bytes from a different one.
    pub async fn get_mesh(
        &mut self,
        body: BodyId,
        lod: Lod,
        generation: Option<u64>,
    ) -> Option<Arc<Vec<u8>>> {
        let Some((gen, snap_id, latest_gen)) = self.latest_snapshot.as_ref().map(|snap| {
            (
                generation.unwrap_or(snap.generation),
                snap.id,
                snap.generation,
            )
        }) else {
            if matches!(generation, None | Some(0)) {
                return self.cached_meshes.get(&(body, lod)).cloned();
            }
            return None;
        };
        let key = MeshKey {
            body,
            lod,
            generation: gen,
        };
        if let Some(bytes) = self.mesh_cache.get(&key) {
            return Some(bytes);
        }
        // V1 serves only the current snapshot's generation; a stale one is a miss.
        if gen != latest_gen {
            return None;
        }
        let bytes = self.meshes.fetch_mesh(body, lod, snap_id).await.ok()?;
        let arc = Arc::new(bytes);
        self.mesh_cache.put(key, arc.clone());
        Some(arc)
    }

    /// Drains a just-committed regen's inline MESH1 artifacts into the cache, so the
    /// [`get_mesh`](Self::get_mesh) calls the `document-changed` event triggers hit
    /// instead of re-tessellating geometry the worker already produced.
    ///
    /// Callable **only** from the committing branch of
    /// [`finish_regen`](Self::finish_regen): the keys carry the published
    /// generation, which is meaningful only once that publish is the live snapshot.
    fn seed_mesh_cache(&mut self, sink: &MeshSink) {
        let seeded = {
            let mut guard = sink
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };
        if seeded.is_empty() {
            return;
        }
        let bytes: usize = seeded.iter().map(|(_, b)| b.len()).sum();
        for (key, blob) in seeded {
            self.mesh_cache.put(key, blob);
        }
        tracing::debug!(
            meshes = self.mesh_cache.len(),
            bytes,
            "regen: seeded the mesh cache from the plan's inline tessellate artifacts"
        );
    }

    // ── Save ─────────────────────────────────────────────────────────────────

    /// Atomically saves the document (+ merged regen geometry outputs) to `path`,
    /// on the **explicit-save** cache policy ([`SaveCaches::explicit`]).
    /// Timestamps come from the caller (the pure core never reads the wall clock).
    ///
    /// Checkpoints are **not** written: they are in-session acceleration only (SCHEMA
    /// §7.7 V2 policy). Worker restore is in-session-map-only, so a container-loaded
    /// checkpoint could never actually be restored — it only cost container growth and
    /// a replay detour. Meshes and the preview thumbnail ARE written: unlike a
    /// checkpoint they need no worker session to be useful, they are read straight
    /// into the viewport at open, and they are the only thing that lets a reopened
    /// document paint before its from-0 regen finishes.
    ///
    /// # Errors
    /// [`IoError`] on a serialization / filesystem failure; the target is left
    /// untouched on any failure.
    pub fn save(&mut self, path: &Path, meta: SaveMeta) -> Result<(), IoError> {
        let payload = self.build_save_payload(meta, SaveCaches::explicit());
        let revision = self.revision();
        Self::write_payload(path, &payload)?;
        self.mark_saved(path, revision);
        Ok(())
    }

    /// Snapshots everything a container write needs, **detached from `&self`** —
    /// the only half of a save that must run under the runtime lock (VF-B7 / F1a).
    ///
    /// Serialization and deflate (an import blob is up to 256 MiB) then happen off
    /// the single-writer lock on a blocking thread via
    /// [`write_payload`](Self::write_payload), so a save/autosave never blocks an
    /// edit. The import carrier is cloned by refcount, not by `memcpy` (the blob
    /// bytes live behind an `Arc`).
    ///
    /// `&mut self` because a save that carries a viewport capture ADOPTS it as the
    /// document's live preview here — one step, under the lock, so the "carry
    /// forward when the next save has none" rule can never observe a half-applied
    /// state. Everything else is still a snapshot: the mesh side only `peek`s the
    /// LRU (no recency perturbation, no IO).
    #[must_use]
    pub fn build_save_payload(&mut self, meta: SaveMeta, caches: SaveCaches) -> SavePayload {
        let mut doc = self.session.document().clone();
        // Merge regen-derived outputs so a reopen shows the tree before regen.
        doc.bodies = self.saved_bodies();
        doc.elements = self.regen.elements.clone();
        doc.repair = self.regen.repair.clone();
        // `none()` (the autosave lane) writes NO cache section at all; any other
        // lane writes the preview it has, fresh capture or carried forward.
        let persists = caches.persists_caches();
        if let Some(png) = caches.preview_png {
            self.preview_png = Some(Arc::new(png));
        }
        SavePayload {
            doc,
            caches: ContainerCaches {
                geometry: BTreeMap::new(),
                meshes: if caches.meshes {
                    self.saved_mesh_caches()
                } else {
                    Vec::new()
                },
                // Checkpoints are in-session only (SCHEMA §7.7 V2 policy).
                checkpoints: Vec::new(),
                preview_png: persists
                    .then(|| self.preview_png.as_ref().map(|p| p.as_ref().clone()))
                    .flatten(),
            },
            imports: self.imports.clone(),
            meta,
        }
    }

    /// The mesh cache entries an explicit save embeds: every body of the head
    /// snapshot whose display-quality MESH1 blob is already resident.
    ///
    /// `peek` only — this runs under the single-writer lock, so it must not fetch,
    /// tessellate, or even perturb LRU recency. A body whose blob was already
    /// evicted is simply not persisted; it arrives with the open-regen like before.
    ///
    /// Body order is sorted by id (deterministic across processes), which also makes
    /// the [`MAX_SAVED_MESH_BYTES`] truncation deterministic: the same document
    /// always drops the same tail.
    fn saved_mesh_caches(&self) -> Vec<MeshCacheBlob> {
        let Some(snap) = self.latest_snapshot.as_ref() else {
            return Vec::new();
        };
        let mut bodies: Vec<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
        bodies.sort_unstable();
        bodies.dedup();

        let mut out = Vec::new();
        let mut bytes = 0usize;
        let mut dropped = 0usize;
        for (i, body) in bodies.iter().enumerate() {
            let key = MeshKey {
                body: *body,
                lod: DISPLAY_LOD,
                generation: snap.generation,
            };
            let Some(blob) = self.mesh_cache.peek(&key) else {
                continue;
            };
            if bytes + blob.len() > MAX_SAVED_MESH_BYTES {
                dropped = bodies.len() - i;
                break;
            }
            bytes += blob.len();
            // `blob` is the LRU's own `Arc` — the carrier shares the allocation
            // rather than copying it (VF-B7; the writer's copy happens off-lock).
            out.push(MeshCacheBlob {
                body: *body,
                lod: lod_str(DISPLAY_LOD).to_string(),
                bytes: blob,
            });
        }
        if dropped > 0 {
            tracing::info!(
                persisted = out.len(),
                dropped,
                bytes,
                budget = MAX_SAVED_MESH_BYTES,
                "save: mesh cache budget reached — the remaining bodies will paint after the \
                 open-regen instead of immediately"
            );
        }
        out
    }

    /// Writes a [`SavePayload`] to `path` atomically. Deliberately **takes no
    /// `self`**: this is the blocking, lock-free half of a save (see
    /// [`build_save_payload`](Self::build_save_payload)) and is meant to be handed
    /// to `tokio::task::spawn_blocking`.
    ///
    /// # Errors
    /// [`IoError`] on a serialization / filesystem failure; the target is left
    /// untouched on any failure.
    pub fn write_payload(path: &Path, payload: &SavePayload) -> Result<(), IoError> {
        ContainerWriter::save_with_imports(
            path,
            &payload.doc,
            &payload.caches,
            &payload.imports,
            &payload.meta,
        )
    }

    /// Adopts the result of a **successful** container write: `path` becomes the
    /// live save target, and the document goes clean **only if no edit landed since
    /// `revision_at_build`**.
    ///
    /// The conditional is load-bearing now that the write runs off the runtime lock:
    /// an edit applied while the bytes were being deflated is NOT in those bytes, so
    /// clearing `dirty` unconditionally would advertise unsaved work as saved. The
    /// path is adopted either way — the file at `path` is a real (if slightly older)
    /// container, and a subsequent Save must target it rather than re-prompting.
    pub fn mark_saved(&mut self, path: &Path, revision_at_build: DocumentRevision) -> bool {
        self.path = Some(path.to_path_buf());
        self.title = title_for_path(path);
        let now = self.fencing.revision();
        if now == revision_at_build {
            self.dirty = false;
            true
        } else {
            tracing::info!(
                "save: document stays DIRTY — edits landed during the container write \
                 (built at revision {}, now {})",
                revision_at_build.0,
                now.0
            );
            false
        }
    }

    /// Writes an autosave copy of the document (+ merged regen outputs) to `path`
    /// **without** touching the live save path or the dirty flag — a crash-recovery
    /// snapshot, not a real save. Reuses the same atomic [`ContainerWriter`] the
    /// autosave layout ([`io::recovery`]) points at. Timestamps come from the caller
    /// (the pure core never reads the wall clock). No cache section is written at
    /// all: no checkpoints (in-session-only policy — see [`save`](Self::save)), and
    /// no meshes/preview ([`SaveCaches::none`] — a recovery container is read once,
    /// by a path that regens from 0 anyway).
    ///
    /// [`io::recovery`]: onecad_core::io::recovery
    ///
    /// # Errors
    /// [`IoError`] on a serialization / filesystem failure; the target is left
    /// untouched on any failure.
    pub fn write_autosave(&mut self, path: &Path, meta: SaveMeta) -> Result<(), IoError> {
        Self::write_payload(path, &self.build_save_payload(meta, SaveCaches::none()))
    }

    /// The document's stable id (the autosave container + crash-marker key,
    /// SCHEMA §7 recovery layout).
    #[must_use]
    pub fn document_uuid(&self) -> DocumentId {
        self.session.document().id
    }

    /// Adopts a recovered document's real on-disk path and title, and marks it
    /// unsaved. Called after opening an autosave container during crash recovery: a
    /// subsequent Save then targets the ORIGINAL path (not the autosave copy), and
    /// the recovered edits stay dirty until the user saves. `original` `None` ⇒ a
    /// never-saved document (Save falls back to Save As).
    ///
    /// **The title has to be restored explicitly.** [`open`](Self::open) derives it
    /// from the file it read, and here that file is `<documentId>.onecad` — so a
    /// recovered document used to come up titled with a raw UUID, which reads as the
    /// wrong document entirely. The fallbacks run title → original path's stem →
    /// leave whatever `open` derived, so a marker written before the title field
    /// existed still gets the saved document's real name.
    pub fn mark_recovered(&mut self, original: Option<PathBuf>, title: Option<String>) {
        if let Some(title) = title.filter(|t| !t.is_empty()) {
            self.title = title;
        } else if let Some(path) = original.as_deref() {
            self.title = title_for_path(path);
        }
        self.path = original;
        self.dirty = true;
    }

    /// Detaches this runtime from the file it was loaded from: no path, no
    /// borrowed title, dirty.
    ///
    /// "New from template" (Component Library WP-B3) loads a template container
    /// and then calls this, so the first Save prompts for a location instead of
    /// writing back over the template. A template is immutable BY BEING NOT
    /// TARGETED, not by a filesystem permission — the copy is the user's
    /// document and the template stays exactly as shipped.
    pub fn detach_from_file(&mut self, title: &str) {
        self.path = None;
        self.title = title.to_string();
        self.dirty = true;
    }

    // ── STEP import (SCHEMA §7.3 `ImportStep`) ───────────────────────────────

    /// Authors one `ImportStep` record from a [`PreparedImport`] the caller built
    /// off the worker probe ([`crate::imports::prepare_import`]), persisting its
    /// blobs into the document's carrier and materializing them for the worker.
    ///
    /// `at_cursor` mirrors [`EditCommand::AddOperation`]: `true` inserts at the
    /// rollback cursor (the in-editor "Import STEP…" lane, so the import lands where
    /// the user is looking), `false` appends at the end.
    ///
    /// The blobs are stored + materialized BEFORE the record is applied, so the
    /// record is never visible to a regen that cannot resolve its source. On a
    /// materialization failure nothing is authored at all.
    ///
    /// # Errors
    /// [`ApiError::InvalidCommand`] when the session rejects the record, or
    /// [`ApiError::Io`] when the blob cannot be written to the workspace.
    pub fn add_import_record(
        &mut self,
        prepared: &PreparedImport,
        at_cursor: bool,
    ) -> Result<CommandOutcome, ApiError> {
        for (sha, blob) in &prepared.blobs {
            if blob.bytes.len() as u64 > MAX_IMPORT_BLOB_BYTES {
                return Err(ApiError::Io(format!(
                    "import blob {sha} is {} bytes, over the {MAX_IMPORT_BLOB_BYTES}-byte limit",
                    blob.bytes.len()
                )));
            }
            self.import_workspace
                .materialize(sha, blob.codec, &blob.bytes)
                .map_err(|e| {
                    ApiError::Io(format!(
                        "cannot stage import source {sha} for the geometry worker: {e}"
                    ))
                })?;
            self.imports.insert(sha.clone(), blob.clone());
        }
        let record = OperationRecord::new(
            RecordId(Uuid::new_v4()),
            0,
            import_record_name(&prepared.params.source_name),
            Operation::Known(KnownOperation::ImportStep(prepared.params.clone())),
        );
        let record_id = record.record_id;
        tracing::info!(
            record = %record_id,
            source = %prepared.params.source_name,
            solids = prepared.solid_count,
            at_cursor,
            "importStep: authoring record"
        );
        let outcome = self.apply(EditCommand::AddOperation { record, at_cursor })?;
        // Product names cannot be applied yet — the bodies are worker-minted and do
        // not exist until this record executes. Parked until adoption.
        if prepared.product_names.iter().any(|n| !n.is_empty()) {
            self.import_names
                .insert(record_id, prepared.product_names.clone());
        }
        Ok(outcome)
    }

    /// Imports another `.onecad` project's timeline and sketches into this document.
    ///
    /// The source records are appended at the rollback cursor (same lane as
    /// "Import STEP…"). Source [`RecordId`]s are preserved so worker-minted body ids
    /// stay deterministic; if any collide with this document the import is refused.
    /// Source sketches are added with their original ids; collisions are also refused.
    /// Import blobs referenced by source `ImportStep` records are copied into this
    /// document's carrier and workspace.
    ///
    /// This is a static snapshot: editing the source file later does NOT update this
    /// document (live XREF is out of scope for V1).
    pub fn import_project(&mut self, path: &std::path::Path) -> Result<CommandOutcome, ApiError> {
        use onecad_core::io::project_import::{
            find_import_collisions, read_project_for_import, ImportCollisions,
        };

        let imported = read_project_for_import(path).map_err(|e| ApiError::Io(e.to_string()))?;

        if let Some(collision) = find_import_collisions(self.session.document(), &imported) {
            return Err(ApiError::InvalidCommand(match collision {
                ImportCollisions::RecordId(id) => {
                    format!("cannot import: source record id {id} already exists in this document")
                }
                ImportCollisions::SketchId(id) => {
                    format!("cannot import: source sketch id {id} already exists in this document")
                }
            }));
        }

        // Stage every import blob the source timeline references.
        for (sha, blob) in &imported.import_blobs {
            if blob.bytes.len() as u64 > MAX_IMPORT_BLOB_BYTES {
                return Err(ApiError::Io(format!(
                    "import blob {sha} is {} bytes, over the {MAX_IMPORT_BLOB_BYTES}-byte limit",
                    blob.bytes.len()
                )));
            }
            self.import_workspace
                .materialize(sha, blob.codec, &blob.bytes)
                .map_err(|e| {
                    ApiError::Io(format!(
                        "cannot stage import source {sha} for the geometry worker: {e}"
                    ))
                })?;
            self.imports.insert(sha.clone(), blob.clone());
        }

        self.session.begin_transaction("Import Project");
        let mut combined = empty_outcome();
        for record in imported.records {
            let name = record.name.clone();
            let record_id = record.record_id;
            let outcome = self
                .session
                .apply(EditCommand::AddOperation {
                    record,
                    at_cursor: true,
                })
                .map_err(ApiError::from)?;
            merge_outcome(&mut combined, &outcome);
            tracing::info!(record = %record_id, name = %name, "importProject: appended record");
        }
        for (sketch_id, sketch) in imported.sketches {
            let outcome = self
                .session
                .apply(EditCommand::AddSketch { sketch })
                .map_err(ApiError::from)?;
            merge_outcome(&mut combined, &outcome);
            tracing::info!(sketch = %sketch_id, "importProject: added sketch");
        }
        if let Some(outcome) = self.session.end_transaction() {
            merge_outcome(&mut combined, &outcome);
        }

        tracing::info!(
            path = %path.display(),
            records = combined.dirty.as_ref().map_or(0, |d| d.to.saturating_sub(d.from) + 1),
            "importProject: imported"
        );
        Ok(combined)
    }

    /// Stages one Component Library geometry blob (spec §2.1 `embedded` /
    /// `document` source) into this document's carrier + worker workspace.
    ///
    /// The SAME section, cap, and materialization path a STEP import uses — a
    /// baked component solid is an import source in every respect that matters:
    /// irreplaceable input for exactly one record, unreconstructable by regen,
    /// and needed as a FILE by the worker. Reusing the section is also what makes
    /// [`referenced_import_shas`](onecad_core::io::imports::referenced_import_shas)
    /// pin it at save.
    ///
    /// Idempotent: staging bytes already carried re-materializes the path (cheap,
    /// and correct after a workspace sweep) and leaves the carrier alone.
    ///
    /// Must be called BEFORE the record that names the blob is authored — and
    /// before the placement PREVIEW, which lowers the same source through the
    /// same wire path.
    ///
    /// # Errors
    /// [`ApiError::Io`] when the bytes are over the cap or the workspace write
    /// fails; [`ApiError::InvalidCommand`] when the bytes do not hash to
    /// `sha256` (a mis-keyed blob must never enter the carrier — the section is
    /// content-addressed and every later read re-verifies it).
    pub fn stage_component_blob(
        &mut self,
        sha256: &str,
        codec: ImportSourceCodec,
        bytes: &[u8],
    ) -> Result<(), ApiError> {
        if bytes.len() as u64 > MAX_IMPORT_BLOB_BYTES {
            return Err(ApiError::Io(format!(
                "component geometry blob {sha256} is {} bytes, over the \
                 {MAX_IMPORT_BLOB_BYTES}-byte limit",
                bytes.len()
            )));
        }
        let actual = crate::imports::sha256_hex(bytes);
        if actual != sha256 {
            return Err(ApiError::InvalidCommand(format!(
                "component geometry blob is keyed {sha256} but its bytes hash to {actual}"
            )));
        }
        self.import_workspace
            .materialize(sha256, codec, bytes)
            .map_err(|e| {
                ApiError::Io(format!(
                    "cannot stage component geometry {sha256} for the geometry worker: {e}"
                ))
            })?;
        self.imports
            .entry(sha256.to_string())
            .or_insert_with(|| ImportBlob {
                codec,
                bytes: Arc::new(bytes.to_vec()),
            });
        Ok(())
    }

    /// The import blobs currently carried (a save writes the referenced subset).
    /// Tests / diagnostics.
    #[must_use]
    pub fn import_blob_shas(&self) -> Vec<String> {
        self.imports.keys().cloned().collect()
    }

    /// The directory this document's import blobs are materialized into (tests).
    #[must_use]
    pub fn import_workspace_dir(&self) -> &Path {
        self.import_workspace.dir()
    }

    /// Loads every import blob from an opened container into the carrier and
    /// materializes it for the worker.
    ///
    /// A missing / corrupt / oversized blob is SKIPPED with a diagnostic, never an
    /// open failure: `io::imports` designs the blast radius to be exactly one
    /// timeline step, so the document opens, the tree renders, and only that one
    /// `ImportStep` fails (loudly) at its own step.
    fn load_import_blobs(&mut self, loaded: &LoadedContainer) {
        for info in loaded.import_blobs() {
            match loaded.read_import_blob(&info.sha256) {
                Ok(bytes) => {
                    if let Err(e) =
                        self.import_workspace
                            .materialize(&info.sha256, info.codec, &bytes)
                    {
                        tracing::warn!(
                            sha = %info.sha256,
                            error = %e,
                            "import blob could not be staged for the worker — that step will fail"
                        );
                    }
                    self.imports.insert(
                        info.sha256.clone(),
                        ImportBlob {
                            codec: info.codec,
                            bytes: Arc::new(bytes),
                        },
                    );
                }
                Err(e) => tracing::warn!(
                    sha = %info.sha256,
                    error = %e,
                    "import blob unreadable — the document opens, that ImportStep step will fail"
                ),
            }
        }
    }

    /// Loads the "paint at open" caches out of a just-opened container: the
    /// `preview.png` thumbnail and every `meshes/<bodyId>.<lod>.mesh` blob.
    ///
    /// Everything here is best-effort by construction (Invariant 7: a cache
    /// degrades performance, never correctness). A stale container, an unreadable
    /// entry, a blob that is not valid MESH1 — all of it degrades to "no cached
    /// geometry", which is exactly the behavior before this feature existed.
    ///
    /// **Staleness is all-or-nothing.** A container-level `opsHash` mismatch means
    /// the timeline no longer matches the geometry those blobs were tessellated
    /// from, so painting ANY of them would show the user a body their document does
    /// not describe. One log line, then nothing is loaded.
    fn load_open_caches(&mut self, loaded: &LoadedContainer) {
        match loaded.read_cache(PREVIEW_PATH) {
            Ok(CacheRead::Present(bytes)) => self.preview_png = Some(Arc::new(bytes)),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "open: preview.png unreadable — no thumbnail"),
        }

        let entries = loaded.mesh_cache_entries();
        if entries.is_empty() {
            return;
        }
        if loaded.outcome.stale_caches {
            tracing::info!(
                entries = entries.len(),
                "open: container mesh caches are STALE (opsHash mismatch) — skipping all of them; \
                 the viewport stays empty until the open-regen publishes"
            );
            return;
        }
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        let reads = match loaded.read_caches(&paths) {
            Ok(reads) => reads,
            Err(e) => {
                tracing::warn!(error = %e, "open: mesh cache batch read failed — none loaded");
                return;
            }
        };

        let mut bytes_total = 0usize;
        let mut skipped = 0usize;
        for (entry, (_, read)) in entries.iter().zip(reads) {
            let blob = match read {
                CacheRead::Present(bytes) => bytes,
                CacheRead::Stale | CacheRead::Missing => {
                    skipped += 1;
                    continue;
                }
            };
            if let Err(e) = onecad_protocol::mesh::validate_mesh_blob(&blob) {
                tracing::warn!(
                    body = %entry.body, error = %e,
                    "open: container mesh cache entry is not valid MESH1 — skipped"
                );
                skipped += 1;
                continue;
            }
            if bytes_total + blob.len() > MAX_LOADED_MESH_BYTES {
                tracing::info!(
                    loaded = self.cached_meshes.len(),
                    bytes = bytes_total,
                    "open: mesh cache budget reached — remaining entries wait for the regen"
                );
                break;
            }
            bytes_total += blob.len();
            self.cached_meshes.insert(
                (entry.body, crate::worker::lod_from_str(&entry.lod)),
                Arc::new(blob),
            );
        }
        tracing::info!(
            loaded = self.cached_meshes.len(),
            skipped,
            bytes = bytes_total,
            "open: seeded last-saved geometry from the container (paints before the regen)"
        );
    }

    /// Names the bodies an `ImportStep` just minted from the STEP product names
    /// captured at import time.
    ///
    /// Runs on the **regen mirror**, immediately before
    /// [`adopt_regen_bodies`](onecad_core::edit::DocumentSession::adopt_regen_bodies)
    /// inserts the document metadata rows — so the name lands in the row the tree
    /// reads, the save persists, and a reopen restores. It is therefore also the
    /// point after which a user rename WINS: `merge_body_metadata` overlays
    /// `document.bodies` onto the mirror unconditionally, and `adopt_regen_bodies`
    /// is insert-only.
    ///
    /// Best effort by design (SCHEMA §7.3: names are recoverable evidence, never
    /// identity): a count mismatch between the probe's `productNames` and the bodies
    /// the op actually produced means the two lists cannot be zipped by ordinal
    /// without risking a wrong label, so the whole record is skipped. Empty names
    /// are skipped individually.
    fn apply_import_body_names(&mut self) {
        if self.import_names.is_empty() {
            return;
        }
        let produced = produced_bodies_of(&self.regen.timeline, &self.regen.bodies);
        for (record, names) in &self.import_names {
            let Some(bodies) = produced.get(record) else {
                continue; // not executed in this regen (rolled back / suppressed)
            };
            if bodies.len() != names.len() {
                tracing::debug!(
                    record = %record,
                    bodies = bodies.len(),
                    names = names.len(),
                    "importStep: productNames count differs from minted bodies — names skipped"
                );
                continue;
            }
            for (body, name) in bodies.iter().zip(names) {
                if !name.is_empty() {
                    self.regen.bodies.set_name(*body, name.clone());
                }
            }
        }
    }

    // ── Checkpoints (SCHEMA §7.7) ────────────────────────────────────────────

    /// Takes a checkpoint of the current head into the cache (the SaveCheckpoint
    /// policy: on explicit `save_document` only — the cheapest sound policy, so a save
    /// is the natural moment a durable acceleration base is minted). No-op unless
    /// [`prepare_checkpoint`](Self::prepare_checkpoint) admits the mint. Best-effort:
    /// a worker failure skips the checkpoint (the cache is disposable — Invariant 7).
    pub async fn take_checkpoint_at_head(&mut self) {
        let Some(ticket) = self.prepare_checkpoint() else {
            return;
        };
        if let Ok(artifacts) = self.engine.save_checkpoint(ticket.step).await {
            self.adopt_checkpoint(ticket, artifacts);
        }
    }

    /// The geometry engine behind this document, cloned out so a caller can drive a
    /// round-trip (e.g. `SaveCheckpoint`) **without holding the runtime lock**.
    /// Pairs with [`prepare_checkpoint`](Self::prepare_checkpoint) /
    /// [`adopt_checkpoint`](Self::adopt_checkpoint).
    #[must_use]
    pub fn engine_arc(&self) -> Arc<dyn GeometryEngine> {
        self.engine.clone()
    }

    /// The mesh provider behind this document, cloned out so a caller can pull
    /// MESH1 bytes for several bodies **without holding the runtime lock**
    /// across the worker round trips (the same R-WP11 rule
    /// [`engine_arc`](Self::engine_arc) exists for). The 3MF exporter is the
    /// first caller: it snapshots this alongside `head_body_ids` /
    /// `head_snapshot_id` under one short lock, then fetches every body's mesh
    /// after releasing it.
    #[must_use]
    pub fn meshes_arc(&self) -> Arc<dyn MeshProvider> {
        self.meshes.clone()
    }

    /// Admits (or refuses) a checkpoint mint at the current head, capturing the
    /// fencing tokens the artifacts must still be current against. `None` ⇒ skip.
    #[must_use]
    pub fn prepare_checkpoint(&self) -> Option<CheckpointTicket> {
        let applied = self.regen.timeline.cursor();
        if applied == 0 {
            return None;
        }
        let head_step = applied - 1;
        // The worker head must be the fully-regenerated document head (its last valid
        // step == head_step), else a checkpoint keyed at head_step would mis-describe
        // the worker's actual geometry.
        if self.latest_snapshot.as_ref().and_then(|s| s.step_index) != Some(head_step) {
            tracing::info!(
                "checkpoint: SKIPPED at save — head step {head_step} not fully regenerated \
                 (snapshot step {:?}); the save proceeds without an acceleration base",
                self.latest_snapshot.as_ref().and_then(|s| s.step_index)
            );
            return None;
        }
        let fencing = self.fencing.get();
        // VF-M2: a restarted worker is a fresh process holding NO geometry, while
        // `latest_snapshot` still describes the pre-restart head — so the check above
        // passes and SaveCheckpoint would mint an EMPTY-prefix checkpoint OVER the
        // valid one (the store's per-step insert overwrites), losing the acceleration
        // base and persisting a corrupt cache entry. Wait for the replay to republish.
        if fencing.1 != self.head_epoch {
            tracing::info!(
                "checkpoint: SKIPPED at save — head geometry was produced under worker epoch \
                 {} but the worker is now at epoch {}; the restart replay must land first",
                self.head_epoch.0,
                fencing.1 .0
            );
            return None;
        }
        Some(CheckpointTicket {
            step: head_step,
            fencing,
        })
    }

    /// Stores just-saved checkpoint artifacts against the ticket that admitted them.
    /// A fencing move since [`prepare_checkpoint`](Self::prepare_checkpoint) means the
    /// artifacts no longer describe the head — a logged skip, never an error (the
    /// cache is disposable, Invariant 7).
    pub fn adopt_checkpoint(&mut self, ticket: CheckpointTicket, artifacts: CheckpointArtifacts) {
        if self.fencing.get() != ticket.fencing {
            tracing::info!(
                "checkpoint: DISCARDED at step {} — fencing moved during SaveCheckpoint \
                 (prepared at rev {} epoch {}, now rev {} epoch {})",
                ticket.step,
                ticket.fencing.0 .0,
                ticket.fencing.1 .0,
                self.fencing.get().0 .0,
                self.fencing.get().1 .0
            );
            return;
        }
        // Adopt the worker's real OCCT fingerprint from the checkpoint so the
        // PlanContext compatibility check (which governs checkpoint selection)
        // matches the envelope — the V1 `occt_fingerprint` placeholder would
        // otherwise reject every checkpoint (Invariant 7 fingerprint gate).
        if let Some(env) = artifacts.representative_envelope() {
            self.occt_fingerprint = env.occt_fingerprint.clone();
        }
        self.checkpoints.save(ticket.step, artifacts);
    }

    /// Whether the most recent [`begin_regen`](Self::begin_regen) compiled a
    /// checkpoint-accelerated (incremental) plan. Observability for tests.
    #[must_use]
    pub fn last_regen_used_checkpoint(&self) -> bool {
        self.last_regen_used_checkpoint
    }

    /// The number of checkpoints in the cache (tests / diagnostics).
    #[must_use]
    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.list().len()
    }

    /// The steps the cached checkpoints sit at, ascending (tests / diagnostics).
    #[must_use]
    pub fn checkpoint_steps(&self) -> Vec<usize> {
        let mut steps: Vec<usize> = self.checkpoints.list().iter().map(|m| m.step).collect();
        steps.sort_unstable();
        steps
    }

    // ── Projection ───────────────────────────────────────────────────────────

    /// Builds the frontend [`DocumentProjection`] from the authoritative document
    /// + the regen mirror (states) + the latest snapshot (body geometry).
    #[must_use]
    pub fn projection(&self) -> DocumentProjection {
        let doc = self.session.document();

        // Bodies: EXACTLY `merged_bodies` — the same registry a save writes, so the
        // tree shows what a reopen would restore. Membership is the regen mirror's
        // alone. A `document.bodies` row the regen does not carry is NOT projected: it
        // is either a row adopted for a body a later regen dropped (a suppressed or
        // deleted feature — projecting it would put a phantom, mesh-less row in the
        // tree) or an `AddBody` registration, which a save has never persisted either.
        let mut bodies = BTreeMap::new();
        for b in self.merged_bodies().bodies() {
            bodies.insert(
                b.id.to_string(),
                BodyDto {
                    id: b.id.to_string(),
                    name: b.name.clone(),
                    visible: b.visible,
                    health: b.health,
                    color: b.color,
                    face_colors: b
                        .face_colors
                        .iter()
                        .map(|(k, v)| (k.to_string(), *v))
                        .collect(),
                },
            );
        }

        // Sketches: real dof/status come from the last solver-lane solve
        // (`sketch_solve`, updated by enter/upsert/end-gesture, SCHEMA §7.4);
        // an unsolved sketch reads `dof:0`/`Ok` until first solved.
        let mut sketches = BTreeMap::new();
        for (id, sk) in &doc.sketches {
            let (dof, status) = self
                .sketch_solve
                .get(id)
                .map_or((0, SketchStatus::Ok), |(dof, st)| (*dof, st.tree_status()));
            sketches.insert(
                id.to_string(),
                SketchDto {
                    id: id.to_string(),
                    name: sk.name.clone(),
                    visible: doc.sketch_visible(*id),
                    dof,
                    status,
                    geometry_token: sketch_geometry_token(sk),
                    host_face: sketch_host_face(sk),
                },
            );
        }

        // Datums: straight off the authoritative document. Unlike bodies there is
        // no regen mirror to reconcile against — a datum is pure core-owned state
        // that never crosses the OCW1 wire, so `document.datum_planes` IS the
        // truth (and is exactly what a save writes / a reopen restores).
        let mut datums = BTreeMap::new();
        for (id, d) in &doc.datum_planes {
            datums.insert(
                id.to_string(),
                DatumDto {
                    id: id.to_string(),
                    name: d.name.clone(),
                    kind: d.kind.name().to_string(),
                    base_plane_id: d.base_plane_id.clone(),
                    offset: d.offset,
                    plane: d.resolved_plane.into(),
                    resolved_valid: d.resolved_valid,
                },
            );
        }

        let features = doc
            .timeline
            .records()
            .iter()
            .enumerate()
            .map(|(i, rec)| self.feature_dto(i, rec))
            .collect();

        DocumentProjection {
            status: DocStatus::Ready,
            document_id: doc.id.to_string(),
            revision: self.fencing.revision().0,
            title: self.title.clone(),
            dirty: self.dirty,
            bodies,
            sketches,
            datums,
            features,
            // Variables: straight off the authoritative document, in declaration
            // order — the same table `list_variables` serves. It rides here (W5)
            // so a variable edit returns the SAME projection every other mutating
            // command does and can be correlated against its regen.
            variables: doc.variables.iter().map(VariableDto::from).collect(),
            // Timeline cursor + length drive the legacy-draft recovery hint
            // (`appliedOps < totalOps` ⇒ ops sit beyond the rollback bar).
            applied_ops: doc.timeline.cursor(),
            total_ops: doc.timeline.len(),
            geometry_source: self.geometry_source().to_string(),
        }
    }

    /// Which geometry the viewport is being fed right now (see
    /// [`DocumentProjection::geometry_source`]). Mirrors exactly what
    /// [`get_mesh`](Self::get_mesh) will serve: a snapshot wins, else the
    /// container's last-saved meshes, else nothing.
    fn geometry_source(&self) -> &'static str {
        if self.latest_snapshot.is_some() {
            GEOMETRY_SOURCE_LIVE
        } else if !self.cached_meshes.is_empty() {
            GEOMETRY_SOURCE_CACHED
        } else {
            GEOMETRY_SOURCE_NONE
        }
    }

    fn feature_dto(&self, index: usize, rec: &OperationRecord) -> FeatureDto {
        let kind = feature_kind(&rec.op);
        let label = if rec.name.is_empty() {
            default_label(&rec.op).to_string()
        } else {
            rec.name.clone()
        };
        let state = self
            .regen
            .timeline
            .state(index)
            .cloned()
            .unwrap_or(StepState::Dirty);
        // ONE call for the row's text AND its inline-editable primary dimension —
        // `dto::feature_value` decides both in a single match so a row can never
        // display one number and edit another (H3).
        let value = feature_value(&rec.op);
        FeatureDto {
            id: rec.record_id.to_string(),
            kind,
            op_type: op_type_name(&rec.op),
            label,
            value_text: value.text,
            primary_value: value.primary,
            primary_value_kind: value.primary_kind.map(ToString::to_string),
            // The variable binding, if any (WP-VE.2). Substitution rewrites a
            // Scalar's cached `value` and never its `expr`, so this is the stored
            // binding whichever record set this projection was built from.
            primary_expr: value.primary_expr,
            status: feature_status(&state),
            // Surface a step's worker failure reason (`StepState::Error{reason}`) so
            // the HistoryList row can tint + tooltip it end-to-end (Codex MAJOR-4).
            status_message: feature_status_message(&state),
            // This map and the step state share one atomic snapshot. A successful
            // retry replaces it without this entry, clearing stale evidence.
            diagnostics: self
                .latest_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.diagnostics_by_step.get(&index))
                .map(|items| items.iter().take(64).cloned().collect())
                .unwrap_or_default(),
            // From the RECORD, not the mirror state: the record is the single source
            // of truth for suppression (the state is derived), and the mirror can lag
            // the authoritative timeline between an edit and its regen.
            suppressed: rec.suppressed,
        }
    }

    // ── Sketch solver lane (SCHEMA §7.4) ─────────────────────────────────────

    /// Enters sketch mode: syncs the authoritative sketch to the worker solver lane
    /// (`SketchUpsert`) and returns the live session (entities/constraints wire form
    /// plus real dof/status). The sketch must already exist (a new sketch is created
    /// via [`EditCommand::AddSketch`] through [`apply`](Self::apply) first).
    ///
    /// # Errors
    /// [`EngineError`] on an unknown sketch or a worker-side failure.
    pub async fn enter_sketch(
        &mut self,
        sketch_id: SketchId,
    ) -> Result<SketchSessionDto, EngineError> {
        let sketch = self.sketch_or_err(sketch_id, "enterSketch")?;
        // B1 squash: remember the pre-session sketch + undo watermark so finish/cancel
        // can collapse every in-session granular edit into ONE net undoable command. If
        // a session for the SAME sketch is already open (a double-enter / a model-mode
        // read that forgot to close), KEEP the older watermark + prior so the squash
        // still covers the whole range — resetting to a later point would strand the
        // earlier granular steps. But REFUSE the keep when the stack has since shrunk
        // below the stored watermark (finding 2b — a document undo popped below the
        // enter point): that watermark is stale, so fall through to a fresh session.
        let keep_existing = matches!(
            &self.sketch_session,
            Some(s) if s.sketch_id == sketch_id && s.undo_watermark <= self.session.undo_depth()
        );
        let (plane, entities, constraints) = crate::worker::wire::sketch_wire(&sketch);
        // The FALLIBLE solve happens BEFORE opening the session (finding 2c): a worker
        // error must not leave a stale open session (`?` returns with the session
        // untouched). Only after it succeeds do we open/keep the watermark.
        let solved = self.solver.sketch_upsert(&sketch).await?;
        self.record_solve(sketch_id, &solved);
        if !keep_existing {
            self.sketch_session = Some(SketchSession {
                sketch_id,
                prior: sketch.clone(),
                undo_watermark: self.session.undo_depth(),
                evicted_at_enter: self.session.evictions(),
            });
        }
        Ok(SketchSessionDto {
            sketch_id: sketch_id.to_string(),
            plane,
            entities,
            constraints,
            dof: solved.dof,
            status: solved.status,
            // Surface the entering solve's conflicting constraints (SCHEMA §7.4) so
            // re-entering a conflicting sketch tints the offending constraints.
            conflicting: solved.conflicting,
            // Same projection for the per-entity state: entering a sketch colours
            // its pinned-down entities without waiting for the next upsert.
            entity_states: solved.entity_states,
        })
    }

    /// Reads a sketch's current geometry as a static snapshot — no worker call, no
    /// session/gesture state, no mutation (`getSketch`; the model-mode display-layer
    /// read). Returns the same [`SketchSessionDto`] shape as
    /// [`enter_sketch`](Self::enter_sketch), but `dof`/`status` come from the
    /// `sketch_solve` cache of the last solver-lane solve rather than a fresh solve;
    /// a sketch that has never been entered/upserted reads `dof:0`/`UnderConstrained`
    /// (the same "not yet solved" fallback [`SketchSolveStatus::parse`] documents).
    ///
    /// # Errors
    /// [`EngineError`] on an unknown sketch.
    pub fn get_sketch(&self, sketch_id: SketchId) -> Result<SketchSessionDto, EngineError> {
        let sketch = self.sketch_or_err(sketch_id, "getSketch")?;
        let (plane, entities, constraints) = crate::worker::wire::sketch_wire(&sketch);
        let (dof, status) = self
            .sketch_solve
            .get(&sketch_id)
            .copied()
            .unwrap_or((0, SketchSolveStatus::UnderConstrained));
        Ok(SketchSessionDto {
            sketch_id: sketch_id.to_string(),
            plane,
            entities,
            constraints,
            dof,
            status,
            // A static read carries no live solve — the sketch_solve cache stores only
            // (dof, status), so there is no conflicting-id evidence to surface here.
            conflicting: Vec::new(),
            // Likewise no per-entity evidence: empty means "nothing to say"
            // (SCHEMA §7.4), which is exactly true of a cache-backed read.
            entity_states: crate::dto::EntityStates::new(),
        })
    }

    /// Captures an immutable, read-only region query for a persisted sketch.
    ///
    /// The returned query is driven after releasing the runtime lock. Unlike
    /// [`finish_sketch`](Self::finish_sketch), it never squashes or closes an
    /// edit session and never changes document/undo state.
    pub fn prepare_sketch_regions(
        &self,
        sketch_id: SketchId,
    ) -> Result<PreparedSketchRegions, EngineError> {
        // `drive()` runs unlocked and re-upserts this snapshot into the worker's
        // per-sketch `SketchStore` entry. Mid-gesture that is destructive, but NOT
        // for the reason this comment used to claim: `SolveDrag` never reads the
        // store (the worker's `Gesture` owns a private `Sketch` clone), so drag
        // replies cannot "apply onto stale state". The damage is at `EndGesture`,
        // which commits by reading the store BACK — it would apply the solved drag
        // positions onto this stale re-upserted clone and `put` them at a revision
        // derived from the pre-query one, which `SketchStore::put` assigns exactly
        // and therefore lets REGRESS. Full write-up + the deferred worker-side
        // monotonicity fix: [`solver_lane`].
        //
        // Keep the named refusal (a better message than the claim's) and THEN take
        // the claim, which is what actually closes the check-then-act window: this
        // check only looks at `active_gesture` right now, while the claim is held
        // through the whole unlocked drive.
        if self
            .active_gesture
            .as_ref()
            .is_some_and(|g| g.sketch_id == sketch_id)
        {
            return Err(op_failed(format!(
                "getSketchRegions: sketch {sketch_id} has an active drag gesture — retry after pointer-up"
            )));
        }
        let sketch = self.sketch_or_err(sketch_id, "getSketchRegions")?;
        let claim = self
            .solver_lane
            .claim(sketch_id, LaneOwner::RegionQuery)
            .map_err(|held| {
                op_failed(format!(
                    "getSketchRegions: {} is in flight for sketch {sketch_id} — retry",
                    held.describe()
                ))
            })?;
        Ok(PreparedSketchRegions {
            sketch,
            solver: self.solver.clone(),
            _claim: claim,
        })
    }

    /// Applies a batch of sketch edits authoritatively (one undoable
    /// [`EditCommand::SketchEdit`]) then re-solves on the worker for live dof/status
    /// (SCHEMA §7.4). A non-drag upsert is an identity solve (no coordinate
    /// write-back — the worker's `SketchUpsert` reports no positions).
    ///
    /// # Errors
    /// [`EngineError`] on a read-only document, an invalid edit, or a worker failure.
    pub async fn sketch_upsert(
        &mut self,
        sketch_id: SketchId,
        ops: Vec<SketchEditOp>,
    ) -> Result<SketchUpsertDto, EngineError> {
        self.sketch_upsert_with_outcome(sketch_id, ops)
            .await
            .map(|(solved, _)| solved)
    }

    /// [`sketch_upsert`](Self::sketch_upsert) plus the exact edit outcome the app
    /// scheduler must consume. `None` means the upsert only refreshed the solve.
    pub async fn sketch_upsert_with_outcome(
        &mut self,
        sketch_id: SketchId,
        ops: Vec<SketchEditOp>,
    ) -> Result<(SketchUpsertDto, Option<CommandOutcome>), EngineError> {
        if self.read_only {
            return Err(op_failed("sketchUpsert: read-only document"));
        }
        let outcome = if ops.is_empty() {
            None
        } else {
            Some(
                self.apply(EditCommand::SketchEdit {
                    sketch: sketch_id,
                    ops,
                })
                .map_err(|e| op_failed(format!("sketchUpsert edit: {e}")))?,
            )
        };
        let sketch = self.sketch_or_err(sketch_id, "sketchUpsert")?;
        let solved = self.solver.sketch_upsert(&sketch).await?;
        self.record_solve(sketch_id, &solved);
        Ok((solved, outcome))
    }

    /// Opens a drag gesture on `drag_point` (SCHEMA §7.4 `BeginGesture`). Snapshots
    /// the pre-gesture sketch (the `before` memento) so pointer-up can commit **one**
    /// undo command for the whole drag.
    ///
    /// `target` declares WHAT the pointer grabbed (point handle / arc endpoint /
    /// radius / whole entity body). [`GestureTarget::point`](wire::GestureTarget::point)
    /// is the plain point drag every pre-SP-2 caller means. `drag_point` stays the
    /// gesture's `SolveDrag` handle for all kinds — the worker ignores `pointId`
    /// off the point path, and the runtime needs a stable per-gesture id anyway.
    ///
    /// # Errors
    /// [`EngineError`] on a read-only document, an unknown sketch, or a worker failure.
    pub async fn begin_gesture(
        &mut self,
        sketch_id: SketchId,
        drag_point: EntityId,
        target: wire::GestureTarget,
    ) -> Result<crate::dto::BeginGestureDto, EngineError> {
        if self.read_only {
            return Err(op_failed("beginGesture: read-only document"));
        }
        let sketch = self.sketch_or_err(sketch_id, "beginGesture")?;
        // SP-0 D3: take the solver lane BEFORE the upsert below, so an in-flight
        // region query (whose `drive()` runs with the runtime lock released) can
        // never have this gesture opened underneath it. See [`solver_lane`] for
        // what the collision corrupts.
        let claim = self.claim_gesture_lane(sketch_id)?;
        // Ensure the worker holds the current sketch (its BeginGesture reads it).
        let solved = self.solver.sketch_upsert(&sketch).await?;
        self.record_solve(sketch_id, &solved);
        let gesture_id = self.next_gesture_id();
        let ready = self
            .solver
            .begin_gesture(
                &sketch_id.to_string(),
                solved.sketch_revision,
                gesture_id,
                drag_point,
                target,
                // solverPolicyHash: reserved, always "" — see wire::begin_gesture_args.
                "",
            )
            .await?;
        // Assigning here drops any previously active gesture (possibly on another
        // sketch), releasing its claim with it — the pre-D3 overwrite semantics,
        // unchanged.
        self.active_gesture = Some(ActiveGesture {
            gesture_id,
            sketch_id,
            drag_point,
            before: sketch,
            next_seq: 1,
            _claim: claim,
        });
        Ok(ready)
    }

    /// Takes `sketch_id`'s solver lane for a drag gesture (SP-0 D3).
    ///
    /// A contending REGION QUERY is refused: its unlocked `drive()` is exactly what
    /// the claim exists to fence against. A contending GESTURE is the same sketch's
    /// own already-open drag, which [`begin_gesture`](Self::begin_gesture) has
    /// always silently REPLACED (its tail assignment overwrites unconditionally);
    /// dropping the superseded record here frees the lane so that behavior is
    /// preserved rather than turned into a new refusal.
    fn claim_gesture_lane(&mut self, sketch_id: SketchId) -> Result<Claim, EngineError> {
        match self.solver_lane.claim(sketch_id, LaneOwner::Gesture) {
            Ok(claim) => Ok(claim),
            Err(LaneOwner::Gesture) => {
                drop(self.active_gesture.take());
                self.solver_lane
                    .claim(sketch_id, LaneOwner::Gesture)
                    .map_err(|held| {
                        op_failed(format!(
                            "beginGesture: {} is in flight for sketch {sketch_id} — retry",
                            held.describe()
                        ))
                    })
            }
            Err(LaneOwner::RegionQuery) => Err(op_failed(format!(
                "beginGesture: a region query is in flight for sketch {sketch_id} — retry"
            ))),
        }
    }

    /// One incremental drag solve (SCHEMA §7.4 `SolveDrag`). Fired latest-wins: the
    /// caller sends the newest `target` without awaiting each serially. The returned
    /// positions are a **preview** (not committed) — only [`end_gesture`](Self::end_gesture)
    /// mutates the document.
    ///
    /// # Errors
    /// [`EngineError`] when no gesture is active or the worker fails.
    pub async fn solve_drag(
        &mut self,
        target: [f64; 2],
    ) -> Result<crate::dto::DragSolveDto, EngineError> {
        let (gesture_id, drag_point, seq) = {
            let g = self
                .active_gesture
                .as_mut()
                .ok_or_else(|| op_failed("solveDrag: no active gesture"))?;
            let seq = g.next_seq;
            g.next_seq += 1;
            (g.gesture_id, g.drag_point, seq)
        };
        self.solver
            .solve_drag(gesture_id, seq, drag_point, target)
            .await
    }

    /// Pointer-up final exact solve (SCHEMA §7.4 `EndGesture`): applies the solved
    /// positions to the `before` memento and commits **one** [`EditCommand::SketchDragGesture`]
    /// (single undo step for the whole drag). Returns the final dof/status/positions.
    ///
    /// # Errors
    /// [`EngineError`] when no gesture is active, the commit is invalid, or the
    /// worker fails.
    pub async fn end_gesture(
        &mut self,
        final_target: Option<[f64; 2]>,
    ) -> Result<SketchUpsertDto, EngineError> {
        self.end_gesture_with_outcome(final_target)
            .await
            .map(|(solved, _)| solved)
    }

    /// [`end_gesture`](Self::end_gesture) plus the exact edit outcome the app
    /// scheduler must consume.
    pub async fn end_gesture_with_outcome(
        &mut self,
        final_target: Option<[f64; 2]>,
    ) -> Result<(SketchUpsertDto, CommandOutcome), EngineError> {
        let gesture = self
            .active_gesture
            .take()
            .ok_or_else(|| op_failed("endGesture: no active gesture"))?;
        let solved = self
            .solver
            .end_gesture(
                &gesture.sketch_id.to_string(),
                gesture.gesture_id,
                final_target,
            )
            .await?;
        let mut after = gesture.before.clone();
        after.apply_solved_positions(&typed_positions(&solved.solved_positions));
        // SP-2: `positions` is point-only. A radius drag moves no point at all, an
        // arcEnd drag reshapes the arc, and a Tangent propagates even a plain point
        // drag into a neighbouring radius — without this the worker's store and the
        // document would silently disagree until the next upsert reverted it.
        after.apply_solved_curves(&typed_curves(&solved.solved_curves));
        let outcome = self
            .apply(EditCommand::SketchDragGesture {
                sketch: gesture.sketch_id,
                before: gesture.before,
                after,
            })
            .map_err(|e| op_failed(format!("endGesture commit: {e}")))?;
        self.record_solve(gesture.sketch_id, &solved);
        Ok((solved, outcome))
    }

    /// Exits sketch mode / cancels an in-flight gesture without committing (SCHEMA
    /// §7.4 — discard scratch). The document is unchanged.
    ///
    /// # Errors
    /// Never fails hard; a best-effort worker `EndGesture` (no commit) is ignored.
    pub async fn cancel_sketch(&mut self, sketch_id: SketchId) -> Result<(), EngineError> {
        if let Some(g) = self.active_gesture.take() {
            // Best-effort: end the worker gesture so it does not leak (no commit).
            let _ = self
                .solver
                .end_gesture(&g.sketch_id.to_string(), g.gesture_id, None)
                .await;
        }
        // B1 squash: the in-flight worker gesture is discarded, but any granular
        // sketch_upsert edits committed during the session collapse into one net
        // undoable command (same as finish) so the user reverts the whole session
        // with a single undo.
        self.squash_sketch_session(sketch_id);
        tracing::info!("cancel_sketch: sketch={sketch_id} squashed (no timeline record)");
        Ok(())
    }

    /// Computes the closed profile regions for a sketch (SCHEMA §7.4 `SketchRegions`)
    /// — the extrude/revolve profile source. Syncs the sketch first so the regions
    /// reflect the latest geometry. Regions are a rebuildable cache, so they are
    /// returned but not persisted (the worker re-derives the same normative
    /// `regionId` during regen).
    ///
    /// # Errors
    /// [`EngineError`] on an unknown sketch or a worker failure.
    pub async fn finish_sketch(
        &mut self,
        sketch_id: SketchId,
    ) -> Result<FinishSketchDto, EngineError> {
        self.finish_sketch_with_outcome(sketch_id)
            .await
            .map(|(dto, _)| dto)
    }

    /// [`finish_sketch`](Self::finish_sketch) plus the timeline-record outcome the
    /// api layer must forward to the regen scheduler (mirrors
    /// [`sketch_upsert_with_outcome`](Self::sketch_upsert_with_outcome)).
    pub async fn finish_sketch_with_outcome(
        &mut self,
        sketch_id: SketchId,
    ) -> Result<(FinishSketchDto, Option<CommandOutcome>), EngineError> {
        let sketch = self.sketch_or_err(sketch_id, "finishSketch")?;
        let solved = self.solver.sketch_upsert(&sketch).await?;
        self.record_solve(sketch_id, &solved);
        let regions = self.solver.sketch_regions(&sketch_id.to_string()).await?;
        // The Enter/E finish handoff can land while a drag is still live (the
        // frontend's pointer-up cancel lost the race) — clear the dangling
        // gesture here too (cancel_sketch's take-once, scoped to this sketch)
        // so it cannot forever block prepare_sketch_regions's guard above nor
        // leak an orphaned gesture on the worker.
        if self
            .active_gesture
            .as_ref()
            .is_some_and(|g| g.sketch_id == sketch_id)
        {
            if let Some(g) = self.active_gesture.take() {
                let _ = self
                    .solver
                    .end_gesture(&g.sketch_id.to_string(), g.gesture_id, None)
                    .await;
            }
        }
        // B1 squash: collapse every in-session granular edit into ONE net command.
        self.squash_sketch_session(sketch_id);
        // The regen plan resolves a modeling op's profile ONLY from a preceding
        // Sketch step, so the finished sketch must own an up-to-date timeline
        // record — without it every extrude off this sketch fails with "profile
        // sketch not found in plan" (the interactive flow authored none before).
        let outcome = self
            .upsert_sketch_record(sketch_id)
            .map_err(|e| op_failed(format!("finishSketch: sketch record: {e}")))?;
        Ok((regions, outcome))
    }

    /// Creates or refreshes the sketch's `Sketch` timeline record from the
    /// current authoritative sketch. Unchanged content is a no-op so an
    /// edit-free finish never dirties regen.
    fn upsert_sketch_record(
        &mut self,
        sketch_id: SketchId,
    ) -> Result<Option<CommandOutcome>, DomainError> {
        let Some(sketch) = self.session.document().sketch(sketch_id).cloned() else {
            return Ok(None);
        };
        let mut op = sketch_record_op(&sketch);
        let existing = self
            .session
            .document()
            .timeline
            .records()
            .iter()
            .find_map(|r| match &r.op {
                Operation::Known(KnownOperation::Sketch(p)) if p.sketch == sketch_id => {
                    Some((r.record_id, r.op.clone()))
                }
                _ => None,
            });
        // VF-B5a anti-time-travel guard: a legacy record backfilled at the FRONT of
        // the timeline can sit ahead of its host body's producer, and stamping
        // `host_face` on it would author an edge `validate_temporal` must reject —
        // turning an ordinary re-finish into a hard error. Drop the stamp for that
        // record instead; the gate's attachment bridge still covers the sketch.
        if let (Some((record_id, _)), Operation::Known(KnownOperation::Sketch(p))) =
            (existing.as_ref(), &mut op)
        {
            let host_body = p.host_face.as_ref().and_then(|f| f.primary.as_ref());
            if let Some(primary) = host_body {
                if !self
                    .session
                    .graph()
                    .produces_before(primary.body, *record_id)
                {
                    tracing::warn!(
                        "sketch record: sketch={sketch_id} record={record_id} precedes its host \
                         body {} — leaving hostFace unstamped (anti-time-travel)",
                        primary.body
                    );
                    p.host_face = None;
                }
            }
        }
        match existing {
            None => {
                let record = OperationRecord::new(RecordId(Uuid::new_v4()), 0, "Sketch", op);
                tracing::info!(
                    "sketch record: MINT sketch={sketch_id} record={}",
                    record.record_id
                );
                self.apply(EditCommand::AddOperation {
                    record,
                    at_cursor: true,
                })
                .map(Some)
            }
            Some((record, old)) if old != op => {
                tracing::info!("sketch record: REFRESH sketch={sketch_id} record={record}");
                self.apply(EditCommand::UpdateOperationParams { record, op })
                    .map(Some)
            }
            Some(_) => {
                tracing::debug!("sketch record: unchanged sketch={sketch_id}");
                Ok(None)
            }
        }
    }

    // ── Element identity (SCHEMA §7.5) ───────────────────────────────────────

    /// Promotes snapshot-scoped TopoKey picks to persistent, globally-unique
    /// `ElementId`s (SCHEMA §7.5 `AcquireElementIds`): the worker returns the
    /// resolved `topoKey → (kind, descriptor, anchor)` evidence and **Rust mints /
    /// owns the ids** ([`mint_element_ids`]). The promotion cache upholds Invariant 1
    /// (re-picking the same `(body, topoKey)` returns the same id) and the binding is
    /// recorded in the document element partition index.
    ///
    /// # Errors
    /// [`EngineError`] on a worker failure, or a recoverable `OpFailed` when the
    /// pick was taken against a snapshot that is no longer the head (VF-M3).
    pub async fn promote_selection(
        &mut self,
        snapshot: SnapshotId,
        body: BodyId,
        picks: Vec<(TopoKey, Option<AnchorIntent>)>,
    ) -> Result<Vec<PromotedElementDto>, EngineError> {
        self.gate_stale_pick(snapshot)?;
        let mut requested_keys: Vec<String> = picks
            .iter()
            .map(|(topo_key, _)| topo_key.as_str().to_string())
            .collect();
        let req = AcquireRequest {
            snapshot_id: snapshot,
            body,
            picks: picks
                .into_iter()
                .map(|(topo_key, anchor)| Pick { topo_key, anchor })
                .collect(),
        };
        let mut evidence = self.engine.acquire_element_ids(req).await?;
        let mut returned_keys: Vec<String> = evidence
            .iter()
            .filter(|entry| entry.body == body)
            .map(|entry| entry.topo_key.as_str().to_string())
            .collect();
        requested_keys.sort();
        returned_keys.sort();
        if returned_keys != requested_keys || evidence.len() != requested_keys.len() {
            return Err(EngineError::OpFailed {
                code: onecad_core::regen::OpFailureCode::RefUnresolved,
                recoverable: true,
                message: "element promotion returned an incomplete or mismatched batch — re-pick"
                    .into(),
                diagnostics: Vec::new(),
            });
        }
        // Rust owns id identity: seed `existing` from the promotion cache so a
        // re-pick of the same (snapshot, body, topoKey) reuses the id (Invariant 1).
        let prev_gen = self.previous_promotion_generation(snapshot);
        for e in &mut evidence {
            if e.existing.is_some() {
                continue;
            }
            if let Some(hit) = self.promoted.get(&(snapshot, e.body, e.topo_key.clone())) {
                e.existing = Some(hit.element_id.clone());
                continue;
            }
            // Cross-generation reuse (Invariant 1 across a regen) is DESCRIPTOR-PINNED.
            // A regen that renumbers ordinals keeps the TopoKey and moves the geometry
            // under it, so the key alone proves nothing; byte-equal worker evidence
            // does. A false negative here just mints a fresh id — the safe direction
            // (a new sketch instead of silently re-entering the wrong one).
            if let Some(prev) = prev_gen {
                if let Some(hit) = self.promoted.get(&(prev, e.body, e.topo_key.clone())) {
                    if hit.kind == e.kind
                        && hit.descriptor.is_some()
                        && hit.descriptor == e.descriptor
                    {
                        e.existing = Some(hit.element_id.clone());
                    }
                }
            }
        }
        let minted = mint_element_ids(evidence);
        let bindings = minted
            .iter()
            .map(|(element_id, evidence)| ElementBinding {
                element_id: element_id.clone(),
                topo_key: evidence.topo_key.clone(),
                body: evidence.body,
                kind: evidence.kind,
                anchor: evidence.anchor.clone(),
            })
            .collect();
        self.engine
            .bind_element_ids(BindElementIdsRequest {
                snapshot_id: snapshot,
                bindings,
            })
            .await?;
        let mut out = Vec::with_capacity(minted.len());
        for (id, ev) in minted {
            self.promoted.insert(
                (snapshot, ev.body, ev.topo_key.clone()),
                PromotionEntry {
                    element_id: id.clone(),
                    kind: ev.kind,
                    descriptor: ev.descriptor.clone(),
                },
            );
            // Record the partition binding into the (regen-mirror) element index,
            // WITH the evidence a fresh session needs to find this element again
            // (DI-4). The anchor and descriptor are already in hand here and were
            // previously dropped on the floor, which is why an authored face colour
            // survived a save and then had nothing to paint after a reopen — the
            // worker's partition is minted on demand and dies with the process.
            self.regen.elements.insert(
                id.clone(),
                ElementEntry::new(ev.body, ev.kind)
                    .with_evidence(ev.anchor.clone(), ev.descriptor.clone()),
            );
            out.push(PromotedElementDto {
                topo_key: ev.topo_key.as_str().to_string(),
                element_id: id.as_str().to_string(),
                kind: kind_str(ev.kind).to_string(),
                body_id: crate::worker::wire::body_id_wire(ev.body),
            });
        }
        // `regen.elements` is PERSISTED — `build_save_payload` writes it out as
        // `doc.elements`. Marking dirty here (rather than at the two commands that
        // reach this method) is what keeps the class closed: any future caller of
        // `promote_selection` inherits it. Without this the promotion rode along in
        // the next save while `dirty` stayed false, so the close prompt was skipped
        // and — once the autosave learned to gate on dirtiness — the promotion was
        // never autosaved at all.
        //
        // NOT on a read-only document. `apply` refuses those outright, so nothing
        // else can dirty one, and a promotion is identity plumbing rather than user
        // intent: claiming unsaved work on a document that cannot be saved would arm
        // a crash marker and offer a face pick back as "unsaved changes".
        if !out.is_empty() && !self.read_only {
            self.dirty = true;
        }
        Ok(out)
    }

    /// Refuses a promotion whose pick was taken against a snapshot that is no
    /// longer the head (VF-M3).
    ///
    /// A `TopoKey` is a 1-based ordinal into the snapshot's shape map, so the SAME
    /// key names a DIFFERENT face once a regen renumbers the ordinals. Promoting it
    /// anyway mints a persistent, op-referencable id for geometry the user never
    /// picked — the H5-B defect class this whole migration exists to kill. A loud
    /// recoverable refusal ("re-pick") is the only safe answer.
    ///
    /// **Skipped when there is no positive head.** `latest_snapshot` is `None`
    /// before the first publish, and a document *clear* publishes `SnapshotId(0)`
    /// (`drive_clear` — no `AcceptPrepared` happened, so there is no worker snapshot
    /// id to address). The frontend only ever adopts a POSITIVE published id, so
    /// gating on `0` would refuse every legitimate first pick.
    ///
    /// The code mirrors the worker's own refusal (`REF_UNRESOLVED`, SCHEMA §7.5) so
    /// the two halves of the gate report the same thing; both surface to the
    /// frontend as `ApiError::OpFailed`.
    /// The id of the last PUBLISHED snapshot, or `None` before the first
    /// publish. A promotion made server-side on the caller's behalf (the
    /// library's mate authoring, WP-H2) addresses this head — it has no older
    /// pick snapshot to be stale against.
    pub fn head_snapshot_id(&self) -> Option<SnapshotId> {
        self.latest_snapshot.as_ref().map(|s| s.id)
    }

    fn gate_stale_pick(&self, snapshot: SnapshotId) -> Result<(), EngineError> {
        let Some(head) = &self.latest_snapshot else {
            return Ok(());
        };
        if head.id == SnapshotId(0) || head.id == snapshot {
            return Ok(());
        }
        Err(EngineError::OpFailed {
            code: onecad_core::regen::OpFailureCode::RefUnresolved,
            recoverable: true,
            message: format!(
                "pick was taken against snapshot {}; head is {} — re-pick",
                snapshot.0, head.id.0
            ),
            diagnostics: Vec::new(),
        })
    }

    /// The newest promotion-cache generation strictly older than `snapshot`, or
    /// `None` when the cache holds nothing older.
    ///
    /// [`prune_promoted`](Self::prune_promoted) keeps at most two generations, so
    /// this is the single "previous" generation the descriptor-pinned reuse rung in
    /// [`promote_selection`](Self::promote_selection) consults. Derived from the
    /// cache rather than tracked in a field so the two can never disagree.
    fn previous_promotion_generation(&self, snapshot: SnapshotId) -> Option<SnapshotId> {
        self.promoted
            .keys()
            .map(|(s, _, _)| *s)
            .filter(|s| *s < snapshot)
            .max()
    }

    /// Drops promotion-cache entries older than the `{current, previous}` snapshot
    /// generations (VF-M4).
    ///
    /// Two generations is what the descriptor-pinned reuse rung needs and no more:
    /// a promoted-but-unconsumed element older than that re-mints a FRESH id on the
    /// next pick. That is the safe direction — a false negative creates a new sketch
    /// instead of re-entering the wrong one — and it bounds a map that would
    /// otherwise grow once per pick per snapshot for the life of the document.
    fn prune_promoted(&mut self, current: SnapshotId, previous: Option<SnapshotId>) {
        self.promoted
            .retain(|(s, _, _), _| *s == current || Some(*s) == previous);
    }

    /// Dry-run ladder resolution for repair dialogs (SCHEMA §7.5 `ResolveRefs`) —
    /// binds nothing.
    ///
    /// The lean `needs-repair` event carries no `ElementRef`, so the repair panel
    /// dry-runs with `refId` ONLY (an empty ref). Such a request is hydrated from the
    /// STORED ref at that op-input slot — the refId grammar `<recordId>.input<k>` the
    /// worker's `PlanExecutor` mints (`<opId>.input<i>`, `opId` = the record uuid) —
    /// so the ladder resolves against the FULL authored evidence (primary + anchor +
    /// intent) instead of an empty ref against an empty body ("No candidates" even when
    /// candidates exist). A request that already carries an `element` is left untouched.
    ///
    /// # Errors
    /// [`EngineError`] on a worker failure.
    pub async fn resolve_refs(
        &self,
        mut req: ResolveRequest,
    ) -> Result<Vec<RefResolution>, EngineError> {
        // Candidate TopoKeys are ordinals into one snapshot. Unlike a generic
        // advisory read, this response can be promoted, so never enumerate a
        // newer head under the caller's older snapshot provenance.
        self.gate_stale_pick(req.snapshot_id)?;
        for r in &mut req.refs {
            if element_ref_is_empty(&r.element) {
                if let Some(stored) = self.stored_input_ref(&r.ref_id) {
                    r.element = stored;
                }
            }
        }
        self.engine.resolve_refs(req).await
    }

    /// Whether a DI-4 re-bind pass is still owed: this session has not attempted one
    /// and the document carries persisted elements with re-bind evidence.
    ///
    /// Cheap by construction — a map scan, no worker traffic — because the caller is
    /// every published regen.
    #[must_use]
    pub fn has_unbound_persisted_elements(&self) -> bool {
        !self.rebind_attempted
            && self
                .regen
                .elements
                .iter()
                .any(|(_, e)| e.has_rebind_evidence())
    }

    /// Re-bind persisted [`ElementId`]s into a FRESH session's partition (DI-4).
    ///
    /// The worker's element map is minted on demand and dies with the process, so
    /// after a reopen nothing knows which face a persisted id names: an authored
    /// face colour is still in the document, and both frontend paint paths come up
    /// empty because `elementInfo` answers `None`. `BindElementIds` had exactly one
    /// production caller — `promote_selection` — and nothing re-bound at open.
    ///
    /// Each entry that carries evidence is resolved through the SAME §7.5 ladder
    /// every other reference uses, and only an `AutoBind` — the worker's own
    /// confidence gate, score ≥ 0.85 and margin ≥ 0.10 — installs the stored id on
    /// the resolved `TopoKey`. Anything else is left unbound and reported: the
    /// colour stays in the file and goes unpainted, which is the honest outcome.
    /// Binding a best guess here would be the silent wrong-bind (H5-B) this whole
    /// migration exists to eliminate.
    ///
    /// Returns `(bound, unresolved)` counts. Never fails the open: a worker error
    /// leaves every id unbound, exactly as before this existed — but re-arms the
    /// pass (bounded), since a transport failure is not a ladder answer.
    pub async fn rebind_persisted_elements(&mut self) -> (usize, usize) {
        self.rebind_attempted = true;
        let Some(snapshot) = self.head_snapshot_id() else {
            return (0, 0);
        };
        // Only entries with evidence, and only ones the CURRENT head still has a
        // body for — a ref into a body that no longer exists has nothing to
        // enumerate against, and the ladder would just report that back at the cost
        // of a round trip. The skip is still worth a line in the log: a stale
        // colour on a deleted body would otherwise vanish without a trace.
        let mut skipped_missing_body = 0usize;
        let pending: Vec<(ElementId, ElementEntry)> = self
            .regen
            .elements
            .iter()
            .filter(|(_, e)| e.has_rebind_evidence())
            .filter(|(_, e)| {
                let present = self.regen.bodies.contains(e.body);
                if !present {
                    skipped_missing_body += 1;
                }
                present
            })
            .map(|(id, e)| (id.clone(), e.clone()))
            .collect();
        if skipped_missing_body > 0 {
            tracing::debug!(
                target: "onecad_lib::document_runtime",
                skipped = skipped_missing_body,
                "rebind: persisted elements skipped — their body is not in the current head"
            );
        }
        if pending.is_empty() {
            return (0, 0);
        }

        // The ref is authored exactly like a stored op input: last-known identity in
        // `primary`, the frozen worker descriptor in `intent`, and the pick anchor.
        // Passing the anchor alone would be an EMPTY ref for most entries — the
        // worker's `AcquireElementIds` answers with a descriptor and no anchor for a
        // programmatic pick — and the ladder would have nothing to work with.
        let refs = pending
            .iter()
            .map(|(id, entry)| ResolveRef {
                ref_id: id.as_str().to_string(),
                element: ElementRef {
                    primary: Some(PrimaryRef {
                        body: entry.body,
                        element: id.clone(),
                        kind: entry.kind,
                        extra: Default::default(),
                    }),
                    intent: entry.descriptor.clone().map(|descriptor| IntentQuery {
                        // The same `descriptorVersion` every plan is compiled with,
                        // so a future bump invalidates stored evidence through one
                        // knob (SCHEMA §6/§13) — see `stamp_intents`.
                        version: PolicyVersions::default().descriptor,
                        kind: entry.kind,
                        descriptor,
                        extra: Default::default(),
                    }),
                    anchor: entry.anchor.clone(),
                    extra: Default::default(),
                },
            })
            .collect();
        let resolutions = match self
            .engine
            .resolve_refs(ResolveRequest {
                snapshot_id: snapshot,
                refs,
            })
            .await
        {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(
                    target: "onecad_lib::document_runtime",
                    error = %err,
                    pending = pending.len(),
                    "rebind: ladder resolution failed; persisted element ids stay unbound"
                );
                self.note_rebind_transport_failure();
                return (0, pending.len());
            }
        };

        let by_entry: std::collections::HashMap<&str, &ElementEntry> = pending
            .iter()
            .map(|(id, entry)| (id.as_str(), entry))
            .collect();
        let mut bindings = Vec::new();
        let mut unresolved = 0usize;
        for res in &resolutions {
            let Some(entry) = by_entry.get(res.ref_id.as_str()) else {
                continue;
            };
            match &res.outcome {
                // The dry run leaves `element_id` EMPTY when the element is not in
                // the partition yet — which is every element right after a reopen —
                // so the TopoKey is what carries the answer here.
                ResolveOutcome::AutoBind {
                    topo_key: Some(key),
                    ..
                } => bindings.push(ElementBinding {
                    element_id: ElementId::new(res.ref_id.clone()),
                    topo_key: key.clone(),
                    body: entry.body,
                    kind: entry.kind,
                    anchor: entry.anchor.clone(),
                }),
                _ => unresolved += 1,
            }
        }
        let bound = bindings.len();
        if !bindings.is_empty() {
            if let Err(err) = self
                .engine
                .bind_element_ids(BindElementIdsRequest {
                    snapshot_id: snapshot,
                    bindings,
                })
                .await
            {
                tracing::warn!(
                    target: "onecad_lib::document_runtime",
                    error = %err,
                    "rebind: BindElementIds failed; persisted element ids stay unbound"
                );
                self.note_rebind_transport_failure();
                return (0, pending.len());
            }
        }
        if unresolved > 0 {
            tracing::info!(
                target: "onecad_lib::document_runtime",
                bound,
                unresolved,
                "rebind: some persisted element ids did not resolve confidently and stay unbound"
            );
        }
        (bound, unresolved)
    }

    /// A transport failure re-arms [`rebind_persisted_elements`] for the next
    /// published regen — the ladder never answered, so "attempted" would be a
    /// false terminal — bounded by [`MAX_REBIND_TRANSPORT_FAILURES`] so a
    /// persistently failing worker costs a fixed number of round trips, not one
    /// per publish for the whole session.
    fn note_rebind_transport_failure(&mut self) {
        self.rebind_transport_failures = self.rebind_transport_failures.saturating_add(1);
        if self.rebind_transport_failures < MAX_REBIND_TRANSPORT_FAILURES {
            self.rebind_attempted = false;
        }
    }

    /// The stored ref body's identity is the body used by repair candidate
    /// enumeration. It accompanies the snapshot/revision provenance to the UI.
    #[must_use]
    pub fn repair_candidate_body(&self, ref_id: &str) -> Option<String> {
        self.stored_input_ref(ref_id)
            .and_then(|reference| reference.primary)
            .map(|primary| primary.body.to_string())
    }

    /// The STORED [`ElementRef`] at the op-input slot a repair `refId`
    /// (`<recordId>.input<k>`) names, or `None` when the id does not parse, the record
    /// is unknown, or that slot carries no typed element ref. Hydrates a lean
    /// refId-only [`resolve_refs`](Self::resolve_refs) request.
    fn stored_input_ref(&self, ref_id: &str) -> Option<ElementRef> {
        let (record_id, index) = parse_input_ref_id(ref_id)?;
        let record = self
            .regen
            .timeline
            .records()
            .iter()
            .find(|r| r.record_id == record_id)?;
        element_ref_input(&record.op, index).cloned()
    }

    /// The stored op's params as the serde JSON the `EditCommand` `op.params` path
    /// accepts (camelCase; `Scalar` = `{value}`), for a scalar parametric re-edit that
    /// must PRESERVE the op's non-scalar inputs (revolve `axis` / shell `openFaces` /
    /// fillet `edges`) rather than rebuild params from scratch. `None` when the record
    /// is unknown or its op carries no `params` object.
    #[must_use]
    pub fn operation_params(&self, record: RecordId) -> Option<serde_json::Value> {
        let rec = self
            .regen
            .timeline
            .records()
            .iter()
            .find(|r| r.record_id == record)?;
        let op = serde_json::to_value(&rec.op).ok()?;
        op.get("params").cloned()
    }

    // ── Sketch-flow helpers ──────────────────────────────────────────────────

    fn sketch_or_err(&self, id: SketchId, verb: &str) -> Result<Sketch, EngineError> {
        self.session
            .document()
            .sketch(id)
            .cloned()
            .ok_or_else(|| op_failed(format!("{verb}: unknown sketch {id}")))
    }

    /// Collapses all in-session granular sketch edits into ONE net undoable
    /// command (B1). Consumes the [`SketchSession`] watermark set by
    /// [`enter_sketch`](Self::enter_sketch) and asks the session to squash every
    /// undo step committed since. A crash between enter and finish/cancel leaves
    /// the granular steps (acceptable — no squash without a live watermark).
    fn squash_sketch_session(&mut self, sketch_id: SketchId) {
        match self.sketch_session.as_ref() {
            // Only the matching session is consumed; a different sketch's open
            // session (a frontend ordering bug) is left intact.
            Some(session) if session.sketch_id == sketch_id => {}
            _ => return,
        }
        let session = self.sketch_session.take().expect("checked Some above");
        // Finding 3: an eviction past the undo cap since enter shifted the stack bottom,
        // so the depth-based watermark no longer addresses the session's steps. Refuse
        // the squash wholesale — the granular steps stay (safe, noisier stack). The core
        // squash's contiguity guard can't catch this: the newest `count` may still all be
        // sketch edits, but `count` now under-counts the session and would strand its
        // earliest edits below the net-inverse.
        if self.session.evictions() != session.evicted_at_enter {
            return;
        }
        let count = self
            .session
            .undo_depth()
            .saturating_sub(session.undo_watermark);
        self.session
            .squash_sketch_session(sketch_id, session.prior, count);
    }

    fn record_solve(&mut self, sketch: SketchId, solved: &SketchUpsertDto) {
        self.sketch_solve
            .insert(sketch, (solved.dof, solved.status));
    }

    fn next_gesture_id(&mut self) -> u64 {
        self.gesture_seq += 1;
        self.gesture_seq
    }
}

/// A compiled, fenced regen ready to drive **lock-free** (phase 2). Produced by
/// [`DocumentRuntime::begin_regen`] under the lock; [`drive`](PreparedRegen::drive)
/// runs the executor on the cloned scratch with the runtime lock released, so a
/// concurrent edit can advance the fencing tokens and supersede a stale prepare.
pub struct PreparedRegen {
    work: PreparedWork,
    scratch: RegenSession,
    fencing: Arc<FencingCell>,
    publisher: Arc<SnapshotPublisher>,
    /// The [`DocumentRuntime::instance`] this regen was compiled against — phase 3
    /// commits only back into that same runtime.
    instance: Uuid,
    expected: (
        onecad_core::ids::DocumentRevision,
        onecad_core::ids::WorkerEpoch,
    ),
    lod: Lod,
    prior: Vec<BodyId>,
    /// The records this regen executes — the scope
    /// [`DocumentRuntime::sync_record_outputs`] is allowed to overwrite (TRUST F1).
    executed: BTreeSet<RecordId>,
    /// Correlation key for the whole phase 1→3 chain; `None` on the CLEAR path
    /// (no plan was compiled, so no job exists — the lane renders `job=clear`).
    job: Option<JobId>,
    /// Short `expectedBaseHash` prefix — the fencing evidence a regen line is read
    /// against. Empty on the CLEAR path (no worker round-trip, no base hash).
    base_hash_prefix: String,
    /// Number of compiled ops (0 on the CLEAR path).
    step_count: usize,
    /// Collects the MESH1 blobs the worker inlined into the `ExecutePlan` terminal.
    /// Drained into the runtime's [`MeshCache`] by
    /// [`finish_regen`](DocumentRuntime::finish_regen) — and ONLY inside the branch
    /// that commits, so a superseded regen's geometry is dropped with the sink.
    /// Always empty on the CLEAR path (no worker round-trip).
    mesh_sink: MeshSink,
    /// How long phase 1 spent compiling this plan (see [`RegenTimings`]).
    planner_ms: u64,
    /// Where the wrapped engine books its worker round trips during phase 2.
    clock: Arc<EngineClock>,
}

/// Renders an optional [`JobId`] for the regen lane: the bare uuid, or `clear` for
/// the worker-less CLEAR publish.
#[derive(Clone, Copy)]
struct JobLabel(Option<JobId>);

impl std::fmt::Display for JobLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(job) => write!(f, "{}", job.0),
            None => f.write_str("clear"),
        }
    }
}

/// The first 12 chars of a hash — enough to correlate `begin_regen` ↔ `regen.drive`
/// ↔ the postmortem line without pasting 64 hex chars onto every event.
fn hash_prefix(hash: &str) -> String {
    hash.chars().take(12).collect()
}

/// The `regen:` postmortem contract — emitted on EVERY finish path (published,
/// window-race supersede, failed/cancelled/noop), plus one `warn` per failed step.
/// `finish_regen` is the single funnel for that, so the inline `run_regen` (tests,
/// headless CLI) and the scheduler-driven app path log identically.
fn log_regen_outcome(job: JobLabel, base: &str, steps: usize, report: &RegenReport) {
    tracing::info!(
        job = %job,
        rev = report.revision,
        srcRev = report.source_revision,
        snapshot = report.snapshot_id,
        base = %base,
        steps,
        changed = report.changed.len(),
        removed = report.removed.len(),
        failedSteps = report.failed_steps.len(),
        needsRepair = report.needs_repair.len(),
        "regen: {}",
        report.outcome_str()
    );
    for step in &report.failed_steps {
        tracing::warn!(
            job = %job,
            record = %step.record_id,
            reason = %step.message,
            "regen: FAILED step"
        );
    }
    for diagnostic in report.diagnostics.iter().take(64) {
        tracing::warn!(
            job = %job,
            code = %diagnostic.code,
            stage = diagnostic.stage.as_deref().unwrap_or(""),
            evidence = ?diagnostic.evidence,
            message = %diagnostic.message,
            "regen: diagnostic"
        );
    }
}

/// What a [`PreparedRegen`] actually does in phase 2.
enum PreparedWork {
    /// Drive a compiled plan over the worker (the normal path).
    Plan {
        plan_req: Box<PlanRequest>,
        engine: Box<AdoptingEngine>,
    },
    /// Publish an EMPTY result Rust-side (TRUST F2): every op the request covers is
    /// suppressed (or nothing is applied), so the target geometry is "no bodies" and
    /// there is nothing for the worker to compute. `target` is the number of leading
    /// timeline steps the clear spans (for the published `step_states`).
    Clear { target: usize },
}

impl PreparedRegen {
    /// Drives the plan to its terminal with the runtime lock **released**. The
    /// executor's [`RevisionGate`](onecad_core::regen::RevisionGate) reads the live
    /// [`FencingCell`], so an edit that lands during worker IO is observed at accept
    /// time (fencing live). Returns the driven result for
    /// [`DocumentRuntime::finish_regen`].
    pub async fn drive(self, cancel: CancelToken) -> DrivenRegen {
        let PreparedRegen {
            work,
            mut scratch,
            fencing,
            publisher,
            instance,
            expected,
            lod,
            prior,
            executed,
            job,
            base_hash_prefix,
            step_count,
            mesh_sink,
            planner_ms,
            clock,
        } = self;
        // The phase-2 span. Everything the executor awaits nests inside it — INCLUDING
        // the `onecad_protocol::frames` tx/rx events, which is the join from a regen to
        // its OCW1 `reqId`s. The forwarded worker-stderr lane does NOT nest (it is a
        // detached per-spawn task); join that one through the frame trace's `reqId`.
        let span = tracing::info_span!(
            "regen.drive",
            job = %JobLabel(job),
            rev = expected.0 .0,
            epoch = expected.1 .0,
            base = %base_hash_prefix,
            steps = step_count,
        );
        let started = std::time::Instant::now();
        let outcome = match work {
            PreparedWork::Plan { plan_req, engine } => {
                let gate = move || fencing.get();
                // The executor deposits the terminal prepare's inline meshes here on a
                // PUBLISH; phase 3 decides whether that publish actually commits.
                let executor = RegenExecutor::new(*engine).with_mesh_sink(mesh_sink.clone());
                executor
                    .run(*plan_req, &mut scratch, &gate, &cancel, &publisher)
                    .instrument(span.clone())
                    .await
            }
            PreparedWork::Clear { target } => {
                span.in_scope(|| drive_clear(&mut scratch, &publisher, &cancel, target))
            }
        };
        // The phase split (measurement only — see `RegenTimings`). Read AFTER the
        // executor returned, so every worker call this regen made is already booked.
        let timings = RegenTimings {
            planner_ms,
            worker_ms: clock.exec_ns() / 1_000_000,
            mesh_ms: clock.mesh_ns() / 1_000_000,
        };
        span.in_scope(|| {
            tracing::info!(
                outcome = outcome_label(&outcome),
                // Machine timing: the span-close `time.busy` is a unit-suffixed STRING.
                elapsed_ms = started.elapsed().as_millis() as u64,
                planner_ms = timings.planner_ms,
                worker_ms = timings.worker_ms,
                mesh_ms = timings.mesh_ms,
                "regen.drive: done"
            );
        });
        DrivenRegen {
            outcome,
            scratch,
            prior,
            instance,
            expected,
            lod,
            executed,
            job,
            base_hash_prefix,
            step_count,
            mesh_sink,
            timings,
        }
    }
}

/// The terminal label of a driven [`Outcome`] (the same vocabulary
/// [`RegenReport::outcome_str`] publishes, before a report exists).
fn outcome_label(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Published(_) => "published",
        Outcome::Superseded => "superseded",
        Outcome::EngineFailed(_) => "failed",
        Outcome::Cancelled => "cancelled",
        Outcome::NoOp => "noop",
    }
}

/// Phase 2 for [`PreparedWork::Clear`]: empties the scratch geometry state and
/// publishes a body-less [`ModelSnapshot`] (TRUST F2). No worker call, so a cancel
/// requested before it starts still wins deterministically.
///
/// `step_index` is `None` — the snapshot represents "base only", which also keeps
/// [`DocumentRuntime::take_checkpoint_at_head`] from minting a checkpoint over it. The
/// per-step states are the mirror's own (already `Suppressed` for every suppressed
/// record — [`Timeline::from_records`] derives them from the record flag).
fn drive_clear(
    scratch: &mut RegenSession,
    publisher: &SnapshotPublisher,
    cancel: &CancelToken,
    target: usize,
) -> Outcome {
    if cancel.is_cancelled() {
        return Outcome::Cancelled;
    }
    scratch.bodies = BodyRegistry::new();
    scratch.elements = onecad_core::document::element_index::ElementIndex::new();
    scratch.repair.clear();
    let step_states: Vec<(usize, StepState)> = (0..target)
        .filter_map(|s| scratch.timeline.state(s).map(|st| (s, st.clone())))
        .collect();
    let snapshot = publisher.publish(|generation| ModelSnapshot {
        // No `AcceptPrepared` happened, so there is no worker snapshot id. `0` is the
        // same "nothing to address" value `noop_report` uses; with zero bodies there is
        // nothing to promote or fetch a mesh for.
        id: SnapshotId(0),
        generation,
        step_index: None,
        bodies: Vec::new(),
        stopped_reason: onecad_core::regen::StoppedReason::Completed,
        step_states,
        signatures: None,
        diagnostics: Vec::new(),
        diagnostics_by_step: BTreeMap::new(),
        repair_summary: onecad_core::regen::RepairSummary::default(),
    });
    Outcome::Published(snapshot)
}

/// The result of driving a [`PreparedRegen`] lock-free (phase 2 → 3 handoff).
pub struct DrivenRegen {
    outcome: Outcome,
    scratch: RegenSession,
    prior: Vec<BodyId>,
    /// Carried verbatim from [`PreparedRegen::instance`] — the cross-document fence.
    instance: Uuid,
    expected: (
        onecad_core::ids::DocumentRevision,
        onecad_core::ids::WorkerEpoch,
    ),
    lod: Lod,
    executed: BTreeSet<RecordId>,
    /// Correlation fields threaded from [`PreparedRegen`] so phase 3's postmortem
    /// line carries the SAME keys phase 1/2 logged (see [`PreparedRegen`]).
    job: Option<JobId>,
    base_hash_prefix: String,
    step_count: usize,
    /// The inline meshes phase 2 collected — see [`PreparedRegen::mesh_sink`].
    mesh_sink: MeshSink,
    /// The phase-1 + phase-2 wall-clock split, measured by [`PreparedRegen::drive`].
    timings: RegenTimings,
}

/// Logs once when an opened container still carries the pre-V2 `checkpoints/` cache.
/// The entries are ignored (checkpoints are in-session acceleration only, SCHEMA §7.7)
/// and the next save drops them, so the container shrinks on its own.
fn log_legacy_checkpoint_cache(loaded: &LoadedContainer) {
    let stale = loaded
        .cache_entries()
        .iter()
        .filter(|e| e.path.starts_with(CHECKPOINTS_DIR))
        .count();
    if stale > 0 {
        tracing::info!(
            "open: {stale} legacy container checkpoint cache entries ignored \
             (in-session-only policy) — shrinks on next save"
        );
    }
}

/// Repopulates the wire split-id interner from a registry's persisted `split_of`
/// entries (the cross-process fix — see [`DocumentRuntime::from_document`]). A body
/// with no split origin is skipped. Idempotent + deterministic (the derived uuid is a
/// pure function of `(opId, k)`).
fn reintern_split_children(bodies: &[onecad_core::document::body::BodyMeta]) {
    for b in bodies {
        if let Some(split) = &b.split_of {
            let derived = crate::worker::wire::intern_split_child(split.op.as_uuid(), split.k);
            debug_assert_eq!(
                derived, b.id,
                "persisted split_of must re-derive the stored BodyId (deterministic)"
            );
        }
    }
}

/// WP-VE.1: reports every step whose `Scalar` expression failed to resolve. A
/// broken variable binding silently reverting to a stale number is exactly the
/// failure mode this pass removes, so it is a WARN on the regen lane, not a
/// debug line.
fn log_unresolved_variables(unresolved: &BTreeMap<usize, UnresolvedVariable>, lane: &str) {
    for item in unresolved.values() {
        tracing::warn!(
            lane,
            step = item.step_index,
            record = %item.record_id,
            expr = %item.expr,
            "regen: variable binding UNRESOLVED — {} (the step will not execute)",
            item.message
        );
    }
}

/// The timeline records left in `StepState::Error` after a regen, as
/// `(recordId, reason)` [`FailedStep`]s — the frontend `failedSteps` correlation
/// source (MODEL-HARDEN finding 1). A published from-0 regen can leave the newly
/// committed op in Error while republishing OTHER bodies; without this the awaiter
/// would read that as a blanket commit success.
fn failed_steps_of(
    timeline: &Timeline,
    diagnostics_by_step: &BTreeMap<usize, Vec<onecad_core::regen::Diagnostic>>,
) -> Vec<FailedStep> {
    timeline
        .records()
        .iter()
        .enumerate()
        .filter_map(|(i, rec)| match timeline.state(i) {
            Some(StepState::Error { reason }) => Some(FailedStep {
                record_id: rec.record_id.to_string(),
                message: reason.clone(),
                diagnostics: diagnostics_by_step
                    .get(&i)
                    .map(|items| items.iter().take(64).cloned().collect())
                    .unwrap_or_default(),
            }),
            _ => None,
        })
        .collect()
}

/// The tree label for an `ImportStep` record: the source basename with its
/// extension dropped, so `bracket.step` reads as `Import bracket` rather than as a
/// path. Falls back to a bare `"Import"` for a nameless source.
fn import_record_name(source_name: &str) -> String {
    let stem = Path::new(source_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if stem.is_empty() {
        "Import".to_string()
    } else {
        format!("Import {stem}")
    }
}

/// The regen mirror's registry **wholesale** with the document's user-authored
/// `name`/`visible` overlaid per id.
///
/// Membership, creation order, the lifecycle `log` and the retired→survivor `aliases`
/// are the regen's — geometry decides which bodies exist, and `resolve`/`merge_winner`
/// read the log and aliases, so they MUST survive the merge (this is why the regen
/// registry is cloned rather than rebuilt from the document rows). Only `name` and
/// `visible` are user intent, and only for a body the regen still carries: the two
/// setters no-op for an unknown id.
fn merge_body_metadata(regen: &BodyRegistry, doc: &BodyRegistry) -> BodyRegistry {
    let mut merged = regen.clone();
    for meta in doc.bodies() {
        merged.set_name(meta.id, meta.name.clone());
        merged.set_visible(meta.id, meta.visible);
        merged.set_color(meta.id, meta.color);
        // Face colors are user intent; overlay them onto the regen mirror so the
        // projection and the saved registry both carry them.
        merged.set_face_colors(meta.id, meta.face_colors.clone());
    }
    merged
}

/// [`merge_body_metadata`] **plus** every document-only row (an id the regen mirror no
/// longer carries), appended verbatim — the registry a SAVE writes (TRUST F4).
///
/// The projection deliberately does NOT use this: membership there is the regen
/// mirror's alone, so a row with no live body never renders as a phantom, mesh-less
/// tree entry. But durability is a different question. Suppress a feature and its body
/// leaves the mirror while the user's `RenameBody` / `SetVisibility` survive only in
/// `document.bodies`; a save that took membership wholesale from the mirror dropped that
/// row on the floor, so re-opening and un-suppressing brought the body back with the
/// re-derived `"Body N"` default and the rename was silently lost. Persisting the row
/// costs one JSON object and is exactly what
/// [`adopt_regen_bodies`](onecad_core::edit::DocumentSession::adopt_regen_bodies) — which
/// is insert-only — needs on reopen to re-overlay the returning body.
fn merge_body_metadata_for_save(regen: &BodyRegistry, doc: &BodyRegistry) -> BodyRegistry {
    let mut merged = merge_body_metadata(regen, doc);
    for meta in doc.bodies() {
        if !merged.contains(meta.id) {
            merged.register(meta.clone());
        }
    }
    merged
}

/// The rows of a loaded `document.bodies` that seed the regen mirror at open: every row
/// EXCEPT those produced by a currently-suppressed record or a removed V2 pattern tail.
///
/// The mirror is seeded from the persisted registry so a reopen renders saved geometry
/// before the first regen. Since a save now also persists the metadata rows of bodies
/// whose feature is suppressed, seeding wholesale would put those back into the mirror —
/// and therefore into the projection — as phantom, mesh-less tree entries until the first
/// regen dropped them again. A suppressed producer means the next regen will not produce
/// the body, so excluding it keeps "no live body ⇒ no tree row" true across save/reopen
/// while the row itself stays in `document.bodies` for its name/visibility.
fn seed_regen_bodies(doc: &Document) -> BodyRegistry {
    let suppressed: HashSet<RecordId> = doc
        .timeline
        .records()
        .iter()
        .filter(|r| r.suppressed)
        .map(|r| r.record_id)
        .collect();
    let active_v2_pattern_children: HashSet<BodyId> = doc
        .timeline
        .records()
        .iter()
        .filter_map(|record| match &record.op {
            Operation::Known(KnownOperation::LinearPattern(params))
                if params.result_policy_version == Some(2)
                    && !params.fuse_result
                    && !record.suppressed =>
            {
                Some(record.outputs.iter().copied())
            }
            Operation::Known(KnownOperation::CircularPattern(params))
                if params.result_policy_version == Some(2)
                    && !params.fuse_result
                    && !record.suppressed =>
            {
                Some(record.outputs.iter().copied())
            }
            _ => None,
        })
        .flatten()
        .collect();
    if suppressed.is_empty() && active_v2_pattern_children.is_empty() {
        return doc.bodies.clone();
    }
    let mut seeded = doc.bodies.clone();
    for meta in doc.bodies.bodies().to_vec() {
        let is_v2_pattern_child = doc.timeline.records().iter().any(|record| {
            if record.record_id != meta.created_by {
                return false;
            }
            match &record.op {
                Operation::Known(KnownOperation::LinearPattern(params)) => {
                    params.result_policy_version == Some(2) && !params.fuse_result
                }
                Operation::Known(KnownOperation::CircularPattern(params)) => {
                    params.result_policy_version == Some(2) && !params.fuse_result
                }
                _ => false,
            }
        });
        if suppressed.contains(&meta.created_by)
            || (is_v2_pattern_child && !active_v2_pattern_children.contains(&meta.id))
        {
            seeded.remove(meta.id);
        }
    }
    seeded
}

/// Per-record-id, the bodies an op CREATED or MODIFIED in the committed regen —
/// derived from the body lifecycle log folded during THIS regen (finding 1). Created
/// (incl. split children `body_<opId>:<k>`, mapped to their derived uuids), Modified
/// (Add/Cut in-place target), Split children and a Merge winner all count; a Deleted
/// parent contributes nothing. Body ids render in the SAME wire form as
/// `document-changed`'s `changedBodies` (`BodyId` Display) so the frontend can match a
/// commit's recordId to a real published body. An op that only deleted a body yields
/// no entry (empty vecs are never inserted).
fn affected_bodies_of(timeline: &Timeline, bodies: &BodyRegistry) -> BTreeMap<String, Vec<String>> {
    produced_bodies_of(timeline, bodies)
        .into_iter()
        .map(|(rec, bs)| (rec.to_string(), bs.iter().map(BodyId::to_string).collect()))
        .collect()
}

/// The typed form of [`affected_bodies_of`]: per [`RecordId`], the bodies that op
/// created or modified in the committed regen, in lifecycle-log order and deduped.
/// Also the `OperationRecord::outputs` source (see
/// [`DocumentRuntime::sync_record_outputs`]).
fn produced_bodies_of(
    timeline: &Timeline,
    bodies: &BodyRegistry,
) -> BTreeMap<RecordId, Vec<BodyId>> {
    let records = timeline.records();
    let mut map: BTreeMap<RecordId, Vec<BodyId>> = BTreeMap::new();
    for entry in bodies.log() {
        let Some(rec) = records.get(entry.step_index) else {
            continue;
        };
        let touched: Vec<BodyId> = match &entry.event {
            BodyLifecycleEvent::Created { body } | BodyLifecycleEvent::Modified { body } => {
                vec![*body]
            }
            BodyLifecycleEvent::Split { children, .. } => children.clone(),
            BodyLifecycleEvent::Merged { winner, .. } => vec![*winner],
            BodyLifecycleEvent::Deleted { .. } => Vec::new(),
        };
        if touched.is_empty() {
            continue;
        }
        let slot = map.entry(rec.record_id).or_default();
        for b in touched {
            if !slot.contains(&b) {
                slot.push(b);
            }
        }
    }
    map
}

/// Stable geometry identity for one sketch projection. Deliberately excludes
/// name, visibility, solver status, and the non-authoritative region cache:
/// changing those must not force a profile refetch. Authoritative ordered
/// geometry and constraints are serde-stable and finite by domain invariant.
fn sketch_geometry_token(sketch: &Sketch) -> String {
    let geometry = (
        "onecad-sketch-geometry-v1",
        &sketch.plane,
        &sketch.attachment,
        sketch.entities(),
        sketch.constraints(),
    );
    let bytes = serde_json::to_vec(&geometry)
        .expect("validated sketch geometry must serialize deterministically");
    format!("{:x}", Sha256::digest(bytes))
}

/// The host-face identity of a face-attached sketch, for the projection.
///
/// Only the `primary` binding is projected. It was built as the pair the frontend
/// compared a promoted face pick against (SKETCH-ON-FACE W3: double-clicking a face
/// re-entered a sketch already hosted there); that consumer was removed in
/// `1fe0cef`, and today the projection is read for presence and for its `body_id`
/// only — see [`crate::dto::SketchHostFaceDto`] for the seam a new comparing
/// consumer would have to close first. A `HostFace` attachment with no `primary` is
/// a face bound by evidence alone; it has no id to project, so it projects as
/// `None` rather than as a fabricated one.
fn sketch_host_face(sketch: &Sketch) -> Option<SketchHostFaceDto> {
    match &sketch.attachment {
        SketchAttachment::HostFace { face, .. } => {
            face.primary.as_ref().map(|p| SketchHostFaceDto {
                body_id: p.body.to_string(),
                element_id: p.element.to_string(),
            })
        }
        SketchAttachment::World { .. } | SketchAttachment::Datum { .. } => None,
    }
}

/// Renders a [`MeshKey`] as the `"<bodyId>:<lod>:<generation>"` string the
/// frontend `document-changed` payload carries (matches the mock's `mockMeshKey`).
#[must_use]
/// Builds the `Sketch` operation mirroring a sketch's current authoritative
/// content — shared by the finish-time upsert and the open-time backfill so a
/// record minted on either path is identical for the same sketch.
fn sketch_record_op(sketch: &Sketch) -> Operation {
    let (_, entities, constraints) = crate::worker::wire::sketch_wire(sketch);
    Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch: sketch.id,
        plane: plane_ref_of(sketch),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        // VF-B5a: stamp the host-face dependency at MINT/REFRESH time — the only
        // moment the record is authored from the live sketch. It is deliberately
        // never derived from the attachment on load (`inputs` sits inside the golden
        // prefix hash and is re-derived on every deserialize, so a backfill would
        // move every legacy document's hash AND bytes); legacy records stay `None`
        // and are gated through the edit layer's attachment bridge instead.
        host_face: sketch.host_face().cloned(),
        extra: Default::default(),
    }))
}

/// Legacy-container backfill. Documents saved before `finish_sketch` minted the
/// sketch's timeline record carry sketches with NO matching `Sketch` record, so
/// the regen planner cannot resolve any modeling op's profile ("profile sketch
/// not found in plan") and EVERY extrude off such a sketch fails at commit
/// (observed: a user document with 20 Extrude records and zero Sketch records).
/// Insert the missing records at the FRONT of the timeline — sketches have no
/// dependencies — preserving the persisted rollback cursor (every pre-existing
/// record shifts by `k`). In-memory only; the next save persists them. Record
/// ids are fresh (never referenced by any other op), and the insertion is
/// fixed-point: a re-saved container already carries the records, so reopening
/// backfills nothing.
fn backfill_missing_sketch_records(doc: &mut Document) {
    let missing: Vec<SketchId> = doc
        .sketches
        .keys()
        .filter(|sid| {
            !doc.timeline.records().iter().any(|r| {
                matches!(&r.op, Operation::Known(KnownOperation::Sketch(p)) if &p.sketch == *sid)
            })
        })
        .copied()
        .collect();
    if missing.is_empty() {
        return;
    }
    tracing::warn!(
        "backfill: minting {} missing Sketch timeline record(s) at open (legacy container)",
        missing.len()
    );
    let shift = missing.len();
    let old_cursor = doc.timeline.cursor();
    let mut records: Vec<OperationRecord> = missing
        .into_iter()
        .map(|sid| {
            let mut op = sketch_record_op(&doc.sketches[&sid]);
            // VF-B5a: these records go in at the FRONT, ahead of every existing op —
            // including whatever produces a host-face sketch's host body. Stamping
            // `host_face` here would author an input edge pointing at a LATER
            // producer (anti-time-travel), so the backfill deliberately drops it.
            // The gate's attachment bridge (`transform_gate_items`) still covers
            // these sketches, and the next `finish_sketch` re-stamps if the record's
            // position allows it.
            if let Operation::Known(KnownOperation::Sketch(p)) = &mut op {
                p.host_face = None;
            }
            OperationRecord::new(RecordId(Uuid::new_v4()), 0, "Sketch", op)
        })
        .collect();
    records.extend_from_slice(doc.timeline.records());
    // `from_records` marks every step Dirty — already the just-loaded state —
    // and places the cursor at the end; restore the persisted applied prefix
    // (shifted past the inserted records).
    let mut timeline = Timeline::from_records(records);
    timeline.set_cursor(old_cursor + shift);
    doc.timeline = timeline;
}

/// The timeline-record plane ref for a sketch's frozen frame. World attachments
/// keep their named kind; host-face / datum frames serialize the resolved
/// custom basis (the frame is frozen with the sketch — MODEL-OPS W2 policy).
fn plane_ref_of(sketch: &Sketch) -> SketchPlaneRef {
    let kind = match &sketch.attachment {
        SketchAttachment::World { plane } => match plane {
            WorldPlane::XY => PlaneKind::Xy,
            WorldPlane::XZ => PlaneKind::Xz,
            WorldPlane::YZ => PlaneKind::Yz,
        },
        _ => PlaneKind::Custom,
    };
    SketchPlaneRef {
        kind,
        origin: sketch.plane.origin,
        x_axis: sketch.plane.x_axis,
        y_axis: sketch.plane.y_axis,
        normal: sketch.plane.normal,
        extra: Default::default(),
    }
}

pub fn mesh_key_string(key: MeshKey) -> String {
    format!("{}:{}:{}", key.body, lod_str(key.lod), key.generation)
}

/// A sketch-flow domain issue as a recoverable [`EngineError`] (the session stays
/// editable) — surfaced to the command as [`ApiError::OpFailed`](crate::error::ApiError).
fn op_failed(message: impl Into<String>) -> onecad_core::regen::EngineError {
    onecad_core::regen::EngineError::OpFailed {
        code: onecad_core::regen::OpFailureCode::OpFailed,
        recoverable: true,
        message: message.into(),
        diagnostics: Vec::new(),
    }
}

/// Converts a solver `positions` map (point-entity-id string → `[x, y]`) into the
/// typed `(EntityId, Vec2)` pairs [`Sketch::apply_solved_positions`] consumes.
/// Non-uuid keys / non-finite coords are skipped.
fn typed_positions(positions: &BTreeMap<String, [f64; 2]>) -> Vec<(EntityId, Vec2)> {
    positions
        .iter()
        .filter_map(|(k, xy)| {
            let id = EntityId::from_str(k).ok()?;
            let v = Vec2::new(xy[0], xy[1])?; // rejects non-finite
            Some((id, v))
        })
        .collect()
}

/// Typed view of a solver `curves` map — the companion to [`typed_positions`].
/// An unparseable key (not an `EntityId`) is dropped; non-finite MEMBERS are
/// dropped downstream by [`Sketch::apply_solved_curves`], which also clamps the
/// radius.
fn typed_curves(
    curves: &BTreeMap<String, crate::dto::CurveParamsDto>,
) -> Vec<(EntityId, CurveParams)> {
    curves
        .iter()
        .filter_map(|(k, c)| {
            let id = EntityId::from_str(k).ok()?;
            Some((
                id,
                CurveParams {
                    radius: c.radius,
                    start_angle: c.start_angle,
                    end_angle: c.end_angle,
                },
            ))
        })
        .collect()
}

/// The wire kind string for an element (SCHEMA §7.5).
fn kind_str(kind: onecad_core::document::refs::ElementKind) -> &'static str {
    use onecad_core::document::refs::ElementKind;
    match kind {
        ElementKind::Face => "face",
        ElementKind::Edge => "edge",
        ElementKind::Vertex => "vertex",
    }
}

/// Parses a repair `refId` `<recordId>.input<k>` into `(RecordId, index)`. The opId
/// segment is the op's record uuid — the worker's `PlanExecutor` mints
/// `<opId>.input<i>` where `opId` is the `opId` Rust sent (`record_id.to_string()`;
/// `wire::wire_op`). `None` on a shape/parse mismatch (fails soft).
fn parse_input_ref_id(ref_id: &str) -> Option<(RecordId, usize)> {
    let (op, k) = ref_id.rsplit_once(".input")?;
    let record = RecordId::from_str(op).ok()?;
    let index: usize = k.parse().ok()?;
    Some((record, index))
}

/// True iff an [`ElementRef`] carries no evidence — the lean refId-only request shape
/// (an empty ref flattened from a `{refId}`-only body).
fn element_ref_is_empty(r: &ElementRef) -> bool {
    r.primary.is_none() && r.intent.is_none() && r.anchor.is_none()
}

/// The `index`-th topological input [`ElementRef`] of an op, in the SAME order the
/// wire `inputs[]` array carries (mirrors `wire::wire_op_inputs`): fillet/chamfer
/// `edges`, a typed Revolve body-edge axis, Shell open faces, extrude ToFace target faces, and a Hole's host face
/// at slot **1** (slot 0 is the host BODY, which is not an element ref). Ops whose
/// inputs are whole bodies (Boolean / pattern / mirror) expose no typed element ref.
///
/// H9: the Hole arm closes the H5 divergence — `wire_op_inputs` and
/// `element_refs_mut` both cover `HoleParams::face`, so a refId-only
/// `resolve_refs` for a hole face used to come back with NO evidence while the
/// planner lowered the full ref. Pinned by `hole_face_input_hydrates_at_slot_1`.
fn element_ref_input(op: &Operation, index: usize) -> Option<&ElementRef> {
    let Operation::Known(k) = op else {
        return None;
    };
    match k {
        KnownOperation::Revolve(p) => match p.axis.as_ref() {
            Some(onecad_core::document::refs::AxisRef::Element {
                edge_ref: Some(edge_ref),
                ..
            }) if index == 0 => Some(edge_ref),
            _ => None,
        },
        KnownOperation::Fillet(p) => p.edges.get(index),
        KnownOperation::Chamfer(p) => p.edges.get(index),
        KnownOperation::Shell(p) => p.faces.get(index),
        // `inputs[0]` is the host body ref (no element), `inputs[1]` the host face.
        KnownOperation::Hole(p) => (index == 1).then_some(&p.face),
        // Operative faces in stored order, then the `Total` opposite face LAST —
        // the SCHEMA §7.3 normative slot table `wire_op_inputs` lowers.
        KnownOperation::OffsetFace(p) => match p.faces.get(index) {
            Some(f) => Some(f),
            None => (index == p.faces.len())
                .then_some(p.opposite_face.as_ref())
                .flatten(),
        },
        KnownOperation::Extrude(p) => {
            let mut faces: Vec<&ElementRef> = Vec::new();
            if p.mode == ExtrudeMode::ToFace {
                if let Some(f) = &p.target_face {
                    faces.push(f);
                }
            }
            if p.two_directions && p.mode2 == ExtrudeMode::ToFace {
                if let Some(f) = &p.target_face2 {
                    faces.push(f);
                }
            }
            faces.get(index).copied()
        }
        // The (optional) mate target is slot 0, and the only slot — mirrors
        // `wire::wire_op_inputs`'s PlaceComponent arm.
        KnownOperation::PlaceComponent(p) => (index == 0)
            .then(|| p.mate.as_ref().map(|m| &m.target))
            .flatten(),
        _ => None,
    }
}

mod ordinal_tripwire;
pub mod solver_lane;

#[cfg(test)]
mod tests;
