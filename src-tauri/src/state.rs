//! Shared application state, managed as `tauri::State<AppState>`.
//!
//! V1 = a single open document behind an async [`tokio::sync::Mutex`] — the
//! single-writer lock the [`DocumentRuntime`] and the regen driver share (plan
//! "DocumentSession single writer"). Commands and the scheduler's
//! [`RegenDriver`](onecad_core::regen::RegenDriver) both lock this one runtime, so
//! writes serialize deterministically.
//!
//! ## Backend factory (R-WP11)
//!
//! [`AppState::default`] builds a factory that spawns a real
//! [`WorkerManager`]-backed backend when the worker binary resolves
//! ([`resolve_worker_path`]), wiring its **restart hook** to bump the document's
//! epoch + enqueue a replay (SCHEMA §8 crash → restart + replay). When no binary
//! is present it falls back to [`PendingBackend`] so the app still boots. The
//! factory also produces the [`GeometryExporter`] (the same `WorkerManager` Arc) that
//! `export_step_file` drives, and spawns a **worker-status forwarder** that relays
//! [`WorkerLifecycle`] transitions to the webview as `worker-status` events.
//!
//! ## Worker slots: `warm` + `live`
//!
//! Two slots own every worker process the app ever spawns, and between them they
//! give each sidecar a bounded lifetime:
//!
//! * **`warm`** — a worker spawned at app start ([`AppState::prewarm`]) and again
//!   after a close, handshaking in the background while the user is still at the
//!   start screen. The factory **adopts** it at the next document open, so the open
//!   pays no cold start. An unusable (`Failed`/`Retired`) warm worker is retired and
//!   the factory cold-spawns instead — a corpse is never adopted.
//! * **`live`** — the OPEN document's worker. A new backend displaces it, and
//!   [`AppState::commit_backend`] retires it once the new document is really open.
//!   That is the fix for the process leak: before it, `WorkerManager::shutdown` had
//!   zero callers, so every new/open spawned a sidecar and orphaned the previous one
//!   for the lifetime of the app.
//!
//! Both are retired together on quit ([`AppState::retire_all`]).
//!
//! ### The swap is two-phase, and that is load-bearing
//!
//! `new_document` / `open_document` / `import_step` / `recover_document` all call
//! [`make_backend`](AppState::make_backend) **before** the work that can fail
//! (reading the container, probing a STEP file) and while the PREVIOUS document is
//! still the live one. So the swap only stages: `make_backend` installs the new
//! backend and holds the displaced one in [`DisplacedBackend`], and the caller then
//! reaches exactly one of
//!
//! * [`commit_backend`](AppState::commit_backend) — after `*guard = Some(rt)`: the
//!   old document is gone, retire its worker;
//! * [`rollback_backend`](AppState::rollback_backend) — on the error path: restore
//!   the previous backend *whole* (worker + all six side seams) and retire the one
//!   the failed open built.
//!
//! Retiring inside `make_backend` would kill a surviving document's worker every
//! time a user picked an unreadable file — and `importStep` explicitly keeps the
//! user on their current document when the probe fails.

use std::sync::{Arc, Mutex as StdMutex, OnceLock, RwLock};

use tauri::{AppHandle, Emitter};
use tokio::sync::{watch, Mutex};

use onecad_core::io::recovery::RecoveryOffer;
use onecad_core::regen::{GeometryEngine, RegenRequest, SchedulerHandle};

use crate::document_runtime::DocumentRuntime;
use crate::dto::WorkerStatusDto;
use crate::events;
use crate::export::GeometryExporter;
use crate::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PendingBackend, PreviewEngine, SolverEngine, StepImport, SupervisorConfig, WorkerLifecycle,
    WorkerManager, WorkerReadiness, WorkerState,
};

/// The geometry backend split into its three facets (the executor drives the
/// [`GeometryEngine`]; the mesh cache pulls bytes from the [`MeshProvider`]; the
/// sketch flow drives the [`SolverEngine`] lane, SCHEMA §7.4).
pub type BackendPair = (
    Arc<dyn GeometryEngine>,
    Arc<dyn MeshProvider>,
    Arc<dyn SolverEngine>,
);

/// A backend bundle: the [`BackendPair`] facets plus the read-only side seams for
/// the same worker — [`GeometryExporter`] (`export_step_file`), [`ElementQuery`]
/// (`face_sketch_plane` / `element_info`), [`PreviewEngine`] (`preview_op`),
/// [`FaceBoundaryProjection`] (`add_sketch_on_face`), [`StepImport`]
/// (`import_step` / `insert_step`), [`CircuitControl`] (`clear_worker_circuit`)
/// and [`WorkerReadiness`] (the cold-worker regen-race gate). Same `WorkerManager`
/// Arc throughout.
pub type BackendBundle = (
    Arc<dyn GeometryEngine>,
    Arc<dyn MeshProvider>,
    Arc<dyn SolverEngine>,
    Arc<dyn GeometryExporter>,
    Arc<dyn ElementQuery>,
    Arc<dyn PreviewEngine>,
    Arc<dyn FaceBoundaryProjection>,
    Arc<dyn StepImport>,
    Arc<dyn CircuitControl>,
    Arc<dyn WorkerReadiness>,
);

