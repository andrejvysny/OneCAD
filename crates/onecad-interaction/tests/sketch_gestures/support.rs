use super::*;

pub fn xy(x: f64, y: f64) -> Vec2 {
    Vec2::new_unchecked(x, y)
}
pub fn eid(slot: usize) -> EntityId {
    ids(1000).entities[slot]
}
pub fn point_target(slot: usize, role: CurvePosition) -> SketchTarget {
    SketchTarget::Point {
        entity: eid(slot),
        role,
    }
}
pub fn state() -> SketchInteraction {
    let mut state = active(SketchPlane::xy(), SketchTool::Select);
    let sketch = &mut state.session.as_mut().unwrap().authoritative;
    for slot in 0..5 {
        sketch
            .add_entity(SketchEntity::point(
                eid(slot),
                xy(slot as f64 * 10.0, 0.0),
                false,
                false,
            ))
            .unwrap();
    }
    for entity in [
        SketchEntity::line(eid(5), eid(0), eid(1), false),
        SketchEntity::circle(eid(6), eid(2), 5.0, false).unwrap(),
        SketchEntity::arc(eid(7), eid(3), 5.0, 0.0, 1.5, false).unwrap(),
        SketchEntity::ellipse(eid(8), eid(4), 5.0, 3.0, 0.0, false).unwrap(),
    ] {
        sketch.add_entity(entity).unwrap();
    }
    state
}

pub fn scope(state: &SketchInteraction) -> GestureScope {
    GestureScope::from_session(state.session.as_ref().unwrap()).unwrap()
}
pub fn baseline(state: &SketchInteraction) -> Box<Sketch> {
    Box::new(state.session.as_ref().unwrap().authoritative.clone())
}
pub fn shifted(state: &SketchInteraction, x: f64) -> Box<Sketch> {
    let mut sketch = baseline(state);
    sketch.apply_solved_positions(&[(eid(0), xy(x, 0.0))]);
    sketch
}

pub fn begin(controller: &mut SketchGestureInteraction, state: &SketchInteraction) -> GestureTag {
    let effects = controller.dispatch(
        GestureAction::Begin {
            scope: scope(state),
            target: point_target(0, CurvePosition::Arbitrary),
            grab: xy(1.0, 2.0),
        },
        state,
    );
    let [GestureEffect::Begin { tag, intent }] = effects.as_slice() else {
        panic!("expected Begin")
    };
    assert_eq!(intent.grab, xy(1.0, 2.0));
    assert_eq!(intent.drag_point, eid(0));
    tag.clone()
}

pub fn ready(
    controller: &mut SketchGestureInteraction,
    state: &SketchInteraction,
    tag: &GestureTag,
) -> Vec<GestureEffect> {
    controller.dispatch(
        GestureAction::Ready {
            tag: tag.clone(),
            worker_id: 7,
            preview: baseline(state),
        },
        state,
    )
}
pub fn movement(
    controller: &mut SketchGestureInteraction,
    state: &SketchInteraction,
    tag: &GestureTag,
    x: f64,
) -> Vec<GestureEffect> {
    controller.dispatch(
        GestureAction::Move {
            tag: tag.clone(),
            point: xy(x, 0.0),
        },
        state,
    )
}
pub fn update_tag(effects: Vec<GestureEffect>, x: f64) -> GestureTag {
    let [GestureEffect::Update {
        tag,
        worker_id,
        point,
    }] = effects.as_slice()
    else {
        panic!("expected Update {effects:?}")
    };
    assert_eq!(*worker_id, 7);
    assert_eq!(*point, xy(x, 0.0));
    tag.clone()
}
pub fn end_tag(effects: Vec<GestureEffect>, x: f64) -> GestureTag {
    let [GestureEffect::End {
        tag,
        worker_id,
        point,
    }] = effects.as_slice()
    else {
        panic!("expected End {effects:?}")
    };
    assert_eq!(*worker_id, 7);
    assert_eq!(*point, xy(x, 0.0));
    tag.clone()
}
pub fn updated(
    controller: &mut SketchGestureInteraction,
    state: &SketchInteraction,
    tag: &GestureTag,
    seq: u64,
    x: f64,
) -> Vec<GestureEffect> {
    controller.dispatch(
        GestureAction::Updated {
            tag: tag.clone(),
            worker_id: 7,
            worker_sequence: seq,
            preview: shifted(state, x),
        },
        state,
    )
}
pub fn cancel_tag(effects: Vec<GestureEffect>) -> GestureTag {
    let [GestureEffect::Cancel { tag, .. }] = effects.as_slice() else {
        panic!("expected Cancel")
    };
    tag.clone()
}
