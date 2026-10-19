//! Start screen (README 01a): sidebar, action tiles and recent cards under the title bar.
//! File intents use the native lifecycle controller; recent metadata comes from application services.
use crate::{app::App, document_flow::Request};
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
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let area = ui.max_rect();
            ui.painter().rect_filled(area, 0.0, app.theme.surface);
            sidebar(app, ui, area);
            let main = Rect::from_min_max(
                pos2(
                    area.left() + sidebar_width(area.width()) + MAIN_PAD_X,
                    area.top() + 32.0,
                ),
                pos2(area.right() - MAIN_PAD_X, area.bottom()),
            );
            ui.scope_builder(UiBuilder::new().max_rect(main), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("start-content")
                    .show(ui, |ui| main_column(app, ui));
            });
        });
    let editing_text = ctx.text_edit_focused();
    if !editing_text && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        leave(app);
    }
}

fn leave(app: &mut App) {
    app.ui.start_open = false;
    app.draw_dirty = true;
}

fn sidebar(app: &App, ui: &mut Ui, area: Rect) {
    let t = &app.theme;
    let rect = Rect::from_min_max(
        area.min,
        pos2(area.left() + sidebar_width(area.width()), area.bottom()),
    );
    ui.painter().rect_filled(rect, 0.0, t.panel);
    ui.painter().vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, t.border),
    );
    brand(app, ui, rect);
    let top = Rect::from_min_size(
        rect.min + vec2(10.0, 64.0),
        vec2(rect.width() - 21.0, 200.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(top), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let items = [
            ("home", "Home"),
            ("recent", "Recent"),
            ("starred", "Starred"),
            ("template", "Templates"),
            ("component", "Component library"),
        ];
        for (i, (icon, label)) in items.into_iter().enumerate() {
            ui.add(
                SidebarNavItem::new(icon, label)
                    .active(i == 0)
                    .enabled(i == 0),
            );
        }
    });
    let bottom = Rect::from_min_max(
        pos2(rect.left() + 10.0, rect.bottom() - 82.0),
        pos2(rect.right() - 11.0, rect.bottom() - 12.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(bottom), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add(SidebarNavItem::new("extension", "Extensions").enabled(false));
        ui.add(SidebarNavItem::new("settings", "Settings").enabled(false));
    });
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