/// Builds a fresh backend bundle for a newly opened document.
pub type BackendFactory = Arc<dyn Fn() -> BackendBundle + Send + Sync>;

/// One of the two worker slots (`warm` / `live`). Shared with the backend factory,
/// which is built before `AppState` exists and therefore cannot reach its fields.
type WorkerSlot = Arc<StdMutex<Option<WorkerManager>>>;

/// Everything one [`AppState::make_backend`] call displaced, held until the open
/// resolves. [`AppState::commit_backend`] throws it away (retiring its worker);
/// [`AppState::rollback_backend`] puts it all back.
///
/// It exists because the open commands do their fallible work — `ContainerReader`,
/// the STEP probe — *after* the swap, while the previous document is still live. A
/// failed open must leave that document exactly as it was.
struct DisplacedBackend {
    /// The previous document's worker (`None` on the [`PendingBackend`] lane).
    worker: Option<WorkerManager>,
    exporter: Arc<dyn GeometryExporter>,
    element_query: Arc<dyn ElementQuery>,
    preview: Arc<dyn PreviewEngine>,
    face_projection: Arc<dyn FaceBoundaryProjection>,
    step_import: Arc<dyn StepImport>,
    circuit: Arc<dyn CircuitControl>,
    readiness: Arc<dyn WorkerReadiness>,
}

/// A shared, set-once regen scheduler handle (the setup wires it; the factory's
/// restart hook reads it to enqueue a replay).
pub type SharedScheduler = Arc<OnceLock<SchedulerHandle>>;

/// A shared, set-once [`AppHandle`] — the setup fills it so the factory's
/// worker-status forwarder can emit events (it is built before the handle exists).
pub type SharedAppHandle = Arc<OnceLock<AppHandle>>;

/// Root application state handed to every command.
pub struct AppState {
    /// The single open document (V1), or `None` when nothing is open. The regen
    /// driver and every command lock this one runtime (single writer).
    pub runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    /// The **persistence lane** — serializes the two writers that can touch the same
    /// on-disk recovery state: an autosave (container + crash marker) and an explicit
    /// save (container + marker/autosave *clearing*).
    ///
    /// Not a general IO lock. It exists for exactly two races opened by moving the
    /// container write off the runtime lock (VF-B7):
    ///
    /// * **M3** — two concurrent writers on one target path eat each other's temps
    ///   (`ContainerWriter` sweeps sibling temps with
    ///   [`cleanup_stale_temps`](onecad_core::io::container) before its rename);
    /// * **M2** — an in-flight autosave landing *after* a save cleared the crash
    ///   marker would resurrect a bogus recovery offer for a cleanly-saved document.
    ///
    /// **Lock order is `runtime` → release → `persistence`, never nested.** No path
    /// may await this lane while holding the runtime lock: both lanes take the
    /// runtime lock first (to build a [`SavePayload`](crate::document_runtime::SavePayload)),
    /// release it, and only then contend here.
    pub persistence: Arc<Mutex<()>>,
    /// The regen scheduler control surface, set once in `crate::run`'s setup.
    pub scheduler: SharedScheduler,
    /// The app handle, set once in `crate::run`'s setup (the worker-status
    /// forwarder + any late event emitter read it).
    pub app: SharedAppHandle,
    /// A monotonic "document mutated" tick the autosave driver debounces on. Every
    /// document-mutating command bumps it via [`note_mutation`](Self::note_mutation);
    /// the driver subscribes in `crate::run`'s setup (the autosave signal seam).
    pub autosave_tick: Arc<watch::Sender<u64>>,
    /// The crash-recovery offer surfaced at startup by
    /// [`check_recovery`](crate::api::check_recovery), consumed by
    /// [`recover_document`](crate::api::recover_document). `None` until scanned / after
    /// a decision (V1 single-document ⇒ at most one offer).
    pub pending_recovery: StdMutex<Option<RecoveryOffer>>,
    /// The current document's STEP exporter (the same `WorkerManager` Arc the
    /// backend uses, or [`PendingBackend`] when no worker). Swapped by
    /// [`make_backend`](AppState::make_backend) on every new/open.
    exporter: RwLock<Arc<dyn GeometryExporter>>,
    /// The current document's element-geometry reader (same `WorkerManager` Arc),
    /// used by `face_sketch_plane`. Swapped alongside the exporter.
    element_query: RwLock<Arc<dyn ElementQuery>>,
    /// The current document's drag-time preview engine (same `WorkerManager` Arc).
    preview: RwLock<Arc<dyn PreviewEngine>>,
    /// The current document's host-face boundary projector (same `WorkerManager`
    /// Arc), used by `add_sketch_on_face`. Swapped alongside the exporter.
    face_projection: RwLock<Arc<dyn FaceBoundaryProjection>>,
    /// The current document's STEP import probe (same `WorkerManager` Arc), used by
    /// `import_step` / `insert_step`. Swapped alongside the exporter.
    step_import: RwLock<Arc<dyn StepImport>>,
    /// The current document's crash-circuit control (same `WorkerManager` Arc), used
    /// by `clear_worker_circuit`. Swapped alongside the exporter.
    circuit: RwLock<Arc<dyn CircuitControl>>,
    /// The current document's worker-readiness gate (same `WorkerManager` Arc),
    /// used to hold a freshly opened/imported/recovered document's first regen
    /// request until the worker can actually serve it. Swapped alongside the
    /// exporter.
    readiness: RwLock<Arc<dyn WorkerReadiness>>,
    backend_factory: BackendFactory,
    /// The pre-warmed worker awaiting adoption (see the module docs). Shared with
    /// the factory, which takes it at the next document open.
    warm: WorkerSlot,
    /// The OPEN document's worker, retired once a newer one is COMMITTED.
    live: WorkerSlot,
    /// The in-flight backend swap's displaced half — `Some` between
    /// [`make_backend`](AppState::make_backend) and its commit/rollback.
    displaced: StdMutex<Option<DisplacedBackend>>,
}

