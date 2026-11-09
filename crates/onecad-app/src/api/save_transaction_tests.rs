#[path = "save_transaction_reopen_support.rs"]
mod reopen_support;
#[path = "save_transaction_worker_support.rs"]
mod worker_support;

use super::*;
use crate::api::recovery_identity_tests::pending_copy;
use onecad_core::io::container::{ContainerReader, SaveTestStage};
use std::sync::Arc;

use worker_support::add_variable;

#[path = "save_admission_tests.rs"]
mod admission_tests;

#[tokio::test]
async fn failed_directory_sync_keeps_old_path_dirty_recovery_and_releases_new_claim() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let old = area.path().join("old.onecad");
    save_document(
        &state,
        host.clone(),
        Some(old.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    let (id, session, generation) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        add_variable(rt, "unsaved", 42.0);
        (
            rt.document_uuid(),
            rt.runtime_session(),
            rt.persisted_generation(),
        )
    };
    let marker = SessionMarker {
        document_id: id,
        pid: 1,
        opened_path: Some(old.clone()),
        runtime_session: Some(session),
        persisted_generation: Some(generation),
        last_autosave: String::new(),
        title: None,
    };
    onecad_core::io::recovery::write_marker(area.path(), &marker).unwrap();
    let marker_path = onecad_core::io::recovery::marker_path(area.path(), id);
    let marker_bytes = std::fs::read(&marker_path).unwrap();
    let recovery = onecad_core::io::recovery::autosave_path(area.path(), id);
    std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
    std::fs::write(&recovery, b"retained recovery").unwrap();
    *state.save_io_hook.lock().unwrap() = Some(Box::new(|stage| {
        if stage == SaveTestStage::BeforeDirectorySync {
            Err(std::io::Error::other("injected sync failure"))
        } else {
            Ok(())
        }
    }));
    let target = area.path().join("new.onecad");
    let error = save_document(
        &state,
        host,
        Some(target.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("target published but parent directory sync failed; durability unconfirmed"),
        "{error}"
    );
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    assert_eq!(rt.path(), Some(old.as_path()));
    assert!(rt.is_dirty());
    drop(guard);
    assert_eq!(std::fs::read(&recovery).unwrap(), b"retained recovery");
    assert_eq!(std::fs::read(marker_path).unwrap(), marker_bytes);
    assert!(!ContainerReader::open(&old)
        .unwrap()
        .document()
        .variables
        .iter()
        .any(|v| v.name == "unsaved"));
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &old)
        .is_err());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_ok());
    assert!(ContainerReader::open(&target)
        .unwrap()
        .document()
        .variables
        .iter()
        .any(|v| v.name == "unsaved"));
}

#[tokio::test]
async fn aborted_inflight_writer_retains_lane_and_claim_until_publication() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let state = Arc::new(state);
    let target = area.path().join("ordered.onecad");
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
    let a_state = state.clone();
    let a_host = host.clone();
    let path = target.to_string_lossy().into_owned();
    let a = tokio::spawn(async move { save_document(&a_state, a_host, Some(path), None).await });
    ready.await.unwrap();
    assert!(!target.exists());
    a.abort();
    assert!(a.await.unwrap_err().is_cancelled());
    assert!(state.persistence.try_lock().is_err());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_err());
    add_variable(state.runtime.lock().await.as_mut().unwrap(), "newest", 42.0);
    use std::{future::Future, pin::Pin, task::Poll};
    let (b_started, mut b_ready) = tokio::sync::oneshot::channel();
    let (b_resume, b_wait) = tokio::sync::oneshot::channel();
    *state.save_before_admission.lock().unwrap() = Some((b_started, b_wait));
    let path = target.to_string_lossy().into_owned();
    let mut b = Box::pin(save_document(&state, host, Some(path), None));
    std::future::poll_fn(|cx| {
        assert!(b.as_mut().poll(cx).is_pending());
        match Pin::new(&mut b_ready).poll(cx) {
            Poll::Ready(result) => {
                result.unwrap();
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    })
    .await;
    b_resume.send(()).unwrap();
    std::future::poll_fn(|cx| {
        assert!(
            b.as_mut().poll(cx).is_pending(),
            "writer B must wait on the held lane"
        );
        Poll::Ready(())
    })
    .await;
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_err());
    release.send(()).unwrap();
    assert!(b.await.unwrap().clean);
    let loaded = ContainerReader::open(&target).unwrap();
    assert!(loaded
        .document()
        .variables
        .iter()
        .any(|v| v.name == "newest"));
}

