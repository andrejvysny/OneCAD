//! A parked document tab drops its tool's in-flight candidates; Republish
//! re-issues the unchanged draft so the tool survives the switch.
use onecad_interaction::*;

#[allow(dead_code)]
mod support;
use support::*;

fn republished(state: &mut Interaction) -> RequestTag {
    let effects = state.dispatch(Action::Republish);
    assert!(matches!(effects.first(), Some(Effect::ClearPreview { .. })));
    evaluation_tag(&effects)
}

#[test]
fn republish_re_evaluates_the_same_draft_under_a_new_sequence() {
    let (mut state, old) = started();
    let draft = state.tool.as_ref().unwrap().draft.clone();
    let tag = republished(&mut state);
    assert!(tag.sequence > old.sequence);
    assert_eq!(
        (&tag.context, tag.record_id, tag.gesture),
        (&old.context, old.record_id, old.gesture)
    );
    let tool = state.tool.as_ref().unwrap();
    assert_eq!(tool.draft, draft);
    assert_eq!(tool.draft_status, DraftStatus::Pending);
    assert_eq!(tool.preview, PreviewStatus::Empty);
    assert_eq!(tool.phase, SessionPhase::Editing);
}

#[test]
fn the_cancelled_request_is_ignored_and_the_republished_one_reaches_confirm() {
    let (mut state, old) = started();
    let tag = republished(&mut state);
    assert!(evaluate(&mut state, &old, 20.0).is_empty(), "stale tag");
    assert_eq!(
        state.tool.as_ref().unwrap().draft_status,
        DraftStatus::Pending
    );
    let effects = evaluate(&mut state, &tag, 20.0);
    assert!(matches!(effects.as_slice(), [Effect::Preview { tag: t, .. }] if t == &tag));
    state.dispatch(Action::PreviewFinished {
        tag: old.clone(),
        result: Ok(preview(&old)),
    });
    assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Pending);
    state.dispatch(Action::PreviewFinished {
        tag: tag.clone(),
        result: Ok(preview(&tag)),
    });
    assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Ready);
    let effects = state.dispatch(Action::Confirm);
    assert!(matches!(
        effects.as_slice(),
        [Effect::ClearPreview { .. }, Effect::ApplyOperation { tag: t, .. }] if t == &tag
    ));
}

#[test]
fn a_ready_preview_must_be_rebuilt_before_confirm() {
    let (mut state, tag) = started();
    evaluate(&mut state, &tag, 20.0);
    state.dispatch(Action::PreviewFinished {
        tag: tag.clone(),
        result: Ok(preview(&tag)),
    });
    republished(&mut state);
    assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Empty);
    assert!(state.dispatch(Action::Confirm).is_empty());
}

#[test]
fn a_commit_in_flight_or_an_absent_tool_is_left_alone() {
    let (mut state, tag) = confirmed();
    assert!(state.dispatch(Action::Republish).is_empty());
    assert_eq!(state.tool.as_ref().unwrap().phase, SessionPhase::Committing);
    assert!(matches!(&state.status, CommitStatus::Pending(pending) if pending == &tag));

    let mut idle = Interaction::default();
    idle.dispatch(Action::ContextChanged(Some(context())));
    assert!(idle.dispatch(Action::Republish).is_empty());
    assert!(idle.tool.is_none());
}

#[test]
fn an_empty_draft_has_nothing_to_republish() {
    let (mut state, _) = started();
    state.dispatch(Action::DraftChanged(String::new()));
    let sequence = state.tool.as_ref().unwrap().tag.sequence;
    assert!(state.dispatch(Action::Republish).is_empty());
    assert_eq!(state.tool.as_ref().unwrap().tag.sequence, sequence);
}

#[test]
fn an_existing_feature_edit_republishes_a_replay_and_can_fall_back_to_apply() {
    let (mut state, _) = editing();
    let tag = republished(&mut state);
    assert!(matches!(
        evaluate(&mut state, &tag, 20.0).as_slice(),
        [Effect::PreviewExistingEdit { .. }]
    ));
    edit_unavailable(&mut state, &tag);
    assert_eq!(
        state.tool.as_ref().unwrap().preview,
        PreviewStatus::UnavailableForEdit
    );
    let effects = state.dispatch(Action::Confirm);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::ApplyOperation { .. })));
}
