use super::*;
use onecad_app::{
    document_runtime::DocumentRuntime, host::*, state::AppState, worker::PendingBackend,
};
use onecad_core::{
    document::variables::{Scalar, Variable},
    edit::EditCommand,
    ids::VariableId,
    io::{
        container::SaveMeta,
        recovery::{SessionMarker, marker_path, write_marker},
    },
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

fn new_state() -> AppState {
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

async fn crashed_copy(root: &Path) -> (HostContext, PathBuf, onecad_core::ids::DocumentId) {
    std::fs::create_dir(root.join("documents")).unwrap();
    let path = root.join("documents/plate.onecad");
    let backend = Arc::new(PendingBackend);
    let mut runtime = DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend);
    runtime
        .save(
            &path,
            SaveMeta {
                app_version: "test".into(),
                occt_fingerprint: None,
                created: "2026-10-19T00:00:00Z".into(),
                modified: "2026-10-19T00:00:00Z".into(),
            },
        )
        .unwrap();
    runtime
        .apply(EditCommand::AddVariable {
            variable: Variable {
                id: VariableId::new(),
                name: "RecoveredDepth".into(),
                value: Scalar::new(8.1234),
                unit: Default::default(),
            },
        })
        .unwrap();
    let id = runtime.document_uuid();
    let state = new_state();
    *state.runtime.lock().await = Some(runtime);
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(root.into()),
            app_config: None,
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    };
    onecad_app::autosave::autosave_current(&state.runtime, root, &state.persistence)
        .await
        .unwrap();
    mark_dead(root, id);
    (host, path, id)
}

fn read_marker(
    root: &Path,
    id: onecad_core::ids::DocumentId,
) -> Result<SessionMarker, serde_json::Error> {
    serde_json::from_slice(&std::fs::read(marker_path(root, id)).unwrap())
}

fn mark_dead(root: &Path, id: onecad_core::ids::DocumentId) {
    let mut marker = read_marker(root, id).unwrap();
    marker.pid = i32::MAX as u32;
    write_marker(root, &marker).unwrap();
}

