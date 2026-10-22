//! [`InProcessHost`]: the iPadOS backend — the OCCT worker library run on a
//! dedicated thread over a socketpair (ADR-0019; docs/ios/ARCHITECTURE.md
//! "In-process failure policy"; docs/ios/KERNEL.md).
//!
//! * Transport: `UnixStream::pair`, `SO_NOSIGPIPE` on both ends (an iOS host
//!   does not ignore SIGPIPE), the worker end `dup`ed because the ABI wants two
//!   fds, the host end split into tokio halves for the shared handshake.
//! * Thread: one `onecad-worker` thread with an 8 MiB stack per instance; the
//!   worker starts its own lanes (8 MiB each in library builds).
//! * Stop: there is no kill. The host end is shut down (`SHUT_RDWR` on a dup,
//!   so it works whoever holds the tokio halves) and the run's exit is awaited
//!   for a bounded grace; a run that does not return is ABANDONED — counted,
//!   left running, and it latches `relaunch-required` for the whole process:
//!   no further run thread starts and every in-process supervisor fails.
//! * Run cap: at most [`DOCUMENT_RUNS`] document runs (open tabs; the pre-warmed
//!   worker is one of them) and [`AUXILIARY_RUNS`] auxiliary run (Component
//!   Library re-bake / ingest) live at once — counted until the run thread
//!   returns, so retiring and abandoned runs keep their slot. Policy and
//!   rationale: docs/ios/KERNEL.md.

use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use onecad_embedded_worker::{JournalEvent, LogLevel, RunConfig, WorkerCallbacks};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};

use super::backend::{EmbeddedKernelConfig, WorkerRole};
use super::host::{KillOutcome, Launched, PlanPermit, PoisonSeed, WorkerHost, WorkerInstance};
use super::identity::HelloAxes;
use super::kernel_journal::{JournalWriter, KernelJournal};

mod preflight;

/// The run thread's stack (the sidecar's reader runs on an 8 MiB main thread).
const RUN_THREAD_STACK: usize = 8 << 20;

/// How long a force-stopped run gets to return once its socket is shut down.
const KILL_GRACE: Duration = Duration::from_secs(1);

/// Document runs (open tabs, the pre-warmed one among them) live at once:
/// docs/ios/ARCHITECTURE.md's two or three tabs.
const DOCUMENT_RUNS: usize = 3;

/// Auxiliary runs (Component Library re-bake / ingest) live at once.
const AUXILIARY_RUNS: usize = 1;

/// How long a launch waits for a free run slot before it is a start failure.
const SLOT_WAIT: Duration = Duration::from_secs(30);

/// The identity `build.rs` embedded from `ONECAD_EMBEDDED_WORKER_DIR`.
const APP_IDENTITY: &str = include_str!(concat!(env!("OUT_DIR"), "/onecad-embedded-kernel.json"));

/// The identity manifest (v2) this binary verifies every in-process run
/// against: the copy `build.rs` embedded. `None` when the build embedded none
/// (a debug build without `ONECAD_EMBEDDED_WORKER_DIR`). Acceptance evidence
/// reports it so a verifier can compare it with the bundle that was built.
#[must_use]
pub fn embedded_identity_json() -> Option<&'static str> {
    (!APP_IDENTITY.trim().is_empty()).then_some(APP_IDENTITY)
}

/// The identity manifest (v2) of the kernel bundle actually linked into this
/// binary (`onecad-embedded-worker`'s copy); `None` in a stub build.
#[must_use]
pub fn linked_identity_json() -> Option<&'static str> {
    onecad_embedded_worker::identity_json()
}

static LAUNCHED: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);
static ABANDONED: AtomicU64 = AtomicU64::new(0);
/// Set by the first abandonment and never cleared (outside tests): the whole
/// process is `relaunch-required` (ADR-0019 decision 4).
static RELAUNCH_REQUIRED: Mutex<Option<String>> = Mutex::new(None);

