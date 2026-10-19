use super::*;
use onecad_app::{
    host::{AppEvent, DialogPort, EventSink, HostContext, HostPaths},
    state::AppState,
    worker::PendingBackend,
};
use std::sync::{Arc, Mutex};

struct Host {
    picks: Mutex<Vec<String>>,
}
impl EventSink for Host {
    fn emit(&self, _: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait::async_trait]
impl DialogPort for Host {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        std::mem::take(&mut *self.picks.lock().unwrap())
    }
}

async fn test_document(root: &Path) -> (AppState, HostContext) {
    fs::create_dir(root.join("documents")).unwrap();
    let state = AppState::new(Arc::new(|| {
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
    }));
    let ports = Arc::new(Host {
        picks: Mutex::new(Vec::new()),
    });
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(root.join("data")),
            app_config: Some(root.join("config")),
        },
        events: ports.clone(),
        dialogs: ports,
    };
    api::new_document(&state, host.clone()).await.unwrap();
    (state, host)
}

fn pick(host: &mut HostContext, path: &Path) {
    host.dialogs = Arc::new(Host {
        picks: Mutex::new(vec![path.to_string_lossy().into_owned()]),
    });
}

#[test]
fn copied_open_never_overwrites_source_or_existing_qa_document() {
    let area = tempfile::tempdir().unwrap();
    let documents = area.path().join("documents");
    fs::create_dir(&documents).unwrap();
    let source = area.path().join("plate.onecad");
    fs::write(&source, b"source").unwrap();
    let first = copy_for_open(&documents, &source).unwrap();
    fs::write(&first, b"edited").unwrap();
    let second = copy_for_open(&documents, &source).unwrap();
    assert_ne!(first, second);
    assert_eq!(fs::read(&first).unwrap(), b"edited");
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert_eq!(copy_for_open(&documents, &first).unwrap(), first);
}

#[tokio::test]
async fn new_save_as_new_save_cannot_overwrite_previous_document() {
    let area = tempfile::tempdir().unwrap();
    let (state, mut host) = test_document(area.path()).await;
    let documents = area.path().join("documents");
    let first = documents.join("first.onecad");
    pick(&mut host, &first);
    assert!(matches!(
        save_to_qa(&state, host.clone(), &documents, false)
            .await
            .unwrap(),
        Outcome::Saved(_)
    ));
    let original = fs::read(&first).unwrap();
    api::new_document(&state, host.clone()).await.unwrap();
    assert!(
        state
            .runtime
            .lock()
            .await
            .as_ref()
            .unwrap()
            .path()
            .is_none()
    );
    assert!(matches!(
        save_to_qa(&state, host.clone(), &documents, false)
            .await
            .unwrap(),
        Outcome::Cancelled
    ));
    assert_eq!(fs::read(&first).unwrap(), original);
    let second = documents.join("second.onecad");
    pick(&mut host, &second);
    save_to_qa(&state, host.clone(), &documents, false)
        .await
        .unwrap();
    assert_ne!(fs::read(&second).unwrap(), original);
    api::open_document(&state, host, first.to_string_lossy().into_owned(), None)
        .await
        .unwrap();
    assert_eq!(
        state.runtime.lock().await.as_ref().unwrap().path(),
        Some(first.as_path())
    );
}

#[tokio::test]
async fn cancelled_and_failed_intents_preserve_runtime() {
    let area = tempfile::tempdir().unwrap();
    let (state, mut host) = test_document(area.path()).await;
    let documents = area.path().join("documents");
    let before = api::get_projection(&state).await.unwrap();
    assert!(api::open_file_dialog(host.clone()).await.unwrap().is_none());
    assert!(
        api::import_file_dialog(host.clone())
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        save_to_qa(&state, host.clone(), &documents, true)
            .await
            .unwrap(),
        Outcome::Cancelled
    ));
    let corrupt = area.path().join("corrupt.onecad");
    fs::write(&corrupt, b"not a container").unwrap();
    assert!(
        api::open_document(
            &state,
            host.clone(),
            corrupt.to_string_lossy().into_owned(),
            None
        )
        .await
        .is_err()
    );
    let outside = area.path().join("outside.onecad");
    pick(&mut host, &outside);
    assert!(save_to_qa(&state, host, &documents, true).await.is_err());
    assert!(!outside.exists());
    let after = api::get_projection(&state).await.unwrap();
    assert_eq!(before.runtime_session, after.runtime_session);
    assert_eq!(before.revision, after.revision);
}

