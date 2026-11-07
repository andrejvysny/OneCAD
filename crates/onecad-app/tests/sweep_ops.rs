//! Bounded analytic Sweep via shared services; not native-input qualification.
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;

use host::Harness;
use onecad_app::{api, dto::DocumentProjection, worker::wire::body_id_wire};
use onecad_core::{
    document::{
        datum::DatumPlane,
        record::{BooleanMode, KnownOperation, Operation, OperationRecord, SweepParams},
        refs::SketchRegionRef,
        variables::Scalar,
    },
    edit::{EditCommand, SketchEditOp},
    ids::{BodyId, ConstraintId, DatumPlaneId, EntityId, RecordId, RegionId, SketchId, SnapshotId},
    math::Vec2,
    regen::Lod,
    sketch::{Constraint, CurvePosition, Sketch, SketchAttachment, SketchEntity, WorldPlane},
};
use onecad_protocol::mesh::{MeshLimits, MeshView};
use std::{f64::consts::PI, path::Path};
use uuid::Uuid;

const PATH_RADIUS: f64 = 4.0;
const FIRST_SECTION: f64 = 0.5;
const EDITED_SECTION: f64 = 0.6;
const REOPEN_SECTION: f64 = 0.7;
// Existing analytic mass-property tolerance used by the shared Loft workflow.
const VOLUME_TOLERANCE_MM3: f64 = 1e-5;
// Existing source-bounds tessellation envelope used by shared reference fixtures.
const MESH_BOUNDS_TOLERANCE_MM: f64 = 0.15;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(0x5ee0_0000 + value)
}
fn sweep_id() -> RecordId {
    RecordId(id(10))
}
fn body_id() -> BodyId {
    BodyId(sweep_id().0)
}
fn section_dimension() -> ConstraintId {
    ConstraintId(id(102))
}
fn volume(radius: f64) -> f64 {
    2.0 * PI * PI * PATH_RADIUS * radius * radius
}

fn circle_ops(n: u128, center: [f64; 2], radius: f64) -> Vec<SketchEditOp> {
    let point = EntityId(id(200 + n));
    let circle = EntityId(id(300 + n));
    let at = Vec2::new_unchecked(center[0], center[1]);
    vec![
        SketchEditOp::AddEntity {
            entity: SketchEntity::point(point, at, false, false),
        },
        SketchEditOp::AddEntity {
            entity: SketchEntity::circle(circle, point, radius, false).unwrap(),
        },
        SketchEditOp::AddConstraint {
            constraint: Constraint::Fixed {
                id: ConstraintId(id(400 + n)),
                point,
                point_position: CurvePosition::Arbitrary,
                at,
            },
        },
        SketchEditOp::AddConstraint {
            constraint: Constraint::Radius {
                id: ConstraintId(id(100 + n)),
                entity: circle,
                value: Scalar::new(radius),
            },
        },
    ]
}

async fn finish_circle(
    h: &Harness,
    sketch: Sketch,
    n: u128,
    center: [f64; 2],
    radius: f64,
) -> SketchRegionRef {
    let id = sketch.id;
    api::apply_edit_command(&h.state, h.host.clone(), EditCommand::AddSketch { sketch })
        .await
        .unwrap();
    api::enter_sketch(&h.state, h.host.clone(), id.to_string(), Some(true))
        .await
        .unwrap();
    api::sketch_upsert(
        &h.state,
        h.host.clone(),
        id.to_string(),
        circle_ops(n, center, radius),
    )
    .await
    .unwrap();
    let finish = api::finish_sketch(&h.state, h.host.clone(), id.to_string())
        .await
        .unwrap();
    h.settled(&h.projection().await).await;
    assert_eq!(finish.regions.len(), 1);
    SketchRegionRef {
        sketch: id,
        region: RegionId::new(&finish.regions[0].region_id),
        region_identity_version: Some(finish.region_identity_version),
        region_anchor: Some(center),
        extra: Default::default(),
    }
}

