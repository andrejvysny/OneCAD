//! ValueTag + HUD (c-valtag): live value at a drag handle; HUD adds operation + confirm.

use egui::{
    Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2, vec2,
};

use crate::fonts;
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::metrics::{self, Elevation, RADIUS_LG, RADIUS_SM};
use crate::theme::{Look, Theme, look};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValueTagTone {
    /// Orange: a value being created.
    #[default]
    Created,
    /// Accent: a value modifying existing geometry.
    Modify,
}

pub struct ValueTag<'a> {
    text: &'a str,
    tone: ValueTagTone,
    hud_icon: Option<&'a str>,
}

impl<'a> ValueTag<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            tone: ValueTagTone::default(),
            hud_icon: None,
        }
    }
    pub fn tone(mut self, tone: ValueTagTone) -> Self {
        self.tone = tone;
        self
    }
    /// HUD variant: operation `icon` plus a confirm tick. The widget's response is the
    /// confirm button's.
    pub fn hud(mut self, icon: &'a str) -> Self {
        self.hud_icon = Some(icon);
        self
    }

    fn colours(&self, l: &Look) -> (Color32, Color32) {
        let t = &l.theme;
        match self.tone {
            ValueTagTone::Created => (t.created_stroke, t.created_text),
            ValueTagTone::Modify => (t.accent, t.sel_text),
        }
    }
}

impl egui::Widget for ValueTag<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let (stroke, text) = self.colours(&l);
        match self.hud_icon {
            Some(icon) => hud(ui, &self, icon, (stroke, text), &l),
            None => tag(ui, self.text, (stroke, text), &l),
        }
    }
}

fn tag(ui: &mut Ui, text: &str, (stroke, ink): (Color32, Color32), l: &Look) -> Response {
    let g = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::new(metrics::SECONDARY, fonts::mono()),
        ink,
    );
    let (rect, r) = ui.allocate_exact_size(vec2(g.size().x + 16.0, 26.0), Sense::hover());
    let cr = CornerRadius::same(RADIUS_SM);
    let p = ui.painter();
    p.add(metrics::shadow(Elevation::Tooltip, &l.theme, l.dark).as_shape(rect, cr));
    p.rect(
        rect,
        cr,
        l.theme.surface,
        Stroke::new(1.5, stroke),
        StrokeKind::Inside,
    );
    p.galley(rect.center() - g.size() / 2.0, g, ink);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    r
}

/// Splits "14.00 mm" into the number and its unit.
fn split_unit(text: &str) -> (&str, &str) {
    text.rsplit_once(' ').unwrap_or((text, ""))
}

fn hud(
    ui: &mut Ui,
    tag: &ValueTag,
    op_icon: &str,
    (stroke, ink): (Color32, Color32),
    l: &Look,
) -> Response {
    let t = &l.theme;
    let (rect, _) = ui.allocate_exact_size(
        vec2(4.0 + 90.0 + 4.0 + 30.0 + 4.0 + 30.0 + 4.0, 40.0),
        Sense::hover(),
    );
    let cr = CornerRadius::same(RADIUS_LG);
    let p = ui.painter();
    p.rect(
        rect,
        cr,
        t.surface,
        Stroke::new(1.5, stroke),
        StrokeKind::Inside,
    );
    let cy = rect.center().y;
    let value = Rect::from_min_size(Pos2::new(rect.left() + 4.0, cy - 15.0), vec2(90.0, 30.0));
    hud_value(p, value, tag.text, ink, t);
    let op = Rect::from_min_size(Pos2::new(value.right() + 4.0, cy - 15.0), Vec2::splat(30.0));
    p.rect_filled(op, CornerRadius::same(RADIUS_SM), t.sel_bg);
    let style = IconStyle {
        tint: t.sel_text,
        bg: t.sel_bg,
        grey: false,
    };
    let glyph = Rect::from_center_size(op.center(), Vec2::splat(20.0));
    icon::paint(p, glyph, op_icon, l.dark, style, 1.0);
    let ok = Rect::from_min_size(Pos2::new(op.right() + 4.0, cy - 15.0), Vec2::splat(30.0));
    hud_confirm(ui, ok, l)
}

/// Warn-tinted value well: number left, dimmed unit right.
fn hud_value(p: &Painter, value: Rect, text: &str, ink: Color32, t: &Theme) {
    let cy = value.center().y;
    p.rect_filled(value, CornerRadius::same(RADIUS_SM), t.warn_surface);
    let (num, unit) = split_unit(text);
    let font = FontId::new(metrics::CONTROL, fonts::mono());
    let g = p.layout_no_wrap(num.to_owned(), font.clone(), ink);
    p.galley(Pos2::new(value.left() + 8.0, cy - g.size().y / 2.0), g, ink);
    let g = p.layout_no_wrap(unit.to_owned(), font, ink);
    let unit_pos = Pos2::new(value.right() - 8.0 - g.size().x, cy - g.size().y / 2.0);
    p.galley(unit_pos, g, ink.gamma_multiply(0.7));
}

fn hud_confirm(ui: &mut Ui, ok: Rect, l: &Look) -> Response {
    let t = &l.theme;
    let resp = ui.interact(ok, ui.scope_id().with("hud-confirm"), Sense::click());
    let fill = if resp.hovered() {
        t.accent_hover
    } else {
        t.accent
    };
    let p = ui.painter();
    p.rect_filled(ok, CornerRadius::same(RADIUS_SM), fill);
    if resp.has_focus() {
        let (w, c) = metrics::focus_ring(t);
        p.rect_stroke(
            ok.expand(w * 0.5),
            CornerRadius::same(RADIUS_SM + 2),
            Stroke::new(w, c),
            StrokeKind::Outside,
        );
    }
    glyph::check(p, ok.center(), 16.0, t.on_accent);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Confirm"));
    resp
}
