//! Parameter-set reducer: several scalars share one correlated evaluation slot,
//! untouched fields stay verbatim, and drags restore reducer-owned originals.
#[allow(dead_code)]
mod support;
use onecad_core::{
    document::{
        record::{KnownOperation, Operation, OperationRecord, TransformBodyParams},
        variables::Scalar,
    },
    edit::EditCommand,
    ids::{BodyId, RecordId},
};
use onecad_interaction::*;
use support::*;

const DISTANCE: ParameterKey = ParameterKey::Scalar(NumericParameter::ExtrudeDistance);
const DRAFT: ParameterKey = ParameterKey::Scalar(NumericParameter::ExtrudeDraftAngle);
const TX: ParameterKey =
    ParameterKey::Scalar(NumericParameter::TransformTranslation(ParameterAxis::X));
const TY: ParameterKey =
    ParameterKey::Scalar(NumericParameter::TransformTranslation(ParameterAxis::Y));

fn extrude_record() -> OperationRecord {
    let (state, _) = started();
    state.tool.unwrap().record
}

fn begin(record: OperationRecord, fields: Vec<ParameterKey>, existing: bool) -> Interaction {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let focus = fields.first().copied();
    if existing {
        state.dispatch(Action::BeginParameterEdit {
            record: Box::new(record),
            parameter: focus.unwrap().parameter(),
            gesture: 3,
            context: context(),
        });
    } else {
        state.dispatch(Action::BeginOperation {
            record: Box::new(record),
            fields,
            focus,
            gesture: 3,
            context: context(),
        });
    }
    assert!(state.tool.is_some(), "{:?}", state.status);
    state
}

fn tool(state: &Interaction) -> &ModelSession {
    state.tool.as_ref().unwrap()
}

fn evaluation(effects: &[Effect]) -> Option<(RequestTag, String, &'static str)> {
    effects.iter().find_map(|effect| match effect {
        Effect::EvaluateExpression {
            tag,
            expression,
            site,
        } => Some((tag.clone(), expression.clone(), *site)),
        _ => None,
    })
}

fn answer(state: &mut Interaction, tag: &RequestTag, dimension: &str, value: f64) -> Vec<Effect> {
    state.dispatch(Action::Evaluated {
        tag: tag.clone(),
        result: Ok(EvaluatedExpressionDto {
            value,
            dimension: dimension.into(),
            error: None,
        }),
    })
}

fn typed(state: &mut Interaction, key: ParameterKey, text: &str) -> Vec<Effect> {
    state.dispatch(Action::ParameterChanged {
        key,
        text: text.into(),
    })
}

fn previewed(effects: &[Effect]) -> Option<Operation> {
    effects.iter().find_map(|effect| match effect {
        Effect::Preview { operation, .. } => Some(operation.as_ref().clone()),
        _ => None,
    })
}

fn applied(effects: &[Effect]) -> Vec<&EditCommand> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::ApplyOperation { command, .. } => Some(command.as_ref()),
            _ => None,
        })
        .collect()
}

fn extrude(op: &Operation) -> &onecad_core::document::record::ExtrudeParams {
    let Operation::Known(KnownOperation::Extrude(params)) = op else {
        panic!("Extrude")
    };
    params
}

/// Answers the start evaluation of a new Extrude `[distance, draft]` with 10 mm.
fn ready_two_field_extrude() -> Interaction {
    let mut state = begin(extrude_record(), vec![DISTANCE, DRAFT], false);
    let tag = tool(&state).tag.clone();
    let effects = answer(&mut state, &tag, "length", 10.0);
    assert!(previewed(&effects).is_some());
    state
}

#[test]
fn legacy_extrude_effect_list_is_unchanged() {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    state.dispatch(Action::SelectProfile(Box::new(selection(context()))));
    let effects = state.dispatch(Action::BeginExtrude {
        record_id: RecordId::new(),
        gesture: 9,
        step_index: 3,
    });
    assert!(matches!(
        effects.as_slice(),
        [Effect::ClearPreview { .. }, Effect::EvaluateExpression { expression, site: "length", .. }]
            if expression == "10"
    ));
    let fields: Vec<_> = tool(&state).params.fields().iter().map(|f| f.key).collect();
    assert_eq!(fields, vec![DISTANCE]);
    assert_eq!(
        tool(&state).parameter(),
        Some(NumericParameter::ExtrudeDistance)
    );
}

