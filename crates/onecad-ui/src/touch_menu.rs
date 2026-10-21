//! File and Edit actions for hosts without a native menu bar (iPadOS). The rows
//! reuse the menu-list component and queue the same `menu::Action`s the AppKit
//! menu sends, so routing, overlay ownership and enablement stay shared.
use crate::{app::App, document_flow::ExportFormat, menu::Action};
use onecad_widgets::components::lists::{MenuRow, MenuSection, menu_divider, menu_frame};

type Row = (&'static str, &'static str, Option<&'static str>, Action);

const FILE: &[Row] = &[
    ("plus", "New Part", Some("⌘N"), Action::New),
    ("folder", "Open…", Some("⌘O"), Action::Open),
    ("save", "Save", Some("⌘S"), Action::Save),
    ("save", "Save As…", Some("⌘⇧S"), Action::SaveAs),
    ("close", "Close Tab", Some("⌘W"), Action::CloseTab),
];

const EXCHANGE: &[Row] = &[
    ("import", "Import…", Some("⌘⇧I"), Action::Import),
    ("component", "Insert STEP…", None, Action::InsertStep),
    (
        "export",
        "Export STEP…",
        None,
        Action::Export(ExportFormat::Step),
    ),
];

const EDIT: &[Row] = &[
    ("undo", "Undo", Some("⌘Z"), Action::Undo),
    ("redo", "Redo", Some("⌘⇧Z"), Action::Redo),
];

/// The title bar's File menu, toggled by `button`.
pub fn show(app: &App, button: &egui::Response) {
    let frame = menu_frame(&button.ctx);
    egui::Popup::menu(button)
        .width(240.0)
        .frame(frame)
        .show(|ui| contents(app, ui));
}

fn contents(app: &App, ui: &mut egui::Ui) {
    let Some(menu) = &app.menu else {
        return;
    };
    for (index, (title, rows)) in [("File", FILE), ("Exchange", EXCHANGE), ("Edit", EDIT)]
        .into_iter()
        .enumerate()
    {
        if index > 0 {
            menu_divider(ui);
        }
        ui.add(MenuSection::new(title));
        for &(icon, label, shortcut, action) in rows {
            let row = MenuRow::new(Some(icon), label)
                .shortcut(shortcut)
                .enabled(menu.enabled(action));
            if ui.add(row).clicked() {
                menu.push(action);
                ui.close();
                // The queued action is routed by the next frame's raw input pass.
                ui.ctx().request_repaint();
            }
        }
    }
}