/// Process-wide in-process instance counts (diagnostics and tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InProcessCounters {
    /// Run threads started.
    pub launched: u64,
    /// Run threads that have not returned yet (abandoned ones included).
    pub live: u64,
    /// Runs given up on after a stop did not end them within the grace.
    pub abandoned: u64,
}

#[must_use]
pub fn in_process_counters() -> InProcessCounters {
    InProcessCounters {
        launched: LAUNCHED.load(Ordering::SeqCst),
        live: LIVE.load(Ordering::SeqCst),
        abandoned: ABANDONED.load(Ordering::SeqCst),
    }
}

/// Why this process may start no further in-process run, once one was
/// abandoned (diagnostics and status).
#[must_use]
pub fn in_process_relaunch_required() -> Option<String> {
    RELAUNCH_REQUIRED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// The first abandonment's reason wins; later ones only count.
fn latch_relaunch_required(reason: String) {
    let mut latch = RELAUNCH_REQUIRED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    latch.get_or_insert(reason);
}

/// Tests that drill an abandonment clear the latch once the abandoned run has
/// returned, so the next test can launch again.
#[cfg(test)]
pub(crate) fn reset_relaunch_latch() {
    *RELAUNCH_REQUIRED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// The run slots `role` draws from, and their size.
fn run_slots(role: WorkerRole) -> (Arc<Semaphore>, usize, &'static str) {
    static DOCUMENT: OnceLock<Arc<Semaphore>> = OnceLock::new();
    static AUXILIARY: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let (cell, size, name) = match role {
        WorkerRole::Document => (&DOCUMENT, DOCUMENT_RUNS, "document"),
        WorkerRole::Rebake | WorkerRole::Ingest => (&AUXILIARY, AUXILIARY_RUNS, "auxiliary"),
    };
    let slots = cell.get_or_init(|| Arc::new(Semaphore::new(size))).clone();
    (slots, size, name)
}

/// Starts in-process worker runs for one supervisor.
#[derive(Debug)]
pub(crate) struct InProcessHost {
    config: EmbeddedKernelConfig,
    role: WorkerRole,
    journal: Option<Arc<KernelJournal>>,
    /// How long a retired run may take to return before it is abandoned.
    retire_deadline: Duration,
}

impl InProcessHost {
    /// `retire_deadline`: the supervisor's longest §8 op deadline. A retired
    /// run finishes its current op before it returns (nothing can stop it
    /// sooner); one still running past the deadline would have been declared
    /// wedged had it not been retired, so only then is it abandoned.
    pub(crate) fn new(
        config: EmbeddedKernelConfig,
        role: WorkerRole,
        retire_deadline: Duration,
    ) -> Self {
        let journal = config.journal_dir.as_deref().map(KernelJournal::shared);
        Self {
            config,
            role,
            journal,
            retire_deadline,
        }
    }

    /// Everything checkable before a thread exists (`preflight::check`).
    fn preflight(&self) -> Result<Option<HelloAxes>, String> {
        let abi = onecad_embedded_worker::abi_version().map_err(|e| e.to_string())?;
        let inputs = preflight::Inputs {
            abi,
            explicit: self.config.identity.as_deref(),
            app: embedded_identity_json(),
            linked: onecad_embedded_worker::identity_json(),
            journal_dir: self.config.journal_dir.as_deref(),
        };
        preflight::check(&inputs, preflight::Policy::BUILD)
    }

    /// A run slot of this host's role, waiting up to [`SLOT_WAIT`]. Cancel-safe.
    async fn reserve(&self) -> Result<OwnedSemaphorePermit, String> {
        let (slots, size, name) = run_slots(self.role);
        tokio::time::timeout(SLOT_WAIT, slots.acquire_owned())
            .await
            .map_err(|_| {
                format!(
                    "in-process {name} run cap reached ({size} per process); \
                     the runs holding it have not returned"
                )
            })?
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl WorkerHost for InProcessHost {
    async fn launch(&self, epoch: u64) -> Result<Launched, String> {
        if let Some(reason) = in_process_relaunch_required() {
            return Err(reason);
        }
        let expected = self.preflight()?;
        // The only await: a cancelled launch has started nothing.
        let permit = self.reserve().await?;
        if let Some(reason) = in_process_relaunch_required() {
            return Err(reason); // abandoned while this one waited for a slot
        }
        let (host, worker) = UnixStream::pair().map_err(|e| format!("socketpair: {e}"))?;
        set_nosigpipe(&host)?;
        set_nosigpipe(&worker)?;
        let input: OwnedFd = worker.into();
        let output = input.try_clone().map_err(|e| format!("dup: {e}"))?;
        let control = host.try_clone().map_err(|e| format!("dup: {e}"))?;
        let callbacks = Arc::new(InstanceCallbacks {
            epoch,
            journal: self.journal.as_ref().map(|journal| journal.writer(epoch)),
        });
        let run = RunConfig {
            input,
            output,
            log_level: self.config.log_level,
            occt_lock_wait_ms: self.config.occt_lock_wait_ms,
        };
        let done = start_run_thread(run, callbacks, permit)?;
        host.set_nonblocking(true).map_err(|e| e.to_string())?;
        let host = tokio::net::UnixStream::from_std(host).map_err(|e| e.to_string())?;
        let (reader, writer) = host.into_split();
        Ok(Launched {
            reader: Box::new(reader),
            writer: Box::new(writer),
            instance: Box::new(InProcessInstance {
                done,
                exited: false,
                control,
                retire_deadline: self.retire_deadline,
            }),
            expected,
        })
    }

    fn relaunch_required(&self) -> Option<String> {
        in_process_relaunch_required()
    }

    async fn plan_permit(&self, keys: &[String]) -> Option<PlanPermit> {
        match &self.journal {
            Some(journal) => Some(journal.plan_permit(keys).await),
            None => None,
        }
    }

    fn poison_seeds(&self, fingerprint: &str) -> Vec<PoisonSeed> {
        self.journal
            .as_ref()
            .map(|journal| journal.seeds(fingerprint))
            .unwrap_or_default()
    }

    fn poison_healed(&self, keys: &[String]) {
        if let Some(journal) = &self.journal {
            journal.heal(keys);
        }
    }

    fn poison_cleared(&self) {
        if let Some(journal) = &self.journal {
            journal.clear();
        }
    }

    fn journal_write_failures(&self) -> u64 {
        self.journal
            .as_ref()
            .map_or(0, |journal| journal.write_failures())
    }
}

/// Runs the worker on its own thread; the receiver resolves with its exit code.
fn start_run_thread(
    run: RunConfig,
    callbacks: Arc<InstanceCallbacks>,
    permit: OwnedSemaphorePermit,
) -> Result<oneshot::Receiver<i32>, String> {
    let (done_tx, done_rx) = oneshot::channel();
    LAUNCHED.fetch_add(1, Ordering::SeqCst);
    LIVE.fetch_add(1, Ordering::SeqCst);
    let spawned = std::thread::Builder::new()
        .name("onecad-worker".into())
        .stack_size(RUN_THREAD_STACK)
        .spawn(move || {
            let callbacks: Arc<dyn WorkerCallbacks> = callbacks;
            // `NotLinked` cannot happen past preflight; 3 = internal, as the C side.
            let code = onecad_embedded_worker::run(run, callbacks).unwrap_or(3);
            // The slot frees only when the run has really returned.
            drop(permit);
            LIVE.fetch_sub(1, Ordering::SeqCst);
            let _ = done_tx.send(code);
        });
    spawned.map(|_detached| done_rx).map_err(|e| {
        LIVE.fetch_sub(1, Ordering::SeqCst);
        format!("spawn the onecad-worker thread: {e}")
    })
}

fn set_nosigpipe(socket: &UnixStream) -> Result<(), String> {
    let on: libc::c_int = 1;
    // SAFETY: `socket` is a live fd for the duration of the call; the option
    // value points at a c_int on this frame and its length says so.
    let rc = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            std::ptr::from_ref(&on).cast(),
            libc::socklen_t::try_from(std::mem::size_of::<libc::c_int>()).unwrap_or(4),
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("SO_NOSIGPIPE: {}", std::io::Error::last_os_error()))
    }
}

/// One running in-process worker.
struct InProcessInstance {
    done: oneshot::Receiver<i32>,
    exited: bool,
    /// A dup of the host end: shutting it down ends the socket for everyone
    /// (EOF to the worker's reader) whoever holds the tokio halves.
    control: UnixStream,
    retire_deadline: Duration,
}

impl InProcessInstance {
    fn close_socket(&self) {
        let _ = self.control.shutdown(std::net::Shutdown::Both);
    }

    /// Closes the socket and waits up to `grace` for the run to return; a run
    /// that does not is abandoned and latches `relaunch-required`.
    async fn stop(&mut self, grace: Duration, why: &str) -> KillOutcome {
        self.close_socket();
        if tokio::time::timeout(grace, self.exited()).await.is_ok() {
            return KillOutcome::Reaped;
        }
        let abandoned = ABANDONED.fetch_add(1, Ordering::SeqCst) + 1;
        let grace_ms = u64::try_from(grace.as_millis()).unwrap_or(u64::MAX);
        latch_relaunch_required(format!(
            "relaunch-required: an in-process worker was abandoned ({why}, no return \
             {grace_ms} ms after its socket closed); kernel work stops until the app is relaunched"
        ));
        tracing::error!(
            abandoned,
            grace_ms,
            why,
            "in-process worker did not return after its socket closed — abandoned (relaunch required)"
        );
        KillOutcome::Abandoned
    }
}

#[async_trait]
impl WorkerInstance for InProcessInstance {
    async fn exited(&mut self) {
        if self.exited {
            return;
        }
        if let Ok(code) = (&mut self.done).await {
            tracing::debug!(target: "worker", code, "in-process worker returned");
        }
        self.exited = true;
    }

    /// Retirement: the run gets at least the retire deadline (its current op
    /// may legitimately still be running) before it is abandoned.
    async fn terminate(&mut self, grace: Duration) {
        let _ = self.stop(grace.max(self.retire_deadline), "retired").await;
    }

    async fn force_kill(&mut self) -> KillOutcome {
        self.stop(KILL_GRACE, "stopped").await
    }
}

impl Drop for InProcessInstance {
    /// The `kill_on_drop` analogue: a dropped instance (failed or cancelled
    /// handshake) always gets EOF, so its run can return.
    fn drop(&mut self) {
        self.close_socket();
    }
}

/// Routes one instance's log lines to `tracing` and its journal events to the
/// crash journal.
struct InstanceCallbacks {
    epoch: u64,
    journal: Option<JournalWriter>,
}

impl WorkerCallbacks for InstanceCallbacks {
    fn log(&self, level: LogLevel, message: &str) {
        // The worker's sink is process-global (the newest run's callbacks get
        // every run's lines), so `epoch` attributes this INSTANCE's lifetime,
        // not necessarily the line's producer.
        let epoch = self.epoch;
        match level {
            LogLevel::Error => tracing::error!(target: "worker", epoch, "{message}"),
            LogLevel::Warn => tracing::warn!(target: "worker", epoch, "{message}"),
            LogLevel::Info => tracing::info!(target: "worker", epoch, "{message}"),
            LogLevel::Debug => tracing::debug!(target: "worker", epoch, "{message}"),
        }
    }

    fn journal_enabled(&self) -> bool {
        self.journal.is_some()
    }

    fn journal(&self, event: &JournalEvent) {
        if let Some(journal) = &self.journal {
            journal.record(event);
        }
    }
}

#[cfg(test)]
mod tests;
