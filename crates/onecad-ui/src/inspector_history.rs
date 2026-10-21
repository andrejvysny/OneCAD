//! Inspector History tab (README 07a): feature rows, rollback marker and the info box.
use crate::app::App;
use egui::{Margin, RichText, Ui};
use onecad_core::document::record::{KnownOperation, Operation};
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
    let Some(display) = app.doc.display.clone() else {
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
    let selectable =
        app.viewport_ready() && app.doc.bridge.valid() && !app.busy() && !app.sketch_active();
    let state = row_state(app, feature, index >= display.projection.applied_ops);
    let row = HistoryRow::new(index + 1, feature_icon(&feature.op_type), &feature.label)
        .meta(Some(&feature.value_text))
        .state(state);
    let response = row.show(ui, None).response;
    if (response.clicked() || response.double_clicked())
        && selectable
        && let Ok(id) = feature.id.parse()
    {
        crate::pill::select_feature(app, id);
        app.doc.selected_sketch = None;
        app.ui.inspector_open = true;
        if response.double_clicked() {
            edit_selected_feature(app);
        }
    }
    if app
        .doc
        .selected_feature
        .is_some_and(|id| id.to_string() == feature.id)
    {
        ui.label(format!("{} · {}", feature.op_type, feature.value_text));
        edit_button(app, ui);
    }
    crate::feature_card::history_editor(app, ui, &feature.id);
    if let Some(message) = &feature.status_message {
        ui.colored_label(app.theme.traffic_close, message);
    }
}

pub(crate) fn edit_selected_feature(app: &mut App) {
    if !app.viewport_ready() || !app.doc.bridge.valid() || app.busy() || app.sketch_active() {
        return;
    }
    let Some(record) = app.doc.selected_feature.and_then(|id| {
        app.doc
            .display
            .as_ref()?
            .records
            .iter()
            .find(|record| record.record_id == id)
            .cloned()
    }) else {
        return;
    };
    if let Operation::Known(KnownOperation::Sketch(params)) = &record.op {
        app.edit_sketch(params.sketch);
        return;
    }
    let Some(context) = app.doc.interaction.context.clone() else {
        return;
    };
    let Some(first) = onecad_interaction::numeric_parameters(&record.op)
        .first()
        .cloned()
    else {
        return;
    };
    app.ui.inspector_open = true;
    app.ui.inspector_tab = crate::ui_state::InspectorTab::History;
    app.doc.gesture += 1;
    app.dispatch(Action::BeginParameterEdit {
        parameter: first.parameter,
        record: Box::new(record),
        gesture: app.doc.gesture,
        context,
    });
}

pub(crate) fn edit_button(app: &mut App, ui: &mut Ui) {
    if app.doc.selected_feature.is_none()
        || app.doc.interaction.tool.is_some()
        || app.sketch_active()
    {
        return;
    }
    let editable = app.doc.selected_feature.is_some_and(|id| {
        app.doc
            .display
            .as_ref()
            .and_then(|display| display.records.iter().find(|record| record.record_id == id))
            .is_some_and(|record| {
                matches!(record.op, Operation::Known(KnownOperation::Sketch(_)))
                    || !onecad_interaction::numeric_parameters(&record.op).is_empty()
            })
    }) && app.viewport_ready()
        && app.doc.bridge.valid()
        && !app.busy();
    if ui
        .add_enabled(editable, egui::Button::new("Edit feature"))
        .clicked()
    {
        edit_selected_feature(app);
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
        .doc
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
