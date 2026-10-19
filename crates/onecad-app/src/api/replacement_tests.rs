use super::*;
use crate::api::recovery_identity_tests::pending_copy;
use onecad_core::io::recovery::{autosave_path, marker_path};

async fn dirty_active(root: &Path) -> (AppState, HostContext, DocumentId) {
    let (state, host, _, offer) = pending_copy(root, false).await;
    recover_document(&state, host.clone(), offer.document_id.to_string(), true)
        .await
        .unwrap();
    (state, host, offer.document_id)
}

#[tokio::test]
async fn explicit_discard_removes_prior_recovery_only_after_replacement() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, id) = dirty_active(area.path()).await;
    let ticket = prepare_replacement_discard(&state).await.unwrap();
    assert!(discard_replaced_document(
        &state,
        host.clone(),
        ticket,
        &get_projection(&state).await.unwrap().runtime_session
    )
    .await
    .is_err());
    assert!(autosave_path(area.path(), id).exists());
    let ticket = prepare_replacement_discard(&state).await.unwrap();
    new_document(&state, host.clone()).await.unwrap();
    let current = get_projection(&state).await.unwrap();
    discard_replaced_document(&state, host, ticket, &current.runtime_session)
        .await
        .unwrap();
    assert!(!autosave_path(area.path(), id).exists());
    assert!(!marker_path(area.path(), id).exists());
    assert_eq!(
        get_projection(&state).await.unwrap().runtime_session,
        current.runtime_session
    );
}

#[tokio::test]
async fn cancelled_and_failed_replacement_preserve_prior_recovery() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, id) = dirty_active(area.path()).await;
    let _ticket = prepare_replacement_discard(&state).await.unwrap();
    assert!(open_file_dialog(host.clone()).await.unwrap().is_none());
    assert!(open_document(
        &state,
        host,
        area.path()
            .join("missing.onecad")
            .to_string_lossy()
            .into_owned(),
        None
    )
    .await
    .is_err());
    assert!(autosave_path(area.path(), id).exists());
    assert!(marker_path(area.path(), id).exists());
    assert_eq!(
        state.runtime.lock().await.as_ref().unwrap().document_uuid(),
        id
    );
}

#[tokio::test]
async fn stale_completion_and_pending_offer_cannot_dispose_recovery() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, id) = dirty_active(area.path()).await;
    let ticket = prepare_replacement_discard(&state).await.unwrap();
    new_document(&state, host.clone()).await.unwrap();
    assert!(
        discard_replaced_document(&state, host.clone(), ticket, "stale-session")
            .await
            .is_err()
    );
    let offer = RecoveryOffer {
        document_id: id,
        autosave_path: autosave_path(area.path(), id),
        marker: None,
        modified_ms: 1,
    };
    // A deferred offer always wins over cleanup consent for another session.
    let ticket = ReplacementDiscard {
        document: id,
        session: "previous-session".into(),
    };
    state.set_pending_recovery([offer]);
    let current = get_projection(&state).await.unwrap();
    assert!(
        discard_replaced_document(&state, host, ticket, &current.runtime_session)
            .await
            .is_err()
    );
    assert!(autosave_path(area.path(), id).exists());
}

#[tokio::test]
async fn reopened_dirty_same_uuid_keeps_its_recovery() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, id) = dirty_active(area.path()).await;
    let current = get_projection(&state).await.unwrap();
    let ticket = ReplacementDiscard {
        document: id,
        session: "prior-runtime-same-document".into(),
    };
    assert!(
        discard_replaced_document(&state, host, ticket, &current.runtime_session)
            .await
            .is_err()
    );
    assert!(autosave_path(area.path(), id).exists());
}

#[tokio::test]
async fn queued_autosave_from_replaced_session_is_fenced_before_write() {
    use std::{future::Future, task::Poll};
    let area = tempfile::tempdir().unwrap();
    let (state, host, id) = dirty_active(area.path()).await;
    let ticket = prepare_replacement_discard(&state).await.unwrap();
    let lane = state.persistence.lock().await;
    let autosave = autosave::autosave_current(&state.runtime, area.path(), &state.persistence);
    tokio::pin!(autosave);
    let first = std::future::poll_fn(|context| Poll::Ready(autosave.as_mut().poll(context))).await;
    assert!(
        first.is_pending(),
        "snapshot must be queued behind the held persistence lane"
    );
    new_document(&state, host.clone()).await.unwrap();
    drop(lane);
    assert!(
        autosave.await.is_none(),
        "obsolete snapshot must not write or report success"
    );
    let current = get_projection(&state).await.unwrap();
    discard_replaced_document(&state, host, ticket, &current.runtime_session)
        .await
        .unwrap();
    assert!(!autosave_path(area.path(), id).exists());
    assert!(!marker_path(area.path(), id).exists());
}

