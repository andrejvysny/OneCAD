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
use onecad_core::document::record::{
    ExtrudeMode, KnownOperation, Operation, OperationRecord, PlaneKind, SketchOpParams,
    SketchPlaneRef,
};
use onecad_core::document::refs::{AnchorIntent, ElementRef};
use onecad_core::document::repair::RepairItem;
use onecad_core::document::Document;
use onecad_core::edit::{CommandOutcome, DocumentSession, EditCommand, SketchEditOp};
use onecad_core::error::DomainError;
use onecad_core::history::{DependencyGraph, StepState, Timeline};
use onecad_core::ids::{
    BodyId, DocumentId, DocumentRevision, ElementId, EntityId, JobId, RecordId, SketchId,
    SnapshotId, TopoKey, WorkerEpoch,
};
use onecad_core::io::container::{
    CheckpointCache, ContainerCaches, ContainerReader, ContainerWriter, LoadedContainer, SaveMeta,
    CHECKPOINTS_DIR,
};
use onecad_core::io::imports::{ImportBlob, ImportBlobs};
use onecad_core::io::IoError;
use onecad_core::math::Vec2;
use onecad_core::regen::{
    mint_element_ids, AcquireRequest, CancelToken, CheckpointArtifacts, CheckpointStore,
    EngineError, GeometryEngine, InMemoryCheckpointStore, Lod, MeshKey, ModelSnapshot, Outcome,
    Pick, PlanArtifacts, PlanContext, PlanRequest, PolicyVersions, RefResolution, RegenExecutor,
    RegenPlanner, RegenRequest, RegenSession, ResolveRequest, SnapshotPublisher, TessellateSpec,
};
use onecad_core::sketch::{Sketch, SketchAttachment, WorldPlane};

use crate::dto::{
    default_label, feature_kind, feature_status, feature_status_message, feature_value_text,
    needs_repair_item_dto, op_type_name, BodyDto, BodyMeshRef, DatumDto, DocStatus, DocumentChange,
    DocumentProjection, FailedStep, FeatureDto, FinishSketchDto, NeedsRepairItemDto,
    PromotedElementDto, SketchDto, SketchHostFaceDto, SketchSessionDto, SketchSolveStatus,
    SketchStatus, SketchUpsertDto,
};
use crate::error::ApiError;
use crate::imports::{ImportWorkspace, PreparedImport};
use crate::mesh_cache::MeshCache;
use crate::worker::{lod_str, AdoptingEngine, MeshProvider, SolverEngine};

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
    /// Per-record-id, the bodies each op CREATED or MODIFIED in this PUBLISHED regen
    /// (`document-changed` wire form). Empty on non-published outcomes. Threaded into
    /// `regen-finished` `affectedBodies` for precise per-commit correlation.
    pub affected_bodies: BTreeMap<String, Vec<String>>,
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
}

impl PreparedSketchRegions {
    /// Syncs the authoritative sketch into the rebuildable solver cache and
    /// derives its closed regions. Document/session/undo state is untouched.
    pub async fn drive(self) -> Result<FinishSketchDto, EngineError> {
        self.solver.sketch_upsert(&self.sketch).await?;
        let regions = self
            .solver
            .sketch_regions(&self.sketch.id.to_string())
            .await?;
        Ok(FinishSketchDto { regions })
    }
}

