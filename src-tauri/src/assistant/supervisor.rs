//! [`AssistantHost`] — the Bun assistant sidecar's lifecycle owner (ADR-0015).
//!
//! Spawns the child with `tokio::process`, forwards its stderr into `tracing`
//! under the target `assistant`, completes the OCAK1 handshake, and supervises
//! it:
//!
//! * **liveness** — `ping` every [`HostConfig::ping_interval`];
//!   `max_missed_pings` consecutive misses ⇒ kill and restart;
//! * **restart** — on exit/crash/kill, reconnect with bounded backoff. A failed
//!   *start* or a **rapid death** (a child that dies within `healthy_threshold`
//!   of becoming ready) both count toward one strike budget; exhausting it ⇒
//!   [`AssistantState::Failed`], because a child that cannot stay up must not be
//!   respawned forever;
//! * **retirement** — [`AssistantHost::retire`] is terminal: it cancels the
//!   handshake, the run loop and any backoff sleep, asks the child to `shutdown`,
//!   and force-kills it after a grace window.
//!
//! What it deliberately does NOT inherit from `crate::worker::manager`: the
//! poison/circuit breaker, the wedged-op wall deadlines, the bundled-manifest
//! verification and the epoch. Those are OCCT-specific — a poison key is built
//! from a plan's history hash, a wedge deadline from a kernel verb, and an epoch
//! exists to fence geometry regen. **The assistant has no epoch**: nothing fences
//! on it, and inventing one would imply a document-state relationship this
//! process does not have (ADR-0018).
//!
//! **Lazy start** (ADR-0015): the app installs an [`AssistantSlot`] at setup and
//! spawns nothing. The first `assistant_start` or bridge call creates the host.
//! An app the user never asks for an assistant in never pays for one, and the
//! child's restarts are entirely decoupled from the geometry worker's.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use onecad_assistant_protocol::{Hello, Principal};
use serde_json::json;
use tokio::sync::Notify;

use super::bridge::{AssistantBridge, BridgeError, BridgeOptions};
use super::provider_gateway::{
    GatewayError, GatewayLimits, ProviderConfig, ProviderGateway, ProviderRegistry,
};

/// The `shutdown` verb (`docs/assistant/wire-protocol.md` §5, host → sidecar).
const VERB_SHUTDOWN: &str = "shutdown";

/// How long a retired host gets to honour its graceful `shutdown` before it is
/// killed. Bounded so a wedged sidecar can never outlive its supervisor.
const RETIRE_GRACE: Duration = Duration::from_millis(500);

/// Largest single stderr line forwarded verbatim; the rest of an over-long line
/// is dropped with a marker, so a runaway log cannot blow up the JSONL sink.
const MAX_HOST_LINE: usize = 8 * 1024;

/// Lifecycle state surfaced to the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantState {
    /// Spawning / connecting for the first time.
    Starting,
    /// Connected and handshook — requests flow.
    Ready,
    /// Died; reconnecting under backoff.
    Restarting,
    /// The flap budget was exhausted. No host until something starts a new one.
    Failed,
    /// **Terminal**: [`AssistantHost::retire`] was called. Sticky.
    Retired,
}

impl AssistantState {
    /// The camelCase spelling the webview sees.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AssistantState::Starting => "starting",
            AssistantState::Ready => "ready",
            AssistantState::Restarting => "restarting",
            AssistantState::Failed => "failed",
            AssistantState::Retired => "retired",
        }
    }

    /// Whether this host can still become ready. A terminal one is replaced
    /// rather than waited on.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, AssistantState::Failed | AssistantState::Retired)
    }
}

/// Tunable supervision policy (production defaults; tests inject fast values).
#[derive(Debug, Clone)]
pub struct HostConfig {
    /// The compiled sidecar.
    pub binary: PathBuf,
    /// Passed as `--app-data-dir`. The child REFUSES to start without it: a
    /// sidecar that invented its own data directory would write the user's chat
    /// history somewhere the app cannot find, back up or delete.
    pub app_data_dir: PathBuf,
    /// Passed as `--log-level`.
    pub log_level: String,
    /// How long one spawn + handshake may take before it counts as a failed start.
    pub handshake_timeout: Duration,
    pub ping_interval: Duration,
    pub ping_timeout: Duration,
    pub max_missed_pings: u32,
    /// Delays between restart attempts, indexed by strike.
    pub backoff: Vec<Duration>,
    /// Strike budget for failed starts + rapid deaths.
    pub max_rapid_deaths: u32,
    /// A child that dies within this window of becoming ready is a flap.
    pub healthy_threshold: Duration,
    /// Per-stream inbound buffer bound (§7).
    pub max_stream_buffer_bytes: usize,
}