#[cfg(unix)]
#[test]
fn save_export_reject_outside_directory_and_symlink_escapes() {
    let area = tempfile::tempdir().unwrap();
    let documents = area.path().join("documents");
    fs::create_dir(&documents).unwrap();
    assert!(confined_target(&documents, &area.path().join("outside.onecad")).is_err());
    let link = documents.join("linked.onecad");
    std::os::unix::fs::symlink(area.path().join("outside.onecad"), &link).unwrap();
    assert!(confined_target(&documents, &link).is_err());
    let directory_link = documents.join("escape");
    std::os::unix::fs::symlink(area.path(), &directory_link).unwrap();
    assert!(confined_target(&documents, &directory_link.join("outside.onecad")).is_err());
}

#[test]
fn export_format_cannot_overwrite_a_project() {
    for format in [
        ExportFormat::Step,
        ExportFormat::Stl,
        ExportFormat::Obj,
        ExportFormat::ThreeMf,
    ] {
        assert!(validate_export_extension(format, Path::new("saved.onecad")).is_err());
    }
    for (format, name) in [
        (ExportFormat::Step, "part.STP"),
        (ExportFormat::Stl, "part.stl"),
        (ExportFormat::Obj, "part.obj"),
        (ExportFormat::ThreeMf, "part.3mf"),
    ] {
        validate_export_extension(format, Path::new(name)).unwrap();
    }
}

#[test]
fn copied_open_rejects_oversized_source_before_creating_copy() {
    let area = tempfile::tempdir().unwrap();
    let documents = area.path().join("documents");
    fs::create_dir(&documents).unwrap();
    let source = area.path().join("large.onecad");
    fs::File::create(&source)
        .unwrap()
        .set_len(onecad_core::io::container::MAX_TOTAL_BYTES + 1)
        .unwrap();
    assert!(copy_for_open(&documents, &source).is_err());
    assert_eq!(fs::read_dir(&documents).unwrap().count(), 0);
}

#[tokio::test]
#[ignore = "requires freshly staged production worker; run with ONECAD_REQUIRE_WORKER=1 and ONECAD_WORKER_PATH"]
async fn real_worker_new_save_as_reopen_preserves_document_identity() {
    let area = tempfile::tempdir().unwrap();
    let (_, mut host) = test_document(area.path()).await;
    let state = AppState::default();
    state.start(host.clone()).unwrap();
    api::new_document(&state, host.clone()).await.unwrap();
    assert!(
        state
            .readiness()
            .wait_ready(std::time::Duration::from_secs(15))
            .await,
        "production worker handshake must succeed"
    );
    let before = api::get_projection(&state).await.unwrap();
    let documents = area.path().join("documents");
    let destination = documents.join("part.onecad");
    pick(&mut host, &destination);
    let outcome = save_to_qa(&state, host.clone(), &documents, false)
        .await
        .unwrap();
    assert!(matches!(outcome, Outcome::Saved(saved) if saved.clean));
    api::new_document(&state, host.clone()).await.unwrap();
    assert_ne!(
        api::get_projection(&state).await.unwrap().document_id,
        before.document_id
    );
    api::open_document(
        &state,
        host.clone(),
        destination.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(
        state
            .readiness()
            .wait_ready(std::time::Duration::from_secs(15))
            .await,
        "reopened document worker must handshake"
    );
    let after = api::get_projection(&state).await.unwrap();
    assert_eq!(before.document_id, after.document_id);
    assert_eq!(before.features.len(), after.features.len());
    assert!(!after.dirty);
    api::close_document(&state, host).await.unwrap();
    state.retire_all();
}

#[tokio::test]
async fn discard_finalization_preserves_cancelled_or_failed_replacement() {
    let area = tempfile::tempdir().unwrap();
    let (state, host) = test_document(area.path()).await;
    api::upsert_variable(&state, host.clone(), "Depth".into(), 8.1234)
        .await
        .unwrap();
    let root = host.paths.app_data.as_ref().unwrap();
    onecad_app::autosave::autosave_current(&state.runtime, root, &state.persistence)
        .await
        .unwrap();
    let id = state.runtime.lock().await.as_ref().unwrap().document_uuid();
    let recovery = onecad_core::io::recovery::autosave_path(root, id);
    let before = fs::read(&recovery).unwrap();
    for result in [Ok(Outcome::Cancelled), Err("failed replacement".into())] {
        let ticket = api::prepare_replacement_discard(&state).await.unwrap();
        let _ = finish_discard(&state, host.clone(), Some(ticket), result).await;
        assert_eq!(fs::read(&recovery).unwrap(), before);
    }
    let ticket = api::prepare_replacement_discard(&state).await.unwrap();
    api::new_document(&state, host.clone()).await.unwrap();
    finish_discard(
        &state,
        host,
        Some(ticket),
        Ok(Outcome::Replaced { warning: None }),
    )
    .await
    .unwrap();
    assert!(!recovery.exists());
}

#[path = "document_io_tests.rs"]
mod document_io_tests;
