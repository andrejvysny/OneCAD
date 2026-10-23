//! Publication and invalidation share the slot lock, so a checked capture cannot revive stale state.
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Serializes disposable-cache eviction with restoration and handoff. Document
/// publication has a separate epoch: dropping rebuildable memory is not a model change.
#[derive(Default)]
pub struct EvictionGeneration(Mutex<u64>);

impl EvictionGeneration {
    pub fn current(&self) -> Result<u64, String> {
        self.0
            .lock()
            .ok()
            .filter(|generation| **generation != u64::MAX)
            .map(|generation| *generation)
            .ok_or_else(|| "Preparation eviction state unavailable; document remains open".into())
    }

    pub fn with_current(&self, expected: u64, apply: impl FnOnce()) -> bool {
        let Ok(generation) = self.0.lock() else {
            return false;
        };
        if *generation == u64::MAX || *generation != expected {
            return false;
        }
        apply();
        true
    }

    pub fn publish(&self, expected: u64, apply: impl FnOnce(bool)) {
        let Ok(generation) = self.0.lock() else {
            return;
        };
        apply(*generation != u64::MAX && *generation == expected);
    }

    pub fn evict(&self, release: impl FnOnce()) {
        // Even exhausted identities may release memory; they cannot admit another preparation.
        let mut generation = self.0.lock().unwrap_or_else(|poisoned| {
            let mut generation = poisoned.into_inner();
            *generation = u64::MAX;
            generation
        });
        *generation = generation.saturating_add(1);
        release();
    }
}

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
    fn eviction_during_execution_rejects_restore_and_keeps_live_allocation_charged() {
        let generation = EvictionGeneration::default();
        let captured = generation.current().unwrap();
        let budget = std::sync::Arc::new(crate::budget::HostBudget::default());
        let allocation = std::sync::Arc::new(budget.reserve(64 << 20).unwrap());
        let executing = allocation.clone();
        let cache = Mutex::new(Some(allocation));
        generation.evict(|| {
            cache.lock().unwrap().take();
        });
        assert_eq!(budget.snapshot().used, 64 << 20);
        assert!(!generation.with_current(captured, || {
            *cache.lock().unwrap() = Some(executing.clone());
        }));
        assert!(cache.lock().unwrap().is_none());
        drop(executing);
        assert_eq!(budget.snapshot().used, 0);
    }

    #[test]
    fn eviction_fences_late_publication_but_allows_new_generation_rebuild() {
        let generation = EvictionGeneration::default();
        let captured = generation.current().unwrap();
        let adopted = Mutex::new(Some("old preparation"));
        let pending = Mutex::new(Some("pending preparation"));
        generation.evict(|| {
            adopted.lock().unwrap().take();
            pending.lock().unwrap().take();
        });
        generation.publish(captured, |current| {
            *pending.lock().unwrap() = current.then_some("late preparation");
        });
        assert!(adopted.lock().unwrap().is_none());
        assert!(pending.lock().unwrap().is_none());
        let rebuilt = generation.current().unwrap();
        assert_ne!(captured, rebuilt);
        assert!(generation.with_current(rebuilt, || {
            *adopted.lock().unwrap() = Some("rebuilt preparation");
        }));
        assert_eq!(*adopted.lock().unwrap(), Some("rebuilt preparation"));
    }

    #[test]
    fn exhaustion_and_poison_refuse_rebuild_without_reusing_an_identity() {
        let generation = EvictionGeneration(Mutex::new(u64::MAX - 1));
        generation.evict(|| {});
        assert!(generation.current().is_err());
        assert!(!generation.with_current(u64::MAX, || panic!("reused identity")));
        let _ = std::panic::catch_unwind(|| {
            let _guard = generation.0.lock().unwrap();
            panic!("injected eviction-state poison");
        });
        assert!(generation.current().is_err());
        assert!(!generation.with_current(0, || panic!("poison admitted work")));
        let mut released = false;
        generation.evict(|| released = true);
        assert!(
            released,
            "degraded state may still release disposable owners"
        );
    }
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
