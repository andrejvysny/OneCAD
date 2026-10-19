#[allow(dead_code)]
#[path = "../src/sketch/test_support.rs"]
mod support;
use onecad_core::sketch::SketchPlane;
use onecad_interaction::sketch::*;
use support::*;

fn finishing() -> (SketchInteraction, SketchTag) {
    let mut state = active(SketchPlane::xy(), SketchTool::Select);
    let effects = state.dispatch(SketchAction::Finish);
    let SketchEffect::FinishSketch { tag, .. } = &effects[0] else {
        panic!("Finish effect")
    };
    (state, tag.clone())
}

#[test]
fn known_finish_refusal_reenters_new_service_owned_visit_without_document_restore() {
    let (mut state, old) = finishing();
    state.dispatch(SketchAction::FinishRefused {
        tag: old.clone(),
        message: "profile overlaps".into(),
        diagnostics: vec![],
    });
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.phase, SketchPhase::Faulted);
    assert!(session.repair_reentry);
    assert!(session.edit_session.is_none() && session.history.is_none());
    let key = session.key.clone();
    let document = session.document.clone();
    let effects = state.dispatch(SketchAction::ReenterForRepair {
        key: key.clone(),
        document: document.clone(),
    });
    let [SketchEffect::EnterSketch { tag, created_here }] = effects.as_slice() else {
        panic!("fresh service entry")
    };
    assert!(!created_here);
    assert_ne!(tag.key.generation, key.generation);
    assert_eq!(tag.key.sketch_id, key.sketch_id);
    assert_eq!(tag.document, document);
    let current = tag.clone();
    state.dispatch(SketchAction::FinishRefused {
        tag: old,
        message: "late refusal".into(),
        diagnostics: vec![],
    });
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.request, current);
    assert_eq!(session.phase, SketchPhase::Entering);
    assert!(!session.repair_reentry);
}

#[test]
fn generic_failure_never_claims_closed_service_session_or_allows_reentry() {
    let (mut state, tag) = finishing();
    state.dispatch(SketchAction::Failed {
        tag,
        message: "transport interrupted".into(),
    });
    let session = state.session.as_ref().unwrap();
    assert!(!session.repair_reentry);
    assert!(state
        .dispatch(SketchAction::ReenterForRepair {
            key: session.key.clone(),
            document: session.document.clone()
        })
        .is_empty());
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Faulted);
}

#[test]
fn stale_repair_document_or_visit_cannot_replace_faulted_session() {
    let (mut state, tag) = finishing();
    state.dispatch(SketchAction::FinishRefused {
        tag,
        message: "profile overlaps".into(),
        diagnostics: vec![],
    });
    let session = state.session.as_ref().unwrap();
    let key = session.key.clone();
    let document = session.document.clone();
    let mut stale_key = key.clone();
    stale_key.generation += 1;
    let mut stale_document = document.clone();
    stale_document.revision += 1;
    for (key, document) in [(stale_key, document), (key.clone(), stale_document)] {
        assert!(state
            .dispatch(SketchAction::ReenterForRepair { key, document })
            .is_empty());
    }
    assert_eq!(state.session.as_ref().unwrap().key, key);
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Faulted);
}
