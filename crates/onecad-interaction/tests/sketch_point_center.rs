#[allow(dead_code)]
#[path = "../src/sketch/test_support.rs"]
mod support;
use onecad_core::{
    document::variables::Scalar,
    edit::SketchEditOp,
    math::Vec2,
    sketch::{Constraint, CurvePosition, SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::{
    inference::{source_for, AcceptedSnap, SnapIntent, SnapKind},
    *,
};
use support::*;

fn inferred(
    state: &mut SketchInteraction,
    xy: [f64; 2],
    seed: u128,
    intent: SnapIntent,
) -> Vec<SketchEffect> {
    let point = point(state, xy[0], xy[1]);
    let session = state.session.as_ref().unwrap();
    let snap = AcceptedSnap {
        source: source_for(
            &session.authoritative,
            point.capture,
            session.request.sequence,
        ),
        point: point.point,
        kind: SnapKind::Origin,
        intent,
    };
    state.dispatch(SketchAction::ClickInferred {
        point,
        ids: ids(seed),
        snap: Some(Box::new(snap)),
    })
}

fn numeric(
    state: &mut SketchInteraction,
    dimensions: CreationDimensions,
    seed: u128,
) -> Vec<SketchEffect> {
    let session = state.session.as_ref().unwrap();
    state.dispatch(SketchAction::CreateNumeric {
        key: session.key.clone(),
        capture: session.accepted_capture.unwrap(),
        ids: ids(seed),
        dimensions: Box::new(dimensions),
    })
}

fn move_to(state: &mut SketchInteraction, xy: [f64; 2]) {
    state.dispatch(SketchAction::Move(point(state, xy[0], xy[1])));
}

fn begin_numeric(state: &mut SketchInteraction) {
    state.dispatch(SketchAction::BeginPointNumeric {
        key: state.session.as_ref().unwrap().key.clone(),
    });
}

#[test]
fn point_clicks_commit_once_and_rearm_only_after_accepted_solve() {
    let mut state = active(SketchPlane::xy(), SketchTool::Point);
    let (tag, ops) = upsert(click(&mut state, 4.0, 5.0, 1000));
    assert_eq!(ops.len(), 1);
    assert!(state
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .entities()
        .is_empty());
    assert!(click(&mut state, 6.0, 7.0, 2000).is_empty());
    let snapshot = applied(&state, &ops);
    complete(&mut state, tag, snapshot);
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.phase, SketchPhase::Active);
    assert!(session.draft.is_none());
    let (tag, ops) = upsert(click(&mut state, 6.0, 7.0, 2000));
    let snapshot = applied(&state, &ops);
    complete(&mut state, tag, snapshot);
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
    assert!(state.session.as_ref().unwrap().draft.is_none());
}

#[test]
fn point_snap_persists_exact_target_once_and_keeps_construction() {
    let mut state = active(SketchPlane::xy(), SketchTool::Point);
    let (tag, ops) = upsert(inferred(&mut state, [0.0, 0.0], 1000, SnapIntent::Origin));
    let snapshot = applied(&state, &ops);
    complete(&mut state, tag, snapshot);
    let target = SketchTarget::Point {
        entity: ids(1000).entities[0],
        role: CurvePosition::Arbitrary,
    };
    state.dispatch(SketchAction::SetConstructionMode {
        key: state.session.as_ref().unwrap().key.clone(),
        construction: true,
    });
    move_to(&mut state, [9.0, 9.0]);
    let (_, ops) = upsert(inferred(
        &mut state,
        [0.0, 0.0],
        2000,
        SnapIntent::Coincident(target),
    ));
    assert_eq!(ops.len(), 2);
    assert!(
        matches!(&ops[0], SketchEditOp::AddEntity { entity: SketchEntity::Point { id, construction: true, .. } } if *id == ids(2000).entities[0])
    );
    assert!(
        matches!(&ops[1], SketchEditOp::AddConstraint { constraint: Constraint::Coincident { point1, point2, .. } } if *point1 == ids(2000).entities[0] && *point2 == ids(1000).entities[0])
    );
}

#[test]
fn point_numeric_seeds_without_pointer_and_keeps_signed_expressions_without_snap() {
    let mut state = active(SketchPlane::xy(), SketchTool::Point);
    begin_numeric(&mut state);
    move_to(&mut state, [9.0, -9.0]);
    assert_eq!(
        state
            .session
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .anchor,
        Vec2::new(0.0, 0.0).unwrap()
    );
    let x = Scalar::with_expr(-15.0, "-edge_offset");
    let y = Scalar::with_expr(0.0, "center_y");
    let (_, ops) = upsert(numeric(
        &mut state,
        CreationDimensions::Point {
            x: x.clone(),
            y: y.clone(),
        },
        1000,
    ));
    assert_eq!(ops.len(), 5);
    assert!(
        matches!(&ops[0], SketchEditOp::AddEntity { entity: SketchEntity::Point { at, construction: false, .. } } if at.x == -15.0 && at.y == 0.0)
    );
    assert!(matches!(
        &ops[1],
        SketchEditOp::AddEntity {
            entity: SketchEntity::Point {
                construction: true,
                ..
            }
        }
    ));
    assert!(ops.iter().any(|op| matches!(op, SketchEditOp::AddConstraint { constraint: Constraint::HorizontalDistance { point1, point2, value, .. } } if *point1 == ids(1000).entities[1] && *point2 == ids(1000).entities[0] && value == &x)));
    assert!(ops.iter().any(|op| matches!(op, SketchEditOp::AddConstraint { constraint: Constraint::VerticalDistance { value, .. } } if value == &y)));
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(
                op,
                SketchEditOp::AddConstraint {
                    constraint: Constraint::Fixed { .. }
                }
            ))
            .count(),
        1
    );
}

