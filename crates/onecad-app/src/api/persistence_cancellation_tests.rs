//! Autosave ownership and repeated recovery use the same admitted transaction rules.
use super::*;
use onecad_core::io::recovery::{autosave_path, marker_path};
use std::{future::Future, sync::Arc, task::Poll};

async fn queued(mut work: std::pin::Pin<&mut impl Future>) {
    std::future::poll_fn(|cx| {
        assert!(work.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[test]
fn cancelled_autosave_keeps_ownership_until_explicit_discard_can_follow_its_write() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(cancelled_autosave_discard());
}

async fn cancelled_autosave_discard() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, alias, offer) =
        recovery_identity_tests::pending_copy(area.path(), false).await;
    let id = offer.document_id;
    recover_document(&state, host.clone(), id.to_string(), true)
        .await
        .unwrap();
    state.add_claim(state.claim_path(&alias).unwrap());
    let session = get_document_state(&state).await.unwrap().runtime_session;
    let (release, wait) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        let _ = started.send(());
        let _ = wait.recv();
    });
    ready.await.unwrap();
    let mut checkpoint = Box::pin(autosave::checkpoint_state_until(
        &state,
        area.path(),
        Some(&session),
        std::future::pending(),
    ));
    queued(checkpoint.as_mut()).await;
    drop(checkpoint);
    let mut close = Box::pin(discard_and_close_document(&state, host, &session));
    queued(close.as_mut()).await;
    assert!(state.runtime.lock().await.is_none());
    state.release_claims();
    assert!(state.persistence.try_lock().is_err());
    let reservations = state.services().reservations();
    assert!(reservations.claim_document(999, id).is_err());
    assert!(reservations.claim_path(999, &alias).is_err());
    assert!(autosave_path(area.path(), id).exists());
    release.send(()).unwrap();
    blocker.await.unwrap();
    close.await.unwrap();
    assert!(!autosave_path(area.path(), id).exists());
    assert!(!marker_path(area.path(), id).exists());
    assert!(reservations.claim_document(999, id).is_ok());
    assert!(reservations.claim_path(999, &alias).is_ok());
}

fn simulate_crash(state: &AppState, root: &Path, id: DocumentId) -> SessionMarker {
    let mut marker: SessionMarker =
        serde_json::from_slice(&std::fs::read(marker_path(root, id)).unwrap()).unwrap();
    marker.pid = i32::MAX as u32;
    onecad_core::io::recovery::write_marker(root, &marker).unwrap();
    state.release_claims();
    marker
}

#[tokio::test]
async fn migrated_recovery_preserves_original_source_through_save_as_and_two_more_crashes() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, offer) = recovery_identity_tests::pending_copy(area.path(), false).await;
    let id = offer.document_id;
    let original = offer.marker.unwrap().opened_path.unwrap();
    let editable = area.path().join("editable.onecad");
    recover_document_at_path(
        &state,
        host.clone(),
        id.to_string(),
        true,
        Some(editable.clone()),
    )
    .await
    .unwrap();
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &original)
        .is_err());
    let saved = area.path().join("save-as.onecad");
    save_document(
        &state,
        host.clone(),
        Some(saved.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &original)
        .is_err());
    {
        let mut runtime = state.runtime.lock().await;
        let runtime = runtime.as_mut().unwrap();
        let command = upsert_variable_command(&runtime.variables(), "depth", 10.0).unwrap();
        runtime.apply(command).unwrap();
    }
    assert!(matches!(
        autosave::checkpoint_state_until(&state, area.path(), None, std::future::pending()).await,
        autosave::AutosaveAttempt::Written(_)
    ));
    let marker = simulate_crash(&state, area.path(), id);
    assert_eq!(marker.opened_path, Some(saved));
    assert_eq!(marker.source_paths, vec![original.canonicalize().unwrap()]);
    drop(state);

    let replacement = area.path().join("recovered-again.onecad");
    let state = recovery_identity_tests::state();
    assert_eq!(check_recovery(&state, host.clone()).await.unwrap().len(), 1);
    assert!(state.has_pending_recovery_for_path(&original));
    recover_document_at_path(
        &state,
        host.clone(),
        id.to_string(),
        true,
        Some(replacement.clone()),
    )
    .await
    .unwrap();
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &original)
        .is_err());
    assert!(matches!(
        autosave::checkpoint_state_until(&state, area.path(), None, std::future::pending()).await,
        autosave::AutosaveAttempt::Written(_)
    ));
    let marker = simulate_crash(&state, area.path(), id);
    assert!(marker
        .source_paths
        .contains(&original.canonicalize().unwrap()));
    assert_eq!(marker.opened_path, Some(replacement));
    drop(state);

    let state = recovery_identity_tests::state();
    let offers = check_recovery(&state, host.clone()).await.unwrap();
    assert_eq!(offers.len(), 1);
    recover_document(&state, host, id.to_string(), true)
        .await
        .unwrap();
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &original)
        .is_err());
    let guard = state.runtime.lock().await;
    assert!(guard.as_ref().unwrap().is_dirty());
    assert!(guard
        .as_ref()
        .unwrap()
        .variables()
        .iter()
        .any(|variable| variable.name == "depth"));
}

