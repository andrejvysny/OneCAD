#![allow(dead_code)]
use crate::support;
use onecad_core::{
    document::{
        record::*,
        refs::{ElementKind, ElementRef, PrimaryRef},
        variables::Scalar,
    },
    edit::EditCommand,
    ids::{BodyId, ElementId, RecordId},
    math::Vec3,
};
use onecad_interaction::*;
use support::{completion, context};
pub fn record(op: KnownOperation) -> OperationRecord {
    OperationRecord::new(RecordId::new(), 7, "Preserve feature", Operation::Known(op))
}
pub fn primary(body: BodyId, kind: ElementKind, id: &str) -> ElementRef {
    ElementRef {
        primary: Some(PrimaryRef {
            body,
            kind,
            element: ElementId::new(id),
            extra: Default::default(),
        }),
        intent: None,
        anchor: None,
        extra: Default::default(),
    }
}
pub fn edge_operations(body: BodyId) -> Vec<KnownOperation> {
    let edges = vec![primary(body, ElementKind::Edge, "el_edge")];
    vec![
        KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(2.0),
            edge_ids: vec![ElementId::new("el_edge")],
            edges: edges.clone(),
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        }),
        KnownOperation::Chamfer(ChamferParams {
            radius: Scalar::new(2.0),
            distance2: Some(Scalar::new(3.0)),
            angle_deg: None,
            edge_ids: vec![ElementId::new("el_edge")],
            edges,
            reference_faces: vec![],
            reference_face_refs: vec![],
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        }),
        KnownOperation::Shell(ShellParams {
            thickness: Scalar::new(2.0),
            open_faces: vec![ElementId::new("el_face")],
            faces: vec![primary(body, ElementKind::Face, "el_face")],
            target_body: Some(body),
            extra: Default::default(),
        }),
    ]
}
pub fn hole(body: BodyId) -> HoleParams {
    HoleParams {
        target_body: body,
        face: primary(body, ElementKind::Face, "el_face"),
        point: Vec3::new_unchecked(3.0, 4.0, 5.0),
        hole_type: HoleType::Simple,
        diameter: Scalar::new(5.0),
        depth: Some(Scalar::new(10.0)),
        cb_diameter: None,
        cb_depth: None,
        cs_diameter: None,
        cs_angle_deg: None,
        thread: Some(HoleThread {
            standard: HoleThreadStandard::Iso261,
            designation: "M6x1".into(),
            major_diameter_mm: 6.0,
            pitch_mm: Scalar::new(1.0),
            depth_mm: Some(Scalar::new(8.0)),
            detail: HoleThreadDetail::Cosmetic,
        }),
        result_policy_version: Some(HOLE_RESULT_POLICY_VERSION),
        extra: Default::default(),
    }
}
pub fn pattern_operations(body: BodyId) -> Vec<KnownOperation> {
    let direction = Vec3::new_unchecked(0.0, 1.0, 0.0);
    vec![
        KnownOperation::LinearPattern(LinearPatternParams {
            source_body: Some(body),
            direction,
            spacing: Scalar::new(8.0),
            count: 17,
            fuse_result: false,
            result_policy_version: Some(2),
            extra: Default::default(),
        }),
        KnownOperation::CircularPattern(CircularPatternParams {
            source_body: Some(body),
            axis_origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            axis_direction: direction,
            angle_deg: Scalar::new(120.0),
            count: 19,
            fuse_result: false,
            result_policy_version: Some(2),
            extra: Default::default(),
        }),
        KnownOperation::FeaturePattern(FeaturePatternParams {
            source_record_ids: vec![RecordId::new()],
            layout: FeaturePatternLayout::Linear {
                direction,
                spacing: Scalar::new(8.0),
            },
            count: 3,
            semantics_version: 1,
            extra: Default::default(),
        }),
        KnownOperation::TransformBody(TransformBodyParams {
            targets: vec![body],
            translate: [Scalar::new(2.0), Scalar::new(-3.0), Scalar::new(4.0)],
            rotate: TransformRotation::default(),
            copy: true,
            extra: Default::default(),
        }),
    ]
}
pub fn operations() -> Vec<KnownOperation> {
    let body = BodyId::new();
    let mut out = edge_operations(body);
    let mut angled = out[1].clone();
    if let KnownOperation::Chamfer(p) = &mut angled {
        p.distance2 = None;
        p.angle_deg = Some(Scalar::new(45.0));
    }
    out.push(angled);
    out.extend(hole_operations(body));
    out.extend(pattern_operations(body));
    out.push(KnownOperation::Revolve(RevolveParams {
        profile: Some(support::selection(context()).profile),
        angle_deg: Scalar::new(120.0),
        axis: None,
        boolean_mode: BooleanMode::Cut,
        target_body: Some(body),
        extra: Default::default(),
    }));
    out.push(KnownOperation::OffsetFace(OffsetFaceParams {
        face_ids: vec![ElementId::new("el_face")],
        primary_face_ids: vec![],
        faces: vec![primary(body, ElementKind::Face, "el_face")],
        distance: Scalar::new(3.0),
        distance_type: OffsetDistanceType::Offset,
        chain_tangent_faces: false,
        opposite_face_id: None,
        opposite_face: None,
        target_body: body,
        result_policy_version: None,
        extra: Default::default(),
    }));
    out.push(circular_feature());
    let (state, _) = support::started();
    let Operation::Known(extrude) = state.tool.unwrap().record.op else {
        panic!()
    };
    out.push(extrude);
    out
}
pub fn begin(record: OperationRecord, parameter: NumericParameter) -> (Interaction, RequestTag) {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let effects = state.dispatch(Action::BeginParameterEdit {
        record: Box::new(record),
        parameter,
        gesture: 42,
        context: context(),
    });
    let tag = effects
        .into_iter()
        .find_map(|e| match e {
            Effect::EvaluateExpression { tag, site, .. } => {
                assert_eq!(site, parameter.site());
                Some(tag)
            }
            _ => None,
        })
        .unwrap();
    (state, tag)
}
pub fn evaluate(
    state: &mut Interaction,
    tag: RequestTag,
    dimension: &str,
    value: f64,
) -> Vec<Effect> {
    state.dispatch(Action::Evaluated {
        tag,
        result: Ok(EvaluatedExpressionDto {
            value,
            dimension: dimension.into(),
            error: None,
        }),
    })
}
pub fn edit(record: OperationRecord, parameter: NumericParameter, value: f64) -> OperationRecord {
    let expected_id = record.record_id;
    let (mut state, _) = begin(record, parameter);
    let effects = state.dispatch(Action::DraftChanged(format!("  {value}  ")));
    let tag = effects
        .into_iter()
        .find_map(|e| match e {
            Effect::EvaluateExpression { tag, .. } => Some(tag),
            _ => None,
        })
        .unwrap();
    assert!(evaluate(&mut state, tag, parameter.site(), value).is_empty());
    assert_eq!(
        state.tool.as_ref().unwrap().preview,
        PreviewStatus::UnavailableForEdit
    );
    let effects = state.dispatch(Action::Confirm);
    assert!(effects.iter().any(
        |e| matches!(e,Effect::ApplyOperation {command,hide_sketch:None,..}
        if matches!(**command,EditCommand::UpdateOperationParams {record,..} if record == expected_id))
    ));
    assert!(state.dispatch(Action::Confirm).is_empty());
    state.tool.unwrap().record
}

