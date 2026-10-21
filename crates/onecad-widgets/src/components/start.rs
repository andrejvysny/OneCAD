//! Start screen parts (g06): SidebarNavItem, ActionTile, RecentCard.
//! Cards c-navitem, c-tile, c-recent.

use egui::{
    Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, vec2,
};

use crate::components::actions::common::fitted;
use crate::fonts;
use crate::icon::{self, IconStyle};
use crate::metrics::{self, Elevation, RADIUS_LG, RADIUS_SM};
use crate::theme::{Look, look};

fn focus_ring(ui: &Ui, rect: Rect, radius: u8, l: &Look) {
    let (w, c) = metrics::focus_ring(&l.theme);
    ui.painter().rect_stroke(
        rect.expand(w * 0.5),
        CornerRadius::same(radius + 2),
        Stroke::new(w, c),
        StrokeKind::Outside,
    );
}

fn sense(enabled: bool) -> Sense {
    if enabled {
        Sense::click()
    } else {
        Sense::hover()
    }
}

/// Start-screen navigation row: h32 r6, icon 18, 13px; active uses selection colours.
pub struct SidebarNavItem<'a> {
    icon: &'a str,
    label: &'a str,
    active: bool,
    enabled: bool,
    badge: Option<&'a str>,
    sub: bool,
}

impl<'a> SidebarNavItem<'a> {
    pub fn new(icon: &'a str, label: &'a str) -> Self {
        Self {
            icon,
            label,
            active: false,
            enabled: true,
            badge: None,
            sub: false,
        }
    }
    /// Accent count circle at the row's right edge.
    pub fn badge(mut self, badge: Option<&'a str>) -> Self {
        self.badge = badge;
        self
    }
    /// Indented child row: h28, no icon, label aligned with the parent label.
    // Builder name fixed by the design API; it is not `std::ops::Sub`.
    #[allow(clippy::should_implement_trait)]
    pub fn sub(mut self, sub: bool) -> Self {
        self.sub = sub;
        self
    }
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl egui::Widget for SidebarNavItem<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let height = if self.sub { 28.0 } else { 32.0 };
        let (rect, r) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), sense(self.enabled));
        let opacity = if self.enabled { 1.0 } else { 0.4 };
        let fill = if self.active {
            t.sel_bg
        } else if r.hovered() && self.enabled {
            t.hover
        } else {
            Color32::TRANSPARENT
        };
        let (ink, family) = if self.active {
            (t.sel_text, fonts::medium())
        } else {
            (t.ink2, egui::FontFamily::Proportional)
        };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
        let glyph = Rect::from_center_size(
            Pos2::new(rect.left() + 10.0 + 9.0, rect.center().y),
            Vec2::splat(18.0),
        );
        let gap = if fill.a() == 255 { fill } else { t.panel };
        let style = IconStyle {
            tint: ink,
            bg: gap,
            grey: !self.enabled,
        };
        if !self.sub {
            icon::paint(p, glyph, self.icon, l.dark, style, opacity);
        }
        let g = p.layout_no_wrap(
            self.label.to_owned(),
            FontId::new(metrics::BODY, family),
            Color32::PLACEHOLDER,
        );
        let pos = Pos2::new(glyph.right() + 10.0, rect.center().y - g.size().y / 2.0);
        p.galley(pos, g, ink.gamma_multiply(opacity));
        if let Some(badge) = self.badge {
            let circle = Rect::from_center_size(
                Pos2::new(rect.right() - 10.0 - 9.0, rect.center().y),
                Vec2::splat(18.0),
            );
            p.circle_filled(circle.center(), 9.0, t.accent.gamma_multiply(opacity));
            p.text(
                circle.center(),
                egui::Align2::CENTER_CENTER,
                badge,
                FontId::new(metrics::SECTION, fonts::semibold()),
                t.on_accent,
            );
        }
        if r.has_focus() {
            focus_ring(ui, rect, RADIUS_SM, &l);
        }
        r.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, self.enabled, self.active, self.label)
        });
        r
    }
}

/// Large create/open entry: h92 r10, icon well 52. Primary is accent-filled.
pub struct ActionTile<'a> {
    icon: &'a str,
    title: &'a str,
    subtitle: &'a str,
    primary: bool,
    enabled: bool,
}

