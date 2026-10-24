//! Native interaction drafts through real OCCT; no GUI or physical-input claim.
#[allow(dead_code)]
mod support;
use onecad_app::{
    document_runtime::DocumentRuntime,
    worker::{
        manager::SupervisorConfig, wire::sketch_wire, ElementQuery, MeshProvider, PreviewEngine,
        SolverEngine, WorkerManager,
    },
};
use onecad_core::{
    document::{record::*, refs::SketchRegionRef, variables::Scalar},
    edit::EditCommand,
    ids::{BodyId, EntityId, RecordId, RegionId, SketchId},
    io::container::SaveMeta,
    math::{Vec2, Vec3},
    regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest},
    sketch::{Sketch, SketchEntity, SketchPlane, WorldPlane},
};
use onecad_interaction::*;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

fn record(id: u128, op: KnownOperation) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(id)),
        0,
        "Extrude mode gate",
        Operation::Known(op),
    )
}

fn circle_sketch(id: u128, plane: SketchPlane, center: f64, annulus: bool) -> Sketch {
    let mut sketch =
        Sketch::on_world_plane(SketchId(Uuid::from_u128(id)), "Profile", WorldPlane::XY);
    sketch.plane = plane;
    let point = EntityId(Uuid::from_u128(id + 100));
    sketch
        .add_entity(SketchEntity::point(
            point,
            Vec2::new_unchecked(center, 0.0),
            false,
            false,
        ))
        .unwrap();
    sketch
        .add_entity(
            SketchEntity::circle(
                EntityId(Uuid::from_u128(id + 101)),
                point,
                if annulus { 5.0 } else { 10.0 },
                false,
            )
            .unwrap(),
        )
        .unwrap();
    if annulus {
        sketch
            .add_entity(
                SketchEntity::circle(EntityId(Uuid::from_u128(id + 102)), point, 2.0, false)
                    .unwrap(),
            )
            .unwrap();
    }
    sketch
}

fn circle_record(id: u128, plane: SketchPlane, center: f64, annulus: bool) -> OperationRecord {
    let sketch = circle_sketch(id, plane, center, annulus);
    let (_, entities, constraints) = sketch_wire(&sketch);
    record(
        id,
        KnownOperation::Sketch(SketchOpParams {
            sketch: sketch.id,
            plane: SketchPlaneRef {
                kind: PlaneKind::Custom,
                origin: plane.origin,
                x_axis: plane.x_axis,
                y_axis: plane.y_axis,
                normal: plane.normal,
                extra: Default::default(),
            },
            entities: entities.as_array().unwrap().clone(),
            constraints: constraints.as_array().unwrap().clone(),
            host_face: None,
            extra: Default::default(),
        }),
    )
}

fn cylinder(id: u128, sketch: u128) -> OperationRecord {
    record(
        id,
        KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch: SketchId(Uuid::from_u128(sketch)),
                region: RegionId::new(""),
                region_identity_version: None,
                region_anchor: None,
                extra: Default::default(),
            }),
            distance: Scalar::new(5.0),
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
        }),
    )
}

fn append(runtime: &mut DocumentRuntime, record: OperationRecord) {
    runtime
        .apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .unwrap();
}

async fn regen(runtime: &mut DocumentRuntime) -> Arc<ModelSnapshot> {
    regenerate(runtime, RegenRequest::ToEnd { from: 0 }).await
}

async fn regenerate(runtime: &mut DocumentRuntime, request: RegenRequest) -> Arc<ModelSnapshot> {
    let report = runtime.run_regen(request, CancelToken::new()).await;
    let Outcome::Published(snapshot) = report.outcome else {
        panic!("{report:?}")
    };
    assert_eq!(snapshot.repair_summary.needs_repair_count, 0);
    snapshot
}

#[tokio::test]
async fn add_cut_intersect_keep_explicit_second_body_profile_hole_and_oriented_frame() {
    let worker_path =
        PathBuf::from(std::env::var_os("ONECAD_WORKER_PATH").expect("required real worker"));
    assert!(worker_path.is_file());
    for plane in [SketchPlane::xy(), SketchPlane::xz()] {
        for mode in [BooleanMode::Add, BooleanMode::Cut, BooleanMode::Intersect] {
            qualify(worker_path.clone(), plane, mode).await;
        }
    }
}