impl HostConfig {
    /// Production supervision policy.
    ///
    /// The handshake budget is generous (15 s) on purpose: the child opens a
    /// SQLite store and builds the whole AgentKit object graph before it sends
    /// `hello`, so `hello` doubles as its readiness signal and a tight budget
    /// would report a cold start as a crash.
    #[must_use]
    pub fn production(binary: PathBuf, app_data_dir: PathBuf) -> Self {
        HostConfig {
            binary,
            app_data_dir,
            log_level: super::assistant_log_level(),
            handshake_timeout: Duration::from_secs(15),
            ping_interval: Duration::from_secs(10),
            ping_timeout: Duration::from_secs(5),
            max_missed_pings: 2,
            backoff: vec![
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
            ],
            max_rapid_deaths: 3,
            healthy_threshold: Duration::from_secs(5),
            max_stream_buffer_bytes: super::bridge::DEFAULT_STREAM_BUFFER_BYTES,
        }
    }
}

struct Shared {
    config: HostConfig,
    /// The app's provider holder, shared with [`AssistantSlot`] and read fresh on
    /// every `provider.fetch` (ADR-0017). Deliberately NOT in [`HostConfig`]:
    /// a config is captured once per host, and a user editing their endpoint must
    /// not have to restart the sidecar to be heard.
    provider: Arc<ProviderRegistry>,
    /// The current connection; `None` while (re)starting, failed or retired.
    conn: Mutex<Option<Arc<AssistantBridge>>>,
    state: Mutex<AssistantState>,
    hello: Mutex<Option<Hello>>,
    /// The last failure reason, for the status surface. A "failed" with no
    /// explanation is not a diagnosis.
    last_error: Mutex<Option<String>>,
    /// The ONE cancellation signal. Latched once, never cleared; observed in
    /// every supervisor state so a retired host can never reconnect.
    retired: AtomicBool,
    /// Wakes every await parked on [`Shared::retired`] the instant it latches —
    /// the supervisor is otherwise inside a handshake, a select, or a sleep.
    retire_signal: Notify,
    /// Set when the supervision task returns, i.e. the host reached a terminal
    /// state AND its child was reaped.
    torn_down: AtomicBool,
}

impl Shared {
    fn set_state(&self, state: AssistantState) {
        let mut current = self.state.lock().expect("assistant state poisoned");
        if *current == AssistantState::Retired {
            return; // terminal and sticky: nothing overwrites a retirement.
        }
        *current = state;
    }

    fn is_retired(&self) -> bool {
        self.retired.load(Ordering::SeqCst)
    }

    fn latch_retired(&self) -> bool {
        !self.retired.swap(true, Ordering::SeqCst)
    }

    /// Resolves as soon as retirement is latched — immediately if it already is.
    /// The waiter is enabled BEFORE the flag is re-read and `retire` stores
    /// before it notifies, so a wakeup can never be lost.
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

    /// A backoff sleep that wakes early on retirement. `true` ⇒ retired.
    async fn sleep_or_retired(&self, delay: Duration) -> bool {
        tokio::select! {
            biased;
            () = self.retired() => true,
            () = tokio::time::sleep(delay) => self.is_retired(),
        }
    }

    fn note_error(&self, reason: &str) {
        *self
            .last_error
            .lock()
            .expect("assistant error slot poisoned") = Some(reason.to_string());
    }
}

/// The assistant sidecar's lifecycle owner (see the module docs).
#[derive(Clone)]
pub struct AssistantHost {
    shared: Arc<Shared>,
}

