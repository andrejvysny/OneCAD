//! Trim/Extend planner tests: fixtures, the real apply path and the accounting
//! invariants every plan must satisfy.

mod extend;
mod props;
mod remap;
mod trim;
mod trim_overlap;

use std::collections::BTreeSet;

use uuid::Uuid;

use super::{AddReason, ModifyIds, ModifyPlan};
use crate::document::Document;
use crate::edit::{DocumentSession, SketchEditOp};
use crate::ids::{ConstraintId, DocumentId, EntityId, SketchId};
use crate::math::Vec2;
use crate::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

/// Deterministic allocator that counts its draws.
pub(super) struct SeqIds {
    next: u128,
    pub drawn: usize,
}

impl SeqIds {
    pub fn new() -> Self {
        Self {
            next: 0xF000_0000,
            drawn: 0,
        }
    }
}

impl ModifyIds for SeqIds {
    fn entity_id(&mut self) -> EntityId {
        self.next += 1;
        self.drawn += 1;
        EntityId(Uuid::from_u128(self.next))
    }
    fn constraint_id(&mut self) -> ConstraintId {
        self.next += 1;
        self.drawn += 1;
        ConstraintId(Uuid::from_u128(self.next))
    }
}

/// Sketch builder with sequential ids.
pub(super) struct B {
    pub s: Sketch,
    n: u128,
}