#[tokio::test]
async fn discovery_blocks_saved_open_and_startup_close_preserves_offer() {
    let directory = tempfile::tempdir().unwrap();
    let (host, path, id) = crashed_copy(directory.path()).await;
    let state = new_state();
    assert_eq!(
        api::check_recovery(&state, host.clone())
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(matches!(
        api::open_document(&state, host.clone(), path.to_string_lossy().into(), None).await,
        Err(onecad_app::error::ApiError::RecoveryPending(_))
    ));
    api::close_document(&state, host.clone()).await.unwrap();
    assert_eq!(
        api::check_recovery(&state, host.clone())
            .await
            .unwrap()
            .len(),
        1
    );
    api::recover_document(&state, host.clone(), id.to_string(), true)
        .await
        .unwrap()
        .unwrap();
    let projection = api::get_projection(&state).await.unwrap();
    assert_eq!(projection.variables.len(), 1);
    assert_eq!(projection.variables[0].name, "RecoveredDepth");
    assert!(projection.dirty);
    assert_eq!(
        read_marker(directory.path(), id).unwrap().pid,
        std::process::id()
    );
    mark_dead(directory.path(), id);
    let restarted = new_state();
    assert_eq!(
        api::check_recovery(&restarted, host.clone())
            .await
            .unwrap()
            .len(),
        1
    );
    api::recover_document(&restarted, host, id.to_string(), true)
        .await
        .unwrap()
        .unwrap();
    let recovered = restarted.runtime.lock().await;
    assert_eq!(
        recovered.as_ref().unwrap().variables()[0].value.value,
        8.1234
    );
}

#[tokio::test]
async fn explicit_open_saved_discards_recovery_and_opens_saved_semantics() {
    let directory = tempfile::tempdir().unwrap();
    let (host, path, _) = crashed_copy(directory.path()).await;
    let state = new_state();
    assert_eq!(
        api::check_recovery(&state, host.clone())
            .await
            .unwrap()
            .len(),
        1
    );
    api::open_document(
        &state,
        host.clone(),
        path.to_string_lossy().into(),
        Some("openSaved".into()),
    )
    .await
    .unwrap();
    assert!(
        api::get_projection(&state)
            .await
            .unwrap()
            .variables
            .is_empty()
    );
    assert!(api::check_recovery(&state, host).await.unwrap().is_empty());
}

#[tokio::test]
async fn explicit_discard_does_not_open_or_mutate_saved_copy() {
    let directory = tempfile::tempdir().unwrap();
    let (host, path, id) = crashed_copy(directory.path()).await;
    let before = std::fs::read(&path).unwrap();
    let state = new_state();
    assert_eq!(
        api::check_recovery(&state, host.clone())
            .await
            .unwrap()
            .len(),
        1
    );
    api::recover_document(&state, host.clone(), id.to_string(), false)
        .await
        .unwrap();
    assert!(state.runtime.lock().await.is_none());
    assert!(api::check_recovery(&state, host).await.unwrap().is_empty());
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[tokio::test]
async fn failed_restore_keeps_offer_and_container_available_for_explicit_discard() {
    let directory = tempfile::tempdir().unwrap();
    let (host, _, id) = crashed_copy(directory.path()).await;
    let state = new_state();
    let offers = api::check_recovery(&state, host.clone()).await.unwrap();
    assert_eq!(offers.len(), 1);
    std::fs::write(&offers[0].autosave_path, b"truncated").unwrap();
    assert!(
        api::recover_document(&state, host.clone(), id.to_string(), true)
            .await
            .is_err()
    );
    assert!(state.peek_pending_recovery(id).is_some());
    assert!(Path::new(&offers[0].autosave_path).exists());
    assert!(state.runtime.lock().await.is_none());
    api::recover_document(&state, host, id.to_string(), false)
        .await
        .unwrap();
    assert!(!Path::new(&offers[0].autosave_path).exists());
}

#[tokio::test]
async fn entered_editor_defers_other_offers_and_recovery_save_paths_stay_isolated() {
    let directory = tempfile::tempdir().unwrap();
    let (host, saved_path, id) = crashed_copy(directory.path()).await;
    let state = new_state();
    let mut offers = api::check_recovery(&state, host).await.unwrap();
    let offer = offers.pop().unwrap();
    assert_eq!(
        destination(&directory.path().join("documents"), &offer).unwrap(),
        saved_path.canonicalize().unwrap()
    );
    let mut deferred = State {
        checked: true,
        offers: vec![offer.clone()],
        busy: false,
        opened: true,
        deferred: false,
    };
    assert!(deferred.offer(&id.to_string()).is_err());
    assert!(
        deferred.discardable(&id.to_string()).is_ok(),
        "discarding replaces nothing, so an open document never blocks it"
    );
    assert_eq!(deferred.offers.len(), 1);
    assert!(
        deferred.ready(),
        "an open document's file actions are not blocked"
    );
    let set_aside = State {
        checked: true,
        offers: vec![offer.clone()],
        deferred: true,
        ..Default::default()
    };
    assert!(
        set_aside.ready(),
        "deferred offers never block file actions"
    );
    assert!(
        !State {
            checked: true,
            offers: vec![offer.clone()],
            ..Default::default()
        }
        .ready()
    );
    deferred.opened = false;
    assert!(deferred.offer(&id.to_string()).is_ok());
    let orphan = RecoveryInfoDto {
        original_path: None,
        ..offer
    };
    assert_eq!(
        destination(&directory.path().join("documents"), &orphan).unwrap(),
        directory
            .path()
            .canonicalize()
            .unwrap()
            .join(format!("documents/recovered-{id}.onecad"))
    );
}

#[tokio::test]
async fn failed_open_saved_preserves_recovery_for_retry_and_restore() {
    let directory = tempfile::tempdir().unwrap();
    let (host, saved_path, id) = crashed_copy(directory.path()).await;
    let state = new_state();
    let offers = api::check_recovery(&state, host.clone()).await.unwrap();
    assert_eq!(offers.len(), 1);
    let autosave = Path::new(&offers[0].autosave_path);
    let preserved = std::fs::read(autosave).unwrap();
    std::fs::write(&saved_path, b"corrupt saved container").unwrap();
    let result = api::open_document(
        &state,
        host.clone(),
        saved_path.to_string_lossy().into(),
        Some("openSaved".into()),
    )
    .await;
    assert!(result.is_err());
    assert!(
        state.peek_pending_recovery(id).is_some(),
        "a failed saved open must keep the recovery decision retryable"
    );
    assert_eq!(std::fs::read(autosave).unwrap(), preserved);
    assert!(read_marker(directory.path(), id).is_ok());
    assert!(state.runtime.lock().await.is_none());
    api::recover_document(&state, host, id.to_string(), true)
        .await
        .unwrap()
        .unwrap();
    let recovered = state.runtime.lock().await;
    assert_eq!(
        recovered.as_ref().unwrap().variables()[0].value.value,
        8.1234
    );
}
