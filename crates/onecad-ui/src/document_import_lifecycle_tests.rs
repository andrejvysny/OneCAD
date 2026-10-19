//! Real STEP replacement must publish lifecycle state even when its scene cannot fit.
use super::*;
use crate::document_flow::{Pending, needs_confirmation};

async fn execute(backend: &mut Backend, request: Request) -> Result<Outcome, String> {
    let permit = Arc::new(tokio::sync::Semaphore::new(1))
        .acquire_owned()
        .await
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    crate::document_services::execute(backend, request, permit, &sender).await;
    match receiver.recv().unwrap() {
        Message::Document { result, .. } => result,
        _ => panic!("expected document completion"),
    }
}

fn lifecycle(backend: &Backend) -> onecad_app::dto::DocumentState {
    backend
        .shared
        .document_state
        .lock()
        .unwrap()
        .latest()
        .unwrap()
        .1
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn dirty_step_import_and_save_publish_lifecycle_despite_failed_capture() {
    let area = tempfile::tempdir().unwrap();
    let (mut backend, _) = backend(area.path()).await;
    crate::document_state::refresh(&backend).await;
    let old_display = crate::services::load_ready_display(&mut backend)
        .await
        .unwrap();
    assert!(!old_display.projection.dirty);
    let previous_session = old_display.projection.runtime_session.clone();
    backend.shared.publish_display(old_display);
    let old_display = backend.shared.display.lock().unwrap().take().unwrap();
    let budget = backend.shared.preparation_budget().unwrap();
    *backend.shared.budget.lock().unwrap() = budget.limit_cpu_prepared_bytes(0);
    let original = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../onecad-app/tests/fixtures/colored_boxes.step");
    let source = backend.paths.root.join("documents/imported.step");
    fs::copy(original, &source).unwrap();
    choose(&mut backend, Some(&source));
    let outcome = execute(&mut backend, Request::Import).await.unwrap();
    assert!(matches!(outcome, Outcome::Replaced { warning: Some(_) }));
    let imported = lifecycle(&backend);
    assert_ne!(imported.runtime_session, previous_session);
    assert!(imported.dirty);
    assert!(imported.path.is_none());
    assert!(!backend.shared.valid.load(Ordering::Acquire));
    assert!(backend.shared.display.lock().unwrap().is_none());
    assert!(!old_display.projection.dirty);
    assert!(needs_confirmation(&Pending::Quit, imported.dirty));
    assert!(needs_confirmation(
        &Pending::Document(Request::New),
        imported.dirty
    ));
    save_import_after_capture_refusal(&mut backend, &imported).await;
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}

async fn save_import_after_capture_refusal(
    backend: &mut Backend,
    imported: &onecad_app::dto::DocumentState,
) {
    let failed = backend.paths.root.join("documents/failed.onecad");
    fs::create_dir(&failed).unwrap();
    choose(backend, Some(&failed));
    assert!(
        execute(backend, Request::Save { save_as: false })
            .await
            .is_err()
    );
    assert_eq!(lifecycle(backend), *imported);
    let saved = backend.paths.root.join("documents/Saved imported.onecad");
    choose(backend, Some(&saved));
    assert!(
        matches!(execute(backend, Request::Save { save_as: false }).await.unwrap(), Outcome::Saved(saved) if saved.clean)
    );
    let current = lifecycle(backend);
    assert!(!current.dirty);
    assert_eq!(current.runtime_session, imported.runtime_session);
    assert_eq!(current.title, "Saved imported");
    assert_eq!(current.path.as_deref(), saved.to_str());
    assert!(saved.is_file());
    assert!(!backend.shared.valid.load(Ordering::Acquire));
    assert!(backend.shared.display.lock().unwrap().is_none());
}
