//! Button (c-button): text action, h30 · r6 · px12 · 12.5 (primary 600) · icon 14.
//! Shared by every card and strip; one primary per surface.

use egui::{Color32, CornerRadius, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget};

use crate::components::tooltip::{self, Tip};
use crate::fonts;
use crate::icon::{self, IconStyle};
use crate::metrics::{H_FIELD, RADIUS_SM};
use crate::theme::{Theme, look};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    #[default]
    Secondary,
    Ghost,
    Danger,
}

pub struct Button<'a> {
    label: &'a str,
    kind: ButtonKind,
    icon: Option<&'a str>,
    enabled: bool,
    tip: Option<Tip<'a>>,
    min_width: f32,
}

impl<'a> Button<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            kind: ButtonKind::default(),
            icon: None,
            enabled: true,
            tip: None,
            min_width: 0.0,
        }
    }
    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn icon(mut self, name: Option<&'a str>) -> Self {
        self.icon = name;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn tip(mut self, tip: Tip<'a>) -> Self {
        self.tip = Some(tip);
        self
    }
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
}

/// (fill, text, border) for a kind in a pointer state.
fn colours(t: &Theme, kind: ButtonKind, hovered: bool) -> (Color32, Color32, Color32) {
    match kind {
        ButtonKind::Primary if hovered => (t.accent_hover, t.on_accent, t.accent_hover),
        ButtonKind::Primary => (t.accent, t.on_accent, t.accent),
        ButtonKind::Secondary => (
            if hovered { t.hover } else { t.surface },
            t.ink2,
            t.border_strong,
        ),
        ButtonKind::Ghost => (
            if hovered {
                t.hover
            } else {
                Color32::TRANSPARENT
            },
            t.ink3,
            Color32::TRANSPARENT,
        ),
        ButtonKind::Danger => (
            if hovered { t.danger_surface } else { t.surface },
            t.danger_strong,
            t.danger_border,
        ),
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let family = if self.kind == ButtonKind::Primary {
            fonts::semibold()
        } else {
            egui::FontFamily::Proportional
        };
        let galley = ui.painter().layout_no_wrap(
            self.label.to_owned(),
            egui::FontId::new(12.5, family),
            Color32::PLACEHOLDER,
        );
        let icon_w = if self.icon.is_some() { 14.0 + 6.0 } else { 0.0 };
        let width = (galley.size().x + icon_w + 24.0).max(self.min_width);
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, H_FIELD), sense);
        let (fill, text, border) = colours(&l.theme, self.kind, response.hovered() && self.enabled);
        let opacity = if self.enabled { 1.0 } else { 0.4 };
        let p = ui.painter();
        p.rect(
            rect,
            CornerRadius::same(RADIUS_SM),
            fill.gamma_multiply(opacity),
            Stroke::new(1.0, border.gamma_multiply(opacity)),
            StrokeKind::Inside,
        );
        if response.has_focus() {
            let (w, c) = crate::metrics::focus_ring(&l.theme);
            p.rect_stroke(
                rect.expand(w * 0.5),
                CornerRadius::same(RADIUS_SM + 2),
                Stroke::new(w, c),
                StrokeKind::Outside,
            );
        }
        let mut x = rect.center().x - (galley.size().x + icon_w) / 2.0;
        if let Some(name) = self.icon {
            let r =
                egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 7.0), Vec2::splat(14.0));
            icon::paint(
                p,
                r,
                name,
                l.dark,
                IconStyle {
                    tint: text,
                    bg: fill,
                    grey: !self.enabled,
                },
                opacity,
            );
            x += icon_w;
        }
        p.galley(
            egui::pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            text.gamma_multiply(opacity),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, self.enabled, self.label)
        });
        match self.tip {
            Some(tip) => tooltip::attach(response, tip),
            None => response,
        }
    }
}