impl<'a> ActionTile<'a> {
    pub fn new(icon: &'a str, title: &'a str, subtitle: &'a str) -> Self {
        Self {
            icon,
            title,
            subtitle,
            primary: false,
            enabled: true,
        }
    }
    pub fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl egui::Widget for ActionTile<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (rect, r) =
            ui.allocate_exact_size(vec2(ui.available_width(), 92.0), sense(self.enabled));
        let hovered = r.hovered() && self.enabled;
        let opacity = if self.enabled { 1.0 } else { 0.4 };
        let cr = CornerRadius::same(RADIUS_LG);
        let (fill, border, title, sub) = if self.primary {
            let f = if hovered { t.accent_hover } else { t.accent };
            (f, f, t.on_accent, t.on_accent.gamma_multiply(0.85))
        } else {
            let b = if hovered {
                t.card_hover_border
            } else {
                t.border
            };
            (t.surface, b, t.ink, t.ink4)
        };
        let p = ui.painter();
        if hovered {
            p.add(metrics::shadow(Elevation::Pill, t, l.dark).as_shape(rect, cr));
        }
        p.rect(
            rect,
            cr,
            fill.gamma_multiply(opacity),
            Stroke::new(1.0, border.gamma_multiply(opacity)),
            StrokeKind::Inside,
        );
        let well = self.paint_well(p, rect, opacity, &l);
        let x = well.right() + 14.0;
        let colours = (title.gamma_multiply(opacity), sub.gamma_multiply(opacity));
        self.paint_text(p, x, rect.center().y, colours);
        if r.has_focus() {
            focus_ring(ui, rect, RADIUS_LG, &l);
        }
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, self.enabled, self.title));
        r
    }
}

impl ActionTile<'_> {
    /// Icon well; returns its rect.
    fn paint_well(&self, p: &egui::Painter, rect: Rect, opacity: f32, l: &Look) -> Rect {
        let t = &l.theme;
        let well = Rect::from_min_size(
            Pos2::new(rect.left() + 16.0, rect.center().y - 26.0),
            Vec2::splat(52.0),
        );
        let well_fill = if self.primary { t.surface } else { t.chip };
        p.rect_filled(
            well,
            CornerRadius::same(RADIUS_LG),
            well_fill.gamma_multiply(opacity),
        );
        let style = IconStyle {
            tint: t.ink4,
            bg: well_fill,
            grey: !self.enabled,
        };
        let glyph = Rect::from_center_size(well.center(), Vec2::splat(36.0));
        icon::paint(p, glyph, self.icon, l.dark, style, opacity);
        well
    }

    fn paint_text(&self, p: &egui::Painter, x: f32, cy: f32, (title, sub): (Color32, Color32)) {
        let gt = p.layout_no_wrap(
            self.title.to_owned(),
            FontId::new(14.0, fonts::semibold()),
            title,
        );
        let gs = p.layout_no_wrap(
            self.subtitle.to_owned(),
            FontId::proportional(metrics::SECONDARY),
            sub,
        );
        let top = cy - (gt.size().y + 3.0 + gs.size().y) / 2.0;
        let th = gt.size().y;
        p.galley(Pos2::new(x, top), gt, title);
        p.galley(Pos2::new(x, top + th + 3.0), gs, sub);
    }
}

/// Project card: r10, 150px thumbnail, name 13/600, meta 11.5. Hover lifts.
pub struct RecentCard<'a> {
    title: &'a str,
    meta: &'a str,
    thumbnail: Option<egui::TextureId>,
    width: f32,
    metadata_only: bool,
    open: bool,
}

const THUMB_H: f32 = 150.0;
const CAPTION_H: f32 = 54.0;

impl<'a> RecentCard<'a> {
    pub fn new(title: &'a str, meta: &'a str) -> Self {
        Self {
            title,
            meta,
            thumbnail: None,
            width: 200.0,
            metadata_only: false,
            open: false,
        }
    }
    /// Marks a recent that is already open in a tab; opening it switches there.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    /// Show only real document metadata when no checked thumbnail is available.
    pub fn metadata_only(mut self) -> Self {
        self.metadata_only = true;
        self
    }
    pub fn thumbnail(mut self, tex: Option<egui::TextureId>) -> Self {
        self.thumbnail = tex;
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
}

/// "Open" pill in the card's top-right corner.
fn paint_open_badge(p: &egui::Painter, card: Rect, l: &crate::theme::Look) {
    let t = &l.theme;
    let font = egui::FontId::proportional(11.0);
    let text = p.layout_no_wrap("Open".into(), font, t.accent);
    let size = text.size() + vec2(12.0, 6.0);
    let badge = Rect::from_min_size(
        Pos2::new(card.right() - 10.0 - size.x, card.top() + 8.0),
        size,
    );
    p.rect(
        badge,
        CornerRadius::same(4),
        t.surface,
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
    p.galley(badge.min + vec2(6.0, 3.0), text, t.accent);
}

impl egui::Widget for RecentCard<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let size = vec2(
            self.width,
            if self.metadata_only {
                CAPTION_H
            } else {
                THUMB_H + 1.0 + CAPTION_H
            },
        );
        let (rect, r) = ui.allocate_exact_size(size, Sense::click());
        let cr = CornerRadius::same(RADIUS_LG);
        let hovered = r.hovered();
        let p = ui.painter();
        if hovered {
            p.add(metrics::shadow(Elevation::CardHover, t, l.dark).as_shape(rect, cr));
        }
        let border = if hovered {
            t.card_hover_border
        } else {
            t.border
        };
        p.rect(
            rect,
            cr,
            t.surface,
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );
        if self.metadata_only {
            self.paint_caption(p, rect.min + vec2(12.0, 10.0), &l);
        } else {
            let thumb =
                Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(self.width - 2.0, THUMB_H));
            let top = CornerRadius {
                nw: RADIUS_LG - 1,
                ne: RADIUS_LG - 1,
                sw: 0,
                se: 0,
            };
            paint_thumb(ui, thumb, top, self.thumbnail, &l);
            let line_y = thumb.bottom();
            let p = ui.painter();
            p.hline(
                thumb.x_range(),
                line_y + 0.5,
                Stroke::new(1.0, t.border_subtle),
            );
            self.paint_caption(p, Pos2::new(rect.left() + 12.0, line_y + 1.0 + 10.0), &l);
        }
        if self.open {
            paint_open_badge(ui.painter(), rect, &l);
        }
        if r.has_focus() {
            focus_ring(ui, rect, RADIUS_LG, &l);
        }
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, self.title));
        r
    }
}

