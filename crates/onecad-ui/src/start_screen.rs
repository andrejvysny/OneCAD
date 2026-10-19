//! Start screen (README 01a): sidebar, action tiles and recent cards under the title bar.
//! File intents use the native lifecycle controller; recent metadata comes from application services.
use crate::{app::App, document_flow::Request, ui_state::StartPage};
use egui::{CornerRadius, Rect, RichText, Stroke, Ui, UiBuilder, pos2, vec2};
use onecad_app::dto::RecentProjectDto;
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        lists::{Segment, SegmentedControl},
        start::{ActionTile, RecentCard, SidebarNavItem},
    },
    fonts,
    icon::{self, IconStyle},
    metrics::TITLE,
};

const SIDEBAR_W: f32 = 240.0;
const MAIN_PAD_X: f32 = 40.0;

pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    // Sheets close themselves on Esc while drawing; sample before so one press closes only the sheet.
    let modal_open = app.ui.library.sheet.is_some()
        || app.ui.library.wizard.is_some()
        || app.ui.extensions.sheet.is_some();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let area = ui.max_rect();
            ui.painter().rect_filled(area, 0.0, app.theme.surface);
            sidebar(app, ui, area);
            page(app, ui, area);
        });
    let editing_text = ctx.text_edit_focused();
    if !editing_text && !modal_open && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        back(app);
    }
}

/// Esc steps from a detail page to its list, and from a list out of the Start screen.
fn back(app: &mut App) {
    app.ui.start_page = match app.ui.start_page {
        StartPage::ComponentDetail | StartPage::StandardPart => StartPage::Library,
        StartPage::ExtensionDetail => StartPage::Extensions,
        StartPage::Home | StartPage::Library | StartPage::Extensions => return leave(app),
    };
}

/// Main area right of the sidebar. Library pages own their padding and scrolling because
/// the standard-part page runs edge to edge.
fn page(app: &mut App, ui: &mut Ui, area: Rect) {
    let content = Rect::from_min_max(
        pos2(area.left() + sidebar_width(area.width()), area.top()),
        area.max,
    );
    if matches!(
        app.ui.start_page,
        StartPage::Library | StartPage::ComponentDetail | StartPage::StandardPart
    ) {
        ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
            crate::library_pages::show(app, ui);
        });
        return;
    }
    let main = Rect::from_min_max(
        pos2(content.left() + MAIN_PAD_X, area.top() + 32.0),
        pos2(area.right() - MAIN_PAD_X, area.bottom()),
    );
    ui.scope_builder(UiBuilder::new().max_rect(main), |ui| {
        egui::ScrollArea::vertical()
            .id_salt("start-content")
            .show(ui, |ui| match app.ui.start_page {
                StartPage::Extensions | StartPage::ExtensionDetail => {
                    crate::extension_pages::show(app, ui);
                }
                _ => main_column(app, ui),
            });
    });
}

fn leave(app: &mut App) {
    app.ui.start_open = false;
    app.ui.start_page = StartPage::Home;
    app.draw_dirty = true;
}

/// Sidebar destinations; `Sub` indexes the library source rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Nav {
    Home,
    Library,
    Sub(usize),
    Extensions,
}

const SUB_ITEMS: [&str; 3] = ["My components", "This project", "Standard parts"];

fn navigate(state: &mut crate::ui_state::UiState, nav: Nav) {
    match nav {
        Nav::Home => state.start_page = StartPage::Home,
        Nav::Library => {
            state.start_page = StartPage::Library;
            state.library.source = 0;
        }
        Nav::Sub(2) => state.start_page = StartPage::StandardPart,
        Nav::Sub(i) => {
            state.start_page = StartPage::Library;
            state.library.source = i + 1;
        }
        Nav::Extensions => state.start_page = StartPage::Extensions,
    }
}

fn library_active(page: StartPage) -> bool {
    matches!(
        page,
        StartPage::Library | StartPage::ComponentDetail | StartPage::StandardPart
    )
}

