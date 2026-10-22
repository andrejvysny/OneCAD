use super::manager::{filtered, no_packages, open_detail};
use super::{PresentationContext, Request, State, ViewAction, common};
use crate::extension_pages::kit;
use egui::{Align, Layout, RichText, Sense, Ui, vec2};
use onecad_extension_host::{PackageState, PackageSummary};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        inputs::Toggle,
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
    let packages = filtered(state);
    if packages.is_empty() {
        no_packages(ui, state);
        return;
    }
    let theme = look(ui.ctx()).theme;
    egui::Frame::new()
        .fill(theme.surface)
        .stroke(egui::Stroke::new(1.0, theme.border))
        .corner_radius(12)
        .inner_margin(8)
        .show(ui, |ui| {
            for (index, package) in packages.iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                installed_row(ui, state, package, context, action);
            }
        });
    ui.add_space(20.0);
    if ui
        .add(
            Button::new(if state.developer_open {
                "Hide developer details"
            } else {
                "Developer details"
            })
            .kind(ButtonKind::Ghost),
        )
        .clicked()
    {
        state.developer_open = !state.developer_open;
    }
    if state.developer_open {
        developer(ui, state);
    }
}

fn installed_row(
    ui: &mut Ui,
    state: &mut State,
    package: &PackageSummary,
    context: PresentationContext,
    action: &mut ViewAction,
) {
    let theme = look(ui.ctx()).theme;
    let failed = matches!(package.state, PackageState::Failed(_));
    let frame = egui::Frame::new()
        .fill(if failed {
            theme.danger_surface
        } else {
            theme.surface
        })
        .corner_radius(8)
        .inner_margin(10);
    frame.show(ui, |ui| {
        let width = ui.available_width();
        ui.horizontal_top(|ui| {
            let (well, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
            kit::well(
                ui.painter(),
                well,
                common::icon(package),
                8,
                18.0,
                &theme,
                look(ui.ctx()).dark,
            );
            ui.allocate_ui_with_layout(
                vec2((width - 250.0).max(120.0), 40.0),
                Layout::top_down(Align::Min),
                |ui| {
                    let title = ui.add(
                        egui::Label::new(
                            RichText::new(&package.manifest.name)
                                .size(13.0)
                                .family(fonts::semibold())
                                .color(theme.ink),
                        )
                        .sense(Sense::click()),
                    );
                    if title.clicked() {
                        open_detail(state, package, action);
                    }
                    common::label(
                        ui,
                        format!(
                            "{} · {} · {}",
                            package.manifest.version,
                            common::summary(package),
                            common::status(package)
                        ),
                        12.0,
                        common::status_color(package, ui),
                    );
                },
            );
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 40.0),
                Layout::right_to_left(Align::Center),
                |ui| {
                    let mut enabled = package.state == PackageState::Enabled;
                    if ui
                        .add(Toggle::new(&mut enabled).enabled(
                            context.idle
                                && package.manifest.runtime
                                    == onecad_extension_api::ExtensionRuntime::PortableComponent,
                        ))
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
                    if ui
                        .add(Button::new("Details").kind(ButtonKind::Ghost))
                        .clicked()
                    {
                        open_detail(state, package, action);
                    }
                },
            );
        });
        if let PackageState::Failed(error) = &package.state {
            common::label(ui, error, 12.0, theme.danger_strong);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(context.idle, Button::new("Review & enable"))
                    .clicked()
                {
                    state.review = Some(package.clone());
                }
                if ui
                    .add(Button::new("Copy diagnostics").kind(ButtonKind::Ghost))
                    .clicked()
                {
                    ui.ctx().copy_text(error.clone());
                }
            });
        }
    });
}

fn developer(ui: &mut Ui, state: &State) {
    kit::caption(ui, "Developer");
    common::label(
        ui,
        "Refresh rechecks installed manifests. File watching and live reload are not available.",
        12.0,
        look(ui.ctx()).theme.ink5,
    );
    for package in &state.packages {
        ui.add_space(12.0);
        common::heading(ui, &package.manifest.id, 12.5);
        common::label(
            ui,
            package.path.display().to_string(),
            11.5,
            look(ui.ctx()).theme.ink5,
        );
        common::label(
            ui,
            format!("Manifest valid · {}", common::summary(package)),
            12.0,
            look(ui.ctx()).theme.ink4,
        );
        ui.add(
            egui::Label::new(
                RichText::new(&package.fingerprint)
                    .family(fonts::mono())
                    .size(11.0),
            )
            .wrap(),
        );
    }
}