async fn create_sketches(h: &Harness) -> (SketchId, SketchRegionRef) {
    let path = SketchId(id(1));
    let path_sketch = Sketch::on_world_plane(path, "Circular path", WorldPlane::XY);
    finish_circle(h, path_sketch, 1, [0.0, 0.0], PATH_RADIUS).await;
    let datum = DatumPlaneId(id(5));
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(datum, "Tangent section", "YZ", 0.0),
        },
    )
    .await
    .unwrap();
    let section = Sketch::new(
        SketchId(id(2)),
        "Circular section",
        SketchAttachment::Datum { datum },
    );
    // The named YZ basis has local +X along world -X. Its -4 mm point is path start +X.
    let profile = finish_circle(h, section, 2, [-PATH_RADIUS, 0.0], FIRST_SECTION).await;
    (path, profile)
}

fn operation(path: SketchId, profile: SketchRegionRef) -> Operation {
    Operation::Known(KnownOperation::Sweep(SweepParams {
        profile: Some(profile),
        path_sketch: Some(path),
        path_edge: None,
        boolean_mode: BooleanMode::NewBody,
        extra: Default::default(),
    }))
}

async fn apply(h: &Harness, command: EditCommand) -> DocumentProjection {
    let response = api::apply_operation(&h.state, h.host.clone(), vec![command], None)
        .await
        .unwrap();
    h.settled(&response.projection).await
}

fn mesh_bounds(bytes: &[u8], radius: f64) {
    let mesh = MeshView::decode(bytes, MeshLimits::default()).unwrap();
    assert!(mesh.header().triangle_count > 0);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for i in 0..mesh.header().triangle_count as usize {
        for v in mesh.triangle(i).unwrap() {
            let p = mesh.position(v as usize).unwrap();
            for axis in 0..3 {
                lo[axis] = lo[axis].min(p[axis] as f64);
                hi[axis] = hi[axis].max(p[axis] as f64);
            }
        }
    }
    let expected = [PATH_RADIUS + radius, PATH_RADIUS + radius, radius];
    for axis in 0..3 {
        assert!((lo[axis] + expected[axis]).abs() <= MESH_BOUNDS_TOLERANCE_MM);
        assert!((hi[axis] - expected[axis]).abs() <= MESH_BOUNDS_TOLERANCE_MM);
    }
}

async fn check_volume(h: &Harness, radius: f64) {
    let measured = api::query_mass_properties(&h.state, body_id_wire(body_id()), None)
        .await
        .unwrap()
        .volume;
    assert!(
        (measured - volume(radius)).abs() < VOLUME_TOLERANCE_MM3,
        "sweep volume {measured} vs analytic {}",
        volume(radius)
    );
}

async fn preview(h: &Harness, operation: Operation) {
    let before = h.projection().await;
    let scope = api::preview_context(&h.state).await.unwrap();
    let result = api::preview_operation(
        &h.state,
        operation,
        sweep_id(),
        None,
        Some(scope.snapshot_id),
        Lod::Fine,
    )
    .await
    .unwrap();
    assert_eq!(result.bodies.len(), 1);
    assert!(result.needs_repair.is_empty());
    mesh_bounds(&result.bodies[0].mesh, FIRST_SECTION);
    let after = h.projection().await;
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.features, after.features);
    assert_eq!(before.dirty, after.dirty);
}

async fn stale_preview(h: &Harness, op: Operation) {
    let before = h.projection().await;
    let scope = api::preview_context(&h.state).await.unwrap();
    let invalid_snapshot = SnapshotId(scope.snapshot_id.0.checked_add(1).unwrap());
    let result = api::preview_operation(
        &h.state,
        op,
        sweep_id(),
        None,
        Some(invalid_snapshot),
        Lod::Fine,
    )
    .await;
    assert!(result.is_err(), "stale preview must be refused");
    let after = h.projection().await;
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.features, after.features);
    assert_eq!(before.dirty, after.dirty);
}

