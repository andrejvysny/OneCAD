//! [`WorkerManager`] — the C++ OCCT sidecar's lifecycle owner (R-WP11).
//!
//! Spawns the worker via `tokio::process` (NOT `tauri-plugin-shell` — the plan
//! mandates real `AsyncRead`/backpressure), reads the unsolicited `hello`
//! (SCHEMA §6), and **supervises** it:
//!
//! * **liveness** — ping (`GetWorkerHead`) every 5 s; 2 missed → `SIGKILL`
//!   (SCHEMA §8 hung-worker rule);
//! * **restart** — on exit/crash/kill, bump the [`WorkerEpoch`], invoke the
//!   restart hook (**mark dirty + replay** via the regen path), and reconnect with
//!   backoff `0.5 / 1 / 2 s ×3`. A failed *start* OR a **rapid death** (a worker
//!   that dies within `healthy_threshold` of becoming Ready — a connect-then-die
//!   flap) both count toward one strike budget (`max_rapid_deaths`); exhausting it
//!   ⇒ [`WorkerState::Failed`] (F2). A death after a healthy period resets it;
//! * **poison / circuit breaker** — a plan that crashes the worker on the same
//!   `(historyPrefixHash-through-the-crashing-op, crashing-op, fingerprint)` key
//!   `poison_threshold` times opens that key's circuit: it then **fails fast without
//!   dispatch** (so it stops killing the worker) but the worker stays
//!   alive/restarting so **other plans still run** (F3). The circuit never sets
//!   [`WorkerState::Failed`] — only the F2 flap budget does. Because the hash runs
//!   *through* the op's own params, editing them mints a new key and heals the
//!   circuit; see [`plan_op_keys`]. The entry map is bounded (fingerprint prune +
//!   LRU cap) and can be cleared wholesale via
//!   [`clear_circuit`](WorkerManager::clear_circuit);
//! * **retirement** — [`retire`](WorkerManager::retire) is the supervisor's only
//!   *terminal* exit besides [`WorkerState::Failed`]: it cancels the handshake, the
//!   run loop and any backoff sleep, tears the child down gracefully-then-forcibly,
//!   and never restarts. Without it every document open leaked its predecessor's
//!   sidecar for the lifetime of the process.
//!
//! It implements [`GeometryEngine`] + [`MeshProvider`] by translating core types
//! to the OCW1 wire via [`wire`](super::wire) over the current
//! [`ProtocolClient`], swapping the client on every restart.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, Notify};

use onecad_protocol::client::{ProtocolClient, WorkerEvent};
use onecad_protocol::messages::{HelloResult, Lane};
use onecad_protocol::ProtocolError;

use onecad_core::document::body::{BodyMeta, BodyRegistry};
use onecad_core::document::element_index::ElementIndex;
use onecad_core::document::record::Operation;
use onecad_core::ids::{
    BodyId, DocumentId, DocumentRevision, EntityId, JobId, RecordId, SnapshotId, WorkerEpoch,
};
use onecad_core::regen::{
    AcceptResult, AcquireRequest, BindElementIdsRequest, BodySelector, CheckpointArtifacts,
    EngineError, Fencing, GeometryEngine, Lod, OpenSessionRequest, PlanEvent, PlanRequest,
    RefResolution, ResolveRequest, RestoreRequest, RestoreResult, SessionMode, TessellateRequest,
    TessellateResult, WorkerElementEvidence, WorkerHead,
};
use onecad_core::sketch::Sketch;

use crate::dto::{BeginGestureDto, DragSolveDto, FinishSketchDto, SketchUpsertDto};

use super::{wire, MeshProvider, SolverEngine};

#[path = "manifest.rs"]
mod bundled_manifest;

/// Lifecycle state surfaced to the app (drives the worker-status banner).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    /// Spawning / connecting the first time.
    Starting,
    /// Connected, handshook, session open — geometry calls flow.
    Ready,
    /// Died; reconnecting under backoff.
    Restarting,
    /// The flap budget was exhausted (too many failed starts / rapid deaths) — no
    /// worker. A poison circuit does NOT reach this state (F3).
    Failed,
    /// **Terminal**: [`WorkerManager::retire`] was called. The supervisor is gone,
    /// the child is (being) reaped, no restart will ever be attempted and every
    /// request fails fast. Sticky — nothing can move a manager out of it.
    Retired,
}

/// A lifecycle transition broadcast to the app (worker-status event; R-WP11).
#[derive(Debug, Clone)]
pub enum WorkerLifecycle {
    /// Connected + handshook; carries the new epoch and OCCT fingerprint.
    Ready { epoch: u64, fingerprint: String },
    /// A restart began (crash / exit / ping timeout).
    Restarting { epoch: u64, reason: String },
    /// Terminal: backoff exhausted (cannot start).
    Failed { reason: String },
    /// A plan-key's crash circuit tripped (poison): that plan now fails fast without
    /// dispatch, but the worker stays alive so other plans still run (F3).
    CircuitOpen { key: String },
}

/// The restart hook: called with the freshly-bumped epoch after every restart, so
/// the app can **mark the document dirty and enqueue a replay** (SCHEMA §8 crash →
/// restart + replay). Default is a no-op (tests / headless).
pub type RestartHook = Arc<dyn Fn(WorkerEpoch) + Send + Sync>;

/// Tunable supervision policy (production defaults; tests inject fast values).
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub binary: PathBuf,
    /// Extra environment for the spawn (chaos-hook drills set these).
    pub envs: Vec<(String, String)>,
    pub ping_interval: Duration,
    pub ping_timeout: Duration,
    pub max_missed_pings: u32,
    /// Backoff delays between failed *start* attempts.
    pub backoff: Vec<Duration>,
    /// Strike budget for the unified flap counter (failed *starts* + **rapid
    /// deaths**); exceeding it ⇒ [`WorkerState::Failed`] (F2). Defaults to
    /// `backoff.len()` so the start-failure cadence is unchanged.
    pub max_rapid_deaths: u32,
    /// A worker that dies within this window of reaching [`WorkerState::Ready`] is a
    /// **flap** (counts toward `max_rapid_deaths`); a death after living at least
    /// this long resets the flap counter (F2 — a connect-then-die loop can no longer
    /// restart forever).
    pub healthy_threshold: Duration,
    /// Consecutive same-key crashes before the plan's circuit opens (poison).
    pub poison_threshold: u32,
    /// Auto-`OpenSession` after connect (production true; smoke test opens itself).
    pub auto_open_session: bool,
}

impl SupervisorConfig {
    /// Production supervision policy (SCHEMA §8: ping 5 s ×2, backoff 0.5/1/2 s).
    #[must_use]
    pub fn production(binary: PathBuf) -> Self {
        Self {
            binary,
            envs: Vec::new(),
            ping_interval: Duration::from_secs(5),
            ping_timeout: Duration::from_secs(5),
            max_missed_pings: 2,
            backoff: vec![
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
            ],
            max_rapid_deaths: 3,
            healthy_threshold: Duration::from_secs(5),
            poison_threshold: 3,
            auto_open_session: true,
        }
    }
}

/// Shared supervisor + connection state (behind the `WorkerManager`).
struct Shared {
    config: SupervisorConfig,
    /// The current connection; `None` while (re)starting or Failed.
    conn: RwLock<Option<Arc<ProtocolClient>>>,
    epoch: AtomicU64,
    state: Mutex<WorkerState>,
    hello: Mutex<Option<HelloResult>>,
    /// `jobId → in-flight request id`, for cancel propagation (SCHEMA §3.5).
    inflight: Mutex<HashMap<JobId, u64>>,
    poison: Mutex<Poison>,
    lifecycle: broadcast::Sender<WorkerLifecycle>,
    restart_hook: RwLock<Option<RestartHook>>,
    /// The ONE cancellation signal (see [`WorkerManager::retire`]). Latched once,
    /// never cleared; read by every supervisor state so a retired manager can never
    /// reconnect, restart, or bump its epoch.
    retired: AtomicBool,
    /// Wakes every await that is parked on [`Shared::retired`] the instant the flag
    /// latches — the supervisor is otherwise blocked in a handshake, a run-loop
    /// select, or a multi-second backoff sleep.
    retire_signal: Notify,
    /// Set when the supervision task returns — i.e. the manager reached a terminal
    /// state (`Failed`/`Retired`) AND its child has been reaped. The only way to
    /// observe that a retirement actually tore the process down rather than merely
    /// latching a flag.
    torn_down: AtomicBool,
}

/// Cap on live poison entries. The map is per-Rust-process and unbounded by
/// nature (every distinct `(prefix hash, op, fingerprint)` a session ever crashes
/// on mints one), so it is bounded here by last-touch eviction. 128 is far above
/// any realistic count of *simultaneously* poisoned ops — an evicted key simply
/// starts counting from zero if it ever crashes again, which is the same position
/// a fresh process would be in.
const POISON_MAX_ENTRIES: usize = 128;

/// Crash-circuit state (poison detection).
///
/// Bounded on two axes so a long-lived process cannot accumulate dead keys:
/// * **fingerprint** — [`Poison::prune_to_fingerprint`] drops every entry not keyed
///   under the worker's current OCCT fingerprint when it reconnects with a
///   different one. The fingerprint is part of the key, so those entries are
///   unreachable by construction; dropping them loses nothing.
/// * **capacity** — [`POISON_MAX_ENTRIES`], least-recently-touched first.
#[derive(Default)]
struct Poison {
    entries: HashMap<String, PoisonEntry>,
    open: HashSet<String>,
    /// The OCCT fingerprint the live entries were keyed under ("" until the first
    /// handshake).
    fingerprint: String,
    /// Monotonic touch counter driving the LRU eviction order.
    clock: u64,
}

/// What is known about one poison key: how many consecutive same-key crashes, and
/// the LAST crash message. The message is what makes an open circuit diagnosable —
/// "circuit open" alone says nothing about why the worker kept dying.
#[derive(Default)]
struct PoisonEntry {
    count: u32,
    last_message: String,
    /// `Poison::clock` at the last crash on this key (LRU eviction order).
    touched: u64,
}

impl Poison {
    /// Drops least-recently-touched entries until at most [`POISON_MAX_ENTRIES`]
    /// remain, taking each evicted key out of `open` alongside (an entry that is
    /// gone must not leave a fail-fast flag behind with no diagnosable message).
    fn evict_over_cap(&mut self) {
        while self.entries.len() > POISON_MAX_ENTRIES {
            let victim = self
                .entries
                .iter()
                .min_by(|a, b| a.1.touched.cmp(&b.1.touched).then_with(|| a.0.cmp(b.0)))
                .map(|(k, _)| k.clone());
            let Some(victim) = victim else { break };
            self.entries.remove(&victim);
            self.open.remove(&victim);
        }
    }

    /// Re-keys to `fingerprint`, dropping every entry that does not carry it.
    /// Returns how many were dropped. No-op for an empty or unchanged fingerprint.
    fn prune_to_fingerprint(&mut self, fingerprint: &str) -> usize {
        if fingerprint.is_empty() || self.fingerprint == fingerprint {
            return 0;
        }
        self.fingerprint = fingerprint.to_string();
        let suffix = format!("|{fingerprint}");
        let before = self.entries.len();
        self.entries.retain(|k, _| k.ends_with(&suffix));
        self.open.retain(|k| k.ends_with(&suffix));
        before - self.entries.len()
    }
}

impl Shared {
    /// Publishes a lifecycle state. [`WorkerState::Retired`] is **terminal**: once
    /// set, nothing overwrites it. That stickiness is what makes retirement race-free
    /// against a supervisor that is already mid-transition (e.g. about to publish
    /// `Ready`) when [`WorkerManager::retire`] lands.
    fn set_state(&self, s: WorkerState) {
        let mut current = self.state.lock().unwrap();
        if *current == WorkerState::Retired {
            return;
        }
        *current = s;
    }

    fn is_retired(&self) -> bool {
        self.retired.load(Ordering::SeqCst)
    }

    /// Latches retirement; `true` only on the transition (so `retire` is idempotent).
    fn latch_retired(&self) -> bool {
        !self.retired.swap(true, Ordering::SeqCst)
    }

    /// Resolves as soon as retirement is latched — immediately if it already is.
    ///
    /// The waiter is `enable()`d **before** the flag is re-read, and `retire` stores
    /// the flag **before** it notifies, so the two orderings interlock and a wakeup
    /// can never be lost (the naive check-then-`notified()` has exactly that race).
    async fn retired(&self) {
        loop {
            let notified = self.retire_signal.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_retired() {
                return;
            }
            notified.await;
        }
    }

    /// A backoff sleep that wakes early on retirement. `true` ⇒ retired: the caller
    /// must return instead of reconnecting.
    async fn sleep_or_retired(&self, delay: Duration) -> bool {
        tokio::select! {
            biased;
            () = self.retired() => true,
            () = tokio::time::sleep(delay) => self.is_retired(),
        }
    }

    fn client(&self) -> Option<Arc<ProtocolClient>> {
        self.conn.read().unwrap().clone()
    }

