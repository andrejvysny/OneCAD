//! Extend: grow a line/arc end along its continuation to the nearest boundary.

use std::f64::consts::TAU;

use super::common::{coincident_class, crossings, modifiable, representative, weld, Crossing};
use super::geom::{
    angle_of, arc_param, curve_of, line_param, normalize_angle, on_circle, point_key, Curve, Pt,
};
use super::slots::{refs, Slot};
use super::{
    AddReason, AddedConstraint, DropReason, DroppedConstraint, ExtendPlan, ExtendRequest,
    ModifyIds, ModifyPlan, ModifyRefusal, PlannedPiece, COINCIDENT_MM, MIN_PIECE_MM,
};
use crate::edit::SketchEditOp;
use crate::ids::{ConstraintId, EntityId};
use crate::math::Vec2;
use crate::sketch::{Constraint, CurvePosition, Sketch, SketchEntity};

/// Plans extending the end of `request.entity` nearer `request.pick`.
///
/// # Errors
/// A [`ModifyRefusal`] when the target is missing, a point, locked, a circle or
/// ellipse, degenerate, picked ambiguously (mid-way, half-way around an arc's
/// gap, at the centre), its end is shared or held by a constraint, already lies
/// on a same-carrier boundary, or its continuation meets no boundary / only a
/// tangent one / one closer than [`MIN_PIECE_MM`]. A refusal draws no id from
/// `ids`.
pub fn plan_extend(
    sketch: &Sketch,
    request: &ExtendRequest,
    ids: &mut dyn ModifyIds,
) -> Result<ExtendPlan, ModifyRefusal> {
    let entity = modifiable(sketch, request.entity)?;
    let id = entity.id();
    let pick = Pt::from(request.pick);
    match (curve_of(sketch, entity), entity) {
        (Some(Curve::Line { a, b }), SketchEntity::Line { start, end, .. }) => {
            extend_line(sketch, id, (a, b), (*start, *end), pick, ids)
        }
        (Some(Curve::Arc { c, r, start, sweep }), SketchEntity::Arc { .. }) => {
            extend_arc(sketch, entity, (c, r, start, sweep), pick, ids)
        }
        (_, SketchEntity::Circle { .. } | SketchEntity::Ellipse { .. }) => {
            Err(ModifyRefusal::UnsupportedKind {
                entity: id,
                kind: if matches!(entity, SketchEntity::Circle { .. }) {
                    "closed circle"
                } else {
                    "ellipse"
                },
                action: "extended",
            })
        }
        _ => Err(ModifyRefusal::DegenerateTarget(id)),
    }
}

fn extend_line(
    sketch: &Sketch,
    id: EntityId,
    (a, b): (Pt, Pt),
    (start, end): (EntityId, EntityId),
    pick: Pt,
    ids: &mut dyn ModifyIds,
) -> Result<ExtendPlan, ModifyRefusal> {
    let len = a.dist(b);
    if len < MIN_PIECE_MM {
        return Err(ModifyRefusal::DegenerateTarget(id));
    }
    let tol = COINCIDENT_MM / len;
    let role = pick_end(line_param(a, b, pick), 1.0, false, tol, id)?;
    let point = if role == CurvePosition::Start {
        start
    } else {
        end
    };
    check_free(sketch, id, role, Some(point))?;
    let ahead = |p: Pt| {
        let t = line_param(a, b, p);
        if role == CurvePosition::End {
            t - 1.0
        } else {
            -t
        }
    };
    let curve = Curve::Line { a, b };
    let raw = crossings(sketch, id, &curve, false, &ahead);
    let end = if role == CurvePosition::End { b } else { a };
    let hit = nearest(sketch, raw, end, (tol, f64::INFINITY), id)?;
    if hit.param * len < MIN_PIECE_MM {
        return Err(ModifyRefusal::DegeneratePiece(id));
    }
    let tip = hit.p.to_vec2().ok_or(ModifyRefusal::DegenerateTarget(id))?;
    let superseded = superseded(sketch, id, &hit);
    let mut ops: Vec<SketchEditOp> = superseded
        .iter()
        .map(|&constraint| SketchEditOp::RemoveConstraint { constraint })
        .collect();
    ops.push(SketchEditOp::SetEntityPositions {
        positions: vec![(point, tip)],
    });
    let grown = Grown {
        id,
        role,
        endpoint: (point, CurvePosition::Arbitrary),
        tip,
        hit,
        superseded,
    };
    Ok(finish(grown, ops, Vec::new(), ids))
}

