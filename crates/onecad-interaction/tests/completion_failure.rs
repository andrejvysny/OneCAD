use onecad_interaction::*;

#[allow(dead_code)]
mod support;
use support::*;

#[test]
fn missing_completion_preserves_acceptance_and_never_repeats_authoring() {
    let (mut state, tag) = confirmed();
    state.dispatch(Action::CommitAccepted {
        tag: tag.clone(),
        revision: 5,
        rollback_token: Some("guarded-receipt".into()),
    });
    assert!(state
        .dispatch(Action::CompletionUnavailable {
            tag: tag.clone(),
            message: "Worker disconnected".into(),
        })
        .is_empty());
    assert!(
        matches!(&state.status, CommitStatus::CompletionUnavailable {
        accepted_revision: 5, rollback_token: Some(receipt), message, ..
    } if receipt == "guarded-receipt" && message == "Worker disconnected")
    );
    assert!(state.dispatch(Action::Confirm).is_empty());
    assert!(state
        .dispatch(Action::CommitRejected {
            tag,
            message: "late rejection".into(),
        })
        .is_empty());
    assert!(matches!(
        state.status,
        CommitStatus::CompletionUnavailable { .. }
    ));
}

#[test]
fn missing_completion_from_old_session_cannot_end_current_commit() {
    let (mut state, tag) = confirmed();
    state.dispatch(Action::CommitAccepted {
        tag: tag.clone(),
        revision: 5,
        rollback_token: None,
    });
    let mut stale = tag;
    stale.context.runtime_session = "retired".into();
    state.dispatch(Action::CompletionUnavailable {
        tag: stale,
        message: "Old worker disconnected".into(),
    });
    assert!(matches!(state.status, CommitStatus::Accepted { .. }));
}
