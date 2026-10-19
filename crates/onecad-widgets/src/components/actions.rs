//! Actions (g01): ToolButton (c-toolbtn), ToolPill (c-pill), SelectionLabel (c-sellabel).

pub mod badge;
pub(crate) mod common;

pub use badge::puzzle;

use egui::{
    Color32, CornerRadius, Frame, InnerResponse, Margin, Rect, Response, Sense, Stroke, Ui, Vec2,
    Widget, pos2, vec2,
};

use crate::components::tooltip::{self, Tip};
use crate::fonts;
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::metrics::{self, Elevation, H_TOOL, RADIUS_MD, RADIUS_XL};
use crate::theme::{Theme, look};
use common::{focus_ring, inset, opacity, sense};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolState {
    #[default]
    Default,
    Active,
    /// Orange: the tool creates geometry.
    Running,
    SketchActive,
}

pub struct ToolButton<'a> {
    icon: &'a str,
    state: ToolState,
    enabled: bool,
    extension: bool,
    tip: Option<Tip<'a>>,
}

impl<'a> ToolButton<'a> {
    pub fn new(icon: &'a str) -> Self {
        Self {
            icon,
            state: ToolState::Default,
            enabled: true,
            extension: false,
            tip: None,
        }
    }
    pub fn state(mut self, state: ToolState) -> Self {
        self.state = state;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// Contributed by an extension: paints the accent puzzle badge.
    pub fn extension(mut self, extension: bool) -> Self {
        self.extension = extension;
        self
    }
    pub fn tip(mut self, tip: Tip<'a>) -> Self {
        self.tip = Some(tip);
        self
    }
}

/// (fill, inset border, icon tint) for a state.
fn tool_colours(t: &Theme, state: ToolState, hovered: bool) -> (Color32, Option<Color32>, Color32) {
    match state {
        ToolState::Default if hovered => (t.hover2, None, t.ink3),
        ToolState::Default => (Color32::TRANSPARENT, None, t.ink3),
        ToolState::Active => (t.sel_bg, None, t.sel_text),
        ToolState::Running => (t.running_bg, Some(t.running_border), t.created_text),
        // #D6E4FA has no token; sel_border is the nearest blue.
        ToolState::SketchActive => (t.sel_border, None, t.sel_text),
    }
}

impl Widget for ToolButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let (rect, response) =
            ui.allocate_exact_size(Vec2::splat(H_TOOL), sense(self.enabled, Sense::click()));
        let hovered = response.hovered() && self.enabled;
        let (fill, border, tint) = tool_colours(&l.theme, self.state, hovered);
        let op = opacity(self.enabled);
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(RADIUS_MD), fill.gamma_multiply(op));
        if let Some(b) = border {
            inset(p, rect, RADIUS_MD, b.gamma_multiply(op));
        }
        if response.has_focus() {
            focus_ring(p, &l.theme, rect, RADIUS_MD);
            inset(p, rect, RADIUS_MD, l.theme.accent);
        }
        let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(24.0));
        let style = IconStyle {
            tint,
            bg: if fill == Color32::TRANSPARENT {
                l.theme.floating
            } else {
                fill
            },
            grey: !self.enabled,
        };
        let vector = glyph::by_name(
            p,
            self.icon,
            icon_rect.center(),
            24.0,
            tint.gamma_multiply(op),
        );
        if !vector {
            icon::paint(p, icon_rect, self.icon, l.dark, style, op);
        }
        if self.extension {
            let halo = l.theme.floating.gamma_multiply(op);
            let at = rect.right_bottom() - vec2(9.0, 9.0);
            badge::puzzle(p, at, 14.0, l.theme.accent.gamma_multiply(op), Some(halo));
        }
        let label = self.tip.map_or(self.icon, |t| t.name);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, self.enabled, label));
        match self.tip {
            Some(tip) => tooltip::attach(response, tip),
            None => response,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PillMode {
    #[default]
    Model,
    Sketch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PillAccent {
    #[default]
    None,
    Selection,
    SketchSelection,
    Running,
}

#[derive(Default)]
pub struct ToolPill {
    mode: PillMode,
    accent: PillAccent,
}

impl ToolPill {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn mode(mut self, mode: PillMode) -> Self {
        self.mode = mode;
        self
    }
    pub fn accent(mut self, accent: PillAccent) -> Self {
        self.accent = accent;
        self
    }

    fn border(&self, t: &Theme) -> Color32 {
        match (self.accent, self.mode) {
            (PillAccent::Selection, _) => t.sel_border,
            (PillAccent::SketchSelection | PillAccent::Running, _) => t.running_border,
            (PillAccent::None, PillMode::Sketch) => t.sketch_chrome_border,
            (PillAccent::None, PillMode::Model) => t.border,
        }
    }

    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        let l = look(ui.ctx());
        let border = self.border(&l.theme);
        let fill = match self.mode {
            PillMode::Model => l.theme.floating,
            PillMode::Sketch => l.theme.toolbar_sketch,
        };
        Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(RADIUS_XL))
            .inner_margin(Margin::same(4))
            .shadow(metrics::shadow(Elevation::Pill, &l.theme, l.dark))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    // `pill_divider` reads this so dividers follow the accent border.
                    ui.visuals_mut().widgets.noninteractive.bg_stroke.color = border;
                    add(ui)
                })
                .inner
            })
    }
}

