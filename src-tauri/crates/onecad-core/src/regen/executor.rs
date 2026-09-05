//! Plan execution driver — the prepare/accept publication path (V1/V2 §4.3/§4.4).
//!
//! [`RegenExecutor`] drives a [`PlanRequest`] over a [`GeometryEngine`], folds the
//! streamed per-step events into a **scratch** copy of the session state, and then
//! commits/discards on the terminal. It never mutates the live session until an
//! [`AcceptPrepared`](GeometryEngine::accept_prepared) succeeds — mirroring the
//! worker's own scratch-then-publish model (SCHEMA §7.2), so a failed or
//! superseded plan leaves the document **exactly** as it was.
//!
//! ## The clear-before-replay contract (body.rs)
//!
//! [`BodyRegistry::fold`](crate::document::body::BodyRegistry::fold) is
//! append-only — replaying a timeline into a non-empty registry duplicates
//! lifecycle-log entries. So the scratch registry is seeded per the plan:
//!
//! * **replay-from-0** ([`PlanRequest::replays_from_base_zero`]) — a **fresh**
//!   [`BodyRegistry`] (the log starts empty; the plan reproduces the whole
//!   lifecycle);
//! * **restore-from-checkpoint** — seeded from the **checkpoint artifacts**, not
//!   from live session state (review F3): the executor calls
//!   [`restore_checkpoint`](GeometryEngine::restore_checkpoint) and uses the
//!   returned [`RestoreResult::base_registry`]/[`base_elements`](RestoreResult::base_elements)
//!   — whose lifecycle log ends at the checkpoint step — as the scratch base, then
//!   folds only the NEW events for steps `≥ start`. Seeding from the immutable
//!   artifacts (rather than cloning the possibly-ahead live registry) is what keeps
//!   a re-run of the same checkpoint plan from duplicating lifecycle entries. A
//!   restore that fails or reports drift ⇒ the checkpoint is unusable ⇒ the
//!   replay-from-0 fallback (review F12, below).
//!
//! The element index is seeded from the restore result (checkpoint case) or cloned
//! from live (replay-from-0), then updated by each step's `element_map_delta`: an
//! `ElementId`'s identity is stable across replay (Invariant 1), only its partition
//! (which body / `TopoKey`) moves — and the owning body now comes from the delta
//! entry's `body` field (review F19), not a guess.
//!
//! ## Fold gating (review F6)
//!
//! Per-step body/element mutations are **buffered**, not folded live. At the
//! terminal they are committed **only for steps `≤ last_valid`** — a failing or
//! `NeedsRepair` step may still emit body/element events, but those MUST NOT reach
//! the accepted registry (Invariant 6: failure at `m` publishes `≤ m−1`). The
//! step's `needsRepair`/signatures/diagnostics are still recorded (that is the
//! whole point of surfacing repair state).
//!
//! ## The fencing seam (review F1/F2)
//!
//! A plan is built against a `(document revision, worker epoch)`; by the time it
//! prepares, the document may have advanced **or** the worker may have restarted.
//! The executor takes a [`RevisionGate`] and reads the **current**
//! `(revision, epoch)` *at accept time*, comparing **both** against the plan's
//! tokens. A mismatch ⇒ the prepared snapshot is stale/orphaned ⇒
//! [`discard_prepared`](GeometryEngine::discard_prepared) + [`Outcome::Superseded`];
//! otherwise it accepts and publishes.
//!
//! **Single-writer precondition.** The gate MUST read the *same authority* that
//! guards `&mut RegenSession` (the document session's single writer). The whole
//! prepare/accept scheme is only sound if no other writer can advance the revision
//! or mutate the session between the gate read and the commit — the executor holds
//! `&mut RegenSession` for exactly that window, so the gate and the `&mut` must
//! come from one serialized owner.
//!
//! ## Cancellation
//!
//! The executor `select!`s (biased on the next [`PlanEvent`]) between the stream
//! and the [`CancelToken`]. A cancel observed mid-await, or rechecked in
//! [`on_prepared`](RegenExecutor::run) before accept (review F7 — a cancel that
//! races a fully-buffered terminal still wins), routes to the cancel path: it calls
//! [`cancel`](GeometryEngine::cancel), **bounded-drains** the channel (the terminal
//! frame is never dropped — SCHEMA §3.5/§5.4 — but a ~2s backstop prevents a hung
//! worker from stalling the executor forever, review F8), discards the scratch job,
//! and returns [`Outcome::Cancelled`] — the live session untouched.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;

use crate::document::body::BodyRegistry;
use crate::document::element_index::{ElementEntry, ElementIndex};
use crate::document::repair::{RepairItem, RepairState};
use crate::history::{StepState, Timeline};
use crate::ids::{DocumentRevision, JobId, RecordId, WorkerEpoch};

use super::engine::{
    Diagnostic, EngineError, Fencing, GeometryEngine, PlanEvent, PlanPrepared, PlanRequest,
    PlanStepEvent, PreparedMeshRef, RestoreRequest, Severity, StepSignatures, StepStatus,
    StoppedReason,
};
use super::planner::{HistoryPrefixHash, RegenPlanner};
use super::snapshot::{
    BodySnapshot, Lod, MeshKey, ModelSnapshot, RepairSummary, SnapshotPublisher,
};
use crate::document::body::{BodyHealth, BodyLifecycleEvent};

// ─────────────────────────────────────────────────────────────────────────────
// Cancellation
// ─────────────────────────────────────────────────────────────────────────────

