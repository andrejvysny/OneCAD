use super::{Request, command_available, export_command, menu_command};
use crate::{app::App, ui_state::StartPage};
use egui::Ui;

/// A portable export contribution surface, also reachable on platforms without native menus.
pub(crate) fn show(app: &mut App, ui: &mut Ui) {
    super::palette::show(app, ui.ctx());
    if !app.ui.extensions.native.attempted && !app.busy() {
        app.extension_request(Request::Refresh);
    }
    let commands = app.ui.extensions.native.commands.clone();
    let document = crate::bars::export_allowed(app);
    let selection = app
        .doc
        .selection
        .items()
        .iter()
        .any(|item| item.body().is_some());
    let mut picked = None;
    let mut export = None;
    let mut manage = false;
    ui.menu_button("Export", |ui| {
        use crate::document_flow::ExportFormat;
        for (title, format) in [
            ("STEP…", ExportFormat::Step),
            ("STL…", ExportFormat::Stl),
            ("OBJ…", ExportFormat::Obj),
            ("3MF…", ExportFormat::ThreeMf),
        ] {
            if ui.add_enabled(document, egui::Button::new(title)).clicked() {
                export = Some(format);
                ui.close();
            }
        }
        ui.separator();
        for entry in commands
            .iter()
            .filter(|entry| export_command(&entry.command))
        {
            let available = entry.enabled
                && !app.busy()
                && !app.ui.extensions.native.pending
                && command_available(&entry.command, document, selection);
            if ui
                .add_enabled(available, egui::Button::new(&entry.command.title))
                .clicked()
            {
                picked = Some(entry.command.id.clone());
                ui.close();
            }
        }
        ui.separator();
        if commands.iter().any(|entry| menu_command(&entry.command)) {
            ui.menu_button("Extension commands", |ui| {
                for entry in commands.iter().filter(|entry| menu_command(&entry.command)) {
                    let enabled = entry.enabled
                        && !app.busy()
                        && !app.ui.extensions.native.pending
                        && command_available(&entry.command, document, selection);
                    if ui
                        .add_enabled(enabled, egui::Button::new(&entry.command.title))
                        .clicked()
                    {
                        picked = Some(entry.command.id.clone());
                        ui.close();
                    }
                }
            });
        }
        if ui.button("Extension commands…").clicked() {
            app.ui.extensions.native.open_palette();
            ui.close();
        }
        if ui
            .add_enabled(
                crate::bars::home_allowed(app),
                egui::Button::new("Manage extensions…"),
            )
            .clicked()
        {
            manage = true;
            ui.close();
        }
    });
    if manage {
        app.open_home();
        app.ui.start_page = StartPage::Extensions;
    }
    if let Some(command) = picked {
        app.run_extension_command(&command);
    }
    if let Some(format) = export {
        app.document_request(crate::document_flow::Request::Export(format));
    }
}