fn sub_active(state: &crate::ui_state::UiState, i: usize) -> bool {
    match (state.start_page, i) {
        (StartPage::StandardPart, 2) => true,
        (StartPage::Library, 0 | 1) => state.library.source == i + 1,
        _ => false,
    }
}

fn sidebar(app: &mut App, ui: &mut Ui, area: Rect) {
    let rect = Rect::from_min_max(
        area.min,
        pos2(area.left() + sidebar_width(area.width()), area.bottom()),
    );
    let t = &app.theme;
    ui.painter().rect_filled(rect, 0.0, t.panel);
    ui.painter().vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, t.border),
    );
    brand(app, ui, rect);
    let mut nav = None;
    let top = Rect::from_min_size(
        rect.min + vec2(10.0, 64.0),
        vec2(rect.width() - 21.0, 260.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(top), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        nav = top_items(app, ui).or(nav);
    });
    let bottom = Rect::from_min_max(
        pos2(rect.left() + 10.0, rect.bottom() - 82.0),
        pos2(rect.right() - 11.0, rect.bottom() - 12.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(bottom), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let page = app.ui.start_page;
        let (_, _, pending) = app.ui.extensions.counts();
        let updates = (pending > 0).then(|| pending.to_string());
        let ext = SidebarNavItem::new("extension", "Extensions")
            .active(matches!(
                page,
                StartPage::Extensions | StartPage::ExtensionDetail
            ))
            .badge(updates.as_deref());
        if ui.add(ext).clicked() {
            nav = Some(Nav::Extensions);
        }
        ui.add(SidebarNavItem::new("settings", "Settings").enabled(false));
    });
    if let Some(nav) = nav {
        navigate(&mut app.ui, nav);
    }
}

fn top_items(app: &App, ui: &mut Ui) -> Option<Nav> {
    let page = app.ui.start_page;
    let mut nav = None;
    let items = [
        ("home", "Home", Some(Nav::Home)),
        ("recent", "Recent", None),
        ("starred", "Starred", None),
        ("template", "Templates", None),
        ("component", "Component library", Some(Nav::Library)),
    ];
    for (icon, label, target) in items {
        let active = match target {
            Some(Nav::Home) => page == StartPage::Home,
            Some(Nav::Library) => library_active(page),
            _ => false,
        };
        let item = SidebarNavItem::new(icon, label)
            .active(active)
            .enabled(target.is_some());
        if ui.add(item).clicked() {
            nav = target;
        }
    }
    if library_active(page) {
        for (i, label) in SUB_ITEMS.into_iter().enumerate() {
            let item = SidebarNavItem::new("", label)
                .sub(true)
                .active(sub_active(&app.ui, i));
            if ui.add(item).clicked() {
                nav = Some(Nav::Sub(i));
            }
        }
    }
    nav
}

fn brand(app: &App, ui: &mut Ui, sidebar: Rect) {
    let t = &app.theme;
    let well = Rect::from_min_size(sidebar.min + vec2(16.0, 16.0), vec2(28.0, 28.0));
    let p = ui.painter();
    p.rect_filled(well, CornerRadius::same(7), t.accent);
    let style = IconStyle {
        tint: t.on_accent,
        bg: t.accent,
        grey: false,
    };
    icon::paint(p, well.shrink(5.0), "body", app.dark, style, 1.0);
    p.text(
        pos2(well.right() + 10.0, well.center().y),
        egui::Align2::LEFT_CENTER,
        "OneCAD",
        egui::FontId::new(TITLE, fonts::semibold()),
        t.ink,
    );
}

fn heading(app: &App, ui: &mut Ui, text: &str) {
    let text = RichText::new(text)
        .size(TITLE)
        .family(fonts::semibold())
        .color(app.theme.ink);
    ui.label(text);
}

