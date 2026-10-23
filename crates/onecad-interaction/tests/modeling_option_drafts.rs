#[allow(dead_code)]
mod support;

use onecad_core::{
    document::record::{KnownOperation, Operation, OperationRecord},
    ids::{BodyId, RecordId},
    math::Vec3,
};
use onecad_interaction::*;
use support::{context, evaluate, preview};

fn begin(record: OperationRecord, existing: bool) -> Interaction {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let action = if existing {
        Action::BeginOperationEdit {
            record: Box::new(record),
            gesture: 1,
            context: context(),
        }
    } else {
        operand_binding::begin_bound(record, 1, context()).unwrap()
    };
    state.dispatch(action);
    assert!(state.tool.is_some());
    state
}

fn tag(state: &Interaction) -> RequestTag {
    state.tool.as_ref().unwrap().tag.clone()
}

fn draft(
    state: &mut Interaction,
    options: Option<ModelingOptions>,
    error: Option<&str>,
) -> Vec<Effect> {
    state.dispatch(Action::ModelingOptionsDraftChanged {
        tag: tag(state),
        options,
        error: error.map(str::to_owned),
    })
}

fn no_preview(effects: &[Effect]) {
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::Preview { .. }
            | Effect::PreviewExistingEdit { .. }
            | Effect::PublishPreview { .. }
            | Effect::ApplyOperation { .. }
    )));
}

fn blocked(state: &mut Interaction) {
    assert!(!state.tool.as_ref().unwrap().ready());
    assert!(state.dispatch(Action::Confirm).is_empty());
    assert!(state.tool.as_ref().unwrap().option_draft_error.is_some());
}

fn mirror_options(fuse: bool) -> ModelingOptions {
    ModelingOptions::Mirror {
        point: Vec3::new_unchecked(3.0, 0.0, 0.0),
        normal: Vec3::new_unchecked(1.0, 0.0, 0.0),
        fuse,
    }
}

#[test]
fn invalid_new_option_draft_fences_old_preview_and_same_value_correction_retries() {
    let mut state = begin(
        modeling_records::mirror(BodyId::new(), RecordId::new(), 0),
        false,
    );
    let previous = tag(&state);
    state.dispatch(Action::PreviewFinished {
        tag: previous.clone(),
        result: Ok(preview(&previous)),
    });
    assert!(state.tool.as_ref().unwrap().ready());
    let effects = draft(&mut state, None, Some("Enter a finite X coordinate"));
    assert!(tag(&state).sequence > previous.sequence);
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::ClearPreview { .. })));
    no_preview(&effects);
    for reply in [previous.clone(), tag(&state)] {
        no_preview(&state.dispatch(Action::PreviewFinished {
            tag: reply.clone(),
            result: Ok(preview(&reply)),
        }));
        blocked(&mut state);
    }
    assert!(state
        .dispatch(Action::ModelingOptionsDraftChanged {
            tag: previous,
            options: None,
            error: None,
        })
        .is_empty());
    blocked(&mut state);
    let invalid = tag(&state);
    let effects = draft(&mut state, None, None);
    assert!(tag(&state).sequence > invalid.sequence);
    assert!(effects.iter().any(|e| matches!(e, Effect::Preview { .. })));
    assert!(!state.tool.as_ref().unwrap().ready());
    no_preview(&state.dispatch(Action::PreviewFinished {
        tag: invalid.clone(),
        result: Ok(preview(&invalid)),
    }));
    assert!(!state.tool.as_ref().unwrap().ready());
    let current = tag(&state);
    state.dispatch(Action::PreviewFinished {
        tag: current.clone(),
        result: Ok(preview(&current)),
    });
    assert!(state.tool.as_ref().unwrap().ready());
    assert!(state
        .dispatch(Action::Confirm)
        .iter()
        .any(|e| matches!(e, Effect::ApplyOperation { .. })));
}