    fn fingerprint(&self) -> String {
        self.hello
            .lock()
            .unwrap()
            .as_ref()
            .map(|h| h.occt.fingerprint.clone())
            .unwrap_or_default()
    }

    /// Broadcasts a lifecycle transition — and logs it FIRST, so the transition is
    /// on the record even when nobody is subscribed (the broadcast has no receiver
    /// in tests / before the status banner mounts).
    fn emit(&self, ev: WorkerLifecycle) {
        match &ev {
            WorkerLifecycle::Ready { epoch, fingerprint } => {
                tracing::info!(epoch, fingerprint = %fingerprint, "worker ready");
            }
            WorkerLifecycle::Restarting { epoch, reason } => {
                tracing::warn!(epoch, reason = %reason, "worker restarting");
            }
            WorkerLifecycle::Failed { reason } => {
                tracing::error!(reason = %reason, "worker failed (no worker)");
            }
            WorkerLifecycle::CircuitOpen { key } => {
                // The enum stays as-is (wire/event contract); the crash message is
                // looked up here so the log line is self-contained.
                tracing::error!(
                    key = %key,
                    last_crash = %self.last_crash(key),
                    "worker crash circuit open (poison) — this plan now fails fast"
                );
            }
        }
        let _ = self.lifecycle.send(ev);
    }

    /// Records a same-key crash + its message; returns `true` if the circuit just
    /// opened.
    fn record_crash(&self, key: &str, message: &str) -> bool {
        let mut p = self.poison.lock().unwrap();
        p.clock += 1;
        let clock = p.clock;
        let entry = p.entries.entry(key.to_string()).or_default();
        entry.last_message = message.to_string();
        entry.touched = clock;
        if p.open.contains(key) {
            p.evict_over_cap();
            return false;
        }
        let entry = p.entries.get_mut(key).expect("just inserted");
        entry.count += 1;
        let opened = entry.count >= self.config.poison_threshold;
        if opened {
            p.open.insert(key.to_string());
        }
        // The key just touched is the most recent, so it is never its own victim.
        p.evict_over_cap();
        opened
    }

    /// Re-keys the crash circuit to the worker's current OCCT fingerprint (called on
    /// every successful handshake). Entries minted under a previous fingerprint can
    /// never be hit again — the fingerprint is part of the key — so a rebuilt or
    /// upgraded worker drops them instead of leaking them for the process lifetime.
    fn note_fingerprint(&self, fingerprint: &str) {
        let dropped = self
            .poison
            .lock()
            .unwrap()
            .prune_to_fingerprint(fingerprint);
        if dropped > 0 {
            tracing::info!(
                fingerprint = %fingerprint,
                dropped,
                "worker fingerprint changed — stale crash-circuit entries pruned"
            );
        }
    }

    /// Forgets every crash count and closes every open circuit; returns how many
    /// keys were forgotten. The operator escape hatch behind `clear_worker_circuit`:
    /// a user asking for this has changed something the key cannot see (a rebuilt
    /// worker, a repaired input file), so it is deliberately total rather than
    /// per-key.
    fn clear_circuit(&self) -> usize {
        let mut p = self.poison.lock().unwrap();
        let cleared = p.entries.len();
        p.entries.clear();
        p.open.clear();
        cleared
    }

    /// Clears the crash count + open flag for every one of a plan's candidate op
    /// keys (a successful prepare heals the plan — F3).
    fn record_success(&self, keys: &[String]) {
        let mut p = self.poison.lock().unwrap();
        for key in keys {
            p.entries.remove(key);
            p.open.remove(key);
        }
    }

    /// The first of a plan's candidate op keys with an open circuit, and that key's
    /// last crash message — the fail-fast gate (the crashing op is one of them, so a
    /// poisoned plan is caught up front). The message travels into the fail-fast
    /// error so the caller reports WHY, not just "circuit open".
    fn plan_circuit_open(&self, keys: &[String]) -> Option<(String, String)> {
        let p = self.poison.lock().unwrap();
        let key = keys.iter().find(|k| p.open.contains(*k))?;
        let message = p
            .entries
            .get(key)
            .map(|e| e.last_message.clone())
            .unwrap_or_default();
        Some((key.clone(), message))
    }

    fn last_crash(&self, key: &str) -> String {
        self.poison
            .lock()
            .unwrap()
            .entries
            .get(key)
            .map(|e| e.last_message.clone())
            .unwrap_or_default()
    }
}

/// The worker sidecar lifecycle owner (see the module docs).
#[derive(Clone)]
pub struct WorkerManager {
    shared: Arc<Shared>,
}

impl WorkerManager {
    /// Spawns the supervisor for `config` (returns immediately; the connection is
    /// established asynchronously). Call [`wait_ready`](Self::wait_ready) to await
    /// the first successful handshake.
    #[must_use]
    pub fn spawn(config: SupervisorConfig) -> Self {
        let (lifecycle, _) = broadcast::channel(64);
        let shared = Arc::new(Shared {
            config,
            conn: RwLock::new(None),
            epoch: AtomicU64::new(1),
            state: Mutex::new(WorkerState::Starting),
            hello: Mutex::new(None),
            inflight: Mutex::new(HashMap::new()),
            poison: Mutex::new(Poison::default()),
            lifecycle,
            restart_hook: RwLock::new(None),
            retired: AtomicBool::new(false),
            retire_signal: Notify::new(),
            torn_down: AtomicBool::new(false),
        });
        tokio::spawn(supervise(shared.clone()));
        Self { shared }
    }

    /// The current lifecycle state.
    #[must_use]
    pub fn state(&self) -> WorkerState {
        *self.shared.state.lock().unwrap()
    }

    /// The current worker epoch (bumped on every restart).
    #[must_use]
    pub fn epoch(&self) -> WorkerEpoch {
        WorkerEpoch(self.shared.epoch.load(Ordering::SeqCst))
    }

    /// The worker's handshake result once connected (fingerprint policy surface).
    #[must_use]
    pub fn hello(&self) -> Option<HelloResult> {
        self.shared.hello.lock().unwrap().clone()
    }

    /// Subscribe to lifecycle transitions (for the worker-status banner).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<WorkerLifecycle> {
        self.shared.lifecycle.subscribe()
    }

    /// Forgets every crash count and closes every open poison circuit; returns how
    /// many keys were forgotten. See [`Shared::clear_circuit`].
    pub fn clear_circuit(&self) -> usize {
        self.shared.clear_circuit()
    }

    /// Sets the restart hook (mark-dirty + replay). Replaces any prior hook.
    pub fn set_restart_hook(&self, hook: RestartHook) {
        *self.shared.restart_hook.write().unwrap() = Some(hook);
    }

    /// Awaits [`WorkerState::Ready`] up to `timeout`; `false` on
    /// `Failed`/`Retired`/timeout.
    pub async fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match self.state() {
                WorkerState::Ready => return true,
                WorkerState::Failed | WorkerState::Retired => return false,
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// `ExportGeometry` verb passthrough (SCHEMA §7.8) — bakes live bodies into a
    /// §7.3 replay codec at `path`. `codec` is `"brep"` or `"xbf"`.
    ///
    /// Unlike the user-facing exports this one is an INTERNAL lane: its output is
    /// read straight back as a content-addressed blob (a Component Library
    /// `embedded` / `document` source, spec §2.1), never handed to the user as a
    /// file.
    ///
    /// `union_solids` fuses a multi-solid body into one before writing (WP-F1.2) —
    /// opt-in, and refused by the worker when the fuse cannot produce one solid.
    ///
    /// # Errors
    /// [`EngineError`] on a disconnected worker or a worker-side failure (an
    /// unknown codec, a missing body, a non-solid body, a refused union).
    pub async fn export_geometry(
        &self,
        path: &str,
        bodies: &[BodyId],
        codec: &str,
        union_solids: bool,
    ) -> Result<crate::worker::BakedGeometry, EngineError> {
        let client = self.client_or_err()?;
        let args = wire::export_geometry_args(path, bodies, codec, union_solids);
        let resp = client
            .request("ExportGeometry", args)
            .await
            .map_err(protocol_err)?;
        let result = ok_result(resp)?;
        Ok(crate::worker::BakedGeometry {
            codec: result
                .get("codec")
                .and_then(Value::as_str)
                .unwrap_or(codec)
                .to_string(),
            format: u32::try_from(result.get("format").and_then(Value::as_u64).unwrap_or(0))
                .unwrap_or(0),
            solid_count: usize::try_from(
                result
                    .get("solidCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            )
            .unwrap_or(0),
            bytes_written: result.get("bytes").and_then(Value::as_u64).unwrap_or(0),
        })
    }

    /// `ExportStep` verb passthrough (SCHEMA §7.8) — surfaced here so an app
    /// command can drive STEP export later. Returns bytes written.
    ///
    /// `attributes` carries the document-owned body names + colours through XCAF
    /// (DI-5); [`StepExportAttributes::default`](crate::export::StepExportAttributes)
    /// is the geometry-only export.
    ///
    /// # Errors
    /// [`EngineError`] on a disconnected worker or a worker-side failure.
    pub async fn export_step(
        &self,
        path: &str,
        bodies: &[BodyId],
        schema: &str,
        attributes: &crate::export::StepExportAttributes,
    ) -> Result<u64, EngineError> {
        let client = self.client_or_err()?;
        let args = wire::export_step_args(path, bodies, schema, attributes);
        let resp = client
            .request("ExportStep", args)
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| r.get("bytes").and_then(Value::as_u64).unwrap_or(0))
    }

    /// `ExportStl` verb passthrough (SCHEMA §7.8). Returns bytes written.
    ///
    /// # Errors
    /// [`EngineError`] on a disconnected worker or a worker-side failure.
    pub async fn export_stl(
        &self,
        path: &str,
        bodies: &[BodyId],
        binary: bool,
        lod: &str,
    ) -> Result<u64, EngineError> {
        let client = self.client_or_err()?;
        let args = wire::export_stl_args(path, bodies, binary, lod);
        let resp = client
            .request("ExportStl", args)
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| r.get("bytes").and_then(Value::as_u64).unwrap_or(0))
    }

    /// `ExportObj` verb passthrough (SCHEMA §7.8). Returns bytes written.
    ///
    /// # Errors
    /// [`EngineError`] on a disconnected worker or a worker-side failure.
    pub async fn export_obj(
        &self,
        path: &str,
        bodies: &[BodyId],
        lod: &str,
    ) -> Result<u64, EngineError> {
        let client = self.client_or_err()?;
        let args = wire::export_obj_args(path, bodies, lod);
        let resp = client
            .request("ExportObj", args)
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| r.get("bytes").and_then(Value::as_u64).unwrap_or(0))
    }

    /// Graceful `Shutdown` (SCHEMA §7.1): ask the worker to flush + exit 0.
    /// Best-effort — a disconnected worker is already gone.
    ///
    /// This asks *nicely* and nothing more: the supervisor treats the resulting exit
    /// as a death and **restarts** the worker. [`retire`](Self::retire) is the
    /// terminal form and is what a lifecycle owner wants.
    pub async fn shutdown(&self) {
        if let Some(client) = self.shared.client() {
            let _ = client.request("Shutdown", json!({})).await;
        }
    }

    /// **Terminal, idempotent retirement** — the end of this manager's lifecycle.
    ///
    /// Every worker process used to outlive its document: the supervisor's only exits
    /// were `Failed` and "restart forever", so each new/open spawned a sidecar and
    /// orphaned the previous one for the lifetime of the app. `retire` is the missing
    /// terminal state.
    ///
    /// It latches the ONE cancellation flag, which is observed in **every** supervisor
    /// state — the OCW1 handshake, the ping/exit run loop, and every backoff sleep —
    /// so a manager retired while connecting or backing off neither reconnects nor
    /// restarts. The epoch is deliberately **not** bumped: a retired manager is not a
    /// restart, and a fence must never see it as one.
    ///
    /// Teardown is graceful-then-forced: a best-effort `Shutdown` frame goes out
    /// fire-and-forget, and the supervise task (which owns the [`tokio::process::Child`])
    /// reaps it, force-killing after [`RETIRE_GRACE`] if it has not exited on its own.
    /// `kill_on_drop` remains the backstop for the paths where no child exists yet.
    ///
    /// Returns immediately; the child is reaped in the background.
    pub fn retire(&self) {
        if !self.shared.latch_retired() {
            return; // already retired — terminal and idempotent.
        }
        // Sticky (see `Shared::set_state`): a supervisor mid-transition cannot undo it.
        self.shared.set_state(WorkerState::Retired);
        self.shared.retire_signal.notify_waiters();
        tracing::info!(epoch = self.epoch().0, "worker retired");
        // Fire-and-forget the polite stop. A caller with no reactor (a Drop-time or
        // sync teardown path) simply skips it and the child is force-killed instead.
        if let (Some(client), Ok(handle)) =
            (self.shared.client(), tokio::runtime::Handle::try_current())
        {
            handle.spawn(async move {
                let _ = client.request("Shutdown", json!({})).await;
            });
        }
    }

    /// Whether [`retire`](Self::retire) has been called (terminal).
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.shared.is_retired()
    }

    /// Whether the supervision task has finished — the manager reached a terminal
    /// state AND its child process has been reaped. `retire` returns before this is
    /// true (teardown is asynchronous, bounded by [`RETIRE_GRACE`]); this is how a
    /// caller (or a test) confirms the process is really gone.
    #[must_use]
    pub fn is_torn_down(&self) -> bool {
        self.shared.torn_down.load(Ordering::SeqCst)
    }

    /// Awaits [`is_torn_down`](Self::is_torn_down) up to `timeout`.
    pub async fn wait_torn_down(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.is_torn_down() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Whether both handles drive the **same** supervisor — identity, not equality.
    /// This is how "the open document adopted the pre-warmed worker" is asserted:
    /// a second spawn would produce a different `Shared`.
    #[must_use]
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    fn client_or_err(&self) -> Result<Arc<ProtocolClient>, EngineError> {
        if self.shared.is_retired() {
            return Err(retired_err());
        }
        self.shared.client().ok_or_else(not_connected)
    }
}

