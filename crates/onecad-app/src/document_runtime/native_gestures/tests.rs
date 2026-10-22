use super::*;
use crate::{api::*, dto::*, state::AppState};
use std::time::Duration;
mod support;
use support::*;

#[tokio::test]
async fn pending_begin_cancel_retains_lane_and_never_holds_runtime_lock() {
    let (state, backend, scope, point) = setup().await;
    *backend.0.gate.lock().unwrap() = Some("begin");
    let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
        .await
        .unwrap();
    backend.wait().await;
    assert!(state.runtime.try_lock().is_ok());
    let control = state.native_gestures.lock().unwrap().clone().unwrap();
    control.cancel();
    assert!(state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .prepare_sketch_regions(scope.sketch_id)
        .is_err());
    assert!(!backend.0.gestures.lock().unwrap().is_empty());
    backend.resume();
    cancel_native_gesture(&handle).await.unwrap();
    assert!(backend.0.gestures.lock().unwrap().is_empty());
    assert_eq!(point_at(&state, scope.sketch_id, point).await, [0.0, 0.0]);
    assert!(state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .prepare_sketch_regions(scope.sketch_id)
        .is_ok());
}

#[tokio::test]
async fn dropped_ready_update_and_end_futures_clean_without_document_edits() {
    for phase in ["begin", "update", "end"] {
        let (state, backend, scope, point) = setup().await;
        if phase == "begin" {
            *backend.0.gate.lock().unwrap() = Some(phase);
        }
        let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
            .await
            .unwrap();
        if phase != "begin" {
            ready_native_gesture(&handle).await.unwrap();
        }
        *backend.0.gate.lock().unwrap() = Some(phase);
        let task = {
            let handle = handle.clone();
            let state = state.clone();
            tokio::spawn(async move {
                match phase {
                    "begin" => {
                        ready_native_gesture(&handle).await?;
                    }
                    "update" => {
                        update_native_gesture(&handle, [9.0, 10.0]).await?;
                    }
                    _ => {
                        end_native_gesture_with_completion(
                            &state,
                            host(),
                            &handle,
                            Some([9.0, 10.0]),
                        )
                        .await?;
                    }
                }
                Ok::<_, ApiError>(())
            })
        };
        backend.wait().await;
        assert!(state.runtime.try_lock().is_ok(), "{phase}");
        task.abort();
        let _ = task.await;
        backend.resume();
        settled(&state).await;
        assert!(backend.0.gestures.lock().unwrap().is_empty(), "{phase}");
        assert_eq!(
            point_at(&state, scope.sketch_id, point).await,
            [0.0, 0.0],
            "{phase}"
        );
        let runtime = state.runtime.lock().await;
        let runtime = runtime.as_ref().unwrap();
        assert_eq!(runtime.revision(), scope.document_revision);
        assert!(!runtime.dirty);
        if phase == "end" {
            assert_eq!(
                backend.0.cache.lock().unwrap().as_ref().unwrap(),
                &runtime.sketch_snapshot(scope.sketch_id, "test").unwrap()
            );
        }
    }
}

#[tokio::test]
async fn no_op_end_preserves_history_dirty_revision_and_autosave() {
    let (state, _backend, scope, point) = setup().await;
    let before_depth = state.runtime.lock().await.as_ref().unwrap().undo_depth();
    let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
        .await
        .unwrap();
    // End is legal before Ready, including a pointer-up with no intermediate solve.
    let result = end_native_gesture_with_completion(&state, host(), &handle, Some([0.0, 0.0]))
        .await
        .unwrap()
        .result
        .unwrap();
    assert!(!result.value.moved);
    assert_eq!(result.scope, scope);
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    assert_eq!(runtime.undo_depth(), before_depth);
    assert_eq!(runtime.revision(), scope.document_revision);
    assert!(!runtime.dirty);
    assert_eq!(*state.autosave_tick.borrow(), 0);
}

#[tokio::test]
async fn accepted_end_is_one_edit_and_previews_never_author_geometry() {
    let (state, _backend, scope, point) = setup().await;
    let before_depth = state.runtime.lock().await.as_ref().unwrap().undo_depth();
    let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
        .await
        .unwrap();
    let ready = ready_native_gesture(&handle).await.unwrap();
    assert_eq!(ready.scope.source, scope.source);
    let preview = update_native_gesture(&handle, [4.0, 5.0])
        .await
        .unwrap()
        .value
        .unwrap();
    assert_ne!(preview.sketch, ready.value.sketch);
    assert_eq!(point_at(&state, scope.sketch_id, point).await, [0.0, 0.0]);
    let end = end_native_gesture_with_completion(&state, host(), &handle, Some([7.0, 8.0]))
        .await
        .unwrap()
        .result
        .unwrap();
    assert!(end.value.moved);
    assert_eq!(point_at(&state, scope.sketch_id, point).await, [7.0, 8.0]);
    assert_eq!(*state.autosave_tick.borrow(), 1);
    let mut guard = state.runtime.lock().await;
    let runtime = guard.as_mut().unwrap();
    assert_eq!(runtime.undo_depth(), before_depth + 1);
    runtime.undo().unwrap();
    drop(guard);
    assert_eq!(point_at(&state, scope.sketch_id, point).await, [0.0, 0.0]);
}

