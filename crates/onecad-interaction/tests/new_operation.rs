mod parameter_support;
#[allow(dead_code)]
mod support;
use onecad_core::{
    document::{record::*, refs::AxisRef},
    edit::EditCommand,
    ids::{BodyId, ElementId, EntityId},
};
use onecad_interaction::*;
use parameter_support::*;
use support::{context, preview};

fn creations() -> Vec<(OperationRecord, NumericParameter)> {
    operations()
        .into_iter()
        .filter_map(|mut op| {
            match &mut op {
                KnownOperation::LinearPattern(_)
                | KnownOperation::CircularPattern(_)
                | KnownOperation::FeaturePattern(_)
                | KnownOperation::TransformBody(_) => return None,
                KnownOperation::Chamfer(p) => {
                    p.distance2 = None;
                    p.angle_deg = None;
                }
                KnownOperation::Revolve(p) => {
                    p.axis = Some(AxisRef::SketchLine {
                        sketch: p.profile.as_ref().unwrap().sketch,
                        line: EntityId::new(),
                        extra: Default::default(),
                    })
                }
                _ => {}
            }
            // The host's candidate already carries exact canonical expressions.
            for (_, dimension, scalar) in op.scalars_mut() {
                scalar.expr = Some(format!(
                    "{}{}",
                    scalar.value,
                    if dimension == onecad_core::expr::Dimension::Angle {
                        "deg"
                    } else {
                        "mm"
                    }
                ));
            }
            let record = record(op);
            let parameter = numeric_parameters(&record.op).first().unwrap().parameter;
            Some((record, parameter))
        })
        .collect()
}
fn begin_new(
    state: &mut Interaction,
    record: OperationRecord,
    parameter: NumericParameter,
) -> Vec<Effect> {
    state.dispatch(Action::BeginOperation {
        record: Box::new(record),
        fields: vec![parameter.into()],
        focus: Some(parameter.into()),
        gesture: 100,
        context: context(),
    })
}
fn new_state(record: OperationRecord, parameter: NumericParameter) -> (Interaction, RequestTag) {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let tag = begin_new(&mut state, record, parameter)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::EvaluateExpression { tag, .. } => Some(tag),
            _ => None,
        })
        .unwrap();
    (state, tag)
}
fn profile(record: &OperationRecord) -> Option<onecad_core::ids::SketchId> {
    match &record.op {
        Operation::Known(KnownOperation::Extrude(p)) => p.profile.as_ref().map(|p| p.sketch),
        Operation::Known(KnownOperation::Revolve(p)) => p.profile.as_ref().map(|p| p.sketch),
        _ => None,
    }
}
#[test]
fn real_preview_intent_and_single_add_keep_exact_typed_operation_targets_and_id() {
    for (before, parameter) in creations() {
        let scalar = numeric_parameters(&before.op)
            .into_iter()
            .find(|d| d.parameter == parameter)
            .unwrap()
            .scalar;
        let (mut state, tag) = new_state(before.clone(), parameter);
        assert!(state.dispatch(Action::Confirm).is_empty());
        let effects = evaluate(&mut state, tag.clone(), parameter.site(), scalar.value);
        let [Effect::Preview {
            operation,
            sketch_id,
            expected_snapshot,
            ..
        }] = effects.as_slice()
        else {
            panic!("missing real preview intent")
        };
        assert_eq!(operation.as_ref(), &before.op);
        assert_eq!(*sketch_id, profile(&before));
        assert_eq!(*expected_snapshot, tag.context.snapshot_id);
        assert!(state.dispatch(Action::Confirm).is_empty());
        state.dispatch(Action::PreviewFinished {
            tag: tag.clone(),
            result: Ok(preview(&tag)),
        });
        let effects = state.dispatch(Action::Confirm);
        let command = effects
            .iter()
            .find_map(|e| match e {
                Effect::ApplyOperation {
                    command,
                    hide_sketch,
                    ..
                } => {
                    assert_eq!(*hide_sketch, profile(&before));
                    Some(command)
                }
                _ => None,
            })
            .unwrap();
        let EditCommand::AddOperation {
            record,
            at_cursor: true,
        } = command.as_ref()
        else {
            panic!("must append exactly one new operation")
        };
        assert_eq!(record, &before);
        assert!(state.dispatch(Action::Confirm).is_empty());
        assert!(begin_new(&mut state, before, parameter).is_empty());
        assert_eq!(state.tool.unwrap().phase, SessionPhase::Committing);
    }
}
#[test]
fn failed_stale_or_repair_preview_never_writes_operation() {
    for failure in 0..3 {
        let (before, parameter) = creations().remove(0);
        let scalar = numeric_parameters(&before.op)[0].scalar.value;
        let (mut state, tag) = new_state(before.clone(), parameter);
        evaluate(&mut state, tag.clone(), parameter.site(), scalar);
        let result = match failure {
            0 => Err("Kernel rejected requested size".into()),
            1 => {
                let mut result = preview(&tag);
                result.snapshot_id += 1;
                Ok(result)
            }
            _ => {
                let mut result = preview(&tag);
                result.needs_repair.push("Unresolved promoted edge".into());
                Ok(result)
            }
        };
        state.dispatch(Action::PreviewFinished { tag, result });
        assert_ne!(state.tool.as_ref().unwrap().preview, PreviewStatus::Ready);
        assert_eq!(state.tool.as_ref().unwrap().record, before);
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
}
#[test]
fn missing_profile_axis_targets_and_promoted_inputs_refuse_before_evaluation() {
    let cases = invalid_candidates();
    for (record, parameter) in cases {
        let mut state = Interaction::default();
        state.dispatch(Action::ContextChanged(Some(context())));
        assert!(begin_new(&mut state, record, parameter).is_empty());
        assert!(matches!(state.status, CommitStatus::Rejected(_)));
        assert!(state.tool.is_none());
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
}
fn invalid_candidates() -> Vec<(OperationRecord, NumericParameter)> {
    let mut out = Vec::new();
    for (before, parameter) in creations() {
        let mut invalid = before;
        match &mut invalid.op {
            Operation::Known(KnownOperation::Extrude(p)) => p.profile = None,
            Operation::Known(KnownOperation::Revolve(p)) => p.axis = None,
            Operation::Known(KnownOperation::Fillet(p)) => p.edges.clear(),
            Operation::Known(KnownOperation::Chamfer(p)) => {
                p.edges[0].primary.as_mut().unwrap().body = BodyId::new()
            }
            Operation::Known(KnownOperation::Shell(p)) => p.target_body = None,
            Operation::Known(KnownOperation::Hole(p)) => p.face.primary = None,
            Operation::Known(KnownOperation::OffsetFace(p)) => p.faces.clear(),
            _ => continue,
        }
        // A single-edge body is its own explicit target; exercise the kind gate instead.
        if let Operation::Known(KnownOperation::Chamfer(p)) = &mut invalid.op {
            p.edges[0].primary.as_mut().unwrap().kind =
                onecad_core::document::refs::ElementKind::Face;
        }
        out.push((invalid, parameter));
    }
    out
}
#[test]
fn snapshot_topology_keys_are_not_promoted_persistent_references() {
    let mut op = edge_operations(BodyId::new()).remove(0);
    let KnownOperation::Fillet(p) = &mut op else {
        panic!()
    };
    p.edge_ids[0] = ElementId::new("e:7");
    p.edges[0].primary.as_mut().unwrap().element = ElementId::new("e:7");
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    begin_new(&mut state, record(op), NumericParameter::FilletRadius);
    assert!(matches!(state.status, CommitStatus::Rejected(_)));
    assert!(state.tool.is_none());
}
#[test]
fn unqualified_pattern_and_expansion_preview_are_explicitly_unavailable() {
    for op in operations().into_iter().filter(|op| {
        matches!(
            op,
            KnownOperation::LinearPattern(_)
                | KnownOperation::CircularPattern(_)
                | KnownOperation::FeaturePattern(_)
        )
    }) {
        let before = record(op);
        let parameter = numeric_parameters(&before.op)[0].parameter;
        let mut state = Interaction::default();
        state.dispatch(Action::ContextChanged(Some(context())));
        begin_new(&mut state, before, parameter);
        assert!(matches!(state.status, CommitStatus::Rejected(_)));
        assert!(state.tool.is_none());
    }
}
#[test]
fn stale_context_cancellation_and_old_preview_result_do_not_restart_new_feature() {
    let (before, parameter) = creations().remove(0);
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let mut stale = context();
    stale.snapshot_id.0 -= 1;
    assert!(state
        .dispatch(Action::BeginOperation {
            record: Box::new(before.clone()),
            fields: vec![parameter.into()],
            focus: Some(parameter.into()),
            gesture: 100,
            context: stale
        })
        .is_empty());
    assert!(state.tool.is_none());
    let tag = begin_new(&mut state, before, parameter)
        .into_iter()
        .find_map(|e| match e {
            Effect::EvaluateExpression { tag, .. } => Some(tag),
            _ => None,
        })
        .unwrap();
    let value = state.tool.as_ref().unwrap().record.clone();
    let scalar = numeric_parameters(&value.op)[0].scalar.value;
    evaluate(&mut state, tag.clone(), parameter.site(), scalar);
    state.dispatch(Action::Cancel);
    assert!(state
        .dispatch(Action::PreviewFinished {
            tag: tag.clone(),
            result: Ok(preview(&tag))
        })
        .is_empty());
    assert!(state.tool.is_none());
    assert!(state.dispatch(Action::Confirm).is_empty());
}

fn asymmetric_chamfer() -> OperationRecord {
    let body = BodyId::new();
    let mut op = edge_operations(body).remove(1);
    let KnownOperation::Chamfer(params) = &mut op else {
        panic!()
    };
    let mut face = primary(
        body,
        onecad_core::document::refs::ElementKind::Face,
        "el_reference_face",
    );
    face.anchor = Some(onecad_core::document::refs::AnchorIntent {
        world_point: onecad_core::math::Vec3::new_unchecked(1.0, 2.0, 3.0),
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    });
    params.reference_faces = vec![ChamferReferenceFace {
        edge_id: params.edge_ids[0].clone(),
        face_id: ElementId::new("el_reference_face"),
    }];
    params.reference_face_refs = vec![face];
    record(op)
}
#[test]
fn new_asymmetric_chamfer_requires_complete_matching_promoted_reference_faces() {
    let valid = asymmetric_chamfer();
    let (state, _) = new_state(valid.clone(), NumericParameter::ChamferDistance);
    assert!(state.tool.is_some());
    for failure in 0..4 {
        let mut candidate = valid.clone();
        let Operation::Known(KnownOperation::Chamfer(p)) = &mut candidate.op else {
            panic!()
        };
        match failure {
            0 => p.reference_face_refs.clear(),
            1 => {
                p.reference_face_refs[0].primary.as_mut().unwrap().element =
                    ElementId::new("el_wrong_face")
            }
            2 => p.reference_face_refs[0].primary.as_mut().unwrap().body = BodyId::new(),
            _ => p.reference_face_refs.push(p.reference_face_refs[0].clone()),
        }
        let mut state = Interaction::default();
        state.dispatch(Action::ContextChanged(Some(context())));
        begin_new(&mut state, candidate, NumericParameter::ChamferDistance);
        assert!(matches!(state.status, CommitStatus::Rejected(_)));
        assert!(state.tool.is_none());
    }
}
#[test]
fn equal_leg_chamfer_refuses_inactive_reference_face_metadata() {
    let mut candidate = asymmetric_chamfer();
    let Operation::Known(KnownOperation::Chamfer(p)) = &mut candidate.op else {
        panic!()
    };
    p.distance2 = None;
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    begin_new(&mut state, candidate, NumericParameter::ChamferDistance);
    assert!(matches!(state.status, CommitStatus::Rejected(_)));
    assert!(state.tool.is_none());
}
