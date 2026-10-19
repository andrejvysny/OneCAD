use super::{
    arc_geometry::{self, AnalyticArc},
    primitives::{PrimitiveBatch, MIN_SIZE},
    *,
};
use onecad_core::{
    edit::SketchEditOp,
    math::Vec2,
    sketch::{Constraint, CurvePosition, SketchEntity},
};

pub(super) struct SlotGeometry {
    pub points: [Vec2; 4],
    pub caps: [AnalyticArc; 2],
    pub radius: f64,
    pub side: usize,
    pub along: f64,
    pub length: f64,
}

pub(super) fn axes(
    first: Vec2,
    second: Vec2,
    cursor: Vec2,
) -> Result<(Vec2, Vec2, f64, f64, f64), String> {
    if !first.is_finite() || !second.is_finite() || !cursor.is_finite() {
        return Err("Slot anchors must be finite".into());
    }
    let (dx, dy) = (second.x - first.x, second.y - first.y);
    let length = dx.hypot(dy);
    if !length.is_finite() || length < MIN_SIZE {
        return Err("Slot centerline must be nonzero and finite".into());
    }
    let tangent = Vec2::new_unchecked(dx / length, dy / length);
    let normal = Vec2::new_unchecked(-tangent.y, tangent.x);
    let (x, y) = (cursor.x - first.x, cursor.y - first.y);
    let along = x * tangent.x + y * tangent.y;
    let signed_width = x * normal.x + y * normal.y;
    if !along.is_finite() || !signed_width.is_finite() {
        return Err("Slot coordinates exceed finite precision".into());
    }
    Ok((tangent, normal, length, along, signed_width))
}

pub(super) fn geometry(first: Vec2, second: Vec2, cursor: Vec2) -> Result<SlotGeometry, String> {
    let (_, normal, length, along, signed_width) = axes(first, second, cursor)?;
    let radius = signed_width.abs();
    if radius < MIN_SIZE {
        return Err("Slot half-width must be at least 1e-9".into());
    }
    let offset = |at: Vec2, sign: f64| {
        Vec2::new(
            at.x + normal.x * radius * sign,
            at.y + normal.y * radius * sign,
        )
        .ok_or_else(|| "Slot coordinates exceed finite precision".to_owned())
    };
    let points = [
        offset(first, 1.0)?,
        offset(second, 1.0)?,
        offset(first, -1.0)?,
        offset(second, -1.0)?,
    ];
    let right = arc_geometry::center_start_end(second, points[3], points[1])
        .map_err(|error| error.to_string())?;
    let left = arc_geometry::center_start_end(first, points[0], points[2])
        .map_err(|error| error.to_string())?;
    Ok(SlotGeometry {
        points,
        caps: [right, left],
        radius,
        side: usize::from(signed_width < 0.0),
        along,
        length,
    })
}

pub(super) fn build(
    first: Vec2,
    second: Vec2,
    cursor: Vec2,
    ids: &PrimitiveIds,
) -> Result<PrimitiveBatch, String> {
    let slot = geometry(first, second, cursor)?;
    let mut ops = Vec::with_capacity(19);
    for (index, at) in slot.points.into_iter().chain([first, second]).enumerate() {
        ops.push(SketchEditOp::AddEntity {
            entity: SketchEntity::point(ids.entities[index], at, false, false),
        });
    }
    for (index, a, b) in [(6, 0, 1), (7, 2, 3)] {
        ops.push(SketchEditOp::AddEntity {
            entity: SketchEntity::line(
                ids.entities[index],
                ids.entities[a],
                ids.entities[b],
                false,
            ),
        });
    }
    for (index, center, arc) in [(8, 5, slot.caps[0]), (9, 4, slot.caps[1])] {
        let entity = SketchEntity::arc(
            ids.entities[index],
            ids.entities[center],
            arc.radius,
            arc.start_angle,
            arc.end_angle,
            false,
        )
        .ok_or("Slot cap must be finite")?;
        ops.push(SketchEditOp::AddEntity { entity });
    }
    ops.extend(
        intrinsic(ids)
            .into_iter()
            .map(|constraint| SketchEditOp::AddConstraint { constraint }),
    );
    Ok((ops, AfterPrimitive::Reset))
}

fn intrinsic(ids: &PrimitiveIds) -> Vec<Constraint> {
    use CurvePosition::{Arbitrary, End, Start};
    let corners = [
        (1, 6, End, 8, End),
        (3, 7, End, 8, Start),
        (0, 6, Start, 9, Start),
        (2, 7, Start, 9, End),
    ];
    let mut constraints = Vec::with_capacity(9);
    for (index, (point, line, line_role, cap, cap_role)) in corners.into_iter().enumerate() {
        constraints.push(Constraint::Coincident {
            id: ids.constraints[index],
            point1: ids.entities[point],
            point1_position: Arbitrary,
            point2: ids.entities[cap],
            point2_position: cap_role,
        });
        constraints.push(Constraint::TangentAtPoint {
            id: ids.constraints[index + 5],
            entity1: ids.entities[line],
            entity1_position: line_role,
            entity2: ids.entities[cap],
            entity2_position: cap_role,
        });
    }
    constraints.push(Constraint::Equal {
        id: ids.constraints[4],
        entity1: ids.entities[8],
        entity2: ids.entities[9],
    });
    constraints
}
