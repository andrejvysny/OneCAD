use super::{DetailTab, PresentationContext, Request, State, ViewAction, application_key, common};
use crate::{extension_pages::kit, ui_state::StartPage};
use egui::{Align, Layout, RichText, Sense, Ui, vec2};
use onecad_extension_api::ExtensionRuntime;
use onecad_extension_host::{PackageState, PackageSummary};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        inputs::Toggle,
        lists::{MenuRow, Tab, TabBar, menu_frame},
    },
    fonts,
    theme::look,
};

pub(super) fn show(
    ui: &mut Ui,
    state: &mut State,
    context: PresentationContext,
    action: &mut ViewAction,
) {
    let package = state
        .packages
        .iter()
        .find(|package| Some(&package.manifest.id) == state.detail.as_ref())
        .cloned();
    let Some(package) = package else {
        action.page = Some(StartPage::Extensions);
        super::manager::show(ui, state, context, action);
        return;
    };
    breadcrumb(ui, &package, action);
    ui.add_space(16.0);
    header(ui, state, &package, context, action);
    ui.add_space(18.0);
    let mut selected = match state.detail_tab {
        DetailTab::Overview => 0,
        DetailTab::Settings => 1,
        DetailTab::Changelog => 2,
    };
    ui.add(TabBar::new(
        "native-extension-detail-tabs",
        &mut selected,
        &[
            Tab {
                label: "Overview",
                count: None,
            },
            Tab {
                label: "Settings",
                count: None,
            },
            Tab {
                label: "Changelog",
                count: None,
            },
        ],
    ));
    state.detail_tab = [
        DetailTab::Overview,
        DetailTab::Settings,
        DetailTab::Changelog,
    ][selected];
    ui.add_space(18.0);
    common::notice(ui, state);
    match state.detail_tab {
        DetailTab::Overview => overview(ui, state, &package, context, action),
        DetailTab::Settings => settings(ui, state, &package, context, action),
        DetailTab::Changelog => common::empty(
            ui,
            "recent",
            "No changelog provided",
            &format!(
                "This local package does not declare a changelog. Installed version: {}. OneCAD cannot check for newer versions.",
                package.manifest.version
            ),
        ),
    }
    ui.add_space(24.0);
}

fn breadcrumb(ui: &mut Ui, package: &PackageSummary, action: &mut ViewAction) {
    let theme = look(ui.ctx()).theme;
    ui.horizontal(|ui| {
        if kit::link(ui, "Extensions", theme.accent, 12.5).clicked() {
            action.page = Some(StartPage::Extensions);
        }
        common::label(ui, "/", 12.5, theme.ink5);
        common::label(ui, &package.manifest.name, 12.5, theme.ink);
    });
}

