use super::*;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn end_rejects_changed_runtime_visit_revision_head_worker_and_geometry() {
    for axis in 0..6 {
        let (state, backend, scope, point) = setup().await;
        let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
            .await
            .unwrap();
        ready_native_gesture(&handle).await.unwrap();
        *backend.0.gate.lock().unwrap() = Some("end");
        let task = {
            let state = state.clone();
            let handle = handle.clone();
            tokio::spawn(async move {
                end_native_gesture_with_completion(&state, host(), &handle, Some([9.0, 10.0])).await
            })
        };
        backend.wait().await;
        {
            let mut guard = state.runtime.lock().await;
            let runtime = guard.as_mut().unwrap();
            match axis {
                0 => runtime.instance = Uuid::new_v4(),
                1 => runtime.sketch_session.as_mut().unwrap().token = Uuid::new_v4(),
                2 => {
                    runtime.fencing.bump_revision();
                }
                3 => {
                    runtime.latest_snapshot = Some(Arc::new(ModelSnapshot {
                        id: SnapshotId(5),
                        generation: 1,
                        step_index: None,
                        bodies: vec![],
                        stopped_reason: StoppedReason::Completed,
                        step_states: vec![],
                        signatures: None,
                        diagnostics: vec![],
                        diagnostics_by_step: BTreeMap::new(),
                        repair_summary: RepairSummary::default(),
                        sketch_placement_by_step: BTreeMap::new(),
                    }));
                }
                4 => backend.0.retired.store(true, Ordering::SeqCst),
                _ => {
                    let before = runtime.sketch_snapshot(scope.sketch_id, "test").unwrap();
                    let mut after = before.clone();
                    after.apply_solved_positions(&[(point, Vec2::new(40.0, 50.0).unwrap())]);
                    runtime
                        .apply(EditCommand::SketchDragGesture {
                            sketch: scope.sketch_id,
                            before,
                            after,
                        })
                        .unwrap();
                    runtime
                        .fencing
                        .revision
                        .store(scope.document_revision.0, Ordering::SeqCst);
                }
            }
        }
        let expected = state
            .runtime
            .lock()
            .await
            .as_ref()
            .unwrap()
            .sketch_snapshot(scope.sketch_id, "test")
            .unwrap();
        backend.resume();
        assert!(task.await.unwrap().is_err(), "axis {axis}");
        settled(&state).await;
        assert_eq!(
            state
                .runtime
                .lock()
                .await
                .as_ref()
                .unwrap()
                .sketch_snapshot(scope.sketch_id, "test")
                .unwrap(),
            expected
        );
        assert_eq!(*state.autosave_tick.borrow(), 0);
        if axis == 4 {
            assert!(!backend.0.calls.lock().unwrap().contains(&"cancel"));
        }
    }
}

#[tokio::test]
async fn mismatched_worker_history_prefix_refuses_before_upsert_or_begin() {
    let (state, backend, mut scope, point) = setup().await;
    scope.expected_base_hash = onecad_core::regen::HistoryPrefixHash("wrong".into());
    let count = backend.0.calls.lock().unwrap().len();
    let handle = begin_native_gesture(&state, scope, wire::GestureTarget::point(point))
        .await
        .unwrap();
    assert!(ready_native_gesture(&handle).await.is_err());
    settled(&state).await;
    assert_eq!(backend.0.calls.lock().unwrap().len(), count);
}