fn extend_arc(
    sketch: &Sketch,
    entity: &SketchEntity,
    (c, r, start, sweep): (Pt, f64, f64, f64),
    pick: Pt,
    ids: &mut dyn ModifyIds,
) -> Result<ExtendPlan, ModifyRefusal> {
    let id = entity.id();
    if r * sweep < MIN_PIECE_MM {
        return Err(ModifyRefusal::DegenerateTarget(id));
    }
    // At the centre the pick has no direction, so no end is nearer.
    if pick.dist(c) <= COINCIDENT_MM {
        return Err(ModifyRefusal::AmbiguousPick(id));
    }
    let tol = COINCIDENT_MM / r;
    let role = pick_end(arc_param(c, r, start, pick), sweep, true, tol, id)?;
    check_free(sketch, id, role, None)?;
    let end_angle = start + sweep;
    let ahead = |p: Pt| {
        let at = angle_of(c, p);
        if role == CurvePosition::End {
            (at - end_angle).rem_euclid(TAU)
        } else {
            (start - at).rem_euclid(TAU)
        }
    };
    let curve = Curve::Arc { c, r, start, sweep };
    let raw = crossings(sketch, id, &curve, false, &ahead);
    let end = on_circle(
        c,
        r,
        if role == CurvePosition::End {
            end_angle
        } else {
            start
        },
    );
    // Growing into the arc's own span would wrap it onto itself.
    let hit = nearest(sketch, raw, end, (tol, TAU - sweep - tol), id)?;
    let hit = not_same_circle(hit, id)?;
    if hit.param * r < MIN_PIECE_MM {
        return Err(ModifyRefusal::DegeneratePiece(id));
    }
    let (new_start, new_end) = if role == CurvePosition::End {
        (start, end_angle + hit.param)
    } else {
        (start - hit.param, end_angle)
    };
    let grown = with_angles(entity, normalize_angle(new_start), normalize_angle(new_end));
    let tip = hit.p.to_vec2().ok_or(ModifyRefusal::DegenerateTarget(id))?;
    let superseded = superseded(sketch, id, &hit);
    let (ops, kept) = replace_arc(sketch, grown, &superseded);
    let grown = Grown {
        id,
        role,
        endpoint: (id, role),
        tip,
        hit,
        superseded,
    };
    Ok(finish(grown, ops, kept, ids))
}

/// An arc target's `overlap` hit is the end point of an arc of its own
/// circle: overlapped, never met, so refused rather than welded.
fn not_same_circle(hit: Crossing, id: EntityId) -> Result<Crossing, ModifyRefusal> {
    if hit.overlap {
        return Err(ModifyRefusal::OverlapsBoundary {
            entity: id,
            boundary: hit.boundary,
        });
    }
    Ok(hit)
}

/// Replaces the arc under its own id (there is no in-place angle op): every
/// constraint naming it is removed and re-added unchanged — and so kept —
/// except the `superseded` ones, which stay removed.
fn replace_arc(
    sketch: &Sketch,
    grown: SketchEntity,
    superseded: &[ConstraintId],
) -> (Vec<SketchEditOp>, Vec<ConstraintId>) {
    let id = grown.id();
    let held: Vec<&Constraint> = sketch
        .constraints()
        .iter()
        .filter(|k| k.entities().contains(&id))
        .collect();
    let mut ops: Vec<SketchEditOp> = held
        .iter()
        .map(|k| SketchEditOp::RemoveConstraint { constraint: k.id() })
        .collect();
    ops.push(SketchEditOp::RemoveEntity { entity: id });
    ops.push(SketchEditOp::AddEntity { entity: grown });
    let readded: Vec<&Constraint> = held
        .into_iter()
        .filter(|k| !superseded.contains(&k.id()))
        .collect();
    ops.extend(readded.iter().map(|k| SketchEditOp::AddConstraint {
        constraint: (*k).clone(),
    }));
    (ops, readded.iter().map(|k| k.id()).collect())
}

/// `OnCurve`s on the target of the boundary end point the extend welds to,
/// or of any point a `Coincident` chain ties to it: once the grown end is
/// `Coincident` with that point, each restates the weld (the solver reports
/// the pair over-constrained). Matched however the point is named.
fn superseded(sketch: &Sketch, id: EntityId, hit: &Crossing) -> Vec<ConstraintId> {
    let Some(welded) = hit.at_endpoint else {
        return Vec::new();
    };
    let class = coincident_class(sketch, welded);
    sketch
        .constraints()
        .iter()
        .filter(|c| {
            matches!(c, Constraint::OnCurve { point, point_position, curve, .. }
                if *curve == id && class.contains(&point_key(sketch, *point, *point_position)))
        })
        .map(Constraint::id)
        .collect()
}

/// The end nearer `param` on `[0, len]`; a pick within `tol` of the middle is
/// ambiguous. On an arc (`wrap`) a pick past the sweep counts for the nearer
/// end around the circle, and one half-way around the gap is ambiguous.
fn pick_end(
    param: f64,
    len: f64,
    wrap: bool,
    tol: f64,
    id: EntityId,
) -> Result<CurvePosition, ModifyRefusal> {
    let param = if wrap && param > len {
        let (to_end, to_start) = (param - len, TAU - param);
        if (to_end - to_start).abs() <= 2.0 * tol {
            return Err(ModifyRefusal::AmbiguousPick(id));
        }
        if to_end < to_start {
            len
        } else {
            0.0
        }
    } else {
        param
    };
    let mid = 0.5 * len;
    if (param - mid).abs() <= tol {
        return Err(ModifyRefusal::AmbiguousPick(id));
    }
    Ok(if param < mid {
        CurvePosition::Start
    } else {
        CurvePosition::End
    })
}

