use super::{
    inference::{persist_snap_to, source_for, AcceptedSnap, SnapIntent, SnapSource},
    primitives::PrimitiveBatch,
    *,
};
use onecad_core::{
    edit::SketchEditOp,
    ids::{ConstraintId, EntityId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, SketchEntity},
};
use onecad_scene::CaptureIdentity;

pub(super) fn append(
    session: &SketchSession,
    capture: CaptureIdentity,
    point: Vec2,
    ids: &PrimitiveIds,
    snap: Option<AcceptedSnap>,
    (mut ops, after): PrimitiveBatch,
) -> Result<PrimitiveBatch, String> {
    let draft = session.draft.as_ref().ok_or("No first primitive anchor")?;
    let second = draft
        .secondary
        .as_ref()
        .ok_or("No second primitive anchor")?;
    let current = source_for(&session.authoritative, capture, session.request.sequence);
    let snaps = [draft.anchor_snap, second.snap, snap];
    for accepted in snaps.into_iter().flatten() {
        valid_source(accepted, current)?;
    }
    let required = snaps
        .into_iter()
        .flatten()
        .map(|s| count(s.intent))
        .sum::<usize>()
        + 1;
    let pool = ids
        .constraints
        .get(12..12 + required)
        .ok_or("Primitive inference identity budget exceeded")?;
    let mut available = pool.iter();
    let mut next = || {
        *available
            .next()
            .expect("preflight covers exact snap arity and optional helper")
    };
    let destinations = destinations(session.tool, draft, point, ids, snap, &mut ops, &mut next)?;
    let additions: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            SketchEditOp::AddEntity { entity } => Some(entity.clone()),
            _ => None,
        })
        .collect();
    for (accepted, destination) in snaps.into_iter().zip(destinations) {
        let Some(accepted) = accepted.filter(|s| s.intent != SnapIntent::GeometryOnly) else {
            continue;
        };
        let target = destination.ok_or("Accepted inference has no primitive target")?;
        let constraints = persist_snap_to(
            &session.authoritative,
            &accepted,
            accepted.source,
            target,
            &additions,
            &mut next,
        )?;
        ops.extend(
            constraints
                .into_iter()
                .map(|constraint| SketchEditOp::AddConstraint { constraint }),
        );
    }
    Ok((ops, after))
}

fn count(intent: SnapIntent) -> usize {
    match intent {
        SnapIntent::GeometryOnly => 0,
        SnapIntent::Intersection { .. }
        | SnapIntent::Quadrant { .. }
        | SnapIntent::Guides { .. } => 2,
        _ => 1,
    }
}

fn valid_source(accepted: AcceptedSnap, current: SnapSource) -> Result<(), String> {
    if accepted.source.sketch != current.sketch
        || accepted.source.generation != current.generation
        || accepted.source.capture.scene != current.capture.scene
        || accepted.source.geometry_fingerprint != current.geometry_fingerprint
    {
        return Err("Accepted primitive inference belongs to an obsolete sketch".into());
    }
    Ok(())
}

fn target(entity: EntityId, role: CurvePosition) -> SketchTarget {
    SketchTarget::Point { entity, role }
}
fn raw(entity: EntityId) -> SketchTarget {
    target(entity, CurvePosition::Arbitrary)
}

fn destinations(
    tool: SketchTool,
    draft: &SketchDraft,
    point: Vec2,
    ids: &PrimitiveIds,
    snap: Option<AcceptedSnap>,
    ops: &mut Vec<SketchEditOp>,
    next: &mut impl FnMut() -> ConstraintId,
) -> Result<[Option<SketchTarget>; 3], String> {
    let second = draft
        .secondary
        .as_ref()
        .ok_or("No second primitive anchor")?;
    let persistent_final = snap.is_some_and(|s| s.intent != SnapIntent::GeometryOnly);
    if tool == SketchTool::Slot {
        let slot = super::slot_creation::geometry(draft.anchor, second.point, point)?;
        let last = if persistent_final {
            if slot.along < 0.0 || slot.along > slot.length {
                return Err("Slot width inference outside the finite wall span is unsupported; suppress snap and retry".into());
            }
            helper(
                ids.entities[10],
                point,
                ids.entities[6 + slot.side],
                ops,
                next,
            );
            Some(raw(ids.entities[10]))
        } else {
            None
        };
        return Ok([Some(raw(ids.entities[4])), Some(raw(ids.entities[5])), last]);
    }
    let first_point = if tool == SketchTool::Arc {
        second.point
    } else {
        draft.anchor
    };
    let start_role = input_start_role(ops, ids, first_point)?;
    let end_role = if start_role == CurvePosition::Start {
        CurvePosition::End
    } else {
        CurvePosition::Start
    };
    let arc_start = Some(target(ids.entities[1], start_role));
    let arc_end = Some(target(ids.entities[1], end_role));
    if tool == SketchTool::Arc {
        return Ok([Some(raw(ids.entities[0])), arc_start, arc_end]);
    }
    let last = if persistent_final {
        helper(ids.entities[2], point, ids.entities[1], ops, next);
        Some(raw(ids.entities[2]))
    } else {
        None
    };
    Ok([arc_start, arc_end, last])
}

fn helper(
    id: EntityId,
    point: Vec2,
    curve: EntityId,
    ops: &mut Vec<SketchEditOp>,
    next: &mut impl FnMut() -> ConstraintId,
) {
    ops.push(SketchEditOp::AddEntity {
        entity: SketchEntity::point(id, point, true, false),
    });
    ops.push(SketchEditOp::AddConstraint {
        constraint: Constraint::OnCurve {
            id: next(),
            point: id,
            point_position: CurvePosition::Arbitrary,
            curve,
            position: CurvePosition::Arbitrary,
        },
    });
}

fn input_start_role(
    ops: &[SketchEditOp],
    ids: &PrimitiveIds,
    point: Vec2,
) -> Result<CurvePosition, String> {
    let center = ops
        .iter()
        .find_map(|op| match op {
            SketchEditOp::AddEntity {
                entity: SketchEntity::Point { id, at, .. },
            } if *id == ids.entities[0] => Some(*at),
            _ => None,
        })
        .ok_or("Arc center is missing")?;
    let (start, end) = ops
        .iter()
        .find_map(|op| match op {
            SketchEditOp::AddEntity {
                entity:
                    SketchEntity::Arc {
                        id,
                        start_angle,
                        end_angle,
                        ..
                    },
            } if *id == ids.entities[1] => Some((*start_angle, *end_angle)),
            _ => None,
        })
        .ok_or("Arc is missing")?;
    let angle = (point.y - center.y).atan2(point.x - center.x);
    for (candidate, role) in [(start, CurvePosition::Start), (end, CurvePosition::End)] {
        let difference = (angle - candidate).rem_euclid(std::f64::consts::TAU);
        if difference.min(std::f64::consts::TAU - difference) <= 32.0 * f64::EPSILON {
            return Ok(role);
        }
    }
    Err("Arc candidate no longer preserves its accepted start direction".into())
}