#[test]
fn stale_result_is_dropped_drain_reissues_and_preview_equals_commit() {
    let mut state = ready_two_field_extrude();
    let distance = evaluation(&typed(&mut state, DISTANCE, "12")).unwrap().0;
    let draft = evaluation(&typed(&mut state, DRAFT, "3")).unwrap();
    assert_eq!(draft.2, "angle");
    assert!(
        answer(&mut state, &distance, "length", 12.0).is_empty(),
        "stale"
    );
    assert_eq!(tool(&state).params.readiness(), Readiness::Pending);
    let drained = answer(&mut state, &draft.0, "angle", 3.0);
    let (resumed, expression, site) = evaluation(&drained).expect("distance re-issued");
    assert_eq!((expression.as_str(), site), ("12", "length"));
    assert!(resumed.sequence > draft.0.sequence);
    assert!(
        previewed(&drained).is_none(),
        "no preview until both are valid"
    );
    let effects = answer(&mut state, &resumed, "length", 12.0);
    let preview = previewed(&effects).expect("preview once both are valid");
    assert_eq!(extrude(&preview).distance, Scalar::with_expr(12.0, "12"));
    assert_eq!(
        extrude(&preview).draft_angle_deg,
        Scalar::with_expr(3.0, "3")
    );
    state.dispatch(Action::PreviewFinished {
        tag: resumed.clone(),
        result: Ok(preview_result(&resumed)),
    });
    let effects = state.dispatch(Action::Confirm);
    let [EditCommand::AddOperation { record, .. }] = applied(&effects).as_slice() else {
        panic!("one command")
    };
    assert_eq!(record.op, preview);
}

#[test]
fn every_sequence_bump_refills_the_evaluation_slot() {
    let mut state = ready_two_field_extrude();
    typed(&mut state, DISTANCE, "12");
    // A focus change never bumps; a draft edit takes the slot from distance.
    let sequence = tool(&state).tag.sequence;
    state.dispatch(Action::FocusParameter(DRAFT));
    assert_eq!(tool(&state).tag.sequence, sequence);
    assert_eq!(tool(&state).draft, "0");
    let draft = evaluation(&state.dispatch(Action::DraftChanged("4".into()))).unwrap();
    assert_eq!(draft.1, "4", "DraftChanged follows the focus");
    // Republish (tab parked) re-issues the focused field, then the drain resumes distance.
    let republished = evaluation(&state.dispatch(Action::Republish)).unwrap();
    assert!(answer(&mut state, &draft.0, "angle", 4.0).is_empty());
    let resumed = evaluation(&answer(&mut state, &republished.0, "angle", 4.0)).unwrap();
    assert_eq!(resumed.1, "12");
}

#[test]
fn op_error_is_recomputed_while_expression_error_sticks() {
    let mut state = ready_two_field_extrude();
    let tag = evaluation(&typed(&mut state, DISTANCE, "0")).unwrap().0;
    answer(&mut state, &tag, "length", 0.0);
    assert!(matches!(tool(&state).draft_status, DraftStatus::Invalid(_)));
    assert!(matches!(
        tool(&state).params.readiness(),
        Readiness::Invalid {
            key: Some(DISTANCE),
            ..
        }
    ));
    assert_eq!(tool(&state).value(DISTANCE), None);
    let tag = evaluation(&typed(&mut state, DISTANCE, "5")).unwrap().0;
    assert!(previewed(&answer(&mut state, &tag, "length", 5.0)).is_some());
    let tag = evaluation(&typed(&mut state, DRAFT, "nope")).unwrap().0;
    state.dispatch(Action::Evaluated {
        tag,
        result: Ok(EvaluatedExpressionDto {
            value: 0.0,
            dimension: "angle".into(),
            error: Some("unknown variable".into()),
        }),
    });
    let tag = evaluation(&state.dispatch(Action::FlipExtrude)).unwrap();
    assert_eq!(tag.1, "-(5)", "flip targets distance, not the focus");
    answer(&mut state, &tag.0, "length", -5.0);
    assert!(matches!(
        tool(&state).params.readiness(),
        Readiness::Invalid {
            key: Some(DRAFT),
            ..
        }
    ));
}

