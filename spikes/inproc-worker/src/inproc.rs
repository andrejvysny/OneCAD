//! In-process host for `onecad_worker_run` (worker/src/capi/onecad_worker.h):
//! a socketpair with SO_NOSIGPIPE on both ends, the worker loop on a dedicated
//! std::thread, and `onecad-protocol`'s `ProtocolClient` on the host end.
use std::ffi::{c_char, c_int, c_void, CStr};
use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use onecad_protocol::client::ProtocolClient;

use crate::journal::WorkerJournal;

pub type LogFn = extern "C" fn(level: c_int, msg: *const c_char, ctx: *mut c_void);
pub type JournalFn = extern "C" fn(event: *const JournalEvent, ctx: *mut c_void);

/// `onecad_worker_journal_event` (worker/src/capi/onecad_worker.h, ABI 3).
#[repr(C)]
pub struct JournalEvent {
    struct_size: u32,
    event: c_int,
    request_id: u64,
    name: *const c_char,
    step_index: u64,
    op_position: u64,
    prefix_hash: *const c_char,
    occt_fingerprint: *const c_char,
}

/// A journal event with its strings copied out of the worker's buffers.
pub struct JournalEventView {
    pub event: i32,
    pub request_id: u64,
    pub name: String,
    pub step_index: u64,
    pub op_position: u64,
    pub prefix_hash: String,
    pub fingerprint: String,
}

/// `onecad_worker_config` (worker/src/capi/onecad_worker.h, ABI 3).
#[repr(C)]
struct WorkerConfig {
    struct_size: u32,
    in_fd: c_int,
    out_fd: c_int,
    log_cb: Option<LogFn>,
    log_ctx: *mut c_void,
    log_level: c_int,
    journal_cb: Option<JournalFn>,
    journal_ctx: *mut c_void,
    occt_lock_wait_ms: u32,
}

/// The ABI this host was written against (`ONECAD_WORKER_ABI_VERSION`).
pub const EXPECTED_ABI: i32 = 3;

extern "C" {
    fn onecad_worker_abi_version() -> c_int;
    fn onecad_worker_run_config(config: *const WorkerConfig) -> c_int;
    fn onecad_spike_route_cout(cb: Option<LogFn>, ctx: *mut c_void);
    fn onecad_spike_cout_selftest(text: *const c_char);
    fn onecad_spike_hold_occt_globals();
    fn onecad_spike_release_occt_globals();
}

pub const LOG_INFO: c_int = 2;

/// Collected worker log lines (level, text). Shared by every worker in the
/// process because the C++ sink is process-global anyway.
#[derive(Default)]
pub struct LogSink {
    pub lines: Mutex<Vec<(i32, String)>>,
    pub echo: bool,
}

impl LogSink {
    pub fn count_containing(&self, needle: &str) -> usize {
        let lines = self.lines.lock().unwrap();
        lines.iter().filter(|(_, l)| l.contains(needle)).count()
    }
}

extern "C" fn log_cb(level: c_int, msg: *const c_char, ctx: *mut c_void) {
    // SAFETY: `ctx` is `Arc::as_ptr` of a `LogSink` the worker thread keeps
    // alive for the whole `onecad_worker_run` call (see `start`); `msg` is a
    // NUL-terminated string valid for the duration of this call.
    let (sink, text) = unsafe { (&*(ctx as *const LogSink), CStr::from_ptr(msg)) };
    let text = text.to_string_lossy().into_owned();
    if sink.echo {
        eprintln!("  [worker L{level}] {text}");
    }
    sink.lines.lock().unwrap().push((level, text));
}

