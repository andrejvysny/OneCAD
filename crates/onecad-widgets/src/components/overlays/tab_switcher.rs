//! TabSwitcher: Ctrl+Tab card grid over the window. Pure presentation; the host owns
//! keyboard handling (Tab / Shift+Tab / W / releasing Ctrl) and the selection.

use egui::{
    Align2, Color32, Context, CornerRadius, FontId, Frame, Id, Margin, Order, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use crate::components::actions::common::fitted;
use crate::fonts;
use crate::icon::{self, IconStyle};
use crate::metrics::{Elevation, RADIUS_MD, RADIUS_XL, SECTION, shadow};
use crate::theme::{Look, look};

const PAD: i8 = 16;
const GAP: f32 = 16.0;
const CARD_W: f32 = 168.0;
const PREVIEW_H: f32 = 106.0;
const TITLE_H: f32 = 20.0;
const CARD_H: f32 = PREVIEW_H + 8.0 + TITLE_H;
const FOOTER_H: f32 = 14.0;

pub struct SwitcherItem<'a> {
    pub title: &'a str,
    pub icon: &'a str,
    pub dirty: bool,
    /// Texture of the last rendered view; `None` shows the icon on a flat fill.
    pub thumbnail: Option<egui::TextureId>,
    /// The Home page: dashed outline, no fill.
    pub home: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabSwitcherOutput {
    pub clicked: Option<usize>,
    /// Area rect of the panel, for layout checks.
    pub rect: Rect,
}

pub struct TabSwitcher<'a> {
    items: &'a [SwitcherItem<'a>],
    selected: usize,
}

impl<'a> TabSwitcher<'a> {
    pub fn new(items: &'a [SwitcherItem<'a>], selected: usize) -> Self {
        Self { items, selected }
    }

    pub fn show(self, ctx: &Context, id: Id) -> TabSwitcherOutput {
        let l = look(ctx);
        let t = &l.theme;
        let screen = ctx.content_rect();
        // Panel padding on both sides is part of the 90% budget.
        let budget = screen.width() * 0.9 - 2.0 * f32::from(PAD);
        let per_row = (((budget + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
        let frame = Frame::new()
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.border_subtle))
            .corner_radius(CornerRadius::same(RADIUS_XL))
            .shadow(shadow(Elevation::Card, t, l.dark))
            .inner_margin(Margin::same(PAD));
        let mut clicked = None;
        let area = crate::frame::floating(id, screen.center(), Order::Foreground)
            .pivot(Align2::CENTER_CENTER);
        let resp = area.show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let cols = per_row.min(self.items.len()).max(1) as f32;
                let width = cols * CARD_W + (cols - 1.0) * GAP;
                ui.set_width(width);
                clicked = self.grid(ui, id, per_row, &l);
                ui.add_space(GAP);
                footer(ui, width, &l);
            });
        });
        TabSwitcherOutput {
            clicked,
            rect: resp.response.rect,
        }
    }

    fn grid(&self, ui: &mut Ui, id: Id, per_row: usize, l: &Look) -> Option<usize> {
        let mut clicked = None;
        for (row, chunk) in self.items.chunks(per_row).enumerate() {
            if row > 0 {
                ui.add_space(GAP);
            }
            let (strip, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), CARD_H), Sense::hover());
            for (col, item) in chunk.iter().enumerate() {
                let index = row * per_row + col;
                let min = strip.min + vec2(col as f32 * (CARD_W + GAP), 0.0);
                let rect = Rect::from_min_size(min, vec2(CARD_W, CARD_H));
                let r = ui.interact(rect, id.with(index), Sense::click());
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, item.title));
                card(ui, rect, item, index == self.selected, r.hovered(), l);
                if r.clicked() {
                    clicked = Some(index);
                }
            }
        }
        clicked
    }
}

