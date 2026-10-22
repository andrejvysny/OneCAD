use super::{PresentationContext, Request, State, command_available, export_command, menu_command};
use crate::extension_pages::kit;
use egui::{Color32, RichText, Ui};
use onecad_extension_api::{Capability, ExtensionRuntime};
use onecad_extension_host::{PackageState, PackageSummary, RegisteredCommand};
use onecad_widgets::{fonts, theme::look};

pub(super) fn label(ui: &mut Ui, text: impl Into<String>, size: f32, ink: Color32) {
    ui.add(egui::Label::new(RichText::new(text.into()).size(size).color(ink)).wrap());
}

pub(super) fn heading(ui: &mut Ui, text: &str, size: f32) {
    ui.label(
        RichText::new(text)
            .size(size)
            .family(fonts::semibold())
            .color(look(ui.ctx()).theme.ink),
    );
}

pub(super) fn icon(package: &PackageSummary) -> &'static str {
    if package
        .manifest
        .capabilities
        .contains(&Capability::Export3mf)
    {
        "export"
    } else {
        "extension"
    }
}

pub(super) fn status(package: &PackageSummary) -> &'static str {
    if package.manifest.runtime == ExtensionRuntime::NativeTrusted {
        return "Unavailable";
    }
    match package.state {
        PackageState::Enabled => "Enabled",
        PackageState::Disabled => "Disabled",
        PackageState::Failed(_) => "Stopped",
    }
}

pub(super) fn status_color(package: &PackageSummary, ui: &Ui) -> Color32 {
    let theme = look(ui.ctx()).theme;
    match status(package) {
        "Enabled" => theme.ok_text,
        "Stopped" => theme.danger_strong,
        "Unavailable" => theme.warn,
        _ => theme.ink5,
    }
}

pub(super) fn summary(package: &PackageSummary) -> String {
    let count = package.manifest.contributions.commands.len();
    format!("{count} command{}", if count == 1 { "" } else { "s" })
}

pub(super) fn locations(entry: &onecad_extension_api::CommandContribution) -> String {
    let mut locations = Vec::new();
    if export_command(entry) {
        locations.push("Export menu");
    }
    if menu_command(entry) {
        locations.push("Extensions menu");
    }
    if entry.locations.iter().any(|location| location == "palette") {
        locations.push("Command search");
    }
    locations.push("Extension details");
    locations.join(" · ")
}

pub(super) fn available(entry: &RegisteredCommand, context: PresentationContext) -> bool {
    context.idle
        && entry.enabled
        && command_available(&entry.command, context.document, context.selection)
}

pub(super) fn unavailable_reason(
    entry: &RegisteredCommand,
    context: PresentationContext,
) -> &'static str {
    if !entry.enabled {
        "Enable this extension to use its commands"
    } else if !context.idle {
        "Wait for the current operation to finish"
    } else if entry.command.requires_document && !context.document {
        "Open a ready document and finish the active tool"
    } else {
        "Select bodies in the current document"
    }
}

pub(super) fn notice(ui: &mut Ui, state: &State) {
    let theme = look(ui.ctx()).theme;
    if state.pending || state.status_warning {
        let (fill, border) = if state.pending {
            (theme.sel_soft, theme.sel_border)
        } else {
            (theme.warn_surface, theme.warn_border)
        };
        egui::Frame::new()
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, border))
            .corner_radius(8)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_min_width((ui.available_width() - 2.0).max(0.0));
                ui.horizontal_top(|ui| {
                    if state.pending {
                        ui.add(egui::Spinner::new().size(16.0));
                    }
                    label(
                        ui,
                        if state.pending {
                            pending_label(state).to_owned()
                        } else {
                            state.status.clone()
                        },
                        12.5,
                        theme.ink3,
                    );
                });
            });
        ui.add_space(12.0);
    } else if !state.status.is_empty() {
        label(ui, &state.status, 12.0, theme.ink5);
        ui.add_space(10.0);
    }
    for error in &state.discovery_errors {
        label(ui, error, 12.0, theme.danger_strong);
    }
}

fn pending_label(state: &State) -> &'static str {
    match state.pending_request.as_ref() {
        Some(Request::InstallFolder) => "Installing local package…",
        Some(Request::UpdateFolder { .. }) => "Updating local package…",
        Some(Request::SetEnabled { enabled: true, .. }) => {
            "Checking permissions and enabling extension…"
        }
        Some(Request::SetEnabled { enabled: false, .. }) => "Disabling extension…",
        Some(Request::Uninstall { .. }) => "Uninstalling extension…",
        Some(Request::SetApplication { .. }) => "Saving application settings…",
        Some(Request::ResetApplications) => "Backing up and resetting application settings…",
        Some(Request::Execute { .. }) => "Running extension command…",
        _ => "Checking installed packages…",
    }
}

pub(super) fn empty(ui: &mut Ui, icon: &str, title: &str, message: &str) {
    let theme = look(ui.ctx()).theme;
    egui::Frame::new()
        .fill(theme.panel)
        .stroke(egui::Stroke::new(1.0, theme.border))
        .corner_radius(10)
        .inner_margin(24)
        .show(ui, |ui| {
            ui.set_min_width((ui.available_width() - 2.0).max(0.0));
            let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
            kit::well(
                ui.painter(),
                rect,
                icon,
                10,
                24.0,
                &theme,
                look(ui.ctx()).dark,
            );
            ui.add_space(12.0);
            heading(ui, title, 14.0);
            label(ui, message, 12.5, theme.ink5);
        });
}

pub(super) fn permission(ui: &mut Ui, capability: Capability) {
    let (icon, title, reason) = match capability {
        Capability::Export3mf => (
            "export",
            "Create export files",
            "Export your document or selected bodies as a local 3MF file.",
        ),
        Capability::LaunchApplication => (
            "workspace",
            "Open desktop applications",
            "Open declared applications with the exported file.",
        ),
    };
    let theme = look(ui.ctx()).theme;
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
        kit::well(
            ui.painter(),
            rect,
            icon,
            6,
            15.0,
            &theme,
            look(ui.ctx()).dark,
        );
        ui.vertical(|ui| {
            heading(ui, title, 13.0);
            label(ui, reason, 12.0, theme.ink5);
        });
    });
    ui.add_space(8.0);
}