fn hole_operations(body: BodyId) -> Vec<KnownOperation> {
    let mut out = Vec::new();
    let base = hole(body);
    out.push(KnownOperation::Hole(base.clone()));
    let mut cb = base.clone();
    cb.hole_type = HoleType::Counterbore;
    cb.cb_diameter = Some(Scalar::new(9.0));
    cb.cb_depth = Some(Scalar::new(2.0));
    out.push(KnownOperation::Hole(cb));
    let mut cs = base;
    cs.hole_type = HoleType::Countersink;
    cs.cs_diameter = Some(Scalar::new(9.0));
    cs.cs_angle_deg = Some(Scalar::new(90.0));
    out.push(KnownOperation::Hole(cs));
    out
}

fn circular_feature() -> KnownOperation {
    KnownOperation::FeaturePattern(FeaturePatternParams {
        source_record_ids: vec![RecordId::new()],
        layout: FeaturePatternLayout::Circular {
            axis_origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            axis_direction: Vec3::new_unchecked(0.0, 0.0, 1.0),
            angle_deg: Scalar::new(120.0),
        },
        count: 3,
        semantics_version: 1,
        extra: Default::default(),
    })
}

pub fn accepted_filleted_edit(defer: bool) -> (Interaction, RequestTag, RegenFinished) {
    let op = operations()
        .into_iter()
        .find(|op| matches!(op, KnownOperation::Fillet(_)))
        .unwrap();
    let (mut state, tag) = begin(record(op), NumericParameter::FilletRadius);
    evaluate(&mut state, tag.clone(), "length", 4.0);
    assert_eq!(
        state
            .dispatch(Action::Confirm)
            .iter()
            .filter(|e| matches!(e, Effect::ApplyOperation { .. }))
            .count(),
        1
    );
    assert!(state.dispatch(Action::Confirm).is_empty());
    let mut done = completion(&tag, 5);
    done.outcome = "failed".into();
    done.message = Some("Kernel radius refusal".into());
    if defer {
        state.dispatch(Action::RegenFinished {
            tag: tag.clone(),
            completion: Box::new(done.clone()),
        });
    }
    state.dispatch(Action::CommitAccepted {
        tag: tag.clone(),
        revision: 5,
        rollback_token: Some("rollback-1".into()),
    });
    (state, tag, done)
}
