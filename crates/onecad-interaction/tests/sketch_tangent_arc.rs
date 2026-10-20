#[allow(dead_code)]
#[path = "../src/sketch/test_support.rs"]
mod support;
use onecad_core::{
    document::variables::Scalar,
    edit::SketchEditOp,
    ids::EntityId,
    sketch::{Constraint, CurvePosition, SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::{
    inference::{source_for, AcceptedSnap, SnapIntent, SnapKind},
    *,
};
use support::*;
use CurvePosition::{Arbitrary, End, Start};

type Xy = [f64; 2];

fn sess(state: &SketchInteraction) -> &SketchSession {
    state.session.as_ref().unwrap()
}

fn click_xy(state: &mut SketchInteraction, xy: Xy, seed: u128) -> Vec<SketchEffect> {
    click(state, xy[0], xy[1], seed)
}

fn close(a: Xy, b: Xy) -> bool {
    (a[0] - b[0]).abs() <= 1e-9 && (a[1] - b[1]).abs() <= 1e-9
}

fn refusal(state: &SketchInteraction) -> String {
    match &sess(state).status {
        SketchStatus::Refused(message) => message.clone(),
        other => panic!("expected refusal, got {other:?}"),
    }
}

fn complete_ops(state: &mut SketchInteraction, ops: Vec<SketchEditOp>, tag: SketchTag) {
    let snapshot = applied(state, &ops);
    complete(state, tag, snapshot);
}

fn commit(state: &mut SketchInteraction, xy: Xy, seed: u128) -> Vec<SketchEditOp> {
    let (tag, ops) = upsert(click_xy(state, xy, seed));
    complete_ops(state, ops.clone(), tag);
    ops
}

/// Line (0,0)->(10,0) committed and solved; chain anchored at (10,0).
fn after_line() -> SketchInteraction {
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    assert!(click_xy(&mut state, [0.0, 0.0], 500).is_empty());
    commit(&mut state, [10.0, 0.0], 1000);
    state
}

fn set_tangent(state: &mut SketchInteraction, enabled: bool) {
    let key = sess(state).key.clone();
    state.dispatch(SketchAction::SetTangentArc { key, enabled });
}

struct Arc {
    id: EntityId,
    center: Xy,
    radius: f64,
    start: f64,
    end: f64,
}

impl Arc {
    fn at(&self, angle: f64) -> Xy {
        [
            self.center[0] + self.radius * angle.cos(),
            self.center[1] + self.radius * angle.sin(),
        ]
    }

    fn role_angle(&self, role: CurvePosition) -> f64 {
        if role == Start {
            self.start
        } else {
            self.end
        }
    }

    /// Unit direction of travel leaving the endpoint of `role` along the arc.
    fn leaving(&self, role: CurvePosition) -> Xy {
        let a = self.role_angle(role);
        if role == Start {
            [-a.sin(), a.cos()]
        } else {
            [a.sin(), -a.cos()]
        }
    }
}

fn arc_of(ops: &[SketchEditOp]) -> Arc {
    let (id, center_id, radius, start, end) = ops
        .iter()
        .find_map(|op| match op {
            SketchEditOp::AddEntity {
                entity:
                    SketchEntity::Arc {
                        id,
                        center,
                        radius,
                        start_angle,
                        end_angle,
                        ..
                    },
            } => Some((*id, *center, *radius, *start_angle, *end_angle)),
            _ => None,
        })
        .expect("arc op");
    let center = ops
        .iter()
        .find_map(|op| match op {
            SketchEditOp::AddEntity {
                entity: SketchEntity::Point { id, at, .. },
            } if *id == center_id => Some([at.x, at.y]),
            _ => None,
        })
        .expect("center op");
    Arc {
        id,
        center,
        radius,
        start,
        end,
    }
}

fn constraints(ops: &[SketchEditOp]) -> Vec<&Constraint> {
    ops.iter()
        .filter_map(|op| match op {
            SketchEditOp::AddConstraint { constraint } => Some(constraint),
            _ => None,
        })
        .collect()
}

fn dot(a: Xy, b: Xy) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: Xy, b: Xy) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// Asserts the exact weld + tangent pair against the preceding curve.
fn assert_weld(
    ops: &[SketchEditOp],
    prev: (EntityId, CurvePosition),
    arc: &Arc,
    role: CurvePosition,
    weld_to: (EntityId, CurvePosition),
) {
    let found = constraints(ops);
    assert_eq!(found.len(), 2);
    assert!(
        matches!(found[0], Constraint::Coincident { point1, point1_position, point2, point2_position, .. }
        if (*point1, *point1_position) == weld_to && *point2 == arc.id && *point2_position == role)
    );
    assert!(
        matches!(found[1], Constraint::TangentAtPoint { entity1, entity1_position, entity2, entity2_position, .. }
        if (*entity1, *entity1_position) == prev && *entity2 == arc.id && *entity2_position == role)
    );
}

#[test]
fn ccw_tangent_arc_commits_exact_ops_and_geometry() {
    let mut state = after_line();
    let anchor = sess(&state).draft.as_ref().unwrap().anchor;
    assert!(close([anchor.x, anchor.y], [10.0, 0.0]));
    set_tangent(&mut state, true);
    let mv = point(&state, 15.0, 5.0);
    state.dispatch(SketchAction::Move(mv));
    assert!(matches!(
        sess(&state).preview,
        Some(DraftPrimitive::Arc { .. })
    ));
    let (tag, ops) = upsert(click_xy(&mut state, [15.0, 5.0], 2000));
    assert_eq!(ops.len(), 4);
    assert!(
        matches!(&ops[0], SketchEditOp::AddEntity { entity: SketchEntity::Point { id, .. } } if *id == ids(2000).entities[0])
    );
    assert!(
        matches!(&ops[1], SketchEditOp::AddEntity { entity: SketchEntity::Arc { id, .. } } if *id == ids(2000).entities[1])
    );
    let arc = arc_of(&ops);
    let (prev_line, prev_end) = (ids(1000).entities[2], ids(1000).entities[1]);
    assert_weld(&ops, (prev_line, End), &arc, Start, (prev_end, Arbitrary));
    assert!(close(arc.at(arc.start), [10.0, 0.0]));
    assert!(close(arc.at(arc.end), [15.0, 5.0]));
    let out = arc.leaving(Start);
    assert!(dot(out, [1.0, 0.0]) > 0.0 && cross(out, [1.0, 0.0]).abs() < 1e-9);
    complete_ops(&mut state, ops, tag);
    let draft = sess(&state).draft.as_ref().unwrap();
    assert_eq!(
        draft.preceding,
        Some(ChainCurve {
            entity: arc.id,
            role: End
        })
    );
    assert!(draft.anchor_id.is_none() && !draft.tangent_arc);
    assert_eq!(draft.first_id, Some(ids(1000).entities[0]));
    assert!(close([draft.anchor.x, draft.anchor.y], [15.0, 5.0]));
}

#[test]
fn clockwise_tangent_arc_swaps_roles() {
    let mut state = after_line();
    set_tangent(&mut state, true);
    let (_, ops) = upsert(click_xy(&mut state, [15.0, -5.0], 2000));
    let arc = arc_of(&ops);
    let prev_end = ids(1000).entities[1];
    assert_weld(
        &ops,
        (ids(1000).entities[2], End),
        &arc,
        End,
        (prev_end, Arbitrary),
    );
    assert!(close(arc.at(arc.end), [10.0, 0.0]));
    assert!(close(arc.at(arc.start), [15.0, -5.0]));
    let out = arc.leaving(End);
    assert!(dot(out, [1.0, 0.0]) > 0.0 && cross(out, [1.0, 0.0]).abs() < 1e-9);
}

#[test]
fn reanchor_uses_solved_arc_not_click() {
    let mut state = after_line();
    set_tangent(&mut state, true);
    let (tag, ops) = upsert(click_xy(&mut state, [15.0, 5.0], 2000));
    let arc = arc_of(&ops);
    let perturbed = arc.end + 0.1;
    let mut solved = ops.clone();
    for op in &mut solved {
        if let SketchEditOp::AddEntity {
            entity: SketchEntity::Arc { end_angle, .. },
        } = op
        {
            *end_angle = perturbed;
        }
    }
    complete_ops(&mut state, solved, tag);
    let anchor = sess(&state).draft.as_ref().unwrap().anchor;
    assert!(close([anchor.x, anchor.y], arc.at(perturbed)));
    assert!(!close([anchor.x, anchor.y], [15.0, 5.0]));
}

#[test]
fn line_after_arc_welds_to_arc_endpoint() {
    let mut state = after_line();
    set_tangent(&mut state, true);
    let arc = arc_of(&commit(&mut state, [15.0, 5.0], 2000));
    let (tag, ops) = upsert(click_xy(&mut state, [25.0, 5.0], 3000));
    assert_eq!(ops.len(), 4);
    assert!(
        matches!(&ops[0], SketchEditOp::AddEntity { entity: SketchEntity::Point { id, at, .. } }
        if *id == ids(3000).entities[0] && close([at.x, at.y], [15.0, 5.0]))
    );
    assert!(matches!(
        &ops[2],
        SketchEditOp::AddEntity {
            entity: SketchEntity::Line { .. }
        }
    ));
    assert!(
        matches!(&ops[3], SketchEditOp::AddConstraint { constraint: Constraint::Coincident { point1, point1_position: Arbitrary, point2, point2_position: End, .. } }
        if *point1 == ids(3000).entities[0] && *point2 == arc.id)
    );
    complete_ops(&mut state, ops, tag);
    assert!(sess(&state).draft.as_ref().unwrap().anchor_id.is_some());
}

#[test]
fn arc_after_arc_welds_both_roles_and_continues_direction() {
    let mut state = after_line();
    set_tangent(&mut state, true);
    let first = arc_of(&commit(&mut state, [15.0, 5.0], 2000));
    set_tangent(&mut state, true);
    let (_, ops) = upsert(click_xy(&mut state, [20.0, 10.0], 3000));
    let second = arc_of(&ops);
    assert_weld(&ops, (first.id, End), &second, End, (first.id, End));
    // First arc arrives at End opposite to its `leaving` direction.
    let (a, b) = (first.leaving(End).map(|v| -v), second.leaving(End));
    assert!(dot(a, b) > 0.0 && cross(a, b).abs() < 1e-9);
    assert!(close(second.at(second.end), [15.0, 5.0]));
    assert!(close(second.at(second.start), [20.0, 10.0]));
}

#[test]
fn toggle_without_preceding_curve_is_refused() {
    let mut state = active(SketchPlane::xy(), SketchTool::Line);
    set_tangent(&mut state, false);
    assert!(matches!(sess(&state).status, SketchStatus::Entered(_)));
    set_tangent(&mut state, true);
    assert_eq!(refusal(&state), "Tangent arc needs a preceding chain curve");
    click_xy(&mut state, [0.0, 0.0], 500);
    set_tangent(&mut state, true);
    assert_eq!(refusal(&state), "Tangent arc needs a preceding chain curve");
    assert!(!sess(&state).draft.as_ref().unwrap().tangent_arc);
}

#[test]
fn collinear_target_is_refused_and_draft_retained() {
    let mut state = after_line();
    set_tangent(&mut state, true);
    let before = sess(&state).authoritative.clone();
    assert!(click_xy(&mut state, [20.0, 0.0], 2000).is_empty());
    assert_eq!(
        refusal(&state),
        "No finite tangent arc reaches a point on the tangent line"
    );
    assert!(sess(&state).authoritative == before);
    let draft = sess(&state).draft.as_ref().unwrap();
    assert!(draft.tangent_arc && draft.preceding.is_some());
}

#[test]
fn numeric_line_is_refused_in_tangent_mode() {
    let mut state = after_line();
    set_tangent(&mut state, true);
    let (key, capture) = (
        sess(&state).key.clone(),
        sess(&state).accepted_capture.unwrap(),
    );
    let dimensions = || {
        Box::new(CreationDimensions::Line {
            length: Scalar::new(5.0),
        })
    };
    let effects = state.dispatch(SketchAction::CreateNumeric {
        key: key.clone(),
        capture,
        ids: ids(2000),
        dimensions: dimensions(),
    });
    assert!(effects.is_empty());
    assert_eq!(
        refusal(&state),
        "Numeric dimensions do not match the active sketch method"
    );
    state.session.as_mut().unwrap().status = SketchStatus::Accepted;
    let before = sess(&state).preview.clone();
    state.dispatch(SketchAction::PreviewNumeric {
        key,
        capture,
        dimensions: dimensions(),
    });
    assert_eq!(
        refusal(&state),
        "Numeric dimensions do not match the active sketch method"
    );
    assert_eq!(sess(&state).preview, before);
}

#[test]
fn escape_and_tool_change_drop_tangent_mode() {
    let mut state = after_line();
    set_tangent(&mut state, true);
    state.dispatch(SketchAction::Escape);
    assert!(sess(&state).draft.is_none());
    let mut state = after_line();
    set_tangent(&mut state, true);
    state.dispatch(SketchAction::SetTool(SketchTool::Line));
    assert!(sess(&state).draft.is_none());
    set_tangent(&mut state, false);
    click_xy(&mut state, [0.0, 0.0], 600);
    assert!(!sess(&state).draft.as_ref().unwrap().tangent_arc);
}

#[test]
fn construction_mode_marks_arc_and_center() {
    let mut state = after_line();
    set_tangent(&mut state, true);
    let key = sess(&state).key.clone();
    state.dispatch(SketchAction::SetConstructionMode {
        key,
        construction: true,
    });
    let (_, ops) = upsert(click_xy(&mut state, [15.0, 5.0], 2000));
    let flags: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            SketchEditOp::AddEntity { entity } => Some(entity.is_construction()),
            _ => None,
        })
        .collect();
    assert_eq!(flags, [true, true]);
}