impl AppState {
    /// Builds state over an explicit backend factory (tests inject a scripted
    /// backend). Production uses [`AppState::default`].
    ///
    /// The `warm`/`live` slots exist but stay empty: only the production factory
    /// populates them, so a scripted backend has no worker lifecycle to manage and
    /// [`prewarm`](Self::prewarm) / [`retire_all`](Self::retire_all) are no-ops.
    #[must_use]
    pub fn new(backend_factory: BackendFactory) -> Self {
        Self {
            warm: Arc::new(StdMutex::new(None)),
            live: Arc::new(StdMutex::new(None)),
            displaced: StdMutex::new(None),
            runtime: Arc::new(Mutex::new(None)),
            persistence: Arc::new(Mutex::new(())),
            scheduler: Arc::new(OnceLock::new()),
            app: Arc::new(OnceLock::new()),
            autosave_tick: Arc::new(watch::channel(0u64).0),
            pending_recovery: StdMutex::new(None),
            exporter: RwLock::new(Arc::new(PendingBackend)),
            element_query: RwLock::new(Arc::new(PendingBackend)),
            preview: RwLock::new(Arc::new(PendingBackend)),
            face_projection: RwLock::new(Arc::new(PendingBackend)),
            step_import: RwLock::new(Arc::new(PendingBackend)),
            circuit: RwLock::new(Arc::new(PendingBackend)),
            readiness: RwLock::new(Arc::new(PendingBackend)),
            backend_factory,
        }
    }

    /// Signals the autosave driver that the open document changed (every
    /// document-mutating command calls this — the autosave debounce seam).
    pub fn note_mutation(&self) {
        self.autosave_tick.send_modify(|v| *v = v.wrapping_add(1));
    }

    /// A fresh backend pair for a new/open document. Also swaps in the matching
    /// [`GeometryExporter`] so a later `export_step_file` routes to this document's
    /// worker.
    ///
    /// **This is the FIRST half of a two-phase swap and it is not complete on its
    /// own.** Every caller must reach exactly one of
    /// [`commit_backend`](Self::commit_backend) (after the new runtime is published)
    /// or [`rollback_backend`](Self::rollback_backend) (if the open fails). The
    /// displaced backend is held intact until then, because the open commands all do
    /// their fallible work — reading the container, probing a STEP file — *after*
    /// this call while the PREVIOUS document is still the live one. Retiring its
    /// worker here would leave a perfectly good document with a dead sidecar every
    /// time a user picked an unreadable file.
    #[must_use]
    pub fn make_backend(&self) -> BackendPair {
        let displaced = self.snapshot_backend();
        let (
            engine,
            meshes,
            solver,
            exporter,
            elements,
            preview,
            face_projection,
            step_import,
            circuit,
            readiness,
        ) = (self.backend_factory)();
        if let Ok(mut slot) = self.exporter.write() {
            *slot = exporter;
        }
        if let Ok(mut slot) = self.element_query.write() {
            *slot = elements;
        }
        if let Ok(mut slot) = self.preview.write() {
            *slot = preview;
        }
        if let Ok(mut slot) = self.face_projection.write() {
            *slot = face_projection;
        }
        if let Ok(mut slot) = self.step_import.write() {
            *slot = step_import;
        }
        if let Ok(mut slot) = self.circuit.write() {
            *slot = circuit;
        }
        if let Ok(mut slot) = self.readiness.write() {
            *slot = readiness;
        }
        *self.displaced.lock().unwrap() = Some(displaced);
        (engine, meshes, solver)
    }

