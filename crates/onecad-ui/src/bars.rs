//! Title bar (workspace switcher, document name, history, File menu).
use crate::{
    app::App,
    bridge::Command,
    document_flow::{ExportFormat, Request},
};
use egui::{Context, Ui};
use onecad_widgets::components::{
    lists::{MenuRow, menu_divider, menu_frame},
    shell::{TitleBar, WorkspaceSpec},
};

pub fn title(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let (can_undo, can_redo) = (
        crate::input::history_available(app, false),
        crate::input::history_available(app, true),
    );
    let name = app
        .document_state
        .as_ref()
        .map_or_else(|| "OneCAD".to_owned(), |state| state.title.clone());
    let dirty = app.document_dirty();
    let workspace = (!app.ui.start_open).then_some(WorkspaceSpec {
        name: "Design",
        icon: "body",
        open: app.ui.workspace_menu_open,
    });
    egui::Panel::top("title")
        .exact_size(44.0)
        .frame(egui::Frame::NONE)
        .resizable(false)
        .show(ui, |ui| {
            let out = TitleBar::new(&name)
                .dirty(dirty)
                .can_undo(can_undo)
                .can_redo(can_redo)
                .workspace(workspace)
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
            file_menu(app, &ctx, &out.file, (can_undo, can_redo));
            if let Some(anchor) = &out.workspace {
                crate::workspace_ui::workspace_menu(app, &ctx, anchor);
            }
        });
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
    !app.busy() && !app.sketch_active() && app.document_open()
}

pub(crate) fn export_allowed(app: &App) -> bool {
    replacement_allowed(app) && app.bridge.valid() && app.display.is_some()
}

fn file_menu(app: &mut App, ctx: &Context, file: &egui::Response, (undo, redo): (bool, bool)) {
    egui::Popup::menu(file).frame(menu_frame(ctx)).show(|ui| {
        ui.set_min_width(180.0);
        document_menu(app, ui);
        menu_divider(ui);
        let row = |icon, label, key, enabled| {
            MenuRow::new(Some(icon), label)
                .shortcut(Some(key))
                .enabled(enabled)
        };
        if ui.add(row("undo", "Undo", "⌘Z", undo)).clicked() {
            crate::input::submit_history(app, Command::Undo);
        }
        if ui.add(row("redo", "Redo", "⇧⌘Z", redo)).clicked() {
            crate::input::submit_history(app, Command::Redo);
        }
        menu_divider(ui);
        ui.add(
            MenuRow::new(Some("search"), "Search commands")
                .shortcut(Some("⌘K"))
                .enabled(false),
        );
    });
}

fn document_menu(app: &mut App, ui: &mut Ui) {
    let replace = replacement_allowed(app);
    for (icon, label, shortcut, request, enabled) in [
        ("body", "New part", "⌘N", Request::New, replace),
        ("folder", "Open…", "⌘O", Request::Open(None), replace),
        ("import", "Import…", "", Request::Import, replace),
        (
            "save",
            "Save",
            "⌘S",
            Request::Save { save_as: false },
            save_allowed(app),
        ),
        (
            "save",
            "Save As…",
            "⇧⌘S",
            Request::Save { save_as: true },
            save_allowed(app),
        ),
    ] {
        if ui
            .add(
                MenuRow::new(Some(icon), label)
                    .shortcut(Some(shortcut))
                    .enabled(enabled),
            )
            .clicked()
        {
            app.document_request(request);
            ui.close();
        }
    }
    ui.menu_button("Export", |ui| {
        for (label, format) in [
            ("STEP…", ExportFormat::Step),
            ("STL…", ExportFormat::Stl),
            ("OBJ…", ExportFormat::Obj),
            ("3MF…", ExportFormat::ThreeMf),
        ] {
            if ui
                .add_enabled(export_allowed(app), egui::Button::new(label))
                .clicked()
            {
                app.document_request(Request::Export(format));
                ui.close();
            }
        }
    });
}