extern "C" fn journal_cb(event: *const JournalEvent, ctx: *mut c_void) {
    // SAFETY: `ctx` is `Arc::as_ptr` of a `WorkerJournal` the worker thread keeps
    // alive for the whole call (see `start_with`); `event` and its three
    // NUL-terminated strings are valid for the duration of this call (ABI).
    let (journal, ev) = unsafe { (&*(ctx as *const WorkerJournal), &*event) };
    let text = |p: *const c_char| {
        // SAFETY: see above; the ABI never passes NULL for these fields.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    };
    debug_assert!(ev.struct_size as usize >= std::mem::size_of::<JournalEvent>());
    journal.on_event(&JournalEventView {
        event: ev.event,
        request_id: ev.request_id,
        name: text(ev.name),
        step_index: ev.step_index,
        op_position: ev.op_position,
        prefix_hash: text(ev.prefix_hash),
        fingerprint: text(ev.occt_fingerprint),
    });
}

pub fn abi_version() -> i32 {
    // SAFETY: pure function, no arguments.
    unsafe { onecad_worker_abi_version() }
}

/// Installs the std::cout bridge (process-global) into `sink`, which must live
/// for the rest of the process (it is leaked by the caller).
pub fn route_cout(sink: &'static LogSink) {
    // SAFETY: `sink` is 'static, so the pointer outlives every later cout write.
    unsafe { onecad_spike_route_cout(Some(log_cb), sink as *const LogSink as *mut c_void) }
}

/// Drill-only: locks `io::occt_globals_mutex()` from a helper thread and keeps
/// it until `release_occt_globals` — a sibling worker wedged inside a STEP call.
pub fn hold_occt_globals() {
    // SAFETY: no arguments; returns once the helper thread owns the lock.
    unsafe { onecad_spike_hold_occt_globals() }
}

pub fn release_occt_globals() {
    // SAFETY: no arguments; a no-op when the lock is not held.
    unsafe { onecad_spike_release_occt_globals() }
}

pub fn cout_selftest(text: &str) {
    let c = std::ffi::CString::new(text).unwrap();
    // SAFETY: `c` is a valid NUL-terminated string for the call.
    unsafe { onecad_spike_cout_selftest(c.as_ptr()) }
}

fn set_nosigpipe(fd: RawFd) -> std::io::Result<()> {
    let on: c_int = 1;
    // SAFETY: valid fd, valid pointer/len for an int option.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            &on as *const c_int as *const c_void,
            std::mem::size_of::<c_int>() as libc::socklen_t,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// How the worker thread ended, as observed by the host within a deadline.
#[derive(Debug, PartialEq, Eq)]
pub enum ExitObservation {
    Exited(i32),
    /// The thread did not finish in time: an op is wedged. The thread cannot be
    /// stopped; only a process relaunch reclaims it.
    RelaunchRequired,
}

pub struct InProcessWorker {
    pub client: Option<ProtocolClient>,
    pub handshake: Duration,
    done_rx: mpsc::Receiver<i32>,
    thread: Option<JoinHandle<()>>,
}

impl InProcessWorker {
    pub async fn start(sink: Arc<LogSink>, level: c_int) -> std::io::Result<InProcessWorker> {
        Self::start_with(sink, level, None, 0).await
    }

    /// As `start`, plus the worker-side crash journal (journal callback) and the
    /// bounded OCCT-lock wait (`0` = the library default, 20 s).
    pub async fn start_with(
        sink: Arc<LogSink>,
        level: c_int,
        journal: Option<Arc<WorkerJournal>>,
        occt_lock_wait_ms: u32,
    ) -> std::io::Result<InProcessWorker> {
        let t0 = Instant::now();
        let (host, worker) = StdUnixStream::pair()?;
        set_nosigpipe(host.as_raw_fd())?;
        set_nosigpipe(worker.as_raw_fd())?;
        let worker_in: OwnedFd = worker.into();
        let worker_out = worker_in.try_clone()?; // dup(): the ABI wants two fds
        let in_fd = worker_in.into_raw_fd();
        let out_fd = worker_out.into_raw_fd();

        let (done_tx, done_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("onecad-worker".into())
            // The sidecar's reader loop runs on the 8 MiB main thread.
            .stack_size(8 << 20)
            .spawn(move || {
                let cfg = WorkerConfig {
                    struct_size: std::mem::size_of::<WorkerConfig>() as u32,
                    in_fd,
                    out_fd,
                    log_cb: Some(log_cb),
                    log_ctx: Arc::as_ptr(&sink) as *mut c_void,
                    log_level: level,
                    journal_cb: journal.as_ref().map(|_| journal_cb as JournalFn),
                    journal_ctx: journal
                        .as_ref()
                        .map_or(std::ptr::null_mut(), |j| Arc::as_ptr(j) as *mut c_void),
                    occt_lock_wait_ms,
                };
                // SAFETY: the fds are owned by this call from here on (ABI
                // contract); both ctx pointers stay valid because `sink` and
                // `journal` live in this closure until after the call returns.
                let code = unsafe { onecad_worker_run_config(&cfg) };
                let _ = done_tx.send(code);
                drop(sink);
                drop(journal);
            })?;

        host.set_nonblocking(true)?;
        let host = tokio::net::UnixStream::from_std(host)?;
        let (r, w) = host.into_split();
        let client = ProtocolClient::connect(r, w)
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(InProcessWorker {
            client: Some(client),
            handshake: t0.elapsed(),
            done_rx,
            thread: Some(thread),
        })
    }

    pub fn client(&self) -> &ProtocolClient {
        self.client.as_ref().expect("client open")
    }

    /// Sends `Shutdown` (bounded), closes the host end, and waits up to
    /// `deadline` for the worker thread to finish.
    pub async fn shutdown(mut self, deadline: Duration) -> ExitObservation {
        if let Some(client) = self.client.take() {
            let _ = client
                .request_timeout("Shutdown", serde_json::json!({}), deadline)
                .await;
            client.close().await; // drops both socket halves: worker reads EOF
        }
        self.wait_exit(deadline)
    }

    /// Sends `Shutdown` but keeps the host end OPEN while waiting: the worker
    /// must wake its own blocked reader (shutdown(SHUT_RD) in library builds).
    pub async fn shutdown_keep_host_open(&mut self, deadline: Duration) -> ExitObservation {
        let _ = self
            .client()
            .request_timeout("Shutdown", serde_json::json!({}), deadline)
            .await;
        let obs = self.wait_exit(deadline);
        self.close_host().await;
        obs
    }

    /// Closes the host end WITHOUT a Shutdown request (EOF path).
    pub async fn close_host(&mut self) {
        if let Some(client) = self.client.take() {
            client.close().await;
        }
    }

    pub fn wait_exit(&mut self, deadline: Duration) -> ExitObservation {
        match self.done_rx.recv_timeout(deadline) {
            Ok(code) => {
                if let Some(t) = self.thread.take() {
                    let _ = t.join();
                }
                ExitObservation::Exited(code)
            }
            Err(_) => ExitObservation::RelaunchRequired,
        }
    }

    /// Abandons a wedged worker: the JoinHandle is dropped (thread detached).
    pub fn leak(mut self) {
        self.thread.take();
    }
}
