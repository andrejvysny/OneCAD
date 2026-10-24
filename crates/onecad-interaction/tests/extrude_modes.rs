#[allow(dead_code)]
mod support;
use onecad_core::{
    document::record::{BooleanMode, KnownOperation, Operation},
    edit::EditCommand,
    ids::BodyId,
};
use onecad_interaction::*;
use support::*;

fn params(state: &Interaction) -> &onecad_core::document::record::ExtrudeParams {
    let Operation::Known(KnownOperation::Extrude(params)) = &state.tool.as_ref().unwrap().record.op
    else {
        panic!("Extrude")
    };
    params
}

#[test]
fn add_cut_intersect_require_explicit_target_and_preserve_profile_and_record_identity() {
    for mode in [BooleanMode::Add, BooleanMode::Cut, BooleanMode::Intersect] {
        let (mut state, old_tag) = started();
        let profile = params(&state).profile.clone();
        let effects = state.dispatch(Action::ExtrudeModeChanged(mode));
        let tag = evaluation_tag(&effects);
        assert_ne!(tag, old_tag);
        assert!(evaluate(&mut state, &old_tag, 10.0).is_empty());
        assert!(evaluate(&mut state, &tag, 10.0).is_empty());
        assert!(matches!(
            state.tool.as_ref().unwrap().preview,
            PreviewStatus::Failed(_)
        ));
        assert!(state.dispatch(Action::Confirm).is_empty());
        assert_eq!(params(&state).target_body, None);
        let target = BodyId::new();
        let effects = state.dispatch(Action::ExtrudeTargetChanged {
            body: Some(target),
            context: context(),
        });
        let target_tag = evaluation_tag(&effects);
        let effects = evaluate(&mut state, &target_tag, 10.0);
        let [Effect::Preview { operation, .. }] = effects.as_slice() else {
            panic!("preview")
        };
        let preview_operation = operation.as_ref().clone();
        assert_eq!(params(&state).profile, profile);
        assert_eq!(params(&state).target_body, Some(target));
        state.dispatch(Action::PreviewFinished {
            tag: target_tag.clone(),
            result: Ok(preview(&target_tag)),
        });
        let effects = state.dispatch(Action::Confirm);
        let command = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::ApplyOperation { command, .. } => Some(command),
                _ => None,
            })
            .unwrap();
        let EditCommand::AddOperation { record, .. } = command.as_ref() else {
            panic!("new operation")
        };
        assert_eq!(record.op, preview_operation);
        assert_eq!(record.record_id, old_tag.record_id);
        assert_eq!(record.inputs, record.op.derive_inputs());
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
}

#[test]
fn intersect_invalidates_ready_preview_and_refuses_delayed_new_body_reply() {
    let (mut state, old_tag) = started();
    evaluate(&mut state, &old_tag, 10.0);
    state.dispatch(Action::PreviewFinished {
        tag: old_tag.clone(),
        result: Ok(preview(&old_tag)),
    });
    assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Ready);
    let effects = state.dispatch(Action::ExtrudeModeChanged(BooleanMode::Intersect));
    let tag = evaluation_tag(&effects);
    assert_eq!(params(&state).boolean_mode, BooleanMode::Intersect);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::ClearPreview { .. })));
    assert_ne!(tag, old_tag);
    assert_ne!(state.tool.as_ref().unwrap().preview, PreviewStatus::Ready);
    assert!(state.dispatch(Action::Confirm).is_empty());
    assert!(state
        .dispatch(Action::PreviewFinished {
            tag: old_tag.clone(),
            result: Ok(preview(&old_tag)),
        })
        .is_empty());
    assert!(evaluate(&mut state, &tag, 10.0).is_empty());
    assert!(matches!(
        &state.tool.as_ref().unwrap().preview,
        PreviewStatus::Failed(reason) if reason.contains("target body")
    ));
    assert!(state.dispatch(Action::Confirm).is_empty());
}

