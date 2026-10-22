//! No-document Home must remain distinct from an empty user-created document.
use super::*;
use crate::bridge::{Command, EventMailbox, Shared};

async fn backend(root: &Path) -> Backend {
    let (state, host) = test_document(root).await;
    api::close_document(&state, host.clone()).await.unwrap();
    let paths = crate::qa::QaPaths::from_args(
        ["--qa-root".into(), root.join("qa").into_os_string()].into_iter(),
    )
    .unwrap();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_features: onecad_render::required_device_features(adapter.features()),
            ..Default::default()
        })
        .await
        .unwrap();
    let renderer =
        onecad_render::Renderer::new(&device, &queue, [0, 0], Default::default()).unwrap();
    let shared = Arc::new(Shared::new(
        egui::Context::default(),
        renderer.preparation_budget(),
    ));
    Backend {
        state: Arc::new(state),
        host,
        paths,
        events: Arc::new(EventMailbox::default()),
        shared,
        startup_opened: false,
        scene_revision: 0,
        previous_scene: None,
        preparation_cache: Default::default(),
        preparation_lease: None,
    }
}

async fn startup(backend: &mut Backend) {
    let permit = Arc::new(tokio::sync::Semaphore::new(1))
        .acquire_owned()
        .await
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    crate::services::execute(backend, Command::Open, permit, &sender).await;
    assert!(matches!(
        receiver.recv().unwrap(),
        Message::Terminal { result: Ok(_), .. }
    ));
}

#[tokio::test]
#[ignore = "requires GPU for the host service budget; run with ONECAD_REQUIRE_GPU=1"]
async fn startup_and_cancelled_failed_file_actions_leave_no_document() {
    let area = tempfile::tempdir().unwrap();
    let mut backend = backend(area.path()).await;
    startup(&mut backend).await;
    assert!(!backend.startup_opened);
    assert!(backend.state.runtime.lock().await.is_none());
    assert!(backend.shared.failure.lock().unwrap().is_none());
    assert!(backend.shared.recovery.lock().unwrap().checked);
    for request in [Request::Open(None), Request::Import(None)] {
        assert!(matches!(
            run(&mut backend, request).await,
            Ok(Outcome::Cancelled)
        ));
    }
    let bad = area.path().join("bad.onecad");
    fs::write(&bad, b"corrupt container").unwrap();
    assert!(run(&mut backend, Request::Open(Some(bad))).await.is_err());
    for request in [
        Request::Save { save_as: false },
        Request::Export(ExportFormat::Step),
    ] {
        assert_eq!(
            run(&mut backend, request).await.unwrap_err(),
            "Create or open a document before saving or exporting"
        );
    }
    assert!(backend.state.runtime.lock().await.is_none());
    assert!(!backend.startup_opened);
}

#[tokio::test]
#[ignore = "requires GPU for the host service budget; run with ONECAD_REQUIRE_GPU=1"]
async fn explicit_new_establishes_one_empty_unsaved_session_and_failure_preserves_it() {
    let area = tempfile::tempdir().unwrap();
    let mut backend = backend(area.path()).await;
    startup(&mut backend).await;
    assert!(matches!(
        run(&mut backend, Request::New).await,
        Ok(Outcome::Replaced { .. })
    ));
    let before = api::get_document_state(&backend.state).await.unwrap();
    assert!(!before.runtime_session.is_empty());
    assert!(before.path.is_none());
    assert!(
        api::get_projection(&backend.state)
            .await
            .unwrap()
            .features
            .is_empty()
    );
    crate::document_state::refresh(&backend).await;
    assert_eq!(
        backend
            .shared
            .document_state
            .lock()
            .unwrap()
            .latest()
            .unwrap()
            .1,
        before
    );
    assert!(backend.startup_opened);
    assert!(
        run(
            &mut backend,
            Request::Open(Some(area.path().join("missing.onecad")))
        )
        .await
        .is_err()
    );
    assert!(matches!(
        run(&mut backend, Request::Import(None)).await,
        Ok(Outcome::Cancelled)
    ));
    assert_eq!(
        api::get_document_state(&backend.state).await.unwrap(),
        before
    );
}

#[tokio::test]
#[ignore = "requires GPU for the host service budget; run with ONECAD_REQUIRE_GPU=1"]
async fn discarding_last_recovery_offer_without_startup_path_leaves_home_empty() {
    let area = tempfile::tempdir().unwrap();
    let mut backend = backend(area.path()).await;
    startup(&mut backend).await;
    run(&mut backend, Request::New).await.unwrap();
    api::upsert_variable(&backend.state, backend.host.clone(), "Depth".into(), 8.0)
        .await
        .unwrap();
    let root = backend.host.paths.app_data.as_ref().unwrap();
    onecad_app::autosave::autosave_current(
        &backend.state.runtime,
        root,
        &backend.state.persistence,
    )
    .await
    .unwrap();
    let id = backend
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .document_uuid();
    let marker_path = onecad_core::io::recovery::marker_path(root, id);
    let mut marker: onecad_core::io::recovery::SessionMarker =
        serde_json::from_slice(&fs::read(marker_path).unwrap()).unwrap();
    marker.pid = i32::MAX as u32;
    onecad_core::io::recovery::write_marker(root, &marker).unwrap();
    *backend.state.runtime.lock().await = None;
    backend.startup_opened = false;
    assert!(crate::startup_recovery::discover(&backend).await.unwrap());
    assert!(
        !crate::startup_recovery::resolve(
            &mut backend,
            &id.to_string(),
            crate::startup_recovery::Choice::Discard,
        )
        .await
        .unwrap()
    );
    assert!(backend.state.runtime.lock().await.is_none());
    assert!(!backend.startup_opened);
    let recovery = backend.shared.recovery.lock().unwrap();
    assert!(recovery.checked && recovery.offers.is_empty() && !recovery.opened);
}
