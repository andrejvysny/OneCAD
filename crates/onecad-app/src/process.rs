//! Process-wide services shared by every open document host.
//!
//! One [`AppState`](crate::state::AppState) hosts one open document: its runtime,
//! worker, regen scheduler, autosave loop and persistence lane. What must exist only
//! once per process lives here instead and is shared by `Arc` between hosts:
//!
//! * the **worker pool** — at most one pre-warmed sidecar. It is spawned *bare*: the
//!   restart hook and status forwarder are installed by the host that adopts it, so
//!   a worker warmed while document A was open reports to B when B adopts it;
//! * the **crash-recovery offers** — a startup scan names autosaves no live session
//!   owns, independent of which host later restores one;
//! * the **reservations** — which host owns which `.onecad` path and document
//!   identity, and the open-document cap (see [`crate::reservations`]);
//! * the **maintenance claim** — the import sweep and library seed run once, not once
//!   per host over the same roots.
//!
//! A host built with [`AppState::default`](crate::state::AppState) gets a private
//! instance, so isolated hosts (tests, headless tools) keep their previous behaviour.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};

use onecad_core::ids::DocumentId;
use onecad_core::io::recovery::RecoveryOffer;

use crate::reservations::Reservations;
use crate::worker::{resolve_worker_path, SupervisorConfig, WorkerManager, WorkerState};

/// A worker slot shared with a backend factory, which is built before the host that
/// owns it exists.
pub(crate) type WorkerSlot = Arc<StdMutex<Option<WorkerManager>>>;

/// Identity of one document host within this process.
pub type HostId = u64;

/// Services shared by every document host in one process.
pub struct ProcessServices {
    warm: WorkerSlot,
    pending_recovery: StdMutex<BTreeMap<DocumentId, RecoveryOffer>>,
    maintenance_claimed: AtomicBool,
    next_host: AtomicU64,
    reservations: Arc<Reservations>,
}

impl Default for ProcessServices {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessServices {
    #[must_use]
    pub fn new() -> Self {
        Self {
            warm: Arc::new(StdMutex::new(None)),
            pending_recovery: StdMutex::new(BTreeMap::new()),
            maintenance_claimed: AtomicBool::new(false),
            // 0 is never a host: it names process-level claims (start-screen file ops).
            next_host: AtomicU64::new(1),
            reservations: Arc::new(Reservations::default()),
        }
    }

    /// A fresh identity for a new document host.
    pub fn next_host_id(&self) -> HostId {
        self.next_host.fetch_add(1, Ordering::Relaxed)
    }

    /// Which host owns which open path and document identity.
    #[must_use]
    pub fn reservations(&self) -> &Arc<Reservations> {
        &self.reservations
    }

    /// `true` exactly once per process: the caller runs the process maintenance.
    pub fn claim_maintenance(&self) -> bool {
        !self.maintenance_claimed.swap(true, Ordering::AcqRel)
    }

    pub(crate) fn warm_slot(&self) -> WorkerSlot {
        self.warm.clone()
    }

    /// Spawns the pool's pre-warmed worker unless one already exists or no worker
    /// binary resolves. Must be called from a Tokio context (the supervisor is a
    /// spawned task, and lives on the runtime that called this).
    pub fn prewarm(&self) {
        let mut warm = self.warm.lock().unwrap();
        if warm.is_some() {
            return;
        }
        let Some(path) = resolve_worker_path() else {
            return; // no binary ⇒ PendingBackend lane; nothing to warm.
        };
        *warm = Some(WorkerManager::spawn(SupervisorConfig::production(path)));
        tracing::info!("geometry worker pre-warmed");
    }

    /// The pre-warmed worker awaiting adoption, if any (introspection / tests).
    #[must_use]
    pub fn warm_worker(&self) -> Option<WorkerManager> {
        self.warm.lock().unwrap().clone()
    }

    /// Retires the pre-warmed worker — the process-quit path. Idempotent.
    pub fn retire_warm(&self) {
        let taken = self.warm.lock().unwrap().take();
        if let Some(wm) = taken {
            wm.retire();
        }
    }

    /// The pending-offer map, poison-tolerant.
    ///
    /// A panic while another thread held this lock must not turn `check_recovery`
    /// and `recover_document` into hard panics — that would put the user's only copy
    /// of their work behind an unrecoverable error. Recovering the inner value is
    /// safe here: the map is a plain scan result with no invariant a partial write
    /// could break.
    pub(crate) fn pending_recovery(&self) -> MutexGuard<'_, BTreeMap<DocumentId, RecoveryOffer>> {
        self.pending_recovery
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn pending_recovery_lock(&self) -> &StdMutex<BTreeMap<DocumentId, RecoveryOffer>> {
        &self.pending_recovery
    }
}

/// Takes the pre-warmed worker **iff** it can still become useful. A `Failed` or
/// already-`Retired` warm worker is retired and dropped so the caller cold-spawns a
/// healthy one — adopting a corpse would hand the new document a backend that can
/// never serve a request.
pub(crate) fn take_warm(warm: &WorkerSlot) -> Option<WorkerManager> {
    let candidate = warm.lock().unwrap().take()?;
    match candidate.state() {
        WorkerState::Starting | WorkerState::Ready | WorkerState::Restarting => Some(candidate),
        WorkerState::Failed | WorkerState::Retired => {
            candidate.retire();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintenance_is_claimed_exactly_once_per_process() {
        let services = ProcessServices::new();
        assert!(services.claim_maintenance());
        assert!(!services.claim_maintenance());
        assert!(!services.claim_maintenance());
        assert!(
            ProcessServices::new().claim_maintenance(),
            "a separate process instance claims its own"
        );
    }

    #[test]
    fn host_ids_are_unique_within_one_process() {
        let services = ProcessServices::new();
        let a = services.next_host_id();
        let b = services.next_host_id();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn prewarm_never_stacks_workers_and_retire_warm_is_idempotent() {
        let services = ProcessServices::new();
        services.prewarm();
        let first = services.warm_worker();
        services.prewarm();
        match (&first, services.warm_worker()) {
            (Some(first), Some(second)) => assert!(first.is_same(&second), "no second spawn"),
            (None, None) => {} // no worker binary resolves: nothing to warm.
            other => panic!("prewarm changed the pool unexpectedly: {other:?}"),
        }
        services.retire_warm();
        services.retire_warm();
        assert!(services.warm_worker().is_none());
        assert!(first.is_none_or(|w| w.is_retired()));
    }
}