#[test]
fn persistent_final_snap_maps_to_arc_travel_end_role() {
    for (target_xy, role) in [([15.0, 5.0], End), ([15.0, -5.0], Start)] {
        let mut state = active(SketchPlane::xy(), SketchTool::Point);
        let (tag, ops) = upsert(click_xy(&mut state, target_xy, 7000));
        complete_ops(&mut state, ops, tag);
        state.dispatch(SketchAction::SetTool(SketchTool::Line));
        click_xy(&mut state, [0.0, 0.0], 500);
        commit(&mut state, [10.0, 0.0], 1000);
        set_tangent(&mut state, true);
        let target = SketchTarget::Point {
            entity: ids(7000).entities[0],
            role: Arbitrary,
        };
        let point = point(&state, target_xy[0], target_xy[1]);
        let snap = AcceptedSnap {
            source: source_for(
                &sess(&state).authoritative,
                point.capture,
                sess(&state).request.sequence,
            ),
            point: point.point,
            kind: SnapKind::Origin,
            intent: SnapIntent::Coincident(target),
        };
        let (_, ops) = upsert(state.dispatch(SketchAction::ClickInferred {
            point,
            ids: ids(2000),
            snap: Some(Box::new(snap)),
        }));
        let arc = arc_of(&ops);
        let mapped = constraints(&ops).into_iter().any(|c| {
            matches!(c, Constraint::Coincident { point1, point1_position, point2, .. }
            if *point1 == arc.id && *point1_position == role && *point2 == ids(7000).entities[0])
        });
        assert!(mapped, "snap must land on arc role {role:?}");
        assert_eq!(constraints(&ops).len(), 3);
    }
}

