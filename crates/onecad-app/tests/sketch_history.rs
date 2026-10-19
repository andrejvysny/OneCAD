//! Scoped history against the real solver; native shortcut acceptance is separate.
use onecad_app::{
    document_runtime::DocumentRuntime,
    dto::SketchHistoryScope,
    worker::{resolve_worker_path, SupervisorConfig, WorkerManager},
};
use onecad_core::{
    edit::{EditCommand, HistoryDirection, SketchEditOp},
    ids::{EntityId, SketchId, SnapshotId},
    math::Vec2,
    sketch::{Sketch, SketchEntity, WorldPlane},
};
use std::{path::PathBuf, sync::Arc, time::Duration};

async fn setup() -> Option<(WorkerManager, DocumentRuntime, SketchId)> {
    let path = std::env::var("ONECAD_WORKER_PATH")
        .ok()
        .map(PathBuf::from)
        .or_else(resolve_worker_path);
    assert!(path.as_ref().is_none_or(|path| path.is_file()));
    assert!(path.is_some() || std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"));
    let worker = WorkerManager::spawn(SupervisorConfig::production(path?));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    );
    let id = SketchId::new();
    let mut sketch = Sketch::on_world_plane(id, "History QA", WorldPlane::XY);
    sketch
        .add_entity(SketchEntity::point(
            EntityId::new(),
            Vec2::new_unchecked(0.0, 0.0),
            false,
            false,
        ))
        .unwrap();
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    runtime.enter_sketch(id).await.unwrap();
    Some((worker, runtime, id))
}

fn scope(runtime: &DocumentRuntime, id: SketchId) -> SketchHistoryScope {
    SketchHistoryScope {
        sketch_id: id,
        edit_session: runtime.get_sketch(id).unwrap().edit_session.unwrap(),
        runtime_session: runtime.runtime_session().to_string(),
        revision: runtime.revision().0,
        snapshot_id: runtime.head_snapshot_id().unwrap_or(SnapshotId(0)),
    }
}

async fn step(runtime: &mut DocumentRuntime, id: SketchId, direction: HistoryDirection) -> bool {
    let Some(prepared) = runtime
        .prepare_sketch_history(scope(runtime, id), direction)
        .await
        .unwrap()
    else {
        return false;
    };
    let driven = prepared.drive().await.unwrap();
    runtime.commit_sketch_history(driven).unwrap().0.moved
}

async fn add_point(runtime: &mut DocumentRuntime, id: SketchId, x: f64) {
    runtime
        .sketch_upsert(
            id,
            vec![SketchEditOp::AddEntity {
                entity: SketchEntity::point(
                    EntityId::new(),
                    Vec2::new_unchecked(x, 0.0),
                    false,
                    false,
                ),
            }],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn scoped_undo_redo_preserve_identity_precision_and_entry_boundary() {
    let Some((worker, mut runtime, id)) = setup().await else {
        return;
    };
    let baseline = runtime.sketch_snapshot(id, "test").unwrap();
    let floor = runtime.undo_depth();
    assert!(!step(&mut runtime, id, HistoryDirection::Undo).await);
    assert!(!step(&mut runtime, id, HistoryDirection::Redo).await);
    add_point(&mut runtime, id, 0.1234).await;
    let edited = runtime.sketch_snapshot(id, "test").unwrap();
    assert!(step(&mut runtime, id, HistoryDirection::Undo).await);
    assert_eq!(runtime.sketch_snapshot(id, "test").unwrap(), baseline);
    assert_eq!(runtime.undo_depth(), floor);
    assert!(!step(&mut runtime, id, HistoryDirection::Undo).await);
    assert!(step(&mut runtime, id, HistoryDirection::Redo).await);
    assert_eq!(runtime.sketch_snapshot(id, "test").unwrap(), edited);
    assert!(!step(&mut runtime, id, HistoryDirection::Redo).await);
    assert!(runtime.get_sketch(id).unwrap().solve_current);
    assert_eq!(
        runtime.get_sketch(id).unwrap().history.unwrap().undo_depth,
        1
    );
    worker.shutdown().await;
}

#[tokio::test]
async fn new_edit_clears_owned_redo_and_new_visit_cannot_replay_foreign_redo() {
    let Some((worker, mut runtime, id)) = setup().await else {
        return;
    };
    add_point(&mut runtime, id, 2.0).await;
    assert!(step(&mut runtime, id, HistoryDirection::Undo).await);
    add_point(&mut runtime, id, 3.0).await;
    assert!(!step(&mut runtime, id, HistoryDirection::Redo).await);
    assert!(step(&mut runtime, id, HistoryDirection::Undo).await);
    let stale = scope(&runtime, id);
    runtime.cancel_sketch(id, false).await.unwrap();
    assert_eq!(runtime.redo_depth(), 0);
    runtime.enter_sketch(id).await.unwrap();
    assert!(runtime
        .prepare_sketch_history(stale, HistoryDirection::Redo)
        .await
        .is_err());
    assert!(!step(&mut runtime, id, HistoryDirection::Redo).await);
    worker.shutdown().await;
}
