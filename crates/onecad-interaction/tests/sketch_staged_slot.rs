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
use std::collections::{BTreeMap, BTreeSet};
use support::*;

type Xy = [f64; 2];

fn sess(state: &SketchInteraction) -> &SketchSession {
    state.session.as_ref().unwrap()
}

fn click_xy(state: &mut SketchInteraction, xy: Xy, seed: u128) -> Vec<SketchEffect> {
    click(state, xy[0], xy[1], seed)
}

fn staged(centers: [Xy; 2]) -> SketchInteraction {
    let mut state = active(SketchPlane::xy(), SketchTool::Slot);
    assert!(click_xy(&mut state, centers[0], 500).is_empty());
    assert!(click_xy(&mut state, centers[1], 500).is_empty());
    state
}

fn slot_ops(cursor: Xy) -> Vec<SketchEditOp> {
    let mut state = staged([[0.0, 0.0], [20.0, 0.0]]);
    upsert(click_xy(&mut state, cursor, 1000)).1
}

fn dims(state: &mut SketchInteraction, width: f64, seed: Option<u128>) -> Vec<SketchEffect> {
    let session = sess(state);
    let (key, capture) = (session.key.clone(), session.accepted_capture.unwrap());
    let dimensions = Box::new(CreationDimensions::Slot {
        width: Scalar::new(width),
    });
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

struct Geometry {
    points: BTreeMap<EntityId, Xy>,
    lines: Vec<(EntityId, EntityId)>,
    arcs: Vec<(EntityId, EntityId, f64, f64, f64)>,
}

fn geometry(ops: &[SketchEditOp]) -> Geometry {
    let mut g = Geometry {
        points: BTreeMap::new(),
        lines: Vec::new(),
        arcs: Vec::new(),
    };
    for op in ops {
        let SketchEditOp::AddEntity { entity } = op else {
            continue;
        };
        match entity {
            SketchEntity::Point { id, at, .. } => {
                g.points.insert(*id, [at.x, at.y]);
            }
            SketchEntity::Line { start, end, .. } => g.lines.push((*start, *end)),
            SketchEntity::Arc {
                id,
                center,
                radius,
                start_angle,
                end_angle,
                ..
            } => g
                .arcs
                .push((*id, *center, *radius, *start_angle, *end_angle)),
            _ => panic!("unexpected entity"),
        }
    }
    g
}

impl Geometry {
    fn arc_end(&self, arc: EntityId, position: CurvePosition) -> Xy {
        let (_, center, radius, start, end) = *self.arcs.iter().find(|a| a.0 == arc).unwrap();
        let angle = if position == CurvePosition::Start {
            start
        } else {
            end
        };
        let c = self.points[&center];
        [c[0] + radius * angle.cos(), c[1] + radius * angle.sin()]
    }

    fn walls(&self) -> Vec<[i64; 4]> {
        let q = |v: f64| (v * 1e6).round() as i64;
        let mut walls: Vec<_> = self
            .lines
            .iter()
            .map(|(a, b)| {
                let (a, b) = (self.points[a], self.points[b]);
                [q(a[0]), q(a[1]), q(b[0]), q(b[1])]
            })
            .collect();
        walls.sort();
        walls
    }
}

fn close(a: Xy, b: Xy) -> bool {
    (a[0] - b[0]).abs() <= 1e-9 && (a[1] - b[1]).abs() <= 1e-9
}

fn constraints(ops: &[SketchEditOp]) -> Vec<&Constraint> {
    ops.iter()
        .filter_map(|op| match op {
            SketchEditOp::AddConstraint { constraint } => Some(constraint),
            _ => None,
        })
        .collect()
}

fn refusal(state: &SketchInteraction) -> String {
    match &sess(state).status {
        SketchStatus::Refused(message) => message.clone(),
        other => panic!("expected refusal, got {other:?}"),
    }
}

fn assert_topology(ops: &[SketchEditOp], g: &Geometry) {
    let count = |f: fn(&SketchEntity) -> bool| {
        ops.iter()
            .filter(|op| matches!(op, SketchEditOp::AddEntity { entity } if f(entity)))
            .count()
    };
    assert_eq!(count(|e| matches!(e, SketchEntity::Point { .. })), 6);
    assert_eq!((g.lines.len(), g.arcs.len()), (2, 2));
    let used: BTreeSet<_> = g
        .lines
        .iter()
        .flat_map(|l| [l.0, l.1])
        .chain(g.arcs.iter().map(|a| a.1))
        .collect();
    assert_eq!(used, g.points.keys().copied().collect());
}

fn assert_constraints(ops: &[SketchEditOp], g: &Geometry) {
    let all = constraints(ops);
    assert_eq!(all.len(), 9);
    let listed: Vec<_> = all.iter().map(|c| c.id()).collect();
    assert_eq!(listed.iter().collect::<BTreeSet<_>>().len(), 9);
    let wanted = ids(1000);
    let expected: BTreeSet<_> = wanted.constraints[..=8].iter().collect();
    assert_eq!(listed.iter().collect::<BTreeSet<_>>(), expected);
    let (mut coincident, mut tangent, mut equal) = (0, 0, 0);
    for constraint in all {
        match constraint {
            Constraint::Coincident {
                point1,
                point2,
                point2_position,
                ..
            } => {
                coincident += 1;
                assert!(close(
                    g.points[point1],
                    g.arc_end(*point2, *point2_position)
                ));
            }
            Constraint::TangentAtPoint { .. } => tangent += 1,
            Constraint::Equal {
                entity1, entity2, ..
            } => {
                equal += 1;
                let arcs: BTreeSet<_> = g.arcs.iter().map(|a| a.0).collect();
                assert_eq!(BTreeSet::from([*entity1, *entity2]), arcs);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!((coincident, tangent, equal), (4, 4, 1));
}

#[test]
fn slot_commits_exact_topology_and_symmetric_geometry() {
    let ops = slot_ops([10.0, 4.0]);
    assert_eq!(ops.len(), 19);
    let g = geometry(&ops);
    assert_topology(&ops, &g);
    assert_constraints(&ops, &g);
    assert!(g.arcs.iter().all(|a| (a.2 - 4.0).abs() < 1e-9));
    let q = |v: f64| (v * 1e6).round() as i64;
    assert_eq!(
        g.walls(),
        vec![
            [q(0.0), q(-4.0), q(20.0), q(-4.0)],
            [q(0.0), q(4.0), q(20.0), q(4.0)]
        ]
    );
    let mirrored = slot_ops([10.0, -4.0]);
    let m = geometry(&mirrored);
    assert_eq!(m.walls(), g.walls());
    assert!(m.arcs.iter().all(|a| (a.2 - 4.0).abs() < 1e-9));
    assert_constraints(&mirrored, &m);
}

#[test]
fn slot_preview_does_not_author_and_repeated_creation_works() {
    let mut state = staged([[0.0, 0.0], [20.0, 0.0]]);
    let before = sess(&state).authoritative.clone();
    state.dispatch(SketchAction::Move(point(&state, 10.0, 4.0)));
    let Some(DraftPrimitive::Slot { radius, .. }) = sess(&state).preview else {
        panic!("slot preview")
    };
    assert!((radius - 4.0).abs() < 1e-9);
    assert!(sess(&state).authoritative == before);
    let (tag, ops) = upsert(click_xy(&mut state, [10.0, 4.0], 1000));
    let snapshot = applied(&state, &ops);
    complete(&mut state, tag, snapshot);
    assert!(sess(&state).draft.is_none());
    assert_eq!(sess(&state).authoritative.entities().len(), 10);
    click_xy(&mut state, [0.0, 50.0], 3000);
    click_xy(&mut state, [20.0, 50.0], 3000);
    let (_, ops) = upsert(click_xy(&mut state, [10.0, 53.0], 3000));
    assert_eq!(ops.len(), 19);
}

#[test]
fn escape_and_tool_change_clear_slot_draft() {
    let mut state = staged([[0.0, 0.0], [20.0, 0.0]]);
    assert!(state.dispatch(SketchAction::Escape).is_empty());
    assert!(sess(&state).draft.is_none() && sess(&state).preview.is_none());
    let mut state = staged([[0.0, 0.0], [20.0, 0.0]]);
    state.dispatch(SketchAction::SetTool(SketchTool::Line));
    state.dispatch(SketchAction::SetTool(SketchTool::Slot));
    assert!(sess(&state).draft.is_none() && sess(&state).preview.is_none());
    assert!(click_xy(&mut state, [10.0, 4.0], 1000).is_empty());
}

#[test]
fn construction_mode_marks_every_slot_entity() {
    let mut state = staged([[0.0, 0.0], [20.0, 0.0]]);
    let key = sess(&state).key.clone();
    state.dispatch(SketchAction::SetConstructionMode {
        key,
        construction: true,
    });
    let (_, ops) = upsert(click_xy(&mut state, [10.0, 4.0], 1000));
    let entities: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            SketchEditOp::AddEntity { entity } => Some(entity),
            _ => None,
        })
        .collect();
    assert_eq!(entities.len(), 10);
    assert!(entities.iter().all(|entity| entity.is_construction()));
}

#[test]
fn degenerate_slots_refuse_without_authoring() {
    let mut state = active(SketchPlane::xy(), SketchTool::Slot);
    let before = sess(&state).authoritative.clone();
    click_xy(&mut state, [0.0, 0.0], 500);
    assert!(click_xy(&mut state, [0.0, 0.0], 500).is_empty());
    assert!(refusal(&state).contains("distinct"));
    assert!(sess(&state).draft.as_ref().unwrap().secondary.is_none());
    click_xy(&mut state, [20.0, 0.0], 500);
    // Cursor on the centerline gives zero half-width.
    assert!(click_xy(&mut state, [10.0, 0.0], 1000).is_empty());
    assert!(
        refusal(&state).contains("half-width"),
        "{}",
        refusal(&state)
    );
    assert!(sess(&state).draft.is_some());
    assert!(sess(&state).authoritative == before);
}

#[test]
fn stale_slot_capture_ignored_and_width_snap_outside_wall_span_refused() {
    let mut state = staged([[0.0, 0.0], [20.0, 0.0]]);
    let mut captured = point(&state, 10.0, 4.0);
    captured.capture.view += 1;
    let ids_ = ids(1000);
    let stale = SketchAction::Click {
        point: captured,
        ids: ids_,
    };
    assert!(state.dispatch(stale).is_empty());
    assert!(sess(&state).draft.is_some());
    let point = point(&state, -5.0, 4.0);
    let session = sess(&state);
    let snap = AcceptedSnap {
        source: source_for(
            &session.authoritative,
            point.capture,
            session.request.sequence,
        ),
        point: point.point,
        kind: SnapKind::Origin,
        intent: SnapIntent::Origin,
    };
    let effects = state.dispatch(SketchAction::ClickInferred {
        point,
        ids: ids(1000),
        snap: Some(Box::new(snap)),
    });
    assert!(effects.is_empty());
    assert!(
        refusal(&state).contains("outside the finite wall span"),
        "{}",
        refusal(&state)
    );
    assert!(sess(&state).authoritative.entities().is_empty());
}

#[test]
fn numeric_slot_previews_purely_and_adds_diameter_constraint() {
    let mut state = staged([[0.0, 0.0], [20.0, 0.0]]);
    state.dispatch(SketchAction::Move(point(&state, 10.0, 4.0)));
    let (before, draft) = (
        sess(&state).authoritative.clone(),
        sess(&state).draft.clone(),
    );
    let draft = draft.unwrap();
    dims(&mut state, 6.0, None);
    assert!(sess(&state).authoritative == before);
    let after = sess(&state).draft.as_ref().unwrap();
    assert!(after.anchor == draft.anchor && after.cursor == draft.cursor);
    let Some(DraftPrimitive::Slot { radius, .. }) = sess(&state).preview else {
        panic!("numeric slot preview")
    };
    assert!((radius - 3.0).abs() < 1e-9);
    let (_, ops) = upsert(dims(&mut state, 6.0, Some(1000)));
    assert_eq!(ops.len(), 20);
    let g = geometry(&ops);
    assert!(g.arcs.iter().all(|a| (a.2 - 3.0).abs() < 1e-9));
    assert!(
        matches!(&ops[19], SketchEditOp::AddConstraint { constraint: Constraint::Diameter { id, entity, value } } if *id == ids(1000).constraints[9] && *entity == ids(1000).entities[8] && value.value == 6.0)
    );
}

#[test]
fn numeric_slot_zero_width_is_refused() {
    let mut state = staged([[0.0, 0.0], [20.0, 0.0]]);
    let before = sess(&state).authoritative.clone();
    assert!(dims(&mut state, 0.0, Some(1000)).is_empty());
    assert!(
        refusal(&state).contains("Slot width"),
        "{}",
        refusal(&state)
    );
    assert!(sess(&state).authoritative == before);
}
