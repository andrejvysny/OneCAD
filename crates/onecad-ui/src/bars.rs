//! Title bar (Home, history) and native window title.
use crate::{app::App, bridge::Command};
use egui::{Context, Ui};
use onecad_widgets::components::shell::TitleBar;

pub fn title(app: &mut App, ui: &mut Ui) {
    let (can_undo, can_redo) = (
        crate::input::history_available(app, false),
        crate::input::history_available(app, true),
    );
    sync_window_title(app, ui.ctx());
    egui::Panel::top("title")
        .exact_size(44.0)
        .frame(egui::Frame::NONE)
        .resizable(false)
        .show(ui, |ui| {
            // Document name and file actions live in the native window title and menu.
            let out = TitleBar::new("")
                .bare(true)
                .can_undo(can_undo)
                .can_redo(can_redo)
                .home_blocked((!home_allowed(app)).then_some("Finish the current command first"))
                .show(ui);
            if out.home && home_allowed(app) {
                app.ui.start_open = true;
            }
            if out.undo {
                crate::input::submit_history(app, Command::Undo);
            }
            if out.redo {
                crate::input::submit_history(app, Command::Redo);
            }
        });
}

/// Native window title: document name, " — Edited" when dirty; sent only on change.
fn sync_window_title(app: &App, ctx: &Context) {
    let name = app
        .document_state
        .as_ref()
        .map_or("OneCAD", |state| state.title.as_str());
    let title = if app.document_dirty() {
        format!("{name} — Edited")
    } else {
        name.to_owned()
    };
    let id = egui::Id::unique("native-window-title");
    let last: Option<String> = ctx.data(|d| d.get_temp(id));
    if last.as_deref() != Some(title.as_str()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
        ctx.data_mut(|d| d.insert_temp(id, title));
    }
}

pub(crate) fn home_allowed(app: &App) -> bool {
    !(app.sketch_active()
        || app.interaction.tool.is_some()
        || app.busy()
        || app.transform_gizmo.gizmo.active())
}

pub(crate) fn replacement_allowed(app: &App) -> bool {
    !app.busy() && !app.sketch_active() && app.interaction.tool.is_none()
}

pub(crate) fn save_allowed(app: &App) -> bool {
    !app.busy() && (!app.sketch_active() || recovery_sketch_save(app)) && app.document_open()
}

pub(crate) fn recovery_sketch_save(app: &App) -> bool {
    !app.bridge.valid()
        && !app.sketch_gesture.busy()
        && app.sketch.session.as_ref().is_some_and(|session| {
            session.phase == onecad_interaction::sketch::SketchPhase::NeedsRefresh
                && session.accepted_capture.is_none()
                && app
                    .document_state
                    .as_ref()
                    .is_some_and(|state| state.runtime_session == session.key.runtime_session)
        })
}

pub(crate) fn export_allowed(app: &App) -> bool {
    replacement_allowed(app) && app.bridge.valid() && app.display.is_some()
}
