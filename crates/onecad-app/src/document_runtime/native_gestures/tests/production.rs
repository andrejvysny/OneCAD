use super::*;
use crate::worker::{resolve_worker_path, SupervisorConfig, WorkerManager};
use onecad_core::ids::ConstraintId;
use onecad_core::sketch::{Constraint, CurvePosition, SketchEntity};
use std::path::PathBuf;

fn entity(id: u128) -> EntityId {
    EntityId(Uuid::from_u128(id))
}
fn worker_path() -> Option<PathBuf> {
    let path = std::env::var_os("ONECAD_WORKER_PATH")
        .map(PathBuf::from)
        .or_else(resolve_worker_path);
    assert!(path.as_ref().is_none_or(|path| path.is_file()));
    assert!(path.is_some() || std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"));
    path
}
struct WorkerGuard(WorkerManager);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.retire();
    }
}

async fn worker() -> Option<WorkerGuard> {
    let manager = WorkerManager::spawn(SupervisorConfig::production(worker_path()?));
    assert!(manager.wait_ready(Duration::from_secs(10)).await);
    Some(WorkerGuard(manager))
}

fn rectangle(id: SketchId) -> Sketch {
    let mut sketch = Sketch::on_world_plane(id, "Native rectangle", WorldPlane::XY);
    let corners = [[0.0, 0.0], [80.0, 0.0], [80.0, 60.0], [0.0, 60.0]];
    for index in 0_usize..8 {
        let at = corners[index.div_ceil(2) % 4];
        sketch
            .add_entity(SketchEntity::point(
                entity(100 + index as u128),
                Vec2::new(at[0], at[1]).unwrap(),
                false,
                false,
            ))
            .unwrap();
    }
    for index in 0..4 {
        let line = entity(200 + index);
        sketch
            .add_entity(SketchEntity::line(
                line,
                entity(100 + index * 2),
                entity(101 + index * 2),
                false,
            ))
            .unwrap();
        sketch
            .add_constraint(Constraint::Coincident {
                id: ConstraintId(Uuid::from_u128(300 + index)),
                point1: entity(101 + index * 2),
                point2: entity(100 + (index * 2 + 2) % 8),
                point1_position: CurvePosition::Arbitrary,
                point2_position: CurvePosition::Arbitrary,
            })
            .unwrap();
        let id = ConstraintId(Uuid::from_u128(304 + index));
        sketch
            .add_constraint(if index % 2 == 0 {
                Constraint::Horizontal { id, line }
            } else {
                Constraint::Vertical { id, line }
            })
            .unwrap();
    }
    sketch
}

async fn app(worker: &WorkerManager, sketch: Sketch) -> (Arc<AppState>, NativeGestureScope) {
    let id = sketch.id;
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    );
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    runtime.enter_sketch(id).await.unwrap();
    let scope = runtime
        .native_gesture_scope(
            id,
            NativeGestureSource {
                capture: 10,
                generation: 20,
            },
        )
        .unwrap();
    let state = Arc::new(AppState::default());
    *state.runtime.lock().await = Some(runtime);
    (state, scope)
}

fn assert_rectangle(sketch: &Sketch, moved: bool) {
    let expected = if moved {
        [
            [22.0, 21.0],
            [80.0, 21.0],
            [80.0, 21.0],
            [80.0, 60.0],
            [80.0, 60.0],
            [22.0, 60.0],
            [22.0, 60.0],
            [22.0, 21.0],
        ]
    } else {
        [
            [0.0, 0.0],
            [80.0, 0.0],
            [80.0, 0.0],
            [80.0, 60.0],
            [80.0, 60.0],
            [0.0, 60.0],
            [0.0, 60.0],
            [0.0, 0.0],
        ]
    };
    for (index, expected) in expected.into_iter().enumerate() {
        let actual = point_at_cached(sketch, entity(100 + index as u128));
        assert!(actual
            .into_iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() < 1e-4));
    }
}

#[tokio::test]
async fn production_rectangle_exact_release_truth_one_edit_undo_redo_and_fresh_reopen() {
    let Some(worker) = worker().await else {
        return;
    };
    let id = SketchId(Uuid::new_v4());
    let (state, scope) = app(&worker.0, rectangle(id)).await;
    let depth = state.runtime.lock().await.as_ref().unwrap().undo_depth();
    let target = wire::GestureTarget {
        grab: Some([1.0, 2.0]),
        ..wire::GestureTarget::point(entity(100))
    };
    let handle = begin_native_gesture(&state, scope, target).await.unwrap();
    let ready = ready_native_gesture(&handle).await.unwrap();
    assert_eq!(ready.value.solve.point_offset, Some([-1.0, -2.0]));
    update_native_gesture(&handle, [23.0, 23.0]).await.unwrap();
    let end = end_native_gesture_with_completion(&state, host(), &handle, None)
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(end.value.solve.dof, 4);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("native-point.onecad");
    {
        let mut guard = state.runtime.lock().await;
        let runtime = guard.as_mut().unwrap();
        assert_eq!(runtime.undo_depth(), depth + 1);
        assert_rectangle(&runtime.sketch_snapshot(id, "test").unwrap(), true);
        runtime.undo().unwrap();
        assert_rectangle(&runtime.sketch_snapshot(id, "test").unwrap(), false);
        runtime.redo().unwrap().unwrap();
        assert_rectangle(&runtime.sketch_snapshot(id, "test").unwrap(), true);
        runtime
            .save(&path, onecad_core::io::container::SaveMeta::default())
            .unwrap();
    }
    worker.0.retire();
    let fresh = self::worker().await.unwrap();
    let mut reopened = DocumentRuntime::open(
        &path,
        Arc::new(fresh.0.clone()),
        Arc::new(fresh.0.clone()),
        Arc::new(fresh.0.clone()),
    )
    .unwrap();
    reopened.enter_sketch(id).await.unwrap();
    assert_rectangle(&reopened.sketch_snapshot(id, "test").unwrap(), true);
}

