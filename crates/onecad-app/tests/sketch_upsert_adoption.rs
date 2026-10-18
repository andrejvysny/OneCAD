//! Successful non-drag solves must publish and persist the worker's pose.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::dto::SketchSessionDto;
use onecad_app::worker::{resolve_worker_path, SupervisorConfig, WorkerManager};
use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, SketchEditOp};
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::Vec2;
use onecad_core::regen::{CancelToken, Lod, Outcome, RegenRequest};
use onecad_core::sketch::{
    Constraint, CurvePosition, Sketch, SketchEntity, SketchPlane, WorldPlane,
};
use onecad_protocol::mesh::{f32_le, validate_mesh_blob};
use uuid::Uuid;

#[path = "sketch_upsert_adoption/helpers.rs"]
mod helpers;
use helpers::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finish_adopts_before_extrude_and_fresh_saved_replay() {
    let Some(path) = worker_path() else {
        return;
    };
    let worker = WorkerManager::spawn(SupervisorConfig::production(path.clone()));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    );
    let id = SketchId(Uuid::from_u128(0xad014));
    runtime
        .apply(EditCommand::AddSketch {
            sketch: skew_rectangle(id),
        })
        .unwrap();
    let profile = runtime
        .prepare_sketch_regions(id)
        .unwrap()
        .drive()
        .await
        .unwrap()
        .regions[0]
        .preview_triangles
        .as_ref()
        .unwrap()
        .positions
        .clone();
    let finished = runtime.finish_sketch(id).await.unwrap();
    let adopted = runtime.get_sketch(id).unwrap();
    assert_adopted(&adopted, &profile);
    let sketch_record = runtime
        .projection()
        .features
        .iter()
        .find(|f| f.op_type == "Sketch")
        .unwrap()
        .id
        .clone();
    let params = runtime
        .operation_params(RecordId::from_str(&sketch_record).unwrap())
        .unwrap();
    for point_id in 100..108 {
        let stored = params["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == entity(point_id).to_string())
            .unwrap();
        assert_eq!(
            stored["at"],
            serde_json::json!(point(&adopted, point_id)),
            "Sketch record must store adopted pose"
        );
    }
    runtime
        .apply(EditCommand::AddOperation {
            record: extrude(
                id,
                &finished.regions[0].region_id,
                finished.region_identity_version,
            ),
            at_cursor: true,
        })
        .unwrap();
    mesh_matches_profile(&mut runtime, &profile).await;
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("solved.onecad");
    runtime
        .save(
            &saved,
            SaveMeta {
                app_version: "adoption-test".into(),
                created: "2026-10-05T00:00:00Z".into(),
                modified: "2026-10-05T00:00:00Z".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let document = runtime.get_sketch(id).unwrap().entities;
    worker.shutdown().await;
    let fresh = WorkerManager::spawn(SupervisorConfig::production(path));
    assert!(fresh.wait_ready(Duration::from_secs(10)).await);
    let mut reopened = DocumentRuntime::open(
        &saved,
        Arc::new(fresh.clone()),
        Arc::new(fresh.clone()),
        Arc::new(fresh.clone()),
    )
    .unwrap();
    assert_eq!(reopened.get_sketch(id).unwrap().entities, document);
    mesh_matches_profile(&mut reopened, &profile).await;
    fresh.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skew_rectangle_document_matches_solved_profile() {
    let Some(path) = worker_path() else {
        return;
    };
    let worker = WorkerManager::spawn(SupervisorConfig::production(path));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    );
    let id = SketchId(Uuid::from_u128(0xad013));
    runtime
        .apply(EditCommand::AddSketch {
            sketch: Sketch::on_world_plane(id, "Skew rectangle", WorldPlane::XY),
        })
        .unwrap();
    runtime.enter_sketch(id).await.unwrap();
    let depth = runtime.undo_depth();
    let (solved, outcome) = runtime
        .sketch_upsert_with_outcome(id, ops(&skew_rectangle(id)))
        .await
        .unwrap();
    let document = runtime.get_sketch(id).unwrap();
    let prepared = runtime.prepare_sketch_regions(id).unwrap();
    let regions = prepared.drive().await.unwrap();
    let positions = &regions.regions[0]
        .preview_triangles
        .as_ref()
        .unwrap()
        .positions;
    eprintln!(
        "authored bottom={:?}/{:?}; worker profile={positions:?}; result={solved:?}",
        point(&document, 100),
        point(&document, 101)
    );
    assert!(
        (point(&document, 100)[1] - point(&document, 101)[1]).abs() < 1e-8,
        "document must adopt Horizontal solve instead of keeping the skewed authored bottom"
    );
    assert!(
        !solved.solved_positions.is_empty(),
        "exact worker corrections must be reported"
    );
    assert!(outcome.is_some());
    assert_eq!(
        runtime.undo_depth(),
        depth + 1,
        "edit and solve share one undo entry"
    );
    for point_id in 100..108 {
        let at = point(&document, point_id);
        assert!(
            positions
                .as_chunks::<2>()
                .0
                .iter()
                .any(|p| { (p[0] - at[0]).abs() < 1e-8 && (p[1] - at[1]).abs() < 1e-8 }),
            "document point {point_id} must match solved profile vertex"
        );
    }
    assert!(runtime.undo().is_some());
    assert!(runtime
        .get_sketch(id)
        .unwrap()
        .entities
        .as_array()
        .unwrap()
        .is_empty());
    assert!(runtime.redo().unwrap().is_some());
    assert_eq!(runtime.get_sketch(id).unwrap().entities, document.entities);
    worker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_entry_correction_is_discardable_and_finish_squashes_it() {
    let Some(path) = worker_path() else {
        return;
    };
    let worker = WorkerManager::spawn(SupervisorConfig::production(path));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    );
    let id = SketchId(Uuid::from_u128(0xad015));
    runtime
        .apply(EditCommand::AddSketch {
            sketch: skew_rectangle(id),
        })
        .unwrap();
    let original = runtime.get_sketch(id).unwrap().entities;
    let depth = runtime.undo_depth();
    let (entered, outcome) = runtime.enter_sketch_with_outcome(id).await.unwrap();
    assert!(outcome.is_some());
    let regions = runtime
        .prepare_sketch_regions(id)
        .unwrap()
        .drive()
        .await
        .unwrap();
    assert_adopted(
        &entered,
        &regions.regions[0]
            .preview_triangles
            .as_ref()
            .unwrap()
            .positions,
    );
    assert_eq!(runtime.undo_depth(), depth + 1);
    assert!(runtime.cancel_sketch(id, true).await.unwrap().discarded);
    assert_eq!(runtime.get_sketch(id).unwrap().entities, original);
    runtime.enter_sketch(id).await.unwrap();
    runtime.finish_sketch(id).await.unwrap();
    assert_eq!(
        runtime.undo_depth(),
        depth + 2,
        "one squashed sketch session plus timeline record"
    );
    runtime.undo().unwrap();
    runtime.undo().unwrap();
    assert_eq!(runtime.get_sketch(id).unwrap().entities, original);
    worker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_begin_keeps_pending_pose_until_end_and_cancel_discards_it() {
    let Some(path) = worker_path() else {
        return;
    };
    let worker = WorkerManager::spawn(SupervisorConfig::production(path));
    assert!(worker.wait_ready(Duration::from_secs(10)).await);
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    );
    let id = SketchId(Uuid::from_u128(0xad016));
    runtime
        .apply(EditCommand::AddSketch {
            sketch: skew_rectangle(id),
        })
        .unwrap();
    let original = runtime.get_sketch(id).unwrap().entities;
    let depth = runtime.undo_depth();
    let target = onecad_app::worker::wire::GestureTarget::point(entity(100));
    runtime
        .begin_gesture(id, entity(100), target)
        .await
        .unwrap();
    assert_eq!(runtime.get_sketch(id).unwrap().entities, original);
    assert!(!runtime.get_sketch(id).unwrap().solve_current);
    runtime.cancel_sketch(id, false).await.unwrap();
    assert_eq!(runtime.get_sketch(id).unwrap().entities, original);
    assert_eq!(runtime.undo_depth(), depth);
    runtime
        .begin_gesture(id, entity(100), target)
        .await
        .unwrap();
    let solved = runtime.end_gesture(None).await.unwrap();
    assert!(
        !solved.solved_positions.is_empty(),
        "End receipt must include pending Begin correction"
    );
    let regions = runtime
        .prepare_sketch_regions(id)
        .unwrap()
        .drive()
        .await
        .unwrap();
    assert_adopted(
        &runtime.get_sketch(id).unwrap(),
        &regions.regions[0]
            .preview_triangles
            .as_ref()
            .unwrap()
            .positions,
    );
    assert_eq!(runtime.undo_depth(), depth + 1);
    runtime.undo().unwrap();
    assert_eq!(runtime.get_sketch(id).unwrap().entities, original);
    worker.shutdown().await;
}

#[path = "fixtures/replay.rs"]
mod fixture_replay;

#[tokio::test]
async fn canonical_upsert_writeback_fixture_replays_on_real_worker() {
    let Some(worker) = worker_path() else {
        return;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fixture_replay::replay(
        &worker,
        &root.join("protocol/fixtures/sketch_upsert_writeback.ndjson"),
    )
    .await;
}

#[tokio::test]
async fn canonical_exact_truth_fixture_replays_on_real_worker() {
    let Some(worker) = worker_path() else {
        return;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fixture_replay::replay(
        &worker,
        &root.join("protocol/fixtures/sketch_exact_truth.ndjson"),
    )
    .await;
}

#[tokio::test]
async fn canonical_point_drag_truth_fixture_replays_on_real_worker() {
    let Some(worker) = worker_path() else {
        return;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fixture_replay::replay(
        &worker,
        &root.join("protocol/fixtures/sketch_point_drag_truth.ndjson"),
    )
    .await;
}