#[test]
fn unchanged_existing_edit_confirms_without_a_command() {
    let mut record = extrude_record();
    if let Operation::Known(KnownOperation::Extrude(p)) = &mut record.op {
        p.distance = Scalar::new(10.0);
        p.draft_angle_deg = Scalar::with_expr(2.0, "=angle");
    }
    let mut state = begin(record.clone(), vec![DISTANCE], true);
    let tag = tool(&state).tag.clone();
    // The stored value is a stale cache: re-evaluating the same text must not
    // author, but display and placement use the fresh value.
    assert!(answer(&mut state, &tag, "length", 8.0).is_empty());
    assert_eq!(
        tool(&state).draft_status,
        DraftStatus::Valid(Scalar::new(8.0))
    );
    assert_eq!(tool(&state).value(DISTANCE), Some(8.0));
    assert!(tool(&state).ready());
    let effects = state.dispatch(Action::Confirm);
    assert!(matches!(effects.as_slice(), [Effect::ClearPreview { .. }]));
    assert!(state.tool.is_none());
    assert!(matches!(state.status, CommitStatus::Idle));

    // Typing the original text back is untouched as well.
    let mut state = begin(record, vec![DISTANCE], true);
    typed(&mut state, DISTANCE, "11");
    typed(&mut state, DISTANCE, "10");
    assert!(tool(&state).ready());
    assert!(applied(&state.dispatch(Action::Confirm)).is_empty());
}

#[test]
fn broken_untouched_binding_never_blocks_other_fields_and_one_apply_updates_both() {
    let mut record = extrude_record();
    if let Operation::Known(KnownOperation::Extrude(p)) = &mut record.op {
        p.distance = Scalar::with_expr(10.0, "=deleted");
    }
    let mut state = begin(record.clone(), vec![DISTANCE], true);
    let start = tool(&state).tag.clone();
    state.dispatch(Action::Evaluated {
        tag: start,
        result: Ok(EvaluatedExpressionDto {
            value: 0.0,
            dimension: "length".into(),
            error: Some("unknown variable deleted".into()),
        }),
    });
    assert_eq!(tool(&state).params.readiness(), Readiness::Ready);
    state.dispatch(Action::FocusParameter(DRAFT));
    let tag = evaluation(&state.dispatch(Action::DraftChanged("3".into())))
        .unwrap()
        .0;
    answer(&mut state, &tag, "angle", 3.0);
    state.dispatch(Action::FocusParameter(DISTANCE));
    assert_eq!(tool(&state).draft, "=deleted");
    let tag = evaluation(&typed(&mut state, DISTANCE, "14")).unwrap().0;
    answer(&mut state, &tag, "length", 14.0);
    let effects = state.dispatch(Action::Confirm);
    let [EditCommand::UpdateOperationParams { op, .. }] = applied(&effects).as_slice() else {
        panic!("one command for two fields")
    };
    assert_eq!(extrude(op).distance, Scalar::with_expr(14.0, "14"));
    assert_eq!(extrude(op).draft_angle_deg, Scalar::with_expr(3.0, "3"));
    assert_eq!(extrude(op).profile, extrude(&record.op).profile);
}

#[test]
fn drag_cancel_restores_expression_and_commit_keeps_value() {
    let mut state = ready_two_field_extrude();
    let tag = evaluation(&typed(&mut state, DISTANCE, "=thickness*2"))
        .unwrap()
        .0;
    answer(&mut state, &tag, "length", 12.0);
    let before = tool(&state).params.clone();
    let sequence = tool(&state).tag.sequence;
    assert!(state
        .dispatch(Action::ParameterDrag(DragPhase::Begin))
        .is_empty());
    assert_eq!(
        tool(&state).tag.sequence,
        sequence,
        "begin is a snapshot only"
    );
    let drag = |text: &str| {
        Action::ParameterDrag(DragPhase::Update {
            key: DISTANCE,
            text: text.into(),
        })
    };
    let tag = evaluation(&state.dispatch(drag("20"))).unwrap().0;
    answer(&mut state, &tag, "length", 20.0);
    assert!(
        tool(&state).dragging(),
        "drag updates never commit the drag"
    );
    let effects = state.dispatch(Action::ParameterDrag(DragPhase::Cancel));
    assert!(matches!(effects.first(), Some(Effect::ClearPreview { .. })));
    let preview = previewed(&effects).expect("restored values re-preview at once");
    assert_eq!(
        extrude(&preview).distance,
        Scalar::with_expr(12.0, "=thickness*2")
    );
    assert_eq!(tool(&state).params.fields(), before.fields());
    assert_eq!(tool(&state).draft, "=thickness*2");
    assert!(!tool(&state).dragging());
    // Nothing changed since begin: cancel is silent.
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    let sequence = tool(&state).tag.sequence;
    assert!(state
        .dispatch(Action::ParameterDrag(DragPhase::Cancel))
        .is_empty());
    assert_eq!(tool(&state).tag.sequence, sequence);
    // Commit keeps the dragged value.
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    state.dispatch(drag("25"));
    state.dispatch(Action::ParameterDrag(DragPhase::Commit));
    assert!(state
        .dispatch(Action::ParameterDrag(DragPhase::Cancel))
        .is_empty());
    assert_eq!(tool(&state).draft, "25");
}