fn snap_click(
    state: &mut SketchInteraction,
    xy: Xy,
    target: SketchTarget,
    seed: u128,
) -> Vec<SketchEffect> {
    let point = point(state, xy[0], xy[1]);
    let snap = AcceptedSnap {
        source: source_for(
            &sess(state).authoritative,
            point.capture,
            sess(state).request.sequence,
        ),
        point: point.point,
        kind: SnapKind::Endpoint,
        intent: SnapIntent::Coincident(target),
    };
    state.dispatch(SketchAction::ClickInferred {
        point,
        ids: ids(seed),
        snap: Some(Box::new(snap)),
    })
}

#[test]
fn unique_existing_line_endpoint_seeds_tangent_without_new_point() {
    for (xy, role, point_id) in [
        ([10.0, 0.0], End, ids(1000).entities[1]),
        ([0.0, 0.0], Start, ids(1000).entities[0]),
    ] {
        let mut state = after_line();
        state.dispatch(SketchAction::SetTool(SketchTool::Line));
        let line = ids(1000).entities[2];
        assert!(snap_click(
            &mut state,
            xy,
            SketchTarget::Point {
                entity: point_id,
                role: Arbitrary
            },
            9000
        )
        .is_empty());
        assert!(sess(&state).can_tangent_arc());
        set_tangent(&mut state, true);
        let (_, ops) = upsert(click_xy(&mut state, [5.0, 5.0], 2000));
        let arc = arc_of(&ops);
        assert_eq!(ops.len(), 4);
        assert_weld(
            &ops,
            (line, role),
            &arc,
            if role == End { Start } else { End },
            (point_id, Arbitrary),
        );
    }
}