fn header(
    ui: &mut Ui,
    state: &mut State,
    package: &PackageSummary,
    context: PresentationContext,
    action: &mut ViewAction,
) {
    let theme = look(ui.ctx()).theme;
    let width = ui.available_width();
    ui.horizontal_top(|ui| {
        let (well, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
        kit::well(
            ui.painter(),
            well,
            common::icon(package),
            12,
            36.0,
            &theme,
            look(ui.ctx()).dark,
        );
        ui.add_space(8.0);
        ui.allocate_ui_with_layout(
            vec2((width - 290.0).max(120.0), 64.0),
            Layout::top_down(Align::Min),
            |ui| {
                common::heading(ui, &package.manifest.name, 20.0);
                common::label(
                    ui,
                    format!("Local package · v{}", package.manifest.version),
                    12.5,
                    theme.ink5,
                );
            },
        );
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 64.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                let more = ui.add(
                    Button::new("More")
                        .icon(Some("overflow"))
                        .kind(ButtonKind::Secondary),
                );
                egui::Popup::menu(&more)
                    .frame(menu_frame(ui.ctx()))
                    .width(210.0)
                    .show(|ui| {
                        ui.add(MenuRow::new(None, "Check for updates").enabled(false))
                            .on_disabled_hover_text("Online updates are not available");
                        if ui
                            .add(MenuRow::new(None, "Update from folder…").enabled(context.idle))
                            .clicked()
                        {
                            action.request = Some(Request::UpdateFolder {
                                id: package.manifest.id.clone(),
                                fingerprint: package.fingerprint.clone(),
                            });
                            ui.close();
                        }
                        if ui.add(MenuRow::new(None, "Copy package path")).clicked() {
                            ui.ctx().copy_text(package.path.display().to_string());
                            ui.close();
                        }
                        if ui
                            .add(MenuRow::new(None, "Uninstall").enabled(context.idle))
                            .clicked()
                        {
                            action.request = Some(Request::Uninstall {
                                id: package.manifest.id.clone(),
                            });
                            ui.close();
                        }
                    });
                let mut enabled = package.state == PackageState::Enabled;
                let portable = package.manifest.runtime == ExtensionRuntime::PortableComponent;
                if ui
                    .add(Toggle::new(&mut enabled).enabled(context.idle && portable))
                    .changed()
                {
                    if enabled {
                        state.review = Some(package.clone());
                    } else {
                        action.request = Some(Request::SetEnabled {
                            id: package.manifest.id.clone(),
                            enabled: false,
                            approval: None,
                        });
                    }
                }
                common::label(
                    ui,
                    common::status(package),
                    12.5,
                    common::status_color(package, ui),
                );
            },
        );
    });
}

fn overview(
    ui: &mut Ui,
    state: &State,
    package: &PackageSummary,
    context: PresentationContext,
    action: &mut ViewAction,
) {
    if ui.available_width() < 740.0 {
        contributions(ui, state, package, context, action);
        ui.add_space(24.0);
        permissions(ui, package);
        return;
    }
    let width = ui.available_width();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 28.0;
        ui.allocate_ui_with_layout(
            vec2(width - 300.0, 0.0),
            Layout::top_down(Align::Min),
            |ui| contributions(ui, state, package, context, action),
        );
        ui.allocate_ui_with_layout(vec2(272.0, 0.0), Layout::top_down(Align::Min), |ui| {
            permissions(ui, package)
        });
    });
}

fn contributions(
    ui: &mut Ui,
    state: &State,
    package: &PackageSummary,
    context: PresentationContext,
    action: &mut ViewAction,
) {
    let theme = look(ui.ctx()).theme;
    common::label(ui, &package.manifest.description, 13.0, theme.ink3);
    ui.add_space(20.0);
    kit::caption(ui, "What it adds · and where");
    ui.add_space(10.0);
    let commands = state
        .commands
        .iter()
        .filter(|entry| entry.extension_id == package.manifest.id)
        .collect::<Vec<_>>();
    if commands.is_empty() {
        common::label(ui, "This package declares no commands.", 12.5, theme.ink5);
    }
    for entry in commands {
        egui::Frame::new()
            .fill(theme.surface)
            .stroke(egui::Stroke::new(1.0, theme.border))
            .corner_radius(8)
            .inner_margin(12)
            .show(ui, |ui| {
                common::heading(ui, &entry.command.title, 13.0);
                if !entry.command.description.is_empty() {
                    common::label(ui, &entry.command.description, 12.5, theme.ink3);
                }
                ui.add_space(6.0);
                common::label(ui, common::locations(&entry.command), 12.0, theme.ink5);
                ui.add_space(8.0);
                if ui
                    .add_enabled(
                        common::available(entry, context),
                        Button::new("Run command"),
                    )
                    .on_disabled_hover_text(common::unavailable_reason(entry, context))
                    .clicked()
                {
                    action.command = Some(entry.command.id.clone());
                }
            });
        ui.add_space(10.0);
    }
    if let PackageState::Failed(error) = &package.state {
        common::label(ui, error, 12.0, theme.danger_strong);
    }
}

