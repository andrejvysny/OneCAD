#[allow(dead_code)]
#[path = "../src/sketch/test_support.rs"]
mod support;
use onecad_core::{
    document::variables::Scalar,
    edit::SketchEditOp,
    math::Vec2,
    sketch::{Constraint, SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::{
    inference::{source_for, AcceptedSnap, SnapIntent, SnapKind},
    *,
};
use support::*;

fn state(tool: SketchTool) -> SketchInteraction {
    let mut state = active(SketchPlane::xy(), tool);
    let session = state.session.as_ref().unwrap();
    let capture = view(&state).identity();
    state.dispatch(SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture,
    });
    state
}

fn dimensions(
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

fn origin_anchor(state: &mut SketchInteraction) {
    let point = point(state, 0.0, 0.0);
    let session = state.session.as_ref().unwrap();
    let snap = AcceptedSnap {
        source: source_for(
            &session.authoritative,
            point.capture,
            session.request.sequence,
        ),
        kind: SnapKind::Origin,
        point: point.point,
        intent: SnapIntent::Origin,
    };
    assert!(state
        .dispatch(SketchAction::ClickInferred {
            point,
            ids: ids(1000),
            snap: Some(Box::new(snap))
        })
        .is_empty());
}

fn move_to(state: &mut SketchInteraction, x: f64, y: f64) {
    let point = point(state, x, y);
    state.dispatch(SketchAction::Move(point));
}

#[test]
fn endpoint_math_preserves_cursor_quadrant_and_rejects_invalid_lengths() {
    let anchor = Vec2::new(0.0, 0.0).unwrap();
    let line = CreationDimensions::Line {
        length: Scalar::new(10.0),
    };
    assert_eq!(
        line.endpoint(anchor, Vec2::new(3.0, 4.0).unwrap()).unwrap(),
        Vec2::new(6.0, 8.0).unwrap()
    );
    assert!(line.endpoint(anchor, anchor).is_err());
    let circle = CreationDimensions::Circle {
        diameter: Scalar::new(6.0),
    };
    assert_eq!(
        circle.endpoint(anchor, anchor).unwrap(),
        Vec2::new(3.0, 0.0).unwrap()
    );
    for (cursor, expected) in [
        ([-1.0, -1.0], [-80.0, -50.0]),
        ([1.0, -1.0], [80.0, -50.0]),
        ([0.0, 0.0], [80.0, 50.0]),
    ] {
        let rectangle = CreationDimensions::Rectangle {
            width: Scalar::new(80.0),
            height: Scalar::new(50.0),
        };
        assert_eq!(
            rectangle
                .endpoint(anchor, Vec2::new(cursor[0], cursor[1]).unwrap())
                .unwrap(),
            Vec2::new(expected[0], expected[1]).unwrap()
        );
    }
    for invalid in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        assert!(CreationDimensions::Circle {
            diameter: Scalar {
                value: invalid,
                expr: None
            }
        }
        .endpoint(anchor, anchor)
        .is_err());
    }
}

#[test]
fn rectangle_batches_keep_anchor_inference_expressions_and_distinct_identity_slots() {
    let mut state = state(SketchTool::Rectangle);
    origin_anchor(&mut state);
    move_to(&mut state, -1.0, -1.0);
    let width = Scalar::with_expr(80.0, "plate_width");
    let height = Scalar::with_expr(50.0, "plate_height");
    let (_, ops) = upsert(dimensions(
        &mut state,
        CreationDimensions::Rectangle {
            width: width.clone(),
            height: height.clone(),
        },
        2000,
    ));
    assert!(state
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .entities()
        .is_empty());
    assert!(ops.iter().any(|op| matches!(op, SketchEditOp::AddEntity { entity: SketchEntity::Point { id, at, .. }} if *id == ids(2000).entities[2] && at.x == -80.0 && at.y == -50.0)));
    let constraints: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            SketchEditOp::AddConstraint { constraint } => Some(constraint),
            _ => None,
        })
        .collect();
    assert_eq!(constraints.len(), 7);
    assert!(constraints.iter().any(|constraint| matches!(constraint, Constraint::Fixed { id, point, .. } if *id == ids(2000).constraints[4] && *point == ids(2000).entities[0])));
    for (slot, end, expected) in [(8, 1, &width), (9, 3, &height)] {
        assert!(constraints.iter().any(|constraint| matches!(constraint, Constraint::Distance { id, entity1, entity2, value, .. } if *id == ids(2000).constraints[slot] && *entity1 == ids(2000).entities[0] && *entity2 == ids(2000).entities[end] && value == expected)));
    }
    let unique: std::collections::BTreeSet<_> = constraints
        .iter()
        .map(|constraint| constraint.id())
        .collect();
    assert_eq!(unique.len(), constraints.len());
}

#[test]
fn circle_uses_diameter_scalar_with_expression_and_radius_hint() {
    let mut state = state(SketchTool::Circle);
    origin_anchor(&mut state);
    move_to(&mut state, 0.0, 2.0);
    let diameter = Scalar::with_expr(6.0, "hole_diameter");
    let (_, ops) = upsert(dimensions(
        &mut state,
        CreationDimensions::Circle {
            diameter: diameter.clone(),
        },
        2000,
    ));
    assert!(ops.iter().any(|op| matches!(op, SketchEditOp::AddEntity { entity: SketchEntity::Circle { radius, .. } } if *radius == 3.0)));
    assert!(ops.iter().any(|op| matches!(op, SketchEditOp::AddConstraint { constraint: Constraint::Diameter { id, entity, value } } if *id == ids(2000).constraints[8] && *entity == ids(2000).entities[1] && *value == diameter)));
}

