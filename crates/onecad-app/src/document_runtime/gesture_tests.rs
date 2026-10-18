use super::*;

fn ready() -> BeginGestureDto {
    BeginGestureDto {
        gesture_id: 0,
        ready: true,
        point_offset: None,
        positions: Default::default(),
        curves: Default::default(),
        entity_states: Default::default(),
    }
}

#[tokio::test]
async fn begin_geometry_correction_is_preview_and_cancel_discards_it() {
    let backend = Arc::new(FakeBackend::new());
    let mut runtime = runtime_with(backend.clone());
    let (sketch, point) = sketch_with_point();
    let id = sketch.id;
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    let mut receipt = ready();
    receipt.point_offset = Some([2.0, 3.0]);
    receipt.positions.insert(point.to_string(), [2.0, 3.0]);
    *backend.begin_override.lock().unwrap() = Some(receipt);
    let revision = runtime.revision();
    let depth = runtime.undo_depth();
    let begin = runtime
        .begin_gesture(id, point, GestureTarget::point(point))
        .await
        .unwrap();
    assert_eq!(begin.positions[&point.to_string()], [2.0, 3.0]);
    assert_eq!(point_at(&runtime, id, point), [0.0, 0.0]);
    assert!(!runtime.get_sketch(id).unwrap().solve_current);
    runtime.cancel_gesture().await.unwrap();
    assert_eq!(point_at(&runtime, id, point), [0.0, 0.0]);
    assert_eq!(runtime.revision(), revision);
    assert_eq!(runtime.undo_depth(), depth);
    assert!(backend.state.lock().unwrap().gestures.is_empty());
    runtime.prepare_sketch_regions(id).unwrap();
}

#[tokio::test]
async fn invalid_begin_payload_cleans_scratch_and_lane_without_authoring() {
    for bad in [0, 1] {
        let backend = Arc::new(FakeBackend::new());
        let mut runtime = runtime_with(backend.clone());
        let (sketch, point) = sketch_with_point();
        let id = sketch.id;
        runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
        let mut receipt = ready();
        if bad == 0 {
            receipt
                .positions
                .insert(EntityId(Uuid::from_u128(0xdead)).to_string(), [1.0, 2.0]);
        } else {
            receipt.point_offset = Some([f64::INFINITY, 0.0]);
        }
        *backend.begin_override.lock().unwrap() = Some(receipt);
        let revision = runtime.revision();
        let depth = runtime.undo_depth();
        assert!(runtime
            .begin_gesture(id, point, GestureTarget::point(point))
            .await
            .is_err());
        assert!(runtime.active_gesture.is_none());
        assert!(backend.state.lock().unwrap().gestures.is_empty());
        assert_eq!(point_at(&runtime, id, point), [0.0, 0.0]);
        assert_eq!(runtime.revision(), revision);
        assert_eq!(runtime.undo_depth(), depth);
        runtime.prepare_sketch_regions(id).unwrap();
    }
}

#[tokio::test]
async fn foreign_drag_receipt_and_failed_cancel_release_claim_without_edit() {
    let backend = Arc::new(FakeBackend::new());
    let mut runtime = runtime_with(backend.clone());
    let (sketch, point) = sketch_with_point();
    let id = sketch.id;
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    runtime
        .begin_gesture(id, point, GestureTarget::point(point))
        .await
        .unwrap();
    *backend.drag_override.lock().unwrap() = Some(DragSolveDto {
        gesture_id: 999,
        seq: 1,
        status: "success".into(),
        dof: 0,
        conflicting: vec![],
        positions: Default::default(),
        curves: Default::default(),
        solve_micros: 0,
        superseded: false,
        diagnostics: None,
    });
    *backend.cancel_fails.lock().unwrap() = true;
    let revision = runtime.revision();
    let depth = runtime.undo_depth();
    assert!(runtime.solve_drag([1.0, 2.0]).await.is_err());
    assert!(runtime.active_gesture.is_none());
    assert_eq!(runtime.revision(), revision);
    assert_eq!(runtime.undo_depth(), depth);
    assert_eq!(point_at(&runtime, id, point), [0.0, 0.0]);
    runtime.prepare_sketch_regions(id).unwrap();
}