    /// Captures the current backend — worker plus all six side seams — so a failed
    /// open can restore it wholesale.
    ///
    /// Snapshotting the six facets also closes a **pre-existing** half-break: they
    /// were already swapped out from under a document that survived a failed open, so
    /// `export_step_file` / `preview_op` / `insert_step` on it silently routed to the
    /// next document's worker.
    fn snapshot_backend(&self) -> DisplacedBackend {
        DisplacedBackend {
            worker: self.live_worker(),
            exporter: self.exporter(),
            element_query: self.element_query(),
            preview: self.preview(),
            face_projection: self.face_projection(),
            step_import: self.step_import(),
            circuit: self.circuit(),
            readiness: self.readiness(),
        }
    }

    /// Second half of the swap, on the SUCCESS path: the new runtime is published, so
    /// the document the old worker served is gone and that worker is retired. This is
    /// the point the process leak actually closes.
    ///
    /// No-op when nothing is pending (so a double call, or a caller that never
    /// swapped, is harmless).
    pub fn commit_backend(&self) {
        let Some(displaced) = self.displaced.lock().unwrap().take() else {
            return;
        };
        let Some(previous) = displaced.worker else {
            return; // the PendingBackend lane — no worker to retire.
        };
        // Defensive: never retire the manager that is currently live (a factory that
        // handed back the same one would otherwise kill the new document's worker).
        if self
            .live_worker()
            .is_some_and(|live| live.is_same(&previous))
        {
            return;
        }
        previous.retire();
    }

    /// Second half of the swap, on the FAILURE path: the open never published a
    /// runtime, so the PREVIOUS document is still the live one and must be handed
    /// back exactly what it had — worker included. The backend the failed open built
    /// is retired instead, and a replacement is pre-warmed so the user's next attempt
    /// is still instant.
    ///
    /// No-op when nothing is pending.
    pub fn rollback_backend(&self) {
        let Some(displaced) = self.displaced.lock().unwrap().take() else {
            return;
        };
        let incoming = self.live_worker();
        // Restore all seven side seams (`export_step_file`, `face_sketch_plane`,
        // `preview_op`, `add_sketch_on_face`, `insert_step`, `clear_worker_circuit`,
        // the readiness gate) plus the live worker, so the surviving document is
        // whole again.
        *self.exporter.write().unwrap() = displaced.exporter;
        *self.element_query.write().unwrap() = displaced.element_query;
        *self.preview.write().unwrap() = displaced.preview;
        *self.face_projection.write().unwrap() = displaced.face_projection;
        *self.step_import.write().unwrap() = displaced.step_import;
        *self.circuit.write().unwrap() = displaced.circuit;
        *self.readiness.write().unwrap() = displaced.readiness;
        *self.live.lock().unwrap() = displaced.worker.clone();

        if let Some(incoming) = incoming {
            if displaced
                .worker
                .is_some_and(|previous| previous.is_same(&incoming))
            {
                return; // nothing was actually swapped.
            }
            // The failed open's worker is unreachable now. Retire rather than return
            // it to `warm`: the open may well have failed *because* of it, and a fresh
            // pre-warm costs one spawn on a path the user rarely hits.
            incoming.retire();
            self.prewarm();
        }
    }

    /// The current document's STEP exporter (see [`make_backend`](Self::make_backend)).
    #[must_use]
    pub fn exporter(&self) -> Arc<dyn GeometryExporter> {
        self.exporter.read().unwrap().clone()
    }

    /// The current document's element-geometry reader (see [`make_backend`](Self::make_backend)).
    #[must_use]
    pub fn element_query(&self) -> Arc<dyn ElementQuery> {
        self.element_query.read().unwrap().clone()
    }

    /// The current document's preview engine (see [`make_backend`](Self::make_backend)).
    #[must_use]
    pub fn preview(&self) -> Arc<dyn PreviewEngine> {
        self.preview.read().unwrap().clone()
    }

    /// The current document's host-face boundary projector
    /// (see [`make_backend`](Self::make_backend)).
    #[must_use]
    pub fn face_projection(&self) -> Arc<dyn FaceBoundaryProjection> {
        self.face_projection.read().unwrap().clone()
    }

    /// The current document's STEP import probe (see [`make_backend`](Self::make_backend)).
    #[must_use]
    pub fn step_import(&self) -> Arc<dyn StepImport> {
        self.step_import.read().unwrap().clone()
    }