#[test]
fn point_numeric_rejects_stale_scope_nonfinite_and_escape_only_cancels_draft() {
    let mut state = active(SketchPlane::xy(), SketchTool::Point);
    let mut key = state.session.as_ref().unwrap().key.clone();
    key.generation += 1;
    state.dispatch(SketchAction::BeginPointNumeric { key });
    assert!(state.session.as_ref().unwrap().draft.is_none());
    let capture = state.session.as_mut().unwrap().accepted_capture.take();
    begin_numeric(&mut state);
    assert!(state.session.as_ref().unwrap().draft.is_none());
    state.session.as_mut().unwrap().accepted_capture = capture;
    begin_numeric(&mut state);
    for bad in [f64::NAN, f64::INFINITY] {
        assert!(numeric(
            &mut state,
            CreationDimensions::Point {
                x: Scalar {
                    value: bad,
                    expr: None
                },
                y: Scalar::new(0.0)
            },
            1000
        )
        .is_empty());
    }
    assert!(state.dispatch(SketchAction::Escape).is_empty());
    let session = state.session.as_ref().unwrap();
    assert!(session.draft.is_none());
    assert_eq!(session.phase, SketchPhase::Active);
    assert!(session.authoritative.entities().is_empty());
}

#[test]
fn centered_rectangle_has_shared_welds_and_persistent_center_in_all_quadrants() {
    for corner in [[10.0, 20.0], [-10.0, 20.0], [-10.0, -20.0], [10.0, -20.0]] {
        let mut state = active(SketchPlane::xy(), SketchTool::CenterRectangle);
        assert!(click(&mut state, 0.0, 0.0, 500).is_empty());
        move_to(&mut state, corner);
        assert!(
            matches!(state.session.as_ref().unwrap().preview, Some(DraftPrimitive::Rectangle { first, opposite }) if (first.x + corner[0]).abs() < 1e-10 && (first.y + corner[1]).abs() < 1e-10 && (opposite.x - corner[0]).abs() < 1e-10 && (opposite.y - corner[1]).abs() < 1e-10)
        );
        let (_, ops) = upsert(click(&mut state, corner[0], corner[1], 1000));
        assert_eq!(ops.len(), 15);
        let snapshot = applied(&state, &ops);
        let sketch = snapshot.sketch;
        assert_eq!(sketch.entities().len(), 10);
        assert_eq!(sketch.constraints().len(), 5);
        for i in 0..4 {
            assert!(
                matches!(sketch.get_entity(ids(1000).entities[i+4]), Some(SketchEntity::Line { start, end, .. }) if *start == ids(1000).entities[i] && *end == ids(1000).entities[(i+1)%4])
            );
        }
        assert!(
            matches!(sketch.get_entity(ids(1000).entities[8]), Some(SketchEntity::Point { construction: true, at, .. }) if at.x == 0.0 && at.y == 0.0)
        );
        assert!(
            matches!(sketch.get_entity(ids(1000).entities[9]), Some(SketchEntity::Line { construction: true, start, end, .. }) if *start == ids(1000).entities[0] && *end == ids(1000).entities[2])
        );
        assert!(
            matches!(sketch.get_constraint(ids(1000).constraints[10]), Some(Constraint::Midpoint { point, line, .. }) if *point == ids(1000).entities[8] && *line == ids(1000).entities[9])
        );
    }
}