#[tokio::test]
async fn in_flight_save_cannot_adopt_into_same_uuid_replacement() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let state = Arc::new(state);
    let original = area.path().join("same-uuid.onecad");
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
        }
        Ok(())
    }));
    let target = area.path().join("old-save.onecad");
    let task_state = state.clone();
    let path = target.to_string_lossy().into_owned();
    let task =
        tokio::spawn(async move { save_document(&task_state, host, Some(path), None).await });
    ready.await.unwrap();
    let backend = Arc::new(crate::worker::PendingBackend);
    let replacement =
        DocumentRuntime::open(&original, backend.clone(), backend.clone(), backend).unwrap();
    assert_eq!(replacement.document_uuid(), id);
    assert_eq!(replacement.revision(), revision);
    let session = replacement.runtime_session();
    assert_ne!(session, old_session);
    let checkpoint = replacement.checkpoint_count();
    let clean = replacement.is_dirty();
    let generation = replacement.persisted_generation();
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
    let recovery = onecad_core::io::recovery::autosave_path(area.path(), id);
    std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
    std::fs::write(&recovery, b"replacement recovery").unwrap();
    release.send(()).unwrap();
    assert!(matches!(task.await.unwrap(), Err(ApiError::NoDocument(_))));
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    assert!(rt.matches_runtime_session(&session));
    assert_eq!(rt.path(), Some(original.as_path()));
    assert_eq!(rt.is_dirty(), clean);
    assert_eq!(rt.persisted_generation(), generation);
    assert_eq!(rt.checkpoint_count(), checkpoint);
    drop(guard);
    assert_eq!(std::fs::read(&recovery).unwrap(), b"replacement recovery");
    assert!(onecad_core::io::recovery::marker_path(area.path(), id).exists());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &target)
        .is_ok());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &original)
        .is_err());
    assert_eq!(ContainerReader::open(&target).unwrap().document().id, id);
}

