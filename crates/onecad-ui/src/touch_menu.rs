//! File and Edit actions for hosts without a native menu bar (iPadOS). The rows
//! reuse the menu-list component and queue the same `menu::Action`s the AppKit
//! menu sends, so routing, overlay ownership and enablement stay shared. Each
//! control carries a stable AccessKit author id (`onecad.menu.*`), which the
//! iPadOS bridge exports as its automation identifier.
use crate::{app::App, document_flow::ExportFormat, menu::Action};
use onecad_widgets::components::lists::{MenuRow, MenuSection, menu_divider, menu_frame};

/// Icon, label, shortcut, action and automation id.
type Row = (
    &'static str,
    &'static str,
    Option<&'static str>,
    Action,
    &'static str,
);

const FILE: &[Row] = &[
    (
        "plus",
        "New Part",
        Some("⌘N"),
        Action::New,
        "onecad.menu.file.new",
    ),
    (
        "folder",
        "Open…",
        Some("⌘O"),
        Action::Open,
        "onecad.menu.file.open",
    ),
    (
        "save",
        "Save",
        Some("⌘S"),
        Action::Save,
        "onecad.menu.file.save",
    ),
    (
        "save",
        "Save As…",
        Some("⌘⇧S"),
        Action::SaveAs,
        "onecad.menu.file.saveAs",
    ),
    (
        "save",
        "Save All",
        None,
        Action::SaveAll,
        "onecad.menu.file.saveAll",
    ),
    (
        "close",
        "Close Tab",
        Some("⌘W"),
        Action::CloseTab,
        "onecad.menu.file.closeTab",
    ),
];

const EXCHANGE: &[Row] = &[
    (
        "import",
        "Import…",
        Some("⌘⇧I"),
        Action::Import,
        "onecad.menu.file.import",
    ),
    (
        "component",
        "Insert STEP…",
        None,
        Action::InsertStep,
        "onecad.menu.file.insertStep",
    ),
    (
        "export",
        "Export STEP…",
        None,
        Action::Export(ExportFormat::Step),
        "onecad.menu.file.export",
    ),
    (
        "export",
        "Send a Copy…",
        None,
        Action::ExportDocument,
        "onecad.menu.file.sendCopy",
    ),
];

const EDIT: &[Row] = &[
    (
        "undo",
        "Undo",
        Some("⌘Z"),
        Action::Undo,
        "onecad.menu.edit.undo",
    ),
    (
        "redo",
        "Redo",
        Some("⌘⇧Z"),
        Action::Redo,
        "onecad.menu.edit.redo",
    ),
];

/// Names a widget for automation: AccessKit's author id is not localized and
/// does not change with the label.
fn automation_id(ctx: &egui::Context, id: egui::Id, name: &str) {
    ctx.accesskit_node_builder(id, |node| node.set_author_id(name));
}

/// The title bar's File menu, toggled by `button`.
pub fn show(app: &App, button: &egui::Response) {
    automation_id(&button.ctx, button.id, "onecad.menu.file");
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
        for &(icon, label, shortcut, action, automation) in rows {
            let row = MenuRow::new(Some(icon), label)
                .shortcut(shortcut)
                .enabled(menu.enabled(action));
            let response = ui.add(row);
            automation_id(ui.ctx(), response.id, automation);
            if response.clicked() {
                menu.push(action);
                ui.close();
                // The queued action is routed by the next frame's raw input pass.
                ui.ctx().request_repaint();
            }
        }
    }
}

/// Send a Copy…: the saved `.onecad` goes to the host's export picker, which
/// writes a copy where the user chooses (managed-copy policy: a Files location
/// never becomes the document's path). Only saved bytes are sent, so a document
/// with unsaved changes is refused rather than sent stale.
pub(crate) fn send_copy(app: &mut App) {
    let path = app
        .doc
        .document_state
        .as_ref()
        .and_then(|state| state.path.clone());
    let message = match path {
        None => "Save the document before sending a copy".to_owned(),
        Some(_) if app.document_dirty() => "Save your changes before sending a copy".to_owned(),
        Some(path) => {
            app.exported.push(std::path::PathBuf::from(path));
            "Choose where to send a copy".to_owned()
        }
    };
    app.doc.command_status = true;
    app.doc.status = message;
}
