use super::*;
use crate::resource_admission::{ArchiveAdmission, ArchiveBudget, ArchiveUse, Retention};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Default)]
pub(crate) struct Probe(Arc<AtomicUsize>);
struct Charge(Arc<AtomicUsize>, usize);
impl Drop for Charge {
    fn drop(&mut self) {
        self.0.fetch_sub(self.1, Ordering::SeqCst);
    }
}
impl ArchiveAdmission for Probe {
    fn reserve(&self, bytes: usize, _: ArchiveUse) -> Result<Retention, String> {
        self.0
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_add(bytes).filter(|sum| *sum <= 805_306_368)
            })
            .map_err(|_| "replay test admission exceeded host cap".to_string())?;
        Ok(Arc::new(Charge(self.0.clone(), bytes)))
    }
}
pub(crate) fn install_budget(state: &AppState) -> Probe {
    let probe = Probe::default();
    state
        .install_archive_budget(ArchiveBudget::new(Arc::new(probe.clone())))
        .unwrap();
    probe
}

pub(crate) async fn cancel_replay_during_worker_io(
    state: &Arc<AppState>,
    session: &str,
    id: ComponentDefId,
    probe: &Probe,
) {
    let before = get_assembly_projection(state, session).await.unwrap();
    let baseline = probe.0.load(Ordering::SeqCst);
    let owned = state.clone();
    let owned_session = session.to_owned();
    let task = tokio::spawn(async move { regenerate_component(&owned, &owned_session, id).await });
    let worker = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let worker = state
                .component_worker
                .lock()
                .unwrap()
                .as_ref()
                .map(|(_, w)| w.clone());
            if let Some(worker) = worker {
                if worker.test_has_inflight_plan() {
                    break worker;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("production replay must enter worker IO");
    assert!(
        probe.0.load(Ordering::SeqCst) > baseline,
        "owned replay must retain its source and task reservation"
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(worker.wait_torn_down(Duration::from_secs(15)).await);
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if probe.0.load(Ordering::SeqCst) == baseline
                && execution_slot().available_permits() == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("aborted replay must release charges only after its task and worker end");
    assert!(state.component_worker.lock().unwrap().is_none());
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(get_assembly_projection(state, session).await.unwrap()).unwrap()
    );
}
