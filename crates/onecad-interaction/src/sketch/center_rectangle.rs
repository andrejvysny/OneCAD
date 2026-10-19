//! A shared construction diagonal keeps the authored center linked to the rectangle.
use super::{primitives::PrimitiveBatch, *};
use onecad_core::{
    edit::SketchEditOp,
    math::Vec2,
    sketch::{Constraint, CurvePosition, SketchEntity},
};

pub(super) fn opposite(center: Vec2, corner: Vec2) -> Result<Vec2, String> {
    let dx = corner.x - center.x;
    let dy = corner.y - center.y;
    if !dx.is_finite()
        || !dy.is_finite()
        || dx.abs() < super::primitives::MIN_SIZE
        || dy.abs() < super::primitives::MIN_SIZE
    {
        return Err("Degenerate center rectangle".into());
    }
    Vec2::new(center.x - dx, center.y - dy)
        .ok_or_else(|| "Center rectangle coordinates are not finite".into())
}

pub(super) fn build(
    center: Vec2,
    corner: Vec2,
    ids: &PrimitiveIds,
) -> Result<PrimitiveBatch, String> {
    let first = opposite(center, corner)?;
    let (mut ops, after) = super::primitives::rectangle(first, corner, ids)?;
    ops.push(SketchEditOp::AddEntity {
        entity: SketchEntity::point(ids.entities[8], center, true, false),
    });
    ops.push(SketchEditOp::AddEntity {
        entity: SketchEntity::line(ids.entities[9], ids.entities[0], ids.entities[2], true),
    });
    ops.push(SketchEditOp::AddConstraint {
        constraint: Constraint::Midpoint {
            id: ids.constraints[10],
            point: ids.entities[8],
            point_position: CurvePosition::Arbitrary,
            line: ids.entities[9],
        },
    });
    Ok((ops, after))
}
