//! MenuList (c-menu): popover frame, section labels, rows with icon + shortcut, dividers.

use egui::{
    Color32, CornerRadius, Frame, Margin, Rect, Response, Sense, Stroke, Ui, Widget, pos2, vec2,
};

use crate::components::actions::common::{focus_ring, galley, opacity, sense};
use crate::fonts;
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::metrics::{CONTROL, Elevation, RADIUS_SM, RADIUS_XL, SECTION, shadow};
use crate::theme::look;

/// Popover surface for menus and select dropdowns: floating fill, border, r12, card shadow.
pub fn menu_frame(ctx: &egui::Context) -> Frame {
    let l = look(ctx);
    Frame::new()
        .fill(l.theme.floating)
        .stroke(Stroke::new(1.0, l.theme.border))
        .corner_radius(CornerRadius::same(RADIUS_XL))
        .inner_margin(Margin::same(6))
        .shadow(shadow(Elevation::Card, &l.theme, l.dark))
}

pub struct MenuSection<'a> {
    text: &'a str,
}

impl<'a> MenuSection<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }
}

impl Widget for MenuSection<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        super::section_label(ui, self.text, 8.0)
    }
}

pub struct MenuRow<'a> {
    icon: Option<&'a str>,
    label: &'a str,
    shortcut: Option<&'a str>,
    enabled: bool,
    checked: bool,
}

impl<'a> MenuRow<'a> {
    pub fn new(icon: Option<&'a str>, label: &'a str) -> Self {
        Self {
            icon,
            label,
            shortcut: None,
            enabled: true,
            checked: false,
        }
    }
    pub fn shortcut(mut self, key: Option<&'a str>) -> Self {
        self.shortcut = key;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// Draws a check at the right edge and the label at full ink.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
}

impl Widget for MenuRow<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (rect, response) = ui.allocate_exact_size(
            vec2(ui.available_width(), 30.0),
            sense(self.enabled, Sense::click()),
        );
        let op = opacity(self.enabled);
        let fill = if response.hovered() && self.enabled {
            t.well
        } else {
            Color32::TRANSPARENT
        };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
        if response.has_focus() {
            focus_ring(p, t, rect, RADIUS_SM);
        }
        let cy = rect.center().y;
        let mut x = rect.left() + 8.0;
        if let Some(name) = self.icon {
            let r = Rect::from_center_size(pos2(x + 10.0, cy), vec2(20.0, 20.0));
            let style = IconStyle {
                tint: t.ink3,
                bg: t.floating,
                grey: !self.enabled,
            };
            icon::paint(p, r, name, l.dark, style, op);
            x += 20.0 + 10.0;
        }
        let mut right = rect.right() - 8.0;
        if self.checked {
            let r = Rect::from_center_size(pos2(right - 7.0, cy), vec2(14.0, 14.0));
            glyph::check(p, r.center(), 14.0, t.accent.gamma_multiply(op));
            right -= 14.0 + 8.0;
        }
        if let Some(key) = self.shortcut {
            let g = galley(p, key, SECTION, fonts::mono());
            p.galley(
                pos2(right - g.size().x, cy - g.size().y / 2.0),
                g,
                t.ink6.gamma_multiply(op),
            );
        }
        let colour = if self.checked { t.ink } else { t.ink2 };
        let g = galley(p, self.label, CONTROL, egui::FontFamily::Proportional);
        p.galley(pos2(x, cy - g.size().y / 2.0), g, colour.gamma_multiply(op));
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::MenuItem, self.enabled, self.checked, self.label)
        });
        response
    }
}

/// 1px divider with 4px vertical and 6px horizontal margins.
pub fn menu_divider(ui: &mut Ui) {
    let color = look(ui.ctx()).theme.border_subtle;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0 + 8.0), Sense::hover());
    let y = rect.center().y;
    ui.painter().rect_filled(
        Rect::from_min_max(
            pos2(rect.left() + 6.0, y - 0.5),
            pos2(rect.right() - 6.0, y + 0.5),
        ),
        0.0,
        color,
    );
}