#[test]
fn unique_existing_arc_endpoint_keeps_curve_role_through_chain_and_closure() {
    let mut state = after_line();
    set_tangent(&mut state, true);
    let arc = arc_of(&commit(&mut state, [15.0, 5.0], 2000));
    state.dispatch(SketchAction::SetTool(SketchTool::Line));
    let target = SketchTarget::Point {
        entity: arc.id,
        role: End,
    };
    assert!(snap_click(&mut state, [15.0, 5.0], target, 9000).is_empty());
    assert!(sess(&state).can_tangent_arc());
    assert_eq!(
        sess(&state).draft.as_ref().unwrap().first_target,
        Some(target)
    );
    set_tangent(&mut state, true);
    let second = arc_of(&commit(&mut state, [20.0, 10.0], 3000));
    assert!(sess(&state).draft.as_ref().unwrap().first_id.is_none());
    let (tag, ops) = upsert(snap_click(&mut state, [15.0, 5.0], target, 4000));
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(
                op,
                SketchEditOp::AddEntity {
                    entity: SketchEntity::Point { .. }
                }
            ))
            .count(),
        2
    );
    assert!(constraints(&ops).iter().any(|constraint| matches!(constraint, Constraint::Coincident { point1, point1_position: Arbitrary, point2, point2_position: End, .. } if *point1 == ids(4000).entities[1] && *point2 == arc.id)));
    assert!(constraints(&ops).iter().any(|constraint| matches!(constraint, Constraint::Coincident { point2, point2_position, .. } if *point2 == second.id && *point2_position == Start)));
    complete_ops(&mut state, ops, tag);
    assert!(sess(&state).draft.is_none());
}

