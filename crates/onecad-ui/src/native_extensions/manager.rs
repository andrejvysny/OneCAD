use super::{ManagerTab, PresentationContext, Request, State, ViewAction, common};
use crate::{extension_pages::kit, ui_state::StartPage};
use egui::{Align, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use onecad_extension_api::Capability;
use onecad_extension_host::{PackageState, PackageSummary};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        inputs::{SearchField, Select},
        lists::{Tab, TabBar},
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
    header(ui, state);
    ui.add_space(18.0);
    tabs(ui, state);
    ui.add_space(16.0);
    common::notice(ui, state);
    if !state.loaded {
        if state.pending || !state.attempted {
            common::empty(
                ui,
                "extension",
                "Loading local packages",
                "Installed extensions will appear when package discovery finishes.",
            );
        } else {
            common::empty(
                ui,
                "extension",
                "Extensions could not be loaded",
                "Refresh to retry reading local packages.",
            );
        }
        ui.add_space(20.0);
        footer(ui, state, context, action);
        return;
    }
    match state.tab {
        ManagerTab::Discover => discover(ui, state, context, action),
        ManagerTab::Installed => super::installed::show(ui, state, context, action),
        ManagerTab::Updates => updates(ui, state, context, action),
    }
    ui.add_space(20.0);
    footer(ui, state, context, action);
}

fn header(ui: &mut Ui, state: &mut State) {
    let theme = look(ui.ctx()).theme;
    let enabled = state
        .packages
        .iter()
        .filter(|package| package.state == PackageState::Enabled)
        .count();
    let wide = ui.available_width() >= 680.0;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            common::heading(ui, "Extensions", 20.0);
            common::label(
                ui,
                if state.loaded {
                    format!("{} installed · {enabled} active", state.packages.len())
                } else {
                    "Local packages".into()
                },
                12.5,
                theme.ink5,
            );
        });
        if wide {
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 44.0),
                Layout::right_to_left(Align::Center),
                |ui| filters(ui, state, 260.0),
            );
        }
    });
    if !wide {
        ui.add_space(12.0);
        ui.horizontal(|ui| filters(ui, state, (ui.available_width() - 145.0).max(150.0)));
    }
}

fn filters(ui: &mut Ui, state: &mut State, width: f32) {
    Select::new(
        "native-extension-source",
        "Local packages",
        &["Local packages"],
    )
    .width(128.0)
    .enabled(false)
    .show(ui)
    .response
    .on_disabled_hover_text("Online extension registries are not available");
    ui.add(
        SearchField::new("native-extension-search", &mut state.search)
            .placeholder("Search extensions")
            .width(width),
    );
}

fn tabs(ui: &mut Ui, state: &mut State) {
    let mut selected = match state.tab {
        ManagerTab::Discover => 0,
        ManagerTab::Installed => 1,
        ManagerTab::Updates => 2,
    };
    ui.add(TabBar::new(
        "native-extension-tabs",
        &mut selected,
        &[
            Tab {
                label: "Discover",
                count: None,
            },
            Tab {
                label: "Installed",
                count: Some(state.packages.len()),
            },
            Tab {
                label: "Updates",
                count: None,
            },
        ],
    ));
    state.tab = [
        ManagerTab::Discover,
        ManagerTab::Installed,
        ManagerTab::Updates,
    ][selected];
}

pub(super) fn filtered(state: &State) -> Vec<PackageSummary> {
    let query = state.search.trim().to_lowercase();
    state
        .packages
        .iter()
        .filter(|package| {
            let searchable = format!(
                "{} {} {}",
                package.manifest.name, package.manifest.id, package.manifest.description
            )
            .to_lowercase();
            searchable.contains(&query)
                && (state.tab != ManagerTab::Discover
                    || match state.category {
                        1 => package
                            .manifest
                            .capabilities
                            .contains(&Capability::Export3mf),
                        2 => !package.manifest.contributions.commands.is_empty(),
                        _ => true,
                    })
        })
        .cloned()
        .collect()
}

fn discover(ui: &mut Ui, state: &mut State, context: PresentationContext, action: &mut ViewAction) {
    categories(ui, state);
    ui.add_space(16.0);
    let theme = look(ui.ctx()).theme;
    common::label(
        ui,
        "Online discovery is not available. These are packages installed in this profile.",
        12.5,
        theme.ink5,
    );
    ui.add_space(16.0);
    let packages = filtered(state);
    if packages.is_empty() {
        no_packages(ui, state);
        return;
    }
    let cols = columns(ui.available_width());
    let width = (ui.available_width() - 16.0 * (cols - 1) as f32) / cols as f32;
    for row in packages.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            for package in row {
                card(ui, state, package, width, context, action);
            }
        });
        ui.add_space(16.0);
    }
}

pub(super) fn columns(width: f32) -> usize {
    ((width + 16.0) / 296.0).floor().clamp(1.0, 3.0) as usize
}

fn categories(ui: &mut Ui, state: &mut State) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (index, name) in ["All", "Import & export", "Commands"]
            .into_iter()
            .enumerate()
        {
            let theme = look(ui.ctx()).theme;
            let selected = state.category == index;
            let label = ui.painter().layout_no_wrap(
                name.into(),
                kit::fid(12.5, fonts::medium()),
                if selected { theme.surface } else { theme.ink3 },
            );
            let (rect, response) =
                ui.allocate_exact_size(vec2(label.size().x + 24.0, 28.0), Sense::click());
            kit::bordered(
                ui.painter(),
                rect,
                6,
                if selected { theme.ink } else { theme.surface },
                theme.border,
            );
            ui.painter().galley(
                pos2(rect.left() + 12.0, rect.center().y - label.size().y * 0.5),
                label,
                theme.ink3,
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, true, selected, name)
            });
            if response.clicked() {
                state.category = index;
            }
        }
    });
}

