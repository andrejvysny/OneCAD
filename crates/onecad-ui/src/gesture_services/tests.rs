use super::*;
use onecad_core::{
    ids::{DocumentRevision, SketchId, SnapshotId, WorkerEpoch},
    regen::HistoryPrefixHash,
};
use onecad_interaction::{
    DocumentContext,
    sketch::{SketchSessionKey, gesture::GestureScope},
};
use onecad_scene::{SceneIdentity, ViewSnapshot};

fn tag() -> GestureTag {
    let document_id = uuid::Uuid::from_u128(1).to_string();
    let runtime_session = uuid::Uuid::from_u128(2).to_string();
    let scene = SceneIdentity {
        session: 1,
        scene: 2,
    };
    GestureTag {
        scope: GestureScope {
            key: SketchSessionKey {
                document_id: document_id.clone(),
                runtime_session: runtime_session.clone(),
                sketch_id: SketchId::new(),
                generation: 4,
            },
            document: DocumentContext {
                document_id,
                runtime_session,
                revision: 7,
                snapshot_id: SnapshotId(9),
                scene,
            },
            edit_session: uuid::Uuid::from_u128(3).to_string(),
            capture: ViewSnapshot::new(
                scene,
                5,
                Default::default(),
                onecad_scene::Viewport {
                    width: 800.0,
                    height: 600.0,
                    scale_factor: 1.0,
                },
            )
            .unwrap()
            .identity(),
        },
        gesture: 8,
        sequence: 0,
    }
}

fn expected(tag: &GestureTag) -> api::PreviewContext {
    api::PreviewContext {
        document_id: tag.scope.document.document_id.clone(),
        runtime_session: tag.scope.document.runtime_session.clone(),
        revision: DocumentRevision(7),
        worker_epoch: WorkerEpoch(11),
        snapshot_id: SnapshotId(9),
        expected_base_hash: HistoryPrefixHash::empty(),
    }
}

#[test]
fn cancel_after_failed_begin_has_terminal_ack_without_an_owner() {
    let (sender, receiver) = mpsc::channel();
    let mut tag = tag();
    tag.sequence = 1;
    unowned(
        GestureEffect::Cancel {
            tag: tag.clone(),
            worker_id: None,
        },
        &sender,
        None,
    );
    let Message::Gesture { delivery, .. } = receiver.try_recv().unwrap() else {
        panic!("gesture reply required")
    };
    assert_eq!(delivery.tag, tag);
    assert!(
        matches!(delivery.actions.as_slice(), [GestureAction::Cancelled { tag: actual }] if actual == &tag)
    );
    assert!(delivery.error.is_none());
    assert!(!delivery.accepted);
}

#[test]
fn terminal_retains_single_workflow_admission_until_consumed() {
    let permits = Arc::new(tokio::sync::Semaphore::new(1));
    let permit = permits.clone().try_acquire_owned().unwrap();
    let (sender, receiver) = mpsc::channel();
    send(
        &sender,
        failed(tag(), "begin rejected".into()),
        Some(permit),
    );
    assert!(permits.clone().try_acquire_owned().is_err());
    let terminal = receiver.try_recv().unwrap();
    assert!(permits.clone().try_acquire_owned().is_err());
    drop(terminal);
    assert!(permits.try_acquire_owned().is_ok());
}

#[test]
fn native_scope_preserves_captured_worker_epoch_and_hash_and_refuses_stale_context() {
    let tag = tag();
    let expected = expected(&tag);
    let native = scope::native(&tag, &expected).unwrap();
    assert_eq!(native.worker_epoch, WorkerEpoch(11));
    assert_eq!(native.expected_base_hash, expected.expected_base_hash);
    assert_eq!(native.source.capture, tag.scope.capture.view);
    assert_eq!(native.source.generation, tag.scope.key.generation);
    for axis in 0..4 {
        let mut stale = expected.clone();
        match axis {
            0 => stale.document_id = uuid::Uuid::from_u128(77).to_string(),
            1 => stale.runtime_session.push_str("-obsolete"),
            2 => stale.revision.0 += 1,
            _ => stale.snapshot_id.0 += 1,
        }
        assert!(scope::native(&tag, &stale).is_err());
    }
}

#[tokio::test]
async fn bounded_result_queue_backpressures_without_overwriting_terminal_results() {
    let (sender, mut receiver) = channel::channel(CAPACITY);
    for sequence in 0..CAPACITY {
        let mut tag = tag();
        tag.sequence = sequence as u64;
        sender
            .send(jobs::ResultMessage {
                tag,
                result: Ok(jobs::Reply::Cancelled),
            })
            .await
            .unwrap();
    }
    let pending = jobs::spawn(tag(), sender, async { Ok(jobs::Reply::Cancelled) });
    tokio::task::yield_now().await;
    assert!(!pending.is_finished());
    assert_eq!(receiver.recv().await.unwrap().tag.sequence, 0);
    assert_eq!(receiver.recv().await.unwrap().tag.sequence, 1);
    assert!(matches!(
        receiver.recv().await.unwrap().result,
        Ok(jobs::Reply::Cancelled)
    ));
    pending.await.unwrap();
}

#[test]
fn arc_center_keeps_typed_role_but_sends_entity_body_worker_target() {
    use onecad_core::{ids::EntityId, math::Vec2, sketch::CurvePosition};
    use onecad_interaction::sketch::{
        SketchTarget,
        gesture::{GestureIntent, GestureKind},
    };
    let arc = EntityId::new();
    let center = EntityId::new();
    let intent = GestureIntent {
        target: SketchTarget::Point {
            entity: arc,
            role: CurvePosition::Center,
        },
        kind: GestureKind::EntityBody,
        entity: arc,
        role: Some(CurvePosition::Center),
        drag_point: center,
        grab: Vec2::new(0.0, 0.0).unwrap(),
    };
    let target = scope::target(&intent).unwrap();
    assert_eq!(target.kind, onecad_app::worker::wire::DragKind::EntityBody);
    assert_eq!(target.entity, arc);
    assert_eq!(target.role, None);
    assert_eq!(intent.role, Some(CurvePosition::Center));
}

#[tokio::test]
async fn completion_observer_is_single_and_survives_unrelated_global_consumption() {
    let slot = Arc::new(std::sync::Mutex::new(None));
    let expected = expected(&tag());
    let reservation = terminal::Reservation::new(slot.clone(), &expected).unwrap();
    assert!(terminal::Reservation::new(slot.clone(), &expected).is_err());
    let event = crate::bridge::BackendEvent::WorkerFailed;
    let global = crate::event_mailbox::EventMailbox::default();
    global.emit(event.clone()).unwrap();
    reservation.mailbox.emit(event).unwrap();
    assert!(matches!(
        global.next().await,
        crate::bridge::BackendEvent::WorkerFailed
    ));
    assert!(matches!(
        reservation.mailbox.next().await,
        crate::bridge::BackendEvent::WorkerFailed
    ));
    drop(reservation);
    assert!(slot.lock().unwrap().is_none());
}

#[tokio::test]
async fn aborting_pending_end_wait_removes_its_completion_observer() {
    let slot = Arc::new(std::sync::Mutex::new(None));
    let reservation = terminal::Reservation::new(slot.clone(), &expected(&tag())).unwrap();
    let task = tokio::spawn(async move {
        let _reservation = reservation;
        std::future::pending::<()>().await;
    });
    tokio::task::yield_now().await;
    assert!(slot.lock().unwrap().is_some());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(slot.lock().unwrap().is_none());
}
