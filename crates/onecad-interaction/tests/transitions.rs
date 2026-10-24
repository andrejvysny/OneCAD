use onecad_app::dto::FailedStep;
use onecad_core::{
    document::record::{KnownOperation, Operation},
    edit::EditCommand,
    ids::{RecordId, SnapshotId},
};
use onecad_interaction::*;
use uuid::Uuid;

mod support;
use support::*;

#[test]
fn profile_deselection_prevents_creation_without_changing_document_context() {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    state.dispatch(Action::SelectProfile(Box::new(selection(context()))));
    assert!(state.dispatch(Action::ClearSelection).is_empty());
    assert!(state.selection.is_none());
    assert_eq!(state.context, Some(context()));
    assert!(state
        .dispatch(Action::BeginExtrude {
            record_id: RecordId::new(),
            gesture: 10,
            step_index: 3,
        })
        .is_empty());
    assert!(state.tool.is_none());
}

#[test]
fn canceling_tool_preserves_input_and_never_mutates_authoritative_document() {
    let (mut state, tag) = started();
    let selected = state.selection.clone();
    let effects = state.dispatch(Action::Cancel);
    assert!(matches!(effects.as_slice(), [Effect::ClearPreview { .. }]));
    assert!(state.tool.is_none());
    assert_eq!(state.selection, selected);
    assert_eq!(state.context, Some(tag.context));
    assert!(state.dispatch(Action::ClearSelection).is_empty());
    assert!(state.selection.is_none());
}

#[test]
fn deselection_does_not_cancel_or_repeat_an_admitted_commit() {
    let (mut state, tag) = confirmed();
    assert!(state.dispatch(Action::ClearSelection).is_empty());
    assert!(state.selection.is_none());
    assert_eq!(state.tool.as_ref().unwrap().phase, SessionPhase::Committing);
    assert!(matches!(&state.status, CommitStatus::Pending(pending) if pending == &tag));
    assert!(state.dispatch(Action::Confirm).is_empty());
}

#[test]
fn exact_profile_and_record_identity_survive_expression_preview_and_commit() {
    let (mut state, _) = started();
    let tag = evaluation_tag(&state.dispatch(Action::DraftChanged("wall * 2".into())));
    let effects = evaluate(&mut state, &tag, 24.0);
    let Effect::Preview {
        tag: preview_tag,
        operation,
        expected_snapshot,
        ..
    } = &effects[0]
    else {
        panic!()
    };
    assert_eq!(preview_tag, &tag);
    assert_eq!(*expected_snapshot, SnapshotId(8));
    let expected = operation.clone();
    state.dispatch(Action::PreviewFinished {
        tag: tag.clone(),
        result: Ok(preview(&tag)),
    });
    let effects = state.dispatch(Action::Confirm);
    let Effect::ApplyOperation {
        tag: commit_tag,
        command,
        hide_sketch,
    } = &effects[1]
    else {
        panic!()
    };
    assert_eq!(commit_tag.record_id, preview_tag.record_id);
    let EditCommand::AddOperation { record, at_cursor } = command.as_ref() else {
        panic!()
    };
    assert!(*at_cursor);
    assert_eq!(record.record_id, tag.record_id);
    assert_eq!(record.op, *expected);
    assert_eq!(*hide_sketch, Some(selection(context()).profile.sketch));
    let Operation::Known(KnownOperation::Extrude(params)) = &record.op else {
        panic!()
    };
    assert_eq!(params.profile.as_ref(), Some(&selection(context()).profile));
    assert_eq!(params.distance.expr.as_deref(), Some("wall * 2"));
}

#[test]
fn duplicate_enter_and_escape_during_admission_never_repeat_or_undo_commit() {
    let (mut state, tag) = confirmed();
    assert!(state.dispatch(Action::Confirm).is_empty());
    let effects = state.dispatch(Action::Cancel);
    assert!(matches!(
        effects.as_slice(),
        [Effect::ClearPreview { gesture: 9 }]
    ));
    assert!(matches!(&state.status, CommitStatus::Pending(pending) if pending == &tag));
    assert!(state.dispatch(Action::Confirm).is_empty());
}

