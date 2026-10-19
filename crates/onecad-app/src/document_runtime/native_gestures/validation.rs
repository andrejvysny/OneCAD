use super::*;
use onecad_core::sketch::SketchEntity;

pub(super) fn target(sketch: &Sketch, target: wire::GestureTarget) -> Result<(), EngineError> {
    use wire::DragKind;
    if target
        .grab
        .is_some_and(|at| !at.iter().all(|v| v.is_finite()))
    {
        return Err(op_failed("Native gesture: nonfinite grab"));
    }
    let entity = sketch
        .get_entity(target.entity)
        .ok_or_else(|| op_failed("Native gesture: target does not exist"))?;
    let supported = match (target.kind, entity) {
        (DragKind::Point, SketchEntity::Point { .. }) => target.role.is_none(),
        (DragKind::Radius, SketchEntity::Circle { .. } | SketchEntity::Arc { .. }) => {
            target.role.is_none()
        }
        (DragKind::ArcEnd, SketchEntity::Arc { .. }) => {
            matches!(target.role, Some("start" | "end"))
        }
        (
            DragKind::EntityBody,
            SketchEntity::Line { .. } | SketchEntity::Circle { .. } | SketchEntity::Arc { .. },
        ) => target.role.is_none(),
        _ => false,
    };
    let protected = match target.kind {
        DragKind::Point => point_protected(sketch, target.entity),
        DragKind::EntityBody => defining_points(entity)
            .into_iter()
            .flatten()
            .any(|id| point_protected(sketch, id)),
        _ => false,
    };
    if !supported || protected || entity.is_reference_locked() {
        return Err(op_failed("Native gesture: unsupported or locked target"));
    }
    Ok(())
}

pub(super) fn begin(
    baseline: &Sketch,
    pending: &SketchUpsertDto,
    ready: &crate::dto::BeginGestureDto,
    id: u64,
) -> Result<(Sketch, SketchUpsertDto), EngineError> {
    if ready.gesture_id != id
        || !ready.ready
        || ready
            .point_offset
            .is_some_and(|at| !at.iter().all(|v| v.is_finite()))
    {
        return Err(op_failed("Native gesture: invalid Begin receipt"));
    }
    let mut correction = pending.clone();
    correction.solved_positions.clone_from(&ready.positions);
    correction.solved_curves.clone_from(&ready.curves);
    let baseline =
        crate::worker::sketch_result::adopted(baseline, &correction).map_err(op_failed)?;
    sketch_adoption::merge_pending_solve(&mut correction, pending.clone());
    Ok((baseline, correction))
}

pub(super) fn update(
    baseline: &Sketch,
    pending: &SketchUpsertDto,
    solved: &crate::dto::DragSolveDto,
    id: u64,
    seq: u64,
) -> Result<Sketch, EngineError> {
    if solved.gesture_id != id
        || solved.seq != seq
        || solved.superseded
        || !matches!(
            solved.status.as_str(),
            "success" | "partial" | "conflicting" | "redundant"
        )
    {
        return Err(op_failed("Native gesture: invalid Update receipt"));
    }
    let mut candidate = pending.clone();
    candidate.solved_positions.clone_from(&solved.positions);
    candidate.solved_curves.clone_from(&solved.curves);
    crate::worker::sketch_result::adopted(baseline, &candidate).map_err(op_failed)
}

/// True when the final pointer intent reproduces the accepted Begin pose exactly.
/// Re-solving an accepted pose can only add solver roundoff, and solver branch
/// history must not turn a return to the starting intent into a document edit.
pub(super) fn unchanged_intent(
    sketch: &Sketch,
    target: wire::GestureTarget,
    end: Option<[f64; 2]>,
    updated: bool,
) -> bool {
    use wire::DragKind;
    // Without a final target the worker keeps its last preview pose.
    let Some(end) = end else {
        return !updated;
    };
    let Some(entity) = sketch.get_entity(target.entity) else {
        return false;
    };
    match (target.kind, target.grab) {
        // ArcEnd ignores grab: its endpoint follows the absolute target.
        (DragKind::ArcEnd, _) => {
            let start = target.role == Some("start");
            crate::worker::sketch_result::arc_endpoint(sketch, target.entity, start)
                .is_ok_and(|at| [at.x, at.y] == end)
        }
        // These kinds retain their Begin grab offset, so the grab reproduces the pose.
        (DragKind::Point | DragKind::Radius | DragKind::EntityBody, Some(grab)) => grab == end,
        // Absent grab anchors on the first owned point in worker handle order.
        (DragKind::Point | DragKind::EntityBody, None) => defining_points(entity)[0]
            .and_then(|id| point_position(sketch, id))
            .is_some_and(|at| at == end),
        // Absent grab means zero radial offset: the target distance is the radius.
        (DragKind::Radius, None) => match entity {
            SketchEntity::Circle { center, radius, .. }
            | SketchEntity::Arc { center, radius, .. } => point_position(sketch, *center)
                .is_some_and(|c| (end[0] - c[0]).hypot(end[1] - c[1]) == *radius),
            _ => false,
        },
    }
}

