//! Painters shared by the library pages and the create-component wizard.
use egui::{
    Color32, CornerRadius, CursorIcon, FontFamily, FontId, Painter, Rect, Response, RichText,
    Sense, Stroke, StrokeKind, Ui, pos2, text::LayoutJob, text::TextWrapping, vec2,
};
use onecad_widgets::{
    fonts,
    icon::{self, IconStyle},
    metrics::{self, Elevation, RADIUS_LG},
    theme::{Theme, look},
};
use std::sync::Arc;

use super::data::Component;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipKind {
    Plain,
    Accent,
    Warn,
    Dark,
}

fn chip_colours(t: &Theme, kind: ChipKind) -> (Color32, Color32, Option<Color32>) {
    match kind {
        ChipKind::Plain => (t.well, t.ink3, None),
        ChipKind::Accent => (t.sel_soft, t.sel_text, None),
        ChipKind::Warn => (t.warn_surface, t.warn, Some(t.warn_border)),
        ChipKind::Dark => (t.tooltip, t.tooltip_text, None),
    }
}

fn chip_galley(p: &Painter, t: &Theme, label: &str, kind: ChipKind) -> Arc<egui::Galley> {
    let ink = chip_colours(t, kind).1;
    p.layout_no_wrap(label.to_owned(), FontId::new(11.0, fonts::medium()), ink)
}

/// 20px tag (r5, 11/500); returns its width so callers can flow chips.
pub fn paint_chip(p: &Painter, t: &Theme, pos: egui::Pos2, label: &str, kind: ChipKind) -> f32 {
    let g = chip_galley(p, t, label, kind);
    let rect = Rect::from_min_size(pos, vec2(g.size().x + 14.0, 20.0));
    let (fill, _, border) = chip_colours(t, kind);
    p.rect_filled(rect, CornerRadius::same(5), fill);
    if let Some(b) = border {
        p.rect_stroke(
            rect,
            CornerRadius::same(5),
            Stroke::new(1.0, b),
            StrokeKind::Inside,
        );
    }
    p.galley(
        pos2(rect.left() + 7.0, rect.center().y - g.size().y / 2.0),
        g,
        t.ink,
    );
    rect.width()
}

pub fn chip(ui: &mut Ui, label: &str, kind: ChipKind) -> Response {
    let t = look(ui.ctx()).theme;
    let w = chip_galley(ui.painter(), &t, label, kind).size().x + 14.0;
    let (rect, r) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
    paint_chip(ui.painter(), &t, rect.min, label, kind);
    r
}

/// Single-line text clipped with an ellipsis at `max_w`.
pub fn fit(
    p: &Painter,
    text: &str,
    font: FontId,
    colour: Color32,
    max_w: f32,
) -> Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, colour);
    job.wrap = TextWrapping {
        max_width: max_w.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    p.layout_job(job)
}

pub fn label(ui: &mut Ui, text: &str, size: f32, family: FontFamily, colour: Color32) -> Response {
    let rich = RichText::new(text).size(size).family(family).color(colour);
    ui.add(egui::Label::new(rich).selectable(false))
}

pub fn title(ui: &mut Ui, text: &str, size: f32) -> Response {
    let ink = look(ui.ctx()).theme.ink;
    label(ui, text, size, fonts::semibold(), ink)
}

pub fn muted(ui: &mut Ui, text: &str, size: f32) -> Response {
    let c = look(ui.ctx()).theme.ink4;
    label(ui, text, size, FontFamily::Proportional, c)
}

/// Accent-coloured text link.
pub fn link(ui: &mut Ui, text: &str, size: f32) -> Response {
    let t = look(ui.ctx()).theme;
    let g = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::new(size, FontFamily::Proportional),
        t.accent,
    );
    let (rect, r) = ui.allocate_exact_size(g.size(), Sense::click());
    ui.painter().galley(rect.min, g, t.accent);
    if r.hovered() {
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 1.0,
            Stroke::new(1.0, t.accent),
        );
    }
    r.on_hover_cursor(CursorIcon::PointingHand)
}

