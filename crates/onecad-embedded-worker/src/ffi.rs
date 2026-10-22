//! The raw C ABI v3 (`worker/src/capi/onecad_worker.h`) and its trampolines.
//! Only compiled when the bundle is linked (`cfg(onecad_embedded_linked)`).

use std::ffi::{c_char, c_int, c_void, CStr};
use std::os::fd::IntoRawFd;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use crate::{JournalEvent, JournalKind, LogLevel, RunConfig, WorkerCallbacks};

/// `onecad_worker_journal_event`.
#[repr(C)]
pub(crate) struct RawJournalEvent {
    pub(crate) struct_size: u32,
    pub(crate) event: c_int,
    pub(crate) request_id: u64,
    pub(crate) name: *const c_char,
    pub(crate) step_index: u64,
    pub(crate) op_position: u64,
    pub(crate) prefix_hash: *const c_char,
    pub(crate) occt_fingerprint: *const c_char,
}

type LogFn = unsafe extern "C" fn(level: c_int, msg: *const c_char, ctx: *mut c_void);
type JournalFn = unsafe extern "C" fn(event: *const RawJournalEvent, ctx: *mut c_void);

/// `onecad_worker_config`.
#[repr(C)]
struct RawConfig {
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

extern "C" {
    fn onecad_worker_abi_version() -> c_int;
    fn onecad_worker_run_config(config: *const RawConfig) -> c_int;
}

pub(crate) fn abi_version() -> i32 {
    // SAFETY: no arguments, no state; the library guarantees no unwinding
    // (its entry points are noexcept).
    unsafe { onecad_worker_abi_version() }
}

/// The callbacks object the ctx pointers name: a THIN pointer to the `Arc`
/// (a `dyn` pointer is fat and does not fit `void*`).
type Ctx = Arc<dyn WorkerCallbacks>;

pub(crate) fn run(config: RunConfig, callbacks: Arc<dyn WorkerCallbacks>) -> i32 {
    let journal = callbacks.journal_enabled();
    // Lives on this frame until after the call returns, so both ctx pointers
    // stay valid for every callback the worker can make (it never calls after
    // returning).
    let holder: Ctx = callbacks;
    let ctx = std::ptr::from_ref(&holder).cast_mut().cast::<c_void>();
    let raw = RawConfig {
        struct_size: u32::try_from(std::mem::size_of::<RawConfig>()).expect("config size fits u32"),
        in_fd: config.input.into_raw_fd(),
        out_fd: config.output.into_raw_fd(),
        log_cb: Some(log_trampoline),
        log_ctx: ctx,
        log_level: config.log_level.to_abi(),
        journal_cb: journal.then_some(journal_trampoline as JournalFn),
        journal_ctx: if journal { ctx } else { std::ptr::null_mut() },
        occt_lock_wait_ms: config.occt_lock_wait_ms,
    };
    // SAFETY: `raw` is a fully initialised config of this header's size; both
    // fds were released from their OwnedFd above, so the worker is their only
    // owner and closes each exactly once; `ctx` points at `holder`, alive until
    // after the call; the entry point is noexcept (no unwinding into Rust).
    let code = unsafe { onecad_worker_run_config(&raw) };
    drop(holder);
    code
}

/// `onecad_worker_run_config` with a caller-declared `struct_size`, no
/// callbacks and the fds passed as given: drives the ABI's short-struct rule.
#[cfg(test)]
pub(crate) fn run_config_sized(struct_size: u32, in_fd: c_int, out_fd: c_int) -> c_int {
    let raw = RawConfig {
        struct_size,
        in_fd,
        out_fd,
        log_cb: None,
        log_ctx: std::ptr::null_mut(),
        log_level: LogLevel::Info.to_abi(),
        journal_cb: None,
        journal_ctx: std::ptr::null_mut(),
        occt_lock_wait_ms: 0,
    };
    // SAFETY: `raw` is fully initialised and outlives the call (the library
    // reads at most `struct_size` bytes of it); no callback pointers; the entry
    // point is noexcept. Fd ownership is the caller's business (see the test).
    unsafe { onecad_worker_run_config(&raw) }
}

/// Copies a C string; NULL reads as empty (the ABI never passes NULL, but a
/// production host does not rely on that).
///
/// # Safety
/// `ptr` is NULL or a NUL-terminated string valid for this call.
unsafe fn copy_c_str(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: non-null and NUL-terminated per this function's contract.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

pub(crate) unsafe extern "C" fn log_trampoline(level: c_int, msg: *const c_char, ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    // SAFETY: `ctx` is the `Ctx` `run` keeps alive for the whole call, and
    // `msg` is NULL or a string valid for this call (ABI).
    let (callbacks, text) = unsafe { (&*ctx.cast::<Ctx>(), copy_c_str(msg)) };
    // Never unwind across the C boundary.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        callbacks.log(LogLevel::from_abi(level), &text)
    }));
}

pub(crate) unsafe extern "C" fn journal_trampoline(
    event: *const RawJournalEvent,
    ctx: *mut c_void,
) {
    if ctx.is_null() || event.is_null() {
        return;
    }
    // SAFETY: `ctx` as in `log_trampoline`; `event` is non-null and valid for
    // this call (ABI).
    let (callbacks, raw) = unsafe { (&*ctx.cast::<Ctx>(), &*event) };
    // A smaller struct than ours is an older ABI: its tail would be read out of
    // bounds, so the event is dropped rather than half-read.
    if (raw.struct_size as usize) < std::mem::size_of::<RawJournalEvent>() {
        return;
    }
    let Some(kind) = JournalKind::from_abi(raw.event) else {
        return;
    };
    // SAFETY: the three strings are NULL or valid for this call (ABI).
    let event = unsafe {
        JournalEvent {
            kind,
            request_id: raw.request_id,
            name: copy_c_str(raw.name),
            step_index: raw.step_index,
            op_position: raw.op_position,
            prefix_hash: copy_c_str(raw.prefix_hash),
            occt_fingerprint: copy_c_str(raw.occt_fingerprint),
        }
    };
    let _ = catch_unwind(AssertUnwindSafe(|| callbacks.journal(&event)));
}