#[test]
fn typed_input_choice_begin_and_confirm_close_an_open_drag() {
    let mut state = ready_two_field_extrude();
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    typed(&mut state, DISTANCE, "15");
    assert!(!tool(&state).dragging(), "typed input commits the drag");
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    state.dispatch(Action::ExtrudeModeChanged(
        onecad_core::document::record::BooleanMode::Cut,
    ));
    assert!(!tool(&state).dragging(), "choices commit the drag");
    state.dispatch(Action::ExtrudeModeChanged(
        onecad_core::document::record::BooleanMode::NewBody,
    ));
    // A second begin commits the first drag: a later cancel restores only the second.
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    state.dispatch(Action::ParameterDrag(DragPhase::Update {
        key: DISTANCE,
        text: "30".into(),
    }));
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    state.dispatch(Action::ParameterDrag(DragPhase::Update {
        key: DISTANCE,
        text: "40".into(),
    }));
    let tag = evaluation(&state.dispatch(Action::ParameterDrag(DragPhase::Cancel)))
        .unwrap()
        .0;
    assert_eq!(tool(&state).draft, "30");
    let preview = previewed(&answer(&mut state, &tag, "length", 30.0)).unwrap();
    state.dispatch(Action::PreviewFinished {
        tag: tag.clone(),
        result: Ok(preview_result(&tag)),
    });
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    let effects = state.dispatch(Action::Confirm);
    let [EditCommand::AddOperation { record, .. }] = applied(&effects).as_slice() else {
        panic!("one command")
    };
    assert_eq!(record.op, preview);
    assert!(!tool(&state).dragging(), "confirm drops the snapshot");
    // Context loss drops the tool with its snapshot.
    let mut state = ready_two_field_extrude();
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    state.dispatch(Action::ContextChanged(None));
    assert!(state.tool.is_none());
}

#[test]
fn drag_cancel_restoring_a_pending_field_reissues_its_evaluation() {
    let mut state = ready_two_field_extrude();
    typed(&mut state, DRAFT, "2");
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    let tag = evaluation(&state.dispatch(Action::ParameterDrag(DragPhase::Update {
        key: DISTANCE,
        text: "20".into(),
    })))
    .unwrap()
    .0;
    answer(&mut state, &tag, "length", 20.0);
    let (_, expression, site) =
        evaluation(&state.dispatch(Action::ParameterDrag(DragPhase::Cancel))).unwrap();
    assert_eq!((expression.as_str(), site), ("2", "angle"));
}

fn transform_record() -> OperationRecord {
    OperationRecord::new(
        RecordId::new(),
        2,
        "Move",
        Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
            targets: vec![BodyId::new()],
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
            copy: false,
            extra: Default::default(),
        })),
    )
}

fn translate(op: &Operation) -> [Scalar; 3] {
    let Operation::Known(KnownOperation::TransformBody(p)) = op else {
        panic!("TransformBody")
    };
    p.translate.clone()
}