#[test]
fn center_anchor_and_corner_snap_map_to_intended_points_with_unique_ids() {
    let mut state = active(SketchPlane::xy(), SketchTool::CenterRectangle);
    assert!(inferred(&mut state, [0.0, 0.0], 500, SnapIntent::Origin).is_empty());
    let target = ids(9000).entities[0];
    state
        .session
        .as_mut()
        .unwrap()
        .authoritative
        .add_entity(SketchEntity::point(
            target,
            Vec2::new_unchecked(40.0, 25.0),
            false,
            false,
        ))
        .unwrap();
    // Capture the anchor against the same geometry as the final click.
    state.session.as_mut().unwrap().draft = None;
    assert!(inferred(&mut state, [0.0, 0.0], 500, SnapIntent::Origin).is_empty());
    let (_, ops) = upsert(inferred(
        &mut state,
        [40.0, 25.0],
        1000,
        SnapIntent::Coincident(SketchTarget::Point {
            entity: target,
            role: CurvePosition::Arbitrary,
        }),
    ));
    assert!(ops.iter().any(|op| matches!(op, SketchEditOp::AddConstraint { constraint: Constraint::Coincident { point1, point2, .. } } if *point1 == ids(1000).entities[2] && *point2 == target)));
    assert!(ops.iter().any(|op| matches!(op, SketchEditOp::AddConstraint { constraint: Constraint::Fixed { point, .. } } if *point == ids(1000).entities[8])));
    assert!(!ops.iter().any(|op| matches!(op, SketchEditOp::AddConstraint { constraint: Constraint::Fixed { point, .. } } if *point == ids(1000).entities[0])));
    let unique: std::collections::BTreeSet<_> = ops
        .iter()
        .filter_map(|op| match op {
            SketchEditOp::AddConstraint { constraint } => Some(constraint.id()),
            _ => None,
        })
        .collect();
    assert_eq!(unique.len(), 7);
}

#[test]
fn centered_numeric_uses_full_sizes_and_expressions_not_half_sizes() {
    let mut state = active(SketchPlane::xy(), SketchTool::CenterRectangle);
    assert!(inferred(&mut state, [0.0, 0.0], 500, SnapIntent::Origin).is_empty());
    move_to(&mut state, [-1.0, -1.0]);
    let width = Scalar::with_expr(80.0, "width");
    let (_, ops) = upsert(numeric(
        &mut state,
        CreationDimensions::CenterRectangle {
            width: width.clone(),
            height: Scalar::new(50.0),
        },
        1000,
    ));
    let sketch = applied(&state, &ops).sketch;
    assert!(
        matches!(sketch.get_entity(ids(1000).entities[2]), Some(SketchEntity::Point { at, .. }) if at.x == -40.0 && at.y == -25.0)
    );
    assert!(
        matches!(sketch.get_constraint(ids(1000).constraints[8]), Some(Constraint::Distance { value, entity1, entity2, .. }) if value == &width && *entity1 == ids(1000).entities[0] && *entity2 == ids(1000).entities[1])
    );
    assert_eq!(sketch.constraints().len(), 8);
}

#[test]
fn centered_degenerate_overflow_stale_capture_and_id_collisions_never_author() {
    let mut state = active(SketchPlane::xy(), SketchTool::CenterRectangle);
    assert!(click(&mut state, 0.0, 0.0, 500).is_empty());
    for corner in [[0.0, 5.0], [5.0, 0.0], [1e-12, 5.0]] {
        assert!(click(&mut state, corner[0], corner[1], 1000).is_empty());
    }
    let mut captured = point(&state, 5.0, 5.0);
    captured.capture.view += 1;
    assert!(state
        .dispatch(SketchAction::Click {
            point: captured,
            ids: ids(1000)
        })
        .is_empty());
    let mut duplicate = ids(1000);
    duplicate.constraints[10] = duplicate.constraints[8];
    assert!(state
        .dispatch(SketchAction::Click {
            point: point(&state, 5.0, 5.0),
            ids: duplicate
        })
        .is_empty());
    let session = state.session.as_mut().unwrap();
    session.draft.as_mut().unwrap().anchor = Vec2::new_unchecked(-f64::MAX, 1.0);
    assert!(state
        .dispatch(SketchAction::Click {
            point: CapturedSketchPoint {
                point: Vec2::new_unchecked(f64::MAX, 5.0),
                ..point(&state, 5.0, 5.0)
            },
            ids: ids(1000)
        })
        .is_empty());
    assert!(state
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .entities()
        .is_empty());
}

#[test]
fn centered_construction_mode_applies_to_outer_entities_and_keeps_helpers_construction() {
    let mut state = active(SketchPlane::xy(), SketchTool::CenterRectangle);
    state.dispatch(SketchAction::SetConstructionMode {
        key: state.session.as_ref().unwrap().key.clone(),
        construction: true,
    });
    assert!(click(&mut state, 0.0, 0.0, 500).is_empty());
    let (_, ops) = upsert(click(&mut state, 40.0, 25.0, 1000));
    let sketch = applied(&state, &ops).sketch;
    assert!(sketch
        .entities()
        .iter()
        .all(|entity| entity.is_construction()));
}
