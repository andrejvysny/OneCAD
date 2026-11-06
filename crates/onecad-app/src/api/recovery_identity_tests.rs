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
        runtime_session: None,
        persisted_generation: None,
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

#[tokio::test]
async fn queued_save_does_not_adopt_into_reopened_same_document() {
    use std::{future::Future, task::Poll};
    let area = tempfile::tempdir().unwrap();
    let (state, host, original, _) = pending_copy(area.path(), false).await;
    let backend = Arc::new(PendingBackend);
    let old = DocumentRuntime::open(&original, backend.clone(), backend.clone(), backend.clone())
        .unwrap();
    *state.runtime.lock().await = Some(old);
    let lane = state.persistence.lock().await;
    let target = area.path().join("queued.onecad");
    let mut save = Box::pin(save_document(
        &state,
        host,
        Some(target.to_string_lossy().into_owned()),
        None,
    ));
    std::future::poll_fn(|cx| {
        assert!(save.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let replacement =
        DocumentRuntime::open(&original, backend.clone(), backend.clone(), backend).unwrap();
    let session = replacement.runtime_session();
    *state.runtime.lock().await = Some(replacement);
    drop(lane);
    assert!(save.await.is_err());
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    assert!(rt.matches_runtime_session(&session));
    assert_eq!(rt.path(), Some(original.as_path()));
}

#[tokio::test]
async fn older_queued_save_preserves_newer_recovery() {
    use std::{future::Future, task::Poll};
    let area = tempfile::tempdir().unwrap();
    let (state, _, _, _) = pending_copy(area.path(), false).await;
    let (id, session, generation, payload) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        (
            rt.document_uuid(),
            rt.runtime_session(),
            rt.persisted_generation(),
            rt.build_save_payload(save_meta(), SaveCaches::none()),
        )
    };
    let lane = state.persistence.lock().await;
    let target = area.path().join("older.onecad");
    let mut save = Box::pin(autosave::write_save_payload_claimed(
        &state.persistence,
        &target,
        payload,
        Some(area.path()),
        id,
        None,
        Some((session.clone(), generation)),
    ));
    std::future::poll_fn(|cx| {
        assert!(save.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let marker = SessionMarker {
        document_id: id,
        pid: 1,
        opened_path: None,
        runtime_session: Some(session),
        persisted_generation: Some(generation + 1),
        last_autosave: String::new(),
        title: None,
    };
    onecad_core::io::recovery::write_marker(area.path(), &marker).unwrap();
    let recovery = onecad_core::io::recovery::autosave_path(area.path(), id);
    std::fs::write(&recovery, b"newer work").unwrap();
    drop(lane);
    save.await.unwrap();
    assert_eq!(std::fs::read(&recovery).unwrap(), b"newer work");
    assert!(onecad_core::io::recovery::marker_path(area.path(), id).exists());
}

#[test]
fn cancelled_save_as_retains_target_claim_until_write_finishes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let area = tempfile::tempdir().unwrap();
        let (state, host, _, _) = pending_copy(area.path(), false).await;
        let state = Arc::new(state);
        let target = area.path().join("claimed.onecad");
        let (release, wait) = std::sync::mpsc::channel();
        let (started, ready) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = started.send(());
            let _ = wait.recv();
        });
        ready.await.unwrap();
        let task_state = state.clone();
        let path = target.to_string_lossy().into_owned();
        let save =
            tokio::spawn(async move { save_document(&task_state, host, Some(path), None).await });
        tokio::task::yield_now().await;
        assert!(state.persistence.try_lock().is_err());
        save.abort();
        let _ = save.await;
        let retained = state
            .services()
            .reservations()
            .claim_path(999, &target)
            .is_err();
        release.send(()).unwrap();
        blocker.await.unwrap();
        assert!(retained, "target claim escaped while write was admitted");
        let _drained = state.persistence.lock().await;
        assert_eq!(
            state.runtime.lock().await.as_ref().unwrap().path(),
            Some(target.as_path())
        );
        assert!(state
            .services()
            .reservations()
            .claim_path(999, &target)
            .is_err());
    });
}

#[tokio::test]
async fn cancelled_close_finishes_recovery_and_releases_claims_after_lane_drains() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, offer) = pending_copy(area.path(), false).await;
    recover_document(&state, host.clone(), offer.document_id.to_string(), true)
        .await
        .unwrap();
    let id = offer.document_id;
    let state = Arc::new(state);
    let lane = state.persistence.lock().await;
    let task_state = state.clone();
    let close = tokio::spawn(async move { close_document(&task_state, host).await });
    while state.runtime.lock().await.is_some() {
        tokio::task::yield_now().await;
    }
    close.abort();
    let _ = close.await;
    assert!(state
        .services()
        .reservations()
        .claim_document(999, id)
        .is_err());
    drop(lane);
    let _drained = state.persistence.lock().await;
    let marker: SessionMarker = serde_json::from_slice(
        &std::fs::read(onecad_core::io::recovery::marker_path(area.path(), id)).unwrap(),
    )
    .unwrap();
    assert_eq!(marker.pid, 0);
    assert!(state
        .services()
        .reservations()
        .claim_document(999, id)
        .is_ok());
}