/// 1x22 line with 3px side margins, tinted like the enclosing pill's border.
pub fn pill_divider(ui: &mut Ui) {
    let color = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let (rect, _) = ui.allocate_exact_size(vec2(1.0 + 6.0, 22.0), Sense::hover());
    ui.painter().rect_filled(
        Rect::from_center_size(rect.center(), vec2(1.0, 22.0)),
        0.0,
        color,
    );
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionVariant {
    #[default]
    Model,
    Sketch,
}

pub struct SelectionLabel<'a> {
    kind: &'a str,
    context: &'a str,
    variant: SelectionVariant,
}

pub struct SelectionLabelOutput {
    pub response: Response,
    /// The x button was clicked.
    pub clear: bool,
}

impl<'a> SelectionLabel<'a> {
    pub fn new(kind: &'a str, context: &'a str) -> Self {
        Self {
            kind,
            context,
            variant: SelectionVariant::Model,
        }
    }
    pub fn variant(mut self, variant: SelectionVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn show(self, ui: &mut Ui) -> SelectionLabelOutput {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (fill, kind_colour) = match self.variant {
            SelectionVariant::Model => (t.sel_soft, t.sel_text),
            // #FFF4E8 has no token; running_bg is the nearest orange wash.
            SelectionVariant::Sketch => (t.running_bg, t.created_text),
        };
        let layout = |text: &str, size: f32, family, colour| {
            ui.painter()
                .layout_no_wrap(text.to_owned(), egui::FontId::new(size, family), colour)
        };
        let kind = layout(self.kind, 12.5, fonts::semibold(), kind_colour);
        let ctx = layout(self.context, 12.0, egui::FontFamily::Proportional, t.ink4);
        let width = 8.0 + kind.size().x + 6.0 + ctx.size().x + 6.0 + 2.0 + 12.0 + 6.0;
        let (rect, response) = ui.allocate_exact_size(vec2(width, H_TOOL), Sense::hover());
        let close = Rect::from_center_size(
            pos2(rect.right() - 12.0, rect.center().y),
            Vec2::splat(20.0),
        );
        let close_resp = ui.interact(close, response.id.with("clear"), Sense::click());
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(RADIUS_MD), fill);
        let mut x = rect.left() + 8.0;
        for (g, colour) in [(kind, kind_colour), (ctx, t.ink4)] {
            let w = g.size().x;
            p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, colour);
            x += w + 6.0;
        }
        let tint = if close_resp.hovered() { t.ink2 } else { t.ink5 };
        glyph::close(p, close.center(), 12.0, tint);
        close_resp
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Clear selection"));
        SelectionLabelOutput {
            response,
            clear: close_resp.clicked(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::headless;

    #[test]
    fn every_component_renders_in_both_themes() {
        for dark in [false, true] {
            headless(dark, Vec::new(), |ui| {
                for s in [
                    ToolState::Default,
                    ToolState::Active,
                    ToolState::Running,
                    ToolState::SketchActive,
                ] {
                    ui.add(ToolButton::new("extrude").state(s));
                }
                for (mode, accent) in [
                    (PillMode::Model, PillAccent::None),
                    (PillMode::Sketch, PillAccent::None),
                    (PillMode::Model, PillAccent::Selection),
                    (PillMode::Model, PillAccent::SketchSelection),
                    (PillMode::Model, PillAccent::Running),
                ] {
                    ToolPill::new().mode(mode).accent(accent).show(ui, |ui| {
                        ui.add(ToolButton::new("select"));
                        pill_divider(ui);
                    });
                }
                for v in [SelectionVariant::Model, SelectionVariant::Sketch] {
                    let out = SelectionLabel::new("Face", "Bracket · planar")
                        .variant(v)
                        .show(ui);
                    assert!(!out.clear);
                }
            });
        }
    }

    #[test]
    fn disabled_tool_button_is_not_clickable() {
        headless(false, Vec::new(), |ui| {
            let off = ui.add(ToolButton::new("extrude").enabled(false));
            assert!(!off.sense.senses_click());
            let on = ui.add(ToolButton::new("extrude"));
            assert!(on.sense.senses_click());
            let ext = ui.add(ToolButton::new("extrude").extension(true));
            assert!(ext.sense.senses_click());
        });
    }
}