fn card(ui: &Ui, rect: Rect, item: &SwitcherItem<'_>, selected: bool, hover: bool, l: &Look) {
    let t = &l.theme;
    let preview = Rect::from_min_size(rect.min, vec2(CARD_W, PREVIEW_H));
    let cr = CornerRadius::same(RADIUS_MD);
    paint_preview(ui, preview, cr, item, l);
    let border = if selected {
        Some(Stroke::new(2.0, t.accent))
    } else if hover {
        Some(Stroke::new(1.0, t.border_strong))
    } else if !item.home {
        Some(Stroke::new(1.0, t.border))
    } else {
        None
    };
    if let Some(s) = border {
        ui.painter().rect_stroke(preview, cr, s, StrokeKind::Inside);
    } else {
        dashed_outline(ui, preview, t.border_strong);
    }
    let title = Rect::from_min_size(
        pos2(rect.left(), preview.bottom() + 8.0),
        vec2(CARD_W, TITLE_H),
    );
    paint_title(ui, title, item, selected, l);
}

/// Texture (full UV; hosts render thumbnails at the card's 168:106 aspect), or the
/// icon on a flat fill. The Home card has no fill.
fn paint_preview(ui: &Ui, rect: Rect, cr: CornerRadius, item: &SwitcherItem<'_>, l: &Look) {
    let t = &l.theme;
    let p = ui.painter();
    if let (Some(tex), false) = (item.thumbnail, item.home) {
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        p.add(egui::epaint::RectShape::filled(rect, cr, Color32::WHITE).with_texture(tex, uv));
        return;
    }
    let (bg, size) = if item.home {
        (t.surface, 20.0)
    } else {
        p.rect_filled(rect, cr, t.well);
        (t.well, 40.0)
    };
    let style = IconStyle {
        tint: t.ink5,
        bg,
        grey: false,
    };
    let glyph = Rect::from_center_size(rect.center(), Vec2::splat(size));
    let name = if item.home { "home" } else { item.icon };
    icon::paint(p, glyph, name, l.dark, style, 1.0);
}

fn dashed_outline(ui: &Ui, rect: Rect, colour: Color32) {
    let pts = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    let stroke = Stroke::new(1.0, colour);
    ui.painter()
        .extend(Shape::dashed_line(&pts, stroke, 5.0, 4.0));
}

fn paint_title(ui: &Ui, rect: Rect, item: &SwitcherItem<'_>, selected: bool, l: &Look) {
    let t = &l.theme;
    let p = ui.painter();
    let colour = if selected { t.accent } else { t.ink2 };
    if selected {
        p.rect_filled(rect, CornerRadius::same(4), t.sel_soft);
    }
    let dot_w = if item.dirty { 6.0 + 6.0 } else { 0.0 };
    let max_w = rect.width() - 12.0 - dot_w;
    let font = FontId::new(13.0, fonts::medium());
    let galley = fitted(p, item.title, font, colour, max_w);
    let x = rect.left() + 6.0;
    let w = galley.size().x;
    p.galley(
        pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        colour,
    );
    if item.dirty {
        p.circle_filled(pos2(x + w + 6.0 + 3.0, rect.center().y), 3.0, t.ink4);
    }
}

fn footer(ui: &mut Ui, width: f32, l: &Look) {
    let t = &l.theme;
    let (rect, _) = ui.allocate_exact_size(vec2(width, FOOTER_H), Sense::hover());
    let p = ui.painter();
    let font = FontId::proportional(SECTION);
    let mut x = rect.left();
    for hint in ["Tab next", "⇧Tab previous", "W close"] {
        let g = p.layout_no_wrap(hint.to_owned(), font.clone(), t.ink5);
        let w = g.size().x;
        p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, t.ink5);
        x += w + GAP;
    }
    let g = p.layout_no_wrap("Release ⌃ to open".to_owned(), font, t.ink5);
    let pos = pos2(
        rect.right() - g.size().x,
        rect.center().y - g.size().y / 2.0,
    );
    p.galley(pos, g, t.ink5);
}