#[tokio::test]
async fn admission_rejects_each_stale_scope_axis_without_worker_dispatch() {
    for axis in 0..8 {
        let (state, backend, mut scope, point) = setup().await;
        match axis {
            0 => scope.document_id = DocumentId(Uuid::new_v4()),
            1 => scope.runtime_session = Uuid::new_v4().to_string(),
            2 => scope.sketch_id = SketchId(Uuid::new_v4()),
            3 => scope.edit_visit = Uuid::new_v4(),
            4 => scope.document_revision.0 += 1,
            5 => scope.worker_epoch.0 += 1,
            6 => scope.snapshot = Some(SnapshotId(987)),
            _ => state.runtime.lock().await.as_mut().unwrap().read_only = true,
        }
        let count = backend.0.calls.lock().unwrap().len();
        assert!(
            begin_native_gesture(&state, scope, wire::GestureTarget::point(point))
                .await
                .is_err(),
            "axis {axis}"
        );
        assert_eq!(backend.0.calls.lock().unwrap().len(), count);
    }
}

#[tokio::test]
async fn queued_updates_coalesce_and_end_flushes_final_target() {
    let (state, backend, scope, point) = setup().await;
    let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
        .await
        .unwrap();
    ready_native_gesture(&handle).await.unwrap();
    *backend.0.gate.lock().unwrap() = Some("update");
    let first = {
        let handle = handle.clone();
        tokio::spawn(async move { update_native_gesture(&handle, [1.0, 2.0]).await })
    };
    backend.wait().await;
    let control = state.native_gestures.lock().unwrap().clone().unwrap();
    let (send2, receive2) = tokio::sync::oneshot::channel();
    control.update([3.0, 4.0], send2).unwrap();
    let (send3, receive3) = tokio::sync::oneshot::channel();
    control.update([5.0, 6.0], send3).unwrap();
    assert!(receive2.await.unwrap().unwrap().is_none());
    let terminal = {
        let state = state.clone();
        let handle = handle.clone();
        tokio::spawn(async move {
            end_native_gesture_with_completion(&state, host(), &handle, Some([7.0, 8.0])).await
        })
    };
    tokio::task::yield_now().await;
    assert!(state.runtime.try_lock().is_ok());
    backend.resume();
    first.await.unwrap().unwrap();
    assert_eq!(receive3.await.unwrap().unwrap().unwrap().solve.seq, 3);
    assert!(terminal.await.unwrap().unwrap().result.unwrap().value.moved);
    assert_eq!(*backend.0.targets.lock().unwrap(), [[1.0, 2.0], [5.0, 6.0]]);
    assert_eq!(point_at(&state, scope.sketch_id, point).await, [7.0, 8.0]);
}

#[tokio::test]
async fn stale_end_reseeds_new_authority_instead_of_original_before() {
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
    }
    backend.resume();
    assert!(task.await.unwrap().is_err());
    settled(&state).await;
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    let accepted = runtime.sketch_snapshot(scope.sketch_id, "test").unwrap();
    assert_eq!(backend.0.cache.lock().unwrap().as_ref().unwrap(), &accepted);
    assert_eq!(point_at_cached(&accepted, point), [40.0, 50.0]);
}

fn point_at_cached(sketch: &Sketch, id: EntityId) -> [f64; 2] {
    match sketch.get_entity(id).unwrap() {
        onecad_core::sketch::SketchEntity::Point { at, .. } => [at.x, at.y],
        _ => panic!("point expected"),
    }
}

#[tokio::test]
async fn malformed_foreign_and_locked_end_channels_never_commit() {
    for bad in 0..3 {
        let (state, backend, mut scope, point) = setup().await;
        let mut positions = BTreeMap::new();
        match bad {
            0 => {
                positions.insert(point.to_string(), [f64::NAN, 3.0]);
            }
            1 => {
                positions.insert(EntityId(Uuid::new_v4()).to_string(), [2.0, 3.0]);
            }
            _ => {
                let locked = EntityId(Uuid::new_v4());
                let mut guard = state.runtime.lock().await;
                let runtime = guard.as_mut().unwrap();
                runtime
                    .apply(EditCommand::SketchEdit {
                        sketch: scope.sketch_id,
                        ops: vec![SketchEditOp::AddEntity {
                            entity: onecad_core::sketch::SketchEntity::point(
                                locked,
                                Vec2::new(2.0, 3.0).unwrap(),
                                false,
                                true,
                            ),
                        }],
                    })
                    .unwrap();
                runtime.dirty = false;
                scope = runtime
                    .native_gesture_scope(scope.sketch_id, scope.source)
                    .unwrap();
                positions.insert(locked.to_string(), [4.0, 5.0]);
            }
        }
        *backend.0.end_positions.lock().unwrap() = Some(positions);
        let before = state
            .runtime
            .lock()
            .await
            .as_ref()
            .unwrap()
            .get_sketch(scope.sketch_id)
            .unwrap();
        let handle = begin_native_gesture(&state, scope.clone(), wire::GestureTarget::point(point))
            .await
            .unwrap();
        assert!(
            end_native_gesture_with_completion(&state, host(), &handle, Some([2.0, 3.0]))
                .await
                .is_err()
        );
        settled(&state).await;
        let guard = state.runtime.lock().await;
        let runtime = guard.as_ref().unwrap();
        assert_eq!(runtime.revision(), scope.document_revision);
        assert_eq!(runtime.get_sketch(scope.sketch_id).unwrap(), before);
        assert!(!runtime.dirty);
        assert_eq!(*state.autosave_tick.borrow(), 0);
    }
}

mod fencing;
#[cfg(feature = "sidecar")]
mod production;
