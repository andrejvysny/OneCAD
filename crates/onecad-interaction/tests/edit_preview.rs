//! Existing edit replay classification and complete request-identity fencing.
#[allow(dead_code)]
mod support;
use onecad_app::{
    api::{EditPreview, EditPreviewOutcome},
    error::ApiError,
};
use onecad_core::ids::RecordId;
use onecad_interaction::*;
use support::*;

fn result(tag: &RequestTag, outcome: EditPreviewOutcome) -> EditPreview {
    EditPreview {
        record: tag.record_id,
        edited_step: Some(2),
        snapshot_id: tag.context.snapshot_id,
        outcome,
        bodies: Vec::new(),
        meshes: Vec::new(),
        suppressed: Vec::new(),
        needs_repair: Vec::new(),
        step_signatures: Vec::new(),
        isolated: false,
    }
}

fn pending() -> (Interaction, RequestTag) {
    let (mut state, tag) = editing();
    let effects = evaluate(&mut state, &tag, 8.0);
    assert!(
        matches!(effects.as_slice(), [Effect::PreviewExistingEdit { tag: sent, .. }] if sent == &tag)
    );
    assert!(state.dispatch(Action::Confirm).is_empty());
    (state, tag)
}

#[test]
fn ready_edit_and_downstream_warning_allow_one_apply_but_own_failure_blocks() {
    for outcome in [
        EditPreviewOutcome::Ready,
        EditPreviewOutcome::DownstreamFailed {
            step: 3,
            record: RecordId::new(),
            message: "dependent failed".into(),
        },
        EditPreviewOutcome::DownstreamNeedsRepair {
            step: 3,
            record: RecordId::new(),
        },
        EditPreviewOutcome::EditedFailed {
            message: "edited feature failed".into(),
            diagnostics: Vec::new(),
            needs_repair: false,
        },
    ] {
        let allowed = !matches!(outcome, EditPreviewOutcome::EditedFailed { .. });
        let warning = matches!(
            outcome,
            EditPreviewOutcome::DownstreamFailed { .. }
                | EditPreviewOutcome::DownstreamNeedsRepair { .. }
        );
        let (mut state, tag) = pending();
        let effects = state.dispatch(Action::EditPreviewFinished {
            tag: tag.clone(),
            result: Ok(result(&tag, outcome)),
        });
        assert!(matches!(
            effects.as_slice(),
            [Effect::PublishPreview { .. }]
        ));
        assert_eq!(state.tool.as_ref().unwrap().ready(), allowed);
        assert_eq!(
            matches!(
                state.tool.as_ref().unwrap().preview,
                PreviewStatus::EditWarning(_)
            ),
            warning
        );
        let effects = state.dispatch(Action::Confirm);
        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, Effect::ApplyOperation { .. }))
                .count(),
            usize::from(allowed)
        );
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
}

#[test]
fn unsupported_and_unreached_edits_allow_apply_without_replacing_geometry() {
    for unsupported in [true, false] {
        let (mut state, tag) = pending();
        let result = if unsupported {
            Err(ApiError::Unsupported("plan.preview unavailable".into()))
        } else {
            Ok(result(
                &tag,
                EditPreviewOutcome::NotReached { ceiling: None },
            ))
        };
        assert!(state
            .dispatch(Action::EditPreviewFinished { tag, result })
            .is_empty());
        assert_eq!(
            state.tool.as_ref().unwrap().preview,
            PreviewStatus::UnavailableForEdit
        );
        assert!(state.tool.as_ref().unwrap().ready());
    }
}

#[test]
fn worker_and_invalid_command_errors_never_masquerade_as_unsupported() {
    for error in [
        ApiError::Worker("worker failed".into()),
        ApiError::InvalidCommand("invalid edit".into()),
    ] {
        let (mut state, tag) = pending();
        assert!(state
            .dispatch(Action::EditPreviewFinished {
                tag,
                result: Err(error)
            })
            .is_empty());
        assert!(matches!(
            state.tool.as_ref().unwrap().preview,
            PreviewStatus::Failed(_)
        ));
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
}

#[test]
fn all_request_fences_reject_late_replies_and_create_results_cannot_complete_edits() {
    for field in 0..7 {
        let (mut state, tag) = pending();
        let mut stale = tag.clone();
        match field {
            0 => stale.sequence += 1,
            1 => stale.gesture += 1,
            2 => stale.record_id = RecordId::new(),
            3 => stale.context.revision += 1,
            4 => stale.context.runtime_session.push_str("-old"),
            5 => stale.context.snapshot_id.0 += 1,
            _ => stale.context.scene.scene += 1,
        }
        assert!(state
            .dispatch(Action::EditPreviewFinished {
                result: Ok(result(&stale, EditPreviewOutcome::Ready)),
                tag: stale
            })
            .is_empty());
        assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Pending);
        assert!(state
            .dispatch(Action::PreviewFinished {
                tag: tag.clone(),
                result: Ok(preview(&tag))
            })
            .is_empty());
        assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Pending);
    }
}

#[test]
fn old_sequence_cancelled_results_and_cancel_never_publish_geometry() {
    let (mut state, old) = pending();
    let new = evaluation_tag(&state.dispatch(Action::DraftChanged("12".into())));
    evaluate(&mut state, &new, 12.0);
    assert!(state
        .dispatch(Action::EditPreviewFinished {
            tag: old.clone(),
            result: Ok(result(&old, EditPreviewOutcome::Ready))
        })
        .is_empty());
    for error in [
        ApiError::StalePreview("moved".into()),
        ApiError::Cancelled("preempted".into()),
    ] {
        assert!(state
            .dispatch(Action::EditPreviewFinished {
                tag: new.clone(),
                result: Err(error)
            })
            .is_empty());
        assert!(!state.tool.as_ref().unwrap().ready());
    }
    state.dispatch(Action::Cancel);
    assert!(state
        .dispatch(Action::EditPreviewFinished {
            tag: new.clone(),
            result: Ok(result(&new, EditPreviewOutcome::Ready))
        })
        .is_empty());
    assert!(state.tool.is_none());
}

#[test]
fn unchanged_result_retains_committed_geometry_and_mismatched_backend_identity_blocks() {
    let (mut state, tag) = pending();
    assert!(state
        .dispatch(Action::EditPreviewFinished {
            tag: tag.clone(),
            result: Ok(result(&tag, EditPreviewOutcome::Unchanged))
        })
        .is_empty());
    assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Ready);
    for record in [true, false] {
        let (mut state, tag) = pending();
        let mut wrong = result(&tag, EditPreviewOutcome::Ready);
        if record {
            wrong.record = RecordId::new();
        } else {
            wrong.snapshot_id.0 += 1;
        }
        assert!(state
            .dispatch(Action::EditPreviewFinished {
                tag,
                result: Ok(wrong)
            })
            .is_empty());
        assert!(!state.tool.as_ref().unwrap().ready());
    }
}
