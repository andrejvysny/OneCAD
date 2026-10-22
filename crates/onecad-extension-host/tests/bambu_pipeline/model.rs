use std::{path::PathBuf, sync::Arc, time::Duration};

use onecad_app::worker::resolve_worker_path;
use onecad_app::{
    document_runtime::DocumentRuntime,
    extension_handoff::ExportTarget,
    state::AppState,
    worker::{SupervisorConfig, WorkerManager},
};
use onecad_core::{
    document::{
        record::{
            BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
            PlaneKind, SketchOpParams, SketchPlaneRef,
        },
        refs::SketchRegionRef,
        variables::Scalar,
    },
    edit::EditCommand,
    ids::{BodyId, EntityId, RecordId, RegionId, SketchId},
    math::{Vec2, Vec3},
    regen::{CancelToken, Outcome, RegenRequest},
    sketch::{Sketch, SketchEntity, WorldPlane},
};
use uuid::Uuid;

pub fn worker_path() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(value);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH must name an existing worker"
        );
        return Some(path);
    }
    let worker = resolve_worker_path();
    assert!(
        worker.is_some() || std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no real worker is available"
    );
    worker
}

pub async fn two_boxes(path: PathBuf) -> (WorkerManager, AppState, ExportTarget, Vec<BodyId>) {
    let worker = WorkerManager::spawn(SupervisorConfig::production(path));
    assert!(worker.wait_ready(Duration::from_secs(30)).await);
    let mut runtime = DocumentRuntime::new_blank(
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    );
    let mut bodies = Vec::new();
    for (base, x) in [(0xBA000, 0.0), (0xBB000, 40.0)] {
        for record in box_records(base, x) {
            runtime
                .apply(EditCommand::AddOperation {
                    record,
                    at_cursor: true,
                })
                .unwrap();
        }
        bodies.push(BodyId(Uuid::from_u128(base + 1)));
    }
    let report = runtime
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{:?}",
        report.outcome
    );
    assert_eq!(runtime.head_body_ids().len(), 2);
    let state = AppState::default();
    *state.runtime.lock().await = Some(runtime);
    let target = capture_target(&state).await;
    (worker, state, target, bodies)
}

pub async fn capture_target(state: &AppState) -> ExportTarget {
    ExportTarget::from(&onecad_app::read::preview_context(state).await.unwrap())
}

pub async fn add_pending_box(state: &AppState) {
    let mut guard = state.runtime.lock().await;
    for record in box_records(0xBC000, 80.0) {
        guard
            .as_mut()
            .unwrap()
            .apply(EditCommand::AddOperation {
                record,
                at_cursor: true,
            })
            .unwrap();
    }
}

fn box_records(base: u128, x: f64) -> [OperationRecord; 2] {
    let sketch_id = SketchId(Uuid::from_u128(base + 2));
    let mut sketch = Sketch::on_world_plane(sketch_id, "Handoff profile", WorldPlane::XY);
    let point = |offset| EntityId(Uuid::from_u128(base + 10 + offset));
    for (index, (px, py)) in [(x, 0.0), (x + 20.0, 0.0), (x + 20.0, 12.0), (x, 12.0)]
        .into_iter()
        .enumerate()
    {
        sketch
            .add_entity(SketchEntity::point(
                point(index as u128),
                Vec2::new_unchecked(px, py),
                false,
                false,
            ))
            .unwrap();
    }
    for index in 0..4 {
        sketch
            .add_entity(SketchEntity::line(
                point(10 + index),
                point(index),
                point((index + 1) % 4),
                false,
            ))
            .unwrap();
    }
    let (_, entities, constraints) = onecad_app::worker::wire::sketch_wire(&sketch);
    let profile = OperationRecord::new(
        RecordId(Uuid::from_u128(base)),
        0,
        "Handoff sketch",
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sketch_id,
            plane: SketchPlaneRef {
                kind: PlaneKind::Xy,
                origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
                x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
                y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
                normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
                extra: Default::default(),
            },
            entities: entities.as_array().unwrap().clone(),
            constraints: constraints.as_array().unwrap().clone(),
            host_face: None,
            extra: Default::default(),
        })),
    );
    let extrude = OperationRecord::new(
        RecordId(Uuid::from_u128(base + 1)),
        0,
        "Handoff box",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch: sketch_id,
                region: RegionId::new(""),
                region_identity_version: None,
                region_anchor: None,
                extra: Default::default(),
            }),
            distance: Scalar::new(8.0),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        })),
    );
    [profile, extrude]
}
