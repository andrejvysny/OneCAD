//! 01a Start screen: title bar with search, sidebar nav, action tiles, recent grid.

use egui::{CornerRadius, Rect, RichText, Stroke, Ui, pos2, vec2};
use onecad_widgets::components::button::{Button, ButtonKind};
use onecad_widgets::components::inputs::SearchField;
use onecad_widgets::components::lists::{Segment, SegmentedControl};
use onecad_widgets::components::start::{ActionTile, RecentCard, SidebarNavItem};
use onecad_widgets::icon::{self, IconStyle};
use onecad_widgets::metrics::{H_TITLEBAR, TITLE};
use onecad_widgets::{Theme, fonts};

use super::chrome::{self, Cx};
use super::layout::Flags;

const SIDEBAR_W: f32 = 240.0;
const MAIN_PAD_X: f32 = 40.0;
const MAIN_W: f32 = 1280.0 - SIDEBAR_W - 2.0 * MAIN_PAD_X;

pub fn s01a(ui: &mut Ui, t: &Theme, dark: bool, stage: Rect) {
    let flags = Flags {
        explorer_open: false,
        inspector_open: false,
    };
    let cx = Cx::new(t, dark, stage, flags);
    ui.painter().rect_filled(stage, 0.0, t.surface);
    title_bar(ui, &cx);
    sidebar(ui, &cx);
    let main = Rect::from_min_max(
        pos2(
            stage.left() + SIDEBAR_W + MAIN_PAD_X,
            stage.top() + H_TITLEBAR + 32.0,
        ),
        pos2(stage.right() - MAIN_PAD_X, stage.bottom()),
    );
    chrome::at(ui, main, |ui| main_column(ui, &cx));
}

fn title_bar(ui: &mut Ui, cx: &Cx) {
    let bar = cx.lay.titlebar;
    let p = ui.painter();
    p.rect_filled(bar, 0.0, cx.t.titlebar);
    p.hline(
        bar.x_range(),
        bar.bottom() - 0.5,
        Stroke::new(1.0, cx.t.border),
    );
    chrome::traffic_lights(ui, cx);
    let search = Rect::from_min_size(
        pos2(bar.right() - 16.0 - 260.0, bar.top() + 8.0),
        vec2(260.0, 28.0),
    );
    chrome::at(ui, search, |ui| {
        let mut q = String::new();
        ui.add(
            SearchField::new("screens-start-search", &mut q)
                .placeholder("Search projects and commands")
                .shortcut(Some("⌘K"))
                .width(260.0),
        );
    });
}

fn sidebar(ui: &mut Ui, cx: &Cx) {
    let s = cx.lay.stage;
    let rect = Rect::from_min_max(
        pos2(s.left(), s.top() + H_TITLEBAR),
        pos2(s.left() + SIDEBAR_W, s.bottom()),
    );
    ui.painter().rect_filled(rect, 0.0, cx.t.panel);
    ui.painter().vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, cx.t.border),
    );
    brand(ui, cx, rect);
    let top = Rect::from_min_size(rect.min + vec2(10.0, 80.0), vec2(SIDEBAR_W - 21.0, 180.0));
    chrome::at(ui, top, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let items = [
            ("home", "Home"),
            ("recent", "Recent"),
            ("starred", "Starred"),
            ("template", "Templates"),
            ("component", "Component library"),
        ];
        for (i, (icon, label)) in items.into_iter().enumerate() {
            ui.add(SidebarNavItem::new(icon, label).active(i == 0));
        }
    });
    let bottom = Rect::from_min_max(
        pos2(rect.left() + 10.0, rect.bottom() - 82.0),
        pos2(rect.right() - 11.0, rect.bottom() - 12.0),
    );
    chrome::at(ui, bottom, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add(SidebarNavItem::new("extension", "Extensions"));
        ui.add(SidebarNavItem::new("settings", "Settings"));
    });
}

fn brand(ui: &mut Ui, cx: &Cx, sidebar: Rect) {
    let t = cx.t;
    let well = Rect::from_min_size(sidebar.min + vec2(16.0, 16.0), vec2(28.0, 28.0));
    let p = ui.painter();
    p.rect_filled(well, CornerRadius::same(7), t.accent);
    let style = IconStyle {
        tint: t.on_accent,
        bg: t.accent,
        grey: false,
    };
    icon::paint(p, well.shrink(5.0), "body", cx.dark, style, 1.0);
    p.text(
        pos2(well.right() + 10.0, well.center().y),
        egui::Align2::LEFT_CENTER,
        "OneCAD",
        egui::FontId::new(TITLE, fonts::semibold()),
        t.ink,
    );
}

fn heading(ui: &mut Ui, cx: &Cx, text: &str) {
    ui.label(
        RichText::new(text)
            .size(TITLE)
            .family(fonts::semibold())
            .color(cx.t.ink),
    );
}

fn main_column(ui: &mut Ui, cx: &Cx) {
    ui.spacing_mut().item_spacing = vec2(14.5, 0.0);
    tiles(ui);
    ui.add_space(28.0);
    recent_header(ui, cx);
    ui.add_space(14.0);
    recent_grid(ui);
    ui.add_space(28.0);
    heading(ui, cx, "Start from a template");
    ui.add_space(14.0);
    templates(ui);
}

fn tiles(ui: &mut Ui) {
    let w = (MAIN_W - 2.0 * 14.5) / 3.0;
    let specs = [
        (
            true,
            "body",
            "New part",
            "Sketch and model a single part · ⌘N",
        ),
        (
            false,
            "component",
            "New assembly",
            "Combine parts and library components",
        ),
        (
            false,
            "import",
            "Open or import",
            ".onecad, STEP, STL, 3MF · ⌘O",
        ),
    ];
    ui.horizontal(|ui| {
        for (primary, icon, title, sub) in specs {
            ui.allocate_ui(vec2(w, 92.0), |ui| {
                ui.add(ActionTile::new(icon, title, sub).primary(primary));
            });
        }
    });
}

fn recent_header(ui: &mut Ui, cx: &Cx) {
    ui.horizontal(|ui| {
        heading(ui, cx, "Recent");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut view = 0usize;
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
            ui.add(SegmentedControl::new("screens-recent-view", &mut view, &segs).width(90.0));
        });
    });
}

fn recent_grid(ui: &mut Ui) {
    let w = (MAIN_W - 3.0 * 16.0) / 4.0;
    let cards = [
        ("Mounting bracket", "Edited 12 min ago · Part"),
        ("Gearbox housing", "Yesterday · Assembly · 6 parts"),
        ("NEMA 17 motor mount", "Oct 14 · Part"),
        ("Pi 5 enclosure", "Oct 9 · Part · STEP import"),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for (name, meta) in cards {
            ui.add(RecentCard::new(name, meta).width(w));
        }
    });
}

fn templates(ui: &mut Ui) {
    let chips = [
        ("template", "3D-print part (mm, 0.4 nozzle)"),
        ("template", "Sheet bracket"),
        ("template", "Enclosure with lid"),
        ("gear", "Spur gear pair"),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        for (icon, label) in chips {
            ui.add(
                Button::new(label)
                    .kind(ButtonKind::Secondary)
                    .icon(Some(icon)),
            );
        }
    });
}