#[tokio::test]
async fn in_flight_save_cannot_clear_later_persisted_selection_promotion() {
    use onecad_core::ids::{ElementId, TopoKey};
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let state = Arc::new(state);
    let (runtime, body, snapshot) = crate::document_runtime::tests::promotable_fake_runtime().await;
    *state.runtime.lock().await = Some(runtime);
    let target = area.path().join("metadata.onecad");
    save_document(
        &state,
        host.clone(),
        Some(target.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    let before = ContainerReader::open(&target).unwrap();
    let geometry = serde_json::to_value(before.document().timeline.records()).unwrap();
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
    let task_state = state.clone();
    let task_host = host.clone();
    let task = tokio::spawn(async move { save_document(&task_state, task_host, None, None).await });
    ready.await.unwrap();
    let promoted = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let revision = rt.revision();
        let generation = rt.persisted_generation();
        let promoted = rt
            .promote_selection(snapshot, body, vec![(TopoKey::new("f:22"), None)])
            .await
            .unwrap();
        assert_eq!(rt.revision(), revision);
        assert!(rt.persisted_generation() > generation);
        assert!(rt.is_dirty());
        ElementId::new(&promoted[0].element_id)
    };
    release.send(()).unwrap();
    assert!(!task.await.unwrap().unwrap().clean);
    assert!(state.runtime.lock().await.as_ref().unwrap().is_dirty());
    assert!(ContainerReader::open(&target)
        .unwrap()
        .document()
        .elements
        .get(&promoted)
        .is_none());
    assert!(save_document(&state, host, None, None).await.unwrap().clean);
    let after = ContainerReader::open(&target).unwrap();
    assert!(after.document().elements.get(&promoted).is_some());
    assert_eq!(
        serde_json::to_value(after.document().timeline.records()).unwrap(),
        geometry
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a production worker"]
async fn real_worker_in_flight_promotion_survives_save_and_reopen() {
    use crate::worker::{wire::body_id_wire, ElementQuery};
    use onecad_core::ids::{ElementId, TopoKey};
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    let area = tempfile::tempdir().unwrap();
    let host = worker_support::host(area.path());
    let state = Arc::new(worker_support::real_state(&host));
    new_document(&state, host.clone()).await.unwrap();
    assert!(
        state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let (body, snapshot) = worker_support::create_body(&state).await;
    let target = area.path().join("real-worker.onecad");
    assert!(
        save_document(
            &state,
            host.clone(),
            Some(target.to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    let original_mass = query_mass_properties(&state, body_id_wire(body), None)
        .await
        .unwrap()
        .volume;
    assert!(
        (original_mass - 75.0 * std::f64::consts::PI).abs() < 1e-6,
        "{original_mass}"
    );
    let worker = state.live_worker().unwrap();
    assert!(worker
        .classify_element_by_topo_key(body, "f:1")
        .await
        .unwrap()
        .is_some());
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
    let task_state = state.clone();
    let task_host = host.clone();
    let task = tokio::spawn(async move { save_document(&task_state, task_host, None, None).await });
    ready.await.unwrap();
    let element = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let revision = rt.revision();
        let generation = rt.persisted_generation();
        let picks = rt
            .promote_selection(snapshot, body, vec![(TopoKey::new("f:1"), None)])
            .await
            .unwrap();
        assert_eq!(rt.revision(), revision);
        assert!(rt.persisted_generation() > generation);
        ElementId::new(&picks[0].element_id)
    };
    release.send(()).unwrap();
    assert!(!task.await.unwrap().unwrap().clean);
    assert!(state.runtime.lock().await.as_ref().unwrap().is_dirty());
    assert!(
        save_document(&state, host.clone(), None, None)
            .await
            .unwrap()
            .clean
    );
    let saved = ContainerReader::open(&target).unwrap();
    let evidence = serde_json::to_value(saved.document().elements.get(&element).unwrap()).unwrap();
    close_document(&state, host.clone()).await.unwrap();
    drop(worker);
    drop(state);
    let mut host2 = worker_support::host(&area.path().join("reopen"));
    let mut events = reopen_support::capture(&mut host2);
    let reopened = worker_support::real_state(&host2);
    open_document(
        &reopened,
        host2.clone(),
        target.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(
        reopened
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let session = reopened
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .runtime_session();
    reopen_support::scheduled_bound(&reopened, &mut events, &session, body, &element).await;
    let mass = query_mass_properties(&reopened, body_id_wire(body), None)
        .await
        .unwrap()
        .volume;
    assert!(
        (mass - original_mass).abs() < 1e-6,
        "before {original_mass}, after {mass}"
    );
    let saved = ContainerReader::open(&target).unwrap();
    assert_eq!(
        serde_json::to_value(saved.document().elements.get(&element).unwrap()).unwrap(),
        evidence
    );
    close_document(&reopened, host2).await.unwrap();
}

#[tokio::test]
async fn save_as_missing_parent_refuses_before_io_and_keeps_old_state() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let old = area.path().join("old-existing.onecad");
    save_document(
        &state,
        host.clone(),
        Some(old.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    add_variable(state.runtime.lock().await.as_mut().unwrap(), "dirty", 5.0);
    let parent = area.path().join("absent");
    let target = parent.join("new.onecad");
    let error = save_document(
        &state,
        host,
        Some(target.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("parent directory is unavailable"));
    assert!(!parent.exists());
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    assert_eq!(rt.path(), Some(old.as_path()));
    assert!(rt.is_dirty());
    assert!(state
        .services()
        .reservations()
        .claim_path(999, &old)
        .is_err());
}

#[path = "save_transaction_interleaving_tests.rs"]
mod interleavings;
