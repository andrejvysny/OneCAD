//! Exported lifecycle, surface and frame functions of `onecad_ios.h`, plus the
//! guard every export runs through: NULL and thread-affinity checks, struct-size
//! checks, panic containment with poisoning, and the per-thread `last_error`.
use crate::app::{Failure, IosApp, Outcome, RUNTIME_EXIT_GRACE};
use crate::ffi::{self, Status};
use crate::wake::Gate;
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

/// The C side's `onecad_ios_app`; only ever handled by pointer.
#[repr(C)]
pub struct OpaqueApp {
    _private: [u8; 0],
}

/// What an `onecad_ios_app*` points at. Other threads read only `owner`, which
/// never changes after creation; everything mutable lives behind the cells and
/// is touched on the owner thread only.
pub struct Handle {
    owner: ThreadId,
    poisoned: Cell<bool>,
    wake: Arc<Gate>,
    host: Arc<crate::host::Queue>,
    app: RefCell<IosApp>,
}

/// One app per process (`onecad_ios.h`).
static LIVE: AtomicBool = AtomicBool::new(false);
/// A destroyed app's service runtime that was still exiting when destroy
/// returned; the next create waits for it, so two runtimes never share the
/// app-support data.
static PREVIOUS: Mutex<Option<onecad_ui::host::RuntimeExit>> = Mutex::new(None);

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

pub fn fail(failure: Failure) -> Status {
    let text = CString::new(failure.message.replace('\0', "")).unwrap_or_default();
    LAST_ERROR.with(|slot| *slot.borrow_mut() = text);
    failure.status
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic".into());
    format!("Rust panic contained; app poisoned: {detail}")
}

/// # Safety
/// `app` is NULL or a pointer returned by `onecad_ios_app_create` and not yet
/// destroyed.
unsafe fn handle<'a>(app: *const OpaqueApp) -> Outcome<&'a Handle> {
    // SAFETY: by this function's contract the pointer is NULL or a live `Handle`.
    let handle = unsafe { app.cast::<Handle>().as_ref() }
        .ok_or_else(|| Failure::new(Status::InvalidArgument, "app is NULL"))?;
    if std::thread::current().id() != handle.owner {
        return Err(Failure::new(
            Status::WrongThread,
            "call onecad_ios functions on the thread that created the app",
        ));
    }
    Ok(handle)
}

/// Runs `body` against the app with every ABI guard applied.
///
/// # Safety
/// As for [`handle`].
pub unsafe fn with_app(app: *const OpaqueApp, body: impl FnOnce(&mut IosApp) -> Outcome) -> Status {
    // SAFETY: forwarded from this function's contract.
    let handle = match unsafe { handle(app) } {
        Ok(handle) => handle,
        Err(failure) => return fail(failure),
    };
    if handle.poisoned.get() {
        return fail(Failure::new(
            Status::Terminal,
            "app poisoned by an earlier panic",
        ));
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut app = handle
            .app
            .try_borrow_mut()
            .map_err(|_| Failure::new(Status::State, "re-entrant onecad_ios call"))?;
        body(&mut app)
    }));
    match result {
        Ok(Ok(())) => Status::Ok,
        Ok(Err(failure)) => fail(failure),
        Err(payload) => {
            handle.poisoned.set(true);
            fail(Failure::new(Status::Panic, panic_text(payload.as_ref())))
        }
    }
}

/// Checks a struct's leading `struct_size` without reading anything else, so an
/// out-parameter whose other fields are uninitialized is never read.
///
/// # Safety
/// `ptr` is NULL or valid for reading a `u32`.
pub unsafe fn check_size<T>(ptr: *const T) -> Outcome {
    if ptr.is_null() {
        return Err(Failure::new(
            Status::InvalidArgument,
            "struct pointer is NULL",
        ));
    }
    // SAFETY: non-NULL, and every ABI struct starts with `uint32_t struct_size`.
    let size = unsafe { ptr.cast::<u32>().read_unaligned() } as usize;
    let expected = std::mem::size_of::<T>();
    if size < expected {
        return Err(Failure::new(
            Status::InvalidArgument,
            format!("struct_size {size} is smaller than {expected}"),
        ));
    }
    Ok(())
}

