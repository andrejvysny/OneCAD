//! Line-chain continuation with a tangent arc. Tangents and anchors come from
//! the authoritative (solved) sketch, never from clicks.
use super::{
    arc_geometry::{self, AnalyticArc},
    inference::{persist_snap_to, AcceptedSnap, SnapSource},
    primitives::PrimitiveBatch,
    *,
};
use onecad_core::{
    edit::SketchEditOp,
    ids::{ConstraintId, EntityId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};

const NOT_CURVE: &str = "Tangent continuation needs a line or arc endpoint";

fn point_at(sketch: &Sketch, id: EntityId) -> Result<Vec2, String> {
    match sketch.get_entity(id) {
        Some(SketchEntity::Point { at, .. }) => Ok(*at),
        _ => Err("Chain point is missing".into()),
    }
}

fn unit(x: f64, y: f64) -> Result<Vec2, String> {
    let length = x.hypot(y);
    if !length.is_finite() || length < MIN_TANGENT {
        return Err("Chain tangent is degenerate".into());
    }
    Ok(Vec2::new_unchecked(x / length, y / length))
}

const MIN_TANGENT: f64 = 1e-12;

/// Direction leaving the chain anchor along the preceding curve.
pub(super) fn outgoing(sketch: &Sketch, curve: ChainCurve) -> Result<Vec2, String> {
    match (sketch.get_entity(curve.entity), curve.role) {
        (Some(SketchEntity::Line { start, end, .. }), CurvePosition::End) => {
            let (a, b) = (point_at(sketch, *start)?, point_at(sketch, *end)?);
            unit(b.x - a.x, b.y - a.y)
        }
        (Some(SketchEntity::Line { start, end, .. }), CurvePosition::Start) => {
            let (a, b) = (point_at(sketch, *start)?, point_at(sketch, *end)?);
            unit(a.x - b.x, a.y - b.y)
        }
        (Some(SketchEntity::Arc { end_angle, .. }), CurvePosition::End) => {
            unit(-end_angle.sin(), end_angle.cos())
        }
        (Some(SketchEntity::Arc { start_angle, .. }), CurvePosition::Start) => {
            unit(start_angle.sin(), -start_angle.cos())
        }
        _ => Err(NOT_CURVE.into()),
    }
}

/// Chain anchor: the last point entity, or the preceding arc's endpoint.
pub(super) fn anchor(
    sketch: &Sketch,
    last: Option<EntityId>,
    preceding: ChainCurve,
) -> Result<Vec2, String> {
    if let Some(id) = last {
        return point_at(sketch, id);
    }
    let Some(SketchEntity::Arc {
        center,
        radius,
        start_angle,
        end_angle,
        ..
    }) = sketch.get_entity(preceding.entity)
    else {
        return Err(NOT_CURVE.into());
    };
    let angle = match preceding.role {
        CurvePosition::Start => *start_angle,
        CurvePosition::End => *end_angle,
        _ => return Err(NOT_CURVE.into()),
    };
    let c = point_at(sketch, *center)?;
    Vec2::new(c.x + radius * angle.cos(), c.y + radius * angle.sin())
        .ok_or_else(|| "Chain anchor is not finite".to_owned())
}

fn candidate(sketch: &Sketch, draft: &SketchDraft, point: Vec2) -> Result<AnalyticArc, String> {
    let preceding = draft
        .preceding
        .ok_or("Tangent arc needs a preceding chain curve")?;
    let tangent = outgoing(sketch, preceding)?;
    arc_geometry::tangent_to(draft.anchor, tangent, point).map_err(|error| error.to_string())
}

pub(super) fn preview(sketch: &Sketch, draft: &SketchDraft, point: Vec2) -> Option<DraftPrimitive> {
    candidate(sketch, draft, point)
        .ok()
        .map(|arc| DraftPrimitive::Arc { arc })
}

pub(super) fn build(
    sketch: &Sketch,
    draft: &SketchDraft,
    point: Vec2,
    ids: &PrimitiveIds,
) -> Result<PrimitiveBatch, String> {
    let preceding = draft
        .preceding
        .ok_or("Tangent arc needs a preceding chain curve")?;
    if draft.first_id.is_none() && draft.first_target.is_none() {
        return Err("Tangent arc needs an existing chain".into());
    }
    let arc = candidate(sketch, draft, point)?;
    let mut ops = super::staged_creation::arc_ops(arc, ids)?;
    let (point1, point1_position) = match draft.anchor_id {
        Some(id) => (id, CurvePosition::Arbitrary),
        None => (preceding.entity, preceding.role),
    };
    let arc_id = ids.entities[1];
    for constraint in [
        Constraint::Coincident {
            id: ids.constraints[0],
            point1,
            point1_position,
            point2: arc_id,
            point2_position: arc.travel_start_role,
        },
        Constraint::TangentAtPoint {
            id: ids.constraints[1],
            entity1: preceding.entity,
            entity1_position: preceding.role,
            entity2: arc_id,
            entity2_position: arc.travel_start_role,
        },
    ] {
        ops.push(SketchEditOp::AddConstraint { constraint });
    }
    let segments = draft
        .segments
        .checked_add(1)
        .ok_or("Line chain is too long")?;
    let closing = draft.segments > 0
        && super::primitives::distance(draft.first, point) < super::primitives::MIN_SIZE;
    if closing {
        let target = chain_start(draft).ok_or("Chain start is missing")?;
        let SketchTarget::Point { entity, role } = target else {
            return Err(NOT_CURVE.into());
        };
        ops.push(SketchEditOp::AddConstraint {
            constraint: Constraint::Coincident {
                id: ids.constraints[2],
                point1: arc_id,
                point1_position: arc.travel_end_role,
                point2: entity,
                point2_position: role,
            },
        });
        return Ok((ops, AfterPrimitive::Reset));
    }
    let after = AfterPrimitive::ContinueLine {
        first: draft.first_id,
        first_target: draft.first_target,
        last: None,
        preceding: ChainCurve {
            entity: arc_id,
            role: arc.travel_end_role,
        },
        segments,
    };
    Ok((ops, after))
}

/// Maps a persistent final snap onto the new arc's travel-end role.
pub(super) fn persist_final(
    session: &SketchSession,
    snap: &AcceptedSnap,
    current: SnapSource,
    point: Vec2,
    ids: &PrimitiveIds,
    ops: &[SketchEditOp],
    next_id: &mut impl FnMut() -> ConstraintId,
) -> Result<Vec<Constraint>, String> {
    let draft = session.draft.as_ref().ok_or("No primitive anchor")?;
    let arc = candidate(&session.authoritative, draft, point)?;
    let additions: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            SketchEditOp::AddEntity { entity } => Some(entity.clone()),
            _ => None,
        })
        .collect();
    let target = SketchTarget::Point {
        entity: ids.entities[1],
        role: arc.travel_end_role,
    };
    persist_snap_to(
        &session.authoritative,
        snap,
        current,
        target,
        &additions,
        next_id,
    )
    .map_err(String::from)
}