async fn edit_section(h: &Harness, radius: f64) {
    let sketch = SketchId(id(2));
    let enter = api::enter_sketch_with_completion(
        &h.state,
        h.host.clone(),
        sketch.to_string(),
        Some(false),
    )
    .await
    .unwrap();
    enter.result.unwrap();
    if enter.regen_expected {
        h.settled(&enter.projection).await;
    }
    let update = api::sketch_upsert_with_completion(
        &h.state,
        h.host.clone(),
        sketch.to_string(),
        vec![SketchEditOp::SetDimension {
            constraint: section_dimension(),
            value: Scalar::new(radius),
        }],
    )
    .await
    .unwrap();
    update.result.unwrap();
    if update.regen_expected {
        h.settled(&update.projection).await;
    }
    let finish = api::finish_sketch_with_completion(&h.state, h.host.clone(), sketch.to_string())
        .await
        .unwrap();
    finish.result.unwrap();
    if finish.regen_expected {
        h.settled(&finish.projection).await;
    }
    check_volume(h, radius).await;
}

async fn refused_edit(h: &Harness, path: SketchId, profile: &SketchRegionRef) {
    let before = h.projection().await;
    let mut missing = profile.clone();
    missing.region = RegionId::new("missing-region");
    missing.region_anchor = None;
    let scope = api::preview_context(&h.state).await.unwrap();
    let preview = api::preview_existing_edit(
        &h.state,
        scope,
        sweep_id(),
        operation(path, missing),
        Lod::Fine,
    )
    .await
    .unwrap();
    assert!(matches!(
        preview.outcome,
        api::EditPreviewOutcome::EditedFailed { .. }
    ));
    assert_eq!(h.projection().await.revision, before.revision);
    check_volume(h, FIRST_SECTION).await;
}

async fn author(root: &Path) {
    let h = Harness::new(&root.join("author")).await;
    api::new_document(&h.state, h.host.clone()).await.unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let (path, profile) = create_sketches(&h).await;
    let sweep = operation(path, profile.clone());
    preview(&h, sweep.clone()).await;
    stale_preview(&h, sweep.clone()).await;
    let projection = apply(
        &h,
        EditCommand::AddOperation {
            record: OperationRecord::new(sweep_id(), 2, "Sweep", sweep),
            at_cursor: true,
        },
    )
    .await;
    assert_eq!(projection.bodies.len(), 1);
    let bytes = api::get_mesh(&h.state, body_id().to_string(), "fine".into(), None, None)
        .await
        .unwrap();
    mesh_bounds(&bytes, FIRST_SECTION);
    check_volume(&h, FIRST_SECTION).await;
    refused_edit(&h, path, &profile).await;
    edit_section(&h, EDITED_SECTION).await;
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undo).await;
    check_volume(&h, FIRST_SECTION).await;
    let redo = api::redo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&redo).await;
    check_volume(&h, EDITED_SECTION).await;
    assert!(
        api::save_document(
            &h.state,
            h.host.clone(),
            Some(root.join("sweep.onecad").to_string_lossy().into_owned()),
            None
        )
        .await
        .unwrap()
        .clean
    );
    h.close().await;
}

async fn reopen(root: &Path) {
    let h = Harness::new(&root.join("reopen")).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        root.join("sweep.onecad").to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let projection = h.settled(&h.projection().await).await;
    assert_eq!(projection.bodies.len(), 1);
    assert!(projection.bodies.contains_key(&body_id().to_string()));
    check_volume(&h, EDITED_SECTION).await;
    let record = api::get_operation_record(&h.state, sweep_id())
        .await
        .unwrap();
    let Operation::Known(KnownOperation::Sweep(params)) = record.op else {
        panic!("stored Sweep")
    };
    assert_eq!(params.path_sketch, Some(SketchId(id(1))));
    assert_eq!(params.profile.unwrap().sketch, SketchId(id(2)));
    edit_section(&h, REOPEN_SECTION).await;
    h.close().await;
}

#[test]
fn sweep_preview_commit_edit_undo_save_cold_reopen() {
    host::required_worker();
    match std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE")
        .ok()
        .as_deref()
    {
        Some(phase @ ("sweep-author" | "sweep-reopen")) => {
            let root = std::path::PathBuf::from(
                std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap(),
            );
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    if phase == "sweep-author" {
                        author(&root).await;
                    } else {
                        reopen(&root).await;
                    }
                });
        }
        _ => {
            let root = tempfile::tempdir().unwrap();
            for phase in ["sweep-author", "sweep-reopen"] {
                host::run_isolated_phase(
                    root.path(),
                    phase,
                    "sweep_preview_commit_edit_undo_save_cold_reopen",
                );
            }
        }
    }
}
