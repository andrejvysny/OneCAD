//! Only applicable typed scalar fields are exposed; authored sibling inputs remain intact.
use crate::app::App;
use onecad_interaction::{Action, ModelSession, SessionPhase, numeric_parameters};

pub fn selector(app: &mut App, ui: &mut egui::Ui, tool: &ModelSession) {
    if !tool.editing_existing || tool.phase != SessionPhase::Editing {
        return;
    }
    let Some(display) = app.display.clone() else {
        return;
    };
    let Some(record) = display
        .records
        .iter()
        .find(|record| record.record_id == tool.tag.record_id)
    else {
        return;
    };
    let mut parameter = tool.parameter;
    egui::ComboBox::from_id_salt("numeric-parameter")
        .selected_text(parameter.label())
        .show_ui(ui, |ui| {
            for descriptor in numeric_parameters(&record.op) {
                ui.selectable_value(&mut parameter, descriptor.parameter, descriptor.label);
            }
        });
    if parameter != tool.parameter
        && let Some(context) = app.interaction.context.clone()
    {
        app.gesture += 1;
        app.dispatch(Action::BeginParameterEdit {
            record: Box::new(record.clone()),
            parameter,
            gesture: app.gesture,
            context,
        });
    }
}