impl B {
    pub fn new() -> Self {
        Self {
            s: Sketch::on_world_plane(SketchId(Uuid::from_u128(1)), "S", WorldPlane::XY),
            n: 0x100,
        }
    }
    fn next(&mut self) -> Uuid {
        self.n += 1;
        Uuid::from_u128(self.n)
    }
    pub fn cid(&mut self) -> ConstraintId {
        ConstraintId(self.next())
    }
    pub fn add(&mut self, e: SketchEntity) -> EntityId {
        let id = e.id();
        self.s.add_entity(e).unwrap();
        id
    }
    pub fn pt(&mut self, x: f64, y: f64) -> EntityId {
        let id = EntityId(self.next());
        self.add(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
    }
    pub fn seg(&mut self, s: EntityId, e: EntityId) -> EntityId {
        let id = EntityId(self.next());
        self.add(SketchEntity::line(id, s, e, false))
    }
    /// `(line, start point, end point)`.
    pub fn line(&mut self, a: (f64, f64), b: (f64, f64)) -> (EntityId, EntityId, EntityId) {
        let s = self.pt(a.0, a.1);
        let e = self.pt(b.0, b.1);
        (self.seg(s, e), s, e)
    }
    /// `(circle, center)`.
    pub fn circle(&mut self, c: (f64, f64), r: f64) -> (EntityId, EntityId) {
        let center = self.pt(c.0, c.1);
        let id = EntityId(self.next());
        (
            self.add(SketchEntity::circle(id, center, r, false).unwrap()),
            center,
        )
    }
    /// `(arc, center)`.
    pub fn arc(&mut self, c: (f64, f64), r: f64, a0: f64, a1: f64) -> (EntityId, EntityId) {
        let center = self.pt(c.0, c.1);
        let id = EntityId(self.next());
        (
            self.add(SketchEntity::arc(id, center, r, a0, a1, false).unwrap()),
            center,
        )
    }
    pub fn ellipse(&mut self, c: (f64, f64), major: f64, minor: f64, rot: f64) -> EntityId {
        let center = self.pt(c.0, c.1);
        let id = EntityId(self.next());
        self.add(SketchEntity::ellipse(id, center, major, minor, rot, false).unwrap())
    }
    pub fn constrain(&mut self, c: Constraint) -> ConstraintId {
        let id = c.id();
        self.s.add_constraint(c).unwrap();
        id
    }
    pub fn horizontal(&mut self, line: EntityId) -> ConstraintId {
        let id = self.cid();
        self.constrain(Constraint::Horizontal { id, line })
    }
    pub fn fixed(&mut self, point: EntityId, role: CurvePosition) -> ConstraintId {
        let id = self.cid();
        let at = Vec2::new_unchecked(0.0, 0.0);
        self.constrain(Constraint::Fixed {
            id,
            point,
            point_position: role,
            at,
        })
    }
    pub fn coincident(
        &mut self,
        (point1, point1_position): (EntityId, CurvePosition),
        (point2, point2_position): (EntityId, CurvePosition),
    ) -> ConstraintId {
        let id = self.cid();
        self.constrain(Constraint::Coincident {
            id,
            point1,
            point2,
            point1_position,
            point2_position,
        })
    }
    pub fn on_curve(&mut self, point: EntityId, curve: EntityId) -> ConstraintId {
        self.on_curve_at(point, CurvePosition::Arbitrary, curve)
    }
    /// `OnCurve` naming the point by role (`Start`/`End` of a line or arc).
    pub fn on_curve_at(
        &mut self,
        point: EntityId,
        point_position: CurvePosition,
        curve: EntityId,
    ) -> ConstraintId {
        let id = self.cid();
        self.constrain(Constraint::OnCurve {
            id,
            point,
            point_position,
            curve,
            position: CurvePosition::Arbitrary,
        })
    }
}

/// A junction at (8,0) on `target`: a stem's foot point and an arc's `Start`
/// tied by a `Coincident`, created stem first unless `arc_first`. Returns the
/// `OnCurve` holding the junction on `target`, which names the arc `Start`
/// (`name_arc`) or the stem's foot point.
pub(super) fn junction(
    b: &mut B,
    target: EntityId,
    arc_first: bool,
    name_arc: bool,
) -> ConstraintId {
    let arc_at = |b: &mut B| b.arc((11.0, 0.0), 3.0, std::f64::consts::PI, 4.7).0;
    let early_arc = arc_first.then(|| arc_at(b));
    let (_, _, foot) = b.line((8.0, 5.0), (8.0, 0.0));
    let arc = early_arc.unwrap_or_else(|| arc_at(b));
    let arc_start = (arc, CurvePosition::Start);
    b.coincident((foot, CurvePosition::Arbitrary), arc_start);
    if name_arc {
        b.on_curve_at(arc, CurvePosition::Start, target)
    } else {
        b.on_curve(foot, target)
    }
}

/// A stem foot at (8,0) on `target` that is also the centre of an arc above
/// the line, tied by a `Coincident` naming the arc's `Center` role
/// (`role_in_weld`) or its centre point. Returns the `OnCurve` holding the
/// junction on `target`, which uses the other spelling of the centre.
pub(super) fn centre_junction(b: &mut B, target: EntityId, role_in_weld: bool) -> ConstraintId {
    let (_, _, foot) = b.line((8.0, 5.0), (8.0, 0.0));
    let (arc, centre) = b.arc((8.0, 0.0), 2.0, 0.3, 1.2);
    let (role, point) = (
        (arc, CurvePosition::Center),
        (centre, CurvePosition::Arbitrary),
    );
    let (in_weld, in_on_curve) = if role_in_weld {
        (role, point)
    } else {
        (point, role)
    };
    b.coincident((foot, CurvePosition::Arbitrary), in_weld);
    b.on_curve_at(in_on_curve.0, in_on_curve.1, target)
}

pub(super) fn v(x: f64, y: f64) -> Vec2 {
    Vec2::new_unchecked(x, y)
}

/// Applies `plan` through the real edit-session validation path.
pub(super) fn apply(sketch: &Sketch, plan: &ModifyPlan) -> Sketch {
    let mut doc = Document::new(DocumentId(Uuid::from_u128(9)));
    doc.sketches.insert(sketch.id, sketch.clone());
    DocumentSession::new(doc)
        .prepare_sketch_edit(sketch.id, &plan.ops)
        .unwrap_or_else(|e| panic!("plan must apply: {e:?}\n{plan:#?}"))
}

pub(super) fn point(s: &Sketch, id: EntityId) -> Vec2 {
    match s.get_entity(id) {
        Some(SketchEntity::Point { at, .. }) => *at,
        other => panic!("{id} is not a point: {other:?}"),
    }
}

/// `(start, end)` positions of a line.
pub(super) fn ends(s: &Sketch, line: EntityId) -> (Vec2, Vec2) {
    match s.get_entity(line) {
        Some(SketchEntity::Line { start, end, .. }) => (point(s, *start), point(s, *end)),
        other => panic!("{line} is not a line: {other:?}"),
    }
}

pub(super) fn arc_angles(s: &Sketch, arc: EntityId) -> (f64, f64) {
    match s.get_entity(arc) {
        Some(SketchEntity::Arc {
            start_angle,
            end_angle,
            ..
        }) => (*start_angle, *end_angle),
        other => panic!("{arc} is not an arc: {other:?}"),
    }
}

pub(super) fn near(a: Vec2, x: f64, y: f64) -> bool {
    a.approx_eq(&v(x, y), 1e-9)
}

pub(super) fn dropped_reason(plan: &ModifyPlan, id: ConstraintId) -> Option<super::DropReason> {
    plan.dropped_constraints
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.reason)
}

/// The invariants every applied plan must satisfy: total constraint
/// accounting, untouched geometry untouched, fresh ids fresh.
/// `moved` lists entities other than the target the plan may legitimately
/// change in place (an extended line's end point).
pub(super) fn check_accounting(
    before: &Sketch,
    after: &Sketch,
    plan: &ModifyPlan,
    target: EntityId,
    moved: &[EntityId],
) {
    let removed: BTreeSet<ConstraintId> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            SketchEditOp::RemoveConstraint { constraint } => Some(*constraint),
            _ => None,
        })
        .collect();
    let kept: BTreeSet<_> = plan.kept_constraints.iter().copied().collect();
    let dropped: BTreeSet<_> = plan.dropped_constraints.iter().map(|d| d.id).collect();
    let added: BTreeSet<_> = plan.added_constraints.iter().map(|a| a.id).collect();
    assert!(kept.is_disjoint(&dropped), "kept and dropped overlap");
    assert_eq!(
        kept.union(&dropped).copied().collect::<BTreeSet<_>>(),
        removed,
        "every removed constraint is kept or dropped"
    );
    for c in before.constraints() {
        if removed.contains(&c.id()) {
            assert_eq!(after.contains_constraint(c.id()), kept.contains(&c.id()));
        } else {
            assert_eq!(
                after.get_constraint(c.id()),
                Some(c),
                "untouched constraint changed"
            );
        }
        assert!(
            !added.contains(&c.id()),
            "added id collides with an existing one"
        );
    }
    let expected: BTreeSet<_> = before
        .constraints()
        .iter()
        .map(Constraint::id)
        .filter(|id| !dropped.contains(id))
        .chain(added.iter().copied())
        .collect();
    let actual: BTreeSet<_> = after.constraints().iter().map(Constraint::id).collect();
    assert_eq!(
        actual, expected,
        "result constraints = untouched + kept + added"
    );
    check_entities(before, after, plan, target, moved);
    check_no_restated_weld(after, plan);
}

