//! Solid Loft through production application services; no native-input claim.
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;
use host::Harness;
use onecad_app::{
    api,
    dto::{DocumentProjection, PreviewResultDto},
    worker::wire::body_id_wire,
};
use onecad_core::{
    document::{
        datum::DatumPlane,
        record::*,
        refs::{ElementKind, SketchRegionRef},
        variables::Scalar,
    },
    edit::{EditCommand, SketchEditOp},
    ids::{BodyId, ConstraintId, DatumPlaneId, EntityId, RecordId, RegionId, SketchId},
    math::Vec2,
    regen::Lod,
    sketch::{Constraint, CurvePosition, Sketch, SketchAttachment, SketchEntity, WorldPlane},
};
use std::{f64::consts::PI, path::Path};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x10f7_0000 + n)
}
fn loft_id() -> RecordId {
    RecordId(id(10))
}
fn body() -> BodyId {
    BodyId(loft_id().0)
}
fn radius_dimension(n: u128) -> ConstraintId {
    ConstraintId(id(100 + n))
}
fn volume(radius: f64) -> f64 {
    20.0 * PI / 3.0 * (25.0 + radius * radius + 5.0 * radius)
}

#[test]
fn loft_preview_commit_edit_upstream_history_and_fresh_reopen() {
    host::required_worker();
    match std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE")
        .ok()
        .as_deref()
    {
        Some(phase @ ("loft-author" | "loft-reopen")) => {
            let root = std::path::PathBuf::from(
                std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap(),
            );
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    if phase == "loft-author" {
                        author(&root).await;
                    } else {
                        reopen(&root).await;
                    }
                });
        }
        _ => {
            let root = tempfile::tempdir().unwrap();
            for phase in ["loft-author", "loft-reopen"] {
                host::run_isolated_phase(
                    root.path(),
                    phase,
                    "loft_preview_commit_edit_upstream_history_and_fresh_reopen",
                );
            }
        }
    }
}

fn circle_ops(n: u128, radius: f64) -> Vec<SketchEditOp> {
    let point = EntityId(id(200 + n));
    let circle = EntityId(id(300 + n));
    vec![
        SketchEditOp::AddEntity {
            entity: SketchEntity::point(point, Vec2::new_unchecked(0.0, 0.0), false, false),
        },
        SketchEditOp::AddEntity {
            entity: SketchEntity::circle(circle, point, radius, false).unwrap(),
        },
        SketchEditOp::AddConstraint {
            constraint: Constraint::Fixed {
                id: ConstraintId(id(400 + n)),
                point,
                point_position: CurvePosition::Arbitrary,
                at: Vec2::new_unchecked(0.0, 0.0),
            },
        },
        SketchEditOp::AddConstraint {
            constraint: Constraint::Radius {
                id: radius_dimension(n),
                entity: circle,
                value: Scalar::new(radius),
            },
        },
    ]
}

async fn section(h: &Harness, n: u128, z: f64, radius: f64) -> SketchRegionRef {
    let sketch = SketchId(id(n));
    let mut empty = Sketch::on_world_plane(sketch, format!("Section {n}"), WorldPlane::XY);
    let datum = DatumPlaneId(id(500 + n));
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddDatumPlane {
            datum: DatumPlane::offset_from_plane(datum, format!("Section {n} plane"), "XY", z),
        },
    )
    .await
    .unwrap();
    empty.attachment = SketchAttachment::Datum { datum };
    api::apply_edit_command(
        &h.state,
        h.host.clone(),
        EditCommand::AddSketch { sketch: empty },
    )
    .await
    .unwrap();
    api::enter_sketch(&h.state, h.host.clone(), sketch.to_string(), Some(true))
        .await
        .unwrap();
    api::sketch_upsert(
        &h.state,
        h.host.clone(),
        sketch.to_string(),
        circle_ops(n, radius),
    )
    .await
    .unwrap();
    let finish = api::finish_sketch(&h.state, h.host.clone(), sketch.to_string())
        .await
        .unwrap();
    h.settled(&h.projection().await).await;
    assert_eq!(finish.regions.len(), 1);
    SketchRegionRef {
        sketch,
        region: RegionId::new(&finish.regions[0].region_id),
        region_identity_version: Some(finish.region_identity_version),
        region_anchor: Some([1.0, 0.0]),
        extra: Default::default(),
    }
}

