//! General settings (design 24a): appearance, units for new documents, startup.
use super::rows::{self, note, row, row_top, section};
use crate::ui_state::Appearance;
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use onecad_widgets::{
    components::inputs::{Select, Toggle},
    fonts,
    icon::{self, IconStyle},
    metrics::{CONTROL, focus_ring},
    theme::look,
};

const CARD: egui::Vec2 = vec2(128.0, 76.0);
const DECIMALS: [&str; 4] = ["0", "0.0", "0.00", "0.000"];

/// Draws the page; returns an appearance the caller must apply through the app.
pub fn show(ui: &mut Ui, appearance: Appearance, decimals: &mut u8) -> Option<Appearance> {
    let subtitle = format!("Applies to OneCAD on {}.", rows::device());
    rows::page_header(ui, "General", &subtitle);
    let chosen = appearance_section(ui, appearance);
    units_section(ui, decimals);
    startup_section(ui);
    chosen
}

fn appearance_section(ui: &mut Ui, appearance: Appearance) -> Option<Appearance> {
    let mut chosen = None;
    section(ui, "APPEARANCE", |ui| {
        row_top(ui, "Theme", |ui| chosen = theme_cards(ui, appearance));
        let caption = Some("Panels, pill and lists");
        row(ui, "Interface density", caption, |ui| {
            let labels = ["Compact", "Default", "Comfortable"];
            let why = "Interface density is not available yet";
            rows::segmented_disabled(ui, "density", 1, &labels, "Interface density", why);
        });
    });
    chosen
}

fn units_section(ui: &mut Ui, decimals: &mut u8) {
    section(ui, "UNITS FOR NEW DOCUMENTS", |ui| {
        let why = "New documents use millimetres; other units are not available yet";
        row(ui, "Length", None, |ui| {
            fixed_select(ui, "length", "Millimetres (mm)", "Length", why);
        });
        row(ui, "Angle", None, |ui| {
            fixed_select(ui, "angle", "Degrees (°)", "Angle", why);
        });
        let caption = Some("Values are stored at full precision");
        row(ui, "Displayed decimals", caption, |ui| {
            decimals_control(ui, decimals);
        });
        note(ui, "Open documents keep their own units.");
    });
}

fn startup_section(ui: &mut Ui) {
    section(ui, "STARTUP", |ui| {
        row(ui, "On launch", None, |ui| {
            let why = "Launch always opens Home";
            fixed_select(ui, "launch", "Show Home", "On launch", why);
        });
        let caption = Some("Once a day, never during modeling");
        row(ui, "Check for updates", caption, |ui| {
            let mut off = false;
            let response = ui.add(Toggle::new(&mut off).enabled(false).hit_height(rows::hit()));
            let why = "Update checks are not available yet";
            rows::reason(response, "Check for updates", why);
        });
    });
}

fn fixed_select(ui: &mut Ui, id: &str, value: &str, name: &str, why: &str) {
    let options = [value];
    let out = Select::new(id, value, &options)
        .width(200.0)
        .hit_height(rows::hit())
        .enabled(false)
        .show(ui);
    rows::reason(out.response, name, why);
}

fn decimals_control(ui: &mut Ui, decimals: &mut u8) {
    if let Some(index) = rows::segmented(ui, "decimals", decimals_index(*decimals), &DECIMALS) {
        *decimals = index as u8;
    }
}

fn decimals_index(decimals: u8) -> usize {
    usize::from(decimals).min(DECIMALS.len() - 1)
}

fn theme_cards(ui: &mut Ui, current: Appearance) -> Option<Appearance> {
    let mut chosen = None;
    ui.spacing_mut().item_spacing.x = 12.0;
    // Three across at design size when there is room, shrunk to fit otherwise.
    let width = ((ui.available_width() - 24.0) / 3.0).clamp(60.0, CARD.x);
    for (choice, label, glyph) in [
        (Appearance::Light, "Light", "themeLight"),
        (Appearance::Dark, "Dark", "themeDark"),
        (Appearance::System, "Match system", "themeSystem"),
    ] {
        if theme_card(ui, choice, (label, glyph), width, choice == current) && choice != current {
            chosen = Some(choice);
        }
    }
    chosen
}

