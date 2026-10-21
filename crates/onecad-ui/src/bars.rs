//! Title bar (Home, history) and native window title.
use crate::{app::App, bridge::Command};
use egui::{Context, Ui};
use onecad_widgets::components::shell::{TabSpec, TabStrip, TitleBar};

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
            // File actions live in the native menu; documents are tabs.
            let summaries = app.tab_summaries();
            let specs: Vec<TabSpec> = summaries
                .iter()
                .filter(|tab| tab.open)
                .map(|tab| TabSpec {
                    id: tab.tab,
                    title: &tab.title,
                    icon: "body",
                    dirty: tab.dirty,
                    active: tab.active,
                    busy: tab.busy,
                })
                .collect();
            let out = TitleBar::new("")
                .tabs(Some(TabStrip {
                    tabs: &specs,
                    home_active: app.ui.start_open,
                    can_add: home_allowed(app),
                }))
                .can_undo(can_undo)
                .can_redo(can_redo)
                .home_blocked((!home_allowed(app)).then_some("Finish the current command first"))
                .search_blocked(Some("Command search is not available yet"))
                .show(ui);
            app.tab_rects = out.tab_rects.clone();
            if (out.home || out.add) && home_allowed(app) {
                app.open_home();
            }
            if let Some(tab) = out.activate
                && !app.busy()
            {
                app.show_tab(tab);
            }
            if let Some(tab) = out.close {
                app.close_tab_from_strip(tab);
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
    let name = if app.doc.document_state.is_some() {
        app.doc.editor_title()
    } else {
        "OneCAD".to_owned()
    };
    let title = if app.document_dirty() {
        format!("{name} — Edited")
    } else {
        name
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
        || app.doc.interaction.tool.is_some()
        || app.busy()
        || app.doc.transform_gizmo.gizmo.active())
}

pub(crate) fn replacement_allowed(app: &App) -> bool {
    !app.busy() && !app.sketch_active() && app.doc.interaction.tool.is_none()
}

pub(crate) fn save_allowed(app: &App) -> bool {
    !app.busy() && (!app.sketch_active() || recovery_sketch_save(app)) && app.document_open()
}

pub(crate) fn recovery_sketch_save(app: &App) -> bool {
    !app.doc.bridge.valid()
        && !app.doc.sketch_gesture.busy()
        && app.doc.sketch.session.as_ref().is_some_and(|session| {
            session.phase == onecad_interaction::sketch::SketchPhase::NeedsRefresh
                && session.accepted_capture.is_none()
                && app
                    .doc
                    .document_state
                    .as_ref()
                    .is_some_and(|state| state.runtime_session == session.key.runtime_session)
        })
}

pub(crate) fn export_allowed(app: &App) -> bool {
    replacement_allowed(app) && app.doc.bridge.valid() && app.doc.display.is_some()
}
