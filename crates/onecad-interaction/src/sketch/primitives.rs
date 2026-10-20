use super::*;
use onecad_core::{
    edit::SketchEditOp,
    ids::EntityId,
    math::Vec2,
    sketch::{Constraint, CurvePosition, SketchEntity},
};
use std::collections::BTreeSet;

pub(crate) const MIN_SIZE: f64 = 1e-9;
pub(crate) type PrimitiveBatch = (Vec<SketchEditOp>, AfterPrimitive);

pub(crate) fn build_primitive(
    session: &SketchSession,
    point: Vec2,
    ids: &PrimitiveIds,
) -> Result<PrimitiveBatch, String> {
    validate_ids(session, ids)?;
    if session.tool == SketchTool::Point {
        let entity = SketchEntity::point(ids.entities[0], point, session.construction, false);
        if !point.is_finite() {
            return Err("Point coordinates must be finite".into());
        }
        return Ok((vec![add_entity(entity)], AfterPrimitive::Reset));
    }
    let draft = session.draft.as_ref().ok_or("No primitive anchor")?;
    let (mut ops, after) = match session.tool {
        SketchTool::Line => line(session, draft, point, ids),
        SketchTool::Rectangle => rectangle(draft.anchor, point, ids),
        SketchTool::CenterRectangle => super::center_rectangle::build(draft.anchor, point, ids),
        SketchTool::Circle => circle(draft.anchor, point, ids),
        SketchTool::Arc | SketchTool::ThreePointArc | SketchTool::Slot => {
            super::staged_creation::build(session.tool, draft, point, ids)
        }
        SketchTool::Select | SketchTool::Point => Err("Select does not author geometry".into()),
    }?;
    if session.construction {
        for op in &mut ops {
            if let SketchEditOp::AddEntity { entity } = op {
                match entity {
                    SketchEntity::Point { construction, .. }
                    | SketchEntity::Line { construction, .. }
                    | SketchEntity::Circle { construction, .. }
                    | SketchEntity::Arc { construction, .. }
                    | SketchEntity::Ellipse { construction, .. } => *construction = true,
                }
            }
        }
    }
    Ok((ops, after))
}

pub(super) fn validate_ids(session: &SketchSession, ids: &PrimitiveIds) -> Result<(), String> {
    let entities: BTreeSet<_> = ids.entities.iter().copied().collect();
    let constraints: BTreeSet<_> = ids.constraints.iter().copied().collect();
    if entities.len() != ids.entities.len()
        || constraints.len() != ids.constraints.len()
        || entities
            .iter()
            .any(|id| session.authoritative.contains_entity(*id))
        || constraints
            .iter()
            .any(|id| session.authoritative.contains_constraint(*id))
    {
        return Err("Primitive ids must be fresh and unique".into());
    }
    Ok(())
}

fn line(
    session: &SketchSession,
    draft: &SketchDraft,
    point: Vec2,
    ids: &PrimitiveIds,
) -> Result<PrimitiveBatch, String> {
    if let Some(preceding) = draft.preceding {
        super::tangent_continuation::outgoing(&session.authoritative, preceding)?;
        super::tangent_continuation::anchor(&session.authoritative, draft.anchor_id, preceding)?;
    }
    if draft.tangent_arc {
        return super::tangent_continuation::build(&session.authoritative, draft, point, ids);
    }
    let closing = draft.segments > 0 && distance(draft.first, point) < MIN_SIZE;
    let endpoint = if closing { draft.first } else { point };
    if !distance(draft.anchor, endpoint).is_finite() || distance(draft.anchor, endpoint) < MIN_SIZE
    {
        return Err("Degenerate line".into());
    }
    let first = draft
        .first_id
        .or_else(|| draft.first_target.is_none().then_some(ids.entities[0]));
    let start = draft.anchor_id.unwrap_or(ids.entities[0]);
    let end = if closing {
        first.unwrap_or(ids.entities[1])
    } else {
        ids.entities[1]
    };
    let mut ops = Vec::with_capacity(4);
    if draft.anchor_id.is_none() {
        ops.push(add_point(start, draft.anchor));
    }
    if !closing || first.is_none() {
        ops.push(add_point(end, endpoint));
    }
    if closing && first.is_none() {
        let Some(SketchTarget::Point { entity, role }) = draft.first_target else {
            return Err("Chain start is missing".into());
        };
        ops.push(SketchEditOp::AddConstraint {
            constraint: Constraint::Coincident {
                id: ids.constraints[2],
                point1: end,
                point1_position: CurvePosition::Arbitrary,
                point2: entity,
                point2_position: role,
            },
        });
    }
    ops.push(add_entity(SketchEntity::line(
        ids.entities[2],
        start,
        end,
        false,
    )));
    push_arc_weld(&mut ops, draft, start, ids);
    let after = if closing {
        AfterPrimitive::Reset
    } else {
        AfterPrimitive::ContinueLine {
            first,
            first_target: draft.first_target,
            last: Some(end),
            preceding: ChainCurve {
                entity: ids.entities[2],
                role: CurvePosition::End,
            },
            segments: draft
                .segments
                .checked_add(1)
                .ok_or("Line chain is too long")?,
        }
    };
    Ok((ops, after))
}