fn point_position(sketch: &Sketch, id: EntityId) -> Option<[f64; 2]> {
    match sketch.get_entity(id) {
        Some(SketchEntity::Point { at, .. }) => Some([at.x, at.y]),
        _ => None,
    }
}

fn defining_points(entity: &SketchEntity) -> [Option<EntityId>; 2] {
    match entity {
        SketchEntity::Point { id, .. } => [Some(*id), None],
        SketchEntity::Line { start, end, .. } => [Some(*start), Some(*end)],
        SketchEntity::Circle { center, .. }
        | SketchEntity::Arc { center, .. }
        | SketchEntity::Ellipse { center, .. } => [Some(*center), None],
    }
}

fn point_protected(sketch: &Sketch, point: EntityId) -> bool {
    sketch.entities().iter().any(|entity| {
        entity.is_reference_locked()
            && defining_points(entity)
                .into_iter()
                .flatten()
                .any(|id| id == point)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::{ids::SketchId, math::Vec2, sketch::WorldPlane};
    use uuid::Uuid;
    use wire::{DragKind, GestureTarget};

    fn eid(n: u128) -> EntityId {
        EntityId(Uuid::from_u128(n))
    }

    fn sketch() -> Sketch {
        let mut sketch = Sketch::on_world_plane(SketchId::new(), "intent", WorldPlane::XY);
        for (n, x, y) in [(1, 0., 5.), (2, 0., 0.), (3, 20., 0.)] {
            let at = Vec2::new_unchecked(x, y);
            sketch
                .add_entity(SketchEntity::point(eid(n), at, false, false))
                .unwrap();
        }
        let arc = SketchEntity::arc(eid(10), eid(1), 5., -std::f64::consts::FRAC_PI_2, 0., false);
        sketch.add_entity(arc.unwrap()).unwrap();
        let line = SketchEntity::line(eid(11), eid(2), eid(3), false);
        sketch.add_entity(line).unwrap();
        sketch
    }

    fn target(kind: DragKind, entity: u128, grab: Option<[f64; 2]>) -> GestureTarget {
        let role = (kind == DragKind::ArcEnd).then_some("start");
        GestureTarget {
            kind,
            entity: eid(entity),
            role,
            grab,
        }
    }

    #[test]
    fn grab_relative_kinds_are_unchanged_only_at_their_exact_grab() {
        let sketch = sketch();
        let grab = [1.25, -0.5];
        for (kind, entity) in [
            (DragKind::Point, 2),
            (DragKind::Radius, 10),
            (DragKind::EntityBody, 11),
        ] {
            let target = target(kind, entity, Some(grab));
            assert!(unchanged_intent(&sketch, target, Some(grab), true));
            let moved = [grab[0], grab[1] + 1e-12];
            assert!(!unchanged_intent(&sketch, target, Some(moved), false));
        }
    }

    #[test]
    fn arc_end_ignores_grab_and_compares_the_accepted_endpoint() {
        let sketch = sketch();
        let at = crate::worker::sketch_result::arc_endpoint(&sketch, eid(10), true).unwrap();
        let endpoint = [at.x, at.y];
        let off = [endpoint[0] + 0.25, endpoint[1]];
        let target = target(DragKind::ArcEnd, 10, Some(off));
        assert!(unchanged_intent(&sketch, target, Some(endpoint), true));
        assert!(!unchanged_intent(&sketch, target, Some(off), false));
    }

    #[test]
    fn absent_grab_and_absent_final_target_follow_worker_anchors() {
        let sketch = sketch();
        let point = target(DragKind::Point, 2, None);
        assert!(unchanged_intent(&sketch, point, Some([0., 0.]), false));
        let body = target(DragKind::EntityBody, 11, None);
        assert!(unchanged_intent(&sketch, body, Some([0., 0.]), false));
        assert!(!unchanged_intent(&sketch, body, Some([20., 0.]), false));
        let radius = target(DragKind::Radius, 10, None);
        assert!(unchanged_intent(&sketch, radius, Some([3., 9.]), false));
        assert!(!unchanged_intent(&sketch, radius, Some([3., 9.5]), false));
        assert!(unchanged_intent(&sketch, point, None, false));
        assert!(!unchanged_intent(&sketch, point, None, true));
    }
}
