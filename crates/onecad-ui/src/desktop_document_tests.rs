use super::*;

#[tokio::test]
async fn desktop_external_open_edit_save_as_and_reopen_keep_selected_paths() {
    let area = tempfile::tempdir().unwrap();
    let paths =
        crate::qa::QaPaths::desktop_at(area.path(), &area.path().join("tmp"), None).unwrap();
    let state_root = area.path().join("state");
    fs::create_dir(&state_root).unwrap();
    let (state, mut host) = test_document(&state_root).await;
    host.paths = HostPaths {
        app_data: Some(paths.data.clone()),
        app_config: Some(paths.config.clone()),
    };
    let external = area.path().join("external.ONECAD");
    api::save_document(
        &state,
        host.clone(),
        Some(external.to_string_lossy().into()),
        None,
    )
    .await
    .unwrap();
    api::close_document(&state, host.clone()).await.unwrap();
    let selected = paths.open_source(&external).unwrap();
    api::open_document(
        &state,
        host.clone(),
        selected.to_string_lossy().into(),
        None,
    )
    .await
    .unwrap();
    let identity = api::get_projection(&state).await.unwrap().document_id;
    api::upsert_variable(&state, host.clone(), "Depth".into(), 9.25)
        .await
        .unwrap();
    let saved = save_with_policy(&state, host.clone(), paths.policy, &paths.documents, false)
        .await
        .unwrap();
    assert!(
        matches!(saved, Outcome::Saved(saved) if saved.clean && Path::new(&saved.path) == selected)
    );
    let loaded = onecad_core::io::container::ContainerReader::open(&external).unwrap();
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
    let chosen = area.path().join("chosen.onecad");
    pick(&mut host, &chosen);
    save_with_policy(&state, host.clone(), paths.policy, &paths.documents, true)
        .await
        .unwrap();
    api::close_document(&state, host.clone()).await.unwrap();
    api::open_document(&state, host.clone(), chosen.to_string_lossy().into(), None)
        .await
        .unwrap();
    let reopened = api::get_projection(&state).await.unwrap();
    assert_eq!(reopened.document_id, identity);
    assert_eq!(reopened.variables[0].value, 9.25);
    assert!(!reopened.dirty);
    api::close_document(&state, host).await.unwrap();
}