#[test]
fn stale_capture_and_visit_never_change_accepted_geometry() {
    let mut state = state(SketchTool::Circle);
    origin_anchor(&mut state);
    let session = state.session.as_ref().unwrap();
    let key = session.key.clone();
    let capture = session.accepted_capture.unwrap();
    let stale = onecad_scene::ViewSnapshot::new(
        capture.scene,
        capture.view + 1,
        view(&state).camera(),
        view(&state).viewport(),
    )
    .unwrap()
    .identity();
    let mut stale_key = key.clone();
    stale_key.generation += 1;
    for (key, capture) in [(stale_key, capture), (key, stale)] {
        assert!(state
            .dispatch(SketchAction::CreateNumeric {
                key,
                capture,
                ids: ids(2000),
                dimensions: Box::new(CreationDimensions::Circle {
                    diameter: Scalar::new(6.0)
                })
            })
            .is_empty());
    }
    assert!(state
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .entities()
        .is_empty());
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
}

#[test]
fn invalid_identity_slots_and_wrong_tool_never_change_accepted_geometry() {
    let mut state = state(SketchTool::Circle);
    origin_anchor(&mut state);
    let capture = state.session.as_ref().unwrap().accepted_capture.unwrap();
    assert!(dimensions(
        &mut state,
        CreationDimensions::Line {
            length: Scalar::new(5.0)
        },
        2000
    )
    .is_empty());
    let session = state.session.as_ref().unwrap();
    let mut duplicate = ids(2000);
    duplicate.constraints[8] = duplicate.constraints[4];
    assert!(state
        .dispatch(SketchAction::CreateNumeric {
            key: session.key.clone(),
            capture,
            ids: duplicate,
            dimensions: Box::new(CreationDimensions::Circle {
                diameter: Scalar::new(6.0)
            })
        })
        .is_empty());
    let session = state.session.as_ref().unwrap();
    assert!(session.authoritative.entities().is_empty());
    assert!(session.authoritative.constraints().is_empty());
    assert_eq!(state.session.as_ref().unwrap().phase, SketchPhase::Active);
}

#[test]
fn accepted_solved_line_reanchors_continuation_and_reuses_exact_point_identity() {
    let mut state = state(SketchTool::Line);
    origin_anchor(&mut state);
    move_to(&mut state, 3.0, 4.0);
    let (tag, ops) = upsert(dimensions(
        &mut state,
        CreationDimensions::Line {
            length: Scalar::with_expr(10.0, "length"),
        },
        2000,
    ));
    let mut snapshot = applied(&state, &ops);
    snapshot
        .sketch
        .apply_solved_positions(&[(ids(2000).entities[1], Vec2::new(0.0, 10.0).unwrap())]);
    complete(&mut state, tag, snapshot);
    let draft = state.session.as_ref().unwrap().draft.as_ref().unwrap();
    assert_eq!(draft.anchor, Vec2::new(0.0, 10.0).unwrap());
    assert_eq!(draft.anchor_id, Some(ids(2000).entities[1]));
    move_to(&mut state, 0.0, 20.0);
    let (_, next) = upsert(dimensions(
        &mut state,
        CreationDimensions::Line {
            length: Scalar::new(5.0),
        },
        3000,
    ));
    assert!(next.iter().any(|op| matches!(op, SketchEditOp::AddEntity { entity: SketchEntity::Line { start, end, .. } } if *start == ids(2000).entities[1] && *end == ids(3000).entities[1])));
    assert!(next.iter().any(|op| matches!(op, SketchEditOp::AddConstraint { constraint: Constraint::Distance { entity1, entity2, value, .. } } if *entity1 == ids(2000).entities[1] && *entity2 == ids(3000).entities[1] && value.value == 5.0)));
    assert_eq!(
        next.iter()
            .filter(|op| matches!(
                op,
                SketchEditOp::AddEntity {
                    entity: SketchEntity::Point { .. }
                }
            ))
            .count(),
        1
    );
}

#[test]
fn stale_anchor_inference_rejects_batch_without_erasing_current_geometry() {
    let mut state = state(SketchTool::Rectangle);
    origin_anchor(&mut state);
    move_to(&mut state, 4.0, 3.0);
    let existing = ids(5000).entities[0];
    state
        .session
        .as_mut()
        .unwrap()
        .authoritative
        .add_entity(SketchEntity::point(
            existing,
            Vec2::new(100.0, 100.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    assert!(dimensions(
        &mut state,
        CreationDimensions::Rectangle {
            width: Scalar::new(80.0),
            height: Scalar::new(50.0)
        },
        2000
    )
    .is_empty());
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.authoritative.entities().len(), 1);
    assert!(session.authoritative.contains_entity(existing));
    assert!(session.authoritative.constraints().is_empty());
    assert!(
        matches!(&session.status, SketchStatus::Refused(message) if message.contains("obsolete sketch"))
    );
    assert_eq!(session.phase, SketchPhase::Active);
}

#[test]
fn invalid_or_missing_numeric_input_keeps_draft_and_accepted_state() {
    let mut state = state(SketchTool::Line);
    assert!(dimensions(
        &mut state,
        CreationDimensions::Line {
            length: Scalar::new(10.0)
        },
        2000
    )
    .is_empty());
    origin_anchor(&mut state);
    move_to(&mut state, 1.0, 0.0);
    for value in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-12] {
        assert!(dimensions(
            &mut state,
            CreationDimensions::Line {
                length: Scalar { value, expr: None }
            },
            2000
        )
        .is_empty());
        let session = state.session.as_ref().unwrap();
        assert!(session.authoritative.entities().is_empty());
        assert!(session.authoritative.constraints().is_empty());
        assert!(session.draft.is_some());
        assert_eq!(session.phase, SketchPhase::Active);
    }
}
