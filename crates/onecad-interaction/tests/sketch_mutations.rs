#[path = "../src/sketch/test_support.rs"]
#[allow(dead_code)]
mod support;
use onecad_core::{
    document::variables::Scalar,
    edit::SketchEditOp,
    ids::{ConstraintId, EntityId},
    sketch::{Constraint, CurvePosition, SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::*;
use support::*;

fn dimension_state() -> (SketchInteraction, ConstraintId) {
    let mut state = active(SketchPlane::xy(), SketchTool::Select);
    let session = state.session.as_mut().unwrap();
    let a = EntityId::new();
    let b = EntityId::new();
    session
        .authoritative
        .add_entity(SketchEntity::point(
            a,
            onecad_core::math::Vec2::new(0.0, 0.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    session
        .authoritative
        .add_entity(SketchEntity::point(
            b,
            onecad_core::math::Vec2::new(80.0, 0.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    let id = ConstraintId::new();
    session
        .authoritative
        .add_constraint(Constraint::HorizontalDistance {
            id,
            point1: a,
            point2: b,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
            value: Scalar::new(80.0),
        })
        .unwrap();
    (state, id)
}

#[test]
fn dimension_intent_keeps_accepted_identity_and_value_until_correlated_solver_admission() {
    let (mut state, id) = dimension_state();
    let session = state.session.as_ref().unwrap();
    let key = session.key.clone();
    let before = session.authoritative.clone();
    let effects = state.dispatch(SketchAction::SetDimension {
        key: key.clone(),
        constraint: id,
        value: Scalar::with_expr(90.0, "width"),
    });
    let [SketchEffect::SetDimension {
        tag,
        constraint,
        value,
    }] = effects.as_slice()
    else {
        panic!("expected strict dimension intent")
    };
    assert_eq!(*constraint, id);
    assert_eq!(value.expr.as_deref(), Some("width"));
    assert_eq!(state.session.as_ref().unwrap().authoritative, before);
    assert!(state
        .dispatch(SketchAction::SetDimension {
            key,
            constraint: id,
            value: Scalar::new(100.0)
        })
        .is_empty());
    state.dispatch(SketchAction::ConstraintRefused {
        tag: tag.clone(),
        message: "Conflicting".into(),
    });
    assert_eq!(state.session.as_ref().unwrap().authoritative, before);
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
}

#[test]
fn repair_intent_cannot_smuggle_geometry_addition_or_a_stale_visit() {
    let (mut state, id) = dimension_state();
    let key = state.session.as_ref().unwrap().key.clone();
    assert!(state
        .dispatch(SketchAction::Repair {
            key: key.clone(),
            ops: vec![SketchEditOp::SetDimension {
                constraint: id,
                value: Scalar::new(90.0)
            }]
        })
        .is_empty());
    let mut stale = key.clone();
    stale.generation += 1;
    assert!(state
        .dispatch(SketchAction::Repair {
            key: stale,
            ops: vec![SketchEditOp::RemoveConstraint { constraint: id }]
        })
        .is_empty());
    let effects = state.dispatch(SketchAction::Repair {
        key,
        ops: vec![SketchEditOp::RemoveConstraint { constraint: id }],
    });
    assert!(matches!(effects.as_slice(),[SketchEffect::Repair{ops,..}] if ops.len()==1));
    assert!(state
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .contains_constraint(id));
}

#[test]
fn construction_mode_marks_new_geometry_without_mutating_accepted_sketch() {
    let mut state = active(SketchPlane::xy(), SketchTool::Circle);
    let key = state.session.as_ref().unwrap().key.clone();
    state.dispatch(SketchAction::SetConstructionMode {
        key,
        construction: true,
    });
    assert!(state
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .entities()
        .is_empty());
    click(&mut state, 0.0, 0.0, 1000);
    let effects = click(&mut state, 3.0, 0.0, 2000);
    let [SketchEffect::Upsert { ops, .. }] = effects.as_slice() else {
        panic!("expected construction batch")
    };
    assert!(ops
        .iter()
        .all(|op| matches!(op,SketchEditOp::AddEntity{entity} if entity.is_construction())));
    assert!(state
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .entities()
        .is_empty());
}