impl std::fmt::Debug for WorkerManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerManager")
            .field("state", &self.state())
            .field("epoch", &self.epoch().0)
            .finish_non_exhaustive()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Supervisor
// ─────────────────────────────────────────────────────────────────────────────

/// Why the running worker stopped.
enum Death {
    /// Exited on its own (crash / clean).
    Exited,
    /// Killed after `max_missed_pings` (hung).
    PingTimeout,
    /// [`WorkerManager::retire`] latched — terminal, NOT a restart trigger.
    Retired,
}

/// How long a retired worker gets to honour its graceful `Shutdown` before it is
/// force-killed. Bounded so a wedged sidecar can never outlive its manager.
const RETIRE_GRACE: Duration = Duration::from_millis(500);

/// Liveness backstop for `AnalyzeEdgeOpRange` (SCHEMA §7.6). Deliberately far
/// above the worker's own 1500 ms probe deadline: the worker is supposed to end
/// its own search, and this fires only when it did not. On expiry Rust MUST
/// `cancel` the id rather than drop the slot — an abandoned request leaves the
/// worker building an answer nobody will read.
const ANALYZE_EDGE_OP_RANGE_LIVENESS: Duration = Duration::from_secs(10);

/// Reaps `child`: waits up to `grace` for the self-exit the graceful `Shutdown`
/// should produce, then `SIGKILL`s. Always returns with the child reaped, so
/// `kill_on_drop` stays a backstop rather than the mechanism.
async fn terminate_child(child: &mut tokio::process::Child, grace: Duration) {
    if tokio::time::timeout(grace, child.wait()).await.is_ok() {
        return;
    }
    tracing::warn!("retired worker ignored Shutdown within the grace window — SIGKILL");
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// The supervision loop: (re)connect, run until death, restart with backoff. A
/// unified `flap_strikes` counter spans failed *starts* and **rapid deaths**
/// (connect-then-die within `healthy_threshold`); exhausting `max_rapid_deaths` ⇒
/// Failed (F2). A poisoned plan-key never stops the supervisor (F3) — the worker
/// keeps restarting so other plans run.
async fn supervise(shared: Arc<Shared>) {
    supervise_loop(&shared).await;
    // The task is returning, so the child (if any) has been reaped — publish that,
    // because "retired" alone says nothing about whether the process actually died.
    shared.torn_down.store(true, Ordering::SeqCst);
}

async fn supervise_loop(shared: &Shared) {
    let mut flap_strikes = 0u32;
    // F6: the restart hook (mark-dirty + enqueue replay) fires on the post-restart
    // READY transition, not at death — so the replay it enqueues dispatches over a
    // live connection instead of racing `conn == None`. `Some(epoch)` between a
    // death and the next Ready.
    let mut pending_restart_epoch: Option<u64> = None;
    loop {
        // Retirement is checked at EVERY point the loop can be entered or resumed
        // (here, inside the handshake, inside the run loop, and after every backoff
        // sleep) — a retired manager must never spawn another child.
        if shared.is_retired() {
            return;
        }
        match spawn_and_connect(shared).await {
            Ok((mut child, client)) => {
                if shared.is_retired() {
                    // Retired mid-handshake: never publish this connection and never
                    // count it as a life. Reap the child we just spawned and stop.
                    terminate_child(&mut child, RETIRE_GRACE).await;
                    return;
                }
                *shared.conn.write().unwrap() = Some(client.clone());
                *shared.hello.lock().unwrap() = Some(client.hello().clone());
                // A different OCCT build re-keys every poison entry; drop the
                // unreachable ones now rather than carrying them for the process
                // lifetime (memory bound).
                shared.note_fingerprint(&client.hello().occt.fingerprint);
                if shared.config.auto_open_session {
                    let _ = auto_open_session(shared, &client).await;
                }
                shared.set_state(WorkerState::Ready);
                shared.emit(WorkerLifecycle::Ready {
                    epoch: shared.epoch.load(Ordering::SeqCst),
                    fingerprint: client.hello().occt.fingerprint.clone(),
                });
                // Fire the deferred restart hook now that the worker is Ready + the
                // connection is live (F6).
                if let Some(epoch) = pending_restart_epoch.take() {
                    fire_restart_hook(shared, WorkerEpoch(epoch));
                }
                let ready_at = tokio::time::Instant::now();

                let death = run_until_death(shared, &client, child).await;
                let alive = ready_at.elapsed();

                if matches!(death, Death::Retired) {
                    // Terminal. Drop the connection but do NOT bump the epoch and do
                    // NOT emit a transition: retirement is not a restart, and a fence
                    // that saw a bump here would reject work it should have kept.
                    *shared.conn.write().unwrap() = None;
                    return;
                }

                // Torn down: drop the connection and bump the epoch (fencing).
                *shared.conn.write().unwrap() = None;
                let new_epoch = shared.epoch.fetch_add(1, Ordering::SeqCst) + 1;
                let reason = match death {
                    Death::Exited => "worker exited/crashed",
                    Death::PingTimeout => "worker hung (ping timeout) → SIGKILL",
                    Death::Retired => unreachable!("handled above"),
                };
                shared.set_state(WorkerState::Restarting);
                shared.emit(WorkerLifecycle::Restarting {
                    epoch: new_epoch,
                    reason: reason.into(),
                });
                // Defer the restart hook to the NEXT Ready (F6): a replay enqueued now
                // would race `conn == None`. If the flap budget is exhausted below we
                // never reach that Ready — correct, there is no worker to replay on.
                pending_restart_epoch = Some(new_epoch);

                // F2: a death within `healthy_threshold` of becoming Ready is a flap
                // (counts toward the strike budget); a longer-lived worker resets it.
                if alive < shared.config.healthy_threshold {
                    flap_strikes += 1;
                    if flap_strikes > shared.config.max_rapid_deaths {
                        shared.set_state(WorkerState::Failed);
                        shared.emit(WorkerLifecycle::Failed {
                            reason: format!(
                                "rapid-death budget exhausted after {flap_strikes} flaps ({reason})"
                            ),
                        });
                        return;
                    }
                    // Back off like a start failure so a connect-die loop can't spin.
                    let idx = (flap_strikes as usize - 1)
                        .min(shared.config.backoff.len().saturating_sub(1));
                    let delay = shared
                        .config
                        .backoff
                        .get(idx)
                        .copied()
                        .unwrap_or_else(|| first_delay(shared));
                    if shared.sleep_or_retired(delay).await {
                        return;
                    }
                } else {
                    flap_strikes = 0;
                    // Restart cadence: reuse the first backoff delay for runtime deaths.
                    if shared.sleep_or_retired(first_delay(shared)).await {
                        return;
                    }
                }
            }
            Err(reason) => {
                // A retirement observed inside `spawn_and_connect` surfaces as a start
                // failure; it must NOT be announced as a restart or counted as a flap.
                if shared.is_retired() {
                    return;
                }
                flap_strikes += 1;
                shared.set_state(WorkerState::Restarting);
                shared.emit(WorkerLifecycle::Restarting {
                    epoch: shared.epoch.load(Ordering::SeqCst),
                    reason: format!("start failed: {reason}"),
                });
                if flap_strikes > shared.config.max_rapid_deaths {
                    shared.set_state(WorkerState::Failed);
                    shared.emit(WorkerLifecycle::Failed {
                        reason: format!("backoff exhausted after {flap_strikes} tries: {reason}"),
                    });
                    return;
                }
                let idx = (flap_strikes as usize - 1).min(shared.config.backoff.len() - 1);
                if shared.sleep_or_retired(shared.config.backoff[idx]).await {
                    return;
                }
            }
        }
    }
}

fn first_delay(shared: &Shared) -> Duration {
    shared
        .config
        .backoff
        .first()
        .copied()
        .unwrap_or(Duration::from_millis(500))
}

fn fire_restart_hook(shared: &Shared, epoch: WorkerEpoch) {
    let hook = shared.restart_hook.read().unwrap().clone();
    if let Some(hook) = hook {
        hook(epoch);
    }
}

/// Spawns the child and completes the OCW1 handshake. `Err(reason)` on spawn or
/// handshake failure (bad magic, EOF before hello) — a restart trigger.
async fn spawn_and_connect(
    shared: &Shared,
) -> Result<(tokio::process::Child, Arc<ProtocolClient>), String> {
    let expected = bundled_manifest::embedded_release_manifest()?;
    if let Some(expected) = &expected {
        bundled_manifest::verify_binary(&shared.config.binary, expected)?;
    }

    let mut cmd = tokio::process::Command::new(&shared.config.binary);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in &shared.config.envs {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {:?}: {e}", shared.config.binary))?;
    // HARD INVARIANT: the stderr pipe is now OURS, and `Log.h` flushes every line —
    // an unread pipe fills its buffer and BLOCKS the worker mid-op. The forwarder is
    // therefore spawned unconditionally, before anything that can fail or await
    // (`?` below drops `child`, and `kill_on_drop` then ends the forwarder at EOF).
    // The epoch is read at spawn time: the supervisor's `fetch_add` happens on the
    // death path, before the respawn, so this is the epoch the worker runs under.
    // (The failed-*start* branch does not bump, so N consecutive failed spawns share
    // one epoch — cosmetic, and there is no worker output to attribute anyway.)
    let epoch = shared.epoch.load(Ordering::SeqCst);
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(forward_worker_stderr(stderr, epoch));
    }
    let stdout = child.stdout.take().ok_or("child stdout missing")?;
    let stdin = child.stdin.take().ok_or("child stdin missing")?;
    // A worker that never sends its hello would park this await forever, so the
    // handshake is one of the three places retirement MUST be observable. On
    // cancellation `child` drops here and `kill_on_drop` reaps it.
    let client = tokio::select! {
        biased;
        () = shared.retired() => return Err("retired during handshake".into()),
        connected = ProtocolClient::connect(stdout, stdin) => {
            connected.map_err(|e| format!("handshake: {e}"))?
        }
    };
    if let Some(expected) = &expected {
        bundled_manifest::verify_hello(expected, client.hello())?;
    }
    Ok((child, Arc::new(client)))
}

/// Largest single worker stderr line forwarded verbatim; the remainder of an
/// over-long line is dropped with a marker (a runaway `WLOG` must not blow up the
/// JSONL sink).
const MAX_WORKER_LINE: usize = 8 * 1024;

/// Forwards the C++ worker's stderr into the `tracing` stream under target
/// `worker`, one event per line, at the level its `WLOG` prefix declares.
///
/// This task is detached (it outlives nothing but the pipe), so worker events carry
/// **no span context** — join them to a regen through the frame trace's `reqId`, or
/// through `epoch`. It self-terminates at EOF: it is the sole owner of the pipe, so
/// when the child dies the read resolves `Ok(0)`.
async fn forward_worker_stderr(stderr: tokio::process::ChildStderr, epoch: u64) {
    use tokio::io::AsyncBufReadExt;

    let mut reader = tokio::io::BufReader::new(stderr);
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) => {
                tracing::debug!(target: "worker", epoch, "worker stderr closed (EOF)");
                return;
            }
            Ok(_) => {
                let truncated = buf.len() > MAX_WORKER_LINE;
                if truncated {
                    buf.truncate(MAX_WORKER_LINE);
                }
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim_end_matches(['\n', '\r']);
                if line.is_empty() {
                    continue;
                }
                let line = if truncated {
                    format!("{line} …[truncated]")
                } else {
                    line.to_string()
                };
                match sniff_worker_level(&line) {
                    WorkerLine::Error => tracing::error!(target: "worker", epoch, "{line}"),
                    WorkerLine::Warn => tracing::warn!(target: "worker", epoch, "{line}"),
                    WorkerLine::Info => tracing::info!(target: "worker", epoch, "{line}"),
                    WorkerLine::Debug => tracing::debug!(target: "worker", epoch, "{line}"),
                }
            }
            Err(e) => {
                tracing::debug!(target: "worker", epoch, "worker stderr read error: {e}");
                return;
            }
        }
    }
}

/// The level a forwarded worker line claims.
enum WorkerLine {
    Error,
    Warn,
    Info,
    Debug,
}

