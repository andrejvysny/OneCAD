//! Staged operands bind to records the shared reducer accepts (ONECAD-316/319).
#[allow(dead_code)]
mod support;

use onecad_app::{
    api::{StagedEdgeOp, StagedOffsetFace},
    dto::{
        EdgeOpEvidenceDto, EdgeOpRefusalDto, OffsetCurrentDimsDto, OffsetFaceEvidenceDto,
        OffsetFaceRefusalDto, PrepareEdgeOpDto, PrepareOffsetFaceDto,
    },
    worker::wire::body_id_wire,
};
use onecad_core::{
    document::{
        record::*,
        refs::{ElementKind, ElementRef, PrimaryRef},
        variables::Scalar,
    },
    edit::EditCommand,
    ids::{BodyId, ElementId, RecordId},
};
use onecad_interaction::{operand_binding::*, *};

fn reference(body: BodyId, kind: ElementKind, id: &str) -> ElementRef {
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

fn edge(key: &str, id: &str, picked: bool) -> EdgeOpEvidenceDto {
    EdgeOpEvidenceDto {
        topo_key: key.into(),
        picked,
        element_id: Some(id.into()),
        body_id: None,
        kind: Some("edge".into()),
        anchor: None,
        descriptor: None,
        contour: Some(0),
        adjacent_faces: None,
    }
}

fn staged_edges(body: BodyId) -> StagedEdgeOp {
    StagedEdgeOp {
        prepared: PrepareEdgeOpDto {
            snapshot_id: 8,
            target_body_id: body_id_wire(body),
            edges: vec![edge("e:4", "el_a", true), edge("e:9", "el_b", false)],
            refusal: None,
        },
        refs: vec![
            reference(body, ElementKind::Edge, "el_a"),
            reference(body, ElementKind::Edge, "el_b"),
        ],
    }
}

fn face(key: &str, picked: bool) -> OffsetFaceEvidenceDto {
    OffsetFaceEvidenceDto {
        topo_key: key.into(),
        picked,
        anchor: None,
        descriptor: None,
    }
}

fn prepared_faces(body: BodyId) -> PrepareOffsetFaceDto {
    PrepareOffsetFaceDto {
        snapshot_id: 8,
        target_body_id: body_id_wire(body),
        faces: vec![face("f:2", true), face("f:5", false)],
        opposite_face: None,
        current_dims: OffsetCurrentDimsDto {
            radius: None,
            thickness: None,
        },
        refusal: None,
    }
}

/// Begins the bound record through the reducer; returns the session's state.
fn begin(record: OperationRecord) -> Interaction {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(support::context())));
    let action = begin_bound(record, 3, support::context()).unwrap();
    let effects = state.dispatch(action);
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::EvaluateExpression { expression, .. } if expression == "1"
        )),
        "{effects:?}"
    );
    assert!(matches!(state.status, CommitStatus::Idle));
    state
}

#[test]
fn fillet_binds_the_full_closure_in_prepared_order_and_begins() {
    let body = BodyId::new();
    let id = RecordId::new();
    let record = bind_edge_op(
        EdgeOpKind::Fillet,
        &staged_edges(body),
        Scalar::new(DEFAULT_OPERATION_SIZE_MM),
        id,
        4,
    )
    .unwrap();
    let Operation::Known(KnownOperation::Fillet(params)) = &record.op else {
        panic!("fillet record expected")
    };
    assert_eq!(
        params.edge_ids,
        [ElementId::new("el_a"), ElementId::new("el_b")]
    );
    assert_eq!(params.edges, staged_edges(body).refs);
    assert_eq!(params.radius, Scalar::new(1.0));
    assert!(params.chain_tangent_edges);
    assert_eq!(params.tangent_closure_version, Some(1));
    assert_eq!((record.record_id, record.step_index), (id, 4));
    assert_eq!(record.inputs, record.op.derive_inputs());
    let state = begin(record);
    let tool = state.tool.unwrap();
    assert_eq!(
        tool.params.focus(),
        Some(NumericParameter::FilletRadius.into())
    );
}

