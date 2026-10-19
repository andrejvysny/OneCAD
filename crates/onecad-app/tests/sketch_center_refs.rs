//! ONECAD-251: typed curve centers share their authored solver Point.
use std::{path::PathBuf, sync::Arc, time::Duration};

use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::worker::wire::GestureTarget;
use onecad_app::worker::{resolve_worker_path, SupervisorConfig, WorkerManager};
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{ConstraintId, EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};
use uuid::Uuid;

#[path = "fixtures/replay.rs"]
mod fixture_replay;

#[tokio::test]
async fn canonical_center_identity_fixture_replays_on_real_worker() {
    let Some(worker) = worker_path() else { return };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fixture_replay::replay(
        &worker,
        &root.join("protocol/fixtures/sketch_center_refs.ndjson"),
    )
    .await;
}

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

async fn curve_runtime(worker: &WorkerManager, curve: SketchEntity) -> (DocumentRuntime, SketchId) {
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    );
    let sid = SketchId::new();
    let mut sketch = Sketch::on_world_plane(sid, "Shared center", WorldPlane::XY);
    sketch
        .add_entity(SketchEntity::point(
            entity(1),
            Vec2::new(10.0, 20.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    sketch.add_entity(curve).unwrap();
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    (runtime, sid)
}

async fn assert_curve_dof(worker: &WorkerManager, curve: SketchEntity, expected: u32) {
    let (mut runtime, sid) = curve_runtime(worker, curve).await;
    let free = runtime.enter_sketch(sid).await.unwrap();
    eprintln!("shared center: DOF expected {expected}, got {}", free.dof);
    assert_eq!(free.dof, expected, "typed curve has one center");
    let constraint = Constraint::Fixed {
        id: ConstraintId(Uuid::from_u128(3)),
        point: entity(1),
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new(10.0, 20.0).unwrap(),
    };
    let fixed = runtime
        .sketch_upsert(sid, vec![SketchEditOp::AddConstraint { constraint }])
        .await
        .unwrap();
    assert_eq!(
        fixed.dof,
        expected - 2,
        "shared center constraint removes 2 DOF"
    );
}

#[tokio::test]
async fn typed_curve_centers_have_no_orphan_dof() {
    let Some(path) = worker_path() else { return };
    let worker = WorkerManager::spawn(SupervisorConfig::production(path));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let cases = [
        (
            SketchEntity::circle(entity(2), entity(1), 3.0, false).unwrap(),
            3,
        ),
        (
            SketchEntity::arc(entity(2), entity(1), 3.0, 0.0, 1.0, false).unwrap(),
            5,
        ),
        (
            SketchEntity::ellipse(entity(2), entity(1), 6.0, 3.0, 0.0, false).unwrap(),
            5,
        ),
    ];
    for (curve, expected) in cases {
        assert_curve_dof(&worker, curve, expected).await;
    }
    worker.shutdown().await;
}

#[tokio::test]
async fn authored_point_drag_moves_circle_center_in_one_undo_entry() {
    let Some(path) = worker_path() else { return };
    let worker = WorkerManager::spawn(SupervisorConfig::production(path));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let curve = SketchEntity::circle(entity(2), entity(1), 3.0, false).unwrap();
    let (mut runtime, sid) = curve_runtime(&worker, curve).await;
    runtime.enter_sketch(sid).await.unwrap();
    let before = runtime.get_sketch(sid).unwrap().entities;
    let depth = runtime.undo_depth();
    runtime
        .begin_gesture(sid, entity(1), GestureTarget::point(entity(1)))
        .await
        .unwrap();
    assert_eq!(
        runtime.solve_drag([15.0, 25.0]).await.unwrap().status,
        "success"
    );
    assert_eq!(runtime.get_sketch(sid).unwrap().entities, before);
    assert_eq!(runtime.end_gesture(None).await.unwrap().dof, 3);
    assert_eq!(runtime.undo_depth(), depth + 1);
    let session = runtime.get_sketch(sid).unwrap();
    let entities = session.entities.as_array().unwrap();
    let point = entities
        .iter()
        .find(|e| e["id"] == entity(1).to_string())
        .unwrap();
    let circle = entities
        .iter()
        .find(|e| e["id"] == entity(2).to_string())
        .unwrap();
    assert_eq!(point["at"], serde_json::json!([15.0, 25.0]));
    assert_eq!(circle["center"], point["at"]);
    runtime.undo().unwrap();
    assert_eq!(runtime.get_sketch(sid).unwrap().entities, before);
    runtime.redo().unwrap().unwrap();
    assert_eq!(runtime.get_sketch(sid).unwrap().entities, session.entities);
    worker.shutdown().await;
}
