//! Point-only scope, scratch cancellation, one edit, and fresh persistence replay.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::worker::wire::GestureTarget;
use onecad_app::worker::{resolve_worker_path, SupervisorConfig, WorkerManager};
use onecad_core::edit::EditCommand;
use onecad_core::ids::{ConstraintId, EntityId, SketchId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::Vec2;
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};
use uuid::Uuid;

fn entity(id: u128) -> EntityId {
    EntityId(Uuid::from_u128(id))
}
fn worker_path() -> Option<PathBuf> {
    let path = std::env::var("ONECAD_WORKER_PATH")
        .ok()
        .map(PathBuf::from)
        .or_else(resolve_worker_path);
    assert!(path.as_ref().is_none_or(|p| p.is_file()));
    assert!(path.is_some() || std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"));
    path
}

fn rectangle(id: SketchId) -> Sketch {
    let mut sketch = Sketch::on_world_plane(id, "Point drag", WorldPlane::XY);
    let corners = [[0.0, 0.0], [80.0, 0.0], [80.0, 60.0], [0.0, 60.0]];
    for i in 0_usize..8 {
        let at = corners[i.div_ceil(2) % 4];
        sketch
            .add_entity(SketchEntity::point(
                entity(100 + i as u128),
                Vec2::new(at[0], at[1]).unwrap(),
                false,
                false,
            ))
            .unwrap();
    }
    for i in 0..4 {
        let line = entity(200 + i);
        sketch
            .add_entity(SketchEntity::line(
                line,
                entity(100 + 2 * i),
                entity(101 + 2 * i),
                false,
            ))
            .unwrap();
        sketch
            .add_constraint(Constraint::Coincident {
                id: ConstraintId(Uuid::from_u128(300 + i)),
                point1: entity(101 + 2 * i),
                point2: entity(100 + (2 * i + 2) % 8),
                point1_position: CurvePosition::Arbitrary,
                point2_position: CurvePosition::Arbitrary,
            })
            .unwrap();
        let id = ConstraintId(Uuid::from_u128(304 + i));
        sketch
            .add_constraint(if i % 2 == 0 {
                Constraint::Horizontal { id, line }
            } else {
                Constraint::Vertical { id, line }
            })
            .unwrap();
    }
    sketch
}

async fn setup() -> Option<(WorkerManager, DocumentRuntime, SketchId)> {
    let worker = WorkerManager::spawn(SupervisorConfig::production(worker_path()?));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    );
    let id = SketchId(Uuid::from_u128(0x1500));
    runtime
        .apply(EditCommand::AddSketch {
            sketch: rectangle(id),
        })
        .unwrap();
    Some((worker, runtime, id))
}

fn assert_rectangle(runtime: &DocumentRuntime, id: SketchId, moved: bool) {
    let session = runtime.get_sketch(id).unwrap();
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
    for (i, at) in expected.iter().enumerate() {
        let point = session
            .entities
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == entity(100 + i as u128).to_string())
            .unwrap();
        for axis in 0..2 {
            assert!((point["at"][axis].as_f64().unwrap() - at[axis]).abs() <= 1e-4);
        }
    }
}

#[tokio::test]
async fn point_offset_commit_is_one_edit_with_undo_redo_and_fresh_reopen() {
    let Some((worker, mut runtime, id)) = setup().await else {
        return;
    };
    let before = runtime.get_sketch(id).unwrap().entities;
    let depth = runtime.undo_depth();
    let revision = runtime.revision();
    let target = GestureTarget {
        grab: Some([1.0, 2.0]),
        ..GestureTarget::point(entity(100))
    };
    let begin = runtime
        .begin_gesture(id, entity(100), target)
        .await
        .unwrap();
    assert_eq!(begin.point_offset, Some([-1.0, -2.0]));
    assert_eq!(runtime.get_sketch(id).unwrap().entities, before);
    assert_eq!(runtime.undo_depth(), depth);
    assert_eq!(runtime.revision(), revision);
    let preview = runtime.solve_drag([23.0, 23.0]).await.unwrap();
    assert_eq!(preview.status, "success");
    assert_eq!(runtime.get_sketch(id).unwrap().entities, before);
    let end = runtime.end_gesture(None).await.unwrap();
    assert_eq!(end.dof, 4);
    assert_eq!(runtime.undo_depth(), depth + 1);
    assert_rectangle(&runtime, id, true);
    runtime.undo().unwrap();
    assert_rectangle(&runtime, id, false);
    runtime.redo().unwrap().unwrap();
    assert_rectangle(&runtime, id, true);
    let saved = runtime.get_sketch(id).unwrap().entities;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("point.onecad");
    runtime
        .save(
            &path,
            SaveMeta {
                app_version: "point-test".into(),
                created: "2026-10-05T00:00:00Z".into(),
                modified: "2026-10-05T00:00:00Z".into(),
                ..Default::default()
            },
        )
        .unwrap();
    worker.shutdown().await;
    let fresh = WorkerManager::spawn(SupervisorConfig::production(worker_path().unwrap()));
    assert!(fresh.wait_ready(Duration::from_secs(10)).await);
    let mut reopened = DocumentRuntime::open(
        &path,
        Arc::new(fresh.clone()),
        Arc::new(fresh.clone()),
        Arc::new(fresh.clone()),
    )
    .unwrap();
    assert_eq!(reopened.get_sketch(id).unwrap().entities, saved);
    reopened.enter_sketch(id).await.unwrap();
    assert_rectangle(&reopened, id, true);
    fresh.shutdown().await;
}

#[tokio::test]
async fn scratch_cancel_preserves_document_revision_history_and_redo() {
    let Some((worker, mut runtime, id)) = setup().await else {
        return;
    };
    runtime
        .begin_gesture(id, entity(100), GestureTarget::point(entity(100)))
        .await
        .unwrap();
    runtime.end_gesture(Some([22.0, 21.0])).await.unwrap();
    runtime.undo().unwrap();
    let original = runtime.get_sketch(id).unwrap().entities;
    let depth = runtime.undo_depth();
    let redo = runtime.redo_depth();
    let revision = runtime.revision();
    runtime
        .begin_gesture(id, entity(100), GestureTarget::point(entity(100)))
        .await
        .unwrap();
    runtime.solve_drag([22.0, 21.0]).await.unwrap();
    runtime.cancel_gesture().await.unwrap();
    assert_eq!(runtime.get_sketch(id).unwrap().entities, original);
    assert_eq!(runtime.undo_depth(), depth);
    assert_eq!(runtime.redo_depth(), redo);
    assert_eq!(runtime.revision(), revision);
    assert!(!runtime.get_sketch(id).unwrap().solve_current);
    assert!(runtime.solve_drag([1.0, 1.0]).await.is_err());
    runtime.redo().unwrap().unwrap();
    assert_rectangle(&runtime, id, true);
    runtime
        .prepare_sketch_regions(id)
        .unwrap()
        .drive()
        .await
        .unwrap();
    worker.shutdown().await;
}
