use super::*;

pub(super) fn resolve(
    sketch: &Sketch,
    target: SketchTarget,
    additions: &[SketchEntity],
    accepted: Vec2,
) -> Result<(EntityId, CurvePosition), &'static str> {
    let SketchTarget::Point { entity, role } = target else {
        return Err("Snap destination must be an explicit point role");
    };
    let point = coordinates(sketch, unique(additions, entity)?, role, additions)?;
    let scale = point
        .x
        .abs()
        .max(point.y.abs())
        .max(accepted.x.abs())
        .max(accepted.y.abs())
        .max(1.0);
    if !point.is_finite()
        || !accepted.is_finite()
        || (point.x - accepted.x).hypot(point.y - accepted.y) > 16.0 * f64::EPSILON * scale
    {
        return Err("Snap destination differs from the accepted winning point");
    }
    Ok((entity, role))
}

fn coordinates(
    sketch: &Sketch,
    owner: &SketchEntity,
    role: CurvePosition,
    additions: &[SketchEntity],
) -> Result<Vec2, &'static str> {
    match (owner, role) {
        (SketchEntity::Point { at, .. }, CurvePosition::Arbitrary) => Ok(*at),
        (
            SketchEntity::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ..
            },
            CurvePosition::Start | CurvePosition::End,
        ) if radius.is_finite() && *radius > 0.0 => {
            let at = center_point(sketch, additions, *center)?;
            let angle = if role == CurvePosition::Start {
                start_angle
            } else {
                end_angle
            };
            Vec2::new(at.x + radius * angle.cos(), at.y + radius * angle.sin())
                .ok_or("Snap arc destination is not finite")
        }
        _ => Err("Snap destination has an unsupported point role"),
    }
}

fn center_point(
    sketch: &Sketch,
    additions: &[SketchEntity],
    id: EntityId,
) -> Result<Vec2, &'static str> {
    let center = if additions.iter().any(|entity| entity.id() == id) {
        Some(unique(additions, id)?)
    } else {
        sketch.get_entity(id)
    };
    match center {
        Some(SketchEntity::Point { at, .. }) => Ok(*at),
        _ => Err("Snap arc destination requires its actual center point"),
    }
}

fn unique(entities: &[SketchEntity], id: EntityId) -> Result<&SketchEntity, &'static str> {
    let mut matching = entities.iter().filter(|entity| entity.id() == id);
    let entity = matching
        .next()
        .ok_or("Snap destination is not in the primitive batch")?;
    if matching.next().is_some() {
        return Err("Snap destination identity is ambiguous");
    }
    Ok(entity)
}