impl AssistantHost {
    /// Spawns the supervisor for `config` and returns immediately; the child is
    /// started and handshaken asynchronously. Must be called from a Tokio context.
    #[must_use]
    pub fn spawn(config: HostConfig, provider: Arc<ProviderRegistry>) -> Self {
        let shared = Arc::new(Shared {
            config,
            provider,
            conn: Mutex::new(None),
            state: Mutex::new(AssistantState::Starting),
            hello: Mutex::new(None),
            last_error: Mutex::new(None),
            retired: AtomicBool::new(false),
            retire_signal: Notify::new(),
            torn_down: AtomicBool::new(false),
        });
        tokio::spawn(supervise(shared.clone()));
        AssistantHost { shared }
    }

    /// The current lifecycle state.
    #[must_use]
    pub fn state(&self) -> AssistantState {
        *self.shared.state.lock().expect("assistant state poisoned")
    }

    /// The child's `hello` once connected (pid + versions).
    #[must_use]
    pub fn hello(&self) -> Option<Hello> {
        self.shared
            .hello
            .lock()
            .expect("assistant hello poisoned")
            .clone()
    }

    /// The last failure reason, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<String> {
        self.shared
            .last_error
            .lock()
            .expect("assistant error slot poisoned")
            .clone()
    }

    /// The live connection, or `None` while starting / restarting / terminal.
    #[must_use]
    pub fn bridge(&self) -> Option<Arc<AssistantBridge>> {
        self.shared
            .conn
            .lock()
            .expect("assistant conn poisoned")
            .clone()
    }

    /// Awaits [`AssistantState::Ready`] up to `timeout`; `false` on a terminal
    /// state or on timeout.
    pub async fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match self.state() {
                AssistantState::Ready => return true,
                state if state.is_terminal() => return false,
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// The live connection, waiting up to `timeout` for the first handshake.
    pub async fn connected(&self, timeout: Duration) -> Result<Arc<AssistantBridge>, BridgeError> {
        if self.wait_ready(timeout).await {
            if let Some(bridge) = self.bridge() {
                return Ok(bridge);
            }
        }
        Err(BridgeError::NotRunning(self.last_error().unwrap_or_else(
            || format!("state {}", self.state().as_str()),
        )))
    }

    /// **Terminal, idempotent retirement.**
    ///
    /// Latches the one cancellation flag, which is observed in every supervisor
    /// state — the handshake, the run loop, and every backoff sleep — so a host
    /// retired while connecting or backing off neither reconnects nor restarts.
    /// Teardown is graceful-then-forced: the run loop sends `shutdown` and reaps
    /// the child, killing it after [`RETIRE_GRACE`]; `kill_on_drop` is the
    /// backstop for the paths where no child exists yet.
    ///
    /// Returns immediately; the child is reaped in the background.
    pub fn retire(&self) {
        if !self.shared.latch_retired() {
            return;
        }
        self.shared.set_state(AssistantState::Retired);
        self.shared.retire_signal.notify_waiters();
        tracing::info!(target: "assistant", "assistant host retired");
    }

    /// Whether [`retire`](Self::retire) has been called.
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.shared.is_retired()
    }

    /// Whether the supervision task has finished — terminal state reached AND the
    /// child reaped. `retire` returns before this is true.
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
}

impl std::fmt::Debug for AssistantHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantHost")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The app-scoped slot
// ─────────────────────────────────────────────────────────────────────────────

/// The app's single assistant-host slot.
///
/// App-scoped, NOT document-scoped: it is deliberately outside `BackendBundle`
/// and the two-phase document swap, because the assistant's lifetime has nothing
/// to do with which document is open and it must not inherit machinery that
/// retires a worker when a document changes.
///
/// The slot is configured at setup and **starts nothing**. A host is created on
/// the first [`start`](Self::start) — which is the first `assistant_start` or the
/// first bridge call.
#[derive(Default)]
pub struct AssistantSlot {
    config: OnceLock<HostConfig>,
    host: Mutex<Option<AssistantHost>>,
    /// The configured local provider, if any.
    ///
    /// Separate from `config` and deliberately NOT a `OnceLock`: the host's
    /// binary and supervision policy are fixed at setup, but the provider is the
    /// user's to change at any moment. Every bridge this slot starts reads out of
    /// this same holder, per request, so an edit reaches a RUNNING sidecar
    /// (ADR-0017).
    provider: Arc<ProviderRegistry>,
}