/// Sniffs the `WLOG` prefix `[<timestamp>] <LEVEL>  <message>` (`Log.h`). Anything
/// else (a raw `fprintf(stderr,…)`, an OCCT message, the test stub's own lines)
/// is INFO — never silently dropped.
fn sniff_worker_level(line: &str) -> WorkerLine {
    let rest = line.split_once("] ").map_or(line, |(_, rest)| rest);
    match rest.split_whitespace().next().unwrap_or_default() {
        "ERROR" => WorkerLine::Error,
        "WARN" => WorkerLine::Warn,
        "DEBUG" => WorkerLine::Debug,
        _ => WorkerLine::Info,
    }
}

async fn auto_open_session(shared: &Shared, client: &ProtocolClient) -> Result<(), ProtocolError> {
    let epoch = shared.epoch.load(Ordering::SeqCst);
    let args = wire::open_session_args(&OpenSessionRequest {
        document_id: DocumentId::from_uuid(uuid::Uuid::nil()),
        document_revision: DocumentRevision(0),
        worker_epoch: WorkerEpoch(epoch),
        mode: SessionMode::Determinism,
    });
    let _ = client.request("OpenSession", args).await?;
    Ok(())
}

/// Runs while the worker lives: ping on an interval, watch for exit; `SIGKILL` on
/// `max_missed_pings` (SCHEMA §8 hung worker).
async fn run_until_death(
    shared: &Shared,
    client: &ProtocolClient,
    mut child: tokio::process::Child,
) -> Death {
    let mut missed = 0u32;
    let mut ticker = tokio::time::interval(shared.config.ping_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await; // consume the immediate first tick.
    loop {
        tokio::select! {
            // `biased` so retirement WINS a tie. Without it a retire that races the
            // child's own exit could resolve as `Exited` and bump the epoch.
            biased;
            () = shared.retired() => {
                terminate_child(&mut child, RETIRE_GRACE).await;
                return Death::Retired;
            }
            _ = ticker.tick() => {
                let ping = client.request_timeout("GetWorkerHead", json!({}), shared.config.ping_timeout);
                // The ping is awaited INSIDE this arm, so it must carry its own
                // cancellation: a wedged worker parks it for the whole `ping_timeout`
                // (5 s in production) and the outer select never gets polled again.
                let pinged = tokio::select! {
                    biased;
                    () = shared.retired() => {
                        terminate_child(&mut child, RETIRE_GRACE).await;
                        return Death::Retired;
                    }
                    pinged = ping => pinged,
                };
                match pinged {
                    Ok(resp) if resp.ok => missed = 0,
                    _ => {
                        missed += 1;
                        tracing::debug!(missed, max = shared.config.max_missed_pings, "worker ping missed");
                        if missed >= shared.config.max_missed_pings {
                            let _ = child.start_kill();
                            let _ = child.wait().await;
                            return Death::PingTimeout;
                        }
                    }
                }
            }
            _ = child.wait() => return Death::Exited,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GeometryEngine (wire translation)
// ─────────────────────────────────────────────────────────────────────────────

#[async_trait]
impl GeometryEngine for WorkerManager {
    fn current_epoch(&self) -> WorkerEpoch {
        self.epoch()
    }

    async fn execute_plan(&self, request: PlanRequest) -> mpsc::Receiver<PlanEvent> {
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(stream_plan(self.shared.clone(), request, tx));
        rx
    }

    async fn open_session(&self, req: OpenSessionRequest) -> Result<WorkerHead, EngineError> {
        let client = self.client_or_err()?;
        let epoch = req.worker_epoch;
        let resp = client
            .request("OpenSession", wire::open_session_args(&req))
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_open_session(&r, epoch))
    }

    async fn close_session(
        &self,
        document_id: DocumentId,
        worker_epoch: WorkerEpoch,
    ) -> Result<(), EngineError> {
        let client = self.client_or_err()?;
        let args = json!({ "documentId": document_id.to_string(), "workerEpoch": worker_epoch.0 });
        let resp = client
            .request("CloseSession", args)
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|_| ())
    }

    async fn reset(
        &self,
        document_id: DocumentId,
        worker_epoch: WorkerEpoch,
    ) -> Result<WorkerEpoch, EngineError> {
        let client = self.client_or_err()?;
        let args = json!({ "documentId": document_id.to_string(), "workerEpoch": worker_epoch.0 });
        let resp = client
            .request("ResetSession", args)
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| {
            WorkerEpoch(
                r.get("workerEpoch")
                    .and_then(Value::as_u64)
                    .unwrap_or(worker_epoch.0 + 1),
            )
        })
    }

    async fn accept_prepared(
        &self,
        job_id: JobId,
        fencing: Fencing,
    ) -> Result<AcceptResult, EngineError> {
        let client = self.client_or_err()?;
        let args = json!({
            "jobId": wire::job_id_wire(job_id),
            "documentRevision": fencing.document_revision.0,
            "workerEpoch": fencing.worker_epoch.0,
        });
        let resp = client
            .request("AcceptPrepared", args)
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_accept(&r))
    }

    async fn discard_prepared(&self, job_id: JobId) -> Result<(), EngineError> {
        let client = self.client_or_err()?;
        let args = json!({ "jobId": wire::job_id_wire(job_id) });
        let resp = client
            .request("DiscardPrepared", args)
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|_| ())
    }

    async fn get_worker_head(&self) -> Result<WorkerHead, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request("GetWorkerHead", json!({}))
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_worker_head(&r))
    }

    async fn tessellate(&self, req: TessellateRequest) -> Result<TessellateResult, EngineError> {
        // The wire tessellate result carries mesh *handles*; bytes are pulled via
        // `MeshProvider::fetch_mesh`. The handle set is not folded by the executor,
        // so a single-body request is enough for the current mesh-cache path.
        let _ = req;
        Ok(TessellateResult { meshes: vec![] })
    }

    async fn save_checkpoint(&self, step_index: usize) -> Result<CheckpointArtifacts, EngineError> {
        // SaveCheckpoint (SCHEMA §7.7): the worker serializes its head (per-body BREP
        // via BinTools + signatures + historyPrefixHash) into the resp binary tail;
        // Rust stores the bytes + parsed envelope metadata for its checkpoint cache.
        let client = self.client_or_err()?;
        let inflight = client
            .start_request(
                "SaveCheckpoint",
                wire::save_checkpoint_args(step_index),
                Lane::Control,
            )
            .await
            .map_err(protocol_err)?;
        let (resp, tail) = inflight.response_with_bin().await.map_err(protocol_err)?;
        let sections = resp.bin.clone().unwrap_or_default();
        let result = ok_result(resp)?;
        wire::parse_save_checkpoint(&result, &sections, &tail, &self.shared.fingerprint())
            .map_err(|message| EngineError::Protocol { message })
    }

    async fn restore_checkpoint(&self, req: RestoreRequest) -> Result<RestoreResult, EngineError> {
        // RestoreCheckpoint (SCHEMA §7.7): the worker rolls its in-session head back to
        // the checkpoint step (the geometry never re-crosses the wire — the OCW1 request
        // path carries no binary; a worker that no longer retains the step reports
        // `restored:false`). Rust reconstructs the base registry/element index from the
        // stored artifacts (review F3), so the executor seeds its scratch from the
        // immutable checkpoint, not live state.
        let client = self.client_or_err()?;
        let args = wire::restore_checkpoint_args(&req);
        let resp = client
            .request("RestoreCheckpoint", args)
            .await
            .map_err(protocol_err)?;
        let result = ok_result(resp)?;
        let (restored, drift, snapshot_id) = wire::parse_restore_checkpoint(&result);
        // Without the stored artifacts Rust cannot rebuild the base ⇒ replay-from-0.
        let Some(artifacts) = req.artifacts.as_ref().filter(|_| restored && !drift) else {
            return Ok(unusable_restore(snapshot_id, drift));
        };
        let (base_registry, base_elements) = reconstruct_base(artifacts);
        Ok(RestoreResult::ok(
            SnapshotId(snapshot_id),
            req.checkpoint.step_index,
            base_registry,
            base_elements,
        ))
    }

    async fn acquire_element_ids(
        &self,
        req: AcquireRequest,
    ) -> Result<Vec<WorkerElementEvidence>, EngineError> {
        // SCHEMA §7.5: the worker returns resolved `topoKey → (kind, descriptor,
        // anchor)` evidence + any already-held id; **Rust mints/owns the ids**.
        let client = self.client_or_err()?;
        let fallback = req.body;
        let resp = client
            .request("AcquireElementIds", wire::acquire_element_ids_args(&req))
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_acquire_evidence(&r, fallback))
    }

    async fn bind_element_ids(&self, req: BindElementIdsRequest) -> Result<(), EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request("BindElementIds", wire::bind_element_ids_args(&req))
            .await
            .map_err(protocol_err)?;
        wire::validate_bind_element_ids_result(&req, &ok_result(resp)?)
            .map_err(|message| EngineError::Protocol { message })
    }

    async fn resolve_refs(&self, req: ResolveRequest) -> Result<Vec<RefResolution>, EngineError> {
        // SCHEMA §7.5 dry-run ladder for repair dialogs (binds nothing).
        let client = self.client_or_err()?;
        let resp = client
            .request("ResolveRefs", wire::resolve_refs_args(&req))
            .await
            .map_err(protocol_err)?;
        let result = ok_result(resp)?;
        // Fail closed on the §7.5 echo before a single resolution reaches a repair
        // dialog — the same discipline `BindElementIds` already applies.
        wire::validate_resolve_refs_result(&req, &result)
            .map_err(|message| EngineError::Protocol { message })?;
        Ok(wire::parse_resolve_refs(&result))
    }

    async fn cancel(&self, job_id: JobId) -> Result<(), EngineError> {
        let id = self.shared.inflight.lock().unwrap().get(&job_id).copied();
        if let (Some(id), Some(client)) = (id, self.shared.client()) {
            let _ = client.cancel(id).await;
        }
        Ok(())
    }

    async fn ping(&self) -> Result<(), EngineError> {
        self.get_worker_head().await.map(|_| ())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SolverEngine (sketch solver lane, SCHEMA §7.4)
// ─────────────────────────────────────────────────────────────────────────────

#[async_trait]
impl SolverEngine for WorkerManager {
    async fn sketch_upsert(&self, sketch: &Sketch) -> Result<SketchUpsertDto, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request("SketchUpsert", wire::sketch_upsert_args(sketch))
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_sketch_upsert(&sketch.id.to_string(), &r))
    }

    async fn begin_gesture(
        &self,
        sketch_id: &str,
        sketch_revision: u64,
        gesture_id: u64,
        drag_point: EntityId,
        target: wire::GestureTarget,
        solver_policy_hash: &str,
    ) -> Result<BeginGestureDto, EngineError> {
        let client = self.client_or_err()?;
        let args = wire::begin_gesture_args(
            sketch_id,
            sketch_revision,
            gesture_id,
            drag_point,
            &target,
            solver_policy_hash,
        );
        let resp = client
            .request("BeginGesture", args)
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_begin_gesture(gesture_id, &r))
    }

    async fn solve_drag(
        &self,
        gesture_id: u64,
        seq: u64,
        drag_point: EntityId,
        target: [f64; 2],
    ) -> Result<DragSolveDto, EngineError> {
        // Fired latest-wins: the client sends the newest seq without serial awaits;
        // a stale seq may resolve `superseded` (SCHEMA §7.4) — parsed, not an error.
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "SolveDrag",
                wire::solve_drag_args(gesture_id, seq, drag_point, target),
            )
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_solve_drag(&r))
    }

    async fn end_gesture(
        &self,
        sketch_id: &str,
        gesture_id: u64,
        final_target: Option<[f64; 2]>,
    ) -> Result<SketchUpsertDto, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "EndGesture",
                wire::end_gesture_args(gesture_id, final_target),
            )
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_sketch_upsert(sketch_id, &r))
    }

    async fn sketch_regions(&self, sketch_id: &str) -> Result<FinishSketchDto, EngineError> {
        // Uses the resp binary tail (previewTriangles bins, SCHEMA §7.4 inline §5.2).
        let client = self.client_or_err()?;
        let inflight = client
            .start_request(
                "SketchRegions",
                wire::sketch_regions_args(sketch_id),
                Lane::Control,
            )
            .await
            .map_err(protocol_err)?;
        let (resp, tail) = inflight.response_with_bin().await.map_err(protocol_err)?;
        let sections = resp.bin.clone().unwrap_or_default();
        let result = ok_result(resp)?;
        wire::parse_sketch_regions(sketch_id, &result, &sections, &tail).map_err(|e| protocol(&e))
    }
}

