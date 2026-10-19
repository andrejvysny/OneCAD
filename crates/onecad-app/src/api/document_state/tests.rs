use super::*;
use crate::api::{new_document, recovery_identity_tests::pending_copy};

#[tokio::test]
async fn lifecycle_read_is_authoritative_without_projection_or_meshes() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let snapshot = new_document(&state, host).await.unwrap();
    let lifecycle = get_document_state(&state).await.unwrap();
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    assert_eq!(lifecycle.document_id, snapshot.document_id);
    assert_eq!(lifecycle.runtime_session, snapshot.runtime_session);
    assert_eq!(lifecycle.revision, runtime.revision().0);
    assert_eq!(lifecycle.dirty, runtime.is_dirty());
    assert_eq!(lifecycle.path, None);
}

#[tokio::test]
async fn lifecycle_metadata_is_bounded_without_changing_backend_values() {
    let area = tempfile::tempdir().unwrap();
    let (state, _, _, _) = pending_copy(area.path(), false).await;
    let title = "界".repeat(2000);
    let path = std::path::PathBuf::from("界".repeat(12000));
    {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .unwrap()
            .mark_recovered(Some(path.clone()), Some(title.clone()));
    }
    let lifecycle = get_document_state(&state).await.unwrap();
    assert!(lifecycle.dirty);
    assert_eq!(lifecycle.title.len(), 4095);
    assert_eq!(lifecycle.path.as_ref().unwrap().len(), 32766);
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    assert_eq!(runtime.title(), title);
    assert_eq!(runtime.path(), Some(path.as_path()));
}

#[tokio::test]
async fn no_document_has_empty_lifecycle_state() {
    let area = tempfile::tempdir().unwrap();
    let (state, _, _, _) = pending_copy(area.path(), true).await;
    *state.runtime.lock().await = None;
    assert_eq!(
        get_document_state(&state).await.unwrap(),
        DocumentState::default()
    );
}