#[test]
fn replaced_target_rejects_delayed_preview_and_new_body_clears_target() {
    let (mut state, _) = started();
    state.dispatch(Action::ExtrudeModeChanged(BooleanMode::Add));
    let effects = state.dispatch(Action::ExtrudeTargetChanged {
        body: Some(BodyId::new()),
        context: context(),
    });
    let old_tag = evaluation_tag(&effects);
    evaluate(&mut state, &old_tag, 10.0);
    let effects = state.dispatch(Action::ExtrudeTargetChanged {
        body: Some(BodyId::new()),
        context: context(),
    });
    let next = evaluation_tag(&effects);
    assert!(state
        .dispatch(Action::PreviewFinished {
            tag: old_tag.clone(),
            result: Ok(preview(&old_tag))
        })
        .is_empty());
    assert_eq!(state.tool.as_ref().unwrap().tag, next);
    assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Empty);
    state.dispatch(Action::ExtrudeModeChanged(BooleanMode::NewBody));
    assert_eq!(params(&state).target_body, None);
    state.dispatch(Action::ExtrudeModeChanged(BooleanMode::Cut));
    assert_eq!(params(&state).target_body, None);
}

#[test]
fn flip_negates_same_expression_through_resolver_and_keeps_profile() {
    let (mut state, _) = started();
    state.dispatch(Action::DraftChanged("Depth + 2mm".into()));
    let profile = params(&state).profile.clone();
    let effects = state.dispatch(Action::FlipExtrude);
    let tag = evaluation_tag(&effects);
    assert_eq!(state.tool.as_ref().unwrap().draft, "-(Depth + 2mm)");
    let effects = evaluate(&mut state, &tag, -12.0);
    let [Effect::Preview { operation, .. }] = effects.as_slice() else {
        panic!("preview")
    };
    let Operation::Known(KnownOperation::Extrude(params)) = operation.as_ref() else {
        panic!("Extrude")
    };
    assert_eq!(params.distance.value, -12.0);
    assert_eq!(params.distance.expr.as_deref(), Some("-(Depth + 2mm)"));
    assert_eq!(params.profile, profile);
}

#[test]
fn stale_targets_cancelled_replies_and_pending_mode_changes_cannot_replace_session() {
    let (mut state, _) = started();
    state.dispatch(Action::ExtrudeModeChanged(BooleanMode::Cut));
    let mut stale = context();
    stale.revision -= 1;
    assert!(state
        .dispatch(Action::ExtrudeTargetChanged {
            body: Some(BodyId::new()),
            context: stale
        })
        .is_empty());
    assert_eq!(params(&state).target_body, None);
    let tag = state.tool.as_ref().unwrap().tag.clone();
    state.dispatch(Action::Cancel);
    assert!(evaluate(&mut state, &tag, 10.0).is_empty());
    assert!(state.tool.is_none());
    let (mut state, tag) = confirmed();
    assert!(state
        .dispatch(Action::ExtrudeModeChanged(BooleanMode::Cut))
        .is_empty());
    assert!(state.dispatch(Action::FlipExtrude).is_empty());
    assert_eq!(state.tool.as_ref().unwrap().tag, tag);
    assert_eq!(params(&state).boolean_mode, BooleanMode::NewBody);
}

#[test]
fn saved_feature_modes_never_start_an_appended_preview() {
    let (mut state, tag) = editing();
    assert!(state
        .dispatch(Action::ExtrudeModeChanged(BooleanMode::Cut))
        .is_empty());
    assert!(state
        .dispatch(Action::ExtrudeTargetChanged {
            body: Some(BodyId::new()),
            context: context()
        })
        .is_empty());
    assert!(matches!(
        evaluate(&mut state, &tag, 8.0).as_slice(),
        [Effect::PreviewExistingEdit { .. }]
    ));
    edit_unavailable(&mut state, &tag);
    assert_eq!(
        state.tool.as_ref().unwrap().preview,
        PreviewStatus::UnavailableForEdit
    );
    let effects = state.dispatch(Action::Confirm);
    assert!(effects.iter().any(|effect| matches!(effect, Effect::ApplyOperation { command, .. } if matches!(command.as_ref(), EditCommand::UpdateOperationParams { .. }))));
}
