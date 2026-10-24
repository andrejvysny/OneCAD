#[allow(dead_code)]
#[path = "../../onecad-app/tests/native_plate_support/host.rs"]
mod host;
#[allow(dead_code)]
#[path = "../../onecad-app/tests/native_plate_support/profile.rs"]
mod profile;
#[allow(dead_code)]
mod support;

use host::Harness;
use onecad_app::{api, dto::DocumentProjection};
use onecad_core::{
    document::{record::*, variables::Scalar},
    edit::EditCommand,
    ids::{BodyId, RecordId, SnapshotId},
    math::Vec3,
    regen::{GeometryEngine, Lod},
};
use onecad_interaction::*;
use onecad_scene::{MeshLimits, MeshView};
use std::path::{Path, PathBuf};

fn record(body: BodyId) -> OperationRecord {
    OperationRecord::new(
        RecordId::new(),
        2,
        "Place body",
        Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
            targets: vec![body],
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: TransformRotation {
                center: Vec3::new_unchecked(-25.0, 40.0, 3.0),
                ..Default::default()
            },
            copy: false,
            extra: Default::default(),
        })),
    )
}

fn begin(record: OperationRecord, context: DocumentContext) -> Interaction {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context.clone())));
    state.dispatch(Action::BeginOperation {
        record: Box::new(record),
        fields: vec![NumericParameter::TransformTranslation(ParameterAxis::X).into()],
        focus: Some(NumericParameter::TransformTranslation(ParameterAxis::X).into()),
        gesture: 12,
        context,
    });
    state
}

fn draft(translate: [f64; 3], axis: [f64; 3], angle_degrees: f64) -> Action {
    Action::TransformDraftChanged {
        translate,
        axis,
        angle_degrees,
    }
}

#[test]
fn gizmo_replaces_placement_and_fences_previous_preview() {
    let before = record(BodyId::new());
    let mut state = begin(before.clone(), support::context());
    let previous = state.tool.as_ref().unwrap().tag.clone();
    let effects = state.dispatch(draft([10.0, -4.0, 2.0], [0.0, 0.0, 1.0], 90.0));
    let tag = state.tool.as_ref().unwrap().tag.clone();
    assert!(tag.sequence > previous.sequence);
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::Preview { tag: submitted, .. } if submitted == &tag)
    ));
    let updated = &state.tool.as_ref().unwrap().record;
    assert_eq!(updated.inputs, before.inputs);
    let Operation::Known(KnownOperation::TransformBody(params)) = &updated.op else {
        panic!()
    };
    assert_eq!(params.targets, before.inputs.bodies);
    assert_eq!(params.rotate.center, Vec3::new_unchecked(-25.0, 40.0, 3.0));
    assert!(!params.copy);
    assert_eq!(
        params.translate.each_ref().map(|s| s.value),
        [10.0, -4.0, 2.0]
    );
    assert_eq!(params.rotate.angle_deg.value, 90.0);
    assert!(support::evaluate(&mut state, &previous, 0.0).is_empty());
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::EvaluateExpression { .. })));
    state.dispatch(draft([2.0, 0.0, 0.0], [0.0, 0.0, 1.0], 0.0));
    let Operation::Known(KnownOperation::TransformBody(params)) =
        &state.tool.as_ref().unwrap().record.op
    else {
        panic!()
    };
    assert_eq!(params.translate[0].value, 2.0);
}

