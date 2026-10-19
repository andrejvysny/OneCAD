#[path = "../src/sketch/test_support.rs"]
#[allow(dead_code)]
mod support;
use onecad_core::{
    edit::SketchEditOp,
    sketch::{Constraint, SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::{
    inference::{source_for, AcceptedSnap, SnapIntent, SnapKind},
    *,
};
use support::*;

#[test]
fn pointer_without_an_accepted_view_cannot_start_or_move_a_draft() {
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    state.session.as_mut().unwrap().accepted_capture = None;
    let point = point(&state, 1.0, 2.0);
    assert!(state
        .dispatch(SketchAction::Click {
            point: point.clone(),
            ids: ids(1000)
        })
        .is_empty());
    state.dispatch(SketchAction::Move(point));
    let session = state.session.as_ref().unwrap();
    assert!(session.draft.is_none() && session.preview.is_none());
    assert!(session.authoritative.entities().is_empty());
}

fn inferred(
    state: &mut SketchInteraction,
    x: f64,
    y: f64,
    seed: u128,
    intent: SnapIntent,
    kind: SnapKind,
) -> Vec<SketchEffect> {
    let point = point(state, x, y);
    let session = state.session.as_ref().unwrap();
    let snap = AcceptedSnap {
        source: source_for(
            &session.authoritative,
            point.capture,
            session.request.sequence,
        ),
        point: point.point,
        intent,
        kind,
    };
    state.dispatch(SketchAction::ClickInferred {
        point,
        ids: ids(seed),
        snap: Some(Box::new(snap)),
    })
}

#[test]
fn origin_anchor_is_authored_once_and_continuation_uses_solved_coordinates() {
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    assert!(inferred(
        &mut state,
        0.0,
        0.0,
        1000,
        SnapIntent::Origin,
        SnapKind::Origin
    )
    .is_empty());
    let (tag, ops) = upsert(click(&mut state, 10.0, 0.0, 2000));
    assert!(
        matches!(ops.last(), Some(SketchEditOp::AddConstraint { constraint: Constraint::Fixed { point, .. } }) if *point == ids(2000).entities[0])
    );
    let mut snapshot = applied(&state, &ops);
    let last = ids(2000).entities[1];
    snapshot.sketch.remove_entity(last).unwrap();
    snapshot
        .sketch
        .add_entity(SketchEntity::point(
            last,
            onecad_core::math::Vec2::new(12.0, 1.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    complete(&mut state, tag, snapshot);
    let draft = state.session.as_ref().unwrap().draft.as_ref().unwrap();
    assert_eq!((draft.anchor.x, draft.anchor.y), (12.0, 1.0));
    assert!(draft.anchor_snap.is_none());
    let (_, next) = upsert(click(&mut state, 20.0, 1.0, 3000));
    assert!(!next
        .iter()
        .any(|op| matches!(op, SketchEditOp::AddConstraint { .. })));
}

#[test]
fn stale_inference_and_old_view_click_leave_accepted_sketch_unchanged() {
    let mut state = active(SketchPlane::xy(), SketchTool::Circle);
    let point = point(&state, 0.0, 0.0);
    let session = state.session.as_ref().unwrap();
    let mut snap = AcceptedSnap {
        source: source_for(
            &session.authoritative,
            point.capture,
            session.request.sequence,
        ),
        point: point.point,
        intent: SnapIntent::Origin,
        kind: SnapKind::Origin,
    };
    snap.source.generation += 1;
    let before = session.authoritative.clone();
    assert!(state
        .dispatch(SketchAction::ClickInferred {
            point: point.clone(),
            ids: ids(1000),
            snap: Some(Box::new(snap))
        })
        .is_empty());
    assert!(state.session.as_ref().unwrap().draft.is_none());
    let mut fresh = point.capture;
    fresh.view += 1;
    let session = state.session.as_ref().unwrap();
    state.dispatch(SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: fresh,
    });
    assert!(state
        .dispatch(SketchAction::Click {
            point,
            ids: ids(2000)
        })
        .is_empty());
    assert_eq!(state.session.as_ref().unwrap().authoritative, before);
    assert!(state.session.as_ref().unwrap().draft.is_none());
}
