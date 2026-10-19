#[allow(dead_code)]
#[path = "../src/sketch/test_support.rs"]
mod support;
use onecad_core::{
    document::variables::Scalar,
    edit::SketchEditOp,
    sketch::{Constraint, CurvePosition, SketchEntity, SketchPlane},
};
use onecad_interaction::sketch::{
    inference::{source_for, AcceptedSnap, SnapIntent, SnapKind},
    *,
};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use support::*;

type Xy = [f64; 2];

fn sess(state: &SketchInteraction) -> &SketchSession {
    state.session.as_ref().unwrap()
}

fn mv(state: &mut SketchInteraction, xy: Xy) {
    state.dispatch(SketchAction::Move(point(state, xy[0], xy[1])));
}

fn click_xy(state: &mut SketchInteraction, xy: Xy, seed: u128) -> Vec<SketchEffect> {
    click(state, xy[0], xy[1], seed)
}

fn inferred(
    state: &mut SketchInteraction,
    xy: Xy,
    seed: u128,
    intent: SnapIntent,
    generation_skew: u64,
) -> Vec<SketchEffect> {
    let point = point(state, xy[0], xy[1]);
    let session = sess(state);
    let snap = AcceptedSnap {
        source: source_for(
            &session.authoritative,
            point.capture,
            session.request.sequence + generation_skew,
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

fn dims_action(
    state: &mut SketchInteraction,
    dimensions: CreationDimensions,
    seed: Option<u128>,
) -> Vec<SketchEffect> {
    let session = sess(state);
    let (key, capture) = (session.key.clone(), session.accepted_capture.unwrap());
    let dimensions = Box::new(dimensions);
    match seed {
        Some(seed) => state.dispatch(SketchAction::CreateNumeric {
            key,
            capture,
            ids: ids(seed),
            dimensions,
        }),
        None => state.dispatch(SketchAction::PreviewNumeric {
            key,
            capture,
            dimensions,
        }),
    }
}

fn arc_dims(radius: f64, sweep: f64) -> CreationDimensions {
    CreationDimensions::Arc {
        radius: Scalar::new(radius),
        sweep,
    }
}

fn staged(tool: SketchTool, first: Xy, second: Xy) -> SketchInteraction {
    let mut state = active(SketchPlane::xy(), tool);
    assert!(click_xy(&mut state, first, 500).is_empty());
    assert!(click_xy(&mut state, second, 500).is_empty());
    state
}

struct Arc {
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
}

fn arc_of(ops: &[SketchEditOp]) -> Arc {
    let (center_id, radius, start, end) = ops
        .iter()
        .find_map(|op| match op {
            SketchEditOp::AddEntity {
                entity:
                    SketchEntity::Arc {
                        center,
                        radius,
                        start_angle,
                        end_angle,
                        ..
                    },
            } => Some((*center, *radius, *start_angle, *end_angle)),
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
        center,
        radius,
        start,
        end,
    }
}

fn close(a: Xy, b: Xy, eps: f64) -> bool {
    (a[0] - b[0]).abs() <= eps && (a[1] - b[1]).abs() <= eps
}

fn assert_no_authoring(state: &SketchInteraction, before: &onecad_core::sketch::Sketch) {
    assert!(sess(state).authoritative == *before);
}

fn refusal(state: &SketchInteraction) -> String {
    match &sess(state).status {
        SketchStatus::Refused(message) => message.clone(),
        other => panic!("expected refusal, got {other:?}"),
    }
}

fn finish(state: &mut SketchInteraction, effects: Vec<SketchEffect>) -> Vec<SketchEditOp> {
    let (tag, ops) = upsert(effects);
    let snapshot = applied(state, &ops);
    complete(state, tag, snapshot);
    ops
}

fn three_point(start: Xy, end: Xy, through: Xy) -> Vec<SketchEditOp> {
    let mut state = staged(SketchTool::ThreePointArc, start, end);
    upsert(click_xy(&mut state, through, 1000)).1
}

fn all_construction(ops: &[SketchEditOp]) -> bool {
    let mut seen = 0;
    for op in ops {
        if let SketchEditOp::AddEntity { entity } = op {
            seen += 1;
            if !entity.is_construction() {
                return false;
            }
        }
    }
    seen > 0
}

fn set_construction(state: &mut SketchInteraction) {
    let key = sess(state).key.clone();
    state.dispatch(SketchAction::SetConstructionMode {
        key,
        construction: true,
    });
}

#[test]
fn center_arc_previews_then_commits_exact_two_ops_and_repeats() {
    let mut state = active(SketchPlane::xy(), SketchTool::Arc);
    let before = sess(&state).authoritative.clone();
    assert!(click_xy(&mut state, [0.0, 0.0], 500).is_empty());
    assert!(click_xy(&mut state, [10.0, 0.0], 500).is_empty());
    assert_no_authoring(&state, &before);
    assert!(matches!(
        sess(&state).preview,
        Some(DraftPrimitive::Line { .. })
    ));
    mv(&mut state, [0.0, 25.0]);
    let Some(DraftPrimitive::Arc { arc }) = &sess(&state).preview else {
        panic!("arc preview")
    };
    assert!(arc.center.x.abs() < 1e-12 && arc.center.y.abs() < 1e-12);
    assert!((arc.radius - 10.0).abs() < 1e-12 && arc.start_angle.abs() < 1e-12);
    assert!((arc.end_angle - FRAC_PI_2).abs() < 1e-12);
    let (tag, ops) = upsert(click_xy(&mut state, [0.0, 25.0], 1000));
    assert_eq!(ops.len(), 2);
    assert!(
        matches!(&ops[0], SketchEditOp::AddEntity { entity: SketchEntity::Point { id, at, construction: false, .. } } if *id == ids(1000).entities[0] && at.x == 0.0 && at.y == 0.0)
    );
    assert!(
        matches!(&ops[1], SketchEditOp::AddEntity { entity: SketchEntity::Arc { id, center, radius, .. } } if *id == ids(1000).entities[1] && *center == ids(1000).entities[0] && (*radius - 10.0).abs() < 1e-9)
    );
    let arc = arc_of(&ops);
    assert!(close(arc.at(arc.start), [10.0, 0.0], 1e-9));
    assert!(close(arc.at(arc.end), [0.0, 10.0], 1e-9));
    let snapshot = applied(&state, &ops);
    complete(&mut state, tag, snapshot);
    assert!(sess(&state).draft.is_none());
    assert_eq!(sess(&state).authoritative.entities().len(), 2);
    click_xy(&mut state, [30.0, 0.0], 2000);
    click_xy(&mut state, [40.0, 0.0], 2000);
    let effects = click_xy(&mut state, [30.0, 10.0], 2000);
    let ops = finish(&mut state, effects);
    assert_eq!(ops.len(), 2);
    assert_eq!(sess(&state).authoritative.entities().len(), 4);
}

#[test]
fn center_arc_major_sweep_is_stored_counterclockwise() {
    let mut state = staged(SketchTool::Arc, [0.0, 0.0], [10.0, 0.0]);
    let (_, ops) = upsert(click_xy(&mut state, [0.0, -3.0], 1000));
    let arc = arc_of(&ops);
    assert!((arc.radius - 10.0).abs() < 1e-12);
    assert!((arc.end - arc.start - 3.0 * FRAC_PI_2).abs() < 1e-12);
    assert!(close(arc.at(arc.start), [10.0, 0.0], 1e-9));
    assert!(close(arc.at(arc.end), [0.0, -10.0], 1e-9));
}

fn in_ccw_span(arc: &Arc, through: Xy) -> bool {
    let angle = (through[1] - arc.center[1]).atan2(through[0] - arc.center[0]);
    (angle - arc.start).rem_euclid(TAU) <= arc.end - arc.start + 1e-9
}

#[test]
fn three_point_arc_reaches_both_clicked_endpoints_and_contains_through() {
    let (start, end) = ([10.0, 0.0], [-10.0, 0.0]);
    for through in [[0.0, 10.0], [0.0, -10.0]] {
        let arc = arc_of(&three_point(start, end, through));
        assert!(close(arc.center, [0.0, 0.0], 1e-9) && (arc.radius - 10.0).abs() < 1e-9);
        let ends = [arc.at(arc.start), arc.at(arc.end)];
        assert!(ends.iter().any(|e| close(*e, start, 1e-9)));
        assert!(ends.iter().any(|e| close(*e, end, 1e-9)));
        assert!(in_ccw_span(&arc, through));
    }
    let clockwise = arc_of(&three_point(start, end, [0.0, -10.0]));
    assert!(close(clockwise.at(clockwise.start), end, 1e-9));
    let ccw = arc_of(&three_point(start, end, [0.0, 10.0]));
    assert!(close(ccw.at(ccw.start), start, 1e-9));
}

fn point_then(tool: SketchTool, at: Xy) -> (SketchInteraction, SketchTarget) {
    let mut state = active(SketchPlane::xy(), SketchTool::Point);
    let effects = click_xy(&mut state, at, 7000);
    finish(&mut state, effects);
    state.dispatch(SketchAction::SetTool(tool));
    let entity = ids(7000).entities[0];
    let role = CurvePosition::Arbitrary;
    (state, SketchTarget::Point { entity, role })
}

fn start_coincident(ops: &[SketchEditOp], target: SketchTarget) -> CurvePosition {
    let SketchTarget::Point { entity, .. } = target else {
        panic!()
    };
    let found: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            SketchEditOp::AddConstraint {
                constraint:
                    Constraint::Coincident {
                        point1,
                        point1_position,
                        point2,
                        ..
                    },
            } if *point1 == ids(1000).entities[1] && *point2 == entity => Some(*point1_position),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1);
    found[0]
}

#[test]
fn three_point_start_snap_maps_to_start_or_end_by_stored_direction() {
    for (through, role) in [
        ([0.0, 10.0], CurvePosition::Start),
        ([0.0, -10.0], CurvePosition::End),
    ] {
        let (mut state, target) = point_then(SketchTool::ThreePointArc, [10.0, 0.0]);
        let intent = SnapIntent::Coincident(target);
        assert!(inferred(&mut state, [10.0, 0.0], 500, intent, 0).is_empty());
        assert!(click_xy(&mut state, [-10.0, 0.0], 500).is_empty());
        let (_, ops) = upsert(click_xy(&mut state, through, 1000));
        assert_eq!(start_coincident(&ops, target), role);
    }
}

#[test]
fn center_arc_start_snap_maps_to_start_including_major_arcs() {
    for end in [[0.0, 25.0], [0.0, -3.0]] {
        let (mut state, target) = point_then(SketchTool::Arc, [10.0, 0.0]);
        assert!(click_xy(&mut state, [0.0, 0.0], 500).is_empty());
        let intent = SnapIntent::Coincident(target);
        assert!(inferred(&mut state, [10.0, 0.0], 500, intent, 0).is_empty());
        let (_, ops) = upsert(click_xy(&mut state, end, 1000));
        assert_eq!(start_coincident(&ops, target), CurvePosition::Start);
    }
}

#[test]
fn escape_and_tool_change_clear_draft_without_authoring() {
    for clicks in [1, 2] {
        let mut state = active(SketchPlane::xy(), SketchTool::Arc);
        let before = sess(&state).authoritative.clone();
        click_xy(&mut state, [0.0, 0.0], 500);
        if clicks == 2 {
            click_xy(&mut state, [10.0, 0.0], 500);
        }
        assert!(state.dispatch(SketchAction::Escape).is_empty());
        assert!(sess(&state).draft.is_none() && sess(&state).preview.is_none());
        assert!(click_xy(&mut state, [5.0, 5.0], 500).is_empty());
        let draft = sess(&state).draft.as_ref().unwrap();
        assert!(
            draft.secondary.is_none() && close([draft.anchor.x, draft.anchor.y], [5.0, 5.0], 1e-9)
        );
        assert_no_authoring(&state, &before);
    }
    let mut state = staged(SketchTool::Arc, [0.0, 0.0], [10.0, 0.0]);
    state.dispatch(SketchAction::SetTool(SketchTool::Line));
    state.dispatch(SketchAction::SetTool(SketchTool::Arc));
    assert!(sess(&state).draft.is_none() && sess(&state).preview.is_none());
    assert!(click_xy(&mut state, [0.0, 25.0], 1000).is_empty());
}

#[test]
fn construction_mode_marks_every_arc_entity() {
    for tool in [SketchTool::Arc, SketchTool::ThreePointArc] {
        let mut state = active(SketchPlane::xy(), tool);
        set_construction(&mut state);
        click_xy(&mut state, [10.0, 0.0], 500);
        click_xy(&mut state, [-10.0, 0.0], 500);
        let (_, ops) = upsert(click_xy(&mut state, [0.0, 10.0], 1000));
        assert!(all_construction(&ops), "{tool:?}");
    }
    let mut state = active(SketchPlane::xy(), SketchTool::Arc);
    set_construction(&mut state);
    click_xy(&mut state, [0.0, 0.0], 500);
    click_xy(&mut state, [10.0, 0.0], 500);
    let (_, ops) = upsert(dims_action(
        &mut state,
        arc_dims(7.0, FRAC_PI_2),
        Some(1000),
    ));
    assert!(all_construction(&ops));
}

#[test]
fn degenerate_arcs_refuse_without_authoring_and_keep_draft() {
    let cases = [
        (
            SketchTool::Arc,
            [0.0, 0.0],
            [0.0, 0.0],
            [5.0, 5.0],
            "distinct",
        ),
        (
            SketchTool::Arc,
            [0.0, 0.0],
            [10.0, 0.0],
            [0.0, 0.0],
            "differ from its center",
        ),
        (
            SketchTool::ThreePointArc,
            [0.0, 0.0],
            [10.0, 0.0],
            [5.0, 0.0],
            "collinear",
        ),
    ];
    for (tool, first, second, last, expected) in cases {
        let mut state = active(SketchPlane::xy(), tool);
        let before = sess(&state).authoritative.clone();
        click_xy(&mut state, first, 500);
        let effects = click_xy(&mut state, second, 500);
        let effects = if effects.is_empty() && second != first {
            click_xy(&mut state, last, 1000)
        } else {
            effects
        };
        assert!(effects.is_empty());
        assert!(refusal(&state).contains(expected), "{}", refusal(&state));
        assert_no_authoring(&state, &before);
        // Production keeps the draft after a refusal so the user can re-aim.
        assert!(sess(&state).draft.is_some());
        assert_eq!(sess(&state).phase, SketchPhase::Active);
    }
}

#[test]
fn stale_capture_is_ignored_and_stale_snap_is_refused() {
    let mut state = active(SketchPlane::xy(), SketchTool::Arc);
    let before = sess(&state).authoritative.clone();
    let mut captured = point(&state, 0.0, 0.0);
    captured.capture.view += 1;
    let ids_ = ids(500);
    let stale = SketchAction::Click {
        point: captured,
        ids: ids_,
    };
    assert!(state.dispatch(stale).is_empty());
    assert!(sess(&state).draft.is_none());
    click_xy(&mut state, [0.0, 0.0], 500);
    assert!(inferred(&mut state, [10.0, 0.0], 500, SnapIntent::Origin, 1).is_empty());
    assert!(refusal(&state).contains("stale"), "{}", refusal(&state));
    assert!(sess(&state).draft.as_ref().unwrap().secondary.is_none());
    assert_no_authoring(&state, &before);
}

#[test]
fn numeric_preview_is_pure_and_create_adds_radius_constraint() {
    let mut state = staged(SketchTool::Arc, [0.0, 0.0], [10.0, 0.0]);
    let (before, draft) = (
        sess(&state).authoritative.clone(),
        sess(&state).draft.clone(),
    );
    let draft = draft.unwrap();
    dims_action(&mut state, arc_dims(7.0, FRAC_PI_2), None);
    assert_no_authoring(&state, &before);
    let after = sess(&state).draft.as_ref().unwrap();
    assert!(after.anchor == draft.anchor && after.first == draft.first);
    assert!(after.secondary.as_ref().unwrap().point == draft.secondary.as_ref().unwrap().point);
    let Some(DraftPrimitive::Arc { arc }) = &sess(&state).preview else {
        panic!("numeric preview")
    };
    assert!((arc.radius - 7.0).abs() < 1e-12);
    let (_, ops) = upsert(dims_action(
        &mut state,
        arc_dims(7.0, FRAC_PI_2),
        Some(1000),
    ));
    assert_eq!(ops.len(), 3);
    let arc = arc_of(&ops);
    assert!((arc.radius - 7.0).abs() < 1e-12 && arc.start.abs() < 1e-12);
    assert!((arc.end - FRAC_PI_2).abs() < 1e-12);
    assert!(
        matches!(&ops[2], SketchEditOp::AddConstraint { constraint: Constraint::Radius { id, entity, value } } if *id == ids(1000).constraints[8] && *entity == ids(1000).entities[1] && value.value == 7.0)
    );
}

#[test]
fn numeric_negative_sweep_stores_counterclockwise_span() {
    let mut state = staged(SketchTool::Arc, [0.0, 0.0], [10.0, 0.0]);
    let (_, ops) = upsert(dims_action(
        &mut state,
        arc_dims(7.0, -FRAC_PI_2),
        Some(1000),
    ));
    let arc = arc_of(&ops);
    assert!((arc.start + FRAC_PI_2).abs() < 1e-12 && arc.end.abs() < 1e-12);
}

#[test]
fn numeric_invalid_inputs_and_three_point_tool_refuse() {
    for (radius, sweep) in [(7.0, 0.0), (7.0, TAU), (7.0, -TAU), (0.0, FRAC_PI_2)] {
        let mut state = staged(SketchTool::Arc, [0.0, 0.0], [10.0, 0.0]);
        let before = sess(&state).authoritative.clone();
        assert!(dims_action(&mut state, arc_dims(radius, sweep), Some(1000)).is_empty());
        refusal(&state);
        assert_no_authoring(&state, &before);
    }
    let mut state = staged(SketchTool::ThreePointArc, [10.0, 0.0], [-10.0, 0.0]);
    assert!(dims_action(&mut state, arc_dims(7.0, PI / 2.0), Some(1000)).is_empty());
    assert!(refusal(&state).contains("do not match"));
}
