use super::*;
use crate::{api::recovery_identity_tests::pending_copy, worker::PendingBackend};
use onecad_core::io::recovery::{autosave_path, marker_path};
use std::{future::Future, sync::Arc, task::Poll};

async fn recovered(root: &Path) -> (AppState, HostContext, PathBuf, DocumentId) {
    let (state, host, alias, offer) = pending_copy(root, false).await;
    recover_document(&state, host.clone(), offer.document_id.to_string(), true)
        .await
        .unwrap();
    assert!(state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .prepare_checkpoint()
        .is_none());
    (state, host, alias, offer.document_id)
}

// With no checkpoint and uncontended runtime locks, the first suspension is
// persistence admission. Polling once avoids timing assumptions and sleeps.
async fn queued(mut save: std::pin::Pin<&mut impl Future>) {
    std::future::poll_fn(|cx| {
        assert!(save.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn old_save_cannot_adopt_same_identity_replacement_or_clear_its_recovery() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, alias, id) = recovered(area.path()).await;
    let target = area.path().join("old-session-save.onecad");
    let lane = state.persistence.lock().await;
    let save = save_document(
        &state,
        host,
        Some(target.to_string_lossy().into_owned()),
        None,
    );
    tokio::pin!(save);
    queued(save.as_mut()).await;

    let backend = Arc::new(PendingBackend);
    let mut replacement =
        DocumentRuntime::open(&alias, backend.clone(), backend.clone(), backend).unwrap();
    replacement.mark_recovered(Some(alias.clone()), Some("Replacement".into()));
    let before = replacement.projection();
    let old = state.runtime.lock().await.replace(replacement).unwrap();
    assert_eq!(old.document_uuid(), id);
    assert_ne!(old.runtime_session(), before.runtime_session);
    assert_eq!(old.revision().0, before.revision);
    let checkpoint = std::fs::read(autosave_path(area.path(), id)).unwrap();
    let marker = std::fs::read(marker_path(area.path(), id)).unwrap();
    drop(lane);

    assert!(matches!(save.await, Err(ApiError::NoDocument(_))));
    assert!(target.exists(), "the old save can finish its own file");
    let guard = state.runtime.lock().await;
    let current = guard.as_ref().unwrap();
    assert_eq!(current.runtime_session(), before.runtime_session);
    assert_eq!(current.path(), Some(alias.as_path()));
    assert_eq!(current.title(), "Replacement");
    assert!(current.is_dirty());
    assert_eq!(
        std::fs::read(autosave_path(area.path(), id)).unwrap(),
        checkpoint
    );
    assert_eq!(std::fs::read(marker_path(area.path(), id)).unwrap(), marker);
}

#[tokio::test]
async fn save_of_older_revision_preserves_newer_recovery_checkpoint() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, id) = recovered(area.path()).await;
    let target = area.path().join("older-revision.onecad");
    let lane = state.persistence.lock().await;
    let save = save_document(
        &state,
        host,
        Some(target.to_string_lossy().into_owned()),
        None,
    );
    tokio::pin!(save);
    queued(save.as_mut()).await;

    let path = autosave_path(area.path(), id);
    {
        let mut guard = state.runtime.lock().await;
        let runtime = guard.as_mut().unwrap();
        let command = upsert_variable_command(&runtime.variables(), "depth", 10.0).unwrap();
        runtime.apply(command).unwrap();
        // The test owns the persistence lane: this represents a newer checkpoint
        // admitted before the queued explicit save.
        runtime.write_autosave(&path, save_meta()).unwrap();
    }
    let checkpoint = std::fs::read(&path).unwrap();
    drop(lane);

    let outcome = save.await.unwrap();
    assert!(!outcome.clean);
    assert!(outcome.current_revision > outcome.saved_revision);
    assert!(state.runtime.lock().await.as_ref().unwrap().is_dirty());
    assert_eq!(std::fs::read(path).unwrap(), checkpoint);
    assert!(marker_path(area.path(), id).exists());
}

#[tokio::test]
async fn save_cannot_overwrite_recovery_containers_or_markers() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, id) = recovered(area.path()).await;
    let checkpoint = std::fs::read(autosave_path(area.path(), id)).unwrap();
    let marker = std::fs::read(marker_path(area.path(), id)).unwrap();
    let targets = [
        autosave_path(area.path(), id),
        autosave_path(area.path(), DocumentId::new()),
        marker_path(area.path(), id),
        area.path().join("missing/../autosave/future.onecad"),
    ];
    for target in targets {
        let result = save_document(
            &state,
            host.clone(),
            Some(target.to_string_lossy().into_owned()),
            None,
        )
        .await;
        assert!(
            matches!(result, Err(ApiError::InvalidCommand(_))),
            "save to {} returned {result:?}",
            target.display()
        );
    }
    assert_eq!(
        std::fs::read(autosave_path(area.path(), id)).unwrap(),
        checkpoint
    );
    assert_eq!(std::fs::read(marker_path(area.path(), id)).unwrap(), marker);
    assert!(state.runtime.lock().await.as_ref().unwrap().is_dirty());
}
