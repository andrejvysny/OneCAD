#[allow(dead_code)]
#[path = "../src/sketch/test_support.rs"]
mod support;

use onecad_core::{
    document::variables::Scalar,
    edit::SketchEditOp,
    math::Vec2,
    sketch::{Constraint, SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::*;
use support::*;

fn dimensions(
    state: &mut SketchInteraction,
    value: CreationDimensions,
    create: bool,
) -> Vec<SketchEffect> {
    let session = state.session.as_ref().unwrap();
    let key = session.key.clone();
    let capture = session.accepted_capture.unwrap();
    let dimensions = Box::new(value);
    state.dispatch(if create {
        SketchAction::CreateNumeric {
            key,
            capture,
            ids: ids(3000),
            dimensions,
        }
    } else {
        SketchAction::PreviewNumeric {
            key,
            capture,
            dimensions,
        }
    })
}

fn staged(through: [f64; 2]) -> SketchInteraction {
    let mut state = active(SketchPlane::xy(), SketchTool::ThreePointArc);
    click(&mut state, 0.0, 0.0, 500);
    click(&mut state, 10.0, 0.0, 500);
    let cursor = point(&state, through[0], through[1]);
    state.dispatch(SketchAction::Move(cursor));
    state
}

fn arc_preview(state: &SketchInteraction) -> arc_geometry::AnalyticArc {
    match state.session.as_ref().unwrap().preview.as_ref().unwrap() {
        DraftPrimitive::Arc { arc } => *arc,
        other => panic!("expected arc, got {other:?}"),
    }
}

fn role_point(arc: arc_geometry::AnalyticArc, start: bool) -> Vec2 {
    let angle = if start != arc.clockwise {
        arc.start_angle
    } else {
        arc.end_angle
    };
    Vec2::new_unchecked(
        arc.center.x + arc.radius * angle.cos(),
        arc.center.y + arc.radius * angle.sin(),
    )
}

#[test]
fn three_point_radius_preserves_endpoints_center_side_and_branch() {
    for through in [[5.0, 2.0], [5.0, -2.0], [5.0, 12.0], [5.0, -12.0]] {
        let mut state = staged(through);
        let before = state.session.as_ref().unwrap().authoritative.clone();
        let selected = arc_preview(&state);
        let value = CreationDimensions::ThreePointArc {
            radius: Scalar::new(8.0),
        };
        assert!(dimensions(&mut state, value.clone(), false).is_empty());
        let preview = arc_preview(&state);
        assert_eq!(state.session.as_ref().unwrap().authoritative, before);
        assert_eq!(preview.clockwise, selected.clockwise);
        assert_eq!(
            preview.sweep > std::f64::consts::PI,
            selected.sweep > std::f64::consts::PI
        );
        assert_eq!(preview.center.y.signum(), selected.center.y.signum());
        assert!((preview.radius - 8.0).abs() < 1e-12);
        let a = role_point(preview, true);
        let b = role_point(preview, false);
        assert!(a.x.hypot(a.y) < 1e-12);
        assert!((b.x - 10.0).hypot(b.y) < 1e-12);
        let (_, ops) = upsert(dimensions(&mut state, value, true));
        assert_eq!(ops.len(), 3);
        assert!(ops.iter().any(|op| matches!(op,
            SketchEditOp::AddEntity { entity: SketchEntity::Arc { radius, start_angle, end_angle, .. } }
            if (*radius - preview.radius).abs() < 1e-12 && *start_angle == preview.start_angle && *end_angle == preview.end_angle)));
        assert!(ops.iter().any(|op| matches!(op,
            SketchEditOp::AddConstraint { constraint: Constraint::Radius { value, .. } } if value.value == 8.0)));
    }
}

#[test]
fn three_point_radius_half_chord_boundary_and_invalid_values() {
    for radius in [0.0, -8.0, f64::NAN, f64::INFINITY, 4.999999999] {
        let mut state = staged([5.0, 2.0]);
        let before = state.session.as_ref().unwrap().authoritative.clone();
        assert!(dimensions(
            &mut state,
            CreationDimensions::ThreePointArc {
                radius: Scalar {
                    value: radius,
                    expr: None
                }
            },
            true
        )
        .is_empty());
        let session = state.session.as_ref().unwrap();
        assert!(matches!(session.status, SketchStatus::Refused(_)));
        assert_eq!(session.authoritative, before);
        assert_eq!(session.phase, SketchPhase::Active);
    }
    let mut state = staged([5.0, 2.0]);
    let draft = state.session.as_ref().unwrap().draft.as_ref().unwrap();
    let end = draft.secondary.as_ref().unwrap().point;
    let half = (end.x - draft.anchor.x).hypot(end.y - draft.anchor.y) * 0.5;
    dimensions(
        &mut state,
        CreationDimensions::ThreePointArc {
            radius: Scalar::new(half),
        },
        false,
    );
    assert!(
        (arc_preview(&state).sweep - std::f64::consts::PI).abs() < 1e-12,
        "{:?} {:?}",
        state.session.as_ref().unwrap().status,
        arc_preview(&state)
    );
}

#[test]
fn tangent_radius_preserves_sweep_side_and_authors_atomic_constraints() {
    for y in [-5.0, 5.0] {
        let mut state = tangent_chain();
        let cursor = point(&state, 15.0, y);
        state.dispatch(SketchAction::Move(cursor));
        let selected = arc_preview(&state);
        let before = state.session.as_ref().unwrap().authoritative.clone();
        let mut radius = Scalar::new(8.0);
        radius.expr = Some("4 + 4".into());
        let value = CreationDimensions::TangentArc { radius };
        dimensions(&mut state, value.clone(), false);
        let preview = arc_preview(&state);
        assert_eq!(state.session.as_ref().unwrap().authoritative, before);
        assert_eq!(selected.clockwise, preview.clockwise);
        assert!((selected.sweep - preview.sweep).abs() < 1e-12);
        assert!((preview.radius - 8.0).abs() < 1e-12);
        let (tag, ops) = upsert(dimensions(&mut state, value, true));
        assert_eq!(ops.len(), 5);
        assert!(ops.iter().any(|op| matches!(
            op,
            SketchEditOp::AddConstraint {
                constraint: Constraint::Coincident { .. }
            }
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            SketchEditOp::AddConstraint {
                constraint: Constraint::TangentAtPoint { .. }
            }
        )));
        assert!(ops.iter().any(|op| matches!(op, SketchEditOp::AddConstraint { constraint: Constraint::Radius { value, .. } } if value.value == 8.0 && value.expr.as_deref() == Some("4 + 4"))));
        let solved = applied(&state, &ops);
        complete(&mut state, tag, solved);
        let draft = state.session.as_ref().unwrap().draft.as_ref().unwrap();
        let endpoint = role_point(preview, false);
        assert!((draft.anchor.x - endpoint.x).hypot(draft.anchor.y - endpoint.y) < 1e-12);
    }
}

#[test]
fn numeric_arc_refusal_keeps_draft_and_authoritative_geometry() {
    for through in [[0.0, 0.0], [5.0, 0.0], [5.0, 1e-12]] {
        let mut state = staged(through);
        let before = state.session.as_ref().unwrap().authoritative.clone();
        assert!(dimensions(
            &mut state,
            CreationDimensions::ThreePointArc {
                radius: Scalar::new(8.0)
            },
            true
        )
        .is_empty());
        let session = state.session.as_ref().unwrap();
        assert!(matches!(session.status, SketchStatus::Refused(_)));
        assert_eq!(session.authoritative, before);
        assert!(session.draft.is_some());
    }
}

fn tangent_chain() -> SketchInteraction {
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    click(&mut state, 0.0, 0.0, 500);
    let (tag, ops) = upsert(click(&mut state, 10.0, 0.0, 1000));
    let solved = applied(&state, &ops);
    complete(&mut state, tag, solved);
    let key = state.session.as_ref().unwrap().key.clone();
    state.dispatch(SketchAction::SetTangentArc { key, enabled: true });
    state
}

#[test]
fn tangent_numeric_rejects_invalid_radius_and_missing_preceding_entity() {
    for radius in [0.0, -8.0, 1e-12, f64::NAN, f64::INFINITY] {
        let mut state = tangent_chain();
        let cursor = point(&state, 15.0, 5.0);
        state.dispatch(SketchAction::Move(cursor));
        let before = state.session.as_ref().unwrap().authoritative.clone();
        let value = CreationDimensions::TangentArc {
            radius: Scalar {
                value: radius,
                expr: None,
            },
        };
        assert!(dimensions(&mut state, value, true).is_empty());
        let session = state.session.as_ref().unwrap();
        assert!(matches!(session.status, SketchStatus::Refused(_)));
        assert_eq!(session.authoritative, before);
        assert!(session.draft.is_some());
    }
    let mut state = tangent_chain();
    let cursor = point(&state, 15.0, 5.0);
    state.dispatch(SketchAction::Move(cursor));
    let preceding = state
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .preceding
        .unwrap()
        .entity;
    state
        .session
        .as_mut()
        .unwrap()
        .authoritative
        .remove_entity(preceding);
    let before = state.session.as_ref().unwrap().authoritative.clone();
    assert!(dimensions(
        &mut state,
        CreationDimensions::TangentArc {
            radius: Scalar::new(8.0)
        },
        true
    )
    .is_empty());
    let session = state.session.as_ref().unwrap();
    assert!(matches!(session.status, SketchStatus::Refused(_)));
    assert_eq!(session.authoritative, before);
}
