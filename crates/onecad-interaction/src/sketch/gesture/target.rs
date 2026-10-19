use super::*;

pub fn classify_candidates(
    sketch: &Sketch,
    candidates: &[SketchTarget],
    grab: Vec2,
) -> Result<GestureIntent, String> {
    match candidates {
        [target] => classify_target(sketch, *target, grab),
        [] => Err("Gesture requires one explicit target".into()),
        _ => Err("Gesture target is ambiguous; choose one point role or whole entity".into()),
    }
}

pub fn classify_target(
    sketch: &Sketch,
    target: SketchTarget,
    grab: Vec2,
) -> Result<GestureIntent, String> {
    if !grab.is_finite() || !target.is_valid(sketch) {
        return Err("Gesture target or initial grab is invalid".into());
    }
    let entity = sketch
        .get_entity(target.owner_id())
        .ok_or("Gesture entity is unavailable")?;
    if entity.is_reference_locked() {
        return Err("Locked projected geometry cannot be dragged".into());
    }
    let (kind, driven, role, drag_point) = resolve(entity, target)?;
    let locked = match kind {
        GestureKind::Point => locked_point(sketch, drag_point),
        GestureKind::EntityBody => defining_points(entity)
            .into_iter()
            .flatten()
            .any(|id| locked_point(sketch, id)),
        GestureKind::Radius | GestureKind::ArcEnd => false,
    };
    if locked {
        return Err("Locked projected point cannot be dragged".into());
    }
    Ok(GestureIntent {
        target,
        kind,
        entity: driven,
        role,
        drag_point,
        grab,
    })
}

fn resolve(
    entity: &SketchEntity,
    target: SketchTarget,
) -> Result<(GestureKind, EntityId, Option<CurvePosition>, EntityId), String> {
    use CurvePosition::{Arbitrary, Center, End, Start};
    use GestureKind::{ArcEnd, EntityBody, Point, Radius};
    let owner = entity.id();
    let role = match target {
        SketchTarget::Entity(_) => None,
        SketchTarget::Point { role, .. } => Some(role),
    };
    let resolved = match (entity, role) {
        (SketchEntity::Point { .. }, Some(Arbitrary)) => (Point, owner, None, owner),
        (SketchEntity::Line { start, .. }, Some(Start)) => (Point, *start, None, *start),
        (SketchEntity::Line { end, .. }, Some(End)) => (Point, *end, None, *end),
        (
            SketchEntity::Circle { center, .. } | SketchEntity::Ellipse { center, .. },
            Some(Center),
        ) => (Point, *center, None, *center),
        (SketchEntity::Arc { center, .. }, Some(Center)) => {
            (EntityBody, owner, Some(Center), *center)
        }
        (SketchEntity::Arc { center, .. }, Some(role @ (Start | End))) => {
            (ArcEnd, owner, Some(role), *center)
        }
        (SketchEntity::Line { start, .. }, None) => (EntityBody, owner, None, *start),
        (SketchEntity::Circle { center, .. } | SketchEntity::Arc { center, .. }, None) => {
            (Radius, owner, None, *center)
        }
        _ => return Err("This entity or point role does not support a sketch gesture".into()),
    };
    Ok(resolved)
}

pub(super) fn defining_points(entity: &SketchEntity) -> [Option<EntityId>; 2] {
    match entity {
        SketchEntity::Point { id, .. } => [Some(*id), None],
        SketchEntity::Line { start, end, .. } => [Some(*start), Some(*end)],
        SketchEntity::Circle { center, .. }
        | SketchEntity::Arc { center, .. }
        | SketchEntity::Ellipse { center, .. } => [Some(*center), None],
    }
}

fn locked_point(sketch: &Sketch, id: EntityId) -> bool {
    !matches!(
        sketch.get_entity(id),
        Some(SketchEntity::Point {
            reference_locked: false,
            ..
        })
    ) || sketch
        .entities()
        .iter()
        .any(|entity| entity.is_reference_locked() && defining_points(entity).contains(&Some(id)))
}