/// Reads a struct after checking its leading `struct_size`.
///
/// # Safety
/// `ptr` is NULL or valid for reading a `u32`, and for a whole initialized `T`
/// whenever that `u32` is at least `size_of::<T>()`.
pub unsafe fn read_struct<T: Copy>(ptr: *const T) -> Outcome<T> {
    // SAFETY: forwarded from this function's contract.
    unsafe { check_size(ptr)? };
    // SAFETY: the caller declared at least `size_of::<T>()` readable bytes.
    Ok(unsafe { ptr.read_unaligned() })
}

/// # Safety
/// `ptr` is NULL or a NUL-terminated string valid for the call.
pub unsafe fn utf8(ptr: *const c_char, what: &str) -> Outcome<String> {
    if ptr.is_null() {
        return Err(Failure::new(
            Status::InvalidArgument,
            format!("{what} is NULL"),
        ));
    }
    // SAFETY: non-NULL and NUL-terminated by the caller's contract.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| Failure::new(Status::InvalidArgument, format!("{what} is not UTF-8")))
}

#[unsafe(no_mangle)]
pub extern "C" fn onecad_ios_abi_version() -> u32 {
    ffi::ABI_VERSION
}

/// # Safety
/// `config` points at an `onecad_ios_config` whose strings are valid for the
/// call; `out_app` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_app_create(
    config: *const ffi::Config,
    out_app: *mut *mut OpaqueApp,
) -> Status {
    if out_app.is_null() {
        return fail(Failure::new(Status::InvalidArgument, "out_app is NULL"));
    }
    if LIVE.swap(true, Ordering::AcqRel) {
        return fail(Failure::new(
            Status::State,
            "one onecad_ios app per process",
        ));
    }
    if let Err(failure) = previous_exited() {
        LIVE.store(false, Ordering::Release);
        return fail(failure);
    }
    // SAFETY: forwarded from this function's contract.
    let created = catch_unwind(AssertUnwindSafe(|| unsafe { create(config) }));
    let created = created
        .unwrap_or_else(|payload| Err(Failure::new(Status::Panic, panic_text(payload.as_ref()))));
    match created {
        Ok(handle) => {
            // SAFETY: `out_app` was checked non-NULL and is writable by contract.
            unsafe { out_app.write(Box::into_raw(Box::new(handle)).cast()) };
            Status::Ok
        }
        Err(failure) => {
            LIVE.store(false, Ordering::Release);
            fail(failure)
        }
    }
}

/// Waits (bounded) for a destroyed app's service runtime that had not exited.
fn previous_exited() -> Outcome {
    let mut previous = PREVIOUS
        .lock()
        .map_err(|_| Failure::new(Status::Internal, "previous app state poisoned"))?;
    match previous.as_ref() {
        Some(exit) if !exit.wait(RUNTIME_EXIT_GRACE) => Err(Failure::new(
            Status::State,
            "the previous app's services are still stopping",
        )),
        _ => {
            *previous = None;
            Ok(())
        }
    }
}

/// Closes the wake gate unless creation finishes, also while a panic unwinds,
/// so nothing a failed create already started can call the host.
struct Unfinished(Option<Arc<Gate>>);

impl Unfinished {
    fn finish(mut self) {
        self.0 = None;
    }
}

impl Drop for Unfinished {
    fn drop(&mut self) {
        if let Some(gate) = self.0.take() {
            gate.close();
        }
    }
}