fn push_arc_weld(
    ops: &mut Vec<SketchEditOp>,
    draft: &SketchDraft,
    start: EntityId,
    ids: &PrimitiveIds,
) {
    if let (None, Some(preceding)) = (draft.anchor_id, draft.preceding) {
        ops.push(SketchEditOp::AddConstraint {
            constraint: Constraint::Coincident {
                id: ids.constraints[0],
                point1: start,
                point1_position: CurvePosition::Arbitrary,
                point2: preceding.entity,
                point2_position: preceding.role,
            },
        });
    }
}

pub(super) fn rectangle(
    first: Vec2,
    opposite: Vec2,
    ids: &PrimitiveIds,
) -> Result<PrimitiveBatch, String> {
    if !(first.x - opposite.x).is_finite()
        || !(first.y - opposite.y).is_finite()
        || (first.x - opposite.x).abs() < MIN_SIZE
        || (first.y - opposite.y).abs() < MIN_SIZE
    {
        return Err("Degenerate rectangle".into());
    }
    let points = [
        first,
        Vec2::new_unchecked(opposite.x, first.y),
        opposite,
        Vec2::new_unchecked(first.x, opposite.y),
    ];
    let mut ops: Vec<_> = points
        .into_iter()
        .enumerate()
        .map(|(i, p)| add_point(ids.entities[i], p))
        .collect();
    for i in 0..4 {
        let line = ids.entities[i + 4];
        ops.push(add_entity(SketchEntity::line(
            line,
            ids.entities[i],
            ids.entities[(i + 1) % 4],
            false,
        )));
        let constraint = if i % 2 == 0 {
            Constraint::Horizontal {
                id: ids.constraints[i],
                line,
            }
        } else {
            Constraint::Vertical {
                id: ids.constraints[i],
                line,
            }
        };
        ops.push(SketchEditOp::AddConstraint { constraint });
    }
    Ok((ops, AfterPrimitive::Reset))
}

fn circle(center: Vec2, edge: Vec2, ids: &PrimitiveIds) -> Result<PrimitiveBatch, String> {
    let radius = distance(center, edge);
    if !radius.is_finite() || radius < MIN_SIZE {
        return Err("Degenerate circle".into());
    }
    let circle = SketchEntity::circle(ids.entities[1], ids.entities[0], radius, false)
        .ok_or("Invalid circle")?;
    Ok((
        vec![add_point(ids.entities[0], center), add_entity(circle)],
        AfterPrimitive::Reset,
    ))
}

fn add_point(id: EntityId, point: Vec2) -> SketchEditOp {
    add_entity(SketchEntity::point(id, point, false, false))
}
fn add_entity(entity: SketchEntity) -> SketchEditOp {
    SketchEditOp::AddEntity { entity }
}
pub(crate) fn distance(a: Vec2, b: Vec2) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}