fn main_column(app: &mut App, ui: &mut Ui) {
    ui.spacing_mut().item_spacing = vec2(14.5, 0.0);
    let back = Button::new("Back to document")
        .kind(ButtonKind::Secondary)
        .icon(Some("home"));
    if ui.add(back).clicked() {
        leave(app);
    }
    ui.add_space(20.0);
    tiles(app, ui);
    ui.add_space(28.0);
    recent_header(app, ui);
    ui.add_space(14.0);
    recent_cards(app, ui);
}

fn sidebar_width(width: f32) -> f32 {
    SIDEBAR_W.min((width * 0.25).max(120.0))
}

struct Tile {
    primary: bool,
    icon: &'static str,
    title: &'static str,
    subtitle: &'static str,
    request: Option<Request>,
}

fn tile_specs() -> [Tile; 3] {
    [
        Tile {
            primary: true,
            icon: "body",
            title: "New part",
            subtitle: "Sketch and model a single part",
            request: Some(Request::New),
        },
        Tile {
            primary: false,
            icon: "component",
            title: "New assembly",
            subtitle: "Not available yet",
            request: None,
        },
        Tile {
            primary: false,
            icon: "import",
            title: "Open or import",
            subtitle: ".onecad and STEP",
            request: Some(Request::Import),
        },
    ]
}

fn tile_columns(width: f32) -> usize {
    ((width + 14.5) / (180.0 + 14.5)).floor().clamp(1.0, 3.0) as usize
}

fn tiles(app: &mut App, ui: &mut Ui) {
    let columns = tile_columns(ui.available_width());
    let width = (ui.available_width() - 14.5 * (columns - 1) as f32) / columns as f32;
    let specs = tile_specs();
    for row in specs.chunks(columns) {
        ui.horizontal(|ui| {
            for tile in row {
                ui.allocate_ui(vec2(width, 92.0), |ui| {
                    let enabled = tile.request.is_some() && crate::bars::replacement_allowed(app);
                    if ui
                        .add(
                            ActionTile::new(tile.icon, tile.title, tile.subtitle)
                                .primary(tile.primary)
                                .enabled(enabled),
                        )
                        .clicked()
                        && let Some(request) = &tile.request
                    {
                        app.document_request(request.clone());
                    }
                });
            }
        });
        ui.add_space(14.5);
    }
}

fn recent_header(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        heading(app, ui, "Recent");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut view = app.ui.recent_view;
            let segs = [
                Segment {
                    label: "Grid",
                    icon: None,
                },
                Segment {
                    label: "List",
                    icon: None,
                },
            ];
            let control = SegmentedControl::new("start-recent-view", &mut view, &segs).width(90.0);
            ui.add(control);
            app.ui.recent_view = view;
        });
    });
}

fn recent_metadata(recent: &RecentProjectDto) -> String {
    if recent.has_recovery {
        "Recovery available; saved file is older".to_owned()
    } else {
        recent.modified_at.clone()
    }
}

fn recent_cards(app: &mut App, ui: &mut Ui) {
    let recents = app
        .bridge
        .shared
        .recents
        .lock()
        .map(|entries| entries.clone())
        .unwrap_or_default();
    if recents.is_empty() {
        ui.label("No recent documents. Create a part or open a document.");
        return;
    }
    let columns = if app.ui.recent_view == 1 {
        1
    } else {
        (ui.available_width() / 220.0).floor().clamp(1.0, 4.0) as usize
    };
    let width = (ui.available_width() - 16.0 * (columns - 1) as f32) / columns as f32;
    let enabled = crate::bars::replacement_allowed(app);
    for row in recents.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            for recent in row {
                let metadata = recent_metadata(recent);
                let response = ui.add_enabled(
                    enabled,
                    RecentCard::new(&recent.name, &metadata)
                        .width(width)
                        .metadata_only(),
                );
                if response
                    .on_hover_text(format!("{}\n{}", recent.path, metadata))
                    .clicked()
                {
                    app.document_request(Request::Open(Some(recent.path.clone().into())));
                }
            }
        });
        ui.add_space(16.0);
    }
}

#[cfg(test)]
mod tests;