impl AssistantSlot {
    /// An empty, unconfigured slot.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the policy a host is started with. Called once at setup; a second
    /// call is ignored (the first configuration is the one the app booted with).
    pub fn configure(&self, config: HostConfig) {
        let binary = config.binary.clone();
        if self.config.set(config).is_ok() {
            tracing::info!(
                target: "assistant",
                binary = %binary.display(),
                "assistant host available (lazy: no child spawned until first use)"
            );
        }
    }

    /// Whether a binary + data directory were resolved at setup.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.config.get().is_some()
    }

    /// The current host, if one has been started and not replaced.
    #[must_use]
    pub fn host(&self) -> Option<AssistantHost> {
        self.host.lock().expect("assistant slot poisoned").clone()
    }

    /// The running host, starting one if there is none or the last one is
    /// terminal (failed / retired). Must be called from a Tokio context.
    ///
    /// A terminal host is REPLACED rather than revived: `retire` is terminal by
    /// design, so "stop then start" has to mint a new supervisor, and a `Failed`
    /// one is exhausted by definition. Both are the user asking again, which is
    /// the only signal that anything might have changed.
    pub fn start(&self) -> Result<AssistantHost, BridgeError> {
        let Some(config) = self.config.get() else {
            return Err(BridgeError::NotRunning(
                "no assistant host binary is available in this build".into(),
            ));
        };
        let mut slot = self.host.lock().expect("assistant slot poisoned");
        if let Some(existing) = slot.as_ref() {
            if !existing.state().is_terminal() {
                return Ok(existing.clone());
            }
        }
        let host = AssistantHost::spawn(config.clone(), self.provider.clone());
        *slot = Some(host.clone());
        Ok(host)
    }

    /// Installs the local provider OneCAD settings describe, or clears it with
    /// `None`. **The one trusted path by which a provider is registered**
    /// (ADR-0017): its only caller is the settings command, and nothing arriving
    /// over the bridge can reach it.
    ///
    /// The gateway is built BEFORE anything is installed, so a rejected
    /// configuration leaves the working one in place. Silently disarming a
    /// provider that answers because the user mistyped the next one would turn
    /// one readable refusal into "unknown provider" on every later call.
    ///
    /// Takes effect on the next `provider.fetch`, with no restart: the bridge
    /// reads the holder per request.
    pub fn configure_provider(&self, config: Option<ProviderConfig>) -> Result<(), GatewayError> {
        let Some(config) = config else {
            self.provider.install(None);
            tracing::info!(target: "assistant", "assistant provider cleared");
            return Ok(());
        };
        let id = config.id.clone();
        let gateway = ProviderGateway::new(vec![config], GatewayLimits::default())?;
        let origin = gateway
            .base(&id)
            .map(|base| base.origin())
            .unwrap_or_default();
        self.provider.install(Some(Arc::new(gateway)));
        tracing::info!(target: "assistant", provider = %id, %origin, "assistant provider configured");
        Ok(())
    }

    /// The provider holder every bridge this slot starts reads from.
    #[must_use]
    pub fn provider_registry(&self) -> Arc<ProviderRegistry> {
        self.provider.clone()
    }

    /// Retires the running host, if any, and empties the slot. Idempotent.
    pub fn stop(&self) {
        let host = self.host.lock().expect("assistant slot poisoned").take();
        if let Some(host) = host {
            host.retire();
        }
    }
}

impl std::fmt::Debug for AssistantSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantSlot")
            .field("configured", &self.is_configured())
            .field("provider", &self.provider)
            .field("host", &self.host())
            .finish()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Supervisor
// ─────────────────────────────────────────────────────────────────────────────

/// Why the running child stopped.
enum Death {
    /// Exited on its own (clean or crash).
    Exited,
    /// The bridge tore down (fatal frame, EOF) while the process lingered.
    BridgeLost(String),
    /// Killed after `max_missed_pings` unanswered pings.
    PingTimeout,
    /// [`AssistantHost::retire`] latched — terminal, NOT a restart trigger.
    Retired,
}

async fn supervise(shared: Arc<Shared>) {
    supervise_loop(&shared).await;
    // The task is returning, so the child (if any) has been reaped — publish
    // that, because "retired" alone says nothing about whether the process died.
    shared.torn_down.store(true, Ordering::SeqCst);
}

