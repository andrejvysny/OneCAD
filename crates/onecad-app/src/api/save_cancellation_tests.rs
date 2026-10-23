//! Deterministic lifetime pins: cancellation cannot detach a write from its ownership.
use super::*;
use crate::persistence::Hooks;
use onecad_core::io::{
    container::ContainerReader,
    recovery::{autosave_path, marker_path},
};
use std::{
    future::Future,
    sync::{mpsc, Arc},
    task::Poll,
};

async fn recovered(root: &Path) -> (Arc<AppState>, HostContext, PathBuf, DocumentId) {
    let (state, host, alias, offer) = recovery_identity_tests::pending_copy(root, false).await;
    recover_document(&state, host.clone(), offer.document_id.to_string(), true)
        .await
        .unwrap();
    (Arc::new(state), host, alias, offer.document_id)
}

fn write_gate(state: &AppState) -> (tokio::sync::oneshot::Receiver<()>, mpsc::Sender<()>) {
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = mpsc::channel();
    *state.save_hooks.lock().unwrap() = Some(Hooks {
        writing: Some((started, wait)),
        ..Default::default()
    });
    (ready, release)
}

fn completion_gate(
    state: &AppState,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = tokio::sync::oneshot::channel();
    *state.save_hooks.lock().unwrap() = Some(Hooks {
        completing: Some((started, wait)),
        ..Default::default()
    });
    (ready, release)
}

fn spawn_save(
    state: &Arc<AppState>,
    host: HostContext,
    target: Option<PathBuf>,
) -> tokio::task::JoinHandle<Result<SaveOutcomeDto, ApiError>> {
    let state = state.clone();
    tokio::spawn(async move {
        save_document(
            &state,
            host,
            target.map(|p| p.to_string_lossy().into_owned()),
            None,
        )
        .await
    })
}

async fn queued(mut work: std::pin::Pin<&mut impl Future>) {
    std::future::poll_fn(|cx| {
        assert!(work.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn cancellation_before_save_admission_writes_nothing_and_releases_staging() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, id) = recovered(area.path()).await;
    let target = area.path().join("cancelled.onecad");
    let lane = state.persistence.lock().await;
    let mut save = Box::pin(save_document(
        &state,
        host,
        Some(target.to_string_lossy().into_owned()),
        None,
    ));
    queued(save.as_mut()).await;
    drop(save);
    state.release_claims();
    let reservations = state.services().reservations();
    assert!(reservations.claim_path(999, &target).is_ok());
    assert!(reservations.claim_document(999, id).is_ok());
    drop(lane);
    assert!(!target.exists());
    assert!(autosave_path(area.path(), id).exists());
}

#[tokio::test]
async fn ordinary_save_keeps_lane_identity_target_and_source_after_waiter_and_host_close() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, source, id) = recovered(area.path()).await;
    state.add_claim(state.claim_path(&source).unwrap());
    let target = state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .path()
        .unwrap()
        .to_path_buf();
    let marker = std::fs::read(marker_path(area.path(), id)).unwrap();
    let checkpoint = std::fs::read(autosave_path(area.path(), id)).unwrap();
    let (ready, release) = write_gate(&state);
    let save = spawn_save(&state, host, None);
    ready.await.unwrap();
    save.abort();
    assert!(save.await.unwrap_err().is_cancelled());
    state.runtime.lock().await.take();
    state.release_claims();
    let reservations = state.services().reservations();
    assert!(reservations.claim_path(999, &target).is_err());
    assert!(reservations.claim_path(999, &source).is_err());
    assert!(reservations.claim_document(999, id).is_err());
    assert!(state.persistence.try_lock().is_err());
    release.send(()).unwrap();
    let _lane = state.persistence.lock().await;
    assert_eq!(ContainerReader::open(&target).unwrap().document().id, id);
    assert!(reservations.claim_path(999, &target).is_ok());
    assert!(reservations.claim_path(999, &source).is_ok());
    assert!(reservations.claim_document(999, id).is_ok());
    assert_eq!(std::fs::read(marker_path(area.path(), id)).unwrap(), marker);
    assert_eq!(
        std::fs::read(autosave_path(area.path(), id)).unwrap(),
        checkpoint
    );
}

