//! A staged STEP probe must neither race connection nor strand the prior owner.
use super::{ApiError, AppState};
use std::{sync::Arc, time::Duration};

pub(super) struct ImportBackend<'a> {
    state: &'a AppState,
    committed: bool,
    owner: Arc<()>,
    readiness: Arc<dyn crate::worker::WorkerReadiness>,
    probe: Arc<dyn crate::worker::StepImport>,
}

impl<'a> ImportBackend<'a> {
    pub(super) fn stage(state: &'a AppState) -> (Self, crate::state::RuntimeBackend) {
        let (backend, owner, readiness, probe) = state.make_backend_with_owner();
        (
            Self {
                state,
                committed: false,
                owner,
                readiness,
                probe,
            },
            backend,
        )
    }

    pub(super) async fn wait_ready(&self, timeout: Duration) -> Result<(), ApiError> {
        match tokio::time::timeout(timeout, self.readiness.wait_ready(timeout)).await {
            Ok(true) => Ok(()),
            _ => Err(ApiError::Worker(format!(
                "STEP import worker did not become ready within {} s",
                timeout.as_secs_f64()
            ))),
        }
    }

    pub(super) fn probe(&self) -> &dyn crate::worker::StepImport {
        self.probe.as_ref()
    }

    pub(super) fn publish<T>(mut self, publish: impl FnOnce() -> T) -> Result<T, ApiError> {
        let result = self
            .state
            .commit_backend_if_current(&self.owner, publish)
            .ok_or_else(|| {
                ApiError::Worker("STEP import backend was superseded before publication".into())
            })?;
        self.committed = true;
        Ok(result)
    }
}

impl Drop for ImportBackend<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.state.rollback_backend_if_current(&self.owner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::{get_projection, import_step},
        worker::{PendingBackend, WorkerReadiness},
    };
    use std::{future::Future, sync::Arc, task::Poll};

    struct NeverReady;
    #[async_trait::async_trait]
    impl WorkerReadiness for NeverReady {
        async fn wait_ready(&self, _: Duration) -> bool {
            std::future::pending().await
        }
    }

    fn never_ready_state() -> AppState {
        AppState::new(Arc::new(|| {
            let backend = Arc::new(PendingBackend);
            (
                backend.clone(),
                backend.clone(),
                backend.clone(),
                backend.clone(),
                backend.clone(),
                backend.clone(),
                backend.clone(),
                backend.clone(),
                backend,
                Arc::new(NeverReady),
            )
        }))
    }

    #[tokio::test]
    async fn readiness_timeout_restores_staged_service_ownership() {
        let state = never_ready_state();
        let before = state.exporter();
        let (staged, _) = ImportBackend::stage(&state);
        assert!(staged.wait_ready(Duration::from_millis(10)).await.is_err());
        drop(staged);
        assert!(Arc::ptr_eq(&before, &state.exporter()));
    }

    #[tokio::test]
    async fn cancelled_readiness_restores_prior_document_and_services() {
        let area = tempfile::tempdir().unwrap();
        let (_, host, _, _) =
            crate::api::recovery_identity_tests::pending_copy(area.path(), true).await;
        let state = never_ready_state();
        crate::api::new_document(&state, host.clone())
            .await
            .unwrap();
        let before = get_projection(&state).await.unwrap();
        let services = state.exporter();
        let mut import = Box::pin(import_step(&state, host, "unused.step".into()));
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(import.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        drop(import);
        assert!(Arc::ptr_eq(&services, &state.exporter()));
        assert_eq!(get_projection(&state).await.unwrap(), before);
    }

    #[tokio::test]
    async fn readiness_refusal_restores_prior_document_and_services() {
        let area = tempfile::tempdir().unwrap();
        let (state, host, _, _) =
            crate::api::recovery_identity_tests::pending_copy(area.path(), true).await;
        let before = get_projection(&state).await.unwrap();
        let services = state.exporter();
        assert!(matches!(
            import_step(&state, host, "unused.step".into()).await,
            Err(ApiError::Worker(_))
        ));
        assert!(Arc::ptr_eq(&services, &state.exporter()));
        assert_eq!(get_projection(&state).await.unwrap(), before);
    }
    #[tokio::test]
    async fn superseded_import_guard_cannot_publish_or_rollback_new_owner() {
        let area = tempfile::tempdir().unwrap();
        let (state, host, _, _) =
            crate::api::recovery_identity_tests::pending_copy(area.path(), true).await;
        let (stale, _) = ImportBackend::stage(&state);
        crate::api::new_document(&state, host.clone())
            .await
            .unwrap();
        let newer = get_projection(&state).await.unwrap();
        let services = state.exporter();
        let published = std::cell::Cell::new(false);
        assert!(stale.publish(|| published.set(true)).is_err());
        assert!(!published.get());
        assert!(Arc::ptr_eq(&services, &state.exporter()));
        assert_eq!(get_projection(&state).await.unwrap(), newer);
        let (stale, _) = ImportBackend::stage(&state);
        crate::api::new_document(&state, host).await.unwrap();
        let newer = get_projection(&state).await.unwrap();
        let services = state.exporter();
        drop(stale);
        assert!(Arc::ptr_eq(&services, &state.exporter()));
        assert_eq!(get_projection(&state).await.unwrap(), newer);
    }
    #[tokio::test]
    async fn close_and_shutdown_invalidate_pending_import_publication() {
        let area = tempfile::tempdir().unwrap();
        let (state, host, _, _) =
            crate::api::recovery_identity_tests::pending_copy(area.path(), true).await;
        let (stale, _) = ImportBackend::stage(&state);
        crate::api::close_document(&state, host.clone())
            .await
            .unwrap();
        assert!(stale
            .publish(|| panic!("closed document must not be republished"))
            .is_err());
        assert!(state.runtime.lock().await.is_none());
        crate::api::new_document(&state, host).await.unwrap();
        let before = get_projection(&state).await.unwrap();
        let (stale, _) = ImportBackend::stage(&state);
        state.retire_all();
        assert!(stale
            .publish(|| panic!("shutdown must not admit a delayed import"))
            .is_err());
        assert_eq!(get_projection(&state).await.unwrap(), before);
    }
    #[tokio::test]
    async fn blocked_close_invalidates_import_before_waiting_for_persistence() {
        let area = tempfile::tempdir().unwrap();
        let (state, host, _, _) =
            crate::api::recovery_identity_tests::pending_copy(area.path(), true).await;
        let (stale, _) = ImportBackend::stage(&state);
        let lane = state.persistence.lock().await;
        let mut close = Box::pin(crate::api::close_document(&state, host));
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(close.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(stale
            .publish(|| panic!("an import cannot reopen a closing runtime"))
            .is_err());
        assert!(state.runtime.lock().await.is_none());
        drop(lane);
        close.await.unwrap();
    }
}