async fn supervise_loop(shared: &Shared) {
    let mut flap_strikes = 0u32;
    loop {
        // Retirement is checked at EVERY point the loop can be entered or
        // resumed: a retired host must never spawn another child.
        if shared.is_retired() {
            return;
        }
        match spawn_and_connect(shared).await {
            Ok((mut child, bridge)) => {
                if shared.is_retired() {
                    terminate_child(&mut child, RETIRE_GRACE).await;
                    return;
                }
                *shared.hello.lock().expect("assistant hello poisoned") =
                    Some(bridge.hello().clone());
                *shared.conn.lock().expect("assistant conn poisoned") = Some(bridge.clone());
                shared.set_state(AssistantState::Ready);
                tracing::info!(
                    target: "assistant",
                    pid = bridge.hello().pid,
                    host_version = %bridge.hello().host_version,
                    "assistant host ready"
                );
                let ready_at = tokio::time::Instant::now();

                let death = run_until_death(shared, &bridge, child).await;
                let alive = ready_at.elapsed();
                *shared.conn.lock().expect("assistant conn poisoned") = None;

                if matches!(death, Death::Retired) {
                    return; // terminal: no restart, no announcement.
                }
                let reason = match &death {
                    Death::Exited => "assistant host exited".to_string(),
                    Death::BridgeLost(why) => format!("assistant bridge lost: {why}"),
                    Death::PingTimeout => "assistant host stopped answering pings".to_string(),
                    Death::Retired => unreachable!("handled above"),
                };
                shared.note_error(&reason);
                shared.set_state(AssistantState::Restarting);
                tracing::warn!(target: "assistant", %reason, "assistant host restarting");

                // A death within `healthy_threshold` of becoming ready is a flap
                // and counts against the budget; a longer-lived child resets it,
                // so an assistant used all day never accumulates strikes.
                let delay = if alive < shared.config.healthy_threshold {
                    flap_strikes += 1;
                    if flap_strikes > shared.config.max_rapid_deaths {
                        shared.set_state(AssistantState::Failed);
                        shared.note_error(&format!(
                            "assistant host died {flap_strikes} times in a row ({reason})"
                        ));
                        tracing::error!(target: "assistant", %reason, "assistant host failed: rapid-death budget exhausted");
                        return;
                    }
                    backoff_delay(shared, flap_strikes)
                } else {
                    flap_strikes = 0;
                    backoff_delay(shared, 1)
                };
                if shared.sleep_or_retired(delay).await {
                    return;
                }
            }
            Err(reason) => {
                // A retirement observed inside `spawn_and_connect` surfaces as a
                // start failure; it must not be counted as a flap.
                if shared.is_retired() {
                    return;
                }
                flap_strikes += 1;
                shared.note_error(&reason);
                shared.set_state(AssistantState::Restarting);
                tracing::warn!(target: "assistant", %reason, "assistant host start failed");
                if flap_strikes > shared.config.max_rapid_deaths {
                    shared.set_state(AssistantState::Failed);
                    shared.note_error(&format!(
                        "assistant host could not start after {flap_strikes} attempts: {reason}"
                    ));
                    tracing::error!(target: "assistant", %reason, "assistant host failed: start budget exhausted");
                    return;
                }
                if shared
                    .sleep_or_retired(backoff_delay(shared, flap_strikes))
                    .await
                {
                    return;
                }
            }
        }
    }
}

fn backoff_delay(shared: &Shared, strike: u32) -> Duration {
    let idx = (strike.max(1) as usize - 1).min(shared.config.backoff.len().saturating_sub(1));
    shared
        .config
        .backoff
        .get(idx)
        .copied()
        .unwrap_or(Duration::from_millis(500))
}

