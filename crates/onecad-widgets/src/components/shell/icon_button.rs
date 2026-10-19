//! Private square icon button shared by shell and overlay components.

use egui::{
    Color32, CornerRadius, Painter, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget,
};

use crate::components::tooltip::{self, Tip};
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::metrics::{self, Elevation};
use crate::theme::{Look, look};

/// What sits behind the glyph.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backdrop {
    /// Transparent until hovered (title bar, nav pill).
    None,
    /// Floating fill with a 1px border (view cluster).
    Floating,
}

pub(crate) struct IconButton<'a> {
    icon: &'a str,
    label: &'a str,
    size: f32,
    icon_px: f32,
    radius: u8,
    backdrop: Backdrop,
    on: bool,
    enabled: bool,
    shadow: Option<Elevation>,
    gap_bg: Option<Color32>,
    tip: Option<Tip<'a>>,
}

impl<'a> IconButton<'a> {
    pub(crate) fn new(icon: &'a str, label: &'a str, size: f32, icon_px: f32) -> Self {
        Self {
            icon,
            label,
            size,
            icon_px,
            radius: metrics::RADIUS_SM,
            backdrop: Backdrop::None,
            on: false,
            enabled: true,
            shadow: None,
            gap_bg: None,
            tip: None,
        }
    }
    pub(crate) fn radius(mut self, radius: u8) -> Self {
        self.radius = radius;
        self
    }
    pub(crate) fn backdrop(mut self, backdrop: Backdrop) -> Self {
        self.backdrop = backdrop;
        self
    }
    pub(crate) fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }
    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub(crate) fn shadow(mut self, shadow: Elevation) -> Self {
        self.shadow = Some(shadow);
        self
    }
    /// Colour behind the glyph when the button itself is transparent.
    pub(crate) fn gap_bg(mut self, bg: Color32) -> Self {
        self.gap_bg = Some(bg);
        self
    }
    pub(crate) fn tip(mut self, tip: Option<Tip<'a>>) -> Self {
        self.tip = tip;
        self
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), sense);
        let opacity = if self.enabled { 1.0 } else { 0.4 };
        let fill = self.paint_body(
            ui.painter(),
            rect,
            response.hovered() && self.enabled,
            opacity,
            &l,
        );
        self.paint_glyph(ui.painter(), rect, fill, opacity, response.has_focus(), &l);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, self.enabled, self.label)
        });
        match self.tip {
            Some(tip) => tooltip::attach(response, tip),
            None => response,
        }
    }
}

impl IconButton<'_> {
    /// Shadow, fill and border; returns the resolved (unfaded) fill.
    fn paint_body(
        &self,
        p: &Painter,
        rect: Rect,
        hovered: bool,
        opacity: f32,
        l: &Look,
    ) -> Color32 {
        let t = &l.theme;
        let radius = CornerRadius::same(self.radius);
        let base = match self.backdrop {
            Backdrop::Floating => t.floating,
            Backdrop::None => Color32::TRANSPARENT,
        };
        let fill = if self.on {
            t.sel_bg
        } else if hovered {
            t.hover
        } else {
            base
        };
        if let Some(e) = self.shadow {
            p.add(metrics::shadow(e, t, l.dark).as_shape(rect, radius));
        }
        let border = match (self.on, self.backdrop) {
            (true, _) => t.sel_border,
            (false, Backdrop::Floating) => t.border,
            (false, Backdrop::None) => Color32::TRANSPARENT,
        };
        p.rect(
            rect,
            radius,
            fill.gamma_multiply(opacity),
            Stroke::new(1.0, border.gamma_multiply(opacity)),
            StrokeKind::Inside,
        );
        fill
    }

    fn paint_glyph(
        &self,
        p: &Painter,
        rect: Rect,
        fill: Color32,
        opacity: f32,
        focus: bool,
        l: &Look,
    ) {
        let t = &l.theme;
        if focus {
            let (w, c) = metrics::focus_ring(t);
            p.rect_stroke(
                rect.expand(w * 0.5),
                CornerRadius::same(self.radius + 2),
                Stroke::new(w, c),
                StrokeKind::Outside,
            );
        }
        let glyph = Rect::from_center_size(rect.center(), Vec2::splat(self.icon_px));
        let gap = if fill.a() == 255 {
            fill
        } else {
            self.gap_bg.unwrap_or(t.surface)
        };
        let style = IconStyle {
            tint: if self.on { t.sel_text } else { t.ink4 },
            bg: gap,
            grey: !self.enabled,
        };
        let tint = style.tint.gamma_multiply(opacity);
        if glyph::by_name(p, self.icon, glyph.center(), self.icon_px, tint) {
            return;
        }
        icon::paint(p, glyph, self.icon, l.dark, style, opacity);
    }
}