#[tokio::test]
async fn another_host_cannot_discard_recovery_owned_by_a_live_or_inflight_document() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, offer) = recovery_identity_tests::pending_copy(area.path(), false).await;
    let id = offer.document_id;
    recover_document(&state, host.clone(), id.to_string(), true)
        .await
        .unwrap();
    let other = recovery_identity_tests::state_with_services(state.services().clone());
    state.set_pending_recovery([offer.clone()]);
    let before = std::fs::read(&offer.autosave_path).unwrap();
    assert!(matches!(
        recover_document(&other, host.clone(), id.to_string(), false).await,
        Err(ApiError::DuplicateDocument { .. })
    ));
    assert!(matches!(
        recover_document(&state, host, id.to_string(), false).await,
        Err(ApiError::RecoveryPending(_))
    ));
    assert_eq!(std::fs::read(&offer.autosave_path).unwrap(), before);
    assert_eq!(state.peek_pending_recovery(id), Some(offer));
}

struct ClosedProjection(Arc<tokio::sync::Notify>);
impl crate::host::EventSink for ClosedProjection {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        if matches!(event, AppEvent::ProjectionUpdated(projection) if projection.document_id.is_empty())
        {
            self.0.notify_one();
        }
        Ok(())
    }
}

#[test]
fn cancelling_close_after_detachment_still_releases_dirty_recovery_and_old_claims() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(cancelled_close_cleanup());
}

async fn cancelled_close_cleanup() {
    let area = tempfile::tempdir().unwrap();
    let (state, mut host, alias, offer) =
        recovery_identity_tests::pending_copy(area.path(), false).await;
    let id = offer.document_id;
    recover_document(&state, host.clone(), id.to_string(), true)
        .await
        .unwrap();
    state.add_claim(state.claim_path(&alias).unwrap());
    let completed = Arc::new(tokio::sync::Notify::new());
    host.events = Arc::new(ClosedProjection(completed.clone()));
    let (release, wait) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        let _ = started.send(());
        let _ = wait.recv();
    });
    ready.await.unwrap();
    let mut checkpoint = Box::pin(autosave::checkpoint_state_until(
        &state,
        area.path(),
        None,
        std::future::pending(),
    ));
    queued(checkpoint.as_mut()).await;
    let mut close = Box::pin(close_document(&state, host));
    queued(close.as_mut()).await;
    assert!(state.runtime.lock().await.is_none());
    drop(close);
    drop(checkpoint);
    assert!(state
        .services()
        .reservations()
        .claim_document(999, id)
        .is_err());
    release.send(()).unwrap();
    blocker.await.unwrap();
    completed.notified().await;
    let _lane = state.persistence.lock().await;
    let marker: SessionMarker =
        serde_json::from_slice(&std::fs::read(marker_path(area.path(), id)).unwrap()).unwrap();
    assert_eq!(marker.pid, 0);
    assert!(autosave_path(area.path(), id).exists());
    assert!(state
        .services()
        .reservations()
        .claim_document(999, id)
        .is_ok());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &alias)
        .is_ok());
}

#[test]
fn an_open_started_before_shutdown_cannot_publish_after_teardown() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(open_during_shutdown());
}

