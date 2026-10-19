use super::{
    arc_geometry,
    primitives::{PrimitiveBatch, MIN_SIZE},
    *,
};
use onecad_core::{math::Vec2, sketch::CurvePosition};

pub(super) fn endpoint(
    dimensions: &CreationDimensions,
    draft: &SketchDraft,
) -> Result<Vec2, String> {
    match dimensions {
        CreationDimensions::Arc { radius, sweep } => arc(draft, radius.value, *sweep).map(|arc| {
            let angle = if arc.clockwise {
                arc.start_angle
            } else {
                arc.end_angle
            };
            Vec2::new_unchecked(
                arc.center.x + arc.radius * angle.cos(),
                arc.center.y + arc.radius * angle.sin(),
            )
        }),
        CreationDimensions::Slot { width } => slot_endpoint(draft, width.value),
        _ => dimensions.endpoint(draft.anchor, draft.cursor),
    }
}

pub(super) fn arc(
    draft: &SketchDraft,
    radius: f64,
    sweep: f64,
) -> Result<arc_geometry::AnalyticArc, String> {
    if !radius.is_finite() || radius < MIN_SIZE {
        return Err("Arc radius must be finite and at least 1e-9".into());
    }
    if !sweep.is_finite() || sweep == 0.0 || sweep.abs() >= std::f64::consts::TAU {
        return Err("Arc sweep must be nonzero and strictly between -360 and 360 degrees".into());
    }
    let second = draft
        .secondary
        .as_ref()
        .ok_or("Place the arc start before numeric creation")?;
    let angle = (second.point.y - draft.anchor.y).atan2(second.point.x - draft.anchor.x);
    let at = |angle: f64| {
        Vec2::new(
            draft.anchor.x + radius * angle.cos(),
            draft.anchor.y + radius * angle.sin(),
        )
        .ok_or_else(|| "Numeric arc coordinates exceed finite precision".to_owned())
    };
    let start = at(angle)?;
    let end = at(angle + sweep)?;
    let mut arc = if sweep > 0.0 {
        arc_geometry::center_start_end(draft.anchor, start, end)
    } else {
        arc_geometry::center_start_end(draft.anchor, end, start)
    }
    .map_err(|error| error.to_string())?;
    if sweep < 0.0 {
        arc.clockwise = true;
        arc.travel_start_role = CurvePosition::End;
        arc.travel_end_role = CurvePosition::Start;
        arc.exit_tangent = Vec2::new_unchecked(arc.start_angle.sin(), -arc.start_angle.cos());
    }
    Ok(arc)
}

fn slot_endpoint(draft: &SketchDraft, width: f64) -> Result<Vec2, String> {
    let radius = width * 0.5;
    if !radius.is_finite() || radius < MIN_SIZE {
        return Err("Slot width must be finite and at least 2e-9".into());
    }
    let second = draft
        .secondary
        .as_ref()
        .ok_or("Place both slot centers before numeric creation")?;
    let (tangent, normal, _, along, side) =
        super::slot_creation::axes(draft.anchor, second.point, draft.cursor)?;
    let offset = if side < 0.0 { -radius } else { radius };
    Vec2::new(
        draft.anchor.x + tangent.x * along + normal.x * offset,
        draft.anchor.y + tangent.y * along + normal.y * offset,
    )
    .ok_or_else(|| "Numeric slot coordinates exceed finite precision".into())
}

pub(super) fn build(
    dimensions: &CreationDimensions,
    session: &SketchSession,
    endpoint: Vec2,
    ids: &PrimitiveIds,
) -> Result<PrimitiveBatch, String> {
    if let CreationDimensions::Arc { radius, sweep } = dimensions {
        super::primitives::validate_ids(session, ids)?;
        let draft = session.draft.as_ref().ok_or("No primitive anchor")?;
        let arc = arc(draft, radius.value, *sweep)?;
        let mut ops = super::staged_creation::arc_ops(arc, ids)?;
        if session.construction {
            for op in &mut ops {
                if let onecad_core::edit::SketchEditOp::AddEntity {
                    entity:
                        onecad_core::sketch::SketchEntity::Point { construction, .. }
                        | onecad_core::sketch::SketchEntity::Arc { construction, .. },
                } = op
                {
                    *construction = true;
                }
            }
        }
        Ok((ops, AfterPrimitive::Reset))
    } else {
        super::primitives::build_primitive(session, endpoint, ids)
    }
}
