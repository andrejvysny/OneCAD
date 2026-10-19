//! Inspector History tab (README 07a): feature rows, rollback marker and the info box.
use crate::app::App;
use egui::{Margin, RichText, Ui};
use onecad_interaction::Action;
use onecad_widgets::components::lists::{HistoryRow, HistoryState, RollbackMarker, SectionLabel};

pub fn feature_icon(op_type: &str) -> &'static str {
    match op_type {
        "Extrude" => "extrude",
        "Revolve" => "revolve",
        "Fillet" => "fillet",
        "Chamfer" => "chamfer",
        "Hole" => "hole",
        "Shell" => "shell",
        "Sketch" => "sketch",
        _ => "body",
    }
}

pub fn history(app: &mut App, ui: &mut Ui) {
    let Some(display) = app.display.clone() else {
        ui.label("Opening document…");
        return;
    };
    let count = display.projection.features.len();
    ui.add(SectionLabel::new(&format!("{count} FEATURES")));
    for (index, feature) in display.projection.features.iter().enumerate() {
        let editable = display
            .records
            .iter()
            .any(|record| record.record_id.to_string() == feature.id)
            && app.viewport_ready()
            && app.bridge.valid()
            && !app.busy()
            && !app.sketch_active();
        let state = row_state(app, feature);
        let row = HistoryRow::new(index + 1, feature_icon(&feature.op_type), &feature.label)
            .meta(Some(&feature.value_text))
            .state(state);
        if row.show(ui, None).response.clicked()
            && editable
            && let Some(record) = display
                .records
                .iter()
                .find(|record| record.record_id.to_string() == feature.id)
            && let Some(context) = app.interaction.context.clone()
            && let Some(first) = onecad_interaction::numeric_parameters(&record.op).first()
        {
            app.gesture += 1;
            app.selected_feature = Some(record.record_id);
            app.dispatch(Action::BeginParameterEdit {
                parameter: first.parameter,
                record: Box::new(record.clone()),
                gesture: app.gesture,
                context,
            });
        }
        if let Some(message) = &feature.status_message {
            ui.colored_label(app.theme.traffic_close, message);
        }
    }
    ui.add(RollbackMarker::new());
    ui.add_space(12.0);
    info_box(app, ui);
}

fn info_box(app: &App, ui: &mut Ui) {
    let text = "Changes regenerate everything below. Features that can no longer resolve are flagged here, not deleted.";
    egui::Frame::new()
        .fill(app.theme.well)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(12.0).color(app.theme.ink4));
        });
}

fn row_state(app: &App, feature: &onecad_app::dto::FeatureDto) -> HistoryState {
    let selected = app
        .selected_feature
        .as_ref()
        .map(|id| id.to_string())
        .as_deref()
        == Some(feature.id.as_str());
    if feature.status_message.is_some() {
        HistoryState::Failed
    } else if selected {
        HistoryState::Selected
    } else {
        HistoryState::Default
    }
}