#[test]
fn invalid_gizmo_clears_candidate_without_changing_frozen_record() {
    for action in [
        draft([f64::NAN, 0.0, 0.0], [0.0, 0.0, 1.0], 0.0),
        draft([0.0; 3], [0.0, f64::INFINITY, 1.0], 0.0),
        draft([0.0; 3], [0.0; 3], 15.0),
        draft([0.0; 3], [0.0, 0.0, 1.0], f64::INFINITY),
    ] {
        let before = record(BodyId::new());
        let mut state = begin(before.clone(), support::context());
        assert!(matches!(
            state.dispatch(action).as_slice(),
            [Effect::ClearPreview { .. }]
        ));
        assert_eq!(state.tool.as_ref().unwrap().record, before);
        assert!(matches!(
            state.tool.as_ref().unwrap().draft_status,
            DraftStatus::Invalid(_)
        ));
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
}

#[test]
fn gizmo_refuses_nontransform_stale_and_committing_sessions() {
    let (mut other, _) = support::started();
    let before = other.tool.as_ref().unwrap().record.clone();
    assert!(other
        .dispatch(draft([1.0; 3], [0.0, 0.0, 1.0], 0.0))
        .is_empty());
    assert_eq!(other.tool.as_ref().unwrap().record, before);
    let mut state = begin(record(BodyId::new()), support::context());
    state.context.as_mut().unwrap().revision += 1;
    assert!(state
        .dispatch(draft([1.0; 3], [0.0, 0.0, 1.0], 0.0))
        .is_empty());
    state.context = Some(support::context());
    state.tool.as_mut().unwrap().phase = SessionPhase::Committing;
    assert!(state
        .dispatch(draft([1.0; 3], [0.0, 0.0, 1.0], 0.0))
        .is_empty());
}

#[test]
fn transform_creation_refuses_malformed_targets_and_input_mismatch() {
    let mut candidate = record(BodyId::new());
    candidate.inputs.bodies.clear();
    assert!(begin(candidate, support::context()).tool.is_none());
    for targets in [Vec::new(), vec![BodyId::new(); 2]] {
        let mut candidate = record(BodyId::new());
        let Operation::Known(KnownOperation::TransformBody(params)) = &mut candidate.op else {
            panic!()
        };
        params.targets = targets;
        candidate.inputs = candidate.op.derive_inputs();
        assert!(begin(candidate, support::context()).tool.is_none());
    }
}

fn expression_record() -> OperationRecord {
    let mut candidate = record(BodyId::new());
    let Operation::Known(KnownOperation::TransformBody(params)) = &mut candidate.op else {
        panic!()
    };
    params.translate = [
        Scalar::with_expr(0.0, "2 - 2"),
        Scalar::with_expr(0.0, "3 - 3"),
        Scalar::with_expr(0.0, "4 - 4"),
    ];
    params.rotate.angle_deg = Scalar::with_expr(0.0, "5 - 5");
    candidate
}

#[test]
fn zero_change_gizmo_preserves_expression_record_and_sequence() {
    let candidate = expression_record();
    let mut state = begin(candidate.clone(), support::context());
    let tag = state.tool.as_ref().unwrap().tag.clone();
    assert!(state
        .dispatch(draft([0.0; 3], [0.0, 0.0, 1.0], 0.0))
        .is_empty());
    assert_eq!(state.tool.as_ref().unwrap().record, candidate);
    assert_eq!(state.tool.as_ref().unwrap().tag, tag);
}

#[test]
fn changed_gizmo_component_keeps_untouched_expression_bindings() {
    let mut state = begin(expression_record(), support::context());
    let effects = state.dispatch(draft([0.0, 7.0, 0.0], [0.0, 0.0, 1.0], 0.0));
    let tag = support::evaluation_tag(&effects);
    assert!(effects.iter().any(|effect| matches!(
        effect, Effect::EvaluateExpression { expression, .. } if expression == "2 - 2"
    )));
    let effects = support::evaluate(&mut state, &tag, 0.0);
    let [Effect::Preview { operation, .. }] = effects.as_slice() else {
        panic!()
    };
    let Operation::Known(KnownOperation::TransformBody(params)) = operation.as_ref() else {
        panic!()
    };
    assert_eq!(params.translate[0].expr.as_deref(), Some("2 - 2"));
    assert_eq!(params.translate[1], Scalar::new(7.0));
    assert_eq!(params.translate[2].expr.as_deref(), Some("4 - 4"));
    assert_eq!(params.rotate.angle_deg.expr.as_deref(), Some("5 - 5"));
}

#[test]
fn real_transform_preview_commit_cancel_and_history() {
    host::required_worker();
    if std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE").as_deref() == Ok("transform") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(workflow(&root));
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("onecad-transform-")
        .tempdir()
        .unwrap();
    host::run_isolated_phase(
        root.path(),
        "transform",
        "real_transform_preview_commit_cancel_and_history",
    );
}

async fn workflow(root: &Path) {
    let harness = Harness::new(&root.join("transform-host")).await;
    let (before, body, mut state) = prepared_plate(&harness).await;
    let effects = state.dispatch(draft([10.0, -4.0, 2.0], [0.0, 0.0, 1.0], 90.0));
    let (tag, operation) = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Preview { tag, operation, .. } => Some((tag.clone(), operation)),
            _ => None,
        })
        .expect("canonical placement previews without expression evaluation");
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::EvaluateExpression { .. })));
    let preview = checked_preview(&harness, &before, operation, &tag).await;
    assert_bounds(
        &preview.bodies[0].mesh,
        [-55.0, 11.0, 2.0],
        [25.0, 61.0, 8.0],
    );
    state.dispatch(Action::PreviewFinished {
        tag: tag.clone(),
        result: Ok(preview),
    });
    let mut cancelled = begin(record(body), state.context.clone().unwrap());
    cancelled.dispatch(draft([5.0; 3], [0.0, 0.0, 1.0], 0.0));
    cancelled.dispatch(Action::Cancel);
    assert!(cancelled.tool.is_none());
    assert_eq!(harness.projection().await, before);
    commit_and_history(&harness, state, &before, body, operation).await;
    harness.close().await;
}

