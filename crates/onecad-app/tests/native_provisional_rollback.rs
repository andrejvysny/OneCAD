//! Shared-API failure policy with a real worker. This exercises guarded receipts,
//! not native input; the host decides which matching terminal permits rollback.
// The shared workflow helper also exports assertions unused by this failure lane.
#[allow(dead_code)]
mod native_plate_support;

use native_plate_support::{profile, Harness};
use onecad_app::{
    api::{self, RollbackFailedOperationResponse},
    dto::{DocumentProjection, FeatureStatus, FinishSketchDto, RegenFinished},
};
use onecad_core::{
    document::{
        record::{FilletParams, KnownOperation, Operation, OperationRecord},
        refs::{ElementKind, ElementRef, PrimaryRef},
        variables::Scalar,
    },
    edit::EditCommand,
    ids::{ConstraintId, ElementId, EntityId, RecordId, SketchId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane},
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const CIRCLE: u128 = 0x273f0;
const FAILED: u128 = 0x273f1;
const REPAIR: u128 = 0x273f2;
fn circle_id() -> SketchId {
    SketchId(Uuid::from_u128(CIRCLE))
}
fn failed_id() -> RecordId {
    RecordId(Uuid::from_u128(FAILED))
}

#[test]
fn real_worker_provisional_rollback_policy() {
    native_plate_support::required_worker();
    if let Ok(phase) = std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(run(&root, &phase));
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("onecad-273-rollback-")
        .tempdir()
        .unwrap();
    for phase in ["new", "edit", "repair", "stale"] {
        native_plate_support::run_isolated_phase(
            root.path(),
            phase,
            "real_worker_provisional_rollback_policy",
        );
    }
}

async fn run(root: &Path, phase: &str) {
    let harness = Harness::new(&root.join(format!("{phase}-host"))).await;
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    let worker = harness
        .state
        .live_worker()
        .expect("real worker must be adopted");
    assert!(worker.wait_ready(std::time::Duration::from_secs(15)).await);
    let rectangle = profile::create(&harness).await;
    let original = apply(
        &harness,
        profile::extrude_id(),
        profile::extrude(&rectangle, 6.0),
        false,
        profile::sketch_id(),
    )
    .await;
    harness.settled(&original.projection).await;
    let circle = circle_profile(&harness).await;
    let baseline = harness.projection().await;
    match phase {
        "new" => failed_new_restores(&harness, &circle, &baseline).await,
        "edit" => failed_edit_is_retained(&harness, &circle, &baseline).await,
        "repair" => needs_repair_is_retained(&harness, &baseline).await,
        "stale" => stale_receipt_never_undoes(&harness, &circle).await,
        _ => panic!("unknown rollback phase"),
    }
    harness.close().await;
}

async fn circle_profile(harness: &Harness) -> FinishSketchDto {
    let sketch = circle_sketch();
    api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddSketch { sketch },
    )
    .await
    .unwrap();
    let session = api::enter_sketch(
        &harness.state,
        harness.host.clone(),
        circle_id().to_string(),
        Some(true),
    )
    .await
    .unwrap();
    assert!(
        session.conflicting.is_empty(),
        "the refusal profile must remain solvable"
    );
    let regions = api::finish_sketch(
        &harness.state,
        harness.host.clone(),
        circle_id().to_string(),
    )
    .await
    .unwrap();
    assert_eq!(regions.regions.len(), 1);
    harness.settled(&harness.projection().await).await;
    regions
}