/// One preview card of `width` (design ratio) with its caption; true when clicked.
fn theme_card(
    ui: &mut Ui,
    choice: Appearance,
    (label, glyph): (&str, &str),
    width: f32,
    selected: bool,
) -> bool {
    let l = look(ui.ctx());
    let t = &l.theme;
    let scale = width / CARD.x;
    let size = vec2(width, CARD.y * scale + 7.0 + 16.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let card = Rect::from_min_size(rect.min, vec2(width, CARD.y * scale));
    paint_preview(ui, card, choice, scale);
    let border = if selected { t.accent } else { t.border_strong };
    let p = ui.painter();
    p.rect_stroke(
        card,
        CornerRadius::same(8),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    if selected {
        let (w, ring) = focus_ring(t);
        p.rect_stroke(
            card.expand(w * 0.5),
            CornerRadius::same(10),
            Stroke::new(w, ring),
            StrokeKind::Outside,
        );
    }
    let ink = if selected { t.ink } else { t.ink3 };
    let at = pos2(rect.center().x, card.bottom() + 15.0);
    caption(ui, at, label, glyph, (ink, selected));
    let name = format!("{label} theme");
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, &name));
    response.clicked()
}

/// Glyph and label centred on `at`.
fn caption(ui: &Ui, at: Pos2, label: &str, glyph: &str, (ink, selected): (Color32, bool)) {
    let l = look(ui.ctx());
    let p = ui.painter();
    let family = if selected {
        fonts::medium()
    } else {
        egui::FontFamily::Proportional
    };
    let galley = p.layout_no_wrap(label.to_owned(), egui::FontId::new(CONTROL, family), ink);
    let x = at.x - (20.0 + galley.size().x) / 2.0;
    let icon_rect = Rect::from_center_size(pos2(x + 7.0, at.y), vec2(14.0, 14.0));
    let style = IconStyle {
        tint: ink,
        bg: l.theme.surface,
        grey: false,
    };
    icon::paint(p, icon_rect, glyph, l.dark, style, 1.0);
    p.galley(pos2(x + 20.0, at.y - galley.size().y / 2.0), galley, ink);
}

// The thumbnails depict the Light and Dark themes themselves, so they use fixed
// colours; theme tokens would change with the active theme and defeat the preview.
const LIGHT_BG: Color32 = Color32::from_rgb(0xEE, 0xF0, 0xF3);
const LIGHT_BLOCK: Color32 = Color32::WHITE;
const LIGHT_EDGE: Color32 = Color32::from_rgb(0xE2, 0xE4, 0xE8);
const DARK_BG: Color32 = Color32::from_rgb(0x2A, 0x2E, 0x34);
const DARK_BLOCK: Color32 = Color32::from_rgb(0x3A, 0x3F, 0x46);

fn paint_preview(ui: &Ui, card: Rect, choice: Appearance, k: f32) {
    let p = ui.painter().with_clip_rect(card);
    let radius = CornerRadius::same(8);
    let half = Rect::from_min_max(card.min, pos2(card.center().x, card.bottom()));
    match choice {
        Appearance::Light => {
            p.rect_filled(card, radius, LIGHT_BG);
        }
        Appearance::Dark => {
            p.rect_filled(card, radius, DARK_BG);
        }
        Appearance::System => {
            p.rect_filled(card, radius, DARK_BG);
            p.rect_filled(
                half,
                CornerRadius {
                    ne: 0,
                    se: 0,
                    ..radius
                },
                LIGHT_BG,
            );
        }
    }
    let side = |x: f32, light: bool| {
        let rect = Rect::from_min_max(
            pos2(x, card.top() + 10.0 * k),
            pos2(x + 30.0 * k, card.bottom() - 10.0 * k),
        );
        block(&p, rect, light);
    };
    match choice {
        Appearance::Light => side(card.left() + 8.0 * k, true),
        Appearance::Dark => side(card.left() + 8.0 * k, false),
        Appearance::System => {
            side(card.left() + 8.0 * k, true);
            side(card.right() - 38.0 * k, false);
        }
    }
    if choice != Appearance::System {
        let top = Pos2::new(card.center().x - 6.0 * k, card.top() + 10.0 * k);
        block(
            &p,
            Rect::from_min_size(top, vec2(40.0 * k, 8.0 * k)),
            choice == Appearance::Light,
        );
    }
}

fn block(p: &egui::Painter, rect: Rect, light: bool) {
    let radius = CornerRadius::same(4);
    if light {
        p.rect_filled(rect, radius, LIGHT_BLOCK);
        p.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, LIGHT_EDGE),
            StrokeKind::Inside,
        );
    } else {
        p.rect_filled(rect, radius, DARK_BLOCK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_segment_maps_both_ways_and_clamps() {
        assert_eq!(decimals_index(0), 0);
        assert_eq!(decimals_index(2), 2);
        assert_eq!(decimals_index(9), 3);
    }
}