    /// The current document's crash-circuit control (see [`make_backend`](Self::make_backend)).
    #[must_use]
    pub fn circuit(&self) -> Arc<dyn CircuitControl> {
        self.circuit.read().unwrap().clone()
    }

    /// The current document's worker-readiness gate (see [`make_backend`](Self::make_backend)).
    #[must_use]
    pub fn readiness(&self) -> Arc<dyn WorkerReadiness> {
        self.readiness.read().unwrap().clone()
    }

    /// Spawns the pre-warmed worker so the NEXT document open adopts a sidecar that
    /// is already through (or well into) its OCW1 handshake instead of paying cold
    /// start. Returns immediately — the handshake completes in the background and
    /// [`make_backend`](Self::make_backend) adopts whatever state it has reached.
    ///
    /// No-op when a warm worker already exists (so it can never stack processes) or
    /// when no worker binary resolves. Must be called from a Tokio context: the
    /// supervisor and the status forwarder are spawned tasks.
    pub fn prewarm(&self) {
        if self.warm.lock().unwrap().is_some() {
            return;
        }
        let Some(path) = resolve_worker_path() else {
            return; // no binary ⇒ PendingBackend lane; nothing to warm.
        };
        let wm = WorkerManager::spawn(SupervisorConfig::production(path));
        // Hook + forwarder are installed HERE rather than at adoption so a warm
        // worker that flaps before it is adopted is still supervised and still
        // reported. With no document open the hook's replay is a no-op.
        install_restart_hook(&wm, self.runtime.clone(), self.scheduler.clone());
        spawn_status_forwarder(&wm, self.app.clone());
        *self.warm.lock().unwrap() = Some(wm);
        tracing::info!("geometry worker pre-warmed");
    }

    /// The `close_document` lane: retires the closed document's worker (it can serve
    /// nothing now) and pre-warms its replacement, so the next open is instant and
    /// exactly one sidecar is alive across the transition.
    pub fn rewarm(&self) {
        retire_slot(&self.live);
        self.prewarm();
    }

    /// Terminal teardown of BOTH slots — the app-quit path. Idempotent.
    ///
    /// Deliberately does not wait: each worker gets its graceful `Shutdown` frame
    /// and, failing that, its stdin closes when this process exits, which every OCW1
    /// worker treats as "exit now". Blocking the quit on a wedged sidecar would buy
    /// nothing.
    pub fn retire_all(&self) {
        retire_slot(&self.warm);
        retire_slot(&self.live);
    }

    /// The pre-warmed worker awaiting adoption, if any (introspection / tests).
    #[must_use]
    pub fn warm_worker(&self) -> Option<WorkerManager> {
        self.warm.lock().unwrap().clone()
    }

    /// The open document's worker, if any (introspection / tests).
    #[must_use]
    pub fn live_worker(&self) -> Option<WorkerManager> {
        self.live.lock().unwrap().clone()
    }
}

impl Default for AppState {
    fn default() -> Self {
        let runtime = Arc::new(Mutex::new(None));
        let scheduler: SharedScheduler = Arc::new(OnceLock::new());
        let app: SharedAppHandle = Arc::new(OnceLock::new());
        let warm: WorkerSlot = Arc::new(StdMutex::new(None));
        let live: WorkerSlot = Arc::new(StdMutex::new(None));
        let backend_factory = real_worker_factory(
            runtime.clone(),
            scheduler.clone(),
            app.clone(),
            warm.clone(),
            live.clone(),
        );
        Self {
            warm,
            live,
            displaced: StdMutex::new(None),
            runtime,
            persistence: Arc::new(Mutex::new(())),
            scheduler,
            app,
            autosave_tick: Arc::new(watch::channel(0u64).0),
            pending_recovery: StdMutex::new(None),
            exporter: RwLock::new(Arc::new(PendingBackend)),
            element_query: RwLock::new(Arc::new(PendingBackend)),
            preview: RwLock::new(Arc::new(PendingBackend)),
            face_projection: RwLock::new(Arc::new(PendingBackend)),
            step_import: RwLock::new(Arc::new(PendingBackend)),
            circuit: RwLock::new(Arc::new(PendingBackend)),
            readiness: RwLock::new(Arc::new(PendingBackend)),
            backend_factory,
        }
    }
}