/// A minimal cooperative cancellation token (cloneable, thread-safe). Backed by a
/// `watch<bool>` so `cancelled()` is an awaitable the executor can `select!` on.
#[derive(Debug, Clone)]
pub struct CancelToken {
    tx: Arc<watch::Sender<bool>>,
    rx: watch::Receiver<bool>,
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelToken {
    /// A fresh, un-cancelled token.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self {
            tx: Arc::new(tx),
            rx,
        }
    }

    /// Requests cancellation (idempotent). Wakes any `cancelled()` waiter.
    pub fn cancel(&self) {
        let _ = self.tx.send(true);
    }

    /// Whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolves once cancellation is requested (immediately if already cancelled).
    pub async fn cancelled(&self) {
        let mut rx = self.rx.clone();
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                return; // sender gone ⇒ never cancels; treat as done.
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The fencing seam
// ─────────────────────────────────────────────────────────────────────────────

/// Reads the **current** authoritative `(document revision, worker epoch)` *at
/// accept time* (review F1/F2). The executor compares **both** against the plan's
/// tokens to detect a supersede (document advanced) or an orphaned prepare (worker
/// restarted ⇒ epoch bumped).
///
/// **Single-writer precondition:** the gate MUST read the same authority that
/// guards `&mut RegenSession`. The prepare/accept publication is only sound if no
/// other writer can advance the revision/epoch or mutate the session between this
/// read and the commit — see the module docs.
///
/// A blanket impl makes any `Fn() -> (DocumentRevision, WorkerEpoch)` a gate, so
/// tests and the app can pass a closure over the live fencing tokens.
pub trait RevisionGate: Send + Sync {
    /// The authoritative `(document revision, worker epoch)` right now.
    fn current(&self) -> (DocumentRevision, WorkerEpoch);
}

impl<F: Fn() -> (DocumentRevision, WorkerEpoch) + Send + Sync> RevisionGate for F {
    fn current(&self) -> (DocumentRevision, WorkerEpoch) {
        self()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Session state + outcome
// ─────────────────────────────────────────────────────────────────────────────

/// The live regen session state the executor reads (to seed scratch) and — on a
/// successful accept — writes back to. Bundling the four pieces keeps the `run`
/// signature small and avoids `&mut` field-splitting on the caller's document.
#[derive(Debug, Default)]
pub struct RegenSession {
    /// Authoritative body registry + lifecycle log.
    pub bodies: BodyRegistry,
    /// The strict-linear timeline (the executor sets per-step states here).
    pub timeline: Timeline,
    /// Topological-naming repair state.
    pub repair: RepairState,
    /// Minted-element partition index.
    pub elements: ElementIndex,
}

impl RegenSession {
    /// A session wrapping an existing timeline (bodies/repair/elements empty).
    #[must_use]
    pub fn with_timeline(timeline: Timeline) -> Self {
        Self {
            timeline,
            ..Self::default()
        }
    }
}

/// The result of driving one plan to its terminal.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// The prepared snapshot was accepted and published. Covers a full success
    /// **and** an accepted early-stop at `m−1` (the snapshot's `stopped_reason`
    /// distinguishes them; the timeline carries `m`'s Error/NeedsRepair state and
    /// `m+1..` Dirty).
    Published(Arc<ModelSnapshot>),
    /// The document revision advanced while the plan ran; the prepared snapshot
    /// was discarded. Nothing was committed.
    Superseded,
    /// A hard engine failure (crash / protocol / non-recoverable op failure /
    /// timeout / accept error). Scratch discarded, the plan's steps marked Dirty,
    /// the live session otherwise intact.
    EngineFailed(EngineError),
    /// Cooperatively cancelled. Scratch discarded, live session untouched.
    Cancelled,
    /// The plan had no ops to execute (empty applied timeline / no-op request).
    NoOp,
}

// ─────────────────────────────────────────────────────────────────────────────
// Scratch fold state
// ─────────────────────────────────────────────────────────────────────────────

/// One step's buffered geometry mutations (review F6): committed to the scratch
/// registry only at the terminal, and only if the step is `≤ last_valid`.
struct BufferedStep {
    by: RecordId,
    body_events: Vec<BodyLifecycleEvent>,
    /// SCHEMA §7.2 `rankKey` evidence, applied after the lifecycle fold (VF-B6).
    rank_keys: BTreeMap<crate::ids::BodyId, crate::document::repair::RankKey>,
    health: BTreeMap<crate::ids::BodyId, BodyHealth>,
    added: Vec<super::engine::ElementMapEntry>,
    relabeled: Vec<super::engine::ElementMapEntry>,
    removed: Vec<crate::ids::ElementId>,
}

/// The scratch state the executor folds streamed events into. Committed to the
/// live [`RegenSession`] only on a successful accept.
///
/// Body/element mutations are **buffered** per step and applied at the terminal,
/// gated by `last_valid` (review F6) — so a failing / `NeedsRepair` step's body or
/// element events never reach the accepted registry.
struct Scratch {
    bodies: BodyRegistry,
    elements: ElementIndex,
    /// Buffered per-step geometry mutations (ascending step order via `BTreeMap`).
    buffered: BTreeMap<usize, BufferedStep>,
    /// Per-step three-signatures.
    step_signatures: BTreeMap<usize, StepSignatures>,
    /// Per-step NeedsRepair items (STATE — SCHEMA §9).
    repair_by_step: BTreeMap<usize, Vec<RepairItem>>,
    /// Per-step diagnostics.
    diagnostics_by_step: BTreeMap<usize, Vec<Diagnostic>>,
    /// Component Library P3 WP-3.1: per-step reseated `PlaceComponent`
    /// placements (SCHEMA §7.2 `matePlacement`), present only for a step
    /// whose mate resolved AutoBind and moved. Applied at the terminal, same
    /// `last_valid` gate as `buffered` — a step beyond the last valid one
    /// never reaches the accepted timeline (Invariant 6).
    mate_placement_by_step: BTreeMap<usize, Box<crate::document::record::FrozenPlacement>>,
    /// Kernel-hardening WP-I: per-step SCHEMA §7.2 `mateResolved` echoes, present
    /// for every `PlaceComponent` step whose mate resolved against a cylindrical
    /// target. Applied at the terminal under the same `last_valid` gate as
    /// `mate_placement_by_step`, and ADOPTED only into a record that carries no
    /// `targetAxis` yet.
    mate_resolved_by_step: BTreeMap<usize, crate::document::record::MateResolved>,
}

impl Scratch {
    /// A scratch seeded with a base body registry + element index.
    fn new(bodies: BodyRegistry, elements: ElementIndex) -> Self {
        Self {
            bodies,
            elements,
            buffered: BTreeMap::new(),
            step_signatures: BTreeMap::new(),
            repair_by_step: BTreeMap::new(),
            diagnostics_by_step: BTreeMap::new(),
            mate_placement_by_step: BTreeMap::new(),
            mate_resolved_by_step: BTreeMap::new(),
        }
    }

    /// Buffers one `planStep` event (V1/V2 §4.3 step epilogue). Body/element
    /// mutations are held (not folded); repair/signatures/diagnostics are recorded
    /// immediately (they carry no geometry into the registry).
    fn buffer_step(&mut self, by: RecordId, event: PlanStepEvent) {
        let step = event.step_index;
        self.buffered.insert(
            step,
            BufferedStep {
                by,
                body_events: event.body_events,
                rank_keys: event.body_rank_keys,
                health: event.body_health,
                added: event.element_map_delta.added,
                relabeled: event.element_map_delta.relabeled,
                removed: event.element_map_delta.removed,
            },
        );
        if !event.needs_repair.is_empty() {
            self.repair_by_step.insert(step, event.needs_repair);
        }
        self.step_signatures.insert(step, event.signatures);
        if !event.diagnostics.is_empty() {
            self.diagnostics_by_step.insert(step, event.diagnostics);
        }
        if let Some(placement) = event.mate_placement {
            self.mate_placement_by_step.insert(step, placement);
        }
        if let Some(resolved) = event.mate_resolved {
            self.mate_resolved_by_step.insert(step, resolved);
        }
    }

    /// Commits buffered body/element mutations for steps `≤ last_valid`, in
    /// ascending step order (review F6). `None` ⇒ only the base is valid ⇒ nothing
    /// is folded. Steps beyond `last_valid` keep their repair/signature/diagnostic
    /// records but contribute NO geometry to the accepted registry (Invariant 6).
    fn apply_buffered(&mut self, last_valid: Option<usize>) {
        let Some(cutoff) = last_valid else {
            return;
        };
        let steps: Vec<usize> = self
            .buffered
            .range(..=cutoff)
            .map(|(step, _)| *step)
            .collect();
        for step in steps {
            let Some(buf) = self.buffered.remove(&step) else {
                continue;
            };
            // Body lifecycle (§2.2 identity rules applied inside `fold`).
            for be in buf.body_events {
                self.bodies.fold(step, buf.by, be);
            }
            for (body, health) in buf.health {
                self.bodies.set_health(body, health);
            }
            // VF-B6 identity evidence, stamped AFTER the fold so the body exists.
            // Rides the same `last_valid` gate as the geometry it describes: a stamp
            // from a step that never reached the accepted registry would anchor the
            // tripwire on geometry the document does not have.
            for (body, key) in buf.rank_keys {
                self.bodies.set_geom_stamp(body, key);
            }
            // Element-map partition delta: identity is stable, only partition moves.
            // The owning body comes from the delta entry itself (review F19).
            for entry in buf.added.into_iter().chain(buf.relabeled) {
                self.elements
                    .insert(entry.element_id, ElementEntry::new(entry.body, entry.kind));
            }
            for id in buf.removed {
                self.elements.remove(&id);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The executor
// ─────────────────────────────────────────────────────────────────────────────

/// Where a run deposits the MESH1 blobs an engine inlined into its terminal
/// prepare, already keyed by the [`MeshKey`] the publish minted.
///
/// The executor only APPENDS; draining (and deciding whether the commit that owns
/// them actually landed) belongs to the caller — a superseded regen simply drops
/// the sink. Plain `std::sync::Mutex`: the lock is held for a `Vec` extend and is
/// never held across an `await`.
pub type MeshSink = Arc<Mutex<Vec<(MeshKey, Arc<Vec<u8>>)>>>;

/// Drives plan execution and the accept/discard handshake over a
/// [`GeometryEngine`].
#[derive(Debug)]
pub struct RegenExecutor<E: GeometryEngine> {
    engine: E,
    /// Optional cache-fill channel for inline prepared meshes (see [`MeshSink`]).
    mesh_sink: Option<MeshSink>,
}

impl<E: GeometryEngine> RegenExecutor<E> {
    /// An executor over `engine`.
    #[must_use]
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            mesh_sink: None,
        }
    }

    /// Collects the terminal prepare's inline meshes into `sink`, keyed by the
    /// generation the publish mints. Nothing is deposited unless the run PUBLISHES.
    #[must_use]
    pub fn with_mesh_sink(mut self, sink: MeshSink) -> Self {
        self.mesh_sink = Some(sink);
        self
    }

    /// The wrapped engine.
    #[must_use]
    pub fn engine(&self) -> &E {
        &self.engine
    }

    /// Drives `request` to its terminal, folding events into a scratch copy of
    /// `session` and committing on a successful, non-superseded accept.
    ///
    /// **Invariant-7 fallback (review F12).** If a plan carried a `base_checkpoint`
    /// and fails at restore/execution **before any step event** (or the restore
    /// reports drift/corruption), the executor strips the checkpoint and retries
    /// **exactly once** with a replay-from-0 plan for the same target, so an
    /// unusable cache degrades performance, never correctness. A *real* op failure
    /// (a step event was received) does NOT retry — it surfaces as usual.
    ///
    /// See the module docs for the scratch seeding, fold-gating, fencing and
    /// cancellation contracts.
    pub async fn run(
        &self,
        request: PlanRequest,
        session: &mut RegenSession,
        gate: &dyn RevisionGate,
        cancel: &CancelToken,
        publisher: &SnapshotPublisher,
    ) -> Outcome {
        if request.start_step().is_none() {
            return Outcome::NoOp; // empty plan.
        }
        // Capture the retry ingredients before the request is consumed.
        let job = request.job_id;
        let plan_rev = request.document_revision;
        let epoch = request.worker_epoch;
        let policy = request.policy_versions;
        let artifacts = request.artifacts.clone();
        let target = request.target_step;
        let edited_from = request.edited_from;
        let had_checkpoint = request.base_checkpoint.is_some();

        match self
            .run_attempt(request, session, gate, cancel, publisher, had_checkpoint)
            .await
        {
            AttemptOutcome::Settled(outcome) => outcome,
            AttemptOutcome::RetryFromZero => {
                // F12: strip the checkpoint, replay from 0, exactly once.
                // The retry re-plans, but the EDIT CONTEXT is a property of why this
                // regen was requested, not of how the base is rebuilt — the geometry
                // is still post-edit, so the veto must stay armed (SCHEMA §7.2/§10).
                let from_zero = RegenPlanner::without_checkpoint(&session.timeline, target)
                    .into_request(job, plan_rev, epoch, policy, artifacts)
                    // A `ToEnd { from: 0 }` claims `editedFrom: 0` on the normal lane
                    // (resolverVersion 4) and stays resolvable only through the §10
                    // anchor-exact carve-out — which the fallback flag below DISABLES.
                    // Carrying the 0 here would veto every descriptor-tied ref in the
                    // document (an anchor-only Hole seat ties by construction) on a
                    // reopen whose checkpoint merely failed to restore. A genuine
                    // upstream edit (`from > 0`) keeps its claim: that geometry did move.
                    .with_edited_from(edited_from.filter(|k| *k > 0))
                    // SCHEMA §7.2: the ONLY place this may be claimed. The anchors
                    // this replay resolves against were frozen for a basis the
                    // checkpoint attempt failed to reproduce, so the §10
                    // anchor-exact carve-out must not rescue a descriptor tie here
                    // (VF-M5 lane D). An ordinary replay-from-0 is byte-identical
                    // as a PLAN, which is exactly why the worker cannot infer it.
                    .as_checkpoint_fallback_replay();
                if from_zero.start_step().is_none() {
                    return Outcome::NoOp;
                }
                match self
                    .run_attempt(from_zero, session, gate, cancel, publisher, false)
                    .await
                {
                    AttemptOutcome::Settled(outcome) => outcome,
                    // `allow_retry = false` never returns RetryFromZero.
                    AttemptOutcome::RetryFromZero => Outcome::EngineFailed(EngineError::Crashed {
                        message: "replay-from-0 retry exhausted".into(),
                    }),
                }
            }
        }
    }

    /// One attempt at driving a plan. Returns [`AttemptOutcome::RetryFromZero`]
    /// (only when `allow_retry`) if the plan carried a checkpoint and failed
    /// **before any step event** (restore drift/failure, or a pre-step engine
    /// failure); otherwise a settled [`Outcome`].
    #[allow(clippy::too_many_arguments)]
    async fn run_attempt(
        &self,
        request: PlanRequest,
        session: &mut RegenSession,
        gate: &dyn RevisionGate,
        cancel: &CancelToken,
        publisher: &SnapshotPublisher,
        allow_retry: bool,
    ) -> AttemptOutcome {
        let Some(start) = request.start_step() else {
            return AttemptOutcome::Settled(Outcome::NoOp);
        };
        // Capture what we need past the `request` move into `execute_plan`.
        let job = request.job_id;
        let plan_rev = request.document_revision;
        let epoch = request.worker_epoch;
        let lod = request.artifacts.tessellate.map_or(Lod::Coarse, |t| t.lod);
        let expected_base_hash = request.expected_base_hash.clone();
        let prefix_hashes = request.prefix_hashes.clone();
        let step_records: BTreeMap<usize, RecordId> = request
            .ops
            .iter()
            .map(|o| (o.step_index, o.record_id))
            .collect();
        let planned_steps: Vec<usize> = request.ops.iter().map(|o| o.step_index).collect();

        // ── Seed the scratch base (review F3) ────────────────────────────────────
        // replay-from-0: fresh registry (clear-before-replay), elements clone from
        // live (identity persists). restore-from-checkpoint: reconstruct BOTH from
        // the checkpoint artifacts via `restore_checkpoint`, never from live state.
        let (base_bodies, base_elements) = if let Some(checkpoint) = request.base_checkpoint.clone()
        {
            match self
                .engine
                .restore_checkpoint(RestoreRequest {
                    checkpoint,
                    expected_history_prefix_hash: expected_base_hash.clone(),
                    worker_epoch: epoch,
                    artifacts: request.base_checkpoint_artifacts.clone(),
                })
                .await
            {
                Ok(r) if r.restored && !r.drift_detected => (r.base_registry, r.base_elements),
                // Restore failed or drifted ⇒ before any step event ⇒ F12 fallback.
                _ => {
                    self.discard(job).await;
                    if allow_retry {
                        return AttemptOutcome::RetryFromZero;
                    }
                    mark_dirty_range(&mut session.timeline, &planned_steps);
                    return AttemptOutcome::Settled(Outcome::EngineFailed(EngineError::Protocol {
                        message: "checkpoint restore failed or reported drift".into(),
                    }));
                }
            }
        } else {
            (BodyRegistry::new(), session.elements.clone())
        };

        let mut scratch = Scratch::new(base_bodies, base_elements);
        let mut receiver = self.engine.execute_plan(request).await;

        // Drive the stream, buffering each step. Biased on the stream so a
        // fully-buffered terminal is observed; a mid-await cancel wins.
        let mut saw_step = false;
        let mut streamed_steps = Vec::new();
        let terminal = loop {
            let event = tokio::select! {
                biased;
                ev = receiver.recv() => ev,
                _ = cancel.cancelled() => break Terminal::CancelRequested,
            };
            match event {
                Some(PlanEvent::Step(e)) => {
                    saw_step = true;
                    streamed_steps.push(e.step_index);
                    let by = step_records
                        .get(&e.step_index)
                        .copied()
                        .unwrap_or(RecordId(uuid::Uuid::nil()));
                    scratch.buffer_step(by, e);
                }
                Some(PlanEvent::Prepared(p)) => break Terminal::Prepared(p),
                Some(PlanEvent::Failed(err)) => break Terminal::Failed(err),
                None => break Terminal::Crashed, // channel closed with no terminal.
            }
        };

        match terminal {
            Terminal::CancelRequested => {
                AttemptOutcome::Settled(self.cancel_path(job, &mut receiver).await)
            }
            Terminal::Prepared(prepared) => AttemptOutcome::Settled(
                self.on_prepared(
                    OnPrepared {
                        prepared,
                        job,
                        plan_rev,
                        epoch,
                        start,
                        lod,
                        planned_steps: &planned_steps,
                        streamed_steps: &streamed_steps,
                        expected_base_hash: &expected_base_hash,
                        prefix_hashes: &prefix_hashes,
                    },
                    scratch,
                    session,
                    gate,
                    cancel,
                    publisher,
                )
                .await,
            ),
            Terminal::Failed(err) => {
                self.discard(job).await;
                // F12: a checkpoint plan that failed before any step event retries.
                if allow_retry && !saw_step {
                    return AttemptOutcome::RetryFromZero;
                }
                mark_dirty_range(&mut session.timeline, &planned_steps);
                AttemptOutcome::Settled(Outcome::EngineFailed(err))
            }
            Terminal::Crashed => {
                self.discard(job).await;
                if allow_retry && !saw_step {
                    return AttemptOutcome::RetryFromZero;
                }
                mark_dirty_range(&mut session.timeline, &planned_steps);
                AttemptOutcome::Settled(Outcome::EngineFailed(EngineError::Crashed {
                    message: "plan channel closed before a terminal event".into(),
                }))
            }
        }
    }

    /// Terminal prepare: cancel-recheck, protocol checks, fence, accept or discard,
    /// and (on accept) commit + publish.
    async fn on_prepared(
        &self,
        ctx: OnPrepared<'_>,
        mut scratch: Scratch,
        session: &mut RegenSession,
        gate: &dyn RevisionGate,
        cancel: &CancelToken,
        publisher: &SnapshotPublisher,
    ) -> Outcome {
        let OnPrepared {
            prepared,
            job,
            plan_rev,
            epoch,
            start,
            lod,
            planned_steps,
            streamed_steps,
            expected_base_hash,
            prefix_hashes,
        } = ctx;

        // F7: recheck cancel FIRST — a cancel that raced a fully-buffered terminal
        // still wins (deterministic Cancelled), before any accept/publish.
        if cancel.is_cancelled() {
            return self.cancel_only(job).await;
        }

        // F23: the prepare must be for THIS job (a stray/mismatched terminal is a
        // protocol violation, never silently accepted).
        if prepared.job_id != job {
            self.discard(job).await;
            return Outcome::EngineFailed(EngineError::Protocol {
                message: format!(
                    "PlanPrepared job {:?} != requested job {:?}",
                    prepared.job_id, job
                ),
            });
        }

        if let Err(message) = validate_prepared_stream(&prepared, planned_steps, streamed_steps) {
            self.discard(job).await;
            return Outcome::EngineFailed(EngineError::Protocol { message });
        }

        // X-WP1 item 2 / review F9: verify the worker's OPAQUE history-prefix echo
        // equals the token Rust minted for the last executed op (or the base hash
        // for a base-only prepare). A mismatch ⇒ the worker executed a different
        // prefix than planned ⇒ PROTOCOL_ERROR.
        let expected_echo = expected_prefix_echo(
            prepared.last_valid_step,
            planned_steps,
            expected_base_hash,
            prefix_hashes,
        );
        if prepared.history_prefix_hash != *expected_echo {
            self.discard(job).await;
            return Outcome::EngineFailed(EngineError::Protocol {
                message: format!(
                    "PlanPrepared.historyPrefixHash echo {:?} != expected {:?}",
                    prepared.history_prefix_hash, expected_echo
                ),
            });
        }

        // Fencing: read the current (revision, epoch) AT accept time and compare
        // BOTH (review F1/F2). A mismatch ⇒ superseded / orphaned prepare.
        if gate.current() != (plan_rev, epoch) {
            self.discard(job).await;
            return Outcome::Superseded;
        }
        let accept = match self
            .engine
            .accept_prepared(
                job,
                Fencing {
                    document_revision: plan_rev,
                    worker_epoch: epoch,
                },
            )
            .await
        {
            Ok(a) => a,
            Err(err) => {
                self.discard(job).await;
                mark_dirty_range(&mut session.timeline, planned_steps);
                return Outcome::EngineFailed(err);
            }
        };

        // F6: commit buffered geometry only for steps ≤ last_valid, THEN move the
        // gated scratch into the live session.
        scratch.apply_buffered(prepared.last_valid_step);
        session.bodies = scratch.bodies;
        session.elements = scratch.elements;

        // Apply per-step states from the authoritative PlanPrepared summary.
        for result in &prepared.per_step {
            if planned_steps.contains(&result.step_index) && !result.diagnostics.is_empty() {
                scratch
                    .diagnostics_by_step
                    .insert(result.step_index, result.diagnostics.clone());
            }
        }
        let status_by_step: BTreeMap<usize, StepStatus> = prepared
            .per_step
            .iter()
            .map(|r| (r.step_index, r.status))
            .collect();
        // A failed step emits no planStep event (its diagnostics never arrive), so the
        // authoritative failure reason rides on its `perStepResults.message` instead.
        let message_by_step: BTreeMap<usize, &str> = prepared
            .per_step
            .iter()
            .filter(|r| !r.message.is_empty())
            .map(|r| (r.step_index, r.message.as_str()))
            .collect();
        for &step in planned_steps {
            let state = match status_by_step.get(&step) {
                Some(StepStatus::Ok) => StepState::Valid,
                Some(StepStatus::NeedsRepair) => StepState::NeedsRepair,
                Some(StepStatus::OpFailed) => StepState::Error {
                    reason: message_by_step.get(&step).map_or_else(
                        || error_reason(step, &scratch.diagnostics_by_step),
                        |m| (*m).to_string(),
                    ),
                },
                // Beyond the last valid step: not executed ⇒ Dirty (Invariant 6).
                None => StepState::Dirty,
            };
            let _ = session.timeline.mark_state(step, state);
        }

        // Apply repair: a re-regen from `start` invalidates its bindings; publish
        // the fresh per-step NeedsRepair sets. SEEDED items survive both calls —
        // they are an edit-layer policy gate (SCHEMA §7.3), not a ladder outcome,
        // so a regen may not clear them.
        session.repair.clear_from(start);
        for (step, items) in &scratch.repair_by_step {
            session.repair.set_step(*step, items.clone());
        }

        // Component Library P3 WP-3.1: apply reseated mate placements — a
        // DERIVED writeback of that record's own `placement` field, same
        // `≤ last_valid` gate as body/element buffering above (a step beyond
        // the last valid one never reaches the accepted timeline, Invariant
        // 6). `set_place_component_placement` itself no-ops (never panics)
        // if the record moved/changed shape since the step index was
        // captured — belt-and-suspenders, not load-bearing here.
        if let Some(cutoff) = prepared.last_valid_step {
            for (&step, placement) in scratch.mate_placement_by_step.range(..=cutoff) {
                session
                    .timeline
                    .set_place_component_placement(step, (**placement).clone());
            }
            // WP-I: the same gate, the same derived-writeback treatment, for the
            // §7.2 `mateResolved` evidence. `adopt_place_component_mate_evidence`
            // refuses a record that already carries a `targetAxis`, so this can
            // only ever run ONCE per record — regen never rewrites frozen
            // evidence, and only the §9 repair re-freezes it.
            for (&step, resolved) in scratch.mate_resolved_by_step.range(..=cutoff) {
                session
                    .timeline
                    .adopt_place_component_mate_evidence(step, *resolved);
            }
        }
        // A seeded step was deliberately EXCLUDED from the plan (the caller's
        // execution ceiling), so the per-step loop above never touched its state.
        // Stamp it here so the timeline says NeedsRepair rather than Dirty — the
        // gate must be visible, not silent.
        for step in session.repair.seeded_steps() {
            let _ = session.timeline.mark_state(step, StepState::NeedsRepair);
        }

        // Build + publish the immutable snapshot (shared id + generation).
        let last_valid = prepared.last_valid_step;
        let signatures = last_valid.and_then(|s| scratch.step_signatures.get(&s).cloned());
        let diagnostics: Vec<Diagnostic> = scratch
            .diagnostics_by_step
            .values()
            .flatten()
            .take(64)
            .cloned()
            .collect();
        // Report the planned steps' states PLUS every seeded-gate step: the gate
        // steps are excluded from the plan by construction, so without this the
        // snapshot would never carry the state that explains the truncation.
        let mut reported: Vec<usize> = planned_steps.to_vec();
        for s in session.repair.seeded_steps() {
            if !reported.contains(&s) {
                reported.push(s);
            }
        }
        reported.sort_unstable();
        let step_states: Vec<(usize, StepState)> = reported
            .iter()
            .filter_map(|&s| session.timeline.state(s).map(|st| (s, st.clone())))
            .collect();
        let repair_summary = repair_summary(&session.repair);
        let bodies = body_snapshots(&session.bodies, lod, signatures.as_ref());

        let snapshot = publisher.publish(|generation| ModelSnapshot {
            id: accept.snapshot_id,
            generation,
            step_index: last_valid,
            bodies: bodies_with_generation(&bodies, generation),
            stopped_reason: prepared.stopped_reason,
            step_states,
            signatures,
            diagnostics,
            diagnostics_by_step: scratch.diagnostics_by_step,
            repair_summary,
        });
        // Key the inline meshes with the SAME generation `bodies_with_generation`
        // just stamped onto the published bodies — that is what makes them cache
        // HITS for the `get_mesh` this publish is about to trigger.
        self.seed_mesh_sink(&prepared.artifact_meshes, snapshot.generation);
        Outcome::Published(snapshot)
    }

    /// Deposits a published prepare's inline meshes into the sink (no-op without
    /// one). Called ONLY on the publish path: a discarded / superseded prepare must
    /// never seed a cache with geometry the document never adopted.
    fn seed_mesh_sink(&self, meshes: &[PreparedMeshRef], generation: u64) {
        let Some(sink) = &self.mesh_sink else {
            return;
        };
        if meshes.is_empty() {
            return;
        }
        let mut guard = sink.lock().unwrap_or_else(PoisonError::into_inner);
        guard.extend(meshes.iter().map(|m| {
            (
                MeshKey {
                    body: m.body,
                    lod: m.lod,
                    generation,
                },
                m.bytes.clone(),
            )
        }));
    }

    /// Cooperative-cancel path: cancel, **bounded-drain** the channel (the terminal
    /// frame is never dropped — SCHEMA §3.5/§5.4 — but a ~2s backstop stops a hung
    /// worker from stalling the executor forever, review F8), discard, no commit.
    async fn cancel_path(
        &self,
        job: JobId,
        receiver: &mut tokio::sync::mpsc::Receiver<PlanEvent>,
    ) -> Outcome {
        let _ = self.engine.cancel(job).await;
        let _ = tokio::time::timeout(DRAIN_BACKSTOP, async {
            while receiver.recv().await.is_some() {}
        })
        .await;
        self.discard(job).await;
        Outcome::Cancelled
    }

    /// Cancel path when the terminal was already consumed (no channel to drain):
    /// cancel + discard, no commit.
    async fn cancel_only(&self, job: JobId) -> Outcome {
        let _ = self.engine.cancel(job).await;
        self.discard(job).await;
        Outcome::Cancelled
    }

    /// Best-effort discard (a discard failure does not change the outcome — the
    /// scratch is dropped regardless).
    async fn discard(&self, job: JobId) {
        let _ = self.engine.discard_prepared(job).await;
    }
}

/// The bounded-drain backstop for the cancel path (review F8).
const DRAIN_BACKSTOP: std::time::Duration = std::time::Duration::from_secs(2);

/// The result of one drive attempt (see [`RegenExecutor::run_attempt`]).
enum AttemptOutcome {
    /// A settled terminal outcome.
    Settled(Outcome),
    /// The checkpoint plan failed before any step event; retry from 0 (review F12).
    RetryFromZero,
}

/// The stream terminal.
enum Terminal {
    Prepared(PlanPrepared),
    Failed(EngineError),
    Crashed,
    /// A cancel was observed mid-await (SCHEMA §3.5) — take the cancel path.
    CancelRequested,
}

/// Grouped `on_prepared` inputs (keeps the arg list readable; the borrowed slices
/// avoid cloning the planned steps / prefix hashes per call).
struct OnPrepared<'a> {
    prepared: PlanPrepared,
    job: JobId,
    plan_rev: DocumentRevision,
    epoch: WorkerEpoch,
    start: usize,
    lod: Lod,
    planned_steps: &'a [usize],
    streamed_steps: &'a [usize],
    expected_base_hash: &'a HistoryPrefixHash,
    prefix_hashes: &'a [HistoryPrefixHash],
}

/// Validates the worker's streamed/terminal execution summary against the exact
/// ordered op slice Rust requested. Any mismatch is a protocol error before accept.
fn validate_prepared_stream(
    prepared: &PlanPrepared,
    planned: &[usize],
    streamed: &[usize],
) -> Result<(), String> {
    if planned.is_empty() || planned.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(format!("ExecutePlan has invalid ordered steps {planned:?}"));
    }
    let rows = &prepared.per_step;
    if rows.is_empty() || rows.len() > planned.len() {
        return Err(format!(
            "PlanPrepared perStepResults cardinality {} invalid for {} planned steps",
            rows.len(),
            planned.len()
        ));
    }
    for (position, row) in rows.iter().enumerate() {
        if row.step_index != planned[position] {
            return Err(format!(
                "PlanPrepared perStepResults[{position}].stepIndex {} != planned step {}",
                row.step_index, planned[position]
            ));
        }
    }
    validate_terminal(prepared, planned)?;
    let streamed_prefix_len = match prepared.stopped_reason {
        StoppedReason::Completed | StoppedReason::NeedsRepair => rows.len(),
        StoppedReason::OpFailed => rows.len() - 1,
    };
    if streamed != &planned[..streamed_prefix_len] {
        return Err(format!(
            "planStep sequence {streamed:?} != executed prefix {:?}",
            &planned[..streamed_prefix_len]
        ));
    }
    Ok(())
}

fn validate_terminal(prepared: &PlanPrepared, planned: &[usize]) -> Result<(), String> {
    let rows = &prepared.per_step;
    let terminal_position = rows.len() - 1;
    let expected_last_valid = terminal_position
        .checked_sub(1)
        .map(|position| planned[position]);
    let (expected_terminal, expected_len, expected_last) = match prepared.stopped_reason {
        StoppedReason::Completed => (StepStatus::Ok, planned.len(), planned.last().copied()),
        StoppedReason::OpFailed => (StepStatus::OpFailed, rows.len(), expected_last_valid),
        StoppedReason::NeedsRepair => (StepStatus::NeedsRepair, rows.len(), expected_last_valid),
    };
    if rows.len() != expected_len
        || rows[..terminal_position]
            .iter()
            .any(|row| row.status != StepStatus::Ok)
        || rows[terminal_position].status != expected_terminal
        || prepared.last_valid_step != expected_last
    {
        return Err(format!(
            "PlanPrepared terminal inconsistent: stoppedReason={:?}, lastValidStep={:?}, statuses={:?}",
            prepared.stopped_reason,
            prepared.last_valid_step,
            rows.iter().map(|row| row.status).collect::<Vec<_>>()
        ));
    }
    Ok(())
}

/// The opaque history-prefix token the worker is expected to echo for a prepare
/// whose last valid step is `last_valid` (X-WP1 item 2): `prefix_hashes[j]` for the
/// executed op at that step, or `expected_base_hash` for a base-only prepare (or a
/// step not found in the plan — a safe fall-back that still catches divergence).
fn expected_prefix_echo<'a>(
    last_valid: Option<usize>,
    planned_steps: &[usize],
    expected_base_hash: &'a HistoryPrefixHash,
    prefix_hashes: &'a [HistoryPrefixHash],
) -> &'a HistoryPrefixHash {
    match last_valid {
        Some(step) => planned_steps
            .iter()
            .position(|&s| s == step)
            .and_then(|j| prefix_hashes.get(j))
            .unwrap_or(expected_base_hash),
        None => expected_base_hash,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Marks each planned step Dirty (preserving `Suppressed`). Used on engine
/// failure — the plan produced no accepted snapshot, so its steps must regen.
fn mark_dirty_range(timeline: &mut Timeline, planned_steps: &[usize]) {
    for &step in planned_steps {
        if timeline.state(step) != Some(&StepState::Suppressed) {
            let _ = timeline.mark_state(step, StepState::Dirty);
        }
    }
}

/// The human-facing reason for a failed step — the first `Error`-severity
/// diagnostic's message, else a generic reason.
fn error_reason(step: usize, diagnostics: &BTreeMap<usize, Vec<Diagnostic>>) -> String {
    diagnostics
        .get(&step)
        .and_then(|ds| ds.iter().find(|d| d.severity == Severity::Error))
        .map(|d| d.message.clone())
        .unwrap_or_else(|| format!("operation failed at step {step}"))
}

/// A compact repair summary from the committed repair state.
fn repair_summary(repair: &RepairState) -> RepairSummary {
    let mut steps: Vec<usize> = repair.items().iter().map(|i| i.step_index).collect();
    steps.sort_unstable();
    steps.dedup();
    RepairSummary {
        needs_repair_count: repair.len(),
        steps,
    }
}

/// One [`BodySnapshot`] per active body (generation filled in later so the mesh
/// key matches the published generation).
///
/// **Per-body signature coarseness (review F23).** Every body currently gets the
/// *same* `geometry` signature — the last-valid step's whole-model geometry
/// signature — rather than a per-body one. This is coarse but sound for drift
/// detection at the model level (Invariant 5): a change anywhere flips the shared
/// signature. A true per-body signature needs the worker to emit body-scoped
/// signatures in the `planStep` event; until then this whole-model value is the
/// honest approximation. Consumers MUST NOT treat two bodies sharing a signature
/// as geometrically identical.
fn body_snapshots(
    bodies: &BodyRegistry,
    lod: Lod,
    signatures: Option<&StepSignatures>,
) -> Vec<BodySnapshot> {
    let sig = signatures
        .map(|s| s.geometry.clone())
        .unwrap_or_else(|| super::engine::Signature::new(String::new()));
    bodies
        .bodies()
        .iter()
        .map(|b| BodySnapshot {
            body: b.id,
            // Placeholder generation 0 — replaced by `bodies_with_generation`.
            mesh_key: MeshKey {
                body: b.id,
                lod,
                generation: 0,
            },
            signature: sig.clone(),
            visible: b.visible,
        })
        .collect()
}

/// Stamps the published `generation` onto each body's mesh key (Invariant 4:
/// bodies/meshes of one publish share the generation).
fn bodies_with_generation(bodies: &[BodySnapshot], generation: u64) -> Vec<BodySnapshot> {
    bodies
        .iter()
        .map(|b| BodySnapshot {
            mesh_key: MeshKey {
                generation,
                ..b.mesh_key
            },
            ..b.clone()
        })
        .collect()
}