#[test]
fn invalid_incomplete_nonfinite_zero_or_wrong_dimension_never_commits() {
    for (value, dimension, error) in [
        (1.0, "length", Some("incomplete expression")),
        (f64::NAN, "length", None),
        (f64::INFINITY, "length", None),
        (0.0, "length", None),
        (20.0, "angle", None),
    ] {
        let (mut state, tag) = started();
        let effects = state.dispatch(Action::Evaluated {
            tag,
            result: Ok(EvaluatedExpressionDto {
                value,
                dimension: dimension.into(),
                error: error.map(str::to_string),
            }),
        });
        assert!(effects.is_empty());
        assert!(matches!(
            state.tool.as_ref().unwrap().draft_status,
            DraftStatus::Invalid(_)
        ));
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
}

#[test]
fn negative_length_keeps_sign_and_empty_draft_rejects_forged_valid_result() {
    let (mut state, _) = started();
    let tag = evaluation_tag(&state.dispatch(Action::DraftChanged("-12 mm".into())));
    assert!(matches!(
        evaluate(&mut state, &tag, -12.0).as_slice(),
        [Effect::Preview { .. }]
    ));
    assert!(
        matches!(&state.tool.as_ref().unwrap().draft_status, DraftStatus::Valid(value) if value.value == -12.0)
    );
    state.dispatch(Action::PreviewFinished {
        tag: tag.clone(),
        result: Ok(preview(&tag)),
    });
    let tag = evaluation_tag(&state.dispatch(Action::DraftChanged(String::new())));
    assert!(evaluate(&mut state, &tag, 1.0).is_empty());
    assert!(state.dispatch(Action::Confirm).is_empty());
}

#[test]
fn newer_numeric_draft_coalesces_evaluation_and_preview_replies() {
    let (mut state, old) = started();
    evaluate(&mut state, &old, 10.0);
    let changed = state.dispatch(Action::DraftChanged("25".into()));
    assert!(!changed.iter().any(|effect| matches!(
        effect,
        Effect::EvaluateExpression { .. } | Effect::Preview { .. }
    )));
    assert!(evaluate(&mut state, &old, 99.0).is_empty());
    let completed = state.dispatch(Action::PreviewFinished {
        tag: old.clone(),
        result: Ok(preview(&old)),
    });
    assert!(completed
        .iter()
        .any(|effect| matches!(effect, Effect::PublishPreview { tag, .. } if tag == &old)));
    assert!(!state.tool.as_ref().unwrap().ready());
    let current = evaluation_tag(&completed);
    assert!(matches!(
        evaluate(&mut state, &current, 25.0).as_slice(),
        [Effect::Preview { .. }]
    ));
    assert!(matches!(
        state
            .dispatch(Action::PreviewFinished {
                tag: current.clone(),
                result: Ok(preview(&current))
            })
            .as_slice(),
        [Effect::PublishPreview { .. }]
    ));
    assert!(state
        .dispatch(Action::PreviewFinished {
            tag: current.clone(),
            result: Ok(preview(&current))
        })
        .is_empty());
}

#[test]
fn escape_invalidates_late_reply_even_when_gesture_and_operation_id_reused() {
    let (mut state, old) = started();
    evaluate(&mut state, &old, 10.0);
    assert!(matches!(
        state.dispatch(Action::Cancel).as_slice(),
        [Effect::ClearPreview { .. }]
    ));
    assert!(state.tool.is_none());
    let effects = state.dispatch(Action::BeginExtrude {
        record_id: old.record_id,
        gesture: old.gesture,
        step_index: 3,
    });
    let current = evaluation_tag(&effects);
    assert_ne!(old.sequence, current.sequence);
    assert!(state
        .dispatch(Action::PreviewFinished {
            tag: old.clone(),
            result: Ok(preview(&old))
        })
        .is_empty());
    assert!(evaluate(&mut state, &old, 10.0).is_empty());
}

#[test]
fn new_runtime_invalidates_all_old_results_and_selection() {
    let (mut state, tag) = confirmed();
    let mut next = context();
    next.runtime_session = "runtime-b".into();
    next.revision = 1;
    state.dispatch(Action::ContextChanged(Some(next)));
    assert!(state.selection.is_none());
    assert!(state.tool.is_none());
    assert!(state
        .dispatch(Action::CommitAccepted {
            tag: tag.clone(),
            revision: 5,
            rollback_token: None
        })
        .is_empty());
    assert!(matches!(state.status, CommitStatus::Idle));
    assert!(evaluate(&mut state, &tag, 10.0).is_empty());
}

#[test]
fn projection_before_acceptance_preserves_commit_and_regen_is_distinct() {
    let (mut state, tag) = confirmed();
    let mut next = context();
    next.revision = 5;
    state.dispatch(Action::ContextChanged(Some(next)));
    assert!(matches!(state.status, CommitStatus::Pending(_)));
    state.dispatch(Action::CommitAccepted {
        tag: tag.clone(),
        revision: 5,
        rollback_token: Some("guarded-token".into()),
    });
    assert!(matches!(
        state.status,
        CommitStatus::Accepted { revision: 5, .. }
    ));
    state.dispatch(Action::RegenFinished {
        tag: tag.clone(),
        completion: Box::new(completion(&tag, 4)),
    });
    assert!(matches!(state.status, CommitStatus::Accepted { .. }));
    let mut old = completion(&tag, 5);
    old.outcome = "superseded".into();
    state.dispatch(Action::RegenFinished {
        tag: tag.clone(),
        completion: Box::new(old),
    });
    assert!(matches!(state.status, CommitStatus::Accepted { .. }));
    state.dispatch(Action::RegenFinished {
        tag: tag.clone(),
        completion: Box::new(completion(&tag, 5)),
    });
    assert!(matches!(
        state.status,
        CommitStatus::Completed {
            outcome: FeatureOutcome::Published,
            ..
        }
    ));
}

#[test]
fn regen_event_before_acceptance_is_buffered_until_its_revision_is_known() {
    let (mut state, tag) = confirmed();
    state.dispatch(Action::RegenFinished {
        tag: tag.clone(),
        completion: Box::new(completion(&tag, 5)),
    });
    assert!(matches!(state.status, CommitStatus::Pending(_)));
    state.dispatch(Action::CommitAccepted {
        tag,
        revision: 5,
        rollback_token: None,
    });
    assert!(matches!(
        state.status,
        CommitStatus::Completed {
            outcome: FeatureOutcome::Published,
            ..
        }
    ));
}

#[test]
fn published_feature_failure_keeps_accepted_history_and_guarded_token() {
    let (mut state, tag) = confirmed();
    state.dispatch(Action::CommitAccepted {
        tag: tag.clone(),
        revision: 5,
        rollback_token: Some("guard".into()),
    });
    let mut report = completion(&tag, 5);
    report.failed_steps.push(FailedStep {
        record_id: tag.record_id.to_string(),
        message: "kernel refused".into(),
        diagnostics: Vec::new(),
    });
    assert!(state
        .dispatch(Action::RegenFinished {
            tag,
            completion: Box::new(report)
        })
        .is_empty());
    assert!(
        matches!(&state.status, CommitStatus::Completed { outcome: FeatureOutcome::Failed(message), rollback_token: Some(token), .. } if message == "kernel refused" && token == "guard")
    );
    assert_eq!(state.tool.as_ref().unwrap().phase, SessionPhase::Completed);
}

#[test]
fn per_feature_repair_and_sibling_failure_are_not_reported_as_generic_success() {
    for own_repair in [true, false] {
        let (mut state, tag) = confirmed();
        state.dispatch(Action::CommitAccepted {
            tag: tag.clone(),
            revision: 5,
            rollback_token: None,
        });
        let mut report = completion(&tag, 5);
        if own_repair {
            report.repair_steps.push(tag.record_id.to_string());
        } else {
            report.failed_steps.push(FailedStep {
                record_id: "sibling".into(),
                message: "other failed".into(),
                diagnostics: Vec::new(),
            });
        }
        state.dispatch(Action::RegenFinished {
            tag,
            completion: Box::new(report),
        });
        let CommitStatus::Completed {
            outcome,
            completion,
            ..
        } = &state.status
        else {
            panic!()
        };
        assert_eq!(
            outcome,
            &if own_repair {
                FeatureOutcome::NeedsRepair
            } else {
                FeatureOutcome::Published
            }
        );
        assert_eq!(completion.failed_steps.len(), usize::from(!own_repair));
    }
}

#[test]
fn committed_extrusion_edit_uses_apply_operation_without_auto_hiding_or_rollback() {
    let (mut state, tag) = editing();
    assert!(matches!(
        evaluate(&mut state, &tag, 30.0).as_slice(),
        [Effect::PreviewExistingEdit { .. }]
    ));
    edit_unavailable(&mut state, &tag);
    assert_eq!(
        state.tool.as_ref().unwrap().preview,
        PreviewStatus::UnavailableForEdit
    );
    let effects = state.dispatch(Action::Confirm);
    let Effect::ApplyOperation {
        command,
        hide_sketch,
        ..
    } = &effects[1]
    else {
        panic!()
    };
    let EditCommand::UpdateOperationParams { record: id, op } = command.as_ref() else {
        panic!()
    };
    assert_eq!(*id, tag.record_id);
    assert!(hide_sketch.is_none());
    let Operation::Known(KnownOperation::Extrude(params)) = op else {
        panic!()
    };
    assert_eq!(params.draft_angle_deg.value, 2.5);
    assert_eq!(params.distance.value, 30.0);
    state.dispatch(Action::CommitAccepted {
        tag: tag.clone(),
        revision: 5,
        rollback_token: None,
    });
    let mut failed = completion(&tag, 5);
    failed.outcome = "failed".into();
    failed.message = Some("edit failed".into());
    assert!(state
        .dispatch(Action::RegenFinished {
            tag,
            completion: Box::new(failed)
        })
        .is_empty());
    assert!(matches!(
        state.status,
        CommitStatus::Completed {
            outcome: FeatureOutcome::Failed(_),
            rollback_token: None,
            ..
        }
    ));
}

#[test]
fn text_focus_owns_undo_even_without_text_history() {
    assert_eq!(undo_route(true, true), UndoRoute::Text);
    assert_eq!(undo_route(true, false), UndoRoute::Text);
    assert_eq!(undo_route(false, true), UndoRoute::Sketch);
    assert_eq!(undo_route(false, false), UndoRoute::Model);
}

#[test]
fn stale_profile_or_preview_snapshot_is_refused() {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let mut selected = selection(context());
    selected.context.revision -= 1;
    state.dispatch(Action::SelectProfile(Box::new(selected)));
    assert!(state
        .dispatch(Action::BeginExtrude {
            record_id: RecordId(Uuid::from_u128(2)),
            gesture: 1,
            step_index: 0
        })
        .is_empty());
    let (mut state, tag) = started();
    evaluate(&mut state, &tag, 10.0);
    let mut result = preview(&tag);
    result.snapshot_id += 1;
    assert!(state
        .dispatch(Action::PreviewFinished {
            tag,
            result: Ok(result)
        })
        .is_empty());
    assert!(matches!(
        state.tool.as_ref().unwrap().preview,
        PreviewStatus::Failed(_)
    ));
}

#[test]
fn new_feature_requires_successful_current_preview_before_confirmation() {
    let (mut state, tag) = started();
    evaluate(&mut state, &tag, 10.0);
    assert!(state.dispatch(Action::Confirm).is_empty());
    state.dispatch(Action::PreviewFinished {
        tag: tag.clone(),
        result: Err("kernel refused".into()),
    });
    assert!(state.dispatch(Action::Confirm).is_empty());
    let tag = evaluation_tag(&state.dispatch(Action::DraftChanged("20".into())));
    evaluate(&mut state, &tag, 20.0);
    let mut result = preview(&tag);
    result.needs_repair.push(Default::default());
    state.dispatch(Action::PreviewFinished {
        tag,
        result: Ok(result),
    });
    assert_eq!(
        state.tool.as_ref().unwrap().preview,
        PreviewStatus::NeedsRepair
    );
    assert!(state.dispatch(Action::Confirm).is_empty());
}

#[test]
fn late_authored_record_fetch_cannot_start_edit_in_new_runtime() {
    let (mut state, _) = started();
    let record = state.tool.as_ref().unwrap().record.clone();
    let mut next = context();
    next.runtime_session = "new-runtime".into();
    state.dispatch(Action::ContextChanged(Some(next)));
    assert!(state
        .dispatch(Action::BeginEdit {
            record: Box::new(record),
            gesture: 10,
            context: context(),
        })
        .is_empty());
    assert!(state.tool.is_none());
}
