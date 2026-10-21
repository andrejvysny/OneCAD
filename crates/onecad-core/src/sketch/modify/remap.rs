//! Constraint remap table for a trim (ONECAD-19 intent preservation).
//!
//! Every constraint that names the trimmed entity, a removed point, or the
//! trimmed line's own length is removed from the sketch and then either
//! re-added under its id with references moved to a piece ("kept"), copied to
//! further pieces under fresh ids ("added"), or dropped with a reason. The
//! table (integrator decision, ONECAD-293):
//!
//! | constraint | rule |
//! |---|---|
//! | endpoint / centre role (`Start`/`End`/`Center`) | follows the piece owning that point; dropped if none |
//! | Horizontal, Vertical, Parallel, Perpendicular (line) | copied to every line piece |
//! | Radius, Diameter (arc/circle) | copied to every arc piece |
//! | Concentric (arc/circle) | first arc piece only — pieces share one centre point, so a copy would restate the same equation |
//! | Tangent | kept on the piece holding the contact point; dropped if it was trimmed away |
//! | OnCurve (anywhere on the target) | kept on the piece holding the point; dropped if trimmed away, or if a cut now welds that point — or any point a `Coincident` chain ties it to — with a `Coincident` (however the point is named: point entity, line role, arc role or centre role) |
//! | Equal on a line, end-to-end Distance | dropped — the measured length changed |
//! | Midpoint of the line | dropped — the midpoint moved |
//! | Equal (arc/circle), Angle, Symmetric axis, point/curve Distance | kept when exactly one piece survives; dropped on a split |
//! | anything else naming the whole entity | dropped as unsupported |

use super::geom::{point_key, ref_point, Curve, Pt};
use super::slots::{map_refs, refs, Slot};
use super::{
    AddReason, AddedConstraint, DropReason, DroppedConstraint, ModifyIds, PlannedPiece,
    COINCIDENT_MM,
};
use crate::ids::{ConstraintId, EntityId};
use crate::sketch::{Constraint, CurvePosition, Sketch};

/// What a trim did to the target, for remapping its constraints.
pub(super) struct RemapInput<'a> {
    pub sketch: &'a Sketch,
    pub target: EntityId,
    pub target_curve: Curve,
    pub pieces: &'a [PlannedPiece],
    /// Points the batch removes.
    pub removed_points: &'a [EntityId],
    /// Existing points a cut `Coincident` now ties to a piece end (each
    /// welded boundary endpoint's whole `Coincident` class), as canonical
    /// references ([`point_key`]).
    pub welded_points: &'a [(EntityId, CurvePosition)],
    /// A line target's `(start, end)` point ids.
    pub line_ends: Option<(EntityId, EntityId)>,
    /// Index of the piece containing a point on the target's carrier.
    pub locate: &'a dyn Fn(Pt) -> Option<usize>,
}

/// Remap outcome: what to remove, what to re-add, and the accounting.
#[derive(Debug, Default)]
pub(super) struct RemapOutput {
    pub removed: Vec<ConstraintId>,
    pub emitted: Vec<Constraint>,
    pub kept: Vec<ConstraintId>,
    pub added: Vec<AddedConstraint>,
    pub dropped: Vec<DroppedConstraint>,
}

pub(super) fn remap(input: &RemapInput, ids: &mut dyn ModifyIds) -> RemapOutput {
    let mut out = RemapOutput::default();
    for c in input.sketch.constraints() {
        let slots = refs(c);
        let names = |e: EntityId| slots.iter().any(|(r, _)| *r == e);
        let verdict = if input.removed_points.iter().any(|p| names(*p)) {
            Err(DropReason::PointRemoved)
        } else if names(input.target) {
            place(input, c, &slots)
        } else if is_line_length(c, input.line_ends) {
            Err(DropReason::LengthChanged)
        } else {
            continue;
        };
        out.removed.push(c.id());
        match verdict {
            Ok(copies) => emit(&mut out, c, copies, ids),
            Err(reason) => out.dropped.push(DroppedConstraint { id: c.id(), reason }),
        }
    }
    out
}

/// Records `copies` (rewritten, all still under `c`'s id): the first keeps
/// the id, the rest get fresh ids.
fn emit(out: &mut RemapOutput, c: &Constraint, copies: Vec<Constraint>, ids: &mut dyn ModifyIds) {
    for (i, copy) in copies.into_iter().enumerate() {
        if i == 0 {
            out.kept.push(c.id());
            out.emitted.push(copy);
            continue;
        }
        let id = ids.constraint_id();
        out.emitted.push(map_refs(&copy, id, &mut |e, _| e));
        out.added.push(AddedConstraint {
            id,
            reason: AddReason::CopiedFrom { of: c.id() },
        });
    }
}

/// The rewritten copies of `c` (one per receiving piece), or why it drops.
fn place(
    input: &RemapInput,
    c: &Constraint,
    slots: &[(EntityId, Slot)],
) -> Result<Vec<Constraint>, DropReason> {
    if input.pieces.is_empty() {
        return Err(DropReason::EntityDeleted);
    }
    let on_target = |slot: Slot| slots.iter().any(|(e, s)| *e == input.target && *s == slot);
    let end_to_end =
        on_target(Slot::Role(CurvePosition::Start)) && on_target(Slot::Role(CurvePosition::End));
    if matches!(c, Constraint::Distance { .. }) && end_to_end {
        return Err(DropReason::LengthChanged);
    }
    let receivers = if on_target(Slot::Whole) {
        whole_rule(input, c)?
    } else {
        vec![0]
    };
    receivers
        .into_iter()
        .map(|piece| rewrite(input, c, piece))
        .collect()
}