#[tokio::test]
async fn last_handle_drop_cancels_scratch_and_releases_slot() {
    let (state, backend, scope, point) = setup().await;
    let handle = begin_native_gesture(&state, scope, wire::GestureTarget::point(point))
        .await
        .unwrap();
    ready_native_gesture(&handle).await.unwrap();
    let other = handle.clone();
    drop(handle);
    assert!(!backend.0.gestures.lock().unwrap().is_empty());
    drop(other);
    settled(&state).await;
    assert!(backend.0.gestures.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uncertain_cleanup_retains_quarantined_claim_and_driver_slot() {
    let (state, backend, scope, point) = setup().await;
    let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
        .await
        .unwrap();
    ready_native_gesture(&handle).await.unwrap();
    backend.0.cancel_ok.store(false, Ordering::SeqCst);
    backend.0.quarantine_ok.store(false, Ordering::SeqCst);
    let error = cancel_native_gesture(&handle).await.unwrap_err();
    assert!(error.to_string().contains("lane remains quarantined"));
    {
        let registry = state.native_gestures.lock().unwrap();
        let control = registry.as_ref().unwrap();
        assert!(control.poisoned_claim.lock().unwrap().is_some());
    }
    assert!(state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .prepare_sketch_regions(scope.sketch_id)
        .is_err());
    assert!(
        begin_native_gesture(&state, scope, wire::GestureTarget::point(point))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn checked_preview_accumulates_disjoint_incremental_point_channels() {
    let (state, backend, mut scope, point) = setup().await;
    let other = EntityId(Uuid::new_v4());
    {
        let mut guard = state.runtime.lock().await;
        let runtime = guard.as_mut().unwrap();
        runtime
            .apply(EditCommand::SketchEdit {
                sketch: scope.sketch_id,
                ops: vec![SketchEditOp::AddEntity {
                    entity: onecad_core::sketch::SketchEntity::point(
                        other,
                        Vec2::new(10.0, 0.0).unwrap(),
                        false,
                        false,
                    ),
                }],
            })
            .unwrap();
        scope = runtime
            .native_gesture_scope(scope.sketch_id, scope.source)
            .unwrap();
    }
    backend.0.update_positions.lock().unwrap().extend([
        [(point.to_string(), [2.0, 3.0])].into(),
        [(other.to_string(), [11.0, 4.0])].into(),
    ]);
    let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
        .await
        .unwrap();
    ready_native_gesture(&handle).await.unwrap();
    update_native_gesture(&handle, [2.0, 3.0]).await.unwrap();
    let preview = update_native_gesture(&handle, [2.0, 4.0])
        .await
        .unwrap()
        .value
        .unwrap();
    assert_eq!(point_at_cached(&preview.sketch, point), [2.0, 3.0]);
    assert_eq!(point_at_cached(&preview.sketch, other), [11.0, 4.0]);
    assert_eq!(point_at(&state, scope.sketch_id, point).await, [0.0, 0.0]);
    cancel_native_gesture(&handle).await.unwrap();
}

#[tokio::test]
async fn accepted_completion_survives_later_cleanup_failure() {
    struct RetireOnPublish(Backend);
    impl crate::host::EventSink for RetireOnPublish {
        fn emit(&self, event: crate::host::AppEvent) -> Result<(), String> {
            if matches!(event, crate::host::AppEvent::SketchSolved(_)) {
                self.0 .0.retired.store(true, Ordering::SeqCst);
                self.0 .0.quarantine_ok.store(false, Ordering::SeqCst);
            }
            Ok(())
        }
    }
    let (state, backend, scope, point) = setup().await;
    let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
        .await
        .unwrap();
    let mut host = host();
    host.events = Arc::new(RetireOnPublish(backend));
    let completion = end_native_gesture_with_completion(&state, host, &handle, Some([7.0, 8.0]))
        .await
        .unwrap();
    let receipt = completion.result.unwrap();
    assert!(receipt.value.moved);
    assert!(receipt
        .value
        .cleanup_error
        .unwrap()
        .contains("lane remains quarantined"));
    assert_eq!(point_at(&state, scope.sketch_id, point).await, [7.0, 8.0]);
    assert_eq!(*state.autosave_tick.borrow(), 1);
    assert!(state
        .native_gestures
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .poisoned_claim
        .lock()
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn native_admission_never_falls_back_to_unbound_solver() {
    let (state, backend, scope, point) = setup().await;
    state.runtime.lock().await.as_mut().unwrap().solver = Arc::new(crate::worker::PendingBackend);
    let calls = backend.0.calls.lock().unwrap().len();
    assert!(
        begin_native_gesture(&state, scope, wire::GestureTarget::point(point))
            .await
            .is_err()
    );
    assert_eq!(backend.0.calls.lock().unwrap().len(), calls);
    assert!(state.native_gestures.lock().unwrap().is_none());
}

#[test]
fn target_lock_policy_distinguishes_moved_points_from_stationary_centers() {
    use onecad_core::sketch::SketchEntity;
    let mut sketch = Sketch::on_world_plane(SketchId(Uuid::new_v4()), "Locks", WorldPlane::XY);
    let free = EntityId(Uuid::new_v4());
    let locked = EntityId(Uuid::new_v4());
    let line = EntityId(Uuid::new_v4());
    let circle = EntityId(Uuid::new_v4());
    sketch
        .add_entity(SketchEntity::point(
            free,
            Vec2::new(0.0, 0.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    sketch
        .add_entity(SketchEntity::point(
            locked,
            Vec2::new(10.0, 0.0).unwrap(),
            false,
            true,
        ))
        .unwrap();
    sketch
        .add_entity(SketchEntity::line(line, free, locked, false))
        .unwrap();
    sketch
        .add_entity(SketchEntity::circle(circle, locked, 5.0, false).unwrap())
        .unwrap();
    assert!(validation::target(&sketch, wire::GestureTarget::point(free)).is_ok());
    assert!(validation::target(&sketch, wire::GestureTarget::point(locked)).is_err());
    let body = wire::GestureTarget {
        kind: wire::DragKind::EntityBody,
        entity: line,
        role: None,
        grab: Some([5.0, 0.0]),
    };
    assert!(validation::target(&sketch, body).is_err());
    let radius = wire::GestureTarget {
        kind: wire::DragKind::Radius,
        entity: circle,
        role: None,
        grab: Some([15.0, 0.0]),
    };
    assert!(validation::target(&sketch, radius).is_ok());
}

#[tokio::test]
async fn unpublished_snapshot_zero_matches_none_but_positive_snapshot_stays_fenced() {
    let (state, _backend, mut scope, point) = setup().await;
    assert_eq!(scope.snapshot, None);
    scope.snapshot = Some(SnapshotId(0));
    let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
        .await
        .unwrap();
    ready_native_gesture(&handle).await.unwrap();
    cancel_native_gesture(&handle).await.unwrap();
    scope.snapshot = Some(SnapshotId(5));
    let error = begin_native_gesture(&state, scope, wire::GestureTarget::point(point))
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("stale snapshot (expected SnapshotId(5), current SnapshotId(0))"));
}