#[test]
fn chamfer_binds_equal_leg_without_reference_faces_and_commits_one_add() {
    let body = BodyId::new();
    let record = bind_edge_op(
        EdgeOpKind::Chamfer,
        &staged_edges(body),
        Scalar::new(1.0),
        RecordId::new(),
        2,
    )
    .unwrap();
    let Operation::Known(KnownOperation::Chamfer(params)) = &record.op else {
        panic!("chamfer record expected")
    };
    assert!(!params.is_asymmetric());
    assert!(params.reference_faces.is_empty() && params.reference_face_refs.is_empty());
    let mut state = begin(record.clone());
    let tag = state.tool.as_ref().unwrap().tag.clone();
    support::evaluate(&mut state, &tag, 1.0);
    state.dispatch(Action::PreviewFinished {
        tag: tag.clone(),
        result: Ok(support::preview(&tag)),
    });
    let command = state
        .dispatch(Action::Confirm)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ApplyOperation { command, .. } => Some(*command),
            _ => None,
        })
        .unwrap();
    let EditCommand::AddOperation {
        record: committed,
        at_cursor: true,
    } = command
    else {
        panic!("a new chamfer appends exactly one operation")
    };
    // The reducer stamps the field's entered text as the expression; nothing else moves.
    let mut expected = record;
    let Operation::Known(KnownOperation::Chamfer(params)) = &mut expected.op else {
        unreachable!()
    };
    params.radius = Scalar::with_expr(1.0, "1");
    assert_eq!(committed, expected);
}

#[test]
fn edge_binding_refuses_refusals_snapshot_keys_and_mismatched_evidence() {
    let body = BodyId::new();
    let bind = |staged: &StagedEdgeOp| {
        bind_edge_op(
            EdgeOpKind::Fillet,
            staged,
            Scalar::new(1.0),
            RecordId::new(),
            1,
        )
    };
    let mut refused = staged_edges(body);
    refused.prepared.refusal = Some(EdgeOpRefusalDto {
        code: "chainMismatch".into(),
        message: "closure grew".into(),
        edges: vec![],
    });
    assert!(bind(&refused).unwrap_err().contains("closure grew"));
    let mut snapshot = staged_edges(body);
    snapshot.refs[1] = reference(body, ElementKind::Edge, "e:9");
    snapshot.prepared.edges[1].element_id = Some("e:9".into());
    assert!(bind(&snapshot).unwrap_err().contains("TopoKey"));
    let mut short = staged_edges(body);
    short.refs.pop();
    assert!(bind(&short).is_err());
    let mut swapped = staged_edges(body);
    swapped.refs.swap(0, 1);
    assert!(bind(&swapped).is_err());
    let mut foreign = staged_edges(body);
    foreign.refs[0] = reference(BodyId::new(), ElementKind::Edge, "el_a");
    assert!(bind(&foreign).is_err());
    let mut face = staged_edges(body);
    face.refs[0] = reference(body, ElementKind::Face, "el_a");
    assert!(bind(&face).is_err());
}

#[test]
fn offset_face_binds_closure_with_picks_as_primary_faces_and_begins() {
    let body = BodyId::new();
    let refs = [
        reference(body, ElementKind::Face, "el_top"),
        reference(body, ElementKind::Face, "el_blend"),
    ];
    let staged = StagedOffsetFace {
        prepared: prepared_faces(body),
        refs: refs.to_vec(),
        surface: None,
    };
    let record = bind_offset_face(&staged, Scalar::new(1.0), RecordId::new(), 5).unwrap();
    let Operation::Known(KnownOperation::OffsetFace(params)) = &record.op else {
        panic!("offset face record expected")
    };
    assert_eq!(
        params.face_ids,
        [ElementId::new("el_top"), ElementId::new("el_blend")]
    );
    assert_eq!(params.primary_face_ids, [ElementId::new("el_top")]);
    assert_eq!(params.target_body, body);
    assert_eq!(params.distance_type, OffsetDistanceType::Offset);
    assert_eq!(
        params.result_policy_version,
        Some(OFFSET_FACE_RESULT_POLICY_VERSION)
    );
    let state = begin(record);
    assert_eq!(
        state.tool.unwrap().params.focus(),
        Some(NumericParameter::OffsetFaceDistance.into())
    );
}