async fn prepared_plate(harness: &Harness) -> (DocumentProjection, BodyId, Interaction) {
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(
        harness
            .state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let region = profile::create(harness).await;
    let extrude = OperationRecord::new(
        profile::extrude_id(),
        1,
        "Plate",
        profile::extrude(&region, 6.0),
    );
    let before = apply(harness, extrude).await;
    let context = api::preview_context(&harness.state).await.unwrap();
    let body: BodyId = before.bodies.keys().next().unwrap().parse().unwrap();
    let state = begin(
        record(body),
        DocumentContext {
            document_id: context.document_id.clone(),
            runtime_session: context.runtime_session.clone(),
            revision: context.revision.0,
            snapshot_id: context.snapshot_id,
            scene: support::context().scene,
        },
    );
    (before, body, state)
}

async fn apply(harness: &Harness, record: OperationRecord) -> DocumentProjection {
    let result = api::apply_operation(
        &harness.state,
        harness.host.clone(),
        vec![EditCommand::AddOperation {
            record,
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    harness.settled(&result.projection).await
}

async fn checked_preview(
    harness: &Harness,
    before: &DocumentProjection,
    operation: &Operation,
    tag: &RequestTag,
) -> onecad_app::dto::PreviewResultDto {
    let worker = harness.state.live_worker().unwrap();
    let head = worker.get_worker_head().await.unwrap();
    let context = api::preview_context(&harness.state).await.unwrap();
    let preview = api::preview_operation(
        &harness.state,
        operation.clone(),
        tag.record_id,
        None,
        Some(tag.context.snapshot_id),
        Lod::Fine,
    )
    .await
    .unwrap();
    assert!(preview.needs_repair.is_empty());
    assert_eq!(preview.bodies.len(), 1);
    assert_eq!(harness.projection().await, *before);
    assert_eq!(api::preview_context(&harness.state).await.unwrap(), context);
    assert_eq!(worker.get_worker_head().await.unwrap(), head);
    assert!(api::preview_operation(
        &harness.state,
        operation.clone(),
        tag.record_id,
        None,
        Some(SnapshotId(tag.context.snapshot_id.0 + 1)),
        Lod::Fine
    )
    .await
    .is_err());
    assert_eq!(worker.get_worker_head().await.unwrap(), head);
    assert_eq!(harness.projection().await, *before);
    preview
}

async fn commit_and_history(
    harness: &Harness,
    mut state: Interaction,
    before: &DocumentProjection,
    body: BodyId,
    preview_operation: &Operation,
) {
    let candidate = state.tool.as_ref().unwrap().record.clone();
    let effects = state.dispatch(Action::Confirm);
    let command = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ApplyOperation { command, .. } => Some(*command),
            _ => None,
        })
        .unwrap();
    let response = api::apply_operation(&harness.state, harness.host.clone(), vec![command], None)
        .await
        .unwrap();
    let moved = harness.settled(&response.projection).await;
    assert_eq!(moved.undo_depth, before.undo_depth + 1);
    assert_eq!(
        moved.bodies.keys().collect::<Vec<_>>(),
        before.bodies.keys().collect::<Vec<_>>()
    );
    let stored = api::get_operation_record(&harness.state, candidate.record_id)
        .await
        .unwrap();
    assert_eq!(&stored.op, preview_operation);
    check_mass_and_mesh(harness, &moved, body, true).await;
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    let restored = harness.settled(&undo).await;
    check_mass_and_mesh(harness, &restored, body, false).await;
    let redo = api::redo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    let repeated = harness.settled(&redo).await;
    check_mass_and_mesh(harness, &repeated, body, true).await;
}

async fn check_mass_and_mesh(
    harness: &Harness,
    projection: &DocumentProjection,
    body: BodyId,
    moved: bool,
) {
    let wire = onecad_app::worker::wire::body_id_wire(body);
    let mass = api::query_mass_properties(&harness.state, wire, None)
        .await
        .unwrap();
    assert!((mass.volume - 24000.0).abs() < 1e-6);
    let centroid = if moved {
        [-15.0, 36.0, 5.0]
    } else {
        [-25.0, 40.0, 3.0]
    };
    for (actual, expected) in mass.centroid.into_iter().zip(centroid) {
        assert!((actual - expected).abs() < 1e-6);
    }
    let mesh = api::get_mesh(
        &harness.state,
        body.to_string(),
        "fine".into(),
        None,
        Some(projection.runtime_session.clone()),
    )
    .await
    .unwrap();
    if moved {
        assert_bounds(&mesh, [-55.0, 11.0, 2.0], [25.0, 61.0, 8.0]);
    } else {
        assert_bounds(&mesh, [-50.0, 0.0, 0.0], [0.0, 80.0, 6.0]);
    }
}

fn assert_bounds(bytes: &[u8], expected_min: [f64; 3], expected_max: [f64; 3]) {
    let mesh = MeshView::decode(bytes, MeshLimits::default()).unwrap();
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for i in 0..mesh.header().vertex_count as usize {
        let p = mesh.position(i).unwrap();
        for axis in 0..3 {
            min[axis] = min[axis].min(f64::from(p[axis]));
            max[axis] = max[axis].max(f64::from(p[axis]));
        }
    }
    for axis in 0..3 {
        assert!((min[axis] - expected_min[axis]).abs() < 1e-5);
        assert!((max[axis] - expected_max[axis]).abs() < 1e-5);
    }
}
