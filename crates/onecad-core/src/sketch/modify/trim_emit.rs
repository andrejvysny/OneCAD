//! Trim: turning surviving spans into entities, constraints and one op batch.

use std::f64::consts::TAU;

use super::common::{orphaned, weld, Crossing};
use super::geom::{normalize_angle, Pt};
use super::remap::{remap, RemapInput, RemapOutput};
use super::trim::{Cut, Span, SpanEnd, Target};
use super::{AddReason, AddedConstraint, ModifyIds, ModifyPlan, PlannedPiece};
use crate::edit::SketchEditOp;
use crate::ids::EntityId;
use crate::sketch::{Constraint, CurvePosition, Sketch, SketchEntity};

/// Entities minted for the surviving spans.
#[derive(Default)]
struct Minted {
    pieces: Vec<PlannedPiece>,
    entities: Vec<SketchEntity>,
    points: Vec<SketchEntity>,
    /// `(endpoint reference, crossing)` for every cut end.
    cuts: Vec<((EntityId, CurvePosition), Crossing)>,
}

impl Minted {
    /// Existing point entities a cut weld ties a piece endpoint to.
    fn welded_points(&self) -> Vec<EntityId> {
        self.cuts
            .iter()
            .filter_map(|(_, c)| match c.at_endpoint {
                Some((p, CurvePosition::Arbitrary)) => Some(p),
                _ => None,
            })
            .collect()
    }
}

pub(super) fn build(
    sketch: &Sketch,
    entity: &SketchEntity,
    target: &Target,
    cut: &Cut,
    ids: &mut dyn ModifyIds,
) -> ModifyPlan {
    let id = entity.id();
    let minted = mint(sketch, entity, target, cut, ids);
    let removed_points = removed_points(sketch, id, target, &minted.pieces);
    let welded_points = minted.welded_points();
    let locate = |p: Pt| locate(target, &cut.spans, p);
    let line_ends = match *target {
        Target::Line { start, end, .. } => Some((start, end)),
        _ => None,
    };
    let remapped = remap(
        &RemapInput {
            sketch,
            target: id,
            target_curve: target.curve(),
            pieces: &minted.pieces,
            removed_points: &removed_points,
            welded_points: &welded_points,
            line_ends,
            locate: &locate,
        },
        ids,
    );
    let welds: Vec<Constraint> = minted
        .cuts
        .iter()
        .map(|(end, crossing)| weld(ids.constraint_id(), *end, crossing))
        .collect();
    assemble(id, minted, removed_points, remapped, welds)
}

fn assemble(
    id: EntityId,
    minted: Minted,
    removed_points: Vec<EntityId>,
    mut remapped: RemapOutput,
    welds: Vec<Constraint>,
) -> ModifyPlan {
    for (weld, (_, crossing)) in welds.iter().zip(&minted.cuts) {
        remapped.added.push(AddedConstraint {
            id: weld.id(),
            reason: AddReason::CutWeld {
                boundary: crossing.boundary,
            },
        });
    }
    let ops = batch(id, &minted, &removed_points, &remapped, welds);
    let mut removed_entities = Vec::new();
    if !minted.pieces.iter().any(|p| p.keeps_target_id) {
        removed_entities.push(id);
    }
    removed_entities.extend(removed_points);
    ModifyPlan {
        ops,
        pieces: minted.pieces,
        removed_entities,
        added_points: minted.points.iter().map(SketchEntity::id).collect(),
        kept_constraints: remapped.kept,
        added_constraints: remapped.added,
        dropped_constraints: remapped.dropped,
    }
}

/// Apply order: constraints out, target and orphans out, points then pieces
/// in, remapped constraints then welds in.
fn batch(
    id: EntityId,
    minted: &Minted,
    removed_points: &[EntityId],
    remapped: &RemapOutput,
    welds: Vec<Constraint>,
) -> Vec<SketchEditOp> {
    let mut ops: Vec<SketchEditOp> = remapped
        .removed
        .iter()
        .map(|&constraint| SketchEditOp::RemoveConstraint { constraint })
        .collect();
    ops.push(SketchEditOp::RemoveEntity { entity: id });
    ops.extend(
        removed_points
            .iter()
            .map(|&entity| SketchEditOp::RemoveEntity { entity }),
    );
    ops.extend(
        minted
            .points
            .iter()
            .chain(&minted.entities)
            .map(|e| SketchEditOp::AddEntity { entity: e.clone() }),
    );
    ops.extend(
        remapped
            .emitted
            .iter()
            .cloned()
            .chain(welds)
            .map(|constraint| SketchEditOp::AddConstraint { constraint }),
    );
    ops
}

