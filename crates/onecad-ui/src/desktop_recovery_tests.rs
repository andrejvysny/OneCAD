use super::*;

async fn crashed_desktop(
    root: &Path,
) -> (QaPaths, HostContext, PathBuf, onecad_core::ids::DocumentId) {
    let paths = QaPaths::desktop_at(root, &root.join("tmp"), None).unwrap();
    let (host, original, id) = crashed_copy(&paths.data).await;
    let external = root.join("external.onecad");
    std::fs::rename(original, &external).unwrap();
    let mut marker = read_marker(&paths.data, id).unwrap();
    marker.opened_path = Some(external.clone());
    write_marker(&paths.data, &marker).unwrap();
    (paths, host, external.canonicalize().unwrap(), id)
}

#[tokio::test]
async fn desktop_restore_keeps_external_save_path_and_survives_save_reopen() {
    let area = tempfile::tempdir().unwrap();
    let (paths, host, external, id) = crashed_desktop(area.path()).await;
    let state = new_state();
    let offers = api::check_recovery(&state, host.clone()).await.unwrap();
    let destination = recovery_destination(&paths, &offers[0]).unwrap();
    assert_eq!(destination, external);
    api::recover_document_at_path(
        &state,
        host.clone(),
        id.to_string(),
        true,
        Some(destination),
    )
    .await
    .unwrap()
    .unwrap();
    let projection = api::get_projection(&state).await.unwrap();
    assert!(projection.dirty);
    assert_eq!(projection.variables[0].value, 8.1234);
    assert_eq!(
        state.runtime.lock().await.as_ref().unwrap().path(),
        Some(external.as_path())
    );
    api::save_document(&state, host.clone(), None, None)
        .await
        .unwrap();
    api::close_document(&state, host.clone()).await.unwrap();
    api::open_document(
        &state,
        host.clone(),
        external.to_string_lossy().into(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        api::get_projection(&state).await.unwrap().variables[0].value,
        8.1234
    );
    assert!(!marker_path(&paths.data, id).exists());
    api::close_document(&state, host).await.unwrap();
}

#[tokio::test]
async fn desktop_open_saved_uses_external_original_and_explicitly_discards_checkpoint() {
    let area = tempfile::tempdir().unwrap();
    let (paths, host, external, id) = crashed_desktop(area.path()).await;
    let state = new_state();
    let offers = api::check_recovery(&state, host.clone()).await.unwrap();
    assert_eq!(recovery_destination(&paths, &offers[0]).unwrap(), external);
    api::open_document(
        &state,
        host.clone(),
        external.to_string_lossy().into(),
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
    assert!(!marker_path(&paths.data, id).exists());
    api::close_document(&state, host).await.unwrap();
}

#[tokio::test]
async fn desktop_missing_saved_file_restores_under_persistent_recovered_directory() {
    let area = tempfile::tempdir().unwrap();
    let (paths, host, external, id) = crashed_desktop(area.path()).await;
    std::fs::remove_file(external).unwrap();
    let state = new_state();
    let offers = api::check_recovery(&state, host.clone()).await.unwrap();
    let destination = recovery_destination(&paths, &offers[0]).unwrap();
    assert_eq!(
        destination,
        paths.root.join(format!("Recovered/recovered-{id}.onecad"))
    );
    paths.target(&destination).unwrap();
    api::recover_document_at_path(
        &state,
        host.clone(),
        id.to_string(),
        true,
        Some(destination.clone()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(api::get_projection(&state).await.unwrap().dirty);
    api::save_document(&state, host.clone(), None, None)
        .await
        .unwrap();
    assert!(destination.is_file());
    api::close_document(&state, host).await.unwrap();
}

#[tokio::test]
async fn desktop_recovery_refuses_external_file_replaced_with_another_identity() {
    let area = tempfile::tempdir().unwrap();
    let (paths, host, external, id) = crashed_desktop(area.path()).await;
    let backend = Arc::new(PendingBackend);
    DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend)
        .save(
            &external,
            SaveMeta {
                app_version: "test".into(),
                occt_fingerprint: None,
                created: "test".into(),
                modified: "test".into(),
            },
        )
        .unwrap();
    let state = new_state();
    let offers = api::check_recovery(&state, host).await.unwrap();
    let checkpoint = std::fs::read(&offers[0].autosave_path).unwrap();
    let saved = std::fs::read(&external).unwrap();
    assert!(
        recovery_destination(&paths, &offers[0])
            .unwrap_err()
            .contains("different document")
    );
    assert_eq!(std::fs::read(external).unwrap(), saved);
    assert_eq!(std::fs::read(&offers[0].autosave_path).unwrap(), checkpoint);
    assert!(marker_path(&paths.data, id).exists());
}
