use super::*;

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn desktop_startup_recovery_routes_external_restore_open_saved_and_discard() {
    use crate::startup_recovery::{Choice, discover, resolve};
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    for choice in [Choice::Restore, Choice::OpenSaved, Choice::Discard] {
        let area = tempfile::tempdir().unwrap();
        let paths =
            crate::qa::QaPaths::desktop_at(area.path(), &area.path().join("tmp"), None).unwrap();
        let (mut backend, _) = backend_at(paths).await;
        let external = area.path().join("external.onecad");
        let id = seed_external_recovery(&mut backend, &external).await;
        if matches!(choice, Choice::Discard) {
            fs::write(&external, b"corrupt saved source").unwrap();
        }
        let saved = fs::read(&external).unwrap();
        assert!(discover(&backend).await.unwrap());
        resolve(&mut backend, &id, choice).await.unwrap();
        let projection = api::get_projection(&backend.state).await.unwrap();
        match choice {
            Choice::Restore => {
                assert_eq!(projection.variables[0].value, 8.1234);
                assert!(projection.dirty);
            }
            Choice::OpenSaved => {
                assert!(projection.variables.is_empty() && !projection.dirty);
            }
            Choice::Discard => {
                assert!(backend.state.runtime.lock().await.is_none());
            }
        }
        if !matches!(choice, Choice::Discard) {
            assert_eq!(
                backend.state.runtime.lock().await.as_ref().unwrap().path(),
                Some(external.canonicalize().unwrap().as_path())
            );
        }
        assert_eq!(fs::read(external).unwrap(), saved);
        api::close_document(&backend.state, backend.host.clone())
            .await
            .unwrap();
        backend.state.retire_all();
    }
}

async fn seed_external_recovery(backend: &mut Backend, external: &Path) -> String {
    api::save_document(
        &backend.state,
        backend.host.clone(),
        Some(external.to_string_lossy().into()),
        None,
    )
    .await
    .unwrap();
    api::upsert_variable(
        &backend.state,
        backend.host.clone(),
        "RecoveredDepth".into(),
        8.1234,
    )
    .await
    .unwrap();
    onecad_app::autosave::autosave_current(
        &backend.state.runtime,
        &backend.paths.data,
        &backend.state.persistence,
    )
    .await
    .unwrap();
    let id = api::get_projection(&backend.state)
        .await
        .unwrap()
        .document_id;
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.startup_opened = false;
    id
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn desktop_real_worker_external_save_reopen_and_step_export() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let area = tempfile::tempdir().unwrap();
    let paths = crate::qa::QaPaths::desktop_args(Vec::new(), area.path(), &area.path().join("tmp"))
        .unwrap();
    let (mut backend, _) = backend_at(paths).await;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../onecad-app/tests/fixtures/colored_boxes.step");
    let result = intent(&mut backend, Request::Import(Some(fixture)))
        .await
        .unwrap();
    assert!(
        matches!(result, Outcome::Replaced { warning: None }),
        "{result:?}"
    );
    let expected = solid_properties(&backend).await;
    let saved = area.path().join("user-project.onecad");
    choose(&mut backend, Some(&saved));
    let result = intent(&mut backend, Request::Save { save_as: true })
        .await
        .unwrap();
    assert!(matches!(result, Outcome::Saved(saved) if saved.clean));
    desktop_reopen_and_edit(&mut backend, &saved).await;
    let actual = solid_properties(&backend).await;
    assert_eq!(expected.len(), actual.len());
    for (expected, actual) in expected.iter().zip(&actual) {
        near(expected.0, actual.0);
    }
    let exported = area.path().join("user-project.step");
    choose(&mut backend, Some(&exported));
    let result = intent(&mut backend, Request::Export(ExportFormat::Step))
        .await
        .unwrap();
    assert!(matches!(result, Outcome::Exported(path) if path == exported.canonicalize().unwrap()));
    let text = fs::read_to_string(exported).unwrap();
    assert!(text.starts_with("ISO-10303-21;") && text.contains("MANIFOLD_SOLID_BREP"));
    assert!(text.trim_end().ends_with("END-ISO-10303-21;"));
    api::close_document(&backend.state, backend.host.clone())
        .await
        .unwrap();
    backend.state.retire_all();
}

async fn desktop_reopen_and_edit(backend: &mut Backend, saved: &Path) {
    let identity = api::get_projection(&backend.state)
        .await
        .unwrap()
        .document_id;
    let result = intent(backend, Request::New).await.unwrap();
    assert!(
        matches!(result, Outcome::Replaced { warning: None }),
        "{result:?}"
    );
    let result = intent(backend, Request::Open(Some(saved.to_owned())))
        .await
        .unwrap();
    assert!(
        matches!(result, Outcome::Replaced { warning: None }),
        "{result:?}"
    );
    assert_eq!(
        backend.state.runtime.lock().await.as_ref().unwrap().path(),
        Some(saved.canonicalize().unwrap().as_path())
    );
    assert_eq!(
        api::get_projection(&backend.state)
            .await
            .unwrap()
            .document_id,
        identity
    );
    api::upsert_variable(&backend.state, backend.host.clone(), "Depth".into(), 9.25)
        .await
        .unwrap();
    let result = intent(backend, Request::Save { save_as: false })
        .await
        .unwrap();
    assert!(matches!(result, Outcome::Saved(saved) if saved.clean));
    let result = intent(backend, Request::Open(Some(saved.to_owned())))
        .await
        .unwrap();
    assert!(
        matches!(result, Outcome::Replaced { warning: None }),
        "{result:?}"
    );
    assert_eq!(
        api::get_projection(&backend.state).await.unwrap().variables[0].value,
        9.25
    );
    let loaded = onecad_core::io::container::ContainerReader::open(saved).unwrap();
    assert_eq!(
        loaded
            .document()
            .variables
            .get("Depth")
            .unwrap()
            .value
            .value,
        9.25
    );
}