#[test]
fn option_error_survives_scalar_evaluation_choices_and_republish() {
    let mut state = begin(
        modeling_records::linear_pattern(BodyId::new(), 10.0, RecordId::new(), 0),
        false,
    );
    let old_evaluation = tag(&state);
    no_preview(&draft(
        &mut state,
        None,
        Some("Enter an integer instance count"),
    ));
    assert!(evaluate(&mut state, &old_evaluation, 10.0).is_empty());
    let current = tag(&state);
    no_preview(&evaluate(&mut state, &current, 10.0));
    blocked(&mut state);
    let effects = state.dispatch(Action::ParameterChanged {
        key: NumericParameter::LinearPatternSpacing.into(),
        text: "20".into(),
    });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::EvaluateExpression { .. })));
    let current = tag(&state);
    no_preview(&evaluate(&mut state, &current, 20.0));
    blocked(&mut state);
    no_preview(&state.dispatch(Action::ModelingOptionsChanged {
        tag: tag(&state),
        options: ModelingOptions::Pattern {
            count: 5,
            direction: Vec3::new_unchecked(0.0, 1.0, 0.0),
            origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            fuse: true,
        },
    }));
    blocked(&mut state);
    no_preview(&state.dispatch(Action::Republish));
    let current = tag(&state);
    no_preview(&evaluate(&mut state, &current, 20.0));
    blocked(&mut state);
    let effects = draft(&mut state, None, None);
    let operation = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Preview { operation, .. } => Some(operation.as_ref()),
            _ => None,
        })
        .expect("corrected options preview the current scalar and choices");
    let Operation::Known(KnownOperation::LinearPattern(params)) = operation else {
        panic!()
    };
    assert_eq!(params.spacing.value, 20.0);
    assert_eq!(params.count, 5);
    assert!(params.fuse_result);
}

#[test]
fn invalid_option_and_valid_sibling_choice_are_one_fenced_candidate() {
    let mut state = begin(
        modeling_records::mirror(BodyId::new(), RecordId::new(), 0),
        false,
    );
    let previous = tag(&state);
    no_preview(&draft(
        &mut state,
        Some(mirror_options(true)),
        Some("Enter a finite normal"),
    ));
    assert_eq!(tag(&state).sequence, previous.sequence + 1);
    blocked(&mut state);
    let Operation::Known(KnownOperation::MirrorBody(params)) =
        state.tool.as_ref().unwrap().effective_op()
    else {
        panic!()
    };
    assert!(params.fuse_with_original);
    let effects = draft(&mut state, None, None);
    assert!(effects.iter().any(|e| matches!(e, Effect::Preview { .. })));
}

#[test]
fn invalid_existing_option_draft_blocks_late_edit_preview_and_apply_fallback() {
    let mut state = begin(
        modeling_records::mirror(BodyId::new(), RecordId::new(), 0),
        true,
    );
    let effects = draft(&mut state, Some(mirror_options(false)), None);
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::PreviewExistingEdit { .. })));
    let previous = tag(&state);
    no_preview(&draft(
        &mut state,
        None,
        Some("Enter a finite plane coordinate"),
    ));
    for reply in [previous, tag(&state)] {
        no_preview(&state.dispatch(Action::EditPreviewFinished {
            tag: reply,
            result: Err(onecad_app::error::ApiError::Unsupported(
                "preview unavailable".into(),
            )),
        }));
        blocked(&mut state);
    }
    let effects = draft(&mut state, None, None);
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::PreviewExistingEdit { .. })));
    let current = tag(&state);
    support::edit_unavailable(&mut state, &current);
    assert!(state.tool.as_ref().unwrap().ready());
    assert!(state
        .dispatch(Action::Confirm)
        .iter()
        .any(|e| matches!(e, Effect::ApplyOperation { .. })));
}

#[test]
fn correcting_unchanged_existing_option_draft_restores_noop_close() {
    let mut state = begin(
        modeling_records::mirror(BodyId::new(), RecordId::new(), 0),
        true,
    );
    assert!(state.tool.as_ref().unwrap().ready());
    no_preview(&draft(
        &mut state,
        None,
        Some("Enter a finite plane coordinate"),
    ));
    blocked(&mut state);
    no_preview(&draft(&mut state, None, None));
    assert!(state.tool.as_ref().unwrap().ready());
    no_preview(&state.dispatch(Action::Confirm));
    assert!(state.tool.is_none());
}
