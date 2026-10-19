use super::*;
use crate::{
    host::{DialogPort, EventSink, FileDialog, HostPaths},
    worker::PendingBackend,
};
use std::sync::Arc;

struct Quiet;
impl EventSink for Quiet {
    fn emit(&self, _: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait::async_trait]
impl DialogPort for Quiet {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}

fn state() -> AppState {
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
            backend.clone(),
            backend,
        )
    }))
}

pub(super) async fn pending_copy(
    root: &Path,
    orphan: bool,
) -> (AppState, HostContext, PathBuf, RecoveryOffer) {
    let state = state();
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(root.into()),
            app_config: Some(root.join("config")),
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    };
    new_document(&state, host.clone()).await.unwrap();
    let original = root.join("original.onecad");
    save_document(
        &state,
        host.clone(),
        Some(original.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    let document_id = state.runtime.lock().await.as_ref().unwrap().document_uuid();
    let alias = root.join("renamed-copy.onecad");
    std::fs::copy(&original, &alias).unwrap();
    let autosave_path = onecad_core::io::recovery::autosave_path(root, document_id);
    std::fs::create_dir_all(autosave_path.parent().unwrap()).unwrap();
    std::fs::copy(&original, &autosave_path).unwrap();
    let marker = (!orphan).then(|| SessionMarker {
        document_id,
        pid: i32::MAX as u32,
        opened_path: Some(original),
        last_autosave: "2026-10-19T00:00:00Z".into(),
        title: Some("Recovery".into()),
    });
    if let Some(marker) = &marker {
        onecad_core::io::recovery::write_marker(root, marker).unwrap();
    }
    let offer = RecoveryOffer {
        document_id,
        autosave_path,
        marker,
        modified_ms: 123,
    };
    state.set_pending_recovery([offer.clone()]);
    new_document(&state, host.clone()).await.unwrap();
    (state, host, alias, offer)
}

async fn refuses_identity_alias(orphan: bool) {
    let area = tempfile::tempdir().unwrap();
    let (state, host, alias, offer) = pending_copy(area.path(), orphan).await;
    let before = get_projection(&state).await.unwrap();
    let exporter = state.exporter();
    let query = state.element_query();
    let bytes = std::fs::read(&offer.autosave_path).unwrap();
    assert!(state.pending_recovery_for_path(&alias).is_none());
    let result = open_document(&state, host, alias.to_string_lossy().into_owned(), None).await;
    assert!(matches!(result, Err(ApiError::RecoveryPending(_))));
    assert_eq!(
        get_projection(&state).await.unwrap().runtime_session,
        before.runtime_session
    );
    assert!(Arc::ptr_eq(&exporter, &state.exporter()));
    assert!(Arc::ptr_eq(&query, &state.element_query()));
    assert_eq!(
        state.peek_pending_recovery(offer.document_id),
        Some(offer.clone())
    );
    assert_eq!(std::fs::read(offer.autosave_path).unwrap(), bytes);
}

#[tokio::test]
async fn renamed_saved_copy_cannot_bypass_pending_uuid_recovery() {
    refuses_identity_alias(false).await;
}

#[tokio::test]
async fn orphan_recovery_blocks_saved_copy_with_same_uuid() {
    refuses_identity_alias(true).await;
}

#[tokio::test]
async fn explicit_open_saved_alias_discards_only_after_successful_replacement() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, alias, offer) = pending_copy(area.path(), true).await;
    open_document(
        &state,
        host,
        alias.to_string_lossy().into_owned(),
        Some("openSaved".into()),
    )
    .await
    .unwrap();
    assert_eq!(
        state.runtime.lock().await.as_ref().unwrap().document_uuid(),
        offer.document_id
    );
    assert!(state.peek_pending_recovery(offer.document_id).is_none());
    assert!(!offer.autosave_path.exists());
}
