#[allow(dead_code)]
#[path = "../src/sketch/test_support.rs"]
mod support;
use onecad_core::{
    ids::EntityId,
    math::Vec2,
    sketch::{CurvePosition, SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::*;
use support::*;

fn state() -> SketchInteraction {
    let mut state = active(SketchPlane::xy(), SketchTool::Select);
    let sketch = &mut state.session.as_mut().unwrap().authoritative;
    let entities = ids(100).entities;
    for (id, at) in [(entities[0], [0.0, 0.0]), (entities[1], [10.0, 0.0])] {
        sketch
            .add_entity(SketchEntity::point(
                id,
                Vec2::new(at[0], at[1]).unwrap(),
                false,
                false,
            ))
            .unwrap();
    }
    sketch
        .add_entity(SketchEntity::line(
            entities[2],
            entities[0],
            entities[1],
            false,
        ))
        .unwrap();
    sketch
        .add_entity(
            SketchEntity::arc(
                entities[3],
                entities[0],
                5.0,
                0.0,
                std::f64::consts::FRAC_PI_2,
                false,
            )
            .unwrap(),
        )
        .unwrap();
    accept_view(&mut state, 1);
    state
}

fn accept_view(state: &mut SketchInteraction, number: u64) {
    let capture = onecad_scene::ViewSnapshot::new(
        context().scene,
        number,
        view(state).camera(),
        view(state).viewport(),
    )
    .unwrap()
    .identity();
    let session = state.session.as_ref().unwrap();
    state.dispatch(SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture,
    });
}

fn select(state: &mut SketchInteraction, targets: Vec<SketchTarget>, mode: SelectionMode) {
    let session = state.session.as_ref().unwrap();
    state.dispatch(SketchAction::Select {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: session.accepted_capture.unwrap(),
        targets,
        mode,
    });
}

fn target(entity: EntityId, role: CurvePosition) -> SketchTarget {
    SketchTarget::Point { entity, role }
}

#[test]
fn point_roles_preserve_owner_and_resolve_only_at_constraint_boundary() {
    let state = state();
    let sketch = &state.session.as_ref().unwrap().authoritative;
    let entities = ids(100).entities;
    assert_eq!(
        target(entities[2], CurvePosition::End).owner_id(),
        entities[2]
    );
    assert_eq!(
        target(entities[2], CurvePosition::End).constraint_point(sketch),
        Some((entities[1], CurvePosition::Arbitrary))
    );
    for (role, expected) in [
        (CurvePosition::Start, [5.0, 0.0]),
        (CurvePosition::End, [0.0, 5.0]),
        (CurvePosition::Center, [0.0, 0.0]),
    ] {
        let point = target(entities[3], role);
        assert_eq!(point.constraint_point(sketch), Some((entities[3], role)));
        let at = point.point_coordinates(sketch).unwrap();
        assert!((at.x - expected[0]).abs() < 1e-12 && (at.y - expected[1]).abs() < 1e-12);
    }
    assert!(!target(entities[3], CurvePosition::Arbitrary).is_valid(sketch));
    assert!(!target(entities[2], CurvePosition::Center).is_valid(sketch));
    assert!(!target(entities[0], CurvePosition::Start).is_valid(sketch));
    assert!(target(entities[0], CurvePosition::Arbitrary).is_valid(sketch));
    assert_ne!(
        SketchTarget::Entity(entities[3]),
        target(entities[3], CurvePosition::Start)
    );
}

#[test]
fn ordered_operands_deduplicate_and_toggle_without_sorting() {
    let mut state = state();
    let entities = ids(100).entities;
    let a = target(entities[2], CurvePosition::End);
    let b = target(entities[3], CurvePosition::Start);
    let axis = SketchTarget::Entity(entities[2]);
    select(&mut state, vec![b, a, axis, a], SelectionMode::Replace);
    assert_eq!(state.session.as_ref().unwrap().selection, vec![b, a, axis]);
    select(&mut state, vec![a, a], SelectionMode::Toggle);
    assert_eq!(state.session.as_ref().unwrap().selection, vec![b, axis]);
    select(&mut state, vec![a], SelectionMode::Add);
    assert_eq!(state.session.as_ref().unwrap().selection, vec![b, axis, a]);
    select(&mut state, vec![axis], SelectionMode::Subtract);
    assert_eq!(state.session.as_ref().unwrap().selection, vec![b, a]);
}

#[test]
fn invalid_role_or_overflow_rejects_entire_candidate() {
    let mut state = state();
    let entity = ids(100).entities[2];
    let valid = SketchTarget::Entity(entity);
    select(&mut state, vec![valid], SelectionMode::Replace);
    select(
        &mut state,
        vec![
            target(entity, CurvePosition::Start),
            target(entity, CurvePosition::Center),
        ],
        SelectionMode::Replace,
    );
    assert_eq!(state.session.as_ref().unwrap().selection, vec![valid]);
    select(
        &mut state,
        vec![valid; MAX_SELECTION_TARGETS + 1],
        SelectionMode::Replace,
    );
    assert_eq!(state.session.as_ref().unwrap().selection, vec![valid]);
}

#[test]
fn revision_visit_and_captured_view_fences_reject_stale_actions() {
    let mut state = state();
    let session = state.session.as_ref().unwrap();
    let key = session.key.clone();
    let document = session.document.clone();
    let capture = session.accepted_capture.unwrap();
    let target = SketchTarget::Entity(ids(100).entities[2]);
    select(&mut state, vec![target], SelectionMode::Replace);
    let mut revision = document.clone();
    revision.revision += 1;
    let mut visit = key.clone();
    visit.generation += 1;
    for (key, document) in [(key.clone(), revision), (visit, document.clone())] {
        state.dispatch(SketchAction::Select {
            key,
            document,
            capture,
            targets: vec![],
            mode: SelectionMode::Replace,
        });
        assert_eq!(state.session.as_ref().unwrap().selection, vec![target]);
    }
    accept_view(&mut state, 2);
    state.dispatch(SketchAction::Select {
        key: key.clone(),
        document: document.clone(),
        capture,
        targets: vec![],
        mode: SelectionMode::Replace,
    });
    state.dispatch(SketchAction::AcceptView {
        key,
        document,
        capture,
    });
    assert_eq!(
        state
            .session
            .as_ref()
            .unwrap()
            .accepted_capture
            .unwrap()
            .view,
        2
    );
    assert_eq!(state.session.as_ref().unwrap().selection, vec![target]);
}

#[test]
fn escape_cancels_marquee_before_selection_and_refresh_invalidates_it() {
    let mut state = state();
    let target = SketchTarget::Entity(ids(100).entities[2]);
    select(&mut state, vec![target], SelectionMode::Replace);
    let session = state.session.as_ref().unwrap();
    state.dispatch(SketchAction::SelectionDrag {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: session.accepted_capture.unwrap(),
        active: true,
    });
    assert!(state.dispatch(SketchAction::Escape).is_empty());
    assert_eq!(state.session.as_ref().unwrap().selection, vec![target]);
    assert!(!state.session.as_ref().unwrap().selection_dragging);
    let mut document = context();
    document.revision += 1;
    state.dispatch(SketchAction::ContextChanged(Some(document)));
    assert!(state.session.as_ref().unwrap().selection.is_empty());
    assert!(state.session.as_ref().unwrap().accepted_capture.is_none());
}

#[test]
fn authoritative_history_removal_invalidates_whole_and_point_role_selection() {
    let mut state = state();
    let arc = ids(100).entities[3];
    select(
        &mut state,
        vec![SketchTarget::Entity(arc), target(arc, CurvePosition::End)],
        SelectionMode::Replace,
    );
    let effects = state.dispatch(SketchAction::History(
        onecad_core::edit::HistoryDirection::Undo,
    ));
    let SketchEffect::History { tag, .. } = &effects[0] else {
        panic!("history effect")
    };
    let mut sketch = state.session.as_ref().unwrap().authoritative.clone();
    sketch.remove_entity(arc).unwrap();
    state.dispatch(SketchAction::HistoryResolved {
        tag: tag.clone(),
        snapshot: SketchSnapshot {
            history: None,
            document: context(),
            sketch: Box::new(sketch.clone()),
        },
        solve: Box::new(enter_dto(sketch.id)),
        moved: true,
        direction: onecad_core::edit::HistoryDirection::Undo,
    });
    assert!(state.session.as_ref().unwrap().selection.is_empty());
    assert!(
        !target(arc, CurvePosition::End).is_valid(&state.session.as_ref().unwrap().authoritative)
    );
}