/// The production factory: **adopt** the pre-warmed worker when there is a usable
/// one, else spawn a fresh [`WorkerManager`] over the resolved binary; else
/// [`PendingBackend`]. Either way the resulting manager is installed as `live`,
/// which retires the previous document's worker.
///
/// The restart hook captures the shared runtime + scheduler so a worker restart
/// re-drives geometry against the freshly-bumped epoch; each real worker also gets a
/// [`WorkerLifecycle`] forwarder emitting `worker-status`. Both are installed once
/// per manager — at pre-warm time for an adopted one, here for a cold spawn.
fn real_worker_factory(
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    scheduler: SharedScheduler,
    app: SharedAppHandle,
    warm: WorkerSlot,
    live: WorkerSlot,
) -> BackendFactory {
    Arc::new(move || {
        let wm = match take_warm(&warm) {
            Some(adopted) => adopted,
            None => {
                let Some(path) = resolve_worker_path() else {
                    return pending_bundle();
                };
                let wm = WorkerManager::spawn(SupervisorConfig::production(path));
                install_restart_hook(&wm, runtime.clone(), scheduler.clone());
                spawn_status_forwarder(&wm, app.clone());
                wm
            }
        };
        adopt_live(&live, &wm);
        bundle_from(wm)
    })
}

/// Takes the pre-warmed worker **iff** it can still become useful. A `Failed` or
/// already-`Retired` warm worker is retired and dropped so the caller cold-spawns a
/// healthy one — adopting a corpse would hand the new document a backend that can
/// never serve a request.
fn take_warm(warm: &WorkerSlot) -> Option<WorkerManager> {
    let candidate = warm.lock().unwrap().take()?;
    match candidate.state() {
        WorkerState::Starting | WorkerState::Ready | WorkerState::Restarting => Some(candidate),
        WorkerState::Failed | WorkerState::Retired => {
            candidate.retire();
            None
        }
    }
}

/// Installs `wm` as the open document's worker.
///
/// It deliberately does NOT retire whoever held the slot: at this point the open can
/// still fail, and the previous document would then be left with a dead sidecar. The
/// displaced manager is captured by [`AppState::make_backend`] and disposed of by
/// [`AppState::commit_backend`] / [`AppState::rollback_backend`].
fn adopt_live(live: &WorkerSlot, wm: &WorkerManager) {
    *live.lock().unwrap() = Some(wm.clone());
}

/// Retires and clears a slot. Idempotent (an empty slot is a no-op).
fn retire_slot(slot: &WorkerSlot) {
    let taken = slot.lock().unwrap().take();
    if let Some(wm) = taken {
        wm.retire();
    }
}

/// Wires a worker's restart hook to mark the open document dirty + enqueue a full
/// replay (SCHEMA §8 crash → restart + replay). With no document open the hook is a
/// no-op, which is what makes it safe to install on a not-yet-adopted warm worker.
fn install_restart_hook(
    wm: &WorkerManager,
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    scheduler: SharedScheduler,
) {
    wm.set_restart_hook(Arc::new(move |epoch| {
        let rt = runtime.clone();
        let sch = scheduler.clone();
        tokio::spawn(async move {
            {
                let mut guard = rt.lock().await;
                if let Some(doc) = guard.as_mut() {
                    doc.on_worker_restart(epoch);
                }
            }
            if let Some(handle) = sch.get() {
                handle.request(RegenRequest::ToEnd { from: 0 });
            }
        });
    }));
}

/// The ten trait facets of one [`BackendBundle`], all over the SAME
/// `WorkerManager` Arc.
fn bundle_from(wm: WorkerManager) -> BackendBundle {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let exporter: Arc<dyn GeometryExporter> = Arc::new(wm.clone());
    let elements: Arc<dyn ElementQuery> = Arc::new(wm.clone());
    let preview: Arc<dyn PreviewEngine> = Arc::new(wm.clone());
    let face_projection: Arc<dyn FaceBoundaryProjection> = Arc::new(wm.clone());
    let step_import: Arc<dyn StepImport> = Arc::new(wm.clone());
    let circuit: Arc<dyn CircuitControl> = Arc::new(wm.clone());
    let readiness: Arc<dyn WorkerReadiness> = Arc::new(wm);
    (
        engine,
        meshes,
        solver,
        exporter,
        elements,
        preview,
        face_projection,
        step_import,
        circuit,
        readiness,
    )
}

/// The no-worker fallback bundle (no binary resolved) — the app still boots.
fn pending_bundle() -> BackendBundle {
    let backend = Arc::new(PendingBackend);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend.clone();
    let exporter: Arc<dyn GeometryExporter> = backend.clone();
    let elements: Arc<dyn ElementQuery> = backend.clone();
    let preview: Arc<dyn PreviewEngine> = backend.clone();
    let face_projection: Arc<dyn FaceBoundaryProjection> = backend.clone();
    let step_import: Arc<dyn StepImport> = backend.clone();
    let circuit: Arc<dyn CircuitControl> = backend.clone();
    let readiness: Arc<dyn WorkerReadiness> = backend;
    (
        engine,
        meshes,
        solver,
        exporter,
        elements,
        preview,
        face_projection,
        step_import,
        circuit,
        readiness,
    )
}