async fn baseline(
    worker: &WorkerManager,
    plane: SketchPlane,
    mode: BooleanMode,
) -> (DocumentRuntime, Arc<ModelSnapshot>, SketchPlane) {
    let engine: Arc<dyn GeometryEngine> = Arc::new(worker.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(worker.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(worker.clone());
    let mut runtime = DocumentRuntime::new_blank(engine, meshes, solver);
    append(&mut runtime, circle_record(1000, plane, 40.0, false));
    append(&mut runtime, cylinder(2000, 1000));
    append(&mut runtime, circle_record(3000, plane, 0.0, false));
    append(&mut runtime, cylinder(4000, 3000));
    let mut profile_plane = plane;
    if mode == BooleanMode::Add {
        profile_plane.origin = Vec3::new_unchecked(
            plane.normal.x * 5.0,
            plane.normal.y * 5.0,
            plane.normal.z * 5.0,
        );
    }
    append(&mut runtime, circle_record(5000, profile_plane, 0.0, true));
    let before = regen(&mut runtime).await;
    assert_eq!(before.bodies.len(), 2);
    (runtime, before, profile_plane)
}

async fn candidate(
    worker: &WorkerManager,
    snapshot: onecad_core::ids::SnapshotId,
    plane: SketchPlane,
    mode: BooleanMode,
) -> (Interaction, RequestTag, Vec<Effect>) {
    worker
        .sketch_upsert(&circle_sketch(5000, plane, 0.0, true))
        .await
        .unwrap();
    let regions = worker
        .sketch_regions(&Uuid::from_u128(5000).to_string())
        .await
        .unwrap();
    let annulus = regions
        .regions
        .iter()
        .find(|region| region.holes.len() == 1)
        .expect("exact annular region");
    let mut context = support::context();
    context.snapshot_id = snapshot;
    let mut selection = support::selection(context.clone());
    selection.profile = SketchRegionRef {
        sketch: SketchId(Uuid::from_u128(5000)),
        region: RegionId::new(&annulus.region_id),
        region_identity_version: Some(regions.region_identity_version),
        region_anchor: Some([3.0, 0.0]),
        extra: Default::default(),
    };
    let mut interaction = Interaction::default();
    interaction.dispatch(Action::ContextChanged(Some(context.clone())));
    interaction.dispatch(Action::SelectProfile(Box::new(selection)));
    interaction.dispatch(Action::BeginExtrude {
        record_id: RecordId(Uuid::from_u128(6000)),
        gesture: 1,
        step_index: 5,
    });
    interaction.dispatch(Action::ExtrudeModeChanged(mode));
    interaction.dispatch(Action::ExtrudeTargetChanged {
        body: Some(BodyId(Uuid::from_u128(4000))),
        context,
    });
    let tag = support::evaluation_tag(&interaction.dispatch(Action::DraftChanged("3mm".into())));
    let effects = support::evaluate(&mut interaction, &tag, 3.0);
    (interaction, tag, effects)
}

async fn preview(worker: &WorkerManager, tag: &RequestTag, effects: &[Effect]) -> PreviewResultDto {
    let [Effect::Preview {
        operation,
        sketch_id,
        expected_snapshot,
        ..
    }] = effects
    else {
        panic!("preview")
    };
    let result = worker
        .preview_op(
            operation.as_ref().clone(),
            tag.record_id.to_string(),
            sketch_id.map(|id| id.to_string()),
            Some(*expected_snapshot),
            Lod::Coarse,
        )
        .await
        .unwrap();
    assert!(result.needs_repair.is_empty());
    assert_eq!(
        result.changed_bodies,
        [BodyId(Uuid::from_u128(4000)).to_string()]
    );
    assert!(result.deleted_bodies.is_empty());
    result
}

fn commit(
    runtime: &mut DocumentRuntime,
    interaction: &mut Interaction,
    tag: RequestTag,
    result: PreviewResultDto,
) {
    interaction.dispatch(Action::PreviewFinished {
        tag,
        result: Ok(result),
    });
    let command = interaction
        .dispatch(Action::Confirm)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ApplyOperation { command, .. } => Some(command),
            _ => None,
        })
        .unwrap();
    runtime.apply(*command).unwrap();
}

fn assert_same_mesh_vertices(old_bytes: &[u8], next_bytes: &[u8]) {
    let old =
        onecad_scene::MeshView::decode(old_bytes, onecad_scene::MeshLimits::default()).unwrap();
    let next =
        onecad_scene::MeshView::decode(next_bytes, onecad_scene::MeshLimits::default()).unwrap();
    assert_eq!(old.header().vertex_count, next.header().vertex_count);
    assert_eq!(old.header().triangle_count, next.header().triangle_count);
    for index in 0..old.header().vertex_count as usize {
        assert_eq!(old.position(index), next.position(index));
    }
}