fn circle_sketch() -> Sketch {
    let center = EntityId(Uuid::from_u128(CIRCLE + 10));
    let curve = EntityId(Uuid::from_u128(CIRCLE + 11));
    let mut sketch = Sketch::on_world_plane(circle_id(), "Draft refusal profile", WorldPlane::XY);
    sketch
        .add_entity(SketchEntity::point(
            center,
            Vec2::new_unchecked(100.0, 0.0),
            false,
            false,
        ))
        .unwrap();
    sketch
        .add_entity(SketchEntity::circle(curve, center, 5.0, false).unwrap())
        .unwrap();
    sketch
        .add_constraint(Constraint::Fixed {
            id: ConstraintId(Uuid::from_u128(CIRCLE + 12)),
            point: center,
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new_unchecked(100.0, 0.0),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Radius {
            id: ConstraintId(Uuid::from_u128(CIRCLE + 13)),
            entity: curve,
            value: Scalar::new(5.0),
        })
        .unwrap();
    sketch
}

fn refused_draft(circle: &FinishSketchDto) -> Operation {
    let mut operation = profile::extrude(circle, 6.0);
    let Operation::Known(KnownOperation::Extrude(params)) = &mut operation else {
        unreachable!()
    };
    params.profile.as_mut().unwrap().sketch = circle_id();
    params.draft_angle_deg = Scalar::new(10.0);
    operation
}

async fn apply(
    harness: &Harness,
    id: RecordId,
    operation: Operation,
    edit: bool,
    sketch: SketchId,
) -> api::ApplyOperationResponse {
    let consumes_sketch = operation.derive_inputs().sketches.contains(&sketch);
    let command = if edit {
        EditCommand::UpdateOperationParams {
            record: id,
            op: operation,
        }
    } else {
        EditCommand::AddOperation {
            record: OperationRecord::new(
                id,
                harness.projection().await.total_ops as u32,
                "Rollback probe",
                operation,
            ),
            at_cursor: true,
        }
    };
    let effects = (!edit && consumes_sketch).then(|| api::OperationEffects {
        hide_sketch_ids: vec![sketch.to_string()],
    });
    api::apply_operation(&harness.state, harness.host.clone(), vec![command], effects)
        .await
        .unwrap()
}

async fn assert_refused(
    harness: &Harness,
    response: &api::ApplyOperationResponse,
    id: RecordId,
    expected: &str,
) -> RegenFinished {
    let terminal = harness.terminal(&response.projection).await;
    assert!(terminal.source_revision >= response.projection.revision);
    assert!(terminal.repair_steps.is_empty(), "{terminal:?}");
    let failure = terminal
        .failed_steps
        .iter()
        .find(|failed| failed.record_id == id.to_string())
        .expect("authoritative terminal must identify failed operation");
    assert!(format!("{failure:?}").contains(expected), "{failure:?}");
    terminal
}

async fn baseline_mesh(harness: &Harness) -> Vec<u8> {
    let projection = harness.projection().await;
    let blob = api::get_mesh(
        &harness.state,
        profile::extrude_id().to_string(),
        "fine".into(),
        None,
        Some(projection.runtime_session),
    )
    .await
    .unwrap();
    native_plate_support::geometry::assert_mesh(&blob, 6.0);
    blob
}

fn legacy_shape(response: &RollbackFailedOperationResponse) {
    let value = serde_json::to_value(response).unwrap();
    assert!(value.get("regenExpected").is_none());
    assert!(value.get("regen_expected").is_none());
    assert_eq!(value.as_object().unwrap().len(), 3);
    assert!(
        value.get("projection").is_some()
            && value.get("rolledBack").is_some()
            && value.get("reason").is_some()
    );
}

async fn failed_new_restores(
    harness: &Harness,
    circle: &FinishSketchDto,
    baseline: &DocumentProjection,
) {
    let mesh = baseline_mesh(harness).await;
    let response = apply(
        harness,
        failed_id(),
        refused_draft(circle),
        false,
        circle_id(),
    )
    .await;
    assert_refused(
        harness,
        &response,
        failed_id(),
        "EXTRUDE_DRAFT_NO_PLANAR_FACE",
    )
    .await;
    assert!(!harness.projection().await.sketches[&circle_id().to_string()].visible);
    let token = response
        .rollback_token
        .expect("accepted new operation has a guarded receipt");
    let rolled =
        api::rollback_failed_operation(&harness.state, harness.host.clone(), token.clone())
            .await
            .unwrap();
    assert!(rolled.rolled_back);
    assert!(
        rolled.regen_expected,
        "a successful feature revert requests scheduler regeneration"
    );
    legacy_shape(&rolled);
    let restored = harness.settled(&rolled.projection).await;
    assert_eq!(restored.undo_depth, baseline.undo_depth);
    assert_eq!(restored.total_ops, baseline.total_ops);
    assert_eq!(restored.bodies, baseline.bodies);
    assert!(restored.sketches[&circle_id().to_string()].visible);
    assert!(api::get_operation_record(&harness.state, failed_id())
        .await
        .is_err());
    assert_eq!(baseline_mesh(harness).await, mesh);
    consumed_receipt_is_noop(harness, token).await;
}

async fn consumed_receipt_is_noop(harness: &Harness, token: String) {
    let before = harness.projection().await;
    let duplicate = api::rollback_failed_operation(&harness.state, harness.host.clone(), token)
        .await
        .unwrap();
    assert!(!duplicate.rolled_back && !duplicate.regen_expected);
    assert_eq!(duplicate.reason, "unknownOrConsumed");
    assert_eq!(duplicate.projection, before);
    legacy_shape(&duplicate);
}

async fn failed_edit_is_retained(
    harness: &Harness,
    _circle: &FinishSketchDto,
    baseline: &DocumentProjection,
) {
    let operation = zero_distance_operation(harness).await;
    let response = apply(
        harness,
        profile::extrude_id(),
        operation.clone(),
        true,
        circle_id(),
    )
    .await;
    assert_refused(
        harness,
        &response,
        profile::extrude_id(),
        "Extrude distance too small",
    )
    .await;
    let current = harness.projection().await;
    assert_eq!(current.undo_depth, baseline.undo_depth + 1);
    assert_eq!(current.total_ops, baseline.total_ops);
    assert!(
        current.sketches[&circle_id().to_string()].visible,
        "existing edit must not auto-hide another profile"
    );
    assert_eq!(
        api::get_operation_record(&harness.state, profile::extrude_id())
            .await
            .unwrap()
            .op,
        operation
    );
    assert_eq!(
        current
            .features
            .iter()
            .find(|feature| feature.id == profile::extrude_id().to_string())
            .unwrap()
            .status,
        FeatureStatus::Error
    );
    assert!(
        response.rollback_token.is_some(),
        "a receipt's existence alone does not authorize rolling back an existing edit"
    );
}

async fn zero_distance_operation(harness: &Harness) -> Operation {
    let mut operation = api::get_operation_record(&harness.state, profile::extrude_id())
        .await
        .unwrap()
        .op;
    let Operation::Known(KnownOperation::Extrude(params)) = &mut operation else {
        unreachable!()
    };
    params.distance = Scalar::new(0.0);
    operation
}

async fn stale_receipt_never_undoes(harness: &Harness, circle: &FinishSketchDto) {
    let response = apply(
        harness,
        failed_id(),
        refused_draft(circle),
        false,
        circle_id(),
    )
    .await;
    assert_refused(
        harness,
        &response,
        failed_id(),
        "EXTRUDE_DRAFT_NO_PLANAR_FACE",
    )
    .await;
    api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::RenameSketch {
            sketch: circle_id(),
            name: "Intervening accepted edit".into(),
        },
    )
    .await
    .unwrap();
    let before = harness.projection().await;
    let rolled = api::rollback_failed_operation(
        &harness.state,
        harness.host.clone(),
        response.rollback_token.unwrap(),
    )
    .await
    .unwrap();
    assert!(!rolled.rolled_back && !rolled.regen_expected);
    assert_eq!(rolled.reason, "stale");
    assert_eq!(rolled.projection, before);
    assert_eq!(harness.projection().await, before);
    assert!(api::get_operation_record(&harness.state, failed_id())
        .await
        .is_ok());
    legacy_shape(&rolled);
}