/// The per-document runtime (V1 single writer).
pub struct DocumentRuntime {
    session: DocumentSession,
    regen: RegenSession,
    /// The lock-free fencing tokens (revision + worker epoch). See [`FencingCell`].
    fencing: Arc<FencingCell>,
    title: String,
    path: Option<PathBuf>,
    dirty: bool,
    read_only: bool,
    mesh_cache: MeshCache,
    latest_snapshot: Option<Arc<ModelSnapshot>>,
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
    /// The open sketch-edit session watermark (B1 squash), set on `enter_sketch`
    /// and consumed on `finish_sketch`/`cancel_sketch`. `None` outside a session.
    sketch_session: Option<SketchSession>,
    /// Monotonic gesture id allocator (SCHEMA §7.4 `gestureId`).
    gesture_seq: u64,
    /// Rust-owned promotion cache `(body, topoKey) → ElementId` so re-picking the
    /// same element in a snapshot returns the **same** id (Invariant 1). The worker
    /// only echoes ids it already holds; Rust owns id identity, so this map upholds
    /// the invariant across `AcquireElementIds` calls.
    promoted: HashMap<(BodyId, TopoKey), ElementId>,
    /// The regen checkpoint cache (SCHEMA §7.7). Populated by
    /// [`take_checkpoint_at_head`](Self::take_checkpoint_at_head) (policy: on explicit
    /// `save_document` only — the cheapest sound policy), persisted into the `.onecad`
    /// container, and reloaded on open. [`begin_regen`](Self::begin_regen) hands its
    /// metadata to the planner so a post-checkpoint edit regens incrementally
    /// (RestoreCheckpoint) instead of from 0. A **disposable cache**: an incompatible
    /// or unavailable checkpoint degrades to replay, never a wrong result (Invariant 7).
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
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Document".to_string());
        let mut rt = Self::from_document(
            doc,
            title,
            Some(path.to_path_buf()),
            read_only,
            engine,
            meshes,
            solver,
        );
        // Reload the persisted checkpoint cache so a post-open edit can regen
        // incrementally (SCHEMA §7.7). Disposable — a stale entry is skipped.
        rt.load_checkpoints(&loaded);
        // Import sources are authoritative-for-a-record: without them the
        // `ImportStep` steps cannot replay. Loaded eagerly (unlike caches) because
        // regen starts immediately after open and the worker needs real files.
        rt.load_import_blobs(&loaded);
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
        let regen = RegenSession {
            bodies: seed_regen_bodies(&doc),
            timeline: doc.timeline.clone(),
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
            fencing: Arc::new(FencingCell::new(1)),
            title,
            path,
            dirty: false,
            read_only,
            mesh_cache: MeshCache::new(),
            latest_snapshot: None,
            publisher: Arc::new(SnapshotPublisher::new()),
            engine,
            meshes,
            solver,
            occt_fingerprint: "pending-r-wp11".to_string(),
            job_seq: 0,
            sketch_solve: BTreeMap::new(),
            active_gesture: None,
            sketch_session: None,
            gesture_seq: 0,
            promoted: HashMap::new(),
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
        let outcome = self.session.apply(cmd)?;
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

    /// Undoes the newest committed edit. Returns `true` if a step was undone.
    pub fn undo(&mut self) -> bool {
        if self.read_only {
            return false;
        }
        let undone = self.session.undo();
        if undone {
            self.after_mutation();
            self.drop_stale_sketch_session();
        }
        undone
    }

    /// Redoes the newest undone edit.
    ///
    /// # Errors
    /// [`DomainError`] if a replayed command fails.
    pub fn redo(&mut self) -> Result<bool, DomainError> {
        if self.read_only {
            return Ok(false);
        }
        let redone = self.session.redo()?;
        if redone {
            self.after_mutation();
            self.drop_stale_sketch_session();
        }
        Ok(redone)
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
    fn sync_regen_timeline(&mut self) {
        let src = &self.session.document().timeline;
        let mut mirror = Timeline::from_records(src.records().to_vec());
        mirror.set_cursor(src.cursor());
        self.regen.timeline = mirror;
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
            failed_steps: Vec::new(),
            affected_bodies: BTreeMap::new(),
        }
    }

    /// Phase 1 (**locked**): compile the plan against the current timeline, capture
    /// the fencing tokens, and **clone** the regen session so the executor can drive
    /// lock-free on the copy. `None` for an empty plan. Enforces D1 body-id
    /// adoption via [`AdoptingEngine`].
    pub fn begin_regen(&mut self, request: RegenRequest) -> Option<PreparedRegen> {
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
        let ceiling = match self.regen.repair.first_seeded_step() {
            None => None,
            // A gate on step 0 leaves nothing legal to execute at all.
            Some(0) => {
                tracing::info!("begin_regen: repair gate at step 0 → nothing may execute (noop)");
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
            if let Some(prepared) = self.prepare_clear_regen(request) {
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
                lod: Lod::Coarse,
                include_edges: true,
            }),
        };
        let mut plan_req =
            plan.into_request(job, plan_rev, epoch, PolicyVersions::default(), artifacts);
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
        // D1: worker-minted `created` ids must match a known op in this plan and be
        // unique. Replay-from-0 base is empty, so collisions are in-plan.
        let known_ops: HashSet<Uuid> = plan_req.ops.iter().map(|o| o.record_id.as_uuid()).collect();
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
        Some(PreparedRegen {
            work: PreparedWork::Plan {
                plan_req: Box::new(plan_req),
                engine: Box::new(AdoptingEngine::new(
                    self.engine.clone(),
                    known_ops,
                    HashSet::new(),
                )),
            },
            scratch: self.clone_regen_session(),
            fencing: self.fencing.clone(),
            publisher: self.publisher.clone(),
            expected: (plan_rev, epoch),
            lod: Lod::Coarse,
            prior,
            executed,
            job: Some(job),
            base_hash_prefix,
            step_count,
        })
    }

    /// Phase 1 for the **CLEAR** case (TRUST F2): a regen whose op list is empty
    /// *because every op it covers is suppressed* (or because nothing is applied at
    /// all) must still publish — as **no geometry** — instead of reporting `NoOp` and
    /// leaving the previous bodies on screen and in the saved container.
    ///
    /// `None` (⇒ keep the `NoOp` terminal) whenever the emptiness is the benign kind:
    /// a request that starts past the applied end ("nothing new to do"), a
    /// checkpoint-based plan whose restored base still holds the geometry, or a
    /// document that has no geometry to clear in the first place.
    ///
    /// The publish is Rust-side, with **no worker round-trip**: there is nothing for the
    /// worker to compute, an ops-empty `ExecutePlan` is not part of the wire contract
    /// (SCHEMA §7.2 — and the executor short-circuits `start_step().is_none()` to
    /// `NoOp`), and the worker's stale head is harmless because the next non-empty regen
    /// from this state replays from an empty base (`start_step == 0`) or restores from
    /// immutable checkpoint artifacts. The result still travels the normal
    /// [`finish_regen`](Self::finish_regen) accept path, so revision/fencing, the
    /// `document-changed` delta and the projection all stay consistent.
    fn prepare_clear_regen(&self, request: RegenRequest) -> Option<PreparedRegen> {
        let target = self.fully_suppressed_target(request)?;
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
            expected: (plan_rev, epoch),
            lod: Lod::Coarse,
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
        })
    }

    /// `Some(steps_covered)` iff `request` covers the applied prefix **from step 0** and
    /// every applied op in it is suppressed — i.e. the geometry this regen describes is
    /// empty. `steps_covered` is the number of leading timeline steps the clear spans
    /// (`0` when nothing is applied).
    ///
    /// Deliberately derived from the timeline + request rather than from the compiled
    /// plan: the planner's "nothing to do" early return also yields an empty op list with
    /// `start_step == 0` for a single-record timeline, and those two must not be confused.
    fn fully_suppressed_target(&self, request: RegenRequest) -> Option<usize> {
        let applied = self.regen.timeline.cursor();
        let (start, target) = match request {
            RegenRequest::ToStep(k) => (k, k),
            RegenRequest::ToEnd { from } => (from, applied.saturating_sub(1)),
        };
        if start != 0 {
            return None; // a suffix regen keeps whatever the base already holds.
        }
        if applied == 0 {
            return Some(0); // nothing applied ⇒ nothing to show.
        }
        let target = target.min(applied - 1);
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
    pub fn finish_regen(&mut self, driven: DrivenRegen) -> RegenReport {
        let DrivenRegen {
            outcome,
            scratch,
            prior,
            expected,
            lod,
            executed,
            job,
            base_hash_prefix,
            step_count,
        } = driven;
        let job = JobLabel(job);
        // The revision this regen was PREPARED for (fenced at begin_regen). Threaded
        // into EVERY outcome — including Superseded/Failed — for commit correlation.
        let source_revision = expected.0 .0;
        if let Outcome::Published(snap) = &outcome {
            if self.fencing.get() == expected {
                let snapshot_id = snap.id.0;
                let (changed, removed) = self.commit_snapshot(scratch, snap, lod, &prior);
                // Write the just-produced body provenance back onto the records so the
                // dependency graph gains its body edges (see `sync_record_outputs`).
                self.sync_record_outputs(&executed);
                // Label freshly-imported bodies from their STEP product names BEFORE
                // the rows are adopted — `adopt_regen_bodies` is insert-only, so this
                // is the one moment the name can reach `document.bodies`.
                self.apply_import_body_names();
                // Give every just-published body a document metadata row, so the body
                // commands (rename / visibility) can address it at all.
                self.session.adopt_regen_bodies(&self.regen.bodies);
                // Post-commit: the live repair state now reflects this regen. A lean
                // per-item set drives the `needs-repair` event (empty ⇒ repairs
                // cleared → banner drop).
                let needs_repair = self.needs_repair_items();
                // Finding 1: a from-0 regen can PUBLISH sibling bodies while leaving the
                // newly-committed op in Error (stale axis/region). Derive the per-record
                // failure + created/modified-body maps from the just-committed regen
                // mirror so the frontend correlates its own commit's recordId precisely.
                let failed_steps = failed_steps_of(&self.regen.timeline);
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
                    affected_bodies,
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
                affected_bodies: BTreeMap::new(),
            };
            log_regen_outcome(job, &base_hash_prefix, step_count, &report);
            return report;
        }
        // Finding 5: an `EngineFailed` outcome whose fencing tokens MOVED since
        // `begin_regen` is really a supersede — a later covering regen is already on the
        // way — so reporting `failed` here would send a spurious failure to a commit that
        // is about to be resolved by that covering regen. Downgrade it to `Superseded`
        // (a hard failure with UNCHANGED fencing is still reported as `failed`). Cancelled
        // and NoOp keep their own terminal.
        let outcome =
            if matches!(outcome, Outcome::EngineFailed(_)) && self.fencing.get() != expected {
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
        let report = RegenReport {
            outcome,
            revision: self.fencing.revision().0,
            source_revision,
            snapshot_id: 0,
            changed: Vec::new(),
            removed: Vec::new(),
            needs_repair: Vec::new(),
            failed_steps: Vec::new(),
            affected_bodies: BTreeMap::new(),
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

    #[must_use]
    pub fn repair_items(&self) -> &[RepairItem] {
        self.regen.repair.items()
    }

    /// Lean per-item NeedsRepair summaries for the `needs-repair` event, resolving
    /// each item's timeline step to its op record id (`opId`).
    fn needs_repair_items(&self) -> Vec<NeedsRepairItemDto> {
        let records = self.regen.timeline.records();
        self.regen
            .repair
            .items()
            .iter()
            .map(|item| {
                let op_id = records
                    .get(item.step_index)
                    .map(|r| r.record_id.to_string())
                    .unwrap_or_default();
                needs_repair_item_dto(op_id, item)
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
        self.regen = scratch;
        self.latest_snapshot = Some(snap.clone());
        self.dirty = true;
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
    pub async fn get_mesh(
        &mut self,
        body: BodyId,
        lod: Lod,
        generation: Option<u64>,
    ) -> Option<Arc<Vec<u8>>> {
        let (gen, snap_id, latest_gen) = {
            let snap = self.latest_snapshot.as_ref()?;
            (
                generation.unwrap_or(snap.generation),
                snap.id,
                snap.generation,
            )
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

    // ── Save ─────────────────────────────────────────────────────────────────

    /// Atomically saves the document (+ merged regen geometry outputs + the regen
    /// checkpoint cache) to `path`. Timestamps come from the caller (the pure core
    /// never reads the wall clock).
    ///
    /// # Errors
    /// [`IoError`] on a serialization / filesystem failure; the target is left
    /// untouched on any failure.
    pub fn save(&mut self, path: &Path, meta: SaveMeta) -> Result<(), IoError> {
        let mut doc = self.session.document().clone();
        // Merge regen-derived outputs so a reopen shows the tree before regen.
        doc.bodies = self.saved_bodies();
        doc.elements = self.regen.elements.clone();
        doc.repair = self.regen.repair.clone();
        let caches = ContainerCaches {
            checkpoints: self.checkpoint_caches(),
            ..ContainerCaches::none()
        };
        ContainerWriter::save_with_imports(path, &doc, &caches, &self.imports, &meta)?;
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    /// Writes an autosave copy of the document (+ merged regen outputs + the
    /// checkpoint cache) to `path` **without** touching the live save path or the
    /// dirty flag — a crash-recovery snapshot, not a real save. Reuses the same
    /// atomic [`ContainerWriter`] the autosave layout ([`io::recovery`]) points at.
    /// Timestamps come from the caller (the pure core never reads the wall clock).
    ///
    /// [`io::recovery`]: onecad_core::io::recovery
    ///
    /// # Errors
    /// [`IoError`] on a serialization / filesystem failure; the target is left
    /// untouched on any failure.
    pub fn write_autosave(&self, path: &Path, meta: SaveMeta) -> Result<(), IoError> {
        let mut doc = self.session.document().clone();
        doc.bodies = self.saved_bodies();
        doc.elements = self.regen.elements.clone();
        doc.repair = self.regen.repair.clone();
        let caches = ContainerCaches {
            checkpoints: self.checkpoint_caches(),
            ..ContainerCaches::none()
        };
        ContainerWriter::save_with_imports(path, &doc, &caches, &self.imports, &meta)
    }

    /// The document's stable id (the autosave container + crash-marker key,
    /// SCHEMA §7 recovery layout).
    #[must_use]
    pub fn document_uuid(&self) -> DocumentId {
        self.session.document().id
    }

    /// Adopts a recovered document's real on-disk path and marks it unsaved. Called
    /// after opening an autosave container during crash recovery: a subsequent Save
    /// then targets the ORIGINAL path (not the autosave copy), and the recovered
    /// edits stay dirty until the user saves. `original` `None` ⇒ a never-saved
    /// document (Save falls back to Save As).
    pub fn mark_recovered(&mut self, original: Option<PathBuf>) {
        self.path = original;
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
                            bytes,
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
    /// is the natural moment a durable acceleration base is minted). No-op unless the
    /// latest published snapshot is exactly the timeline head (so the worker's head
    /// geometry matches the checkpoint step). Best-effort: a worker failure skips the
    /// checkpoint (the cache is disposable — Invariant 7).
    pub async fn take_checkpoint_at_head(&mut self) {
        let applied = self.regen.timeline.cursor();
        if applied == 0 {
            return;
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
            return;
        }
        if let Ok(artifacts) = self.engine.save_checkpoint(head_step).await {
            // Adopt the worker's real OCCT fingerprint from the checkpoint so the
            // PlanContext compatibility check (which governs checkpoint selection)
            // matches the envelope — the V1 `occt_fingerprint` placeholder would
            // otherwise reject every checkpoint (Invariant 7 fingerprint gate).
            if let Some(env) = artifacts.representative_envelope() {
                self.occt_fingerprint = env.occt_fingerprint.clone();
            }
            self.checkpoints.save(head_step, artifacts);
        }
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

    /// Serializes the checkpoint cache into container [`CheckpointCache`] entries
    /// (`checkpoints/<step>.json`). V1 stores the whole [`CheckpointArtifacts`] as
    /// JSON (BREP bytes inline as an array) — a size inefficiency (documented
    /// divergence from the §7.7 split json/bin), sound for the small V1 artifacts.
    fn checkpoint_caches(&self) -> Vec<CheckpointCache> {
        self.checkpoints
            .list()
            .iter()
            .filter_map(|m| {
                let stored = self.checkpoints.load(m.step)?;
                let json = serde_json::to_vec(&stored.artifacts).ok()?;
                Some(CheckpointCache {
                    step: m.step,
                    json,
                    bin: None,
                })
            })
            .collect()
    }

    /// Loads persisted checkpoints from an opened container into the cache. A stale /
    /// unparseable / hash-mismatched entry is skipped (Invariant 7 — a bad cache
    /// degrades to replay, never a wrong result).
    fn load_checkpoints(&mut self, loaded: &onecad_core::io::container::LoadedContainer) {
        use onecad_core::io::container::CacheRead;
        for entry in loaded.cache_entries() {
            let Some(rest) = entry.path.strip_prefix(CHECKPOINTS_DIR) else {
                continue;
            };
            let Some(step_str) = rest.strip_suffix(".json") else {
                continue;
            };
            let Ok(step) = step_str.parse::<usize>() else {
                continue;
            };
            if let Ok(CacheRead::Present(bytes)) = loaded.read_cache(&entry.path) {
                if let Ok(artifacts) = serde_json::from_slice::<CheckpointArtifacts>(&bytes) {
                    // Adopt the persisted worker fingerprint so a post-open regen's
                    // PlanContext matches the loaded envelopes (see take_checkpoint).
                    if let Some(env) = artifacts.representative_envelope() {
                        self.occt_fingerprint = env.occt_fingerprint.clone();
                    }
                    self.checkpoints.save(step, artifacts);
                }
            }
        }
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
            // Timeline cursor + length drive the legacy-draft recovery hint
            // (`appliedOps < totalOps` ⇒ ops sit beyond the rollback bar).
            applied_ops: doc.timeline.cursor(),
            total_ops: doc.timeline.len(),
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
        FeatureDto {
            id: rec.record_id.to_string(),
            kind,
            op_type: op_type_name(&rec.op),
            label,
            value_text: feature_value_text(&rec.op),
            status: feature_status(&state),
            // Surface a step's worker failure reason (`StepState::Error{reason}`) so
            // the HistoryList row can tint + tooltip it end-to-end (Codex MAJOR-4).
            status_message: feature_status_message(&state),
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
        // solver cache; mid-gesture that would clobber the live drag with
        // pre-drag geometry, so subsequent SolveDrag replies apply onto stale
        // state. Refuse loudly instead — a different sketch's gesture is fine.
        if self
            .active_gesture
            .as_ref()
            .is_some_and(|g| g.sketch_id == sketch_id)
        {
            return Err(op_failed(format!(
                "getSketchRegions: sketch {sketch_id} has an active drag gesture — retry after pointer-up"
            )));
        }
        Ok(PreparedSketchRegions {
            sketch: self.sketch_or_err(sketch_id, "getSketchRegions")?,
            solver: self.solver.clone(),
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
    /// # Errors
    /// [`EngineError`] on a read-only document, an unknown sketch, or a worker failure.
    pub async fn begin_gesture(
        &mut self,
        sketch_id: SketchId,
        drag_point: EntityId,
    ) -> Result<crate::dto::BeginGestureDto, EngineError> {
        if self.read_only {
            return Err(op_failed("beginGesture: read-only document"));
        }
        let sketch = self.sketch_or_err(sketch_id, "beginGesture")?;
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
                // solverPolicyHash: reserved, always "" — see wire::begin_gesture_args.
                "",
            )
            .await?;
        self.active_gesture = Some(ActiveGesture {
            gesture_id,
            sketch_id,
            drag_point,
            before: sketch,
            next_seq: 1,
        });
        Ok(ready)
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
        Ok((FinishSketchDto { regions }, outcome))
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
        let op = sketch_record_op(&sketch);
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
    /// [`EngineError`] on a worker failure.
    pub async fn promote_selection(
        &mut self,
        snapshot: SnapshotId,
        body: BodyId,
        picks: Vec<(TopoKey, Option<AnchorIntent>)>,
    ) -> Result<Vec<PromotedElementDto>, EngineError> {
        let req = AcquireRequest {
            snapshot_id: snapshot,
            body,
            picks: picks
                .into_iter()
                .map(|(topo_key, anchor)| Pick { topo_key, anchor })
                .collect(),
        };
        let mut evidence = self.engine.acquire_element_ids(req).await?;
        // Rust owns id identity: seed `existing` from the promotion cache so a
        // re-pick of the same (body, topoKey) reuses the id (Invariant 1).
        for e in &mut evidence {
            if e.existing.is_none() {
                if let Some(id) = self.promoted.get(&(e.body, e.topo_key.clone())) {
                    e.existing = Some(id.clone());
                }
            }
        }
        let minted = mint_element_ids(evidence);
        let mut out = Vec::with_capacity(minted.len());
        for (id, ev) in minted {
            self.promoted
                .insert((ev.body, ev.topo_key.clone()), id.clone());
            // Record the partition binding into the (regen-mirror) element index.
            self.regen
                .elements
                .insert(id.clone(), ElementEntry::new(ev.body, ev.kind));
            out.push(PromotedElementDto {
                topo_key: ev.topo_key.as_str().to_string(),
                element_id: id.as_str().to_string(),
                kind: kind_str(ev.kind).to_string(),
                body_id: crate::worker::wire::body_id_wire(ev.body),
            });
        }
        Ok(out)
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
        for r in &mut req.refs {
            if element_ref_is_empty(&r.element) {
                if let Some(stored) = self.stored_input_ref(&r.ref_id) {
                    r.element = stored;
                }
            }
        }
        self.engine.resolve_refs(req).await
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
            expected,
            lod,
            prior,
            executed,
            job,
            base_hash_prefix,
            step_count,
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
                let executor = RegenExecutor::new(*engine);
                executor
                    .run(*plan_req, &mut scratch, &gate, &cancel, &publisher)
                    .instrument(span.clone())
                    .await
            }
            PreparedWork::Clear { target } => {
                span.in_scope(|| drive_clear(&mut scratch, &publisher, &cancel, target))
            }
        };
        span.in_scope(|| {
            tracing::info!(
                outcome = outcome_label(&outcome),
                // Machine timing: the span-close `time.busy` is a unit-suffixed STRING.
                elapsed_ms = started.elapsed().as_millis() as u64,
                "regen.drive: done"
            );
        });
        DrivenRegen {
            outcome,
            scratch,
            prior,
            expected,
            lod,
            executed,
            job,
            base_hash_prefix,
            step_count,
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
        repair_summary: onecad_core::regen::RepairSummary::default(),
    });
    Outcome::Published(snapshot)
}

/// The result of driving a [`PreparedRegen`] lock-free (phase 2 → 3 handoff).
pub struct DrivenRegen {
    outcome: Outcome,
    scratch: RegenSession,
    prior: Vec<BodyId>,
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

/// The timeline records left in `StepState::Error` after a regen, as
/// `(recordId, reason)` [`FailedStep`]s — the frontend `failedSteps` correlation
/// source (MODEL-HARDEN finding 1). A published from-0 regen can leave the newly
/// committed op in Error while republishing OTHER bodies; without this the awaiter
/// would read that as a blanket commit success.
fn failed_steps_of(timeline: &Timeline) -> Vec<FailedStep> {
    timeline
        .records()
        .iter()
        .enumerate()
        .filter_map(|(i, rec)| match timeline.state(i) {
            Some(StepState::Error { reason }) => Some(FailedStep {
                record_id: rec.record_id.to_string(),
                message: reason.clone(),
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
/// EXCEPT those produced by a currently-suppressed record (TRUST F4).
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
    if suppressed.is_empty() {
        return doc.bodies.clone();
    }
    let mut seeded = doc.bodies.clone();
    for meta in doc.bodies.bodies().to_vec() {
        if suppressed.contains(&meta.created_by) {
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
/// Only the `primary` binding is projected — that is the pair the frontend
/// compares a promoted face pick against (SKETCH-ON-FACE W3: double-clicking a
/// face re-enters a sketch already hosted there). A `HostFace` attachment with no
/// `primary` is a face bound by evidence alone; it has no id to compare, so it
/// projects as `None` rather than as a fabricated one.
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
            OperationRecord::new(
                RecordId(Uuid::new_v4()),
                0,
                "Sketch",
                sketch_record_op(&doc.sketches[&sid]),
            )
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
/// `edges`, then extrude ToFace target faces. Ops whose inputs are whole bodies
/// (Boolean / pattern / mirror) or bare ids (Shell open faces) expose no typed
/// element ref here, so a refId-only resolve for them stays un-hydrated.
fn element_ref_input(op: &Operation, index: usize) -> Option<&ElementRef> {
    let Operation::Known(k) = op else {
        return None;
    };
    match k {
        KnownOperation::Fillet(p) => p.edges.get(index),
        KnownOperation::Chamfer(p) => p.edges.get(index),
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
        _ => None,
    }
}

#[cfg(test)]
mod tests;