fn permissions(ui: &mut Ui, package: &PackageSummary) {
    let theme = look(ui.ctx()).theme;
    kit::caption(ui, "Permissions");
    ui.add_space(10.0);
    for capability in &package.manifest.capabilities {
        common::permission(ui, *capability);
    }
    if package.manifest.capabilities.is_empty() {
        common::label(
            ui,
            "No application capabilities declared.",
            12.5,
            theme.ink5,
        );
    }
    common::label(
        ui,
        "No network, clipboard or document editing access.",
        12.0,
        theme.ink5,
    );
    ui.add_space(20.0);
    ui.separator();
    for (name, value) in [
        ("Version", package.manifest.version.as_str()),
        ("Source", "Local package"),
        ("Publisher", "Not verified"),
        (
            "Runtime",
            if package.manifest.runtime == ExtensionRuntime::PortableComponent {
                "Sandboxed component"
            } else {
                "Native code · unavailable"
            },
        ),
    ] {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(78.0, 20.0), Layout::top_down(Align::Min), |ui| {
                common::label(ui, name, 12.0, theme.ink5)
            });
            common::label(ui, value, 12.0, theme.ink3);
        });
    }
    ui.add_space(12.0);
    egui::CollapsingHeader::new("Package details").show(ui, |ui| {
        common::label(ui, &package.manifest.id, 11.5, theme.ink5);
        common::label(ui, package.path.display().to_string(), 11.5, theme.ink5);
        ui.add(
            egui::Label::new(
                RichText::new(&package.fingerprint)
                    .family(fonts::mono())
                    .size(11.0),
            )
            .wrap(),
        );
    });
    ui.add_space(16.0);
    common::label(
        ui,
        "Disabling or uninstalling keeps your documents and previously exported files.",
        12.0,
        theme.ink5,
    );
}

fn settings(
    ui: &mut Ui,
    state: &mut State,
    package: &PackageSummary,
    context: PresentationContext,
    action: &mut ViewAction,
) {
    let theme = look(ui.ctx()).theme;
    if package.manifest.applications.is_empty() {
        common::empty(
            ui,
            "settings",
            "No application settings",
            "This extension does not declare any external applications.",
        );
    }
    for application in &package.manifest.applications {
        ui.push_id((&package.manifest.id, &application.id), |ui| {
            common::heading(ui, &application.name, 13.0);
            common::label(ui, "Application executable or macOS app bundle", 12.0, theme.ink5);
            ui.add_space(8.0);
            let value = state.application_drafts.entry(application_key(&package.manifest.id, &application.id)).or_default();
            ui.add_enabled(context.idle, egui::TextEdit::singleline(value).id(egui::Id::unique(("native-app-path", &package.manifest.id, &application.id))).desired_width(ui.available_width()).hint_text("Absolute path · leave empty for automatic discovery").font(egui::FontId::proportional(12.5)).margin(vec2(10.0, 7.0)));
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(context.idle, Button::new("Save path").kind(ButtonKind::Primary)).clicked() { action.request = Some(Request::SetApplication { extension: package.manifest.id.clone(), application: application.id.clone(), executable: (!value.trim().is_empty()).then(|| value.trim().into()) }); }
                if ui.add_enabled(context.idle, Button::new("Use automatic")).clicked() { action.request = Some(Request::SetApplication { extension: package.manifest.id.clone(), application: application.id.clone(), executable: None }); }
            });
            if let Some(bundle) = &application.macos_bundle_name { common::label(ui, format!("Automatic discovery checks for {bundle} on macOS. Other platforms require an executable path."), 12.0, theme.ink5); }
            ui.add_space(24.0);
        });
    }
    ui.separator();
    common::label(
        ui,
        "Application settings apply to every document in this profile.",
        12.0,
        theme.ink5,
    );
    if ui
        .add_enabled(
            context.idle,
            Button::new("Reset application paths…").kind(ButtonKind::Ghost),
        )
        .clicked()
    {
        state.reset_confirmation = true;
    }
}