#[tokio::test]
async fn delayed_old_save_captures_after_newer_save_instead_of_overwriting_it() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let state = Arc::new(state);
    let target = area.path().join("ordered.onecad");
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    *state.save_before_admission.lock().unwrap() = Some((started, resume));
    let old_state = state.clone();
    let old_host = host.clone();
    let old_path = target.to_string_lossy().into_owned();
    let old =
        tokio::spawn(
            async move { save_document(&old_state, old_host, Some(old_path), None).await },
        );
    ready.await.unwrap();
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        rt.apply(onecad_core::edit::EditCommand::AddVariable {
            variable: onecad_core::document::variables::Variable {
                id: onecad_core::ids::VariableId::new(),
                name: "later".into(),
                value: onecad_core::document::variables::Scalar::new(42.0),
                unit: Default::default(),
            },
        })
        .unwrap();
    }
    assert!(
        save_document(
            &state,
            host,
            Some(target.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    release.send(()).unwrap();
    assert!(old.await.unwrap().unwrap().clean);
    let loaded = onecad_core::io::container::ContainerReader::open(&target).unwrap();
    assert!(loaded
        .document()
        .variables
        .iter()
        .any(|variable| variable.name == "later"));
}

#[tokio::test]
async fn restored_pair_survives_second_crash_and_save_retires_it() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, offer) = pending_copy(area.path(), false).await;
    recover_document(&state, host.clone(), offer.document_id.to_string(), true)
        .await
        .unwrap();
    let marker: SessionMarker = serde_json::from_slice(
        &std::fs::read(onecad_core::io::recovery::marker_path(
            area.path(),
            offer.document_id,
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(marker.runtime_session.is_some());
    assert!(marker.persisted_generation.is_some());
    let offers = onecad_core::io::recovery::scan_recoverable(
        area.path(),
        |_| false,
        std::time::SystemTime::now(),
    )
    .unwrap();
    assert!(offers
        .iter()
        .any(|item| item.document_id == offer.document_id));
    assert!(
        save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    close_document(&state, host.clone()).await.unwrap();
    assert!(check_recovery(&state, host).await.unwrap().is_empty());
}

#[tokio::test]
async fn cancelled_shutdown_still_drains_close_and_releases_claims() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, offer) = pending_copy(area.path(), false).await;
    recover_document(&state, host.clone(), offer.document_id.to_string(), true)
        .await
        .unwrap();
    let id = offer.document_id;
    let state = Arc::new(state);
    let lane = state.persistence.lock().await;
    let task_state = state.clone();
    let shutdown = tokio::spawn(async move { shutdown_host(&task_state, host, None).await });
    while state.runtime.lock().await.is_some() {
        tokio::task::yield_now().await;
    }
    shutdown.abort();
    let _ = shutdown.await;
    assert!(state
        .services()
        .reservations()
        .claim_document(999, id)
        .is_err());
    drop(lane);
    let _drained = state.persistence.lock().await;
    let marker: SessionMarker = serde_json::from_slice(
        &std::fs::read(onecad_core::io::recovery::marker_path(area.path(), id)).unwrap(),
    )
    .unwrap();
    assert_eq!(marker.pid, 0);
    assert!(state
        .services()
        .reservations()
        .claim_document(999, id)
        .is_ok());
    assert!(state.ensure_document_capacity().is_err());
}

#[tokio::test]
async fn reject_waits_for_lane_and_accept_rechecks_pending_offer() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, offer) = pending_copy(area.path(), false).await;
    let state = Arc::new(state);
    let lane = state.persistence.lock().await;
    let reject_state = state.clone();
    let reject_host = host.clone();
    let id = offer.document_id.to_string();
    let reject_id = id.clone();
    let reject = tokio::spawn(async move {
        recover_document(&reject_state, reject_host, reject_id, false).await
    });
    tokio::task::yield_now().await;
    assert!(state.peek_pending_recovery(offer.document_id).is_some());
    assert!(offer.autosave_path.exists());
    let accept_state = state.clone();
    let accept = tokio::spawn(async move { recover_document(&accept_state, host, id, true).await });
    tokio::task::yield_now().await;
    drop(lane);
    assert!(reject.await.unwrap().unwrap().is_none());
    assert!(accept.await.unwrap().unwrap().is_none());
    assert!(!offer.autosave_path.exists());
}

#[tokio::test]
async fn failed_recovery_marker_write_preserves_offer_and_existing_runtime() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, offer) = pending_copy(area.path(), false).await;
    let before = get_document_state(&state).await.unwrap().runtime_session;
    let marker = onecad_core::io::recovery::marker_path(area.path(), offer.document_id);
    std::fs::remove_file(&marker).unwrap();
    std::fs::create_dir(&marker).unwrap();
    assert!(
        recover_document(&state, host, offer.document_id.to_string(), true)
            .await
            .is_err()
    );
    assert!(state.peek_pending_recovery(offer.document_id).is_some());
    assert!(offer.autosave_path.exists());
    assert_eq!(
        get_document_state(&state).await.unwrap().runtime_session,
        before
    );
}