async fn needs_repair_is_retained(harness: &Harness, baseline: &DocumentProjection) {
    let operation = unresolved_fillet();
    let id = RecordId(Uuid::from_u128(REPAIR));
    let response = apply(harness, id, operation.clone(), false, profile::sketch_id()).await;
    let terminal = harness.terminal(&response.projection).await;
    assert!(terminal.failed_steps.is_empty(), "{terminal:?}");
    assert!(
        terminal.repair_steps.contains(&id.to_string()),
        "{terminal:?}"
    );
    let current = harness.projection().await;
    assert_eq!(current.undo_depth, baseline.undo_depth + 1);
    assert_eq!(
        api::get_operation_record(&harness.state, id)
            .await
            .unwrap()
            .op,
        operation
    );
    assert_eq!(
        current
            .features
            .iter()
            .find(|feature| feature.id == id.to_string())
            .unwrap()
            .status,
        FeatureStatus::NeedsRepair
    );
    assert!(
        response.rollback_token.is_some(),
        "NeedsRepair keeps its accepted record despite a receipt"
    );
}

fn unresolved_fillet() -> Operation {
    let element = ElementId::new("el_unresolvable");
    let reference = ElementRef {
        primary: Some(PrimaryRef {
            body: profile::extrude_id().to_string().parse().unwrap(),
            element: element.clone(),
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: None,
        extra: Default::default(),
    };
    Operation::Known(KnownOperation::Fillet(FilletParams {
        radius: Scalar::new(1.0),
        edge_ids: vec![element],
        edges: vec![reference],
        chain_tangent_edges: false,
        tangent_closure_version: None,
        extra: Default::default(),
    }))
}
