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
                entity
                    .get("construction")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            ))
        })
        .collect();
    lines.sort_by_key(|(_, construction)| !construction);
    lines
        .into_iter()
        .enumerate()
        .map(|(i, (id, construction))| {
            (
                id,
                format!(
                    "{} line {}",
                    if construction {
                        "Construction"
                    } else {
                        "Profile"
                    },
                    i + 1
                ),
            )
        })
        .collect()
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