#[tokio::test]
async fn cancellation_on_changed_worker_epoch_never_adopts_pending_pose() {
    let backend = Arc::new(FakeBackend::new());
    let mut runtime = runtime_with(backend);
    let (sketch, point) = sketch_with_point();
    let id = sketch.id;
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    runtime
        .begin_gesture(id, point, GestureTarget::point(point))
        .await
        .unwrap();
    let revision = runtime.revision();
    runtime.fencing().set_epoch(2);
    assert!(runtime.cancel_gesture().await.is_err());
    assert!(runtime.active_gesture.is_none());
    assert_eq!(runtime.revision(), revision);
    assert_eq!(point_at(&runtime, id, point), [0.0, 0.0]);
    runtime.prepare_sketch_regions(id).unwrap();
}

#[tokio::test]
async fn changed_epoch_during_begin_drag_or_end_cannot_publish_or_hold_lane() {
    for phase in ["BeginGesture", "SolveDrag", "EndGesture"] {
        let backend = Arc::new(FakeBackend::new());
        let mut runtime = runtime_with(backend.clone());
        let (sketch, point) = sketch_with_point();
        let id = sketch.id;
        runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
        let revision = runtime.revision();
        let depth = runtime.undo_depth();
        *backend.gesture_reply_epoch.lock().unwrap() = Some((phase, runtime.fencing()));
        let begin = runtime
            .begin_gesture(id, point, GestureTarget::point(point))
            .await;
        match phase {
            "BeginGesture" => assert!(begin.is_err()),
            "SolveDrag" => {
                begin.unwrap();
                assert!(runtime.solve_drag([2.0, 3.0]).await.is_err());
            }
            _ => {
                begin.unwrap();
                assert!(runtime.end_gesture(Some([2.0, 3.0])).await.is_err());
            }
        }
        assert!(runtime.active_gesture.is_none());
        assert_eq!(runtime.revision(), revision);
        assert_eq!(runtime.undo_depth(), depth);
        assert_eq!(point_at(&runtime, id, point), [0.0, 0.0]);
        runtime.prepare_sketch_regions(id).unwrap();
    }
}

#[tokio::test]
async fn wrong_drag_sequence_cancels_scratch_and_preserves_redo() {
    let backend = Arc::new(FakeBackend::new());
    let mut runtime = runtime_with(backend.clone());
    let (sketch, point) = sketch_with_point();
    let id = sketch.id;
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    runtime
        .begin_gesture(id, point, GestureTarget::point(point))
        .await
        .unwrap();
    runtime.end_gesture(Some([1.0, 2.0])).await.unwrap();
    runtime.undo().unwrap();
    let begin = runtime
        .begin_gesture(id, point, GestureTarget::point(point))
        .await
        .unwrap();
    let redo = runtime.redo_depth();
    let revision = runtime.revision();
    *backend.drag_override.lock().unwrap() = Some(DragSolveDto {
        gesture_id: begin.gesture_id,
        seq: 99,
        status: "success".into(),
        dof: 0,
        conflicting: vec![],
        positions: Default::default(),
        curves: Default::default(),
        solve_micros: 0,
        superseded: false,
        diagnostics: None,
    });
    assert!(runtime.solve_drag([3.0, 4.0]).await.is_err());
    assert_eq!(runtime.redo_depth(), redo);
    assert_eq!(runtime.revision(), revision);
    assert_eq!(point_at(&runtime, id, point), [0.0, 0.0]);
    assert!(runtime.active_gesture.is_none());
    assert!(backend.state.lock().unwrap().gestures.is_empty());
    runtime.prepare_sketch_regions(id).unwrap();
}