/// Mints piece entities (and a line's cut points) span by span, in order.
fn mint(
    sketch: &Sketch,
    entity: &SketchEntity,
    target: &Target,
    cut: &Cut,
    ids: &mut dyn ModifyIds,
) -> Minted {
    let mut m = Minted::default();
    for span in &cut.spans {
        let keeps = span.start == SpanEnd::Original && !matches!(target, Target::Circle { .. });
        let piece = if keeps { entity.id() } else { ids.entity_id() };
        let new = match *target {
            Target::Line { start, end, .. } => {
                let s = line_end(sketch, start, span.start, cut, ids, &mut m);
                let e = line_end(sketch, end, span.end, cut, ids, &mut m);
                SketchEntity::line(piece, s, e, entity.is_construction())
            }
            Target::Arc {
                center, r, start, ..
            } => arc_piece(
                &mut m,
                piece,
                (center, r, start),
                span,
                cut,
                entity.is_construction(),
            ),
            Target::Circle { center, r, .. } => arc_piece(
                &mut m,
                piece,
                (center, r, 0.0),
                span,
                cut,
                entity.is_construction(),
            ),
        };
        m.pieces.push(PlannedPiece {
            id: piece,
            keeps_target_id: keeps,
            owns_start: span.start == SpanEnd::Original,
            owns_end: span.end == SpanEnd::Original,
        });
        m.entities.push(new);
    }
    m
}

/// A line piece end: the original point, or a fresh point at the cut.
fn line_end(
    sketch: &Sketch,
    original: EntityId,
    end: SpanEnd,
    cut: &Cut,
    ids: &mut dyn ModifyIds,
    m: &mut Minted,
) -> EntityId {
    let SpanEnd::Cut(i) = end else {
        return original;
    };
    let crossing = cut.crossings[i];
    let id = ids.entity_id();
    let construction = sketch
        .get_entity(original)
        .is_some_and(SketchEntity::is_construction);
    let at = crossing
        .p
        .to_vec2()
        .expect("crossings are filtered to finite points");
    m.points
        .push(SketchEntity::point(id, at, construction, false));
    m.cuts.push(((id, CurvePosition::Arbitrary), crossing));
    id
}

/// An arc piece over `span`, angles relative to `base` (the arc's start, or 0
/// for a circle's absolute angles); cut ends are the arc's own Start/End roles.
fn arc_piece(
    m: &mut Minted,
    piece: EntityId,
    (center, r, base): (EntityId, f64, f64),
    span: &Span,
    cut: &Cut,
    construction: bool,
) -> SketchEntity {
    for (end, role) in [
        (span.start, CurvePosition::Start),
        (span.end, CurvePosition::End),
    ] {
        if let SpanEnd::Cut(i) = end {
            m.cuts.push(((piece, role), cut.crossings[i]));
        }
    }
    SketchEntity::Arc {
        id: piece,
        center,
        radius: r,
        start_angle: normalize_angle(base + span.from),
        end_angle: normalize_angle(base + span.to),
        construction,
        reference_locked: false,
    }
}

/// Defining points no surviving piece uses and no other entity is built from:
/// a line's trimmed-away end points, or a wholly deleted curve's centre.
fn removed_points(
    sketch: &Sketch,
    id: EntityId,
    target: &Target,
    pieces: &[PlannedPiece],
) -> Vec<EntityId> {
    let candidates = match *target {
        Target::Line { start, end, .. } => {
            let mut v = Vec::new();
            if !pieces.iter().any(|p| p.owns_start) {
                v.push(start);
            }
            if !pieces.iter().any(|p| p.owns_end) {
                v.push(end);
            }
            v
        }
        Target::Arc { center, .. } | Target::Circle { center, .. } if pieces.is_empty() => {
            vec![center]
        }
        _ => Vec::new(),
    };
    candidates
        .into_iter()
        .filter(|p| orphaned(sketch, *p, id))
        .collect()
}

/// Index of the span holding `p` (a point on the target's carrier).
fn locate(target: &Target, spans: &[Span], p: Pt) -> Option<usize> {
    let tol = target.tol();
    let raw = target.param(p);
    spans.iter().position(|s| {
        let t = if matches!(target, Target::Circle { .. }) && raw < s.from - tol {
            raw + TAU
        } else {
            raw
        };
        t >= s.from - tol && t <= s.to + tol
    })
}