fn card(
    ui: &mut Ui,
    state: &mut State,
    package: &PackageSummary,
    width: f32,
    _context: PresentationContext,
    action: &mut ViewAction,
) {
    let theme = look(ui.ctx()).theme;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 180.0), Sense::click());
    let border = if response.hovered() {
        theme.card_hover_border
    } else {
        theme.border
    };
    kit::bordered(ui.painter(), rect, 10, theme.surface, border);
    let mut card = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink(16.0))
            .layout(Layout::top_down(Align::Min))
            .id_salt(&package.manifest.id),
    );
    card.set_clip_rect(rect.shrink(1.0));
    card.horizontal(|ui| {
        let (well, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        kit::well(
            ui.painter(),
            well,
            common::icon(package),
            10,
            24.0,
            &theme,
            look(ui.ctx()).dark,
        );
        ui.vertical(|ui| {
            let mut name = egui::text::LayoutJob::single_section(
                package.manifest.name.clone(),
                egui::text::TextFormat {
                    font_id: kit::fid(13.5, fonts::semibold()),
                    color: theme.ink,
                    line_height: Some(18.0),
                    ..Default::default()
                },
            );
            name.wrap.max_width = width - 88.0;
            name.wrap.max_rows = 2;
            ui.add(egui::Label::new(name));
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("Local package · {}", package.manifest.version))
                        .size(12.0)
                        .color(theme.ink5),
                )
                .truncate(),
            );
        });
    });
    card.add_space(10.0);
    let mut description = egui::text::LayoutJob::single_section(
        package.manifest.description.clone(),
        egui::text::TextFormat {
            font_id: egui::FontId::proportional(12.5),
            color: theme.ink3,
            line_height: Some(19.0),
            ..Default::default()
        },
    );
    description.wrap.max_width = width - 32.0;
    description.wrap.max_rows = 2;
    card.add(egui::Label::new(description));
    let bottom = Rect::from_min_max(
        pos2(rect.left() + 16.0, rect.bottom() - 38.0),
        pos2(rect.right() - 16.0, rect.bottom() - 12.0),
    );
    let mut footer = ui.new_child(
        UiBuilder::new()
            .max_rect(bottom)
            .layout(Layout::top_down(Align::Min)),
    );
    footer.horizontal(|ui| {
        common::label(ui, common::summary(package), 11.5, theme.ink5);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            common::label(
                ui,
                common::status(package),
                12.0,
                common::status_color(package, ui),
            )
        });
    });
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            true,
            format!(
                "{} · {} · Open details",
                package.manifest.name,
                common::status(package)
            ),
        )
    });
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        open_detail(state, package, action);
    }
}

pub(super) fn open_detail(state: &mut State, package: &PackageSummary, action: &mut ViewAction) {
    state.detail = Some(package.manifest.id.clone());
    state.detail_tab = Default::default();
    action.page = Some(StartPage::ExtensionDetail);
}

pub(super) fn no_packages(ui: &mut Ui, state: &State) {
    if state.search.trim().is_empty() && (state.tab != ManagerTab::Discover || state.category == 0)
    {
        common::empty(
            ui,
            "extension",
            "No extensions installed",
            "Install a package folder to add tools and integrations to this profile.",
        );
    } else {
        common::empty(
            ui,
            "search",
            "No matching extensions",
            "Try another name or category. Search only includes installed local packages.",
        );
    }
}

fn footer(ui: &mut Ui, state: &mut State, context: PresentationContext, action: &mut ViewAction) {
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                context.idle,
                Button::new("Install from folder…")
                    .icon(Some("folder"))
                    .kind(ButtonKind::Primary),
            )
            .clicked()
        {
            action.request = Some(Request::InstallFolder);
        }
        if ui
            .add_enabled(context.idle, Button::new("Refresh"))
            .clicked()
        {
            action.request = Some(Request::Refresh);
        }
        if ui
            .add_enabled(
                context.idle,
                Button::new("Reset application paths…").kind(ButtonKind::Ghost),
            )
            .clicked()
        {
            state.reset_confirmation = true;
        }
    });
    ui.add_space(10.0);
    common::label(
        ui,
        "Local packages require permission review. Disabling or uninstalling an extension keeps your document data.",
        12.0,
        look(ui.ctx()).theme.ink5,
    );
}

fn updates(ui: &mut Ui, state: &State, context: PresentationContext, action: &mut ViewAction) {
    common::empty(
        ui,
        "recent",
        "Online update checks are not available",
        "Choose a local replacement package to update an installed extension. A valid update turns the extension off until you review its new permissions. Invalid replacements leave the installed package unchanged.",
    );
    ui.add_space(16.0);
    for package in filtered(state) {
        ui.horizontal_wrapped(|ui| {
            common::heading(
                ui,
                &format!("{} · {}", package.manifest.name, package.manifest.version),
                13.0,
            );
            if ui
                .add_enabled(context.idle, Button::new("Update from folder…"))
                .clicked()
            {
                action.request = Some(Request::UpdateFolder {
                    id: package.manifest.id.clone(),
                    fingerprint: package.fingerprint.clone(),
                });
            }
        });
        ui.add_space(8.0);
    }
}
