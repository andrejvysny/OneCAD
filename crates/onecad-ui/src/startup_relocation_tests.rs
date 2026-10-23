use super::*;

struct Relocated {
    host: HostContext,
    documents: PathBuf,
    saved: PathBuf,
    alias: PathBuf,
    id: onecad_core::ids::DocumentId,
}

async fn relocated_checkpoint(base: &Path) -> Relocated {
    let containers = base.join("Containers/Data/Application");
    let old = containers.join(uuid::Uuid::new_v4().to_string());
    let data_suffix = Path::new("Library/Application Support/data");
    let data = old.join(data_suffix);
    std::fs::create_dir_all(&data).unwrap();
    let (mut host, saved, id) = crashed_copy(&data).await;
    let documents = old.join("Documents");
    std::fs::create_dir_all(documents.join("Imported")).unwrap();
    std::fs::create_dir_all(documents.join("Inbox")).unwrap();
    let target = documents.join("Imported/plate.onecad");
    let alias = documents.join("Inbox/plate.onecad");
    std::fs::rename(saved, &target).unwrap();
    std::fs::copy(&target, &alias).unwrap();
    let mut marker = read_marker(&data, id).unwrap();
    marker.opened_path = Some(target);
    marker.source_paths = vec![alias];
    write_marker(&data, &marker).unwrap();
    let current = containers.join(uuid::Uuid::new_v4().to_string());
    std::fs::rename(&old, &current).unwrap();
    host.paths.app_data = Some(current.join(data_suffix));
    Relocated {
        host,
        documents: current.join("Documents"),
        saved: current.join("Documents/Imported/plate.onecad"),
        alias: current.join("Documents/Inbox/plate.onecad"),
        id,
    }
}

#[tokio::test]
async fn container_relocation_restores_checkpoint_and_aliases_through_second_recovery() {
    let area = tempfile::tempdir().unwrap();
    let moved = relocated_checkpoint(area.path()).await;
    let state = new_state();
    let offers = api::check_recovery_in_documents(&state, moved.host.clone(), &moved.documents)
        .await
        .unwrap();
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].original_path.as_deref(), moved.saved.to_str());
    let target = destination(&moved.documents, &offers[0]).unwrap();
    assert_eq!(target, moved.saved);
    let pending = state
        .peek_pending_recovery(moved.id)
        .unwrap()
        .marker
        .unwrap();
    assert_eq!(pending.source_paths, vec![moved.alias.clone()]);
    api::recover_document_at_path(
        &state,
        moved.host.clone(),
        moved.id.to_string(),
        true,
        Some(target),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        state.services().reservations().path_owner(&moved.alias),
        Some(state.host_id())
    );
    let root = moved.host.paths.app_data.as_deref().unwrap();
    let marker = read_marker(root, moved.id).unwrap();
    assert_eq!(marker.opened_path, Some(moved.saved.clone()));
    assert!(marker.source_paths.contains(&moved.alias));
    drop(state);
    mark_dead(root, moved.id);
    let restarted = new_state();
    api::check_recovery_in_documents(&restarted, moved.host.clone(), &moved.documents)
        .await
        .unwrap();
    api::recover_document(&restarted, moved.host.clone(), moved.id.to_string(), true)
        .await
        .unwrap()
        .unwrap();
    let projection = api::get_projection(&restarted).await.unwrap();
    assert_eq!(projection.variables[0].name, "RecoveredDepth");
    assert!(projection.dirty);
    api::save_document(&restarted, moved.host.clone(), None, None)
        .await
        .unwrap();
    api::close_document(&restarted, moved.host.clone())
        .await
        .unwrap();
    let reopened = new_state();
    api::open_document(
        &reopened,
        moved.host,
        moved.saved.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let projection = api::get_projection(&reopened).await.unwrap();
    assert_eq!(projection.variables[0].name, "RecoveredDepth");
    assert!(!projection.dirty);
}

#[tokio::test]
async fn mismatched_relocated_file_keeps_offer_and_checkpoint_and_uses_safe_restore_target() {
    let area = tempfile::tempdir().unwrap();
    let moved = relocated_checkpoint(area.path()).await;
    let backend = Arc::new(PendingBackend);
    DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend)
        .save(
            &moved.saved,
            SaveMeta {
                app_version: "test".into(),
                occt_fingerprint: None,
                created: "2026-10-23T00:00:00Z".into(),
                modified: "2026-10-23T00:00:00Z".into(),
            },
        )
        .unwrap();
    let saved_before = std::fs::read(&moved.saved).unwrap();
    let checkpoint = onecad_core::io::recovery::autosave_path(
        moved.host.paths.app_data.as_deref().unwrap(),
        moved.id,
    );
    let checkpoint_before = std::fs::read(&checkpoint).unwrap();
    let state = new_state();
    let offers = api::check_recovery_in_documents(&state, moved.host.clone(), &moved.documents)
        .await
        .unwrap();
    assert_eq!(offers.len(), 1);
    assert_ne!(offers[0].original_path.as_deref(), moved.saved.to_str());
    let target = destination(&moved.documents, &offers[0]).unwrap();
    assert_ne!(target, moved.saved);
    api::recover_document_at_path(
        &state,
        moved.host.clone(),
        moved.id.to_string(),
        true,
        Some(target),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(std::fs::read(&moved.saved).unwrap(), saved_before);
    assert_eq!(std::fs::read(checkpoint).unwrap(), checkpoint_before);
    assert!(api::get_projection(&state).await.unwrap().dirty);
}

