use super::*;

pub(super) fn initial_fields(session: &onecad_interaction::sketch::SketchSession) -> Vec<String> {
    let tool = session.tool;
    let Some(draft) = session.draft.as_ref() else {
        return Vec::new();
    };
    if session.numeric_method() == NumericCreationMethod::TangentArc {
        return vec![tangent_radius(session).to_string()];
    }
    let (anchor, cursor) = (draft.anchor, draft.cursor);
    let delta = Vec2::new_unchecked(cursor.x - anchor.x, cursor.y - anchor.y);
    let values = match tool {
        SketchTool::Point => vec![cursor.x, cursor.y],
        SketchTool::CenterRectangle => vec![2.0 * delta.x.abs(), 2.0 * delta.y.abs()],
        SketchTool::Line => vec![delta.x.hypot(delta.y)],
        SketchTool::Rectangle => vec![delta.x.abs(), delta.y.abs()],
        SketchTool::Circle => vec![2.0 * delta.x.hypot(delta.y)],
        SketchTool::Arc => arc_values(draft),
        SketchTool::Slot => slot_values(draft),
        SketchTool::ThreePointArc => vec![three_point_radius(draft)],
        SketchTool::Select => Vec::new(),
    };
    values
        .into_iter()
        .map(|value| {
            if tool == SketchTool::Point || value.is_finite() && value > 0.0 {
                value
            } else {
                value.max(1.0)
            }
            .to_string()
        })
        .collect()
}

fn tangent_radius(session: &onecad_interaction::sketch::SketchSession) -> f64 {
    let valid = CreationDimensions::TangentArc {
        radius: Scalar::new(1.0),
    }
    .preview_for_session(session)
    .is_ok();
    match session.preview.as_ref() {
        Some(onecad_interaction::sketch::DraftPrimitive::Arc { arc }) if valid => arc.radius,
        _ => 1.0,
    }
}

fn three_point_radius(draft: &SketchDraft) -> f64 {
    draft
        .secondary
        .as_ref()
        .and_then(|second| {
            onecad_interaction::sketch::arc_geometry::through_points(
                draft.anchor,
                second.point,
                draft.cursor,
            )
            .ok()
        })
        .map_or(1.0, |arc| arc.radius)
}

fn arc_values(draft: &SketchDraft) -> Vec<f64> {
    let Some(start) = draft.secondary.as_ref().map(|anchor| anchor.point) else {
        return Vec::new();
    };
    let radius = (start.x - draft.anchor.x).hypot(start.y - draft.anchor.y);
    let sweep = onecad_interaction::sketch::arc_geometry::center_start_end(
        draft.anchor,
        start,
        draft.cursor,
    )
    .map_or(90.0, |arc| arc.sweep.to_degrees());
    vec![radius, sweep]
}

fn slot_values(draft: &SketchDraft) -> Vec<f64> {
    let Some(end) = draft.secondary.as_ref().map(|anchor| anchor.point) else {
        return Vec::new();
    };
    let (dx, dy) = (end.x - draft.anchor.x, end.y - draft.anchor.y);
    vec![
        2.0 * (dx * (draft.cursor.y - draft.anchor.y) - dy * (draft.cursor.x - draft.anchor.x))
            .abs()
            / dx.hypot(dy),
    ]
}

pub(super) fn dimensions(draft: &Draft, app: &App) -> Result<CreationDimensions, String> {
    let variables = app
        .display
        .as_ref()
        .map_or(&[][..], |display| display.projection.variables.as_slice());
    let values: Vec<Scalar> = draft
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let text = &field.text;
            if draft.tool == SketchTool::Arc && index == 1 {
                crate::constraint_authoring::angle_scalar(text, variables)
            } else if draft.tool == SketchTool::Point {
                crate::constraint_authoring::coordinate_scalar(text, variables)
            } else {
                crate::constraint_authoring::creation_scalar(text, variables)
            }
        })
        .collect::<Result<_, _>>()?;
    if draft.method == NumericCreationMethod::TangentArc {
        return Ok(CreationDimensions::TangentArc {
            radius: values[0].clone(),
        });
    }
    Ok(match draft.tool {
        SketchTool::Point => CreationDimensions::Point {
            x: values[0].clone(),
            y: values[1].clone(),
        },
        SketchTool::CenterRectangle => CreationDimensions::CenterRectangle {
            width: values[0].clone(),
            height: values[1].clone(),
        },
        SketchTool::Line => CreationDimensions::Line {
            length: values[0].clone(),
        },
        SketchTool::Rectangle => CreationDimensions::Rectangle {
            width: values[0].clone(),
            height: values[1].clone(),
        },
        SketchTool::Circle => CreationDimensions::Circle {
            diameter: values[0].clone(),
        },
        SketchTool::Arc => CreationDimensions::Arc {
            radius: values[0].clone(),
            sweep: values[1].value,
        },
        SketchTool::Slot => CreationDimensions::Slot {
            width: values[0].clone(),
        },
        SketchTool::ThreePointArc => CreationDimensions::ThreePointArc {
            radius: values[0].clone(),
        },
        SketchTool::Select => return Err("Choose a creation tool".into()),
    })
}
