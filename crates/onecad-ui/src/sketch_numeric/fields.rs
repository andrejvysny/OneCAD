use super::format::{format_angle_degrees, format_measure};
use super::*;

/// Display text plus the full-precision value untouched fields commit from.
pub(super) fn initial_fields(
    session: &onecad_interaction::sketch::SketchSession,
) -> Vec<(String, f64)> {
    let tool = session.tool;
    let Some(draft) = session.draft.as_ref() else {
        return Vec::new();
    };
    if session.numeric_method() == NumericCreationMethod::TangentArc {
        let radius = tangent_radius(session);
        return vec![(format_measure(radius), radius)];
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
        .enumerate()
        .map(|(index, value)| {
            let value = if tool == SketchTool::Point || value.is_finite() && value > 0.0 {
                value
            } else {
                value.max(1.0)
            };
            let text = if tool == SketchTool::Arc && index == 1 {
                format_angle_degrees(value)
            } else {
                format_measure(value)
            };
            (text, value)
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
        .doc
        .display
        .as_ref()
        .map_or(&[][..], |display| display.projection.variables.as_slice());
    let values: Vec<Scalar> = draft
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| field_scalar(draft, index, field, variables))
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

fn field_scalar(
    draft: &Draft,
    index: usize,
    field: &FieldDraft,
    variables: &[onecad_app::dto::VariableDto],
) -> Result<Scalar, String> {
    let sweep = draft.tool == SketchTool::Arc && index == 1;
    if !field.authored && !field.measured.is_finite() {
        return Err("Creation values must be finite".into());
    }
    if !field.authored {
        // Display text is rounded; geometry must keep the measured value.
        let value = if sweep {
            field.measured.to_radians()
        } else {
            field.measured
        };
        return Ok(Scalar::new(value));
    }
    let text = &field.text;
    if sweep {
        crate::constraint_authoring::angle_scalar(text, variables)
    } else if draft.tool == SketchTool::Point {
        crate::constraint_authoring::coordinate_scalar(text, variables)
    } else {
        crate::constraint_authoring::creation_scalar(text, variables)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(tool: SketchTool, authored: bool) -> Draft {
        let mut draft = super::super::editor::tests::draft();
        draft.tool = tool;
        draft.fields = vec![
            FieldDraft {
                field: CreationField::Radius,
                text: "1.79".into(),
                measured: 1.7949439391,
                authored,
            },
            FieldDraft {
                field: CreationField::Sweep,
                text: "90".into(),
                measured: 89.99,
                authored,
            },
        ];
        draft
    }

    #[test]
    fn untouched_fields_commit_full_precision_not_rounded_text() {
        let draft = draft(SketchTool::Arc, false);
        let radius = field_scalar(&draft, 0, &draft.fields[0], &[]).unwrap();
        assert_eq!(radius.value, 1.7949439391);
        let sweep = field_scalar(&draft, 1, &draft.fields[1], &[]).unwrap();
        assert_eq!(sweep.value, 89.99_f64.to_radians());
    }

    #[test]
    fn authored_fields_commit_the_typed_text() {
        let draft = draft(SketchTool::Arc, true);
        let radius = field_scalar(&draft, 0, &draft.fields[0], &[]).unwrap();
        assert_eq!(radius.value, 1.79);
        let sweep = field_scalar(&draft, 1, &draft.fields[1], &[]).unwrap();
        assert_eq!(sweep.value, 90.0_f64.to_radians());
    }
}