#[tokio::test]
async fn close_fences_queued_autosave_and_preserves_unresolved_offers() {
    use std::{future::Future, task::Poll};
    let area = tempfile::tempdir().unwrap();
    let (state, host, id) = dirty_active(area.path()).await;
    let lane = state.persistence.lock().await;
    let autosave = autosave::autosave_current(&state.runtime, area.path(), &state.persistence);
    tokio::pin!(autosave);
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(autosave.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    let close = close_document(&state, host.clone());
    tokio::pin!(close);
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(close.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert!(state.runtime.lock().await.is_none());
    drop(lane);
    assert!(autosave.await.is_none());
    close.await.unwrap();
    assert!(!autosave_path(area.path(), id).exists());
    assert!(!marker_path(area.path(), id).exists());

    let (state, host, _, offer) = pending_copy(area.path(), true).await;
    recover_document(&state, host.clone(), offer.document_id.to_string(), true)
        .await
        .unwrap();
    state.set_pending_recovery([offer.clone()]);
    close_document(&state, host).await.unwrap();
    assert!(offer.autosave_path.exists());
    assert_eq!(state.peek_pending_recovery(offer.document_id), Some(offer));
}

#[test]
fn close_waits_for_admitted_autosave_before_recovery_cleanup() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(admitted_autosave_close_race());
}

async fn admitted_autosave_close_race() {
    use std::{future::Future, task::Poll};
    let area = tempfile::tempdir().unwrap();
    let (state, host, id) = dirty_active(area.path()).await;
    let (release, blocker) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    let blocker_task = tokio::task::spawn_blocking(move || {
        let _ = started.send(());
        blocker.recv().unwrap();
    });
    ready.await.unwrap();
    let autosave = autosave::autosave_current(&state.runtime, area.path(), &state.persistence);
    tokio::pin!(autosave);
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(autosave.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert!(
        state.persistence.try_lock().is_err(),
        "autosave must own the lane before close"
    );
    let close = close_document(&state, host);
    tokio::pin!(close);
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(close.as_mut().poll(cx)))
            .await
            .is_pending(),
        "close must await the admitted writer"
    );
    assert!(state.runtime.lock().await.is_none());
    release.send(()).unwrap();
    blocker_task.await.unwrap();
    assert!(
        autosave.await.is_some(),
        "already admitted autosave must complete"
    );
    close.await.unwrap();
    assert!(!autosave_path(area.path(), id).exists());
    assert!(!marker_path(area.path(), id).exists());
}

#[derive(Default)]
struct ProjectionEvents(std::sync::Mutex<Vec<String>>);
impl crate::host::EventSink for ProjectionEvents {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        if let AppEvent::ProjectionUpdated(projection) = event {
            self.0.lock().unwrap().push(projection.runtime_session);
        }
        Ok(())
    }
}

#[tokio::test]
async fn blocked_close_preserves_new_document_owner_without_late_empty_projection() {
    let area = tempfile::tempdir().unwrap();
    let (state, mut host, _) = dirty_active(area.path()).await;
    assert_new_owner_survives_close(&state, &mut host).await;
}

async fn assert_new_owner_survives_close(state: &AppState, host: &mut HostContext) {
    use std::{future::Future, task::Poll};
    let events = std::sync::Arc::new(ProjectionEvents::default());
    host.events = events.clone();
    let lane = state.persistence.lock().await;
    let close = close_document(state, host.clone());
    tokio::pin!(close);
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(close.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert!(state.runtime.lock().await.is_none());
    new_document(state, host.clone()).await.unwrap();
    let session = get_projection(state).await.unwrap().runtime_session;
    let exporter = state.exporter();
    let worker = state.live_worker();
    events.0.lock().unwrap().clear();
    drop(lane);
    close.await.unwrap();
    assert_eq!(
        get_projection(state).await.unwrap().runtime_session,
        session
    );
    assert!(std::sync::Arc::ptr_eq(&exporter, &state.exporter()));
    if let Some(worker) = worker {
        assert!(!worker.is_retired());
        assert!(state.live_worker().is_some());
    }
    assert!(
        events.0.lock().unwrap().is_empty(),
        "stale close must not publish an empty projection"
    );
}

#[tokio::test]
#[ignore = "requires freshly staged production worker; run with ONECAD_REQUIRE_WORKER=1 and ONECAD_WORKER_PATH"]
async fn real_worker_blocked_close_preserves_replacement_owner() {
    let area = tempfile::tempdir().unwrap();
    let (_, mut host, _, _) = pending_copy(area.path(), false).await;
    let state = AppState::default();
    state.start(host.clone()).unwrap();
    new_document(&state, host.clone()).await.unwrap();
    assert!(
        state
            .readiness()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    assert_new_owner_survives_close(&state, &mut host).await;
    assert!(
        state
            .readiness()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    state.retire_all();
}
