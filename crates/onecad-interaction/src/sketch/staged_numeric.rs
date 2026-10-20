use super::{
    arc_geometry,
    primitives::{PrimitiveBatch, MIN_SIZE},
    *,
};
use onecad_core::{math::Vec2, sketch::CurvePosition};

pub(super) fn session_endpoint(
    dimensions: &CreationDimensions,
    session: &SketchSession,
) -> Result<Vec2, String> {
    let draft = session.draft.as_ref().ok_or("No primitive anchor")?;
    if matches!(dimensions, CreationDimensions::TangentArc { .. }) {
        return session_arc(dimensions, session).map(travel_end);
    }
    endpoint(dimensions, draft)
}

fn travel_end(arc: arc_geometry::AnalyticArc) -> Vec2 {
    let angle = if arc.clockwise {
        arc.start_angle
    } else {
        arc.end_angle
    };
    Vec2::new_unchecked(
        arc.center.x + arc.radius * angle.cos(),
        arc.center.y + arc.radius * angle.sin(),
    )
}

pub(super) fn session_arc(
    dimensions: &CreationDimensions,
    session: &SketchSession,
) -> Result<arc_geometry::AnalyticArc, String> {
    let draft = session.draft.as_ref().ok_or("No primitive anchor")?;
    let CreationDimensions::TangentArc { radius } = dimensions else {
        return draft_arc(dimensions, draft);
    };
    valid_radius(radius.value)?;
    let preceding = draft
        .preceding
        .ok_or("Tangent arc needs a preceding chain curve")?;
    let tangent = super::tangent_continuation::outgoing(&session.authoritative, preceding)?;
    let accepted =
        super::tangent_continuation::anchor(&session.authoritative, draft.anchor_id, preceding)?;
    if super::primitives::distance(accepted, draft.anchor) >= MIN_SIZE {
        return Err("Tangent arc anchor no longer matches the solved endpoint".into());
    }
    let pointer = arc_geometry::tangent_to(accepted, tangent, draft.cursor)
        .map_err(|error| error.to_string())?;
    let scale = radius.value / pointer.radius;
    let end = Vec2::new(
        accepted.x + (draft.cursor.x - accepted.x) * scale,
        accepted.y + (draft.cursor.y - accepted.y) * scale,
    )
    .ok_or("Numeric arc coordinates exceed finite precision")?;
    arc_geometry::tangent_to(accepted, tangent, end).map_err(|error| error.to_string())
}

pub(super) fn draft_arc(
    dimensions: &CreationDimensions,
    draft: &SketchDraft,
) -> Result<arc_geometry::AnalyticArc, String> {
    match dimensions {
        CreationDimensions::Arc { radius, sweep } => arc(draft, radius.value, *sweep),
        CreationDimensions::ThreePointArc { radius } => three_point(draft, radius.value),
        _ => Err("Numeric method does not describe an arc".into()),
    }
}

fn valid_radius(radius: f64) -> Result<(), String> {
    if !radius.is_finite() || radius < MIN_SIZE {
        Err("Arc radius must be finite and at least 1e-9".into())
    } else {
        Ok(())
    }
}

fn three_point(draft: &SketchDraft, radius: f64) -> Result<arc_geometry::AnalyticArc, String> {
    valid_radius(radius)?;
    let end = draft
        .secondary
        .as_ref()
        .ok_or("Place both arc endpoints first")?
        .point;
    let selected = arc_geometry::through_points(draft.anchor, end, draft.cursor)
        .map_err(|error| error.to_string())?;
    let dx = end.x - draft.anchor.x;
    let dy = end.y - draft.anchor.y;
    let chord = dx.hypot(dy);
    let half = chord * 0.5;
    if radius < half {
        return Err("Arc radius must be at least half the endpoint chord".into());
    }
    let midpoint = Vec2::new(draft.anchor.x + dx * 0.5, draft.anchor.y + dy * 0.5)
        .ok_or("Numeric arc coordinates exceed finite precision")?;
    let (nx, ny) = (-dy / chord, dx / chord);
    let side = (selected.center.x - midpoint.x) * nx + (selected.center.y - midpoint.y) * ny;
    // Factored form avoids overflow in radius squared and cancellation at half-chord.
    let height = radius * ((1.0 - half / radius) * (1.0 + half / radius)).sqrt();
    let offset = if side < 0.0 { -height } else { height };
    let center = Vec2::new(midpoint.x + nx * offset, midpoint.y + ny * offset)
        .ok_or("Numeric arc coordinates exceed finite precision")?;
    let mut result = if selected.clockwise {
        arc_geometry::center_start_end(center, end, draft.anchor)
    } else {
        arc_geometry::center_start_end(center, draft.anchor, end)
    }
    .map_err(|error| error.to_string())?;
    if selected.clockwise {
        result.clockwise = true;
        result.travel_start_role = CurvePosition::End;
        result.travel_end_role = CurvePosition::Start;
        result.exit_tangent =
            Vec2::new_unchecked(result.start_angle.sin(), -result.start_angle.cos());
    }
    Ok(result)
}

pub(super) fn endpoint(
    dimensions: &CreationDimensions,
    draft: &SketchDraft,
) -> Result<Vec2, String> {
    match dimensions {
        CreationDimensions::Arc { .. } | CreationDimensions::ThreePointArc { .. } => {
            draft_arc(dimensions, draft).map(travel_end)
        }
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
    if matches!(
        dimensions,
        CreationDimensions::Arc { .. } | CreationDimensions::ThreePointArc { .. }
    ) {
        super::primitives::validate_ids(session, ids)?;
        let arc = session_arc(dimensions, session)?;
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