/// # Safety
/// As for `onecad_ios_app_create`'s `config`.
unsafe fn create(config: *const ffi::Config) -> Outcome<Handle> {
    // SAFETY: forwarded from this function's contract.
    let config = unsafe { read_struct(config)? };
    if config.flags != 0 {
        return Err(Failure::new(
            Status::InvalidArgument,
            "flags are reserved (0)",
        ));
    }
    // SAFETY: the config's strings are valid for the call.
    let dir = |ptr, what| unsafe { utf8(ptr, what) }.map(PathBuf::from);
    let layout = onecad_ui::HostLayout {
        documents: dir(config.documents_dir, "documents_dir")?,
        app_support: dir(config.app_support_dir, "app_support_dir")?,
        caches: dir(config.caches_dir, "caches_dir")?,
        temporary: dir(config.tmp_dir, "tmp_dir")?,
    };
    let owner = std::thread::current().id();
    let wake = Arc::new(Gate::new(config.wake, config.wake_ctx, owner));
    let unfinished = Unfinished(Some(wake.clone()));
    let app = IosApp::create(&layout, wake.clone())?;
    unfinished.finish();
    Ok(Handle {
        owner,
        poisoned: Cell::new(false),
        host: app.host.clone(),
        wake,
        app: RefCell::new(app),
    })
}

/// # Safety
/// `app` is NULL or a live app pointer; it is invalid after this returns `OK`
/// or `ERR_PANIC`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_app_destroy(app: *mut OpaqueApp) -> Status {
    // SAFETY: forwarded from this function's contract.
    if let Err(failure) = unsafe { handle(app) } {
        return fail(failure);
    }
    // SAFETY: a live pointer from `create`, checked above; ownership returns here.
    let handle = unsafe { Box::from_raw(app.cast::<Handle>()) };
    let Handle {
        wake, host, app, ..
    } = *handle;
    let result = catch_unwind(AssertUnwindSafe(|| app.into_inner().teardown()));
    // Even after a panic, no host callback or request may outlive destroy.
    wake.close();
    host.close();
    let result = result.map(|exit| {
        if let Some(exit) = exit {
            tracing::warn!("service runtime still exiting after destroy; the next create waits");
            if let Ok(mut previous) = PREVIOUS.lock() {
                *previous = Some(exit);
            }
        }
    });
    LIVE.store(false, Ordering::Release);
    match result {
        Ok(()) => Status::Ok,
        Err(payload) => fail(Failure::new(Status::Panic, panic_text(payload.as_ref()))),
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `desc` points at a surface descriptor
/// whose `ca_metal_layer` the host keeps retained until detach or destroy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_surface_attach(
    app: *mut OpaqueApp,
    desc: *const ffi::SurfaceDesc,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            let desc = read_struct(desc)?;
            state.attach(&desc)
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `desc` is a readable surface descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_surface_resize(
    app: *mut OpaqueApp,
    desc: *const ffi::SurfaceDesc,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_app(app, |state| state.resize(&read_struct(desc)?)) }
}

/// # Safety
/// `app` is NULL or a live app pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_surface_detach(app: *mut OpaqueApp) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            state.detach();
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer; `out` is a writable frame result whose
/// `struct_size` the host has set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_frame(
    app: *mut OpaqueApp,
    time_s: f64,
    out: *mut ffi::FrameResult,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe {
        with_app(app, |state| {
            check_size(out.cast_const())?;
            let result = state.frame(time_s)?;
            // SAFETY: `out` was checked non-NULL and is writable by contract.
            out.write_unaligned(result);
            Ok(())
        })
    }
}

/// # Safety
/// `app` is NULL or a live app pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn onecad_ios_lifecycle(app: *mut OpaqueApp, event: u32) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { with_app(app, |state| state.lifecycle(event)) }
}

/// Never NULL: an empty string when this thread has no failure recorded.
#[unsafe(no_mangle)]
pub extern "C" fn onecad_ios_last_error(_app: *const OpaqueApp) -> *const c_char {
    // The buffer lives in this thread's slot until the next failure replaces it.
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

#[cfg(test)]
#[path = "abi_tests.rs"]
mod tests;