/// Drives one `ExecutePlan`: send it, forward each `planStep` `event` as
/// [`PlanEvent::Step`], then the terminal `PlanPrepared`/error/crash. Records
/// crashes against the **crashing op's** poison key and fast-fails an open circuit
/// (SCHEMA §8 / F3).
async fn stream_plan(shared: Arc<Shared>, request: PlanRequest, tx: mpsc::Sender<PlanEvent>) {
    let job = request.job_id;
    // The planner keeps `prefix_hashes` index-aligned with `ops`
    // (`regen::planner::compute_hashes`). Asserted at the consumption site so a
    // hand-rolled `PlanRequest` that breaks the invariant is loud in dev; release
    // still degrades gracefully (see `plan_op_keys`) rather than aborting a regen.
    debug_assert_eq!(
        request.prefix_hashes.len(),
        request.ops.len(),
        "PlanRequest.prefix_hashes must be index-aligned with ops"
    );
    // Retirement is terminal, so it outranks even the poison gate: there is no worker
    // and there never will be one again. Fail fast with no dispatch and no wait — a
    // retired manager must never make a caller sit through a readiness timeout.
    if shared.is_retired() {
        let _ = tx.send(PlanEvent::Failed(retired_err())).await;
        return;
    }
    // One candidate poison key per op (`prefixHash(op) | opRecordId | fingerprint`);
    // the crashing op is one of them, so a poisoned plan is caught up front (F3).
    //
    // Deliberately BEFORE the connection check: a plan that keeps killing the worker
    // is most likely asked for again mid-restart, and the whole point of F3 is that
    // it then reports the CIRCUIT (with the last crash message) rather than a bare
    // "not connected". That is sound because `hello` is not cleared on a disconnect,
    // so `fingerprint()` still returns the last handshake's value and the keys match
    // the ones the same plan minted before the crash. The one case it does NOT hold
    // — no handshake has ever completed, fingerprint "" — is handled inside
    // `plan_op_keys`, which mints NO keys there; the gate then finds nothing and the
    // connection check below reports the honest "worker not connected" (VF-M1
    // secondary: keys minted under an empty fingerprint could never match the ones
    // the same plan mints after connecting, so they must not be minted at all).
    let op_keys = plan_op_keys(&request, &shared.fingerprint());
    if let Some((key, last)) = shared.plan_circuit_open(&op_keys) {
        let _ = tx
            .send(PlanEvent::Failed(EngineError::Crashed {
                message: format!(
                    "crash circuit breaker open (poison) — plan abandoned \
                     (key {key}; last crash: {last})"
                ),
            }))
            .await;
        return;
    }
    let Some(client) = shared.client() else {
        let _ = tx
            .send(PlanEvent::Failed(EngineError::NotConnected {
                message: "no session yet (handshake not completed, or restarting)".into(),
            }))
            .await;
        return;
    };

    let mut events = client.subscribe();
    let inflight = match client
        .start_request(
            "ExecutePlan",
            wire::execute_plan_args(&request),
            Lane::Control,
        )
        .await
    {
        Ok(i) => i,
        Err(e) => {
            let _ = tx.send(PlanEvent::Failed(protocol_err(e))).await;
            return;
        }
    };
    let id = inflight.id;
    shared.inflight.lock().unwrap().insert(job, id);
    // `response_with_bin`, not `response`: the terminal's binary tail carries the
    // inline `artifacts.tessellate` MESH1 blobs for every prepared body (SCHEMA §7.2).
    // Dropping the tail here is what forced a second full tessellation per regen.
    let mut resp_fut = Box::pin(inflight.response_with_bin());

    // Count executed steps so a transport loss can be attributed to the crashing op
    // (= the op at `steps_received`, clamped) rather than the plan's last op (F3).
    let mut steps_received = 0usize;
    let terminal = loop {
        tokio::select! {
            biased;
            ev = events.recv() => match ev {
                Ok(WorkerEvent::Event(e)) if e.id == id && e.event == "planStep" => {
                    let step = e.step_index.map_or(0, |s| s as usize);
                    match wire::parse_plan_step(&e.payload, step) {
                        Ok(s) => {
                            steps_received += 1;
                            if tx.send(PlanEvent::Step(s)).await.is_err() {
                                shared.inflight.lock().unwrap().remove(&job);
                                return;
                            }
                        }
                        Err(msg) => break StreamEnd::Local(EngineError::Protocol { message: msg }),
                    }
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    break StreamEnd::Local(EngineError::Protocol {
                        message: "planStep event stream lagged (dropped frame)".into(),
                    });
                }
                Err(broadcast::error::RecvError::Closed) => {} // resp future resolves ConnectionLost.
            },
            resp = &mut resp_fut => break StreamEnd::Resp(resp),
        }
    };

    shared.inflight.lock().unwrap().remove(&job);
    let event = finish_plan(&shared, &op_keys, steps_received, job, terminal);
    let _ = tx.send(event).await;
}

/// The stream terminal: a wire `resp` **with its binary tail** (or transport error)
/// vs a local parse error. The tail carries the inline tessellate artifacts.
enum StreamEnd {
    Resp(Result<(onecad_protocol::messages::RespFrame, Vec<u8>), ProtocolError>),
    Local(EngineError),
}

/// Maps a stream terminal to the final [`PlanEvent`], updating poison state.
/// `op_keys` is the plan's per-op candidate keys (execution order); `steps_received`
/// is how many `planStep`s arrived, so a crash is attributed to the crashing op.
fn finish_plan(
    shared: &Shared,
    op_keys: &[String],
    steps_received: usize,
    job: JobId,
    terminal: StreamEnd,
) -> PlanEvent {
    match terminal {
        StreamEnd::Local(err) => PlanEvent::Failed(err),
        StreamEnd::Resp(Ok((resp, tail))) if resp.ok => {
            let sections = resp.bin.clone();
            let result = resp.result.unwrap_or(Value::Null);
            match wire::parse_plan_prepared_with_artifacts(job, &result, sections.as_deref(), &tail)
            {
                Ok(p) => {
                    shared.record_success(op_keys);
                    PlanEvent::Prepared(p)
                }
                Err(msg) => PlanEvent::Failed(EngineError::Protocol { message: msg }),
            }
        }
        StreamEnd::Resp(Ok((resp, _tail))) => {
            // Well-framed worker error (recoverable op failure / protocol) — NOT a
            // crash, so the poison counter is untouched.
            let err = resp.error.as_ref().map_or_else(
                || EngineError::Protocol {
                    message: "resp ok=false without error".into(),
                },
                wire::map_error,
            );
            PlanEvent::Failed(err)
        }
        StreamEnd::Resp(Err(proto)) => {
            // Transport died mid-plan ⇒ crash. Key the poison entry on the CRASHING
            // op (the op at `steps_received`, clamped to the last), not the plan's
            // last op (F3). Opening the circuit FAILS FAST that plan-key next time,
            // but does NOT kill the worker (the supervisor keeps it alive so other
            // plans still run) — only the F2 flap budget reaches Failed.
            if let Some(key) = op_keys.get(steps_received.min(op_keys.len().saturating_sub(1))) {
                if shared.record_crash(key, &proto.to_string()) {
                    shared.emit(WorkerLifecycle::CircuitOpen { key: key.clone() });
                }
            }
            PlanEvent::Failed(EngineError::Crashed {
                message: format!("worker crashed mid-plan: {proto}"),
            })
        }
    }
}