#[test]
fn tangent_arc_closure_welds_travel_end_once_and_completes_chain() {
    let mut state = after_line();
    set_tangent(&mut state, true);
    commit(&mut state, [15.0, 5.0], 2000);
    set_tangent(&mut state, true);
    let target = SketchTarget::Point {
        entity: ids(1000).entities[0],
        role: Arbitrary,
    };
    let (tag, ops) = upsert(snap_click(&mut state, [0.0, 0.0], target, 3000));
    let arc = arc_of(&ops);
    assert_eq!(constraints(&ops).len(), 3);
    assert!(constraints(&ops).iter().any(|constraint| matches!(constraint, Constraint::Coincident { point1, point1_position, point2, point2_position: Arbitrary, .. } if *point1 == arc.id && *point1_position == End && *point2 == ids(1000).entities[0])));
    complete_ops(&mut state, ops, tag);
    assert!(sess(&state).draft.is_none());
}

#[test]
fn junction_seed_does_not_guess_preceding_curve() {
    let mut state = after_line();
    commit(&mut state, [20.0, 0.0], 2000);
    state.dispatch(SketchAction::SetTool(SketchTool::Line));
    let target = SketchTarget::Point {
        entity: ids(1000).entities[1],
        role: Arbitrary,
    };
    snap_click(&mut state, [10.0, 0.0], target, 9000);
    assert!(!sess(&state).can_tangent_arc());
    set_tangent(&mut state, true);
    assert_eq!(refusal(&state), "Tangent arc needs a preceding chain curve");
}