/// Breadcrumb trail; all but the last segment are links. Returns the clicked index.
pub fn breadcrumb(ui: &mut Ui, crumbs: &[&str]) -> Option<usize> {
    let t = look(ui.ctx()).theme;
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (i, crumb) in crumbs.iter().enumerate() {
            if i + 1 == crumbs.len() {
                label(ui, crumb, 12.5, FontFamily::Proportional, t.ink);
            } else {
                let colour = if i == 0 { t.accent } else { t.ink5 };
                let r = {
                    let g = ui.painter().layout_no_wrap(
                        (*crumb).to_owned(),
                        FontId::proportional(12.5),
                        colour,
                    );
                    let (rect, r) = ui.allocate_exact_size(g.size(), Sense::click());
                    ui.painter().galley(rect.min, g, colour);
                    r.on_hover_cursor(CursorIcon::PointingHand)
                };
                if r.clicked() {
                    clicked = Some(i);
                }
                label(ui, "/", 12.5, FontFamily::Proportional, t.ink5);
            }
        }
    });
    clicked
}

/// Section heading: 14/600 title, optional muted subtitle.
pub fn section_header(ui: &mut Ui, text: &str, sub: Option<&str>) {
    ui.horizontal(|ui| {
        title(ui, text, 14.0);
        if let Some(sub) = sub {
            let c = look(ui.ctx()).theme.ink5;
            label(ui, sub, 12.0, FontFamily::Proportional, c);
        }
    });
}

/// Square-ish icon button with a hover wash (close, overflow).
pub fn icon_button(ui: &mut Ui, name: &str, size: f32, bordered: bool) -> Response {
    let l = look(ui.ctx());
    let t = &l.theme;
    let (rect, r) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let fill = if r.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(7), fill);
    if bordered {
        p.rect_stroke(
            rect,
            CornerRadius::same(7),
            Stroke::new(1.0, t.border_strong),
            StrokeKind::Inside,
        );
    }
    let style = IconStyle {
        tint: t.ink3,
        bg: if r.hovered() { t.hover } else { t.surface },
        grey: false,
    };
    icon::paint(
        p,
        Rect::from_center_size(rect.center(), vec2(16.0, 16.0)),
        name,
        l.dark,
        style,
        1.0,
    );
    r.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn paint_icon(p: &Painter, rect: Rect, name: &str, bg: Color32, tint: Color32) {
    let dark = look(p.ctx()).dark;
    icon::paint(
        p,
        rect,
        name,
        dark,
        IconStyle {
            tint,
            bg,
            grey: false,
        },
        1.0,
    );
}