/// A plan's candidate poison keys — one per op, `historyPrefixHash | opRecordId |
/// fingerprint` in execution order (SCHEMA §8 keys on `(history hash, op,
/// fingerprint)`; the crashing op is one of these — F3).
///
/// The hash is `prefix_hashes[i]`, the hash **through** op `i`, NOT the plan's
/// `expected_base_hash`. That distinction is the whole point (VF-M1): the base hash
/// is identical for every op of a plan and is unchanged by editing any of them, so a
/// base-keyed circuit could never be healed by fixing the crashing parameter — the
/// user's only escape was restarting the app. A prefix hash folds the timeline
/// through op `i`'s own params, so a param edit mints a new key and the circuit
/// heals on the next dispatch.
///
/// **Threshold economics.** `poison_threshold` (3) was calibrated against the coarse
/// per-plan key, where all N ops shared one counter and ANY crash in the plan
/// advanced it — three crashes anywhere tripped the breaker. Per-op keys are
/// strictly sharper: a crash now advances only the counter of the op it was
/// attributed to (`steps_received`, clamped), so tripping still needs three crashes
/// but they must all be the SAME op at the SAME timeline prefix. That can only make
/// the breaker slower to open, never faster, and the thing it protects — "stop
/// re-dispatching a plan that reliably kills the worker" — needs exactly that
/// repetition to be established. 3 therefore remains sound and is left alone.
fn plan_op_keys(req: &PlanRequest, fingerprint: &str) -> Vec<String> {
    // No live handshake ⇒ no fingerprint. Keys minted now could never match the ones
    // the same plan mints after the worker connects, so they would only accumulate
    // as unreachable entries while the fail-fast gate stayed blind. Key nothing.
    if fingerprint.is_empty() {
        return Vec::new();
    }
    req.ops
        .iter()
        .enumerate()
        .map(|(i, o)| {
            // Defensive fallback: `PlanRequest` has public fields, so a hand-rolled
            // request can carry a short `prefix_hashes`. Degrade that op to the old
            // coarse base key — still deterministic, just no longer param-sensitive
            // — rather than panicking on the dispatch path.
            let prefix = req
                .prefix_hashes
                .get(i)
                .unwrap_or(&req.expected_base_hash)
                .as_str();
            format!("{}|{}|{}", prefix, o.record_id, fingerprint)
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// MeshProvider (Tessellate + MESH1 bulk assembly, SCHEMA §5.2 / §7.6)
// ─────────────────────────────────────────────────────────────────────────────

/// A finalized bulk payload: `(bytes, manifest totalBytes, manifest sha256)`.
type MeshPayload = (Vec<u8>, Option<u64>, Option<String>);

/// A bulk MESH1 stream being reassembled from chunk frames. Records the manifest's
/// integrity fields + every received byte segment so [`finalize`](Self::finalize)
/// can verify the payload tiles `[0, totalBytes)` EXACTLY — a gap or overlap is a
/// PROTOCOL_ERROR, never zero-fill-and-hope (F5).
#[derive(Default)]
struct StreamAcc {
    buf: Vec<u8>,
    total_bytes: Option<u64>,
    sha256: Option<String>,
    /// Received `(start, end)` byte ranges, in arrival order.
    segments: Vec<(u64, u64)>,
}

impl StreamAcc {
    fn apply(&mut self, c: &onecad_protocol::messages::ChunkFrame, bin: &[u8]) {
        match c.kind {
            onecad_protocol::messages::ChunkKind::Manifest => {
                self.total_bytes = c.total_bytes;
                self.sha256 = c.sha256.clone();
            }
            onecad_protocol::messages::ChunkKind::Data => {
                let off = c.byte_offset.unwrap_or(0);
                let end = off + bin.len() as u64;
                let (start_us, end_us) = (off as usize, end as usize);
                if self.buf.len() < end_us {
                    self.buf.resize(end_us, 0);
                }
                self.buf[start_us..end_us].copy_from_slice(bin);
                self.segments.push((off, end));
            }
        }
    }

    /// Verifies the received segments tile `[0, totalBytes)` with no gap and no
    /// overlap, then returns `(payload, manifest totalBytes, manifest sha256)`.
    fn finalize(mut self) -> Result<MeshPayload, EngineError> {
        let mut segs = self.segments.clone();
        segs.sort_by_key(|&(start, _)| start);
        let mut cursor = 0u64;
        for &(start, end) in &segs {
            match start.cmp(&cursor) {
                std::cmp::Ordering::Greater => {
                    return Err(protocol("MESH1 stream has a gap between chunks"));
                }
                std::cmp::Ordering::Less => {
                    return Err(protocol("MESH1 stream has overlapping chunks"));
                }
                std::cmp::Ordering::Equal => cursor = end,
            }
        }
        if let Some(t) = self.total_bytes {
            if cursor != t {
                return Err(protocol("MESH1 stream chunks do not cover [0, totalBytes)"));
            }
            self.buf.truncate(t as usize);
        }
        Ok((self.buf, self.total_bytes, self.sha256))
    }
}

fn parse_preview_result(
    result: &Value,
    sections: &[onecad_protocol::messages::BinSection],
    tail: &[u8],
) -> Result<crate::dto::PreviewResultDto, EngineError> {
    let snapshot_id = result
        .get("snapshotId")
        .and_then(Value::as_u64)
        .ok_or_else(|| protocol("PreviewOp: missing/invalid snapshotId"))?;
    let body_events = parse_preview_body_events(result.get("bodyEvents"))?;
    let changed_bodies = preview_body_id_array(result.get("changedBodies"), "changedBodies")?;
    let deleted_bodies = preview_body_id_array(result.get("deletedBodies"), "deletedBodies")?;
    let needs_repair = preview_value_array(result.get("needsRepair"), "needsRepair")?;
    let section_table =
        wire::validate_bin_sections("PreviewOp", sections, tail).map_err(|e| protocol(&e))?;
    let (bodies, referenced) = parse_preview_meshes(result.get("meshes"), &section_table, tail)?;
    wire::reject_unreferenced_sections("PreviewOp", &section_table, &referenced)
        .map_err(|e| protocol(&e))?;
    validate_preview_body_sets(&body_events, &changed_bodies, &deleted_bodies, &bodies)?;
    Ok(crate::dto::PreviewResultDto {
        snapshot_id,
        bodies,
        body_events,
        changed_bodies,
        deleted_bodies,
        needs_repair,
    })
}

fn preview_value_array(value: Option<&Value>, field: &str) -> Result<Vec<Value>, EngineError> {
    match value {
        Some(Value::Array(items)) => Ok(items.clone()),
        _ => Err(protocol(&format!(
            "PreviewOp: missing/invalid {field} array"
        ))),
    }
}

fn preview_body_id_array(value: Option<&Value>, field: &str) -> Result<Vec<String>, EngineError> {
    preview_value_array(value, field)?
        .into_iter()
        .map(|item| {
            let id = item.as_str().filter(|id| !id.is_empty()).ok_or_else(|| {
                protocol(&format!(
                    "PreviewOp: {field} contains a non-string/empty id"
                ))
            })?;
            normalize_preview_body_id(id, field)
        })
        .collect()
}

fn parse_preview_body_events(
    value: Option<&Value>,
) -> Result<Vec<crate::dto::PreviewBodyEventDto>, EngineError> {
    preview_value_array(value, "bodyEvents")?
        .into_iter()
        .map(|event| {
            let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
            let body_id = event.get("bodyId").and_then(Value::as_str).unwrap_or("");
            if !matches!(kind, "created" | "modified" | "deleted") || body_id.is_empty() {
                return Err(protocol("PreviewOp: malformed bodyEvents entry"));
            }
            Ok(crate::dto::PreviewBodyEventDto {
                kind: kind.to_string(),
                body_id: normalize_preview_body_id(body_id, "bodyEvents")?,
            })
        })
        .collect()
}

fn normalize_preview_body_id(body_id: &str, field: &str) -> Result<String, EngineError> {
    wire::parse_body_id(body_id)
        .map(|id| id.to_string())
        .map_err(|e| protocol(&format!("PreviewOp: invalid {field} body id: {e}")))
}

fn parse_preview_meshes<'a>(
    value: Option<&Value>,
    sections: &HashMap<&'a str, &'a onecad_protocol::messages::BinSection>,
    tail: &[u8],
) -> Result<(Vec<crate::dto::PreviewBodyDto>, HashSet<String>), EngineError> {
    let meshes = preview_value_array(value, "meshes")?;
    let mut referenced = HashSet::with_capacity(meshes.len());
    let mut parsed = Vec::with_capacity(meshes.len());
    for mesh in meshes {
        let (body, section) = parse_preview_mesh(&mesh, sections, tail)?;
        if !referenced.insert(section.clone()) {
            return Err(protocol(&format!(
                "PreviewOp: binary section {section:?} reused"
            )));
        }
        parsed.push(body);
    }
    Ok((parsed, referenced))
}

fn parse_preview_mesh<'a>(
    mesh: &Value,
    sections: &HashMap<&'a str, &'a onecad_protocol::messages::BinSection>,
    tail: &[u8],
) -> Result<(crate::dto::PreviewBodyDto, String), EngineError> {
    let body_id = mesh
        .get("bodyId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| protocol("PreviewOp: mesh missing bodyId"))?;
    let section_name = mesh
        .get("bin")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| protocol("PreviewOp: mesh missing bin"))?;
    let section = sections
        .get(section_name)
        .copied()
        .ok_or_else(|| protocol("PreviewOp: mesh bin section missing"))?;
    let start = section.off as usize;
    let end = start
        .checked_add(section.len as usize)
        .ok_or_else(|| protocol("PreviewOp: mesh section range overflow"))?;
    let bytes = tail
        .get(start..end)
        .ok_or_else(|| protocol("PreviewOp: mesh section out of range"))?;
    onecad_protocol::mesh::validate_mesh_blob(bytes)
        .map_err(|e| protocol(&format!("PreviewOp: invalid MESH1 for {body_id}: {e}")))?;
    Ok((
        crate::dto::PreviewBodyDto {
            body_id: normalize_preview_body_id(body_id, "meshes")?,
            mesh: bytes.to_vec(),
        },
        section_name.to_string(),
    ))
}

fn validate_preview_body_sets(
    events: &[crate::dto::PreviewBodyEventDto],
    changed: &[String],
    deleted: &[String],
    bodies: &[crate::dto::PreviewBodyDto],
) -> Result<(), EngineError> {
    let changed_set: HashSet<&str> = changed.iter().map(String::as_str).collect();
    let deleted_set: HashSet<&str> = deleted.iter().map(String::as_str).collect();
    let mesh_set: HashSet<&str> = bodies.iter().map(|body| body.body_id.as_str()).collect();
    if changed_set.len() != changed.len()
        || deleted_set.len() != deleted.len()
        || mesh_set.len() != bodies.len()
    {
        return Err(protocol("PreviewOp: duplicate body id"));
    }
    if !changed_set.is_disjoint(&deleted_set) {
        return Err(protocol("PreviewOp: body is both changed and deleted"));
    }
    if changed_set != mesh_set {
        return Err(protocol("PreviewOp: changedBodies/meshes mismatch"));
    }
    let mut event_ids = HashSet::new();
    let mut event_changed = HashSet::new();
    let mut event_deleted = HashSet::new();
    for event in events {
        if !event_ids.insert(event.body_id.as_str()) {
            return Err(protocol("PreviewOp: duplicate/contradictory bodyEvent"));
        }
        match event.kind.as_str() {
            "created" | "modified" => {
                event_changed.insert(event.body_id.as_str());
            }
            "deleted" => {
                event_deleted.insert(event.body_id.as_str());
            }
            _ => unreachable!("body event kind validated while parsing"),
        }
    }
    if event_changed != changed_set || event_deleted != deleted_set {
        return Err(protocol("PreviewOp: bodyEvents/lifecycle arrays mismatch"));
    }
    Ok(())
}

#[async_trait]
impl crate::worker::PreviewEngine for WorkerManager {
    async fn preview_op(
        &self,
        operation: Operation,
        op_id: String,
        sketch_id: Option<String>,
        expected_snapshot: Option<SnapshotId>,
        lod: Lod,
    ) -> Result<crate::dto::PreviewResultDto, EngineError> {
        let client = self.client_or_err()?;
        let op = wire::preview_wire_op(&operation, &op_id);
        let mut args = serde_json::json!({ "op": op, "lod": crate::worker::lod_str(lod) });
        if let Some(sid) = sketch_id {
            args["sketchId"] = Value::String(sid);
        }
        if let Some(snapshot) = expected_snapshot {
            args["expectedSnapshotId"] = json!(snapshot.0);
        }
        // Preview meshes ride INLINE in the response tail (coarse LOD, only the
        // bodies the op touched), so there is no bulk-chunk lane to drain here —
        // unlike `fetch_mesh`, which serves arbitrary-size document meshes.
        let (resp, tail) = client
            .start_request("PreviewOp", args, Lane::Control)
            .await
            .map_err(protocol_err)?
            .response_with_bin()
            .await
            .map_err(protocol_err)?;
        let sections = resp.bin.clone().unwrap_or_default();
        let result = ok_result(resp)?;
        parse_preview_result(&result, &sections, &tail)
    }
}

#[async_trait]
impl crate::worker::ElementQuery for WorkerManager {
    async fn query_element(
        &self,
        snapshot: SnapshotId,
        body: BodyId,
        element: &str,
    ) -> Result<Option<crate::dto::ElementInfoDto>, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "QueryElement",
                wire::query_element_args(snapshot, body, element),
            )
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_query_element(&r))
    }

    async fn query_element_by_topo_key(
        &self,
        snapshot: SnapshotId,
        body: BodyId,
        topo_key: &str,
    ) -> Result<Option<crate::dto::ElementInfoDto>, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "QueryElement",
                wire::query_element_by_topo_key_args(snapshot, body, topo_key),
            )
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_query_element(&r))
    }

    async fn query_mass_properties(
        &self,
        body: BodyId,
        body_id_label: String,
    ) -> Result<crate::dto::MassPropertiesDto, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "QueryMassProperties",
                wire::query_mass_properties_args(body),
            )
            .await
            .map_err(protocol_err)?;
        // A malformed reading is a PROTOCOL break, never a default: a fabricated
        // zero volume would render as a genuine "0 mm³" measurement.
        wire::parse_mass_properties(body_id_label, &ok_result(resp)?)
            .map_err(|message| EngineError::Protocol { message })
    }

    async fn classify_element(
        &self,
        body: BodyId,
        element: &str,
    ) -> Result<Option<crate::dto::ClassifyElementDto>, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "ClassifyElement",
                wire::classify_element_args(body, element),
            )
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_classify_element(&r))
    }

    async fn classify_element_by_topo_key(
        &self,
        body: BodyId,
        topo_key: &str,
    ) -> Result<Option<crate::dto::ClassifyElementDto>, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "ClassifyElement",
                wire::classify_element_by_topo_key_args(body, topo_key),
            )
            .await
            .map_err(protocol_err)?;
        ok_result(resp).map(|r| wire::parse_classify_element(&r))
    }

    async fn query_body_topology(
        &self,
        body: BodyId,
        body_id_label: String,
    ) -> Result<crate::dto::BodyTopologyDto, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request("QueryBodyTopology", wire::query_body_topology_args(body))
            .await
            .map_err(protocol_err)?;
        wire::parse_body_topology(body_id_label, &ok_result(resp)?)
            .map_err(|message| EngineError::Protocol { message })
    }
}

