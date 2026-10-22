use super::{Request, State, application_key, common};
use crate::extension_pages::kit;
use egui::{Context, RichText, Ui, vec2};
use onecad_extension_api::ExtensionRuntime;
use onecad_extension_host::{Approval, PackageSummary};
use onecad_widgets::{
    components::button::{Button, ButtonKind},
    fonts,
    metrics::{Elevation, shadow},
    theme::look,
};

pub(super) fn show(ctx: &Context, state: &mut State, idle: bool) -> Option<Request> {
    if state.reset_confirmation {
        return reset(ctx, state, idle);
    }
    let package = state.review.clone()?;
    let look = look(ctx);
    let frame = egui::Frame::new()
        .fill(look.theme.surface)
        .stroke(egui::Stroke::new(1.0, look.theme.border))
        .corner_radius(12)
        .inner_margin(20)
        .shadow(shadow(Elevation::Card, &look.theme, look.dark));
    let mut result = None;
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("extension-permissions"))
        .frame(frame)
        .backdrop_color(egui::Color32::from_black_alpha(90))
        .show(ctx, |ui| {
            ui.set_width(400.0_f32.min((ctx.content_rect().width() - 60.0).max(260.0)));
            header(ui, &package);
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 230.0).max(160.0))
                .show(ui, |ui| body(ui, state, &package));
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(Button::new("Cancel")).clicked() {
                    close = true;
                }
                let portable = package.manifest.runtime == ExtensionRuntime::PortableComponent;
                if ui
                    .add_enabled(
                        idle && portable,
                        Button::new("Allow & enable").kind(ButtonKind::Primary),
                    )
                    .clicked()
                {
                    result = Some(Request::SetEnabled {
                        id: package.manifest.id.clone(),
                        enabled: true,
                        approval: Some(Approval {
                            fingerprint: package.fingerprint.clone(),
                            capabilities: package.manifest.capabilities.clone(),
                            applications: package
                                .manifest
                                .applications
                                .iter()
                                .map(|app| app.id.clone())
                                .collect(),
                            trust_native: false,
                        }),
                    });
                    close = true;
                }
            });
        });
    if close || modal.should_close() {
        state.review = None;
    }
    result
}

fn header(ui: &mut Ui, package: &PackageSummary) {
    let theme = look(ui.ctx()).theme;
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(48.0, 48.0), egui::Sense::hover());
        kit::well(
            ui.painter(),
            rect,
            common::icon(package),
            10,
            28.0,
            &theme,
            look(ui.ctx()).dark,
        );
        ui.vertical(|ui| {
            common::heading(ui, &format!("Enable {}?", package.manifest.name), 15.0);
            common::label(
                ui,
                format!("Local package · {}", package.manifest.version),
                12.5,
                theme.ink5,
            );
        });
    });
    ui.add_space(18.0);
}

fn body(ui: &mut Ui, state: &State, package: &PackageSummary) {
    let theme = look(ui.ctx()).theme;
    kit::caption(ui, "Adds");
    for command in &package.manifest.contributions.commands {
        ui.add_space(6.0);
        common::heading(ui, &command.title, 13.0);
        common::label(ui, common::locations(command), 12.0, theme.ink5);
    }
    ui.add_space(18.0);
    kit::caption(ui, "Can");
    ui.add_space(10.0);
    for capability in &package.manifest.capabilities {
        common::permission(ui, *capability);
    }
    for application in &package.manifest.applications {
        common::heading(ui, &application.name, 12.5);
        if let Some(path) = state
            .applications
            .get(&application_key(&package.manifest.id, &application.id))
        {
            common::label(
                ui,
                format!("Application: {}", path.display()),
                12.0,
                theme.ink5,
            );
        } else if let Some(bundle) = &application.macos_bundle_name {
            common::label(
                ui,
                format!("macOS application to discover: {bundle}"),
                12.0,
                theme.ink5,
            );
        } else {
            common::label(
                ui,
                "Choose an application path in Settings before running this command.",
                12.0,
                theme.ink5,
            );
        }
    }
    if package.manifest.runtime == ExtensionRuntime::NativeTrusted {
        common::label(
            ui,
            "This package uses native code. This version of OneCAD cannot run native extensions.",
            12.5,
            theme.warn,
        );
    }
    ui.add_space(12.0);
    egui::Frame::new()
        .fill(theme.warn_surface)
        .corner_radius(8)
        .inner_margin(10)
        .show(ui, |ui| {
            common::label(
                ui,
                "Publisher identity has not been verified for this local package.",
                12.0,
                theme.warn,
            )
        });
    egui::CollapsingHeader::new("Package details").show(ui, |ui| {
        common::label(ui, &package.manifest.id, 11.5, theme.ink5);
        ui.add(
            egui::Label::new(
                RichText::new(&package.fingerprint)
                    .family(fonts::mono())
                    .size(11.0),
            )
            .wrap(),
        );
    });
}

fn reset(ctx: &Context, state: &mut State, idle: bool) -> Option<Request> {
    let mut action = None;
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("extension-reset-applications")).show(ctx, |ui| {
        ui.set_max_width(400.0);
        common::heading(ui, "Reset application paths?", 15.0);
        common::label(ui, "Clear saved application paths for all extensions in this profile. A backup of the existing settings file will be kept. Installed packages and permission approvals are retained.", 12.5, look(ctx).theme.ink3);
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.add(Button::new("Cancel")).clicked() { close = true; }
            if ui.add_enabled(idle, Button::new("Back up & reset").kind(ButtonKind::Primary)).clicked() { action = Some(Request::ResetApplications); close = true; }
        });
    });
    if close || modal.should_close() {
        state.reset_confirmation = false;
    }
    action
}
