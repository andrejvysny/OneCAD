//! Persist each accepted click once, against the geometry on which it was picked.
use super::{
    inference::{persist_snap, source_for, AcceptedSnap, SnapIntent},
    primitives::PrimitiveBatch,
    *,
};
use onecad_core::{
    edit::SketchEditOp,
    sketch::{Constraint, CurvePosition, SketchEntity},
};
use onecad_scene::CaptureIdentity;

pub(super) fn append(
    session: &SketchSession,
    capture: CaptureIdentity,
    point: onecad_core::math::Vec2,
    ids: &PrimitiveIds,
    snap: Option<AcceptedSnap>,
    (mut ops, after): PrimitiveBatch,
) -> Result<PrimitiveBatch, String> {
    if super::staged_creation::is_staged(session.tool) {
        return super::staged_inference::append(session, capture, point, ids, snap, (ops, after));
    }
    let current = source_for(&session.authoritative, capture, session.request.sequence);
    let mut next = 4;
    let mut next_id = || {
        let id = *ids
            .constraints
            .get(next)
            .expect("bounded primitive inference budget");
        next += 1;
        id
    };
    if session.tool != SketchTool::Point {
        let anchor = if session.tool == SketchTool::CenterRectangle {
            ids.entities[8]
        } else {
            ids.entities[0]
        };
        append_anchor(session, current, anchor, &mut next_id, &mut ops)?;
    }
    if let Some(snap) = snap.filter(|snap| snap.intent != SnapIntent::GeometryOnly) {
        let constraints =
            snap_constraints(session, &snap, current, point, ids, &mut ops, &mut next_id)?;
        ops.extend(
            constraints
                .into_iter()
                .map(|constraint| SketchEditOp::AddConstraint { constraint }),
        );
    }
    Ok((ops, after))
}

fn snap_constraints(
    session: &SketchSession,
    snap: &AcceptedSnap,
    current: super::inference::SnapSource,
    point: onecad_core::math::Vec2,
    ids: &PrimitiveIds,
    ops: &mut Vec<SketchEditOp>,
    next_id: &mut impl FnMut() -> onecad_core::ids::ConstraintId,
) -> Result<Vec<Constraint>, String> {
    if tangent_arc(session) {
        return super::tangent_continuation::persist_final(
            session,
            snap,
            current,
            point,
            ids,
            ops.as_slice(),
            next_id,
        );
    }
    let Some(endpoint) = endpoint(session.tool, point, ids, next_id, ops) else {
        return Ok(Vec::new());
    };
    persist_snap(&session.authoritative, snap, current, endpoint, next_id).map_err(String::from)
}

fn tangent_arc(session: &SketchSession) -> bool {
    session.tool == SketchTool::Line && session.draft.as_ref().is_some_and(|d| d.tangent_arc)
}

fn append_anchor(
    session: &SketchSession,
    current: super::inference::SnapSource,
    point: onecad_core::ids::EntityId,
    next_id: &mut impl FnMut() -> onecad_core::ids::ConstraintId,
    ops: &mut Vec<SketchEditOp>,
) -> Result<(), String> {
    let draft = session.draft.as_ref().ok_or("No primitive anchor")?;
    let Some(anchor) = draft.anchor_snap.filter(|_| draft.anchor_id.is_none()) else {
        return Ok(());
    };
    if anchor.source.generation != current.generation
        || anchor.source.capture.scene != current.capture.scene
        || anchor.source.geometry_fingerprint != current.geometry_fingerprint
    {
        return Err("Accepted anchor inference belongs to an obsolete sketch".into());
    }
    ops.extend(
        persist_snap(
            &session.authoritative,
            &anchor,
            anchor.source,
            point,
            next_id,
        )?
        .into_iter()
        .map(|constraint| SketchEditOp::AddConstraint { constraint }),
    );
    Ok(())
}

fn endpoint(
    tool: SketchTool,
    point: onecad_core::math::Vec2,
    ids: &PrimitiveIds,
    next_id: &mut impl FnMut() -> onecad_core::ids::ConstraintId,
    ops: &mut Vec<SketchEditOp>,
) -> Option<onecad_core::ids::EntityId> {
    match tool {
        SketchTool::Rectangle | SketchTool::CenterRectangle => Some(ids.entities[2]),
        SketchTool::Point => Some(ids.entities[0]),
        SketchTool::Line => ops
            .iter()
            .any(|op| {
                matches!(op,
            SketchEditOp::AddEntity { entity } if entity.id() == ids.entities[1])
            })
            .then_some(ids.entities[1]),
        SketchTool::Circle => {
            let id = ids.entities[2];
            ops.push(SketchEditOp::AddEntity {
                entity: SketchEntity::point(id, point, true, false),
            });
            ops.push(SketchEditOp::AddConstraint {
                constraint: Constraint::OnCurve {
                    id: next_id(),
                    point: id,
                    point_position: CurvePosition::Arbitrary,
                    curve: ids.entities[1],
                    position: CurvePosition::Arbitrary,
                },
            });
            Some(id)
        }
        SketchTool::Select | SketchTool::Arc | SketchTool::ThreePointArc | SketchTool::Slot => None,
    }
}