async fn open_during_shutdown() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, alias, offer) =
        recovery_identity_tests::pending_copy(area.path(), false).await;
    state.take_pending_recovery(offer.document_id);
    let (release, wait) = std::sync::mpsc::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        let _ = started.send(());
        let _ = wait.recv();
    });
    ready.await.unwrap();
    let mut open = Box::pin(open_document(
        &state,
        host.clone(),
        alias.to_string_lossy().into_owned(),
        None,
    ));
    queued(open.as_mut()).await;
    shutdown_host(&state, host, None).await.unwrap();
    assert!(state.runtime.lock().await.is_none());
    release.send(()).unwrap();
    blocker.await.unwrap();
    assert!(matches!(open.await, Err(ApiError::InvalidCommand(_))));
    assert!(state.runtime.lock().await.is_none());
    assert_eq!(state.services().reservations().open_documents(), 0);
}

#[tokio::test]
async fn refused_publication_preserves_the_previous_runtime_and_ownership() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, offer) = recovery_identity_tests::pending_copy(area.path(), false).await;
    recover_document(&state, host.clone(), offer.document_id.to_string(), true)
        .await
        .unwrap();
    let before = get_document_state(&state).await.unwrap();
    let backend = state.exporter();
    let guard = state.runtime.lock().await;
    let mut new = Box::pin(new_document(&state, host));
    queued(new.as_mut()).await;
    state.begin_closing();
    drop(guard);
    assert!(matches!(new.await, Err(ApiError::InvalidCommand(_))));
    assert_eq!(get_document_state(&state).await.unwrap(), before);
    assert!(Arc::ptr_eq(&backend, &state.exporter()));
    assert_eq!(state.services().reservations().open_documents(), 1);
    assert!(state
        .services()
        .reservations()
        .claim_document(999, offer.document_id)
        .is_err());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, offer.marker.unwrap().opened_path.as_ref().unwrap())
        .is_err());
}

#[tokio::test]
async fn cancelling_admitted_shutdown_stops_host_tasks_and_keeps_recovery_cleanup() {
    let area = tempfile::tempdir().unwrap();
    let (state, mut host, _, offer) =
        recovery_identity_tests::pending_copy(area.path(), false).await;
    recover_document(&state, host.clone(), offer.document_id.to_string(), true)
        .await
        .unwrap();
    let completed = Arc::new(tokio::sync::Notify::new());
    host.events = Arc::new(ClosedProjection(completed.clone()));
    let stop = state.tasks.stop_signal();
    let (stopped, done) = tokio::sync::oneshot::channel();
    state.tasks.track(
        "shutdown-regression",
        tokio::spawn(async move {
            crate::host_tasks::stopped(stop).await;
            let _ = stopped.send(());
        }),
    );
    let lane = state.persistence.lock().await;
    let mut shutdown = Box::pin(shutdown_host(&state, host, None));
    queued(shutdown.as_mut()).await;
    assert!(state.runtime.lock().await.is_none());
    assert!(*state.tasks.stop_signal().borrow());
    drop(shutdown);
    done.await.unwrap();
    assert!(state
        .services()
        .reservations()
        .claim_document(999, offer.document_id)
        .is_err());
    drop(lane);
    completed.notified().await;
    let _lane = state.persistence.lock().await;
    let marker: SessionMarker = serde_json::from_slice(
        &std::fs::read(marker_path(area.path(), offer.document_id)).unwrap(),
    )
    .unwrap();
    assert_eq!(marker.pid, 0);
    assert!(state
        .services()
        .reservations()
        .claim_document(999, offer.document_id)
        .is_ok());
}

#[tokio::test]
async fn cancelling_shutdown_before_admission_leaves_the_host_open() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = recovery_identity_tests::pending_copy(area.path(), false).await;
    let before = get_document_state(&state).await.unwrap();
    let guard = state.runtime.lock().await;
    let mut shutdown = Box::pin(shutdown_host(&state, host, None));
    queued(shutdown.as_mut()).await;
    drop(shutdown);
    drop(guard);
    state.ensure_document_capacity().unwrap();
    assert!(!*state.tasks.stop_signal().borrow());
    assert_eq!(get_document_state(&state).await.unwrap(), before);
}