#[tokio::test]
async fn explicit_discard_does_not_validate_or_modify_a_conflicting_saved_target() {
    let area = tempfile::tempdir().unwrap();
    let paths = crate::qa::QaPaths::from_args(
        [
            std::ffi::OsString::from("--qa-root"),
            area.path().join("qa").into_os_string(),
        ]
        .into_iter(),
    )
    .unwrap();
    let (host, original, id) = crashed_copy(&paths.data).await;
    let saved = paths.documents.join("plate.onecad");
    std::fs::rename(&original, &saved).unwrap();
    let mut marker = read_marker(&paths.data, id).unwrap();
    marker.opened_path = Some(saved.clone());
    write_marker(&paths.data, &marker).unwrap();
    let pending = Arc::new(PendingBackend);
    DocumentRuntime::new_blank(pending.clone(), pending.clone(), pending)
        .save(
            &saved,
            SaveMeta {
                app_version: "test".into(),
                occt_fingerprint: None,
                created: "2026-10-23T00:00:00Z".into(),
                modified: "2026-10-23T00:00:00Z".into(),
            },
        )
        .unwrap();
    let saved_before = std::fs::read(&saved).unwrap();
    let state = Arc::new(new_state());
    let offers = api::check_recovery(&state, host.clone()).await.unwrap();
    assert!(destination(&paths.documents, &offers[0]).is_err());
    let checkpoint = PathBuf::from(&offers[0].autosave_path);
    apply_file_choice(&state, host, &paths.documents, &offers[0], Choice::Discard)
        .await
        .unwrap();
    assert!(state.peek_pending_recovery(id).is_none());
    assert!(!checkpoint.exists());
    assert_eq!(std::fs::read(saved).unwrap(), saved_before);
    assert!(state.runtime.lock().await.is_none());
}

#[tokio::test]
async fn restore_escapes_corrupt_or_conflicting_saved_destinations_without_overwriting_them() {
    for corrupt in [true, false] {
        let area = tempfile::tempdir().unwrap();
        let (host, source, id) = crashed_copy(area.path()).await;
        let documents = area.path().join("documents");
        if corrupt {
            std::fs::write(&source, b"corrupt saved copy").unwrap();
        } else {
            let backend = Arc::new(PendingBackend);
            DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend)
                .save(
                    &source,
                    SaveMeta {
                        app_version: "test".into(),
                        occt_fingerprint: None,
                        created: "2026-10-23T00:00:00Z".into(),
                        modified: "2026-10-23T00:00:00Z".into(),
                    },
                )
                .unwrap();
        }
        let before = std::fs::read(&source).unwrap();
        let state = new_state();
        let offers = api::check_recovery(&state, host.clone()).await.unwrap();
        let checkpoint = Path::new(&offers[0].autosave_path);
        let checkpoint_before = std::fs::read(checkpoint).unwrap();
        assert!(
            apply_file_choice(
                &state,
                host.clone(),
                &documents,
                &offers[0],
                Choice::OpenSaved
            )
            .await
            .is_err()
        );
        let restored = apply_file_choice(
            &state,
            host.clone(),
            &documents,
            &offers[0],
            Choice::Restore,
        )
        .await
        .unwrap()
        .unwrap();
        assert_ne!(restored, source);
        assert!(restored.starts_with(&documents));
        assert!(!restored.exists());
        assert_eq!(std::fs::read(&source).unwrap(), before);
        assert_eq!(std::fs::read(checkpoint).unwrap(), checkpoint_before);
        assert_eq!(
            state.services().reservations().path_owner(&source),
            Some(state.host_id())
        );
        assert_eq!(
            state.services().reservations().path_owner(&restored),
            Some(state.host_id())
        );
        let projection = api::get_projection(&state).await.unwrap();
        assert!(projection.dirty);
        assert_eq!(projection.variables[0].name, "RecoveredDepth");
        assert_eq!(
            read_marker(area.path(), id).unwrap().opened_path,
            Some(restored.clone())
        );
        api::save_document(&state, host, None, None).await.unwrap();
        assert!(restored.is_file());
        assert_eq!(std::fs::read(&source).unwrap(), before);
    }
}

#[tokio::test]
async fn conflicting_fallback_does_not_prevent_restoring_an_orphan_checkpoint() {
    let area = tempfile::tempdir().unwrap();
    let (host, source, id) = crashed_copy(area.path()).await;
    let documents = area.path().join("documents");
    std::fs::remove_file(source).unwrap();
    let collision = documents.join(format!("recovered-{id}.onecad"));
    std::fs::write(&collision, b"unrelated existing file").unwrap();
    let state = new_state();
    let offers = api::check_recovery(&state, host.clone()).await.unwrap();
    let restored = apply_file_choice(&state, host, &documents, &offers[0], Choice::Restore)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(restored, collision);
    assert!(restored.starts_with(&documents));
    assert_eq!(
        std::fs::read(collision).unwrap(),
        b"unrelated existing file"
    );
    assert!(api::get_projection(&state).await.unwrap().dirty);
}
