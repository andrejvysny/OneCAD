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

fn drag(state: &SketchInteraction) -> PrimitiveDrag {
    let session = state.session.as_ref().unwrap();
    PrimitiveDrag {
        start: point(state, 2.0, 3.0),
        end: point(state, 12.0, 13.0),
        start_snap: None,
        end_snap: None,
        tool: session.tool,
        draft_generation: session.draft_generation,
        request_sequence: session.request.sequence,
        construction: session.construction,
        ids: ids(2000),
    }
}

fn commit(state: &mut SketchInteraction, drag: PrimitiveDrag) -> Vec<SketchEffect> {
    state.dispatch(SketchAction::CommitPrimitiveDrag(Box::new(drag)))
}

#[test]
fn each_exact_tool_submits_one_batch_without_mutating_authority_or_prior_draft() {
    for (tool, entities) in [
        (SketchTool::Line, 3),
        (SketchTool::Rectangle, 8),
        (SketchTool::CenterRectangle, 10),
        (SketchTool::Circle, 2),
    ] {
        let mut state = active(SketchPlane::xy(), tool);
        click(&mut state, -10.0, -20.0, 1000);
        let before = state.session.as_ref().unwrap().clone();
        let payload = drag(&state);
        let (tag, ops) = upsert(commit(&mut state, payload.clone()));
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, SketchEditOp::AddEntity { .. }))
                .count(),
            entities
        );
        let session = state.session.as_ref().unwrap();
        assert_eq!(session.authoritative, before.authoritative);
        assert_eq!(
            format!("{:?}", session.draft),
            format!("{:?}", before.draft)
        );
        assert!(
            commit(&mut state, payload).is_empty(),
            "duplicate release must not submit"
        );
        let snapshot = applied(&state, &ops);
        complete(&mut state, tag, snapshot);
        assert!(
            state.session.as_ref().unwrap().draft.is_none(),
            "a lifted primitive must leave the next Undo for its accepted batch"
        );
        let sketch = &state.session.as_ref().unwrap().authoritative;
        assert_eq!(sketch.entities().len(), entities);
        let start = match sketch.get_entity(ids(2000).entities[0]).unwrap() {
            SketchEntity::Point { at, .. } => *at,
            _ => panic!("missing start"),
        };
        if tool != SketchTool::CenterRectangle {
            assert!((start.x - 2.0).abs() < 1e-8 && (start.y - 3.0).abs() < 1e-8);
        }
    }
}

#[test]
fn stale_drag_fences_preserve_existing_pending_draft() {
    for stale in 0..8 {
        let mut state = active(SketchPlane::xy(), SketchTool::Rectangle);
        click(&mut state, -10.0, -20.0, 1000);
        let before = state.session.as_ref().unwrap().clone();
        let mut payload = drag(&state);
        match stale {
            0 => payload.start.capture.view += 1,
            1 => payload.end.capture.scene.scene += 1,
            2 => payload.start.key.generation += 1,
            3 => payload.draft_generation += 1,
            4 => payload.request_sequence += 1,
            5 => payload.tool = SketchTool::Circle,
            6 => payload.construction = true,
            _ => state.context.as_mut().unwrap().revision += 1,
        }
        assert!(commit(&mut state, payload).is_empty(), "fence {stale}");
        let session = state.session.as_ref().unwrap();
        assert_eq!(session.authoritative, before.authoritative);
        assert_eq!(
            format!("{:?}", session.draft),
            format!("{:?}", before.draft)
        );
        assert_eq!(session.phase, SketchPhase::Active);
    }
}

#[test]
fn degenerate_drag_refuses_without_replacing_a_pending_draft() {
    let mut state = active(SketchPlane::xy(), SketchTool::Rectangle);
    click(&mut state, -10.0, -20.0, 1000);
    let before = state.session.as_ref().unwrap().clone();
    let mut payload = drag(&state);
    payload.end.point = payload.start.point;
    assert!(commit(&mut state, payload).is_empty());
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.authoritative, before.authoritative);
    assert_eq!(
        format!("{:?}", session.draft),
        format!("{:?}", before.draft)
    );
    assert!(
        matches!(&session.status, SketchStatus::Refused(message) if message == "Degenerate rectangle")
    );
}

#[test]
fn drag_anchor_inference_uses_the_current_capture_and_construction_mode() {
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    state.session.as_mut().unwrap().construction = true;
    let mut payload = drag(&state);
    payload.start.point = onecad_core::math::Vec2::new(0.0, 0.0).unwrap();
    let session = state.session.as_ref().unwrap();
    payload.start_snap = Some(AcceptedSnap {
        source: source_for(
            &session.authoritative,
            payload.start.capture,
            session.request.sequence,
        ),
        point: payload.start.point,
        intent: SnapIntent::Origin,
        kind: SnapKind::Origin,
    });
    let (_, ops) = upsert(commit(&mut state, payload));
    assert!(ops.iter().any(|op| matches!(
        op,
        SketchEditOp::AddConstraint {
            constraint: Constraint::Fixed { .. }
        }
    )));
    assert!(ops
        .iter()
        .filter_map(|op| match op {
            SketchEditOp::AddEntity { entity } => Some(entity),
            _ => None,
        })
        .all(SketchEntity::is_construction));
}

#[test]
fn stale_endpoint_inference_refuses_the_whole_drag() {
    let mut state = active(SketchPlane::xy(), SketchTool::Circle);
    click(&mut state, -10.0, -20.0, 1000);
    let before = state.session.as_ref().unwrap().clone();
    let mut payload = drag(&state);
    let session = state.session.as_ref().unwrap();
    let mut source = source_for(
        &session.authoritative,
        payload.end.capture,
        session.request.sequence,
    );
    source.generation += 1;
    payload.end_snap = Some(AcceptedSnap {
        source,
        point: payload.end.point,
        intent: SnapIntent::GeometryOnly,
        kind: SnapKind::Grid,
    });
    assert!(commit(&mut state, payload).is_empty());
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.authoritative, before.authoritative);
    assert_eq!(
        format!("{:?}", session.draft),
        format!("{:?}", before.draft)
    );
}
