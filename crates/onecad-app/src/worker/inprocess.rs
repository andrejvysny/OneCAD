//! [`InProcessHost`]: the iPadOS backend — the OCCT worker library run on a
//! dedicated thread over a socketpair (ADR-0018; docs/ios/ARCHITECTURE.md
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
//!   left running, never replaced (the supervisor fails `relaunch-required`).
//! * Instance cap: auxiliary instances (Component Library re-bake / ingest) are
//!   limited to ONE per process; document instances are bounded by the open-
//!   document cap. Policy and rationale: docs/ios/KERNEL.md.

use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use onecad_embedded_worker::{JournalEvent, LogLevel, RunConfig, WorkerCallbacks};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};

use super::backend::{EmbeddedKernelConfig, WorkerRole};
use super::host::{KillOutcome, Launched, PoisonSeed, WorkerHost, WorkerInstance};
use super::identity::{parse_embedded_identity, HelloAxes};
use super::kernel_journal::{JournalWriter, KernelJournal};

/// The run thread's stack (the sidecar's reader runs on an 8 MiB main thread).
const RUN_THREAD_STACK: usize = 8 << 20;

/// How long a force-stopped run gets to return once its socket is shut down.
const KILL_GRACE: Duration = Duration::from_secs(1);

/// Auxiliary instances that may run at once in this process.
const AUXILIARY_INSTANCES: usize = 1;

/// How long an auxiliary launch waits for the cap before it is a start failure.
const AUXILIARY_WAIT: Duration = Duration::from_secs(30);

/// The identity `build.rs` embedded from `ONECAD_EMBEDDED_WORKER_DIR`.
const APP_IDENTITY: &str = include_str!(concat!(env!("OUT_DIR"), "/onecad-embedded-kernel.json"));

static LAUNCHED: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);
static ABANDONED: AtomicU64 = AtomicU64::new(0);

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

fn auxiliary_permits() -> Arc<Semaphore> {
    static PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    PERMITS
        .get_or_init(|| Arc::new(Semaphore::new(AUXILIARY_INSTANCES)))
        .clone()
}

/// Starts in-process worker runs for one supervisor.
#[derive(Debug)]
pub(crate) struct InProcessHost {
    config: EmbeddedKernelConfig,
    role: WorkerRole,
    journal: Option<Arc<KernelJournal>>,
}

impl InProcessHost {
    pub(crate) fn new(config: EmbeddedKernelConfig, role: WorkerRole) -> Self {
        let journal = config.journal_dir.as_deref().map(KernelJournal::shared);
        Self {
            config,
            role,
            journal,
        }
    }

    /// The identity text to verify against: the explicit one, else the one
    /// this crate embedded — which must equal the linked bundle's own.
    fn identity_text(&self) -> Result<Option<String>, String> {
        if let Some(explicit) = &self.config.identity {
            return Ok(Some(explicit.clone()));
        }
        let embedded = (!APP_IDENTITY.trim().is_empty()).then(|| APP_IDENTITY.to_string());
        if let (Some(app), Some(linked)) = (&embedded, onecad_embedded_worker::identity_json()) {
            if app != linked {
                return Err(
                    "onecad-app and onecad-embedded-worker embed different kernel \
                            identities (stale build: rebuild with one ONECAD_EMBEDDED_WORKER_DIR)"
                        .into(),
                );
            }
        }
        Ok(embedded)
    }

    /// ABI first, then the identity manifest: everything checkable before a
    /// thread exists. Returns the hello axes the handshake must match.
    fn preflight(&self) -> Result<Option<HelloAxes>, String> {
        let abi = onecad_embedded_worker::abi_version().map_err(|e| e.to_string())?;
        if abi != onecad_embedded_worker::ABI_VERSION {
            return Err(format!(
                "embedded kernel ABI {abi}; this host speaks ABI {}",
                onecad_embedded_worker::ABI_VERSION
            ));
        }
        let Some(text) = self.identity_text()? else {
            if cfg!(debug_assertions) {
                tracing::warn!(
                    "embedded kernel has no identity manifest (debug build): hello unchecked"
                );
                return Ok(None);
            }
            return Err("release build without an embedded kernel identity".into());
        };
        let identity = parse_embedded_identity(&text)?;
        if identity.abi_version != abi {
            return Err(format!(
                "embedded kernel identity names ABI {}, the linked library is ABI {abi}",
                identity.abi_version
            ));
        }
        if !cfg!(debug_assertions) && cfg!(target_os = "ios") && identity.test_hooks {
            return Err("an iOS release must not run a kernel built with test hooks".into());
        }
        Ok(Some(identity.axes()))
    }

    async fn reserve(&self) -> Result<Option<OwnedSemaphorePermit>, String> {
        if self.role == WorkerRole::Document {
            return Ok(None);
        }
        tokio::time::timeout(AUXILIARY_WAIT, auxiliary_permits().acquire_owned())
            .await
            .map_err(|_| {
                "in-process auxiliary worker cap reached (one per process); \
                 another re-bake or ingest is still running"
                    .to_string()
            })?
            .map(Some)
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl WorkerHost for InProcessHost {
    async fn launch(&self, epoch: u64) -> Result<Launched, String> {
        let expected = self.preflight()?;
        let permit = self.reserve().await?;
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
            }),
            expected,
        })
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
    permit: Option<OwnedSemaphorePermit>,
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
}

impl InProcessInstance {
    fn close_socket(&self) {
        let _ = self.control.shutdown(std::net::Shutdown::Both);
    }

    /// Closes the socket and waits up to `grace` for the run to return.
    async fn stop(&mut self, grace: Duration) -> KillOutcome {
        self.close_socket();
        if tokio::time::timeout(grace, self.exited()).await.is_ok() {
            return KillOutcome::Reaped;
        }
        let abandoned = ABANDONED.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::error!(
            abandoned,
            grace_ms = u64::try_from(grace.as_millis()).unwrap_or(u64::MAX),
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

    async fn terminate(&mut self, grace: Duration) {
        let _ = self.stop(grace).await;
    }

    async fn force_kill(&mut self) -> KillOutcome {
        self.stop(KILL_GRACE).await
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