/// Spawns the child and completes the OCAK1 handshake. `Err(reason)` on a spawn
/// or handshake failure — a restart trigger.
async fn spawn_and_connect(
    shared: &Shared,
) -> Result<(tokio::process::Child, Arc<AssistantBridge>), String> {
    let mut cmd = tokio::process::Command::new(&shared.config.binary);
    cmd.arg("--app-data-dir")
        .arg(&shared.config.app_data_dir)
        .arg("--log-level")
        .arg(&shared.config.log_level)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {:?}: {e}", shared.config.binary))?;
    // HARD INVARIANT (inherited from the geometry worker, same reason): the
    // stderr pipe is now OURS, and an unread pipe fills its buffer and BLOCKS the
    // child mid-write. The forwarder is spawned unconditionally, before anything
    // that can fail or await — a `?` below drops `child`, and `kill_on_drop` then
    // ends the forwarder at EOF.
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(forward_host_stderr(stderr));
    }
    let stdout = child.stdout.take().ok_or("child stdout missing")?;
    let stdin = child.stdin.take().ok_or("child stdin missing")?;

    let options = BridgeOptions {
        max_stream_buffer_bytes: shared.config.max_stream_buffer_bytes,
        provider_registry: shared.provider.clone(),
        ..BridgeOptions::default()
    };
    // A child that never sends `hello` would park this await forever, so the
    // handshake is one of the three places retirement MUST be observable — and
    // it carries its own wall deadline besides. On cancellation `child` drops
    // here and `kill_on_drop` reaps it.
    let connected = tokio::select! {
        biased;
        () = shared.retired() => return Err("retired during handshake".into()),
        result = tokio::time::timeout(
            shared.config.handshake_timeout,
            AssistantBridge::connect(stdout, stdin, options),
        ) => result,
    };
    let bridge = match connected {
        Ok(Ok(bridge)) => bridge,
        Ok(Err(err)) => return Err(format!("handshake: {err}")),
        Err(_) => {
            return Err(format!(
                "handshake: no hello within {:?}",
                shared.config.handshake_timeout
            ))
        }
    };
    Ok((child, Arc::new(bridge)))
}

/// Runs the connection until something ends it.
async fn run_until_death(
    shared: &Shared,
    bridge: &Arc<AssistantBridge>,
    mut child: tokio::process::Child,
) -> Death {
    let mut missed = 0u32;
    loop {
        tokio::select! {
            biased;
            () = shared.retired() => {
                // Ask nicely first: the child answers `shutdown` and then exits,
                // so a supervisor that killed it outright could not tell a clean
                // stop from a crash in the exit status.
                let _ = tokio::time::timeout(
                    RETIRE_GRACE,
                    bridge.request(VERB_SHUTDOWN, Principal::Ui, json!({})),
                )
                .await;
                bridge.close("assistant host retired");
                terminate_child(&mut child, RETIRE_GRACE).await;
                return Death::Retired;
            }
            status = child.wait() => {
                bridge.close("assistant host exited");
                match status {
                    Ok(status) => tracing::info!(target: "assistant", code = status.code(), "assistant host exited"),
                    Err(err) => tracing::warn!(target: "assistant", error = %err, "assistant host wait failed"),
                }
                return Death::Exited;
            }
            () = bridge.closed() => {
                // A fatal frame or a closed stdout with the process still alive.
                // There is no resync (§1), so the child is torn down and respawned.
                terminate_child(&mut child, RETIRE_GRACE).await;
                return Death::BridgeLost("frame stream ended".into());
            }
            () = tokio::time::sleep(shared.config.ping_interval) => {
                match tokio::time::timeout(shared.config.ping_timeout, bridge.ping()).await {
                    Ok(Ok(())) => missed = 0,
                    Ok(Err(err)) => {
                        // The bridge is gone; the `closed()` arm will win the next
                        // pass. Recording the reason here keeps the log honest.
                        tracing::debug!(target: "assistant", error = %err, "assistant ping failed");
                    }
                    Err(_) => {
                        missed += 1;
                        tracing::warn!(target: "assistant", missed, "assistant host missed a ping");
                        if missed >= shared.config.max_missed_pings {
                            bridge.close("assistant host stopped answering pings");
                            terminate_child(&mut child, Duration::ZERO).await;
                            return Death::PingTimeout;
                        }
                    }
                }
            }
        }
    }
}