/// Four-button view strip floating over a preview (fit, iso, front, top).
pub fn view_strip(ui: &mut Ui, preview: Rect, selected: &mut usize) {
    let l = look(ui.ctx());
    let t = &l.theme;
    let strip = Rect::from_min_size(
        pos2(
            preview.right() - 10.0 - 134.0,
            preview.bottom() - 10.0 - 36.0,
        ),
        vec2(134.0, 36.0),
    );
    let p = ui.painter();
    p.rect_filled(strip, CornerRadius::same(8), t.surface);
    p.rect_stroke(
        strip,
        CornerRadius::same(8),
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
    for (i, name) in ["fit", "viewIso", "viewFront", "viewTop"]
        .into_iter()
        .enumerate()
    {
        let cell = Rect::from_min_size(
            strip.min + vec2(3.0 + i as f32 * 32.0, 3.0),
            vec2(30.0, 30.0),
        );
        let r = ui.interact(cell, ui.scope_id().with(("view", i)), Sense::click());
        let on = *selected == i;
        let fill = if on {
            t.sel_bg
        } else if r.hovered() {
            t.hover
        } else {
            t.surface
        };
        let p = ui.painter();
        p.rect_filled(cell, CornerRadius::same(6), fill);
        let tint = if on { t.sel_text } else { t.ink3 };
        paint_icon(
            p,
            Rect::from_center_size(cell.center(), vec2(18.0, 18.0)),
            name,
            fill,
            tint,
        );
        if r.clicked() {
            *selected = i;
        }
    }
}

const CARD_H: f32 = 214.0;
const THUMB_H: f32 = 128.0;

/// 08a card: thumbnail well, title, meta line and chips.
pub fn component_card(ui: &mut Ui, w: f32, c: &Component) -> Response {
    let l = look(ui.ctx());
    let t = &l.theme;
    let (rect, r) = ui.allocate_exact_size(vec2(w, CARD_H), Sense::click());
    let radius = CornerRadius::same(RADIUS_LG);
    let p = ui.painter();
    if r.hovered() {
        p.add(metrics::shadow(Elevation::Pill, t, l.dark).as_shape(rect, radius));
    }
    p.rect_filled(rect, radius, t.surface);
    let thumb = Rect::from_min_size(rect.min, vec2(w, THUMB_H));
    let top = CornerRadius {
        nw: RADIUS_LG,
        ne: RADIUS_LG,
        sw: 0,
        se: 0,
    };
    p.rect_filled(thumb, top, t.well);
    p.hline(
        rect.x_range(),
        thumb.bottom(),
        Stroke::new(1.0, t.border_subtle),
    );
    let icon = Rect::from_center_size(thumb.center(), vec2(64.0, 64.0));
    paint_icon(p, icon, c.icon, t.well, t.ink4);
    if c.update {
        paint_chip(
            p,
            t,
            thumb.min + vec2(10.0, 10.0),
            "v4 available",
            ChipKind::Warn,
        );
    }
    let x = rect.left() + 12.0;
    let max_w = w - 24.0;
    let g = fit(
        p,
        c.name,
        FontId::new(13.0, fonts::semibold()),
        t.ink,
        max_w,
    );
    p.galley(pos2(x, thumb.bottom() + 10.0), g, t.ink);
    let g = fit(p, c.meta, FontId::proportional(11.5), t.ink5, max_w);
    p.galley(pos2(x, thumb.bottom() + 31.0), g, t.ink5);
    let mut cx = x;
    for (text, kind) in c.chips {
        cx += paint_chip(p, t, pos2(cx, thumb.bottom() + 53.0), text, *kind) + 4.0;
    }
    let border = if r.hovered() {
        t.card_hover_border
    } else {
        t.border
    };
    p.rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    r.on_hover_cursor(CursorIcon::PointingHand)
}

/// 08a category tile: icon well, name, type count.
pub fn category_tile(ui: &mut Ui, w: f32, name: &str, count: &str, icon_name: &str) -> Response {
    let l = look(ui.ctx());
    let t = &l.theme;
    let (rect, r) = ui.allocate_exact_size(vec2(w, 64.0), Sense::click());
    let radius = CornerRadius::same(RADIUS_LG);
    let p = ui.painter();
    p.rect_filled(rect, radius, t.surface);
    let well = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.center().y - 20.0),
        vec2(40.0, 40.0),
    );
    p.rect_filled(well, CornerRadius::same(7), t.well);
    p.rect_stroke(
        well,
        CornerRadius::same(7),
        Stroke::new(1.0, t.border_subtle),
        StrokeKind::Inside,
    );
    paint_icon(
        p,
        Rect::from_center_size(well.center(), vec2(27.0, 27.0)),
        icon_name,
        t.well,
        t.ink4,
    );
    let x = well.right() + 12.0;
    let max_w = rect.right() - 8.0 - x;
    let g = fit(p, name, FontId::new(13.0, fonts::semibold()), t.ink, max_w);
    p.galley(pos2(x, rect.center().y - 17.0), g, t.ink);
    let g = fit(p, count, FontId::proportional(11.5), t.ink5, max_w);
    p.galley(pos2(x, rect.center().y + 3.0), g, t.ink5);
    let border = if r.hovered() {
        t.card_hover_border
    } else {
        t.border
    };
    p.rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    r.on_hover_cursor(CursorIcon::PointingHand)
}

/// Frame shared by the centred modals (sheet, wizard): r12, border, card shadow.
pub fn modal_frame(ctx: &egui::Context) -> egui::Frame {
    let l = look(ctx);
    egui::Frame::new()
        .fill(l.theme.surface)
        .corner_radius(CornerRadius::same(12))
        .stroke(Stroke::new(1.0, l.theme.border))
        .shadow(metrics::shadow(Elevation::Card, &l.theme, l.dark))
}
