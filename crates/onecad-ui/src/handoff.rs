//! Publication and invalidation share the slot lock, so a checked capture cannot revive stale state.
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

pub fn with_current<T>(
    slot: &Mutex<Option<T>>,
    epoch: &AtomicU64,
    stopping: &AtomicBool,
    expected: u64,
    apply: impl FnOnce(&mut Option<T>),
) -> bool {
    let Ok(mut slot) = slot.lock() else {
        return false;
    };
    if epoch.load(Ordering::Acquire) != expected || stopping.load(Ordering::Acquire) {
        return false;
    }
    apply(&mut slot);
    true
}
pub fn invalidate<T>(
    slot: &Mutex<Option<T>>,
    epoch: &AtomicU64,
    valid: &AtomicBool,
    after: impl FnOnce(),
) {
    if let Ok(mut slot) = slot.lock() {
        epoch.fetch_add(1, Ordering::AcqRel);
        *slot = None;
        valid.store(false, Ordering::Release);
        after();
    }
}
pub fn take_current<T>(
    slot: &Mutex<Option<(u64, T)>>,
    generation: &AtomicU64,
    valid: &AtomicBool,
    stopping: &AtomicBool,
) -> Option<T> {
    let (expected, value) = slot.lock().ok()?.take()?;
    (generation.load(Ordering::Acquire) == expected
        && valid.load(Ordering::Acquire)
        && !stopping.load(Ordering::Acquire))
    .then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalidation_between_capture_and_publication_rejects_publish_and_accept() {
        let slot = Mutex::new(Some(1));
        let epoch = AtomicU64::new(0);
        let valid = AtomicBool::new(true);
        let stopping = AtomicBool::new(false);
        let captured = epoch.load(Ordering::Acquire);
        invalidate(&slot, &epoch, &valid, || {});
        assert!(!with_current(&slot, &epoch, &stopping, captured, |slot| {
            *slot = Some(2);
            valid.store(true, Ordering::Release);
        }));
        assert!(!with_current(&slot, &epoch, &stopping, captured, |_| {
            valid.store(true, Ordering::Release);
        }));
        assert!(!valid.load(Ordering::Acquire));
        assert_eq!(*slot.lock().unwrap(), None);
    }
    #[test]
    fn completion_enqueued_after_cancel_is_rechecked_on_take() {
        let generation = AtomicU64::new(4);
        let slot = Mutex::new(None);
        let valid = AtomicBool::new(true);
        let stopping = AtomicBool::new(false);
        generation.store(5, Ordering::Release);
        *slot.lock().unwrap() = Some((4, "old result"));
        assert_eq!(take_current(&slot, &generation, &valid, &stopping), None);
        *slot.lock().unwrap() = Some((5, "current result"));
        assert_eq!(
            take_current(&slot, &generation, &valid, &stopping),
            Some("current result")
        );
    }
}