type PointRef = (EntityId, CurvePosition);

/// Canonical point reference, independent of the planner's `point_key`: a
/// line `Start`/`End` role and an arc/circle/ellipse `Center` role name their
/// point entity.
fn canon(s: &Sketch, entity: EntityId, role: CurvePosition) -> PointRef {
    let point = match (s.get_entity(entity), role) {
        (Some(SketchEntity::Line { start, .. }), CurvePosition::Start) => Some(*start),
        (Some(SketchEntity::Line { end, .. }), CurvePosition::End) => Some(*end),
        (
            Some(
                SketchEntity::Arc { center, .. }
                | SketchEntity::Circle { center, .. }
                | SketchEntity::Ellipse { center, .. },
            ),
            CurvePosition::Center,
        ) => Some(*center),
        _ => None,
    };
    point.map_or((entity, role), |p| (p, CurvePosition::Arbitrary))
}

/// Union-find over every `Coincident` of a sketch, on canonical references.
struct Welded {
    refs: Vec<PointRef>,
    parent: Vec<usize>,
}

impl Welded {
    fn of(s: &Sketch) -> Self {
        let mut w = Self {
            refs: Vec::new(),
            parent: Vec::new(),
        };
        for c in s.constraints() {
            if let Constraint::Coincident {
                point1,
                point2,
                point1_position,
                point2_position,
                ..
            } = *c
            {
                let a = w.root(canon(s, point1, point1_position));
                let b = w.root(canon(s, point2, point2_position));
                w.parent[a] = b;
            }
        }
        w
    }

    fn root(&mut self, r: PointRef) -> usize {
        let mut i = self.refs.iter().position(|x| *x == r).unwrap_or_else(|| {
            self.refs.push(r);
            self.parent.push(self.refs.len() - 1);
            self.refs.len() - 1
        });
        while self.parent[i] != i {
            i = self.parent[i];
        }
        i
    }
}

/// No `OnCurve` in the result restates a `Coincident` weld the plan added: a
/// weld tying a piece end to point P makes `OnCurve(Q, piece)` the same
/// equation twice for P and every Q a `Coincident` chain ties to P, which the
/// worker solver reports as over-constrained. Point references compare
/// canonically (line and centre roles are their point entities).
pub(super) fn check_no_restated_weld(after: &Sketch, plan: &ModifyPlan) {
    let mut welded = Welded::of(after);
    let pieces: Vec<EntityId> = plan.pieces.iter().map(|p| p.id).collect();
    for added in &plan.added_constraints {
        if !matches!(
            added.reason,
            AddReason::CutWeld { .. } | AddReason::ExtendWeld { .. }
        ) {
            continue;
        }
        let Some(Constraint::Coincident {
            point1,
            point1_position,
            ..
        }) = after.get_constraint(added.id)
        else {
            continue;
        };
        let weld = welded.root(canon(after, *point1, *point1_position));
        for c in after.constraints() {
            if let Constraint::OnCurve {
                point,
                point_position,
                curve,
                ..
            } = c
            {
                assert!(
                    !(pieces.contains(curve)
                        && welded.root(canon(after, *point, *point_position)) == weld),
                    "{c:?} restates weld {}",
                    added.id
                );
            }
        }
    }
}

fn check_entities(
    before: &Sketch,
    after: &Sketch,
    plan: &ModifyPlan,
    target: EntityId,
    moved: &[EntityId],
) {
    for e in before.entities() {
        let id = e.id();
        if plan.removed_entities.contains(&id) {
            assert!(!after.contains_entity(id), "removed entity {id} survived");
        } else if id != target && !moved.contains(&id) {
            assert_eq!(after.get_entity(id), Some(e), "entity {id} changed");
        }
    }
    for piece in &plan.pieces {
        assert!(after.contains_entity(piece.id));
        assert_eq!(piece.keeps_target_id, piece.id == target);
        if !piece.keeps_target_id {
            assert!(!before.contains_entity(piece.id), "fresh piece id reused");
        }
    }
    for p in &plan.added_points {
        assert!(!before.contains_entity(*p) && after.contains_entity(*p));
    }
    assert_eq!(
        after.contains_entity(target),
        plan.pieces.iter().any(|p| p.keeps_target_id)
    );
}