#[test]
fn gizmo_motion_keeps_typed_expression_and_cancel_restores_it() {
    let mut state = begin(transform_record(), vec![TX], false);
    let tag = evaluation(&typed(&mut state, TX, "=a")).unwrap().0;
    answer(&mut state, &tag, "length", 3.0);
    let gizmo = |translate: [f64; 3]| Action::TransformDraftChanged {
        translate,
        axis: [0.0, 0.0, 1.0],
        angle_degrees: 0.0,
    };
    state.dispatch(Action::ParameterDrag(DragPhase::Begin));
    let tag = evaluation(&state.dispatch(gizmo([3.0, 7.0, 0.0]))).unwrap();
    assert_eq!(tag.1, "=a", "untouched X keeps its expression");
    let preview = previewed(&answer(&mut state, &tag.0, "length", 3.0)).unwrap();
    assert_eq!(translate(&preview)[0], Scalar::with_expr(3.0, "=a"));
    assert_eq!(translate(&preview)[1], Scalar::new(7.0));
    assert!(
        tool(&state).dragging(),
        "placement updates never commit the drag"
    );
    let tag = evaluation(&state.dispatch(gizmo([5.0, 7.0, 0.0]))).unwrap();
    assert_eq!(tag.1, "5");
    assert_eq!(tool(&state).params.field(TX).unwrap().original, "0");
    let effects = state.dispatch(Action::ParameterDrag(DragPhase::Cancel));
    let preview = previewed(&effects).expect("restored placement previews");
    assert_eq!(translate(&preview)[0], Scalar::with_expr(3.0, "=a"));
    assert_eq!(translate(&preview)[1], Scalar::new(0.0));
    assert_eq!(tool(&state).draft, "=a");
}

/// Answers every evaluation in turn, numbers by parsing and `=a` as 3.
fn settle(state: &mut Interaction, mut effects: Vec<Effect>) {
    while let Some((tag, text, _)) = evaluation(&effects) {
        let value = if text == "=a" {
            3.0
        } else {
            text.parse().unwrap()
        };
        effects = answer(state, &tag, "length", value);
    }
}

fn gizmo(translate: [f64; 3]) -> Action {
    Action::TransformDraftChanged {
        translate,
        axis: [0.0, 0.0, 1.0],
        angle_degrees: 0.0,
    }
}

fn committed_translate(state: &mut Interaction) -> [Scalar; 3] {
    let effects = state.dispatch(Action::Confirm);
    let [EditCommand::UpdateOperationParams { op, .. }] = applied(&effects).as_slice() else {
        panic!("one update")
    };
    translate(op)
}

#[test]
fn existing_transform_field_typed_back_to_original_commits_the_authored_slot() {
    let edit = |state: &mut Interaction| {
        let effects = state.dispatch(Action::BeginParameterEdit {
            record: Box::new(transform_record()),
            parameter: TX.parameter(),
            gesture: 3,
            context: context(),
        });
        settle(state, effects);
    };
    // Expression on X, gizmo on Y, then X typed back to its original.
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    edit(&mut state);
    let effects = typed(&mut state, TX, "=a");
    settle(&mut state, effects);
    let effects = state.dispatch(gizmo([3.0, 7.0, 0.0]));
    settle(&mut state, effects);
    let effects = typed(&mut state, TX, "0");
    settle(&mut state, effects);
    assert_eq!(
        tool(&state).params.field(TX).unwrap().status,
        FieldStatus::Authored
    );
    assert_eq!(tool(&state).value(TX), Some(0.0));
    assert_eq!(tool(&state).value(TY), Some(7.0));
    let committed = committed_translate(&mut state);
    assert_eq!(committed[0], Scalar::new(0.0));
    assert_eq!(committed[1].value, 7.0);

    // The gizmo itself moved X, then X was typed back to its original.
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    edit(&mut state);
    let effects = state.dispatch(gizmo([5.0, 7.0, 0.0]));
    settle(&mut state, effects);
    let effects = typed(&mut state, TX, "0");
    settle(&mut state, effects);
    assert_eq!(tool(&state).value(TX), Some(0.0));
    assert_eq!(
        translate(&tool(&state).effective_op())[0],
        Scalar::new(0.0),
        "placement shows the authored slot"
    );
    let committed = committed_translate(&mut state);
    assert_eq!(committed[0], Scalar::new(0.0));
    assert_eq!(committed[1].value, 7.0);
}

fn preview_result(tag: &RequestTag) -> PreviewResultDto {
    support::preview(tag)
}
