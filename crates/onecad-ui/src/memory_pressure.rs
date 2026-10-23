//! macOS pressure callbacks wake the host; only its UI thread performs relief.
use std::sync::atomic::{AtomicU8, Ordering};

const ACTIVE: u8 = 1;
const PENDING: u8 = 2;

struct Signal(AtomicU8);

impl Signal {
    fn new() -> Self {
        Self(AtomicU8::new(ACTIVE))
    }

    /// Return true only for the first notification since the UI last drained it.
    fn notify(&self) -> bool {
        self.0
            .compare_exchange(
                ACTIVE,
                ACTIVE | PENDING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn take_pending(&self) -> bool {
        self.0
            .compare_exchange(
                ACTIVE | PENDING,
                ACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn close(&self) {
        self.0.store(0, Ordering::Release);
    }
}

#[cfg(target_os = "macos")]
pub(crate) use native::Monitor;

#[cfg(target_os = "macos")]
mod native {
    use super::Signal;
    use std::{ffi::c_void, ptr::NonNull, sync::Arc};

    type DispatchObject = *mut c_void;
    type Handler = unsafe extern "C" fn(*mut c_void);

    // These stable C APIs are provided by macOS libSystem. Using the function
    // handlers avoids Objective-C blocks and keeps ownership in one boxed value.
    #[link(name = "System")]
    unsafe extern "C" {
        #[link_name = "_dispatch_source_type_memorypressure"]
        static MEMORY_PRESSURE_TYPE: u8;
        fn dispatch_queue_create(
            label: *const std::ffi::c_char,
            attr: *const c_void,
        ) -> DispatchObject;
        fn dispatch_source_create(
            kind: *const c_void,
            handle: usize,
            mask: std::ffi::c_ulong,
            queue: DispatchObject,
        ) -> DispatchObject;
        fn dispatch_set_context(object: DispatchObject, context: *mut c_void);
        fn dispatch_source_set_event_handler_f(source: DispatchObject, handler: Handler);
        fn dispatch_source_set_cancel_handler_f(source: DispatchObject, handler: Handler);
        fn dispatch_resume(object: DispatchObject);
        fn dispatch_source_cancel(source: DispatchObject);
        fn dispatch_release(object: DispatchObject);
    }

    struct Callback {
        signal: Arc<Signal>,
        ctx: egui::Context,
    }

    pub(crate) struct Monitor {
        signal: Arc<Signal>,
        source: NonNull<c_void>,
    }

    impl Monitor {
        pub(crate) fn new(ctx: &egui::Context) -> Result<Self, &'static str> {
            let signal = Arc::new(Signal::new());
            let source = create_source()?;
            let callback = Box::into_raw(Box::new(Callback {
                signal: signal.clone(),
                ctx: ctx.clone(),
            }));
            // SAFETY: The suspended source is exclusively owned here. Its boxed
            // context is installed before activation and freed only by cancel,
            // after all event handlers on its serial queue have completed.
            unsafe {
                dispatch_set_context(source.as_ptr(), callback.cast());
                dispatch_source_set_event_handler_f(source.as_ptr(), notify);
                dispatch_source_set_cancel_handler_f(source.as_ptr(), cancel);
                dispatch_resume(source.as_ptr());
            }
            Ok(Self { signal, source })
        }

        pub(crate) fn take_pending(&self) -> bool {
            self.signal.take_pending()
        }
    }

    fn create_source() -> Result<NonNull<c_void>, &'static str> {
        // SAFETY: The static label is NUL terminated, null attributes select a
        // serial queue, and the source type is an opaque exported symbol whose
        // address (never its value) identifies the memory-pressure source.
        unsafe {
            let queue =
                dispatch_queue_create(c"com.onecad.memory-pressure".as_ptr(), std::ptr::null());
            let queue = NonNull::new(queue).ok_or("memory-pressure queue creation failed")?;
            let source = dispatch_source_create(
                (&raw const MEMORY_PRESSURE_TYPE).cast(),
                0,
                0x2 | 0x4, // DISPATCH_MEMORYPRESSURE_WARN | CRITICAL
                queue.as_ptr(),
            );
            // A successfully created source retains its target queue.
            dispatch_release(queue.as_ptr());
            NonNull::new(source).ok_or("memory-pressure source creation failed")
        }
    }

    impl Drop for Monitor {
        fn drop(&mut self) {
            self.signal.close();
            // SAFETY: This is our single owning source reference; it has been
            // resumed. Cancellation runs asynchronously and keeps the callback
            // context alive until in-flight handlers finish. No callback borrows
            // App or Monitor, and a late repaint holds its own egui context.
            unsafe {
                dispatch_source_cancel(self.source.as_ptr());
                dispatch_release(self.source.as_ptr());
            }
        }
    }

    unsafe extern "C" fn notify(context: *mut c_void) {
        // SAFETY: GCD passes our installed boxed context; cancellation cannot
        // free it until this handler returns on the source's serial queue.
        let callback = unsafe { &*context.cast::<Callback>() };
        if callback.signal.notify() {
            callback.ctx.request_repaint();
        }
    }

    unsafe extern "C" fn cancel(context: *mut c_void) {
        // SAFETY: GCD invokes cancellation once, after event handlers complete.
        // This is the sole reconstruction of the Box transferred at activation.
        drop(unsafe { Box::from_raw(context.cast::<Callback>()) });
    }

    #[cfg(test)]
    mod tests {
        use super::Monitor;
        use std::{sync::Arc, time::Duration, time::Instant};

        #[test]
        fn native_source_registers_and_releases_callback_after_cancel_without_a_gpu() {
            let monitor = Monitor::new(&egui::Context::default()).unwrap();
            let callback_lifetime = Arc::downgrade(&monitor.signal);
            drop(monitor);
            let deadline = Instant::now() + Duration::from_secs(5);
            while callback_lifetime.strong_count() != 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(callback_lifetime.upgrade().is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Signal;
    use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering};

    #[test]
    fn notifications_coalesce_until_the_ui_drains_then_rearm() {
        let signal = Signal::new();
        assert!(!signal.take_pending());
        assert!(signal.notify());
        assert!(!signal.notify());
        assert!(signal.take_pending());
        assert!(!signal.take_pending());
        assert!(signal.notify());
        assert!(signal.take_pending());
    }

    #[test]
    fn concurrent_callbacks_request_one_wakeup() {
        let signal = Signal::new();
        let wakeups = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    if signal.notify() {
                        wakeups.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        });
        assert_eq!(wakeups.load(Ordering::Relaxed), 1);
        assert!(signal.take_pending());
    }

    #[test]
    fn shutdown_discards_pending_and_rejects_late_callbacks() {
        let signal = Signal::new();
        assert!(signal.notify());
        signal.close();
        assert!(!signal.take_pending());
        assert!(!signal.notify());
        assert!(!signal.take_pending());
    }

    #[test]
    fn callback_reference_outlives_owner_without_reopening_signal() {
        let owner = Arc::new(Signal::new());
        let callback = owner.clone();
        let lifetime = Arc::downgrade(&owner);
        owner.close();
        drop(owner);
        assert!(!callback.notify());
        assert!(lifetime.upgrade().is_some());
        drop(callback);
        assert!(lifetime.upgrade().is_none());
    }
}