fn loft(profiles: Vec<SketchRegionRef>, is_ruled: bool) -> Operation {
    Operation::Known(KnownOperation::Loft(LoftParams {
        profiles,
        is_solid: true,
        is_ruled,
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

async fn preview(h: &Harness, operation: Operation, record: RecordId) -> PreviewResultDto {
    let before = h.projection().await;
    let scope = api::preview_context(&h.state).await.unwrap();
    let result = api::preview_operation(
        &h.state,
        operation,
        record,
        None,
        Some(scope.snapshot_id),
        Lod::Fine,
    )
    .await
    .unwrap();
    let after = h.projection().await;
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.features, after.features);
    assert_eq!(before.dirty, after.dirty);
    assert!(result.needs_repair.is_empty());
    result
}

async fn exact_volume(h: &Harness) -> f64 {
    api::query_mass_properties(&h.state, body_id_wire(body()), None)
        .await
        .unwrap()
        .volume
}

async fn close_volume(h: &Harness, expected: f64) {
    let actual = exact_volume(h).await;
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

async fn edit(h: &Harness, operation: Operation) {
    let before = h.projection().await;
    let scope = api::preview_context(&h.state).await.unwrap();
    let result =
        api::preview_existing_edit(&h.state, scope, loft_id(), operation.clone(), Lod::Fine)
            .await
            .unwrap();
    assert_eq!(result.outcome, api::EditPreviewOutcome::Ready);
    assert_eq!(h.projection().await.revision, before.revision);
    let after = apply(
        h,
        EditCommand::UpdateOperationParams {
            record: loft_id(),
            op: operation,
        },
    )
    .await;
    assert_eq!(before.features.len(), after.features.len());
}

async fn refusals(h: &Harness, profiles: &[SketchRegionRef]) {
    let before = h.projection().await;
    let expected = exact_volume(h).await;
    let mut wrong_region = profiles.to_vec();
    wrong_region[1].region = RegionId::new("missing-region");
    wrong_region[1].region_anchor = None;
    let candidates = [
        loft(
            vec![
                profiles[0].clone(),
                profiles[2].clone(),
                profiles[1].clone(),
            ],
            true,
        ),
        loft(wrong_region, true),
    ];
    for candidate in candidates {
        let scope = api::preview_context(&h.state).await.unwrap();
        let result = api::preview_existing_edit(&h.state, scope, loft_id(), candidate, Lod::Fine)
            .await
            .unwrap();
        assert!(
            matches!(result.outcome, api::EditPreviewOutcome::EditedFailed { .. }),
            "{result:?}"
        );
        assert_eq!(h.projection().await.revision, before.revision);
        close_volume(h, expected).await;
    }
}

async fn upstream_radius(h: &Harness, radius: f64, drilled: bool) {
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
            constraint: radius_dimension(2),
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
    close_volume(h, volume(radius) - if drilled { 20.0 * PI } else { 0.0 }).await;
}

async fn downstream_reference_survives_upstream_history(h: &Harness) {
    let hole = RecordId(id(11));
    let original = api::get_operation_record(&h.state, hole).await.unwrap();
    upstream_radius(h, 10.0, true).await;
    assert_eq!(
        api::get_operation_record(&h.state, hole).await.unwrap().op,
        original.op
    );
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undo).await;
    close_volume(h, volume(9.0) - 20.0 * PI).await;
    let redo = api::redo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&redo).await;
    close_volume(h, volume(10.0) - 20.0 * PI).await;
    let restored = api::get_operation_record(&h.state, hole).await.unwrap();
    assert_eq!(restored.op, original.op);
    assert_eq!(restored.inputs, original.inputs);
}

async fn downstream_hole(h: &Harness) {
    let scope = api::preview_context(&h.state).await.unwrap();
    let mut top = None;
    for ordinal in 1..=128 {
        let key = format!("f:{ordinal}");
        let Some(info) = api::element_info(
            &h.state,
            scope.snapshot_id.0,
            body_id_wire(body()),
            String::new(),
            Some(key.clone()),
            None,
        )
        .await
        .unwrap() else {
            break;
        };
        if info.has_normal && info.normal[2] > 0.99 && (info.center[2] - 20.0).abs() < 1e-5 {
            top = Some(key);
            break;
        }
    }
    let (face, point) = api::stage_hole(
        &h.state,
        &scope,
        api::StagePick {
            body: body(),
            kind: ElementKind::Face,
            address: api::StageAddress::TopoKey(top.expect("Loft planar top face")),
        },
    )
    .await
    .unwrap();
    let operation = Operation::Known(KnownOperation::Hole(HoleParams {
        target_body: body(),
        face,
        point,
        points: None,
        hole_type: HoleType::Simple,
        diameter: Scalar::new(2.0),
        depth: None,
        cb_diameter: None,
        cb_depth: None,
        cs_diameter: None,
        cs_angle_deg: None,
        thread: None,
        result_policy_version: Some(2),
        extra: Default::default(),
    }));
    let record = RecordId(id(11));
    preview(h, operation.clone(), record).await;
    apply(
        h,
        EditCommand::AddOperation {
            record: OperationRecord::new(record, 4, "Hole on Loft", operation),
            at_cursor: true,
        },
    )
    .await;
    close_volume(h, volume(9.0) - 20.0 * PI).await;
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
    let mut profiles = Vec::new();
    for (n, z, radius) in [(1, 0.0, 5.0), (2, 10.0, 8.0), (3, 20.0, 5.0)] {
        profiles.push(section(&h, n, z, radius).await);
    }
    let operation = loft(profiles.clone(), false);
    let result = preview(&h, operation.clone(), loft_id()).await;
    assert_eq!(result.bodies.len(), 1);
    apply(
        &h,
        EditCommand::AddOperation {
            record: OperationRecord::new(loft_id(), 3, "Handle Loft", operation),
            at_cursor: true,
        },
    )
    .await;
    let smooth_volume = exact_volume(&h).await;
    assert!(smooth_volume > 500.0 * PI && smooth_volume < 1280.0 * PI);
    edit(&h, loft(profiles.clone(), true)).await;
    close_volume(&h, volume(8.0)).await;
    refusals(&h, &profiles).await;
    profiles.reverse();
    edit(&h, loft(profiles, true)).await;
    close_volume(&h, volume(8.0)).await;
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undo).await;
    close_volume(&h, volume(8.0)).await;
    let redo = api::redo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&redo).await;
    upstream_radius(&h, 9.0, false).await;
    downstream_hole(&h).await;
    downstream_reference_survives_upstream_history(&h).await;
    let projection = h.projection().await;
    assert!(projection.sketches.values().all(|s| s.visible));
    assert_eq!(
        projection
            .features
            .iter()
            .filter(|f| f.op_type == "Loft")
            .count(),
        1
    );
    api::save_document(
        &h.state,
        h.host.clone(),
        Some(root.join("loft.onecad").to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    h.close().await;
}

async fn reopen(root: &Path) {
    let h = Harness::new(&root.join("reopen")).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        root.join("loft.onecad").to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let projection = h.settled(&h.projection().await).await;
    assert_eq!(projection.bodies.len(), 1);
    assert!(projection.bodies.contains_key(&body().to_string()));
    close_volume(&h, volume(10.0) - 20.0 * PI).await;
    let record = api::get_operation_record(&h.state, loft_id())
        .await
        .unwrap();
    let Operation::Known(KnownOperation::Loft(p)) = record.op else {
        panic!("stored Loft");
    };
    assert!(p.is_ruled && p.is_solid);
    assert_eq!(
        p.profiles.iter().map(|p| p.sketch).collect::<Vec<_>>(),
        [SketchId(id(3)), SketchId(id(2)), SketchId(id(1))]
    );
    assert!(projection.sketches.values().all(|s| s.visible));
    h.close().await;
}