#[async_trait]
impl crate::worker::FaceBoundaryProjection for WorkerManager {
    async fn project_face_boundary_frame(
        &self,
        snapshot: SnapshotId,
        body: BodyId,
        address: wire::FaceAddress<'_>,
    ) -> Result<Option<onecad_core::sketch::FaceFrame>, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "ProjectFaceBoundary",
                wire::project_face_boundary_frame_args(snapshot, body, address),
            )
            .await
            .map_err(protocol_err)?;
        // A malformed frame is a PROTOCOL break, never a default: fabricating
        // `(0,0,0)/(0,0,1)` would silently sketch on the world XY plane.
        wire::parse_project_face_boundary_frame(&ok_result(resp)?)
            .map_err(|message| EngineError::Protocol { message })
    }

    async fn project_face_boundary(
        &self,
        snapshot: SnapshotId,
        body: BodyId,
        address: wire::FaceAddress<'_>,
        plane: &onecad_core::sketch::SketchPlane,
        scope: wire::ProjectionScope,
    ) -> Result<Option<onecad_core::sketch::ProjectionPayload>, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "ProjectFaceBoundary",
                wire::project_face_boundary_args(snapshot, body, address, plane, scope),
            )
            .await
            .map_err(protocol_err)?;
        wire::parse_project_face_boundary(&ok_result(resp)?)
            .map_err(|message| EngineError::Protocol { message })
    }

    async fn prepare_offset_face(
        &self,
        snapshot: SnapshotId,
        picks: &[wire::OffsetFacePick<'_>],
        chain_tangent_faces: bool,
        distance_type: onecad_core::document::record::OffsetDistanceType,
    ) -> Result<crate::dto::PrepareOffsetFaceDto, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "PrepareOffsetFace",
                wire::prepare_offset_face_args(snapshot, picks, chain_tangent_faces, distance_type),
            )
            .await
            .map_err(protocol_err)?;
        // A malformed closure is a PROTOCOL break, never a default: this response
        // is what the record FREEZES, so a fabricated body id or a silently empty
        // face list would author an op against the wrong geometry.
        wire::parse_prepare_offset_face(&ok_result(resp)?)
            .map_err(|message| EngineError::Protocol { message })
    }

    async fn prepare_edge_op(
        &self,
        snapshot: SnapshotId,
        mode: wire::EdgeOpMode,
        picks: &[wire::EdgeOpPick<'_>],
        chain_tangent_edges: bool,
    ) -> Result<crate::dto::PrepareEdgeOpDto, EngineError> {
        let client = self.client_or_err()?;
        let resp = client
            .request(
                "PrepareEdgeOp",
                wire::prepare_edge_op_args(snapshot, mode, picks, chain_tangent_edges),
            )
            .await
            .map_err(protocol_err)?;
        wire::parse_prepare_edge_op(&ok_result(resp)?)
            .map_err(|message| EngineError::Protocol { message })
    }

    async fn analyze_edge_op_range(
        &self,
        snapshot: SnapshotId,
        mode: wire::EdgeOpMode,
        picks: &[wire::EdgeOpPick<'_>],
        chain_tangent_edges: bool,
        request: wire::EdgeOpRangeRequest,
    ) -> Result<crate::dto::EdgeOpRangeDto, EngineError> {
        let client = self.client_or_err()?;
        // `start_request` rather than `request`, for the id: this is the one
        // read-only verb that runs dozens of OCCT builds, so it is the one that
        // can outlive its usefulness, and cancelling it needs the correlation id.
        let inflight = client
            .start_request(
                "AnalyzeEdgeOpRange",
                wire::analyze_edge_op_range_args(
                    snapshot,
                    mode,
                    picks,
                    chain_tangent_edges,
                    request,
                ),
                Lane::Control,
            )
            .await
            .map_err(protocol_err)?;
        let id = inflight.id;
        match tokio::time::timeout(ANALYZE_EDGE_OP_RANGE_LIVENESS, inflight.response()).await {
            Ok(resp) => wire::parse_analyze_edge_op_range(&ok_result(resp.map_err(protocol_err)?)?)
                .map_err(|message| EngineError::Protocol { message }),
            Err(_) => {
                // A LIVENESS backstop, not a policy timeout: the worker's own
                // deadline should have ended the search long before this. Reaching
                // here means the worker is wedged or the machine is pathological —
                // and the pending slot must be told, or the worker keeps building
                // for an answer nobody will read (SCHEMA §3.5: a cancel still gets
                // a terminal resp, so the slot resolves).
                let _ = client.cancel(id).await;
                Err(EngineError::Protocol {
                    message: "AnalyzeEdgeOpRange: worker exceeded its liveness backstop".into(),
                })
            }
        }
    }
}

#[async_trait]
impl crate::worker::StepImport for WorkerManager {
    async fn inspect_step(
        &self,
        path: &std::path::Path,
        include_geometry: bool,
    ) -> Result<crate::worker::StepInspection, EngineError> {
        let client = self.client_or_err()?;
        // The healed replay bytes ride INLINE in the response tail (SCHEMA §7.8) —
        // one `geometry` section, no bulk-chunk lane — so this is the same
        // `response_with_bin` shape `SketchRegions`/`PreviewOp` use.
        let (resp, tail) = client
            .start_request(
                "InspectStep",
                wire::inspect_step_args(&path.to_string_lossy(), include_geometry),
                Lane::Control,
            )
            .await
            .map_err(protocol_err)?
            .response_with_bin()
            .await
            .map_err(protocol_err)?;
        let sections = resp.bin.clone().unwrap_or_default();
        let result = ok_result(resp)?;
        let mut inspection = wire::parse_inspect_step(&result);
        if include_geometry {
            let by_name = wire::validate_bin_sections("InspectStep", &sections, &tail)
                .map_err(|e| protocol(&e))?;
            // A missing section when the bytes were REQUESTED is a protocol break, not
            // a soft miss: the caller is about to author a converted-codec record
            // against them, so an empty blob would persist a record whose replay source
            // does not exist.
            let section = by_name.get("geometry").ok_or_else(|| {
                protocol("InspectStep: includeGeometry set but no `geometry` bin section")
            })?;
            let start = section.off as usize;
            let end = start + section.len as usize;
            inspection.geometry_bytes = Some(tail[start..end].to_vec());
        }
        Ok(inspection)
    }
}

impl crate::worker::CircuitControl for WorkerManager {
    fn clear_circuit(&self) -> usize {
        WorkerManager::clear_circuit(self)
    }
}

#[async_trait]
impl crate::worker::WorkerReadiness for WorkerManager {
    async fn wait_ready(&self, timeout: Duration) -> bool {
        // Delegates to the inherent method (method-call syntax prefers it over
        // the trait method being implemented, so this is not recursive).
        WorkerManager::wait_ready(self, timeout).await
    }
}

#[async_trait]
impl MeshProvider for WorkerManager {
    async fn fetch_mesh(
        &self,
        body: BodyId,
        lod: Lod,
        _snapshot: SnapshotId,
    ) -> Result<Vec<u8>, EngineError> {
        let client = self.client_or_err()?;
        let req = TessellateRequest {
            bodies: BodySelector::Ids(vec![body]),
            lod,
            include_edges: true,
        };
        let mut events = client.subscribe();
        let inflight = client
            .start_request("Tessellate", wire::tessellate_args(&req), Lane::Control)
            .await
            .map_err(protocol_err)?;
        let id = inflight.id;
        let mut resp_fut = Box::pin(inflight.response_with_bin());
        let mut streams: HashMap<u64, StreamAcc> = HashMap::new();

        let terminal = loop {
            tokio::select! {
                biased;
                ev = events.recv() => match ev {
                    Ok(WorkerEvent::Chunk(c, bin)) if c.id == id => {
                        let is_data = matches!(c.kind, onecad_protocol::messages::ChunkKind::Data);
                        streams.entry(c.stream_id).or_default().apply(&c, &bin);
                        if is_data && !bin.is_empty() {
                            let _ = client.grant_credit(bin.len() as u64).await; // replenish (SCHEMA §5.3)
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) =>
                        return Err(EngineError::Protocol { message: "mesh chunk stream lagged".into() }),
                    Err(broadcast::error::RecvError::Closed) => {}
                },
                resp = &mut resp_fut => break resp,
            }
        };

        let (resp, tail) = terminal.map_err(protocol_err)?;
        // Drain any chunk frames buffered ahead of the terminal.
        while let Ok(WorkerEvent::Chunk(c, bin)) = events.try_recv() {
            if c.id == id {
                streams.entry(c.stream_id).or_default().apply(&c, &bin);
            }
        }
        let result = ok_result(resp.clone())?;
        assemble_mesh(&result, &resp, &tail, &mut streams)
    }
}

/// Extracts + verifies one MESH1 blob from a `Tessellate` result: inline (`bin`
/// section in the resp tail) or a bulk stream (`streamId`). A streamed payload's
/// chunks must tile `[0, totalBytes)` exactly (F5 gap-detection). The integrity
/// fields (`totalBytes`/`sha256`) are cross-checked between the manifest and the
/// resp handle when both are present (mismatch = error); when the resp handle omits
/// them the manifest's are used (a worker omitting resp fields never skips
/// verification); when BOTH are absent only the MESH1 header is validated. Verifies
/// size + SHA-256 and validates the header (Invariant 5 forward-verbatim).
fn assemble_mesh(
    result: &Value,
    resp: &onecad_protocol::messages::RespFrame,
    tail: &[u8],
    streams: &mut HashMap<u64, StreamAcc>,
) -> Result<Vec<u8>, EngineError> {
    let mesh = result
        .get("meshes")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| protocol("Tessellate result carried no meshes"))?;

    let resp_total = mesh.get("totalBytes").and_then(Value::as_u64);
    let resp_sha = mesh
        .get("sha256")
        .and_then(Value::as_str)
        .map(str::to_string);

    // (blob, manifest totalBytes, manifest sha256). Inline handles carry no manifest.
    let (blob, manifest_total, manifest_sha) =
        if let Some(name) = mesh.get("bin").and_then(Value::as_str) {
            let section = resp
                .bin
                .as_ref()
                .and_then(|secs| secs.iter().find(|s| s.name == name))
                .ok_or_else(|| protocol("inline mesh bin section missing"))?;
            let start = section.off as usize;
            let end = start + section.len as usize;
            let bytes = tail
                .get(start..end)
                .ok_or_else(|| protocol("inline mesh bin section out of range"))?
                .to_vec();
            (bytes, None, None)
        } else if let Some(stream_id) = mesh.get("streamId").and_then(Value::as_u64) {
            let acc = streams
                .remove(&stream_id)
                .ok_or_else(|| protocol("mesh stream frames never arrived"))?;
            acc.finalize()? // gap/overlap detection (F5)
        } else {
            return Err(protocol("mesh handle has neither inline bin nor streamId"));
        };

    let total = wire::reconcile_field(manifest_total, resp_total, "totalBytes")?;
    let sha = wire::reconcile_field(manifest_sha, resp_sha, "sha256")?;
    if total.is_none() && sha.is_none() {
        tracing::warn!(
            "mesh handle carried no totalBytes/sha256 (manifest or resp) — \
             validating MESH1 header only"
        );
    }
    wire::verify_mesh(&blob, total, sha.as_deref())?;
    Ok(blob)
}

// ─────────────────────────────────────────────────────────────────────────────
// Small helpers
// ─────────────────────────────────────────────────────────────────────────────

fn not_connected() -> EngineError {
    EngineError::NotConnected {
        message: "restarting or failed".into(),
    }
}

/// The fail-fast terminal for a retired manager. Distinct from [`not_connected`]
/// because it is NOT transient: no restart is coming, so a caller must not retry or
/// wait for readiness.
fn retired_err() -> EngineError {
    EngineError::NotConnected {
        message: "retired (its document was closed, or the app is quitting)".into(),
    }
}

/// Reconstructs the base [`BodyRegistry`] + [`ElementIndex`] a checkpoint restore
/// seeds the executor scratch with (review F3 — from the immutable artifacts, not
/// live state). Each artifact contributes its body; the base element index is empty
/// in V1 (the worker persists a placeholder partition — a documented divergence — and
/// the in-session restore uses the retained partition, so a checkpoint history with
/// no pre-referenced sub-elements reconstructs an empty index correctly).
fn reconstruct_base(artifacts: &CheckpointArtifacts) -> (BodyRegistry, ElementIndex) {
    let mut registry = BodyRegistry::new();
    for (i, artifact) in artifacts.artifacts.iter().enumerate() {
        registry.register(BodyMeta::new(
            artifact.envelope.body,
            format!("Body {}", i + 1),
            RecordId(uuid::Uuid::nil()),
        ));
    }
    (registry, ElementIndex::new())
}

/// An unusable restore (`restored:false` or drift) — the executor discards + replays
/// from 0 (Invariant 7). Carries empty base state (ignored on the fallback path).
fn unusable_restore(snapshot_id: u64, drift: bool) -> RestoreResult {
    RestoreResult {
        restored: false,
        snapshot_id: SnapshotId(snapshot_id),
        drift_detected: drift,
        drift_detail: None,
        checkpoint_step: 0,
        base_registry: BodyRegistry::new(),
        base_elements: ElementIndex::new(),
    }
}

fn protocol(msg: &str) -> EngineError {
    EngineError::Protocol {
        message: msg.into(),
    }
}

/// Maps a transport [`ProtocolError`] to the engine taxonomy: a lost connection is
/// a crash (restart + replay); anything else is a protocol violation (SCHEMA §8).
fn protocol_err(e: ProtocolError) -> EngineError {
    match e {
        ProtocolError::ConnectionLost(_) | ProtocolError::Timeout => EngineError::Crashed {
            message: format!("worker connection lost: {e}"),
        },
        other => EngineError::Protocol {
            message: other.to_string(),
        },
    }
}

/// Unwraps a success `resp` into its `result` object, or maps its error terminal.
fn ok_result(resp: onecad_protocol::messages::RespFrame) -> Result<Value, EngineError> {
    if resp.ok {
        Ok(resp.result.unwrap_or(Value::Null))
    } else {
        Err(resp
            .error
            .as_ref()
            .map_or_else(|| protocol("resp ok=false without error"), wire::map_error))
    }
}

#[cfg(test)]
mod preview_parse_tests {
    use super::*;

    fn wire_body(seed: u128) -> String {
        format!("body_{}", uuid::Uuid::from_u128(seed))
    }

    fn empty_mesh_blob() -> Vec<u8> {
        let mut blob = vec![0; onecad_protocol::mesh::MESH_HEADER_LEN];
        blob[0..4].copy_from_slice(&onecad_protocol::mesh::MESH_MAGIC_LE.to_le_bytes());
        blob[4..6].copy_from_slice(&onecad_protocol::mesh::MESH_VERSION.to_le_bytes());
        blob
    }

    fn base_result() -> Value {
        json!({
            "snapshotId": 7,
            "bodyEvents": [],
            "changedBodies": [],
            "deletedBodies": [],
            "needsRepair": [],
            "meshes": []
        })
    }

    fn assert_protocol_error(result: Result<crate::dto::PreviewResultDto, EngineError>) {
        assert!(matches!(result, Err(EngineError::Protocol { .. })));
    }

    #[test]
    fn malformed_preview_body_event_is_not_silently_dropped() {
        let mut result = base_result();
        result["bodyEvents"] = json!([{ "kind": "modified" }]);
        assert_protocol_error(parse_preview_result(&result, &[], &[]));
    }

    #[test]
    fn malformed_preview_mesh_handle_is_not_silently_dropped() {
        let mut result = base_result();
        result["changedBodies"] = json!(["body_preview"]);
        result["meshes"] = json!([{ "bodyId": "body_preview" }]);
        assert_protocol_error(parse_preview_result(&result, &[], &[]));
    }

    #[test]
    fn missing_preview_mesh_section_is_a_protocol_error() {
        let mut result = base_result();
        result["changedBodies"] = json!(["body_preview"]);
        result["meshes"] = json!([{ "bodyId": "body_preview", "bin": "mesh:body_preview" }]);
        assert_protocol_error(parse_preview_result(&result, &[], &[]));
    }

    #[test]
    fn preview_lifecycle_arrays_must_match_body_events() {
        let mut result = base_result();
        result["bodyEvents"] = json!([{ "kind": "deleted", "bodyId": "body_old" }]);
        assert_protocol_error(parse_preview_result(&result, &[], &[]));
    }

    #[test]
    fn duplicate_preview_body_events_are_rejected() {
        let mut result = base_result();
        result["bodyEvents"] = json!([
            { "kind": "modified", "bodyId": "body_same" },
            { "kind": "deleted", "bodyId": "body_same" }
        ]);
        result["deletedBodies"] = json!(["body_same"]);
        assert_protocol_error(parse_preview_result(&result, &[], &[]));
    }

    #[test]
    fn preview_body_ids_are_normalized_and_deletion_only_is_preserved() {
        let body = wire_body(0x123);
        let bare = uuid::Uuid::from_u128(0x123).to_string();
        let mut result = base_result();
        result["bodyEvents"] = json!([{ "kind": "deleted", "bodyId": body }]);
        result["deletedBodies"] = json!([body]);

        let parsed = parse_preview_result(&result, &[], &[]).unwrap();
        assert_eq!(parsed.deleted_bodies, [bare.as_str()]);
        assert_eq!(parsed.body_events[0].body_id, bare);
        assert!(parsed.bodies.is_empty());
    }

    #[test]
    fn preview_mesh_body_id_and_lifecycle_keep_the_same_normalized_id() {
        let body = wire_body(0x456);
        let bare = uuid::Uuid::from_u128(0x456).to_string();
        let mesh = empty_mesh_blob();
        let sections = [onecad_protocol::messages::BinSection {
            name: "mesh:candidate".into(),
            off: 0,
            len: mesh.len() as u32,
        }];
        let mut result = base_result();
        result["bodyEvents"] = json!([{ "kind": "modified", "bodyId": body }]);
        result["changedBodies"] = json!([body]);
        result["meshes"] = json!([{ "bodyId": body, "bin": "mesh:candidate" }]);

        let parsed = parse_preview_result(&result, &sections, &mesh).unwrap();
        assert_eq!(parsed.changed_bodies, [bare.as_str()]);
        assert_eq!(parsed.body_events[0].body_id, bare);
        assert_eq!(parsed.bodies[0].body_id, parsed.changed_bodies[0]);
    }

    #[test]
    fn preview_rejects_duplicate_overlapping_and_unreferenced_sections() {
        let result = base_result();
        let tail = vec![0; 128];
        let section = |name: &str, off, len| onecad_protocol::messages::BinSection {
            name: name.into(),
            off,
            len,
        };
        assert_protocol_error(parse_preview_result(
            &result,
            &[section("same", 0, 64), section("same", 64, 64)],
            &tail,
        ));
        assert_protocol_error(parse_preview_result(
            &result,
            &[section("a", 0, 64), section("b", 32, 64)],
            &tail,
        ));
        assert_protocol_error(parse_preview_result(
            &result,
            &[section("unexpected", 0, 64)],
            &tail,
        ));
    }

    #[test]
    fn preview_rejects_a_binary_section_reused_by_two_meshes() {
        let first = wire_body(0xA);
        let second = wire_body(0xB);
        let mesh = empty_mesh_blob();
        let sections = [onecad_protocol::messages::BinSection {
            name: "mesh:shared".into(),
            off: 0,
            len: mesh.len() as u32,
        }];
        let mut result = base_result();
        result["bodyEvents"] = json!([
            { "kind": "modified", "bodyId": first },
            { "kind": "modified", "bodyId": second }
        ]);
        result["changedBodies"] = json!([first, second]);
        result["meshes"] = json!([
            { "bodyId": first, "bin": "mesh:shared" },
            { "bodyId": second, "bin": "mesh:shared" }
        ]);
        assert_protocol_error(parse_preview_result(&result, &sections, &mesh));
    }
}

#[cfg(test)]
mod poison_tests {
    //! VF-M1 — the crash circuit's key, reset policy and memory bound.
    //!
    //! Exercised over [`Shared`] + [`plan_op_keys`] directly: the behaviour under
    //! test is pure bookkeeping, and driving it through a live supervisor would
    //! replace a deterministic assertion with a crashing-worker race.

    use super::*;
    use onecad_core::document::record::{DeterminismSettings, OpaqueOperation, OperationInputs};
    use onecad_core::regen::{HistoryPrefixHash, PlanArtifacts, PlannedOp, PolicyVersions};

    const FP: &str = "occt-793-aaaa";

    /// A `Shared` with no supervisor attached — nothing here touches `conn`.
    fn shared(poison_threshold: u32) -> Arc<Shared> {
        let (lifecycle, _) = broadcast::channel(8);
        Arc::new(Shared {
            config: SupervisorConfig {
                binary: PathBuf::from("/nonexistent/onecad-worker"),
                envs: Vec::new(),
                ping_interval: Duration::from_secs(5),
                ping_timeout: Duration::from_secs(5),
                max_missed_pings: 2,
                backoff: Vec::new(),
                max_rapid_deaths: 3,
                healthy_threshold: Duration::from_secs(5),
                poison_threshold,
                auto_open_session: false,
            },
            conn: RwLock::new(None),
            epoch: AtomicU64::new(1),
            state: Mutex::new(WorkerState::Starting),
            hello: Mutex::new(None),
            inflight: Mutex::new(HashMap::new()),
            poison: Mutex::new(Poison::default()),
            lifecycle,
            restart_hook: RwLock::new(None),
            retired: AtomicBool::new(false),
            retire_signal: Notify::new(),
            torn_down: AtomicBool::new(false),
        })
    }

    fn op(record: u128) -> PlannedOp {
        PlannedOp {
            record_id: RecordId(uuid::Uuid::from_u128(record)),
            step_index: 0,
            operation: Operation::Opaque(OpaqueOperation {
                raw: Default::default(),
            }),
            inputs: OperationInputs::default(),
            determinism: DeterminismSettings::default(),
        }
    }

    /// A two-op plan over a fixed base whose per-op prefix hashes are given. The
    /// planner would derive these from the ops' params; spelling them out is what
    /// lets a test model "the user edited op 1" as a single changed hash.
    fn plan(prefixes: &[&str]) -> PlanRequest {
        PlanRequest {
            job_id: JobId(uuid::Uuid::from_u128(0x101)),
            document_revision: DocumentRevision(1),
            worker_epoch: WorkerEpoch(1),
            expected_base_hash: HistoryPrefixHash::new("base-hash"),
            prefix_hashes: prefixes
                .iter()
                .map(|h| HistoryPrefixHash::new(*h))
                .collect(),
            base_checkpoint: None,
            base_checkpoint_artifacts: None,
            ops: vec![op(0xA), op(0xB)],
            policy_versions: PolicyVersions::default(),
            target_step: 1,
            artifacts: PlanArtifacts::default(),
            edited_from: None,
            checkpoint_fallback_replay: false,
        }
    }

    // ── 1. The key must follow the op's params, not just the plan's base ──────
    #[test]
    fn editing_the_crashing_ops_params_heals_the_circuit() {
        let s = shared(3);
        let before = plan(&["h0", "h1-crashy"]);
        let keys = plan_op_keys(&before, FP);
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1], "one key per op, not one key per plan");

        // Op 1 kills the worker three times.
        for i in 0..3 {
            let opened = s.record_crash(&keys[1], "SIGSEGV");
            assert_eq!(opened, i == 2, "the circuit opens exactly on the threshold");
        }
        assert_eq!(
            s.plan_circuit_open(&keys).map(|(k, _)| k),
            Some(keys[1].clone()),
            "the poisoned plan now fails fast"
        );

        // The user edits op 1's parameters: same record, same timeline base, new
        // prefix hash through that op. THIS is what a base-keyed circuit could never
        // see (VF-M1) — with per-op prefix keys the plan dispatches again.
        let after = plan(&["h0", "h1-fixed"]);
        let fixed = plan_op_keys(&after, FP);
        assert_eq!(fixed[0], keys[0], "an untouched op keeps its key");
        assert_ne!(fixed[1], keys[1], "the edited op mints a new key");
        assert!(
            s.plan_circuit_open(&fixed).is_none(),
            "the edited plan must DISPATCH, not fail fast"
        );

        // A crash message still travels with the still-open original key.
        assert_eq!(s.last_crash(&keys[1]), "SIGSEGV");
    }

    #[test]
    fn a_successful_prepare_still_heals_every_candidate_key() {
        let s = shared(1);
        let keys = plan_op_keys(&plan(&["h0", "h1"]), FP);
        assert!(s.record_crash(&keys[1], "boom"));
        assert!(s.plan_circuit_open(&keys).is_some());
        s.record_success(&keys);
        assert!(s.plan_circuit_open(&keys).is_none());
        assert_eq!(s.poison.lock().unwrap().entries.len(), 0);
    }

    // ── 2. Memory bound ──────────────────────────────────────────────────────
    #[test]
    fn the_entry_map_is_bounded_by_last_touch_eviction() {
        let s = shared(1); // every crash opens its key, so `open` is pruned too
        for i in 0..200 {
            s.record_crash(&format!("h{i}|rec|{FP}"), "boom");
        }
        let p = s.poison.lock().unwrap();
        assert!(
            p.entries.len() <= POISON_MAX_ENTRIES,
            "entries {} exceeded the cap",
            p.entries.len()
        );
        assert!(
            p.open.len() <= POISON_MAX_ENTRIES,
            "the open set must be pruned alongside the entries"
        );
        // Eviction is least-recently-touched, so the newest key survives and the
        // oldest is gone.
        assert!(p.entries.contains_key(&format!("h199|rec|{FP}")));
        assert!(!p.entries.contains_key(&format!("h0|rec|{FP}")));
    }

    #[test]
    fn a_re_crash_refreshes_an_entrys_place_in_the_eviction_order() {
        let s = shared(10); // never open a circuit — this is purely about touch order
        let hot = format!("hot|rec|{FP}");
        s.record_crash(&hot, "boom");
        for i in 0..POISON_MAX_ENTRIES {
            s.record_crash(&format!("cold{i}|rec|{FP}"), "boom");
            s.record_crash(&hot, "boom"); // keep it warm
        }
        let p = s.poison.lock().unwrap();
        assert!(p.entries.len() <= POISON_MAX_ENTRIES);
        assert!(
            p.entries.contains_key(&hot),
            "a repeatedly-crashing key must never be evicted out from under its count"
        );
    }

    // ── 3. Fingerprint flip ──────────────────────────────────────────────────
    #[test]
    fn a_new_worker_fingerprint_prunes_the_keys_it_can_never_match() {
        let s = shared(1);
        let old = plan_op_keys(&plan(&["h0", "h1"]), "occt-793-old");
        let new = plan_op_keys(&plan(&["h0", "h1"]), "occt-794-new");
        for k in &old {
            s.record_crash(k, "boom");
        }
        assert_eq!(s.poison.lock().unwrap().entries.len(), 2);

        s.note_fingerprint("occt-793-old"); // first handshake — same keys, nothing dropped
        assert_eq!(s.poison.lock().unwrap().entries.len(), 2);

        s.note_fingerprint("occt-794-new");
        {
            let p = s.poison.lock().unwrap();
            assert!(
                p.entries.is_empty(),
                "old-fingerprint entries are unreachable"
            );
            assert!(p.open.is_empty(), "and so are their open flags");
        }
        assert!(
            s.plan_circuit_open(&new).is_none(),
            "the rebuilt worker starts clean"
        );

        // An empty fingerprint (disconnected) must never be treated as a flip.
        s.record_crash(&new[0], "boom");
        s.note_fingerprint("");
        assert_eq!(s.poison.lock().unwrap().entries.len(), 1);
    }

    // ── 4. The operator escape hatch ─────────────────────────────────────────
    #[test]
    fn clear_circuit_forgets_counts_and_open_flags() {
        let s = shared(2);
        let keys = plan_op_keys(&plan(&["h0", "h1"]), FP);
        s.record_crash(&keys[0], "boom"); // count 1, not open
        s.record_crash(&keys[1], "boom");
        s.record_crash(&keys[1], "boom"); // open
        assert!(s.plan_circuit_open(&keys).is_some());

        assert_eq!(s.clear_circuit(), 2, "reports how many keys it forgot");
        {
            let p = s.poison.lock().unwrap();
            assert!(p.entries.is_empty());
            assert!(p.open.is_empty());
        }
        assert!(s.plan_circuit_open(&keys).is_none());
        // Counts are forgotten too: the next crash starts from one.
        s.record_crash(&keys[1], "boom");
        assert!(
            s.plan_circuit_open(&keys).is_none(),
            "a cleared key needs the full threshold again"
        );
    }

    // ── 5. Defensive paths ───────────────────────────────────────────────────
    #[test]
    fn a_short_prefix_hash_vector_degrades_to_the_base_hash_without_panicking() {
        // A hand-rolled `PlanRequest` (public fields) that skipped the planner.
        let req = plan(&["h0"]); // two ops, ONE prefix hash
        let keys = plan_op_keys(&req, FP);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], format!("h0|{}|{FP}", req.ops[0].record_id));
        assert_eq!(keys[1], format!("base-hash|{}|{FP}", req.ops[1].record_id));
        // Deterministic: the same request keys identically every time.
        assert_eq!(keys, plan_op_keys(&req, FP));
    }

    #[test]
    fn no_fingerprint_means_no_keys_at_all() {
        // While disconnected the fingerprint is "", so any key minted now could never
        // match the post-reconnect key for the same plan. Keying nothing keeps the
        // circuit from filling with entries the fail-fast gate never reads.
        assert!(plan_op_keys(&plan(&["h0", "h1"]), "").is_empty());
        let s = shared(1);
        assert!(s.plan_circuit_open(&[]).is_none());
    }
}