/// Subscribes to a worker's [`WorkerLifecycle`] broadcast and relays each
/// transition to the webview as a `worker-status` event, plus one immediate emit
/// of the current state at subscription time (so a late-loading webview still
/// learns the state without a separate fetch).
fn spawn_status_forwarder(wm: &WorkerManager, app: SharedAppHandle) {
    let mut rx = wm.subscribe();
    let initial = status_from_state(wm.state(), wm.epoch().0);
    tokio::spawn(async move {
        if let Some(handle) = app.get() {
            let _ = handle.emit(events::WORKER_STATUS, &initial);
        }
        while let Ok(ev) = rx.recv().await {
            if let Some(dto) = status_from_lifecycle(&ev) {
                if let Some(handle) = app.get() {
                    let _ = handle.emit(events::WORKER_STATUS, &dto);
                }
            }
        }
    });
}

/// Maps a current [`WorkerState`] to a `worker-status` payload (initial emit).
fn status_from_state(state: WorkerState, epoch: u64) -> WorkerStatusDto {
    let state = match state {
        WorkerState::Starting => "starting",
        WorkerState::Ready => "ready",
        WorkerState::Restarting => "restarting",
        // Retirement is deliberately NOT a new wire value: `WorkerStatusDto.state` is
        // a frozen four-value contract, and to the UI a retired worker is exactly a
        // worker that will not serve requests.
        WorkerState::Failed | WorkerState::Retired => "failed",
    };
    WorkerStatusDto {
        state: state.into(),
        epoch,
    }
}

/// Maps a [`WorkerLifecycle`] transition to a `worker-status` payload. `CircuitOpen`
/// is a per-plan poison event (not a worker-level state), so it is not forwarded.
fn status_from_lifecycle(ev: &WorkerLifecycle) -> Option<WorkerStatusDto> {
    match ev {
        WorkerLifecycle::Ready { epoch, .. } => Some(WorkerStatusDto {
            state: "ready".into(),
            epoch: *epoch,
        }),
        WorkerLifecycle::Restarting { epoch, .. } => Some(WorkerStatusDto {
            state: "restarting".into(),
            epoch: *epoch,
        }),
        WorkerLifecycle::Failed { .. } => Some(WorkerStatusDto {
            state: "failed".into(),
            epoch: 0,
        }),
        WorkerLifecycle::CircuitOpen { .. } => None,
    }
}

#[cfg(test)]
mod slot_tests {
    //! The `warm`/`live` slot mechanics — adoption, rejection of an unusable warm
    //! worker, and the retire-the-previous-live rule that closes the process leak.
    //!
    //! Driven over managers whose binary path cannot exist: every assertion here is
    //! about the slots, so a real sidecar would only add a spawn race. `Starting` /
    //! `Restarting` are the states such a manager passes through, and both are
    //! adoptable — which is the point (a warm worker mid-handshake is still the one
    //! the next open should take).

    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    /// A manager over a nonexistent binary. `backoff` is effectively infinite so it
    /// parks in `Restarting` instead of racing toward `Failed` mid-assertion.
    fn unstartable() -> WorkerManager {
        let mut config = SupervisorConfig::production(PathBuf::from("/nonexistent/onecad-worker"));
        config.backoff = vec![Duration::from_secs(3600)];
        config.max_rapid_deaths = u32::MAX;
        WorkerManager::spawn(config)
    }

