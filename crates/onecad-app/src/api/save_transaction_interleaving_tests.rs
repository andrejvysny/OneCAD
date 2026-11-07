//! Deterministic save admission, publication and runtime replacement interleavings.
use super::*;

#[tokio::test]
async fn cancellation_before_admission_writes_nothing_and_releases_claim() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let state = Arc::new(state);
    add_variable(state.runtime.lock().await.as_mut().unwrap(), "dirty", 5.0);
    let target = area.path().join("cancelled.onecad");
    std::fs::write(&target, b"original").unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (_release, resume) = tokio::sync::oneshot::channel();
    *state.save_before_admission.lock().unwrap() = Some((started, resume));
    let task_state = state.clone();
    let path = target.to_string_lossy().into_owned();
    let task =
        tokio::spawn(async move { save_document(&task_state, host, Some(path), None).await });
    ready.await.unwrap();
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_err());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert!(state.runtime.lock().await.as_ref().unwrap().is_dirty());
    assert!(state.persistence.try_lock().is_ok());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_ok());
}

#[tokio::test]
async fn actual_rename_failure_preserves_target_and_releases_resources() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let target = area.path().join("occupied.onecad");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("sentinel"), b"original").unwrap();
    let before: std::collections::BTreeSet<_> = std::fs::read_dir(area.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    *state.save_io_hook.lock().unwrap() = Some(Box::new(|stage| {
        if stage == SaveTestStage::BeforeRename {
            return Ok(());
        }
        panic!("rename unexpectedly succeeded");
    }));
    let error = save_document(
        &state,
        host,
        Some(target.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("publish container:"), "{error}");
    assert_eq!(std::fs::read(target.join("sentinel")).unwrap(), b"original");
    let after: std::collections::BTreeSet<_> = std::fs::read_dir(area.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(after, before, "owned temporary file must be removed");
    assert!(state.persistence.try_lock().is_ok());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_ok());
}

#[tokio::test]
async fn competing_hosts_cannot_save_to_claimed_target() {
    let area = tempfile::tempdir().unwrap();
    let (first, host, _, _) = pending_copy(area.path(), false).await;
    let second = AppState::new_with_services(
        Arc::new(|| {
            let b = Arc::new(crate::worker::PendingBackend);
            (
                b.clone(),
                b.clone(),
                b.clone(),
                b.clone(),
                b.clone(),
                b.clone(),
                b.clone(),
                b.clone(),
                b.clone(),
                b,
            )
        }),
        first.services().clone(),
    );
    new_document(&second, host.clone()).await.unwrap();
    let first = Arc::new(first);
    let target = area.path().join("competing.onecad");
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let mut started = Some(started);
    *first.save_io_hook.lock().unwrap() = Some(Box::new(move |stage| {
        if stage == SaveTestStage::BeforeRename {
            started.take().unwrap().send(()).unwrap();
            wait.recv().unwrap();
        }
        Ok(())
    }));
    let task_state = first.clone();
    let task_host = host.clone();
    let path = target.to_string_lossy().into_owned();
    let task =
        tokio::spawn(async move { save_document(&task_state, task_host, Some(path), None).await });
    ready.await.unwrap();
    let result = save_document(
        &second,
        host,
        Some(target.to_string_lossy().into_owned()),
        None,
    )
    .await;
    assert!(
        matches!(result, Err(ApiError::AlreadyOpen { .. })),
        "{result:?}"
    );
    assert!(!target.exists());
    release.send(()).unwrap();
    assert!(task.await.unwrap().unwrap().clean);
    assert!(ContainerReader::open(&target).is_ok());
}

#[tokio::test]
async fn failed_old_save_cannot_modify_same_uuid_replacement() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let state = Arc::new(state);
    let original = area.path().join("replacement.onecad");
    save_document(
        &state,
        host.clone(),
        Some(original.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    let (id, old_session, revision) = {
        let guard = state.runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        (rt.document_uuid(), rt.runtime_session(), rt.revision())
    };
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let mut started = Some(started);
    *state.save_io_hook.lock().unwrap() = Some(Box::new(move |stage| {
        if stage == SaveTestStage::BeforeRename {
            started.take().unwrap().send(()).unwrap();
            wait.recv().unwrap();
            return Err(std::io::Error::other("injected write failure"));
        }
        Ok(())
    }));
    let target = area.path().join("failed-old.onecad");
    let task_state = state.clone();
    let path = target.to_string_lossy().into_owned();
    let task =
        tokio::spawn(async move { save_document(&task_state, host, Some(path), None).await });
    ready.await.unwrap();
    let backend = Arc::new(crate::worker::PendingBackend);
    let mut replacement =
        DocumentRuntime::open(&original, backend.clone(), backend.clone(), backend).unwrap();
    assert_eq!(replacement.document_uuid(), id);
    assert_eq!(replacement.revision(), revision);
    let session = replacement.runtime_session();
    assert_ne!(session, old_session);
    replacement.mark_recovered(Some(original.clone()), None);
    let generation = replacement.persisted_generation();
    let checkpoint = replacement.checkpoint_count();
    *state.runtime.lock().await = Some(replacement);
    let marker = SessionMarker {
        document_id: id,
        pid: 1,
        opened_path: Some(original.clone()),
        runtime_session: Some(session.clone()),
        persisted_generation: Some(generation),
        last_autosave: String::new(),
        title: None,
    };
    onecad_core::io::recovery::write_marker(area.path(), &marker).unwrap();
    let marker_path = onecad_core::io::recovery::marker_path(area.path(), id);
    let marker_bytes = std::fs::read(&marker_path).unwrap();
    let recovery = onecad_core::io::recovery::autosave_path(area.path(), id);
    std::fs::write(&recovery, b"replacement recovery").unwrap();
    release.send(()).unwrap();
    assert!(task.await.unwrap().is_err());
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    assert!(rt.matches_runtime_session(&session));
    assert_eq!(rt.path(), Some(original.as_path()));
    assert!(rt.is_dirty());
    assert_eq!(rt.persisted_generation(), generation);
    assert_eq!(rt.checkpoint_count(), checkpoint);
    drop(guard);
    assert_eq!(std::fs::read(marker_path).unwrap(), marker_bytes);
    assert_eq!(std::fs::read(recovery).unwrap(), b"replacement recovery");
    assert!(!target.exists());
    assert!(state.persistence.try_lock().is_ok());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_ok());
}

#[tokio::test]
async fn cancelled_caller_cannot_adopt_or_clear_same_uuid_replacement() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let state = Arc::new(state);
    add_variable(
        state.runtime.lock().await.as_mut().unwrap(),
        "old_payload",
        17.0,
    );
    let original = area.path().join("cancel-replacement.onecad");
    save_document(
        &state,
        host.clone(),
        Some(original.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    let backend = Arc::new(crate::worker::PendingBackend);
    let reopened =
        DocumentRuntime::open(&original, backend.clone(), backend.clone(), backend).unwrap();
    *state.runtime.lock().await = Some(reopened);
    let (id, old_session, revision) = {
        let guard = state.runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        (rt.document_uuid(), rt.runtime_session(), rt.revision())
    };
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let mut started = Some(started);
    *state.save_io_hook.lock().unwrap() = Some(Box::new(move |stage| {
        if stage == SaveTestStage::BeforeRename {
            started.take().unwrap().send(()).unwrap();
            wait.recv().unwrap();
        }
        Ok(())
    }));
    let target = area.path().join("cancelled-old.onecad");
    let task_state = state.clone();
    let path = target.to_string_lossy().into_owned();
    let task =
        tokio::spawn(async move { save_document(&task_state, host, Some(path), None).await });
    ready.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(state.persistence.try_lock().is_err());
    let backend = Arc::new(crate::worker::PendingBackend);
    let mut replacement =
        DocumentRuntime::open(&original, backend.clone(), backend.clone(), backend).unwrap();
    assert_eq!(replacement.document_uuid(), id);
    assert_eq!(replacement.revision(), revision);
    let session = replacement.runtime_session();
    assert_ne!(session, old_session);
    replacement.mark_recovered(Some(original.clone()), None);
    let generation = replacement.persisted_generation();
    let checkpoints = replacement.checkpoint_count();
    *state.runtime.lock().await = Some(replacement);
    let marker = SessionMarker {
        document_id: id,
        pid: 1,
        opened_path: Some(original.clone()),
        runtime_session: Some(session.clone()),
        persisted_generation: Some(generation),
        last_autosave: String::new(),
        title: None,
    };
    onecad_core::io::recovery::write_marker(area.path(), &marker).unwrap();
    let marker_path = onecad_core::io::recovery::marker_path(area.path(), id);
    let marker_bytes = std::fs::read(&marker_path).unwrap();
    let recovery = onecad_core::io::recovery::autosave_path(area.path(), id);
    std::fs::write(&recovery, b"replacement recovery").unwrap();
    release.send(()).unwrap();
    let lane = state.persistence.lock().await;
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    assert!(rt.matches_runtime_session(&session));
    assert_eq!(rt.path(), Some(original.as_path()));
    assert!(rt.is_dirty());
    assert_eq!(rt.persisted_generation(), generation);
    assert_eq!(rt.checkpoint_count(), checkpoints);
    drop(guard);
    assert_eq!(std::fs::read(marker_path).unwrap(), marker_bytes);
    assert_eq!(std::fs::read(recovery).unwrap(), b"replacement recovery");
    let saved = ContainerReader::open(&target).unwrap();
    let expected = ContainerReader::open(&original).unwrap();
    assert_eq!(
        serde_json::to_value(saved.document()).unwrap(),
        serde_json::to_value(expected.document()).unwrap()
    );
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_ok());
    drop(lane);
}
