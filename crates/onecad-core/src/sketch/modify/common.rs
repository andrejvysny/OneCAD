//! Pieces shared by Trim and Extend: target validation, boundary crossings and
//! the weld that holds a new endpoint on its boundary.

use super::geom::{curve_of, point_key, Curve, Pt};
use super::intersect::carrier_hits;
use super::ModifyRefusal;
use crate::ids::{ConstraintId, EntityId};
use crate::sketch::{Constraint, CurvePosition, Sketch, SketchEntity};

/// The entity to modify, refused when absent, a point or reference-locked.
pub(super) fn modifiable(sketch: &Sketch, id: EntityId) -> Result<&SketchEntity, ModifyRefusal> {
    let entity = sketch
        .get_entity(id)
        .ok_or(ModifyRefusal::UnknownEntity(id))?;
    if matches!(entity, SketchEntity::Point { .. }) {
        return Err(ModifyRefusal::NotACurve(id));
    }
    if entity.is_reference_locked() {
        return Err(ModifyRefusal::ReferenceLocked(id));
    }
    Ok(entity)
}

/// Where the target's carrier meets one boundary curve.
#[derive(Debug, Clone, Copy)]
pub(super) struct Crossing {
    /// Parameter on the target (the caller's param space).
    pub param: f64,
    /// Plane position.
    pub p: Pt,
    /// The boundary curve.
    pub boundary: EntityId,
    /// The boundary's own endpoint at `p`, when there is one to weld to.
    pub at_endpoint: Option<(EntityId, CurvePosition)>,
    /// The carriers only touch here.
    pub tangent: bool,
    /// An end point of a boundary lying on the target's own carrier.
    pub overlap: bool,
}

/// Every crossing of `target`'s carrier with another curve's DRAWN extent, in
/// sketch order. With `within_target`, hits off the target's own extent are
/// discarded too (trim); without it the whole carrier counts (extend).
pub(super) fn crossings(
    sketch: &Sketch,
    target_id: EntityId,
    target: &Curve,
    within_target: bool,
    param: &dyn Fn(Pt) -> f64,
) -> Vec<Crossing> {
    let mut out = Vec::new();
    for entity in sketch.entities() {
        if entity.id() == target_id {
            continue;
        }
        let Some(boundary) = curve_of(sketch, entity) else {
            continue;
        };
        for hit in carrier_hits(target, &boundary) {
            if !hit.p.is_finite() || !boundary.contains(hit.p) {
                continue;
            }
            if within_target && !target.contains(hit.p) {
                continue;
            }
            out.push(Crossing {
                param: param(hit.p),
                p: hit.p,
                boundary: entity.id(),
                at_endpoint: boundary.endpoint_ref(entity, hit.p),
                tangent: hit.tangent,
                overlap: hit.overlap,
            });
        }
    }
    out
}

/// Of crossings within `tol` of each other, the one a weld should use: a
/// boundary endpoint first (a `Coincident` is exact), then a transversal
/// crossing, then a same-carrier end point (so a refusal of an arc overlap is
/// never masked by another curve meeting there), then iteration order.
/// Extend drops tangent crossings from the cluster before ranking, so a
/// tangent arc end never outranks a transversal stop there.
pub(super) fn representative<'a>(cluster: impl Iterator<Item = &'a Crossing>) -> Crossing {
    let rank = |c: &Crossing| (c.at_endpoint.is_some(), !c.tangent, c.overlap);
    let mut best: Option<&Crossing> = None;
    for c in cluster {
        let better = match best {
            None => true,
            Some(b) => rank(c) > rank(b),
        };
        if better {
            best = Some(c);
        }
    }
    *best.expect("clusters are never empty")
}

/// The constraint holding endpoint `(entity, role)` on the crossing's
/// boundary: `Coincident` with the boundary's own endpoint there, otherwise
/// `OnCurve` anywhere on the boundary.
pub(super) fn weld(
    id: ConstraintId,
    endpoint: (EntityId, CurvePosition),
    crossing: &Crossing,
) -> Constraint {
    match crossing.at_endpoint {
        Some((point2, point2_position)) => Constraint::Coincident {
            id,
            point1: endpoint.0,
            point2,
            point1_position: endpoint.1,
            point2_position,
        },
        None => Constraint::OnCurve {
            id,
            point: endpoint.0,
            point_position: endpoint.1,
            curve: crossing.boundary,
            position: CurvePosition::Arbitrary,
        },
    }
}

/// Every point reference a chain of `Coincident`s ties to `seed`, `seed`
/// included, all canonical ([`point_key`]). A weld at any member welds them
/// all, so an `OnCurve` naming any member restates it.
pub(super) fn coincident_class(
    sketch: &Sketch,
    (entity, role): (EntityId, CurvePosition),
) -> Vec<(EntityId, CurvePosition)> {
    let links: Vec<_> = sketch
        .constraints()
        .iter()
        .filter_map(|c| match *c {
            Constraint::Coincident {
                point1,
                point2,
                point1_position,
                point2_position,
                ..
            } => Some((
                point_key(sketch, point1, point1_position),
                point_key(sketch, point2, point2_position),
            )),
            _ => None,
        })
        .collect();
    let mut class = vec![point_key(sketch, entity, role)];
    let mut next = 0;
    while let Some(&at) = class.get(next) {
        for &(a, b) in &links {
            let other = if a == at {
                b
            } else if b == at {
                a
            } else {
                continue;
            };
            if !class.contains(&other) {
                class.push(other);
            }
        }
        next += 1;
    }
    class
}

/// True when no entity other than `owner` is built from `point`, so removing
/// `owner` leaves it stray. Locked points are never reported orphaned.
pub(super) fn orphaned(sketch: &Sketch, point: EntityId, owner: EntityId) -> bool {
    let locked = sketch
        .get_entity(point)
        .is_some_and(SketchEntity::is_reference_locked);
    !locked
        && !sketch
            .entities()
            .iter()
            .any(|e| e.id() != owner && e.referenced_entities().contains(&point))
}