/// Reaps `child`: waits up to `grace` for a self-exit, then kills it. Always
/// returns with the child reaped, so `kill_on_drop` stays a backstop rather than
/// the mechanism.
async fn terminate_child(child: &mut tokio::process::Child, grace: Duration) {
    if !grace.is_zero() && tokio::time::timeout(grace, child.wait()).await.is_ok() {
        return;
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Forwards the child's stderr into `tracing` under the target `assistant`, one
/// event per line, at the level its prefix declares.
///
/// §8: the child's **stdout carries frames only**; every log line goes to stderr.
/// This task is detached and self-terminates at EOF — it is the sole owner of the
/// pipe, so when the child dies the read resolves `Ok(0)`.
async fn forward_host_stderr(stderr: tokio::process::ChildStderr) {
    use tokio::io::AsyncBufReadExt;

    let mut reader = tokio::io::BufReader::new(stderr);
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) => {
                tracing::debug!(target: "assistant", "assistant host stderr closed (EOF)");
                return;
            }
            Ok(_) => {
                let truncated = buf.len() > MAX_HOST_LINE;
                if truncated {
                    buf.truncate(MAX_HOST_LINE);
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
                match sniff_host_level(&line) {
                    HostLine::Error => tracing::error!(target: "assistant", "{line}"),
                    HostLine::Warn => tracing::warn!(target: "assistant", "{line}"),
                    HostLine::Info => tracing::info!(target: "assistant", "{line}"),
                    HostLine::Debug => tracing::debug!(target: "assistant", "{line}"),
                }
            }
            Err(err) => {
                tracing::debug!(target: "assistant", error = %err, "assistant host stderr read error");
                return;
            }
        }
    }
}

enum HostLine {
    Error,
    Warn,
    Info,
    Debug,
}