impl SketchInteraction {
    pub(crate) fn set_tangent_arc(&mut self, key: SketchSessionKey, enabled: bool) {
        let Some(session) = self
            .session
            .as_mut()
            .filter(|session| session.key == key && session.phase == SketchPhase::Active)
        else {
            return;
        };
        if session.tool != SketchTool::Line {
            return;
        }
        let eligible = session.can_tangent_arc();
        if !eligible && enabled {
            self.refuse("Tangent arc needs a preceding chain curve");
            return;
        }
        let Some(draft) = session.draft.as_mut() else {
            return;
        };
        draft.tangent_arc = enabled;
        let cursor = draft.cursor;
        session.preview = if enabled {
            preview(&session.authoritative, draft, cursor)
        } else {
            Some(DraftPrimitive::Line {
                start: draft.anchor,
                end: cursor,
            })
        };
    }
}

pub(super) fn target_point(sketch: &Sketch, target: SketchTarget) -> Result<Vec2, String> {
    target
        .point_coordinates(sketch)
        .ok_or_else(|| "Chain endpoint is missing".into())
}

pub(super) fn chain_start(draft: &SketchDraft) -> Option<SketchTarget> {
    draft
        .first_id
        .map(|entity| SketchTarget::Point {
            entity,
            role: CurvePosition::Arbitrary,
        })
        .or(draft.first_target)
}

/// Only an accepted endpoint snap may seed a fresh chain; junctions never guess.
pub(super) fn seed(session: &mut SketchSession) {
    if session.tool != SketchTool::Line {
        return;
    }
    let Some(draft) = session.draft.as_mut() else {
        return;
    };
    if !matches!(
        draft.anchor_snap.map(|snap| snap.intent),
        Some(super::inference::SnapIntent::Coincident(_))
    ) {
        return;
    }
    let Some(curve) = unique_endpoint(&session.authoritative, draft.anchor) else {
        return;
    };
    let point_id = match session.authoritative.get_entity(curve.entity) {
        Some(SketchEntity::Line { start, end, .. }) => {
            Some(if curve.role == CurvePosition::Start {
                *start
            } else {
                *end
            })
        }
        _ => None,
    };
    let Some(point) = (SketchTarget::Point {
        entity: curve.entity,
        role: curve.role,
    })
    .point_coordinates(&session.authoritative) else {
        return;
    };
    draft.first = point;
    draft.anchor = point;
    draft.cursor = point;
    draft.preceding = Some(curve);
    draft.anchor_id = point_id;
    draft.first_id = point_id;
    draft.first_target = point_id.is_none().then_some(SketchTarget::Point {
        entity: curve.entity,
        role: curve.role,
    });
    draft.anchor_snap = None;
}

// The frozen endpoint tangent search uses a 1e-6 world-space tolerance floor.
fn unique_endpoint(sketch: &Sketch, point: Vec2) -> Option<ChainCurve> {
    let mut matching = sketch
        .entities()
        .iter()
        .filter(|entity| matches!(entity, SketchEntity::Line { .. } | SketchEntity::Arc { .. }))
        .flat_map(|entity| {
            [CurvePosition::Start, CurvePosition::End].map(|role| ChainCurve {
                entity: entity.id(),
                role,
            })
        })
        .filter(|curve| {
            SketchTarget::Point {
                entity: curve.entity,
                role: curve.role,
            }
            .point_coordinates(sketch)
            .is_some_and(|at| super::primitives::distance(at, point) <= 1e-6)
                && outgoing(sketch, *curve).is_ok()
        });
    let curve = matching.next()?;
    matching.next().is_none().then_some(curve)
}

impl SketchSession {
    pub fn can_tangent_arc(&self) -> bool {
        self.phase == SketchPhase::Active
            && self.tool == SketchTool::Line
            && self.draft.as_ref().is_some_and(|draft| {
                draft.preceding.is_some_and(|curve| {
                    outgoing(&self.authoritative, curve).is_ok()
                        && anchor(&self.authoritative, draft.anchor_id, curve).is_ok()
                })
            })
    }
}
