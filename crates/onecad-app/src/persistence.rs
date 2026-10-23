//! Admitted disk work owns its lane until both I/O and completion have finished.
use std::{future::Future, sync::Arc};
use tokio::{sync::OwnedMutexGuard, task::JoinHandle};

/// Dropping the waiter detaches the coordinator, never its transaction guard.
/// Admission remains cancellable; callers invoke this immediately after admission.
pub(crate) struct Admission {
    _lane: OwnedMutexGuard<()>,
    _claims: Vec<crate::reservations::Claim>,
}

pub(crate) fn spawn<T: Send + 'static, F: Future<Output = T> + Send + 'static>(
    lane: OwnedMutexGuard<()>,
    claims: Vec<crate::reservations::Claim>,
    transaction: impl FnOnce(Arc<Admission>) -> F + Send + 'static,
) -> JoinHandle<T> {
    let admission = Arc::new(Admission {
        _lane: lane,
        _claims: claims,
    });
    tokio::spawn(async move {
        let work = transaction(admission.clone());
        // The blocking writer also retains admission. If runtime shutdown aborts
        // this coordinator, ownership lasts until the actual disk work finishes.
        let _admission = admission;
        work.await
    })
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct Hooks {
    pub writing: Option<(
        tokio::sync::oneshot::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    )>,
    pub completing: Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>,
}

#[cfg(test)]
impl Hooks {
    pub fn split(self) -> (WriteHook, FinishHook) {
        (WriteHook(self.writing), FinishHook(self.completing))
    }
}

#[cfg(test)]
pub(crate) struct WriteHook(
    Option<(
        tokio::sync::oneshot::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    )>,
);

#[cfg(test)]
impl WriteHook {
    pub fn wait(self) {
        if let Some((started, release)) = self.0 {
            let _ = started.send(());
            let _ = release.recv();
        }
    }
}

#[cfg(test)]
pub(crate) struct FinishHook(
    Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>,
);

#[cfg(test)]
impl FinishHook {
    pub async fn wait(self) {
        if let Some((started, release)) = self.0 {
            let _ = started.send(());
            let _ = release.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn aborting_coordinator_retains_the_blocking_writers_lane_and_claims() {
        let lane = Arc::new(tokio::sync::Mutex::new(()));
        let table = Arc::new(crate::reservations::Reservations::default());
        let id = onecad_core::ids::DocumentId::new();
        let claim = table.claim_document(1, id).unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (finished, done) = tokio::sync::oneshot::channel();
        let task = spawn(
            lane.clone().lock_owned().await,
            vec![claim],
            move |admission| async move {
                tokio::task::spawn_blocking(move || {
                    let _ = started.send(());
                    let _ = wait.recv();
                    drop(admission);
                    let _ = finished.send(());
                })
                .await
                .unwrap();
            },
        );
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(lane.try_lock().is_err());
        assert!(table.claim_document(2, id).is_err());
        release.send(()).unwrap();
        done.await.unwrap();
        assert!(lane.try_lock().is_ok());
        assert!(table.claim_document(2, id).is_ok());
    }
}