/// Sniffs the level out of a forwarded line.
///
/// The sidecar's logger writes NDJSON with a `"level"` field
/// (`assistant-host/src/log.ts`); a bare `process.stderr.write` from the startup
/// failure path, from Bun itself, or from a dependency writes plain text.
/// Anything unrecognised is INFO — never silently dropped.
fn sniff_host_level(line: &str) -> HostLine {
    let trimmed = line.trim_start();
    if trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            match value.get("level").and_then(|v| v.as_str()) {
                Some("error") => return HostLine::Error,
                Some("warn") => return HostLine::Warn,
                Some("debug") => return HostLine::Debug,
                Some(_) => return HostLine::Info,
                None => {}
            }
        }
    }
    match trimmed
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches(':')
        .to_ascii_uppercase()
        .as_str()
    {
        "ERROR" => HostLine::Error,
        "WARN" | "WARNING" => HostLine::Warn,
        "DEBUG" => HostLine::Debug,
        _ => HostLine::Info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> HostConfig {
        HostConfig::production(
            PathBuf::from("/nonexistent"),
            PathBuf::from("/tmp/onecad-test"),
        )
    }

    #[test]
    fn state_spellings_are_the_camel_case_the_webview_reads() {
        assert_eq!(AssistantState::Starting.as_str(), "starting");
        assert_eq!(AssistantState::Ready.as_str(), "ready");
        assert_eq!(AssistantState::Restarting.as_str(), "restarting");
        assert_eq!(AssistantState::Failed.as_str(), "failed");
        assert_eq!(AssistantState::Retired.as_str(), "retired");
        assert!(AssistantState::Failed.is_terminal());
        assert!(AssistantState::Retired.is_terminal());
        assert!(!AssistantState::Restarting.is_terminal());
    }

    #[test]
    fn an_unconfigured_slot_starts_nothing_and_says_why() {
        let slot = AssistantSlot::new();
        assert!(!slot.is_configured());
        assert!(slot.host().is_none());
        let err = slot.start().expect_err("no binary, no host");
        assert!(matches!(err, BridgeError::NotRunning(_)), "{err}");
        slot.stop(); // idempotent on an empty slot
    }

    #[tokio::test]
    async fn configuring_the_slot_spawns_nothing_until_start() {
        let slot = AssistantSlot::new();
        slot.configure(config());
        assert!(slot.is_configured());
        // LAZY: configuration alone must not create a supervisor.
        assert!(slot.host().is_none());

        let host = slot.start().expect("configured");
        assert!(slot.host().is_some());
        // The same host is returned while it is not terminal.
        let again = slot.start().expect("configured");
        assert!(Arc::ptr_eq(&host.shared, &again.shared));
    }

    #[tokio::test]
    async fn a_host_whose_binary_does_not_exist_fails_and_is_replaced_by_the_next_start() {
        let slot = AssistantSlot::new();
        slot.configure(HostConfig {
            backoff: vec![Duration::from_millis(1)],
            max_rapid_deaths: 1,
            ..config()
        });
        let host = slot.start().expect("configured");
        // Two failed spawns exhaust the budget; nothing hangs and nothing loops.
        for _ in 0..200 {
            if host.state() == AssistantState::Failed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(host.state(), AssistantState::Failed);
        assert!(host.last_error().expect("a reason").contains("spawn"));
        assert!(!host.wait_ready(Duration::from_millis(20)).await);

        // A terminal host is replaced, not revived.
        let next = slot.start().expect("configured");
        assert!(!Arc::ptr_eq(&host.shared, &next.shared));
        next.retire();
    }

    #[tokio::test]
    async fn retirement_is_terminal_idempotent_and_sticky() {
        let host = AssistantHost::spawn(config(), Arc::new(ProviderRegistry::new()));
        host.retire();
        host.retire();
        assert!(host.is_retired());
        assert_eq!(host.state(), AssistantState::Retired);
        // Nothing can move it out of Retired, including the supervisor's own
        // transitions racing the latch.
        host.shared.set_state(AssistantState::Ready);
        assert_eq!(host.state(), AssistantState::Retired);
        assert!(host.wait_torn_down(Duration::from_secs(2)).await);
        assert!(host.bridge().is_none());
    }

    /// The trusted settings path, at the slot: install, refuse, clear.
    ///
    /// The refusal case is the one that matters. A user who mistypes the next
    /// endpoint must keep the one that works — a failed reconfigure that
    /// disarmed the registry would surface later as "unknown provider", which
    /// names the wrong fault entirely.
    #[test]
    fn configure_provider_installs_refuses_without_disarming_and_clears() {
        let slot = AssistantSlot::new();
        assert!(slot.provider_registry().current().is_none());

        slot.configure_provider(Some(ProviderConfig {
            id: "local".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "qwen3:8b".into(),
            api_key: None,
        }))
        .expect("a loopback provider is installable");
        let installed = slot.provider_registry().current().expect("installed");
        assert_eq!(installed.provider_ids(), vec!["local"]);
        assert_eq!(installed.model("local"), Some("qwen3:8b"));

        let err = slot
            .configure_provider(Some(ProviderConfig {
                id: "local".into(),
                base_url: "http://api.example.com/v1".into(),
                model: "gpt".into(),
                api_key: None,
            }))
            .expect_err("a non-loopback base is refused");
        assert_eq!(err.code(), "internal", "a registration-time refusal: {err}");
        let still = slot
            .provider_registry()
            .current()
            .expect("the working provider survives a refused reconfigure");
        assert_eq!(still.model("local"), Some("qwen3:8b"));

        slot.configure_provider(None).expect("clearing never fails");
        assert!(slot.provider_registry().current().is_none());
    }

    #[test]
    fn backoff_saturates_at_the_last_delay() {
        let shared = Shared {
            config: config(),
            provider: Arc::new(ProviderRegistry::new()),
            conn: Mutex::new(None),
            state: Mutex::new(AssistantState::Starting),
            hello: Mutex::new(None),
            last_error: Mutex::new(None),
            retired: AtomicBool::new(false),
            retire_signal: Notify::new(),
            torn_down: AtomicBool::new(false),
        };
        assert_eq!(backoff_delay(&shared, 1), Duration::from_millis(500));
        assert_eq!(backoff_delay(&shared, 3), Duration::from_secs(2));
        assert_eq!(backoff_delay(&shared, 99), Duration::from_secs(2));
    }

    #[test]
    fn stderr_levels_are_sniffed_from_ndjson_and_from_plain_text() {
        assert!(matches!(
            sniff_host_level(r#"{"level":"error","message":"boom"}"#),
            HostLine::Error
        ));
        assert!(matches!(
            sniff_host_level(r#"{"level":"warn","message":"hm"}"#),
            HostLine::Warn
        ));
        assert!(matches!(
            sniff_host_level(r#"{"level":"debug","message":"trace"}"#),
            HostLine::Debug
        ));
        assert!(matches!(
            sniff_host_level(r#"{"level":"info","message":"ok"}"#),
            HostLine::Info
        ));
        assert!(matches!(
            sniff_host_level("onecad-assistant-host: --app-data-dir is required"),
            HostLine::Info
        ));
        assert!(matches!(
            sniff_host_level("ERROR something broke"),
            HostLine::Error
        ));
        // Not JSON, despite the brace: falls through to the word sniff.
        assert!(matches!(sniff_host_level("{not json"), HostLine::Info));
    }
}