async fn assert_volume(worker: &WorkerManager, mode: BooleanMode) {
    let target = BodyId(Uuid::from_u128(4000));
    let mass = worker
        .query_mass_properties(target, target.to_string())
        .await
        .unwrap();
    let delta = std::f64::consts::PI * (25.0 - 4.0) * 3.0;
    let cylinder = std::f64::consts::PI * 100.0 * 5.0;
    let expected = match mode {
        BooleanMode::NewBody => cylinder,
        BooleanMode::Add => cylinder + delta,
        BooleanMode::Cut => cylinder - delta,
        BooleanMode::Intersect => delta,
    };
    assert!(
        (mass.volume - expected).abs() < 1e-6,
        "{mode:?} {} != {expected}",
        mass.volume
    );
}

async fn qualify(worker_path: PathBuf, plane: SketchPlane, mode: BooleanMode) {
    let worker = WorkerManager::spawn(SupervisorConfig::production(worker_path.clone()));
    assert!(worker.wait_ready(Duration::from_secs(15)).await);
    let (mut runtime, before, profile_plane) = baseline(&worker, plane, mode).await;
    let other = BodyId(Uuid::from_u128(2000));
    let other_mass = worker
        .query_mass_properties(other, other.to_string())
        .await
        .unwrap();
    let other_mesh = runtime.get_mesh(other, Lod::Coarse, None).await.unwrap();
    let (mut interaction, tag, effects) = candidate(&worker, before.id, profile_plane, mode).await;
    let result = preview(&worker, &tag, &effects).await;
    assert_eq!(
        runtime
            .subscribe_snapshots()
            .borrow()
            .as_ref()
            .unwrap()
            .as_ref(),
        before.as_ref()
    );
    commit(&mut runtime, &mut interaction, tag, result);
    assert_eq!(regen(&mut runtime).await.bodies.len(), 2);
    assert_eq!(
        other_mass,
        worker
            .query_mass_properties(other, other.to_string())
            .await
            .unwrap()
    );
    let next_mesh = runtime.get_mesh(other, Lod::Coarse, None).await.unwrap();
    assert_same_mesh_vertices(&other_mesh, &next_mesh);
    assert_volume(&worker, mode).await;
    let saved = tempfile::tempdir().unwrap();
    let path = saved.path().join("intersect.onecad");
    if mode == BooleanMode::Intersect {
        intersect_history_and_save(&mut runtime, &worker, &path).await;
    }
    worker.retire();
    assert!(worker.wait_torn_down(Duration::from_secs(10)).await);
    if mode == BooleanMode::Intersect {
        reopen_intersect(worker_path, &path).await;
    }
}

async fn intersect_history_and_save(
    runtime: &mut DocumentRuntime,
    worker: &WorkerManager,
    path: &Path,
) {
    let undo = runtime.undo().expect("undo Intersect");
    regenerate(runtime, undo.regen.expect("undo regeneration")).await;
    assert_eq!(runtime.projection().features.len(), 5);
    assert_volume(worker, BooleanMode::NewBody).await;
    let redo = runtime.redo().unwrap().expect("redo Intersect");
    regenerate(runtime, redo.regen.expect("redo regeneration")).await;
    assert_eq!(runtime.projection().features.len(), 6);
    assert_volume(worker, BooleanMode::Intersect).await;
    runtime
        .save(
            path,
            SaveMeta {
                app_version: "0.1.0-test".into(),
                occt_fingerprint: None,
                created: "2026-10-23T00:00:00Z".into(),
                modified: "2026-10-23T00:00:00Z".into(),
            },
        )
        .unwrap();
}

async fn reopen_intersect(worker_path: PathBuf, path: &Path) {
    let worker = WorkerManager::spawn(SupervisorConfig::production(worker_path));
    assert!(worker.wait_ready(Duration::from_secs(15)).await);
    let mut runtime = DocumentRuntime::open(
        path,
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
        Arc::new(worker.clone()),
    )
    .unwrap();
    regenerate(&mut runtime, RegenRequest::RevertToEnd { from: 0 }).await;
    assert_eq!(runtime.projection().features.len(), 6);
    let record = runtime
        .operation_record(RecordId(Uuid::from_u128(6000)))
        .unwrap();
    let Operation::Known(KnownOperation::Extrude(params)) = &record.op else {
        panic!("saved Intersect feature")
    };
    assert_eq!(params.boolean_mode, BooleanMode::Intersect);
    assert_eq!(params.target_body, Some(BodyId(Uuid::from_u128(4000))));
    let ids = runtime.head_body_ids();
    assert_eq!(ids.len(), 2);
    for id in [2000, 4000] {
        assert!(ids.contains(&BodyId(Uuid::from_u128(id))));
    }
    assert_volume(&worker, BooleanMode::Intersect).await;
    worker.retire();
    assert!(worker.wait_torn_down(Duration::from_secs(10)).await);
}
