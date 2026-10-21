//! FloatingPanel (c-panel) and PanelToggle (c-ptoggle).

use egui::{
    Color32, Context, CornerRadius, Order, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
    vec2,
};

use crate::components::tooltip::{self, Tip};
use crate::frame;
use crate::icon::{self, IconStyle};
use crate::metrics::{self, Elevation, RADIUS_XL};
use crate::theme::{Look, look};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

const ANIM_SECS: f32 = 0.16;
const SLIDE: f32 = 12.0;

/// Frame for Explorer / Inspector. The caller computes `rect` (README: Explorer 248 wide
/// at left 10 / top 62 / bottom 10; Inspector 288 at right 10 / top 10 / bottom 10).
pub struct FloatingPanel {
    id: egui::Id,
    side: Side,
    open: bool,
    rect: Rect,
}

impl FloatingPanel {
    pub fn new(id: egui::Id, side: Side) -> Self {
        Self {
            id,
            side,
            open: true,
            rect: Rect::from_min_size(egui::Pos2::ZERO, vec2(248.0, 400.0)),
        }
    }
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    pub fn rect(mut self, rect: Rect) -> Self {
        self.rect = rect;
        self
    }

    /// `None` once fully closed. Content decides its own padding.
    pub fn show(self, ctx: &Context, add: impl FnOnce(&mut Ui)) -> Option<Response> {
        let t = ctx.animate_bool_with_time_and_easing(
            self.id.with("anim"),
            self.open,
            ANIM_SECS,
            egui::emath::easing::cubic_out,
        );
        if t <= 0.0 {
            return None;
        }
        let dir = if self.side == Side::Left { -1.0 } else { 1.0 };
        let rect = self.rect.translate(vec2(dir * SLIDE * (1.0 - t), 0.0));
        let l = look(ctx);
        // Fade comes only from `ui.set_opacity` below; scaling the frame colours too
        // would apply alpha twice.
        let frame = frame::surface(&l.theme, l.dark, RADIUS_XL, Elevation::Panel);
        let inner = rect.size() - Vec2::splat(2.0);
        let area = frame::floating(self.id, rect.min, Order::Middle).show(ctx, |ui| {
            ui.set_opacity(t);
            frame.show(ui, |ui| {
                ui.set_min_size(inner);
                ui.set_max_size(inner);
                ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
                add(ui);
            });
        });
        Some(area.response)
    }
}

/// 44×44 r12 toggle for a side panel (compact: in-panel 32×32 r7).
pub struct PanelToggle<'a> {
    side: Side,
    open: bool,
    dot: bool,
    compact: bool,
    tip: Option<Tip<'a>>,
    icon: Option<&'a str>,
}

impl<'a> PanelToggle<'a> {
    pub fn new(side: Side) -> Self {
        Self {
            side,
            open: false,
            dot: false,
            compact: false,
            tip: None,
            icon: None,
        }
    }
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    /// "Selection waiting" marker on a hidden panel.
    pub fn dot(mut self, dot: bool) -> Self {
        self.dot = dot;
        self
    }
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }
    /// Replaces the default panel glyph, e.g. when the toggle names what the panel holds.
    pub fn icon(mut self, name: &'a str) -> Self {
        self.icon = Some(name);
        self
    }
    pub fn tip(mut self, tip: Tip<'a>) -> Self {
        self.tip = Some(tip);
        self
    }
}

impl egui::Widget for PanelToggle<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (size, radius) = if self.compact { (32.0, 7) } else { (44.0, 12) };
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
        let cr = CornerRadius::same(radius);
        let (fill, border, tint) = if self.open {
            (t.sel_bg, t.sel_border, t.sel_text)
        } else {
            let f = if response.hovered() {
                t.hover
            } else {
                t.floating
            };
            (f, t.border, t.ink3)
        };
        let p = ui.painter();
        if !self.compact {
            p.add(metrics::shadow(Elevation::Pill, t, l.dark).as_shape(rect, cr));
        }
        p.rect(rect, cr, fill, Stroke::new(1.0, border), StrokeKind::Inside);
        if response.has_focus() {
            let (w, c) = metrics::focus_ring(t);
            p.rect_stroke(
                rect.expand(w * 0.5),
                CornerRadius::same(radius + 2),
                Stroke::new(w, c),
                StrokeKind::Outside,
            );
        }
        self.paint_glyph(p, rect, fill, tint, &l);
        let label = match self.side {
            Side::Left => "Explorer",
            Side::Right => "Inspector",
        };
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, self.open, label));
        match self.tip {
            Some(tip) => tooltip::attach(response, tip),
            None => response,
        }
    }
}

impl PanelToggle<'_> {
    fn paint_glyph(&self, p: &egui::Painter, rect: Rect, fill: Color32, tint: Color32, l: &Look) {
        let t = &l.theme;
        let name = self.icon.unwrap_or(match (self.side, self.open) {
            (Side::Left, false) => "panelLeft",
            (Side::Left, true) => "panelLeftOpen",
            (Side::Right, false) => "panelRight",
            (Side::Right, true) => "panelRightOpen",
        });
        let glyph_px = if self.compact { 18.0 } else { 20.0 };
        let glyph = Rect::from_center_size(rect.center(), Vec2::splat(glyph_px));
        let style = IconStyle {
            tint,
            bg: if fill.a() == 255 { fill } else { t.surface },
            grey: false,
        };
        icon::paint(p, glyph, name, l.dark, style, 1.0);
        if self.dot {
            let c = egui::pos2(rect.right() - 7.0 - 3.5, rect.top() + 7.0 + 3.5);
            p.circle_filled(c, 5.5, t.surface);
            p.circle_filled(c, 3.5, t.accent);
        }
    }
}