#[test]
fn offset_binding_refuses_refusals_missing_picks_and_short_refs() {
    let body = BodyId::new();
    let refs = [
        reference(body, ElementKind::Face, "el_top"),
        reference(body, ElementKind::Face, "el_blend"),
    ];
    let bind = |prepared: &PrepareOffsetFaceDto, refs: &[ElementRef]| {
        let staged = StagedOffsetFace {
            prepared: prepared.clone(),
            refs: refs.to_vec(),
            surface: None,
        };
        bind_offset_face(&staged, Scalar::new(1.0), RecordId::new(), 1)
    };
    let mut refused = prepared_faces(body);
    refused.refusal = Some(OffsetFaceRefusalDto {
        code: "crossBody".into(),
        message: "two bodies".into(),
        faces: vec![],
    });
    assert!(bind(&refused, &refs).unwrap_err().contains("two bodies"));
    let mut unpicked = prepared_faces(body);
    unpicked.faces[0].picked = false;
    assert!(bind(&unpicked, &refs).is_err());
    assert!(bind(&prepared_faces(body), &refs[..1]).is_err());
    let edges = [
        reference(body, ElementKind::Edge, "el_top"),
        reference(body, ElementKind::Face, "el_blend"),
    ];
    assert!(bind(&prepared_faces(body), &edges).is_err());
    let zero = StagedOffsetFace {
        prepared: prepared_faces(body),
        refs: refs.to_vec(),
        surface: None,
    };
    assert!(
        bind_offset_face(&zero, Scalar::new(0.0), RecordId::new(), 1).is_err(),
        "a zero signed offset is refused by the record contract"
    );
}

#[test]
fn transform_record_declares_exact_translation_and_angle_entry() {
    let body = BodyId::new();
    let record = transform_record(body, [30.0, 20.0, 2.0], RecordId::new(), 3);
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(support::context())));
    let fields = transform_fields();
    let effects = state.dispatch(Action::BeginOperation {
        record: Box::new(record),
        fields: fields.clone(),
        focus: Some(fields[0]),
        gesture: 4,
        context: support::context(),
    });
    let tool = state.tool.as_ref().unwrap();
    assert_eq!(
        tool.params
            .active()
            .map(|field| field.key)
            .collect::<Vec<_>>(),
        fields
    );
    answer(&mut state, &effects, 0.0, "length");
    for (key, value) in [(fields[1], 7.0), (fields[3], 90.0)] {
        let effects = state.dispatch(Action::ParameterChanged {
            key,
            text: value.to_string(),
        });
        answer(&mut state, &effects, value, key.site());
    }
    assert_eq!(
        state.tool.as_ref().unwrap().params.readiness(),
        Readiness::Ready
    );
    let Operation::Known(KnownOperation::TransformBody(params)) =
        state.tool.as_ref().unwrap().effective_op()
    else {
        panic!("transform expected")
    };
    assert_eq!(params.translate[1].value, 7.0);
    assert_eq!(params.rotate.angle_deg.value, 90.0);
    assert_eq!(params.rotate.axis.z, 1.0);
    assert_eq!(params.targets, [body]);
}

/// Answers the one evaluation `effects` requested with `value`.
fn answer(state: &mut Interaction, effects: &[Effect], value: f64, dimension: &str) {
    let tag = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::EvaluateExpression { tag, .. } => Some(tag.clone()),
            _ => None,
        })
        .expect("an evaluation request");
    let effects = state.dispatch(Action::Evaluated {
        tag,
        result: Ok(EvaluatedExpressionDto {
            value,
            dimension: dimension.into(),
            error: None,
        }),
    });
    for effect in effects {
        if let Effect::Preview { tag, .. } = effect {
            state.dispatch(Action::PreviewFinished {
                result: Ok(support::preview(&tag)),
                tag,
            });
        }
    }
}
