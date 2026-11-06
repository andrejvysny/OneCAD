//! Host-independent admission for archive allocations that outlive a request.
use std::sync::Arc;

/// The lifetime class of an archive allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveUse {
    /// Immutable bytes retained by a draft, document, or save payload.
    Retained,
    /// Temporary memory needed while encoding, decoding, or saving.
    Scratch,
}

/// Opaque host-owned admission, held until the allocation is released.
pub type Retention = Arc<dyn Send + Sync>;

/// Synchronous, fail-closed admission before an archive allocation.
pub trait ArchiveAdmission: Send + Sync {
    /// Reserves `bytes` until the returned retention is dropped.
    fn reserve(&self, bytes: usize, purpose: ArchiveUse) -> Result<Retention, String>;
}

/// Uses the host's shared budget when installed; headless callers retain existing caps.
#[derive(Clone, Default)]
pub struct ArchiveBudget(Option<Arc<dyn ArchiveAdmission>>);

impl ArchiveBudget {
    #[must_use]
    pub fn new(admission: Arc<dyn ArchiveAdmission>) -> Self {
        Self(Some(admission))
    }

    /// Reserves before allocating. The returned guard must outlive the bytes.
    pub fn reserve(&self, bytes: usize, purpose: ArchiveUse) -> Result<Retention, String> {
        match &self.0 {
            Some(admission) => admission.reserve(bytes, purpose),
            None => Ok(Arc::new(())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(Arc<AtomicUsize>);
    struct Guard(Arc<AtomicUsize>, usize);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_sub(self.1, Ordering::SeqCst);
        }
    }
    impl ArchiveAdmission for Counting {
        fn reserve(&self, bytes: usize, _: ArchiveUse) -> Result<Retention, String> {
            self.0.fetch_add(bytes, Ordering::SeqCst);
            Ok(Arc::new(Guard(self.0.clone(), bytes)))
        }
    }
    #[test]
    fn cloned_guards_retain_exact_admission_until_last_owner() {
        let total = Arc::new(AtomicUsize::new(0));
        let budget = ArchiveBudget::new(Arc::new(Counting(total.clone())));
        let draft = budget.reserve(123, ArchiveUse::Retained).unwrap();
        let save = draft.clone();
        drop(draft);
        assert_eq!(total.load(Ordering::SeqCst), 123);
        drop(save);
        assert_eq!(total.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn headless_retention_has_no_host_dependency() {
        assert!(ArchiveBudget::default()
            .reserve(10, ArchiveUse::Scratch)
            .is_ok());
    }

    #[test]
    fn source_resolution_refuses_before_read_when_budget_denies() {
        struct Deny;
        impl ArchiveAdmission for Deny {
            fn reserve(&self, _: usize, _: ArchiveUse) -> Result<Retention, String> {
                Err("source budget denied".into())
            }
        }
        let budget = ArchiveBudget::new(Arc::new(Deny));
        let result = crate::library::admitted_component_snapshot_at(
            "test",
            std::path::Path::new("/missing-library"),
            "missing",
            "1",
            None,
            &budget,
        );
        assert!(
            matches!(result, Err(crate::error::ApiError::Io(message)) if message == "source budget denied")
        );
    }
}
