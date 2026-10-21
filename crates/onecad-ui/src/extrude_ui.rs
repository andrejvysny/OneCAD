//! New extrusion mode and target controls share the distance candidate's correlation.
use crate::app::App;
use onecad_core::{
    document::{
        body::BodyHealth,
        record::{BooleanMode, KnownOperation, Operation},
    },
    ids::BodyId,
};
use onecad_interaction::{Action, ModelSession, SessionPhase};
use onecad_widgets::components::overlays::FeatureCardRows;

pub(crate) fn params(tool: &ModelSession) -> Option<&onecad_core::document::record::ExtrudeParams> {
    if tool.editing_existing || tool.phase != SessionPhase::Editing {
        return None;
    }
    match &tool.record.op {
        Operation::Known(KnownOperation::Extrude(params)) => Some(params),
        _ => None,
    }
}

pub(crate) fn choosing_target(app: &App) -> bool {
    app.doc
        .interaction
        .tool
        .as_ref()
        .and_then(params)
        .is_some_and(|params| params.boolean_mode != BooleanMode::NewBody)
}

fn targets(app: &App) -> Vec<(BodyId, String)> {
    let Some(display) = &app.doc.display else {
        return Vec::new();
    };
    display
        .projection
        .bodies
        .values()
        .filter_map(|body| {
            let id = body.id.parse().ok()?;
            (body.visible
                && body.health == BodyHealth::Healthy
                && display
                    .scene
                    .bodies()
                    .iter()
                    .any(|body| body.body_id() == id))
            .then(|| (id, body.name.clone()))
        })
        .collect()
}

/// An explicit tree or viewport body pick may replace only the target, never the profile.
pub(crate) fn select_target(app: &mut App, body: BodyId) -> bool {
    if !choosing_target(app) {
        return false;
    }
    if targets(app).iter().any(|(id, _)| *id == body)
        && let Some(context) = app.doc.interaction.context.clone()
    {
        app.dispatch(Action::ExtrudeTargetChanged {
            body: Some(body),
            context,
        });
        app.doc.selected_body = Some(body);
        app.draw_dirty = true;
    }
    true
}

pub(crate) fn rows(app: &mut App, rows: &mut FeatureCardRows, tool: &ModelSession) {
    let Some(params) = params(tool) else {
        return;
    };
    let mut mode = params.boolean_mode;
    rows.row("Operation", |ui| {
        egui::ComboBox::from_id_salt("extrude-mode")
            .selected_text(mode_label(mode))
            .show_ui(ui, |ui| {
                for value in [BooleanMode::NewBody, BooleanMode::Add, BooleanMode::Cut] {
                    ui.selectable_value(&mut mode, value, mode_label(value));
                }
            });
    });
    if mode != params.boolean_mode {
        app.dispatch(Action::ExtrudeModeChanged(mode));
        if mode == BooleanMode::NewBody {
            app.doc.selected_body = None;
            app.draw_dirty = true;
        }
        return;
    }
    if mode != BooleanMode::NewBody {
        target_row(app, rows, params.target_body);
    }
    rows.row("Direction", |ui| {
        let direction = match &tool.draft_status {
            onecad_interaction::DraftStatus::Valid(value) if value.value < 0.0 => "Reverse",
            onecad_interaction::DraftStatus::Valid(_) => "Forward",
            _ => "Evaluating",
        };
        ui.label(direction);
        if ui.button("Flip").clicked() {
            app.dispatch(Action::FlipExtrude);
        }
    });
}

fn target_row(app: &mut App, rows: &mut FeatureCardRows, target: Option<BodyId>) {
    let targets = targets(app);
    let mut selected = target;
    rows.row("Target body", |ui| {
        let name = targets
            .iter()
            .find(|(id, _)| Some(*id) == target)
            .map_or("Select body", |(_, name)| name.as_str());
        egui::ComboBox::from_id_salt("extrude-target")
            .selected_text(name)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, None, "Select body");
                for (id, name) in &targets {
                    ui.selectable_value(&mut selected, Some(*id), name)
                        .on_hover_text(id.to_string());
                }
            });
        if let Some(id) = target {
            ui.add(egui::Label::new(egui::RichText::new(id.to_string()).small()).wrap());
        } else {
            ui.label("Choose body here or in the tree");
        }
    });
    if selected != target
        && let Some(context) = app.doc.interaction.context.clone()
    {
        app.dispatch(Action::ExtrudeTargetChanged {
            body: selected,
            context,
        });
        app.doc.selected_body = selected;
        app.draw_dirty = true;
    }
}

fn mode_label(mode: BooleanMode) -> &'static str {
    match mode {
        BooleanMode::NewBody => "New Body",
        BooleanMode::Add => "Add",
        BooleanMode::Cut => "Cut",
        BooleanMode::Intersect => "Intersect",
    }
}
