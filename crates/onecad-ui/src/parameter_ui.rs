//! Re-edit field selector: switching moves focus inside one session, so edits
//! to several fields accumulate into one Apply and one Undo.
use crate::app::App;
use onecad_interaction::{Action, ModelSession, SessionPhase};

pub fn selector(app: &mut App, ui: &mut egui::Ui, tool: &ModelSession) {
    if !tool.editing_existing || tool.phase != SessionPhase::Editing {
        return;
    }
    let Some(focus) = tool.params.focus() else {
        return;
    };
    let mut selected = focus;
    egui::ComboBox::from_id_salt("numeric-parameter")
        .selected_text(focus.label())
        .show_ui(ui, |ui| {
            for field in tool.params.active() {
                ui.selectable_value(&mut selected, field.key, field.key.label());
            }
        });
    if selected != focus {
        app.dispatch(Action::FocusParameter(selected));
    }
}
