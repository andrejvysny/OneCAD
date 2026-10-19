use super::{
    arc_geometry::{self, AnalyticArc},
    inference::AcceptedSnap,
    primitives::{distance, PrimitiveBatch, MIN_SIZE},
    *,
};
use onecad_core::{edit::SketchEditOp, math::Vec2, sketch::SketchEntity};

pub(super) fn is_staged(tool: SketchTool) -> bool {
    matches!(
        tool,
        SketchTool::Arc | SketchTool::ThreePointArc | SketchTool::Slot
    )
}

pub(super) fn second_anchor(
    session: &mut SketchSession,
    point: Vec2,
    snap: Option<AcceptedSnap>,
) -> Result<(), String> {
    let draft = session.draft.as_mut().ok_or("No first primitive anchor")?;
    let length = distance(draft.anchor, point);
    if !length.is_finite() || length < MIN_SIZE {
        return Err("Second primitive anchor must be distinct and finite".into());
    }
    draft.secondary = Some(PrimitiveAnchor { point, snap });
    draft.cursor = point;
    session.preview = Some(DraftPrimitive::Line {
        start: draft.anchor,
        end: point,
    });
    Ok(())
}

pub(super) fn analytic(
    tool: SketchTool,
    draft: &SketchDraft,
    point: Vec2,
) -> Result<AnalyticArc, String> {
    let second = draft
        .secondary
        .as_ref()
        .ok_or("Place the second arc anchor first")?
        .point;
    match tool {
        SketchTool::Arc => arc_geometry::center_start_end(draft.anchor, second, point),
        SketchTool::ThreePointArc => arc_geometry::through_points(draft.anchor, second, point),
        _ => return Err("Active tool does not author an arc".into()),
    }
    .map_err(|error| error.to_string())
}

pub(super) fn preview(
    tool: SketchTool,
    draft: &SketchDraft,
    point: Vec2,
) -> Result<DraftPrimitive, String> {
    let Some(second) = draft.secondary.as_ref() else {
        return Ok(DraftPrimitive::Line {
            start: draft.anchor,
            end: point,
        });
    };
    if tool == SketchTool::Slot {
        let slot = super::slot_creation::geometry(draft.anchor, second.point, point)?;
        Ok(DraftPrimitive::Slot {
            center_start: draft.anchor,
            center_end: second.point,
            radius: slot.radius,
        })
    } else {
        analytic(tool, draft, point).map(|arc| DraftPrimitive::Arc { arc })
    }
}

pub(super) fn build(
    tool: SketchTool,
    draft: &SketchDraft,
    point: Vec2,
    ids: &PrimitiveIds,
) -> Result<PrimitiveBatch, String> {
    if tool == SketchTool::Slot {
        let second = draft
            .secondary
            .as_ref()
            .ok_or("Place the second slot center first")?;
        return super::slot_creation::build(draft.anchor, second.point, point, ids);
    }
    let arc = analytic(tool, draft, point)?;
    Ok((arc_ops(arc, ids)?, AfterPrimitive::Reset))
}

pub(super) fn arc_ops(arc: AnalyticArc, ids: &PrimitiveIds) -> Result<Vec<SketchEditOp>, String> {
    let entity = SketchEntity::arc(
        ids.entities[1],
        ids.entities[0],
        arc.radius,
        arc.start_angle,
        arc.end_angle,
        false,
    )
    .ok_or("Arc geometry must be finite")?;
    Ok(vec![
        SketchEditOp::AddEntity {
            entity: SketchEntity::point(ids.entities[0], arc.center, false, false),
        },
        SketchEditOp::AddEntity { entity },
    ])
}
