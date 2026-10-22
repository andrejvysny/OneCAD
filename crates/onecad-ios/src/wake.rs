//! The host's wake callback behind a gate that `destroy` closes. egui and the
//! service threads may request a repaint from any thread; once the gate is closed
//! (and any call in progress has returned) the host is never called again.
use crate::ffi::WakeFn;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::ThreadId;

struct Target {
    wake: WakeFn,
    /// The host's opaque context pointer, carried as an address.
    ctx: usize,
}

pub struct Gate {
    target: Mutex<Option<Target>>,
    owner: ThreadId,
    /// Set while the owner thread runs an egui pass; its repaint requests are
    /// reported through the frame result instead of a wake.
    in_frame: AtomicBool,
}

impl Gate {
    pub fn new(wake: Option<WakeFn>, ctx: *mut c_void, owner: ThreadId) -> Self {
        Self {
            target: Mutex::new(wake.map(|wake| Target {
                wake,
                ctx: ctx as usize,
            })),
            owner,
            in_frame: AtomicBool::new(false),
        }
    }

    pub fn set_in_frame(&self, value: bool) {
        self.in_frame.store(value, Ordering::Release);
    }

    pub fn wake(&self) {
        let in_frame = self.in_frame.load(Ordering::Acquire);
        if in_frame && std::thread::current().id() == self.owner {
            return;
        }
        // Holding the lock while calling keeps `close` from returning mid-call.
        if let Ok(target) = self.target.lock()
            && let Some(target) = target.as_ref()
        {
            (target.wake)(target.ctx as *mut c_void);
        }
    }

    /// After this returns no wake runs or is running.
    pub fn close(&self) {
        if let Ok(mut target) = self.target.lock() {
            *target = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn count(_ctx: *mut c_void) {
        CALLS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn closed_gate_never_calls_the_host_and_frames_suppress_owner_wakes() {
        let gate = Arc::new(Gate::new(
            Some(count),
            std::ptr::null_mut(),
            std::thread::current().id(),
        ));
        gate.set_in_frame(true);
        gate.wake();
        assert_eq!(CALLS.load(Ordering::SeqCst), 0);
        let remote = gate.clone();
        std::thread::spawn(move || remote.wake()).join().unwrap();
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        gate.set_in_frame(false);
        gate.close();
        gate.wake();
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
}
