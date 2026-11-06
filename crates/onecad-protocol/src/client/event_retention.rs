//! Shared event storage keeps its transport-wide admission through every receiver.
use std::ops::Deref;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, OnceLock,
};

const MAX_RETAINED_EVENT_BYTES: usize = 4 << 20;
static BUDGET: OnceLock<Arc<Budget>> = OnceLock::new();

#[derive(Debug, Default)]
struct Budget(AtomicUsize);

#[derive(Debug)]
struct Lease {
    budget: Arc<Budget>,
    bytes: usize,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget.0.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[derive(Debug)]
struct Storage<T> {
    value: T,
    _lease: Option<Lease>,
}

/// An immutable event shared by the broadcast ring and receivers. Cloning this
/// handle shares both storage and admission instead of cloning an owned JSON tree.
#[derive(Debug, Clone)]
pub struct RetainedEvent<T>(Arc<Storage<T>>);

impl<T> Deref for RetainedEvent<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0.value
    }
}

impl<T: serde::Serialize> serde::Serialize for RetainedEvent<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.value.serialize(serializer)
    }
}

impl<T> RetainedEvent<T> {
    pub(super) fn untracked(value: T) -> Self {
        Self(Arc::new(Storage {
            value,
            _lease: None,
        }))
    }

    pub(super) fn new(value: T, bytes: usize) -> Option<Self> {
        Self::with_budget(
            value,
            bytes,
            BUDGET.get_or_init(|| Arc::new(Budget::default())).clone(),
        )
    }

    fn with_budget(value: T, bytes: usize, budget: Arc<Budget>) -> Option<Self> {
        let bytes = bytes.checked_add(size_of::<Storage<T>>() + 2 * size_of::<usize>())?;
        budget
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|next| *next <= MAX_RETAINED_EVENT_BYTES)
            })
            .ok()?;
        Some(Self(Arc::new(Storage {
            value,
            _lease: Some(Lease { budget, bytes }),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_requests_keep_shared_event_charge_until_final_receiver_drops() {
        let budget = Arc::new(Budget::default());
        let event =
            RetainedEvent::with_budget(String::from("prior request"), 2 << 20, budget.clone())
                .unwrap();
        let receiver = event.clone();
        let charged = budget.0.load(Ordering::Acquire);
        assert!(Arc::ptr_eq(&event.0, &receiver.0));
        drop(event);
        assert_eq!(budget.0.load(Ordering::Acquire), charged);
        assert!(RetainedEvent::with_budget("next request", 2 << 20, budget.clone()).is_none());
        drop(receiver);
        assert_eq!(budget.0.load(Ordering::Acquire), 0);
        assert!(RetainedEvent::with_budget("retry", 2 << 20, budget).is_some());
    }
}
