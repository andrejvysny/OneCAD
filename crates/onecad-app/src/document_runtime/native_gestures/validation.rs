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