#[tokio::test]
async fn cancelled_save_waiter_still_commits_a_successful_live_save_as() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, source, id) = recovered(area.path()).await;
    state.add_claim(state.claim_path(&source).unwrap());
    let target = area.path().join("successful.onecad");
    let (ready, release) = write_gate(&state);
    let save = spawn_save(&state, host, Some(target.clone()));
    ready.await.unwrap();
    save.abort();
    let _ = save.await;
    release.send(()).unwrap();
    let _lane = state.persistence.lock().await;
    let runtime = state.runtime.lock().await;
    let runtime = runtime.as_ref().unwrap();
    assert_eq!(runtime.path(), Some(target.as_path()));
    assert!(!runtime.is_dirty());
    assert!(!autosave_path(area.path(), id).exists());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &source)
        .is_err());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_err());
    assert!(ContainerReader::open(&target).is_ok());
}

#[tokio::test]
async fn failed_detached_save_as_keeps_runtime_and_recovery_and_releases_target() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, id) = recovered(area.path()).await;
    let before = get_document_state(&state).await.unwrap();
    let checkpoint = std::fs::read(autosave_path(area.path(), id)).unwrap();
    let marker = std::fs::read(marker_path(area.path(), id)).unwrap();
    let target = area.path().join("blocked.onecad");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep"), b"untouched").unwrap();
    let (ready, release) = write_gate(&state);
    let save = spawn_save(&state, host, Some(target.clone()));
    ready.await.unwrap();
    save.abort();
    let _ = save.await;
    release.send(()).unwrap();
    let _lane = state.persistence.lock().await;
    assert_eq!(get_document_state(&state).await.unwrap(), before);
    assert_eq!(
        std::fs::read(autosave_path(area.path(), id)).unwrap(),
        checkpoint
    );
    assert_eq!(std::fs::read(marker_path(area.path(), id)).unwrap(), marker);
    assert_eq!(std::fs::read(target.join("keep")).unwrap(), b"untouched");
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_ok());
}

#[tokio::test]
async fn cancellation_after_write_keeps_completion_fenced_from_same_uuid_replacement() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, alias, id) = recovered(area.path()).await;
    let target = area.path().join("old-session.onecad");
    let (written, release) = completion_gate(&state);
    let save = spawn_save(&state, host, Some(target.clone()));
    written.await.unwrap();
    assert!(ContainerReader::open(&target).is_ok());
    save.abort();
    let _ = save.await;
    let backend = Arc::new(crate::worker::PendingBackend);
    let mut replacement =
        DocumentRuntime::open(&alias, backend.clone(), backend.clone(), backend).unwrap();
    replacement.mark_recovered(Some(alias.clone()), Some("Replacement".into()));
    let session = replacement.runtime_session();
    state.runtime.lock().await.replace(replacement);
    let marker = std::fs::read(marker_path(area.path(), id)).unwrap();
    let checkpoint = std::fs::read(autosave_path(area.path(), id)).unwrap();
    release.send(()).unwrap();
    let _lane = state.persistence.lock().await;
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    assert_eq!(runtime.runtime_session(), session);
    assert_eq!(runtime.path(), Some(alias.as_path()));
    assert!(runtime.is_dirty());
    assert_eq!(std::fs::read(marker_path(area.path(), id)).unwrap(), marker);
    assert_eq!(
        std::fs::read(autosave_path(area.path(), id)).unwrap(),
        checkpoint
    );
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_ok());
}

#[tokio::test]
async fn cancellation_after_write_preserves_newer_recovery_and_retargets_its_metadata() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, source, id) = recovered(area.path()).await;
    state.add_claim(state.claim_path(&source).unwrap());
    let target = area.path().join("dirty-save-as.onecad");
    let (written, release) = completion_gate(&state);
    let save = spawn_save(&state, host, Some(target.clone()));
    written.await.unwrap();
    save.abort();
    let _ = save.await;
    let checkpoint_path = autosave_path(area.path(), id);
    {
        let mut guard = state.runtime.lock().await;
        let runtime = guard.as_mut().unwrap();
        let command = upsert_variable_command(&runtime.variables(), "depth", 10.0).unwrap();
        runtime.apply(command).unwrap();
        runtime
            .write_autosave(&checkpoint_path, save_meta())
            .unwrap();
    }
    let checkpoint = std::fs::read(&checkpoint_path).unwrap();
    release.send(()).unwrap();
    let _lane = state.persistence.lock().await;
    assert!(get_document_state(&state).await.unwrap().dirty);
    assert_eq!(std::fs::read(checkpoint_path).unwrap(), checkpoint);
    let marker: SessionMarker =
        serde_json::from_slice(&std::fs::read(marker_path(area.path(), id)).unwrap()).unwrap();
    assert_eq!(marker.opened_path, Some(target));
    assert_eq!(marker.source_paths, vec![source.canonicalize().unwrap()]);
}
