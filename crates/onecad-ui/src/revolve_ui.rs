//! Sketch-line axes let a profile become the first solid in an empty model.
use crate::app::App;
use onecad_core::{
    document::{
        record::{KnownOperation, Operation, RevolveParams},
        refs::AxisRef,
    },
    ids::{EntityId, RecordId, SketchId},
};
use onecad_interaction::{
    Action, ModelSession, ModelingOptions, modeling_records, operand_binding,
};
use onecad_widgets::components::overlays::FeatureCardRows;

fn lines(app: &App, sketch: SketchId) -> Vec<(EntityId, String)> {
    let Some(display) = &app.doc.display else {
        return Vec::new();
    };
    let Some(params) = display.records.iter().find_map(|record| match &record.op {
        Operation::Known(KnownOperation::Sketch(p)) if p.sketch == sketch => Some(p),
        _ => None,
    }) else {
        return Vec::new();
    };
    // SketchOpParams stores the worker's wire entities, including construction
    // lines. Preserve their authored ids; never synthesize a topology ordinal.
    let mut lines: Vec<_> = params
        .entities
        .iter()
        .filter_map(|entity| {
            if entity.get("type")?.as_str()? != "Line" {
                return None;
            }
            let id = entity.get("id")?.as_str()?.parse().ok()?;
            Some((
                id,
                entity,
                entity
                    .get("construction")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            ))
        })
        .collect();
    lines.sort_by_key(|(_, _, construction)| !construction);
    lines
        .into_iter()
        .enumerate()
        .map(|(i, (id, entity, construction))| {
            (
                id,
                axis_label(entity, &params.entities, construction, i + 1),
            )
        })
        .collect()
}

fn axis_label(
    entity: &serde_json::Value,
    entities: &[serde_json::Value],
    construction: bool,
    index: usize,
) -> String {
    let kind = if construction {
        "Construction"
    } else {
        "Profile"
    };
    let name = format!("{kind} line {index}");
    match (
        endpoint(entity, entities, "p0"),
        endpoint(entity, entities, "p1"),
    ) {
        (Some([x0, y0]), Some([x1, y1])) => {
            format!("{name}: ({x0:.3}, {y0:.3}) to ({x1:.3}, {y1:.3}) mm")
        }
        _ => format!("{name}: coordinates unavailable"),
    }
}

fn endpoint(
    entity: &serde_json::Value,
    entities: &[serde_json::Value],
    key: &str,
) -> Option<[f64; 2]> {
    let point = if let Some(reference) = entity.get(format!("{key}Ref")).and_then(|id| id.as_str())
    {
        entities
            .iter()
            .find(|point| point.get("id").and_then(|id| id.as_str()) == Some(reference))?
            .get("at")?
    } else {
        entity.get(key)?
    };
    let values = point.as_array()?;
    let [x, y] = [values.first()?.as_f64()?, values.get(1)?.as_f64()?];
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

pub(crate) fn ready(app: &App) -> bool {
    app.doc
        .interaction
        .selection
        .as_ref()
        .is_some_and(|p| !lines(app, p.profile.sketch).is_empty())
}

pub(crate) fn begin(app: &mut App) {
    let Some(profile) = app.doc.interaction.selection.clone() else {
        return;
    };
    let Some((line, _)) = lines(app, profile.profile.sketch).into_iter().next() else {
        return;
    };
    let axis = AxisRef::SketchLine {
        sketch: profile.profile.sketch,
        line,
        extra: Default::default(),
    };
    let step = app
        .doc
        .display
        .as_ref()
        .map_or(0, |d| d.projection.applied_ops as u32);
    let record = modeling_records::revolve_axis(profile.profile, axis, RecordId::new(), step);
    app.doc.gesture += 1;
    match operand_binding::begin_bound(record, app.doc.gesture, profile.context) {
        Ok(action) => app.dispatch(action),
        Err(error) => app.doc.status = error,
    }
    app.draw_dirty = true;
}

pub(crate) fn rows(
    app: &mut App,
    rows: &mut FeatureCardRows,
    tool: &ModelSession,
    p: &RevolveParams,
) {
    let Some(AxisRef::SketchLine { sketch, line, .. }) = &p.axis else {
        return;
    };
    let lines = lines(app, *sketch);
    let mut selected = *line;
    rows.row("Axis", |ui| {
        let response = egui::ComboBox::from_id_salt("revolve-axis")
            .width(ui.available_width())
            .wrap()
            .selected_text(
                lines
                    .iter()
                    .find(|(id, _)| id == line)
                    .map_or("Sketch line", |(_, label)| label),
            )
            .show_ui(ui, |ui| {
                for (id, label) in lines {
                    ui.selectable_value(&mut selected, id, label);
                }
            })
            .response;
        crate::automation::name(ui.ctx(), response.id, "onecad.option.revolveAxis");
    });
    rows.caption("Axis endpoints in sketch coordinates (mm)");
    if selected != *line {
        app.dispatch(Action::ModelingOptionsChanged {
            tag: tool.tag.clone(),
            options: ModelingOptions::RevolveAxis(Box::new(AxisRef::SketchLine {
                sketch: *sketch,
                line: selected,
                extra: Default::default(),
            })),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn axis_names_include_type_and_resolved_endpoint_coordinates() {
        let points = [
            json!({"id": "start", "type": "Point", "at": [1.25, -2.5]}),
            json!({"id": "end", "type": "Point", "at": [1.25, 40.0]}),
        ];
        let line = json!({"type": "Line", "p0Ref": "start", "p1Ref": "end"});
        assert_eq!(
            axis_label(&line, &points, true, 1),
            "Construction line 1: (1.250, -2.500) to (1.250, 40.000) mm"
        );
        let legacy = json!({"type": "Line", "p0": [0, 0], "p1": [20, 0]});
        assert_eq!(
            axis_label(&legacy, &[], false, 2),
            "Profile line 2: (0.000, 0.000) to (20.000, 0.000) mm"
        );
    }

    #[test]
    fn missing_axis_points_are_reported_without_raw_ids() {
        let line = json!({"type": "Line", "p0Ref": "missing", "p1Ref": "missing"});
        assert_eq!(
            axis_label(&line, &[], false, 2),
            "Profile line 2: coordinates unavailable"
        );
    }
}
