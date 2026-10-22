//! C ABI v3 declarations and static link for the in-process OCCT worker.
//!
//! Contract: `worker/src/capi/onecad_worker.h`, `docs/ios/ARCHITECTURE.md` and
//! `docs/ios/KERNEL.md`. This crate links the bundle `scripts/build-ios-worker.sh`
//! staged (see `build.rs`) and wraps the C entry points in a safe API.
//! Supervision, transport and identity policy live in `onecad-app`.
//!
//! Without `ONECAD_EMBEDDED_WORKER_DIR` the crate is a stub: [`run`] and
//! [`abi_version`] return [`NotLinked`], so host builds of the workspace never
//! need the kernel.

use std::fmt;
use std::os::fd::OwnedFd;
use std::sync::Arc;

#[cfg(onecad_embedded_linked)]
mod ffi;

/// The `ONECAD_WORKER_ABI_VERSION` these declarations were written against.
pub const ABI_VERSION: i32 = 3;

/// `onecad-embedded-kernel.json` of the linked bundle, embedded at build time
/// (empty in a stub build).
const IDENTITY: &str = include_str!(concat!(env!("OUT_DIR"), "/onecad-embedded-kernel.json"));

/// The identity manifest (v2) of the linked bundle, or `None` in a stub build.
#[must_use]
pub fn identity_json() -> Option<&'static str> {
    (!IDENTITY.trim().is_empty()).then_some(IDENTITY)
}

/// Whether this build links the real kernel.
#[must_use]
pub const fn is_linked() -> bool {
    cfg!(onecad_embedded_linked)
}

/// The kernel is not linked into this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotLinked;

impl fmt::Display for NotLinked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "embedded kernel not linked (build with ONECAD_EMBEDDED_WORKER_DIR set to a \
             scripts/build-ios-worker.sh --out directory)",
        )
    }
}

impl std::error::Error for NotLinked {}

/// `onecad_worker_abi_version()` of the linked library.
///
/// # Errors
/// [`NotLinked`] in a stub build.
pub fn abi_version() -> Result<i32, NotLinked> {
    #[cfg(onecad_embedded_linked)]
    {
        Ok(ffi::abi_version())
    }
    #[cfg(not(onecad_embedded_linked))]
    {
        Err(NotLinked)
    }
}

/// Worker log levels (`ONECAD_WORKER_LOG_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    /// The level for an ABI integer. Unknown integers map to `Info`, as the
    /// worker itself does.
    #[must_use]
    pub fn from_abi(level: i32) -> Self {
        match level {
            0 => Self::Error,
            1 => Self::Warn,
            3 => Self::Debug,
            _ => Self::Info,
        }
    }

    #[must_use]
    pub fn to_abi(self) -> i32 {
        match self {
            Self::Error => 0,
            Self::Warn => 1,
            Self::Info => 2,
            Self::Debug => 3,
        }
    }
}

/// `ONECAD_WORKER_JOURNAL_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalKind {
    /// A kernel job was dequeued.
    JobBegin,
    /// An ExecutePlan / PreviewPlan op is about to run.
    PlanOpBegin,
    /// The job's terminal frame was written.
    JobEnd,
}

impl JournalKind {
    /// `None` for an event a newer worker added (ignored, never guessed).
    #[must_use]
    pub fn from_abi(event: i32) -> Option<Self> {
        match event {
            0 => Some(Self::JobBegin),
            1 => Some(Self::PlanOpBegin),
            2 => Some(Self::JobEnd),
            _ => None,
        }
    }
}

/// One journal event with every string copied out of the worker's buffers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEvent {
    pub kind: JournalKind,
    /// The OCW1 request id.
    pub request_id: u64,
    /// The verb (job events) or the opId (`PlanOpBegin`).
    pub name: String,
    pub step_index: u64,
    /// The op's index in `args.ops` (== its `prefixHashes` index).
    pub op_position: u64,
    /// `prefixHashes[op_position]`, else `expectedBaseHash`; empty for job events.
    pub prefix_hash: String,
    /// The hello's `occt.fingerprint`.
    pub occt_fingerprint: String,
}

/// Host callbacks for one [`run`]. Called from worker threads while the run is
/// live and never after it returns; implementations must not block for long
/// and must not call back into the worker. A panic inside a callback is caught
/// at the ABI boundary and that one call is dropped.
pub trait WorkerCallbacks: Send + Sync {
    /// One worker log line (no level prefix, no newline), from any worker
    /// thread. The worker's sink is process-global: while several runs are
    /// live, every run's lines reach the NEWEST run's callbacks.
    fn log(&self, level: LogLevel, message: &str);

    /// Whether [`journal`](Self::journal) is wired at all.
    fn journal_enabled(&self) -> bool {
        false
    }

    /// One journal event, synchronously on the kernel thread, before the named
    /// work runs (`JobEnd`: after the job's terminal frame was written).
    fn journal(&self, event: &JournalEvent) {
        let _ = event;
    }
}

/// What one in-process worker run needs. Both fds are consumed: ownership
/// passes to the worker, which closes each exactly once before [`run`] returns.
#[derive(Debug)]
pub struct RunConfig {
    /// The worker's read end. A socket: `Shutdown` wakes it with `SHUT_RD`.
    pub input: OwnedFd,
    /// The worker's write end; a different fd (`dup` the socket).
    pub output: OwnedFd,
    pub log_level: LogLevel,
    /// Bounded wait for the process-wide OCCT locks, ms; 0 = the 20 s default.
    pub occt_lock_wait_ms: u32,
}

/// Runs the worker's OCW1 loop on the CALLING thread (give it a large stack)
/// until EOF, `Shutdown` or a protocol error AND every lane has finished.
/// Returns the worker's exit code: 0 clean, 2 protocol loss, 3 internal.
///
/// # Errors
/// [`NotLinked`] in a stub build (both fds are closed).
pub fn run(config: RunConfig, callbacks: Arc<dyn WorkerCallbacks>) -> Result<i32, NotLinked> {
    #[cfg(onecad_embedded_linked)]
    {
        Ok(ffi::run(config, callbacks))
    }
    #[cfg(not(onecad_embedded_linked))]
    {
        drop((config, callbacks));
        Err(NotLinked)
    }
}

#[cfg(test)]
mod tests;
