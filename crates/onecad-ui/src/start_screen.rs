//! Start screen (README 01a): sidebar, action tiles and recent cards under the title bar.
//! Everything except "Back to document" is display-only; creation and file flows are not
//! wired in this host yet.
use crate::app::App;
use egui::{CornerRadius, Rect, RichText, Stroke, Ui, UiBuilder, pos2, vec2};
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
const NOT_YET: &str = "Not available yet";

pub fn show(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let area = ui.max_rect();
            ui.painter().rect_filled(area, 0.0, app.theme.surface);
            sidebar(app, ui, area);
            let main = Rect::from_min_max(
                pos2(area.left() + SIDEBAR_W + MAIN_PAD_X, area.top() + 32.0),
                pos2(area.right() - MAIN_PAD_X, area.bottom()),
            );
            ui.scope_builder(UiBuilder::new().max_rect(main), |ui| main_column(app, ui));
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
    let rect = Rect::from_min_max(area.min, pos2(area.left() + SIDEBAR_W, area.bottom()));
    ui.painter().rect_filled(rect, 0.0, t.panel);
    ui.painter().vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, t.border),
    );
    brand(app, ui, rect);
    let top = Rect::from_min_size(rect.min + vec2(10.0, 64.0), vec2(SIDEBAR_W - 21.0, 200.0));
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
    tiles(ui);
    ui.add_space(28.0);
    recent_header(app, ui);
    ui.add_space(14.0);
    recent_cards(app, ui);
}

fn tiles(ui: &mut Ui) {
    let w = ((ui.available_width() - 2.0 * 14.5) / 3.0).max(120.0);
    let specs = [
        (true, "body", "New part", "Sketch and model a single part"),
        (
            false,
            "component",
            "New assembly",
            "Combine parts and library components",
        ),
        (false, "import", "Open or import", ".onecad, STEP, STL, 3MF"),
    ];
    ui.horizontal(|ui| {
        for (primary, icon, title, sub) in specs {
            ui.allocate_ui(vec2(w, 92.0), |ui| {
                let tile = ActionTile::new(icon, title, sub)
                    .primary(primary)
                    .enabled(false);
                ui.add(tile).on_hover_text(NOT_YET);
            });
        }
    });
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

fn recent_cards(app: &App, ui: &mut Ui) {
    let current = app
        .display
        .as_ref()
        .map_or_else(|| "OneCAD".to_owned(), |d| d.projection.title.clone());
    let cards = [
        (current.as_str(), "Edited today"),
        ("Gearbox housing", "Yesterday"),
        ("Fan bracket", "Oct 14"),
        ("Cable clip", "Oct 9"),
    ];
    let w = ((ui.available_width() - 3.0 * 16.0) / 4.0).max(120.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for (name, meta) in cards {
            ui.add(RecentCard::new(name, meta).width(w));
        }
    });
}
