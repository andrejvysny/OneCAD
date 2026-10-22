//! Title bar (Home, history) and native window title.
use crate::{app::App, bridge::Command};
use egui::{Context, Ui};
use onecad_widgets::components::shell::{TabSpec, TabStrip, TitleBar};

pub fn title(app: &mut App, ui: &mut Ui) {
    app.sync_extension_menu();
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
            // File actions live in the native menu (or, without one, the title bar's
            // File menu); documents are tabs.
            let area = ui.available_rect_before_wrap();
            let mut title = area;
            title.max.x -= 90.0;
            let theme = onecad_widgets::theme::look(ui.ctx()).theme;
            ui.painter().rect_filled(area, 0.0, theme.titlebar);
            ui.painter().hline(
                area.x_range(),
                area.bottom() - 0.5,
                egui::Stroke::new(1.0, theme.border),
            );
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
            let out = ui
                .scope_builder(egui::UiBuilder::new().max_rect(title), |ui| {
                    let bar = TitleBar::new("");
                    // No traffic lights or movable window without the desktop host.
                    #[cfg(not(feature = "desktop"))]
                    let bar = bar.leading_inset(0.0).drag_window(false).tab_file(true);
                    bar.tabs(Some(TabStrip {
                        tabs: &specs,
                        home_active: app.ui.start_open,
                        can_add: home_allowed(app),
                    }))
                    .can_undo(can_undo)
                    .can_redo(can_redo)
                    .home_blocked(
                        (!home_allowed(app)).then_some("Finish the current command first"),
                    )
                    .search_blocked(None)
                    .show(ui)
                })
                .inner;
            app.tab_rects = out.tab_rects.clone();
            name_tabs(ui.ctx(), &out);
            #[cfg(not(feature = "desktop"))]
            if let Some(file) = &out.file {
                crate::touch_menu::show(app, file);
            }
            if out.search {
                app.ui.extensions.native.open_palette();
            }
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
            let menu = egui::Rect::from_min_size(
                egui::pos2(title.right() + 8.0, area.top() + 8.0),
                egui::vec2(74.0, 28.0),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(menu), |ui| {
                crate::native_extensions::menu(app, ui)
            });
        });
}

/// Document tabs are 0-based in strip order; "+" opens a new document.
pub(crate) fn name_tabs(ctx: &Context, out: &onecad_widgets::components::shell::TitleBarOutput) {
    for (index, (_, id)) in out.tab_ids.iter().enumerate() {
        crate::automation::name(ctx, *id, &crate::automation::tab(index));
        ctx.accesskit_node_builder(*id, |node| node.set_role(egui::accesskit::Role::Tab));
    }
    if let Some(id) = out.add_id {
        crate::automation::name(ctx, id, crate::automation::TAB_NEW);
    }
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

/// Insert STEP mutates the active document, so it needs one and an idle editor.
pub(crate) fn insert_allowed(app: &App) -> bool {
    replacement_allowed(app) && app.document_open()
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
