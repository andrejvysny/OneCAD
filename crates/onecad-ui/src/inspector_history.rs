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
    let cursor = display.projection.applied_ops.min(count);
    egui::ScrollArea::vertical()
        .id_salt("inspector-history")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, feature) in display.projection.features.iter().enumerate() {
                if index == cursor {
                    ui.add_enabled(false, RollbackMarker::new());
                }
                feature_row(app, ui, &display, index, feature);
            }
            if cursor == count {
                ui.add_enabled(false, RollbackMarker::new());
            }
            ui.add_space(12.0);
            info_box(app, ui);
        });
}

fn feature_row(
    app: &mut App,
    ui: &mut Ui,
    display: &crate::bridge::Display,
    index: usize,
    feature: &onecad_app::dto::FeatureDto,
) {
    let editable = display
        .records
        .iter()
        .any(|record| record.record_id.to_string() == feature.id)
        && app.viewport_ready()
        && app.bridge.valid()
        && !app.busy()
        && !app.sketch_active();
    let state = row_state(app, feature, index >= display.projection.applied_ops);
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
        crate::pill::select_feature(app, record.record_id);
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

fn row_state(app: &App, feature: &onecad_app::dto::FeatureDto, rolled_back: bool) -> HistoryState {
    let selected = app
        .selected_feature
        .as_ref()
        .map(|id| id.to_string())
        .as_deref()
        == Some(feature.id.as_str());
    history_state(feature.status, feature.suppressed, rolled_back, selected)
}

fn history_state(
    status: onecad_app::dto::FeatureStatus,
    suppressed: bool,
    rolled_back: bool,
    selected: bool,
) -> HistoryState {
    use onecad_app::dto::FeatureStatus;
    if rolled_back {
        HistoryState::RolledBack
    } else if suppressed {
        HistoryState::Suppressed
    } else if matches!(status, FeatureStatus::Error | FeatureStatus::NeedsRepair) {
        HistoryState::Failed
    } else if selected {
        HistoryState::Selected
    } else {
        HistoryState::Default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_app::dto::FeatureStatus;

    #[test]
    fn history_uses_authoritative_status_and_cursor() {
        assert_eq!(
            history_state(FeatureStatus::Ok, false, false, false),
            HistoryState::Default
        );
        assert_eq!(
            history_state(FeatureStatus::Ok, false, false, true),
            HistoryState::Selected
        );
        for status in [FeatureStatus::Error, FeatureStatus::NeedsRepair] {
            assert_eq!(
                history_state(status, false, false, true),
                HistoryState::Failed
            );
        }
        assert_eq!(
            history_state(FeatureStatus::Dirty, true, false, true),
            HistoryState::Suppressed
        );
        assert_eq!(
            history_state(FeatureStatus::Error, true, true, true),
            HistoryState::RolledBack
        );
    }
}