impl RecentCard<'_> {
    /// Title and meta, elided to the card's inner width (12px padding each side).
    fn paint_caption(&self, p: &egui::Painter, pos: Pos2, l: &Look) {
        let t = &l.theme;
        let max_w = self.width - 24.0;
        let font = FontId::new(metrics::BODY, fonts::semibold());
        let gt = fitted(p, self.title, font, t.ink, max_w);
        let gm = fitted(p, self.meta, FontId::proportional(11.5), t.ink5, max_w);
        let th = gt.size().y;
        p.galley(pos, gt, t.ink);
        p.galley(Pos2::new(pos.x, pos.y + th + 3.0), gm, t.ink5);
    }
}

/// Texture, or a flat canvas with faint hatching when the document has no preview.
fn paint_thumb(ui: &Ui, rect: Rect, cr: CornerRadius, tex: Option<egui::TextureId>, l: &Look) {
    let t = &l.theme;
    let p = ui.painter();
    match tex {
        Some(id) => {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            p.add(egui::epaint::RectShape::filled(rect, cr, Color32::WHITE).with_texture(id, uv));
        }
        None => {
            p.rect_filled(rect, cr, t.canvas_model);
            let p = p.with_clip_rect(rect);
            let stroke = Stroke::new(1.0, t.grid_minor);
            let mut x = rect.left() - rect.height();
            while x < rect.right() {
                p.line_segment(
                    [
                        Pos2::new(x, rect.bottom()),
                        Pos2::new(x + rect.height(), rect.top()),
                    ],
                    stroke,
                );
                x += 14.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Widget;

    fn run(dark: bool, mut f: impl FnMut(&mut Ui)) {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, crate::theme::load(dark).unwrap(), dark);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| f(ui));
        out.textures_delta.clear();
    }

    #[test]
    fn metadata_card_is_bounded_and_clickable_without_thumbnail() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, crate::theme::load(true).unwrap(), true);
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 200.0))),
            ..Default::default()
        };
        let mut bounds = Rect::NOTHING;
        let mut first = ctx.run_ui(raw.clone(), |ui| {
            bounds = ui
                .add(
                    RecentCard::new("Actual part", "Recovery available")
                        .width(220.0)
                        .metadata_only(),
                )
                .rect;
        });
        first.textures_delta.clear();
        assert_eq!(bounds.size(), vec2(220.0, CAPTION_H));
        let pos = bounds.center();
        let mut click = raw;
        click.events = vec![egui::Event::PointerMoved(pos)];
        for pressed in [true, false] {
            click.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let mut clicked = false;
        let mut second = ctx.run_ui(click, |ui| {
            clicked = ui
                .add(
                    RecentCard::new("Actual part", "Recovery available")
                        .width(220.0)
                        .metadata_only(),
                )
                .clicked();
        });
        second.textures_delta.clear();
        assert!(clicked);
    }

    #[test]
    fn start_parts_render_light_and_dark() {
        for dark in [false, true] {
            run(dark, |ui| {
                let r = SidebarNavItem::new("home", "Home").active(true).ui(ui);
                assert_eq!(r.rect.height(), 32.0);
                SidebarNavItem::new("recent", "Recent")
                    .enabled(false)
                    .ui(ui);
                let r = ActionTile::new("body", "New part", "⌘N")
                    .primary(true)
                    .ui(ui);
                assert_eq!(r.rect.height(), 92.0);
                ActionTile::new("import", "Open", "STEP").ui(ui);
                let r = RecentCard::new("Bracket", "12 min ago").width(200.0).ui(ui);
                assert_eq!(r.rect.width(), 200.0);
            });
        }
    }
}
