//! Extend: grow a line/arc end along its continuation to the nearest boundary.

use std::f64::consts::TAU;

use super::common::{crossings, modifiable, representative, weld, Crossing};
use super::geom::{angle_of, arc_param, curve_of, line_param, normalize_angle, Curve, Pt};
use super::slots::{refs, Slot};
use super::{
    AddReason, AddedConstraint, ExtendPlan, ExtendRequest, ModifyIds, ModifyPlan, ModifyRefusal,
    PlannedPiece, COINCIDENT_MM, MIN_PIECE_MM,
};
use crate::edit::SketchEditOp;
use crate::ids::{ConstraintId, EntityId};
use crate::math::Vec2;
use crate::sketch::{Constraint, CurvePosition, Sketch, SketchEntity};

/// Plans extending the end of `request.entity` nearer `request.pick`.
///
/// # Errors
/// A [`ModifyRefusal`] when the target is missing, a point, locked, a circle or
/// ellipse, degenerate, picked exactly mid-way, its end is shared or held by a
/// constraint, or its continuation meets no boundary / only a tangent one / one
/// closer than [`MIN_PIECE_MM`]. A refusal draws no id from `ids`.
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
    let hit = nearest(raw, tol, f64::INFINITY, id)?;
    if hit.param * len < MIN_PIECE_MM {
        return Err(ModifyRefusal::DegeneratePiece(id));
    }
    let tip = hit.p.to_vec2().ok_or(ModifyRefusal::DegenerateTarget(id))?;
    let ops = vec![SketchEditOp::SetEntityPositions {
        positions: vec![(point, tip)],
    }];
    let grown = Grown {
        id,
        role,
        endpoint: (point, CurvePosition::Arbitrary),
        tip,
        hit,
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
    // Growing into the arc's own span would wrap it onto itself.
    let hit = nearest(raw, tol, TAU - sweep - tol, id)?;
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
    let (ops, kept) = replace_arc(sketch, grown);
    let grown = Grown {
        id,
        role,
        endpoint: (id, role),
        tip,
        hit,
    };
    Ok(finish(grown, ops, kept, ids))
}

/// Replaces the arc under its own id (there is no in-place angle op): every
/// constraint naming it is removed and re-added unchanged, so all are kept.
fn replace_arc(sketch: &Sketch, grown: SketchEntity) -> (Vec<SketchEditOp>, Vec<ConstraintId>) {
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
    ops.extend(held.iter().map(|k| SketchEditOp::AddConstraint {
        constraint: (*k).clone(),
    }));
    (ops, held.iter().map(|k| k.id()).collect())
}

/// The end nearer `param` on `[0, len]`; a pick within `tol` of the middle is
/// ambiguous. On an arc (`wrap`) a pick past the sweep counts for the nearer
/// end around the circle.
fn pick_end(
    param: f64,
    len: f64,
    wrap: bool,
    tol: f64,
    id: EntityId,
) -> Result<CurvePosition, ModifyRefusal> {
    let param = match wrap && param > len {
        true if param - len < TAU - param => len,
        true => 0.0,
        false => param,
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
/// none, or when every crossing at that distance is a tangential touch.
fn nearest(
    raw: Vec<Crossing>,
    tol: f64,
    limit: f64,
    id: EntityId,
) -> Result<Crossing, ModifyRefusal> {
    let ahead: Vec<Crossing> = raw
        .into_iter()
        .filter(|c| c.param > tol && c.param < limit)
        .collect();
    let min = ahead
        .iter()
        .map(|c| c.param)
        .min_by(f64::total_cmp)
        .ok_or(ModifyRefusal::NoBoundaryHit(id))?;
    let hit = representative(ahead.iter().filter(|c| c.param - min <= tol));
    if hit.tangent {
        return Err(ModifyRefusal::TangentHit(id));
    }
    Ok(hit)
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
            dropped_constraints: Vec::new(),
        },
        end: grown.role,
        tip: grown.tip,
    }
}