/// `c` with every reference to the target moved: whole-entity slots to
/// `pieces[piece]`, roles to the piece owning that point.
fn rewrite(input: &RemapInput, c: &Constraint, piece: usize) -> Result<Constraint, DropReason> {
    let mut lost = false;
    let out = map_refs(c, c.id(), &mut |e, slot| {
        if e != input.target {
            return e;
        }
        let owner = match slot {
            Slot::Whole => Some(&input.pieces[piece]),
            Slot::Role(CurvePosition::Start) => input.pieces.iter().find(|p| p.owns_start),
            Slot::Role(CurvePosition::End) => input.pieces.iter().find(|p| p.owns_end),
            Slot::Role(CurvePosition::Center) if input.line_ends.is_none() => input.pieces.first(),
            Slot::Role(_) => None,
        };
        owner.map_or_else(
            || {
                lost = true;
                e
            },
            |p| p.id,
        )
    });
    if lost {
        Err(DropReason::PointRemoved)
    } else {
        Ok(out)
    }
}

/// Which pieces receive a constraint naming the target as a whole.
fn whole_rule(input: &RemapInput, c: &Constraint) -> Result<Vec<usize>, DropReason> {
    let line = input.line_ends.is_some();
    let all = (0..input.pieces.len()).collect::<Vec<_>>();
    let unsplit = || {
        if input.pieces.len() == 1 {
            Ok(vec![0])
        } else {
            Err(DropReason::SplitAmbiguous)
        }
    };
    match c {
        Constraint::Horizontal { .. }
        | Constraint::Vertical { .. }
        | Constraint::Parallel { .. }
        | Constraint::Perpendicular { .. }
            if line =>
        {
            Ok(all)
        }
        Constraint::Radius { .. } | Constraint::Diameter { .. } if !line => Ok(all),
        Constraint::Concentric { .. } if !line => Ok(vec![0]),
        Constraint::Equal { .. } if line => Err(DropReason::LengthChanged),
        Constraint::Midpoint { .. } if line => Err(DropReason::MidpointMoved),
        Constraint::Equal { .. } | Constraint::Distance { .. } => unsplit(),
        Constraint::Angle { .. } | Constraint::Symmetric { .. } if line => unsplit(),
        Constraint::OnCurve {
            point,
            point_position,
            ..
        } => on_curve(input, *point, *point_position),
        Constraint::Tangent {
            entity1, entity2, ..
        } => {
            let other = if *entity1 == input.target {
                *entity2
            } else {
                *entity1
            };
            tangent(input, other)
        }
        _ => Err(DropReason::Unsupported),
    }
}

fn on_curve(
    input: &RemapInput,
    point: EntityId,
    role: CurvePosition,
) -> Result<Vec<usize>, DropReason> {
    if point == input.target {
        return Err(DropReason::Unsupported);
    }
    if input
        .welded_points
        .contains(&point_key(input.sketch, point, role))
    {
        return Err(DropReason::SupersededByCutWeld);
    }
    let at = ref_point(input.sketch, point, role).ok_or(DropReason::Unsupported)?;
    (input.locate)(at)
        .map(|i| vec![i])
        .ok_or(DropReason::OnRemovedSpan)
}

fn tangent(input: &RemapInput, other: EntityId) -> Result<Vec<usize>, DropReason> {
    let other = input
        .sketch
        .get_entity(other)
        .and_then(|e| super::geom::curve_of(input.sketch, e))
        .ok_or(DropReason::Unsupported)?;
    let at = contact(&input.target_curve, &other).ok_or(DropReason::Unsupported)?;
    (input.locate)(at)
        .map(|i| vec![i])
        .ok_or(DropReason::OnRemovedSpan)
}

/// The tangency contact point on `target`'s carrier, for line/circle pairs.
fn contact(target: &Curve, other: &Curve) -> Option<Pt> {
    let foot = |a: Pt, b: Pt, c: Pt| {
        let d = b - a;
        a + d * ((c - a).dot(d) / d.dot(d))
    };
    match (*target, *other) {
        (Curve::Line { a, b }, Curve::Circle { c, .. } | Curve::Arc { c, .. }) => {
            Some(foot(a, b, c))
        }
        (Curve::Circle { c, r } | Curve::Arc { c, r, .. }, Curve::Line { a, b }) => {
            let f = foot(a, b, c);
            let d = f.dist(c);
            (d > COINCIDENT_MM).then(|| c + (f - c) * (r / d))
        }
        (
            Curve::Circle { c, r } | Curve::Arc { c, r, .. },
            Curve::Circle { c: c2, r: r2 } | Curve::Arc { c: c2, r: r2, .. },
        ) => {
            let d = c.dist(c2);
            if d <= COINCIDENT_MM {
                return None;
            }
            let u = (c2 - c) * (1.0 / d);
            let external = (d - (r + r2)).abs() <= (d - (r - r2).abs()).abs();
            let toward = external || r >= r2;
            Some(if toward { c + u * r } else { c - u * r })
        }
        _ => None,
    }
}

/// A line-length `Distance` between the target line's own end points.
fn is_line_length(c: &Constraint, ends: Option<(EntityId, EntityId)>) -> bool {
    let (Some((s, e)), Constraint::Distance { .. }) = (ends, c) else {
        return false;
    };
    let slots = refs(c);
    slots.len() == 2
        && slots.iter().all(|(_, slot)| *slot == Slot::Whole)
        && ((slots[0].0 == s && slots[1].0 == e) || (slots[0].0 == e && slots[1].0 == s))
}