#[tokio::test]
async fn open_paused_before_publication_cannot_reopen_shutdown_host() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, alias, offer) = pending_copy(area.path(), false).await;
    let state = Arc::new(state);
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    *state.open_before_publish.lock().unwrap() = Some((started, resume));
    let opening_state = state.clone();
    let opening_host = host.clone();
    let open = tokio::spawn(async move {
        open_document(
            &opening_state,
            opening_host,
            alias.to_string_lossy().into_owned(),
            Some("openSaved".into()),
        )
        .await
    });
    ready.await.unwrap();
    shutdown_host(&state, host, None).await.unwrap();
    release.send(()).unwrap();
    assert!(open.await.unwrap().is_err());
    assert!(state.runtime.lock().await.is_none());
    assert!(state.peek_pending_recovery(offer.document_id).is_some());
    assert!(offer.autosave_path.exists());
}

#[test]
fn supersession_requires_valid_same_document_session_and_generation() {
    let area = tempfile::tempdir().unwrap();
    let id = DocumentId::new();
    let path = onecad_core::io::recovery::autosave_path(area.path(), id);
    let marker_path = onecad_core::io::recovery::marker_path(area.path(), id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let marker = SessionMarker {
        document_id: id,
        pid: 1,
        opened_path: None,
        runtime_session: Some("owner".into()),
        persisted_generation: Some(2),
        last_autosave: String::new(),
        title: None,
    };
    for bytes in [
        None,
        Some(b"invalid".to_vec()),
        Some(
            serde_json::to_vec(&SessionMarker {
                runtime_session: Some("foreign".into()),
                ..marker.clone()
            })
            .unwrap(),
        ),
        Some(
            serde_json::to_vec(&SessionMarker {
                document_id: DocumentId::new(),
                ..marker.clone()
            })
            .unwrap(),
        ),
        Some(
            serde_json::to_vec(&SessionMarker {
                persisted_generation: None,
                ..marker.clone()
            })
            .unwrap(),
        ),
    ] {
        std::fs::write(&path, b"recovery").unwrap();
        let _ = std::fs::remove_file(&marker_path);
        if let Some(bytes) = bytes {
            std::fs::write(&marker_path, bytes).unwrap();
        }
        autosave::clear_superseded_recovery(area.path(), id, "owner", 2);
        assert_eq!(std::fs::read(&path).unwrap(), b"recovery");
    }
    onecad_core::io::recovery::write_marker(area.path(), &marker).unwrap();
    autosave::clear_superseded_recovery(area.path(), id, "owner", 2);
    assert!(!path.exists());
    assert!(!marker_path.exists());
}

#[tokio::test]
async fn dirty_close_preserves_foreign_recovery_marker() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, offer) = pending_copy(area.path(), false).await;
    recover_document(&state, host.clone(), offer.document_id.to_string(), true)
        .await
        .unwrap();
    let marker_path = onecad_core::io::recovery::marker_path(area.path(), offer.document_id);
    let mut marker: SessionMarker =
        serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
    marker.runtime_session = Some("foreign".into());
    onecad_core::io::recovery::write_marker(area.path(), &marker).unwrap();
    let before = std::fs::read(&marker_path).unwrap();
    close_document(&state, host).await.unwrap();
    assert_eq!(std::fs::read(marker_path).unwrap(), before);
    assert!(offer.autosave_path.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_published_runtime_and_claims_to_commit_together() {
    use std::{future::Future, task::Poll};
    let area = tempfile::tempdir().unwrap();
    let (state, host, alias, offer) = pending_copy(area.path(), false).await;
    let state = Arc::new(state);
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = std::sync::mpsc::channel();
    *state.open_after_publish.lock().unwrap() = Some((started, resume));
    let opening_state = state.clone();
    let opening_host = host.clone();
    let path = alias.to_string_lossy().into_owned();
    let open = tokio::spawn(async move {
        open_document(&opening_state, opening_host, path, Some("openSaved".into())).await
    });
    ready.await.unwrap();
    let mut shutdown = Box::pin(shutdown_host(&state, host, None));
    std::future::poll_fn(|cx| {
        assert!(shutdown.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(state.runtime.try_lock().is_err());
    release.send(()).unwrap();
    let (open_result, shutdown_result) = tokio::join!(open, shutdown);
    open_result.unwrap().unwrap();
    shutdown_result.unwrap();
    assert!(state.runtime.lock().await.is_none());
    assert!(state
        .services()
        .reservations()
        .claim_document(999, offer.document_id)
        .is_ok());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &alias)
        .is_ok());
    assert!(state.ensure_document_capacity().is_err());
}
