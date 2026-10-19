use super::*;
use onecad_core::sketch::{Constraint, CurvePosition};
use std::collections::BTreeMap;

// Approved internal arc coupling budget: sketch-solver-truth.md, Degeneracy.
const ARC_COUPLING_TOLERANCE_MM: f64 = 1e-4;

pub(super) enum Handle {
    Point(EntityId),
    ArcEndpoint(EntityId, bool),
}

pub(super) fn resolve(sketch: &Sketch, key: &str) -> Result<Handle, String> {
    if let Ok(id) = EntityId::from_str(key) {
        return match sketch.get_entity(id) {
            Some(SketchEntity::Point { .. }) => Ok(Handle::Point(id)),
            _ => Err(format!("SketchUpsert: {key} is not a sketch point")),
        };
    }
    let (owner, role) = key
        .rsplit_once('.')
        .ok_or_else(|| format!("SketchUpsert: invalid point handle {key}"))?;
    let id = entity(owner)?;
    match (sketch.get_entity(id), role) {
        (
            Some(
                SketchEntity::Circle { center, .. }
                | SketchEntity::Arc { center, .. }
                | SketchEntity::Ellipse { center, .. },
            ),
            "center",
        ) => Ok(Handle::Point(*center)),
        (Some(SketchEntity::Arc { .. }), "start") => Ok(Handle::ArcEndpoint(id, true)),
        (Some(SketchEntity::Arc { .. }), "end") => Ok(Handle::ArcEndpoint(id, false)),
        _ => Err(format!("SketchUpsert: incompatible point handle {key}")),
    }
}

pub(super) fn insert_point(
    points: &mut BTreeMap<EntityId, Vec2>,
    id: EntityId,
    at: Vec2,
) -> Result<(), String> {
    if points.insert(id, at).is_some_and(|old| old != at) {
        return Err(format!("SketchUpsert: conflicting aliases for point {id}"));
    }
    Ok(())
}

pub(super) fn endpoint(sketch: &Sketch, id: EntityId, start: bool) -> Result<Vec2, String> {
    let Some(SketchEntity::Arc {
        center,
        radius,
        start_angle,
        end_angle,
        ..
    }) = sketch.get_entity(id)
    else {
        return Err(format!("SketchUpsert: {id} is not an arc"));
    };
    let center = point(sketch, *center)?;
    let angle = if start { *start_angle } else { *end_angle };
    Vec2::new(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    )
    .ok_or_else(|| format!("SketchUpsert: non-finite arc endpoint for {id}"))
}

pub(super) fn validate_endpoint(
    sketch: &Sketch,
    id: EntityId,
    start: bool,
    at: Vec2,
) -> Result<(), String> {
    let expected = endpoint(sketch, id, start)?;
    if (expected.x - at.x).hypot(expected.y - at.y) > ARC_COUPLING_TOLERANCE_MM {
        return Err(format!("SketchUpsert: inconsistent arc endpoint for {id}"));
    }
    Ok(())
}

fn point(sketch: &Sketch, id: EntityId) -> Result<Vec2, String> {
    match sketch.get_entity(id) {
        Some(SketchEntity::Point { at, .. }) => Ok(*at),
        _ => Err(format!("SketchUpsert: {id} is not a sketch point")),
    }
}

fn fixed_position(sketch: &Sketch, id: EntityId, role: CurvePosition) -> Result<Vec2, String> {
    match sketch.get_entity(id) {
        Some(SketchEntity::Point { .. }) => point(sketch, id),
        Some(
            SketchEntity::Circle { center, .. }
            | SketchEntity::Arc { center, .. }
            | SketchEntity::Ellipse { center, .. },
        ) if role == CurvePosition::Center => point(sketch, *center),
        Some(SketchEntity::Arc { .. }) if role == CurvePosition::Start => {
            endpoint(sketch, id, true)
        }
        Some(SketchEntity::Arc { .. }) if role == CurvePosition::End => endpoint(sketch, id, false),
        Some(SketchEntity::Line { start, .. }) if role == CurvePosition::Start => {
            point(sketch, *start)
        }
        Some(SketchEntity::Line { end, .. }) if role == CurvePosition::End => point(sketch, *end),
        _ => Err(format!("SketchUpsert: unreadable Fixed operand {id}")),
    }
}

pub(super) fn validate_fixed(before: &Sketch, after: &Sketch) -> Result<(), String> {
    for constraint in before.constraints() {
        if let Constraint::Fixed {
            point,
            point_position,
            ..
        } = constraint
        {
            if fixed_position(before, *point, *point_position)?
                != fixed_position(after, *point, *point_position)?
            {
                return Err(format!("SketchUpsert: Fixed operand {point} moved"));
            }
        }
    }
    Ok(())
}