#[test]
fn removed_or_repaired_preceding_curve_refuses_without_partial_batch() {
    for replacement in [false, true] {
        let mut state = after_line();
        set_tangent(&mut state, true);
        let session = state.session.as_mut().unwrap();
        session.authoritative.remove_entity(ids(1000).entities[2]);
        if replacement {
            session
                .authoritative
                .add_entity(SketchEntity::point(
                    ids(1000).entities[2],
                    onecad_core::math::Vec2::new(10.0, 0.0).unwrap(),
                    false,
                    false,
                ))
                .unwrap();
        }
        let before = sess(&state).authoritative.clone();
        assert!(!sess(&state).can_tangent_arc());
        assert!(click_xy(&mut state, [15.0, 5.0], 2000).is_empty());
        assert_eq!(sess(&state).authoritative, before);
        assert_eq!(sess(&state).phase, SketchPhase::Active);
    }
}

#[test]
fn accepted_preserve_snapshot_invalidates_missing_or_repaired_chain_without_fault() {
    for replacement in [false, true] {
        let mut state = after_line();
        set_tangent(&mut state, true);
        let constraint = Constraint::Fixed {
            id: ids(8000).constraints[0],
            point: ids(1000).entities[0],
            point_position: Arbitrary,
            at: onecad_core::math::Vec2::new(0.0, 0.0).unwrap(),
        };
        let key = sess(&state).key.clone();
        let effects = state.dispatch(SketchAction::AddConstraints {
            key,
            constraints: vec![constraint.clone()],
        });
        let [SketchEffect::AddConstraints { tag, .. }] = effects.as_slice() else {
            panic!("expected constraint batch");
        };
        let mut snapshot = applied(&state, &[SketchEditOp::AddConstraint { constraint }]);
        snapshot.sketch.remove_entity(ids(1000).entities[2]);
        if replacement {
            snapshot
                .sketch
                .add_entity(SketchEntity::point(
                    ids(1000).entities[2],
                    onecad_core::math::Vec2::new(10.0, 0.0).unwrap(),
                    false,
                    false,
                ))
                .unwrap();
        }
        complete(&mut state, tag.clone(), snapshot);
        assert_eq!(sess(&state).phase, SketchPhase::Active);
        assert!(sess(&state).draft.is_none());
        assert!(sess(&state).preview.is_none());
        assert!(!sess(&state).can_tangent_arc());
        let before = sess(&state).authoritative.clone();
        assert!(click_xy(&mut state, [15.0, 5.0], 9000).is_empty());
        assert_eq!(sess(&state).authoritative, before);
        assert!(sess(&state).draft.as_ref().unwrap().preceding.is_none());
    }
}

#[test]
fn standalone_arc_seed_uses_both_solved_endpoint_roles_not_chord() {
    for (at, role, end, tangent) in [
        ([10.0, 0.0], Start, [15.0, -5.0], [0.0, -1.0]),
        ([0.0, 10.0], End, [-5.0, 15.0], [-1.0, 0.0]),
    ] {
        let mut state = active(SketchPlane::xy(), SketchTool::Arc);
        click_xy(&mut state, [0.0, 0.0], 500);
        click_xy(&mut state, [10.0, 0.0], 600);
        let arc = arc_of(&commit(&mut state, [0.0, 10.0], 1000));
        state.dispatch(SketchAction::SetTool(SketchTool::Line));
        let target = SketchTarget::Point {
            entity: arc.id,
            role,
        };
        snap_click(&mut state, at, target, 9000);
        assert!(sess(&state).can_tangent_arc());
        let draft = sess(&state).draft.as_ref().unwrap();
        assert_eq!(draft.first_target, Some(target));
        assert!(draft.anchor_id.is_none() && draft.first_id.is_none());
        set_tangent(&mut state, true);
        let (_, ops) = upsert(click_xy(&mut state, end, 2000));
        let next = arc_of(&ops);
        assert_eq!(ops.len(), 4);
        let start_role = if close(next.at(next.start), at) {
            Start
        } else {
            End
        };
        assert_weld(&ops, (arc.id, role), &next, start_role, (arc.id, role));
        let actual = next.leaving(start_role);
        assert!(dot(actual, tangent) > 0.0 && cross(actual, tangent).abs() < 1e-9);
    }
}
