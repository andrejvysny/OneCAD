//! Cancellation and admission share one linearization point before document mutation.
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

#[derive(Clone, Debug, Default)]
pub struct SketchCandidateControl(Arc<AtomicU8>);

impl SketchCandidateControl {
    /// Returns false once admission owns the candidate. Execution must still finish safely.
    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == 1
    }

    pub(crate) fn admit(&self) -> bool {
        self.0
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_and_admission_have_one_winner() {
        let cancelled = SketchCandidateControl::default();
        assert!(cancelled.clone().cancel());
        assert!(!cancelled.admit());
        let admitted = SketchCandidateControl::default();
        assert!(admitted.admit());
        assert!(!admitted.cancel());
        assert!(!admitted.admit());
    }
}
