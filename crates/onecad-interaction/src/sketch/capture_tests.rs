use super::support::*;
use onecad_core::{math::Vec3, sketch::SketchPlane};
use onecad_interaction::sketch::*;
use onecad_scene::{Camera, DVec3, Projection, ViewSnapshot};

#[test]
fn grazing_intersection_inside_camera_clip_remains_usable() {
    let state = active(SketchPlane::xy(), SketchTool::Line);
    let session = state.session.as_ref().unwrap();
    let grazing = ViewSnapshot::new(
        session.document.scene,
        2,
        Camera {
            target: DVec3::ZERO,
            offset: DVec3::new(100.0, 0.0, 1e-11),
            projection: Projection::Orthographic,
            ..Camera::default()
        },
        view(&state).viewport(),
    )
    .unwrap();
    let logical = grazing.project(DVec3::ZERO).unwrap().logical;
    let point = capture_sketch_pointer(session, &grazing, logical).unwrap();
    assert!(point.point.is_finite());
    assert!(point.point.x.hypot(point.point.y) < 1e-6);
}

#[test]
fn invalid_captured_plane_never_creates_an_authored_sketch() {
    let mut state = SketchInteraction::default();
    state.dispatch(SketchAction::ContextChanged(Some(context())));
    let mut plane = SketchPlane::xy();
    plane.y_axis = plane.x_axis;
    assert!(state
        .dispatch(SketchAction::Begin {
            sketch: Box::new(empty(plane)),
            create: true,
            context: context()
        })
        .is_empty());
    assert!(state.session.is_none());
    plane = SketchPlane::xy();
    plane.normal = Vec3::new(0.0, 0.0, -1.0).unwrap();
    assert!(state
        .dispatch(SketchAction::Begin {
            sketch: Box::new(empty(plane)),
            create: true,
            context: context()
        })
        .is_empty());
}

#[test]
fn current_fenced_snapshot_adopts_after_document_revision_changes() {
    let mut state = active(SketchPlane::xy(), SketchTool::Circle);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, mut snapshot) = primitive(&mut state, 3.0, 4.0, 2000);
    let mut next = context();
    next.revision += 1;
    state.dispatch(SketchAction::ContextChanged(Some(next.clone())));
    snapshot.document = next;
    complete(&mut state, tag, snapshot);
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
    assert_eq!(
        state
            .session
            .as_ref()
            .unwrap()
            .authoritative
            .entities()
            .len(),
        2
    );
}

#[test]
fn failure_reports_only_proven_acceptance_without_rollback() {
    for accepted in [false, true] {
        let mut state = active(SketchPlane::xy(), SketchTool::Circle);
        click(&mut state, 0.0, 0.0, 1000);
        let (tag, _) = primitive(&mut state, 3.0, 4.0, 2000);
        if accepted {
            state.dispatch(SketchAction::BatchAccepted { tag: tag.clone() });
        }
        assert!(state
            .dispatch(SketchAction::Failed {
                tag,
                message: "solver unavailable".into()
            })
            .is_empty());
        assert!(
            matches!(&state.session.as_ref().unwrap().status,SketchStatus::Failed{accepted:evidence,..}
            if *evidence == accepted.then_some(true))
        );
    }
}

#[test]
fn escape_during_pending_line_does_not_resurrect_chain_on_solver_reply() {
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, snapshot) = primitive(&mut state, 10.0, 0.0, 2000);
    assert!(state.dispatch(SketchAction::Escape).is_empty());
    complete(&mut state, tag, snapshot);
    assert!(state.session.as_ref().unwrap().draft.is_none());
    assert_eq!(
        state
            .session
            .as_ref()
            .unwrap()
            .authoritative
            .entities()
            .len(),
        3
    );
}

#[test]
fn pending_discard_cannot_be_overwritten_by_later_enter() {
    let mut state = active(SketchPlane::xy(), SketchTool::Circle);
    click(&mut state, 0.0, 0.0, 1000);
    let (tag, snapshot) = primitive(&mut state, 3.0, 4.0, 2000);
    state.dispatch(SketchAction::Discard);
    state.dispatch(SketchAction::Enter);
    assert!(matches!(
        complete(&mut state, tag, snapshot).as_slice(),
        [SketchEffect::DiscardSketch { .. }]
    ));
}

#[test]
fn active_head_change_drops_draft_and_blocks_editing_until_fenced_refresh() {
    let mut state = active(SketchPlane::xy(), SketchTool::Rectangle);
    click(&mut state, 0.0, 0.0, 1000);
    let mut next = context();
    next.revision += 1;
    let effects = state.dispatch(SketchAction::ContextChanged(Some(next.clone())));
    let [SketchEffect::RefreshSketch { tag: old }] = effects.as_slice() else {
        panic!()
    };
    assert_eq!(
        state.session.as_ref().unwrap().phase,
        SketchPhase::NeedsRefresh
    );
    assert!(state.session.as_ref().unwrap().draft.is_none());
    assert!(click(&mut state, 30.0, 20.0, 2000).is_empty());
    assert!(state
        .dispatch(SketchAction::ContextChanged(Some(next.clone())))
        .is_empty());
    assert_eq!(state.session.as_ref().unwrap().request, *old);
    next.snapshot_id.0 += 1;
    let effects = state.dispatch(SketchAction::ContextChanged(Some(next.clone())));
    let [SketchEffect::RefreshSketch { tag }] = effects.as_slice() else {
        panic!()
    };
    let sketch = state.session.as_ref().unwrap().authoritative.clone();
    state.dispatch(SketchAction::Refreshed {
        tag: old.clone(),
        snapshot: SketchSnapshot {
            history: None,
            document: next.clone(),
            sketch: Box::new(sketch.clone()),
        },
        solve: Box::new(enter_dto(sketch.id)),
    });
    assert_eq!(
        state.session.as_ref().unwrap().phase,
        SketchPhase::NeedsRefresh
    );
    state.dispatch(SketchAction::Refreshed {
        tag: tag.clone(),
        snapshot: SketchSnapshot {
            history: None,
            document: next,
            sketch: Box::new(sketch.clone()),
        },
        solve: Box::new(enter_dto(sketch.id)),
    });
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
    assert!(click(&mut state, 30.0, 20.0, 3000).is_empty());
    assert!(state.session.as_ref().unwrap().draft.is_none());
    let session = state.session.as_ref().unwrap();
    state.dispatch(SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view(&state).identity(),
    });
    assert!(click(&mut state, 30.0, 20.0, 4000).is_empty());
    assert!(state.session.as_ref().unwrap().draft.is_some());
}