/// Refuses an end that is a shared point, or that any constraint pins: the
/// end point / role itself, a line's `Equal` length or a `Midpoint` on it.
fn check_free(
    sketch: &Sketch,
    id: EntityId,
    role: CurvePosition,
    point: Option<EntityId>,
) -> Result<(), ModifyRefusal> {
    if let Some(p) = point {
        let shared = sketch
            .entities()
            .iter()
            .any(|e| e.id() != id && e.referenced_entities().contains(&p));
        if shared {
            return Err(ModifyRefusal::EndpointShared {
                entity: id,
                point: p,
            });
        }
        if sketch
            .get_entity(p)
            .is_some_and(SketchEntity::is_reference_locked)
        {
            return Err(ModifyRefusal::ReferenceLocked(p));
        }
    }
    let line = point.is_some();
    let held: Vec<ConstraintId> = sketch
        .constraints()
        .iter()
        .filter(|c| {
            let slots = refs(c);
            let at_end = slots
                .iter()
                .any(|(e, s)| Some(*e) == point || (*e == id && *s == Slot::Role(role)));
            let on_length = line
                && matches!(c, Constraint::Equal { .. } | Constraint::Midpoint { .. })
                && slots.iter().any(|(e, s)| *e == id && *s == Slot::Whole);
            at_end || on_length
        })
        .map(Constraint::id)
        .collect();
    if held.is_empty() {
        Ok(())
    } else {
        Err(ModifyRefusal::EndpointConstrained {
            entity: id,
            constraints: held,
        })
    }
}

/// The nearest crossing strictly ahead (`tol < param < limit`); refused when
/// none, when every crossing at that distance is a tangential touch, or when
/// `end` already lies on a same-carrier boundary that continues ahead.
fn nearest(
    sketch: &Sketch,
    raw: Vec<Crossing>,
    end: Pt,
    (tol, limit): (f64, f64),
    id: EntityId,
) -> Result<Crossing, ModifyRefusal> {
    let ahead: Vec<Crossing> = raw
        .into_iter()
        .filter(|c| c.param > tol && c.param < limit)
        .collect();
    for c in ahead.iter().filter(|c| c.overlap) {
        let covers_end = sketch
            .get_entity(c.boundary)
            .and_then(|e| curve_of(sketch, e))
            .is_some_and(|b| b.contains(end));
        if covers_end {
            return Err(ModifyRefusal::OverlapsBoundary {
                entity: id,
                boundary: c.boundary,
            });
        }
    }
    let min = ahead
        .iter()
        .map(|c| c.param)
        .min_by(f64::total_cmp)
        .ok_or(ModifyRefusal::NoBoundaryHit(id))?;
    let cluster: Vec<&Crossing> = ahead.iter().filter(|c| c.param - min <= tol).collect();
    if cluster.iter().all(|c| c.tangent) {
        return Err(ModifyRefusal::TangentHit(id));
    }
    // A tangent touch (e.g. an arc ending there) does not stop the extension,
    // so it must not outrank a transversal crossing as the weld target.
    Ok(representative(cluster.into_iter().filter(|c| !c.tangent)))
}

fn with_angles(entity: &SketchEntity, start_angle: f64, end_angle: f64) -> SketchEntity {
    let mut grown = entity.clone();
    if let SketchEntity::Arc {
        start_angle: s,
        end_angle: e,
        ..
    } = &mut grown
    {
        *s = start_angle;
        *e = end_angle;
    }
    grown
}

/// Where an extend landed.
struct Grown {
    id: EntityId,
    role: CurvePosition,
    /// The moved end as a constraint reference (point entity or arc role).
    endpoint: (EntityId, CurvePosition),
    tip: Vec2,
    hit: Crossing,
    /// `OnCurve`s the weld makes redundant; removed by `ops`, reported here.
    superseded: Vec<ConstraintId>,
}

/// Appends the weld holding the grown end on its boundary and the accounting.
fn finish(
    grown: Grown,
    mut ops: Vec<SketchEditOp>,
    kept_constraints: Vec<ConstraintId>,
    ids: &mut dyn ModifyIds,
) -> ExtendPlan {
    let weld = weld(ids.constraint_id(), grown.endpoint, &grown.hit);
    let added = AddedConstraint {
        id: weld.id(),
        reason: AddReason::ExtendWeld {
            boundary: grown.hit.boundary,
        },
    };
    ops.push(SketchEditOp::AddConstraint { constraint: weld });
    ExtendPlan {
        plan: ModifyPlan {
            ops,
            pieces: vec![PlannedPiece {
                id: grown.id,
                keeps_target_id: true,
                owns_start: true,
                owns_end: true,
            }],
            removed_entities: Vec::new(),
            added_points: Vec::new(),
            kept_constraints,
            added_constraints: vec![added],
            dropped_constraints: grown
                .superseded
                .iter()
                .map(|&id| DroppedConstraint {
                    id,
                    reason: DropReason::SupersededByExtendWeld,
                })
                .collect(),
        },
        end: grown.role,
        tip: grown.tip,
    }
}