fn curve_sketch(id: SketchId, arc: bool) -> Sketch {
    let mut sketch = Sketch::on_world_plane(id, "Native curve", WorldPlane::XY);
    sketch
        .add_entity(SketchEntity::point(
            entity(1),
            Vec2::new(0.0, 0.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    sketch
        .add_entity(if arc {
            SketchEntity::arc(
                entity(2),
                entity(1),
                5.0,
                0.0,
                std::f64::consts::FRAC_PI_2,
                false,
            )
            .unwrap()
        } else {
            SketchEntity::circle(entity(2), entity(1), 5.0, false).unwrap()
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Fixed {
            id: ConstraintId(Uuid::new_v4()),
            point: entity(1),
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new(0.0, 0.0).unwrap(),
        })
        .unwrap();
    sketch
}

#[tokio::test]
async fn production_radius_and_arc_endpoint_channels_are_checked_and_committed() {
    let Some(worker) = worker().await else {
        return;
    };
    for arc in [false, true] {
        let id = SketchId(Uuid::new_v4());
        let (state, scope) = app(&worker.0, curve_sketch(id, arc)).await;
        let target = wire::GestureTarget {
            kind: if arc {
                wire::DragKind::ArcEnd
            } else {
                wire::DragKind::Radius
            },
            entity: entity(2),
            role: arc.then_some("end"),
            grab: Some(if arc { [0.0, 5.0] } else { [5.0, 0.0] }),
        };
        let handle = begin_native_gesture(&state, scope, target).await.unwrap();
        ready_native_gesture(&handle).await.unwrap();
        let cursor = if arc { [0.0, 8.0] } else { [8.0, 0.0] };
        let preview = update_native_gesture(&handle, cursor)
            .await
            .unwrap()
            .value
            .unwrap();
        assert!(preview
            .solve
            .curves
            .get(&entity(2).to_string())
            .is_some_and(|curve| curve.radius.is_some()));
        let cursor = if arc {
            let diagonal = 8.0 / std::f64::consts::SQRT_2;
            let second = update_native_gesture(&handle, [diagonal, diagonal])
                .await
                .unwrap()
                .value
                .unwrap();
            match second.sketch.get_entity(entity(2)).unwrap() {
                SketchEntity::Arc {
                    radius, end_angle, ..
                } => {
                    assert!((*radius - 8.0).abs() < 1e-4);
                    assert!((*end_angle - std::f64::consts::FRAC_PI_4).abs() < 1e-4);
                }
                _ => panic!("arc expected"),
            }
            assert!(second
                .solve
                .curves
                .get(&entity(2).to_string())
                .is_some_and(|curve| curve.radius.is_none() && curve.end_angle.is_some()));
            [diagonal, diagonal]
        } else {
            cursor
        };
        let end = end_native_gesture_with_completion(&state, host(), &handle, Some(cursor))
            .await
            .unwrap()
            .result
            .unwrap();
        assert!(end.value.moved);
        let guard = state.runtime.lock().await;
        let sketch = guard.as_ref().unwrap().sketch_snapshot(id, "test").unwrap();
        match sketch.get_entity(entity(2)).unwrap() {
            SketchEntity::Circle { radius, .. } | SketchEntity::Arc { radius, .. } => {
                assert!((*radius - 8.0).abs() < 1e-4)
            }
            _ => panic!("curve expected"),
        }
        assert_eq!(point_at_cached(&sketch, entity(1)), [0.0, 0.0]);
    }
}

#[tokio::test]
async fn production_old_connection_cancel_cannot_touch_replacement_gesture_with_same_id() {
    let Some(worker) = worker().await else {
        return;
    };
    let sketch = rectangle(SketchId(Uuid::new_v4()));
    let old = worker.0.bind_native_gesture().unwrap();
    let solved = old.sketch_upsert(&sketch).await.unwrap();
    old.begin_gesture(
        &sketch.id.to_string(),
        solved.sketch_revision,
        42,
        entity(100),
        wire::GestureTarget::point(entity(100)),
        "",
    )
    .await
    .unwrap();
    let epoch = worker.0.epoch();
    worker.0.shutdown().await;
    tokio::time::timeout(Duration::from_secs(15), async {
        while worker.0.epoch() == epoch || !worker.0.wait_ready(Duration::from_millis(50)).await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let fresh = worker.0.bind_native_gesture().unwrap();
    let solved = fresh.sketch_upsert(&sketch).await.unwrap();
    fresh
        .begin_gesture(
            &sketch.id.to_string(),
            solved.sketch_revision,
            42,
            entity(100),
            wire::GestureTarget::point(entity(100)),
            "",
        )
        .await
        .unwrap();
    assert!(!old.is_current());
    assert!(old.cancel_gesture(42).await.is_err());
    assert!(old.quarantine().await);
    assert!(fresh.is_current());
    let solved = fresh
        .solve_drag(42, 1, entity(100), [22.0, 21.0])
        .await
        .unwrap();
    assert_eq!(solved.status, "success");
    fresh.cancel_gesture(42).await.unwrap();
}