    /// A manager that reaches `Failed` on its first failed spawn (budget 0).
    async fn failed() -> WorkerManager {
        let mut config = SupervisorConfig::production(PathBuf::from("/nonexistent/onecad-worker"));
        config.backoff = vec![Duration::from_millis(1)];
        config.max_rapid_deaths = 0;
        let wm = WorkerManager::spawn(config);
        for _ in 0..200 {
            if wm.state() == WorkerState::Failed {
                return wm;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("a nonexistent binary with a zero flap budget must reach Failed");
    }

    #[tokio::test]
    async fn adoption_takes_the_warm_worker_exactly_once() {
        let warm: WorkerSlot = Arc::new(StdMutex::new(Some(unstartable())));
        let adopted = take_warm(&warm).expect("a usable warm worker is adopted");
        assert!(!adopted.is_retired(), "adoption must not retire the worker");
        assert!(
            warm.lock().unwrap().is_none(),
            "the warm slot is consumed, so a second open cold-spawns"
        );
        assert!(take_warm(&warm).is_none(), "adoption is not repeatable");
        adopted.retire();
    }

    #[tokio::test]
    async fn a_failed_warm_worker_is_retired_instead_of_adopted() {
        let dead = failed().await;
        let warm: WorkerSlot = Arc::new(StdMutex::new(Some(dead.clone())));
        assert!(
            take_warm(&warm).is_none(),
            "a Failed warm worker must never be handed to a new document"
        );
        assert!(dead.is_retired(), "and it is retired rather than leaked");
        assert!(warm.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn an_already_retired_warm_worker_is_rejected() {
        let wm = unstartable();
        wm.retire();
        let warm: WorkerSlot = Arc::new(StdMutex::new(Some(wm)));
        assert!(take_warm(&warm).is_none());
    }

    /// Stages a backend swap exactly as [`AppState::make_backend`] does, so both
    /// resolution halves can be tested without driving a real factory.
    fn stage_swap(state: &AppState, incoming: &WorkerManager) {
        let displaced = state.snapshot_backend();
        adopt_live(&state.live, incoming);
        *state.displaced.lock().unwrap() = Some(displaced);
    }

    #[tokio::test]
    async fn committing_a_swap_retires_the_previous_documents_worker() {
        let state = AppState::new(Arc::new(pending_bundle));
        let first = unstartable();
        let second = unstartable();

        stage_swap(&state, &first);
        state.commit_backend();
        assert!(!first.is_retired(), "the first open has nothing to retire");

        stage_swap(&state, &second);
        assert!(
            !first.is_retired(),
            "STAGING must not retire — the open can still fail from here"
        );
        state.commit_backend();
        assert!(
            first.is_retired(),
            "THE leak fix: committing a new document retires the previous one's worker"
        );
        assert!(!second.is_retired(), "the incoming worker stays live");
        assert!(state.live_worker().is_some_and(|wm| wm.is_same(&second)));

        state.commit_backend(); // idempotent — nothing pending
        assert!(!second.is_retired());
        state.retire_all();
    }

    #[tokio::test]
    async fn rolling_a_swap_back_restores_the_previous_worker_and_retires_the_incoming() {
        let state = AppState::new(Arc::new(pending_bundle));
        let previous = unstartable();
        *state.live.lock().unwrap() = Some(previous.clone());
        let incoming = unstartable();

        stage_swap(&state, &incoming);
        state.rollback_backend();

        assert!(
            state.live_worker().is_some_and(|wm| wm.is_same(&previous)),
            "a failed open hands the surviving document its own worker back"
        );
        assert!(
            !previous.is_retired(),
            "and that worker must still be usable — this is the regression being pinned"
        );
        assert!(
            incoming.is_retired(),
            "the worker the failed open built is the one that goes away"
        );

        state.rollback_backend(); // idempotent — nothing pending
        assert!(!previous.is_retired());
        state.retire_all();
    }

    #[tokio::test]
    async fn rolling_back_restores_every_side_seam_too() {
        // The pre-existing half-break: `export_step_file` / `face_sketch_plane` /
        // `preview_op` / `add_sketch_on_face` / `insert_step` / `clear_worker_circuit`
        // / the readiness gate all read slots the staged swap overwrites, and a
        // surviving document was left pointing at the NEXT document's backend.
        let state = AppState::new(Arc::new(pending_bundle));
        let (exporter, elements, preview, faces, imports, circuit, readiness) = (
            state.exporter(),
            state.element_query(),
            state.preview(),
            state.face_projection(),
            state.step_import(),
            state.circuit(),
            state.readiness(),
        );

        let _incoming = state.make_backend(); // installs a fresh facet set
        state.rollback_backend();

        assert!(Arc::ptr_eq(&exporter, &state.exporter()));
        assert!(Arc::ptr_eq(&elements, &state.element_query()));
        assert!(Arc::ptr_eq(&preview, &state.preview()));
        assert!(Arc::ptr_eq(&faces, &state.face_projection()));
        assert!(Arc::ptr_eq(&imports, &state.step_import()));
        assert!(Arc::ptr_eq(&circuit, &state.circuit()));
        assert!(Arc::ptr_eq(&readiness, &state.readiness()));
    }

    #[tokio::test]
    async fn retire_slot_is_idempotent_and_clears() {
        let slot: WorkerSlot = Arc::new(StdMutex::new(None));
        retire_slot(&slot); // empty — no-op
        let wm = unstartable();
        *slot.lock().unwrap() = Some(wm.clone());
        retire_slot(&slot);
        assert!(wm.is_retired());
        assert!(slot.lock().unwrap().is_none());
        retire_slot(&slot);
    }

    #[tokio::test]
    async fn retire_all_empties_both_slots() {
        let state = AppState::new(Arc::new(pending_bundle));
        *state.warm.lock().unwrap() = Some(unstartable());
        *state.live.lock().unwrap() = Some(unstartable());
        let (warm, live) = (
            state.warm_worker().expect("warm"),
            state.live_worker().expect("live"),
        );

        state.retire_all();
        assert!(warm.is_retired() && live.is_retired());
        assert!(state.warm_worker().is_none() && state.live_worker().is_none());
        state.retire_all(); // idempotent
    }

    #[test]
    fn a_retired_worker_reports_as_failed_on_the_wire() {
        // `WorkerStatusDto.state` stays a four-value contract.
        assert_eq!(status_from_state(WorkerState::Retired, 7).state, "failed");
    }
}
