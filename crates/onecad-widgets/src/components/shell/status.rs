//! StatusBar (c-status): h28, state/hint left; projection, coordinates and unit right.

use egui::{
    Color32, CornerRadius, FontId, Galley, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    Vec2, vec2,
};
use std::sync::Arc;

use crate::components::actions::common::fitted;
use crate::fonts;
use crate::metrics::{self, Elevation, H_STATUSBAR};
use crate::theme::{Look, look};

pub struct StatusItem<'a> {
    pub label: Option<&'a str>,
    pub value: &'a str,
    pub mono: bool,
}

#[derive(Default)]
pub struct StatusOutput {
    /// Set when the user clicks Persp (`true`) or Ortho (`false`).
    pub projection: Option<bool>,
}

pub struct StatusBar<'a> {
    sketch: bool,
    dot: Option<Color32>,
    text: &'a str,
    hint: Option<&'a str>,
    projection: Option<bool>,
    items: &'a [StatusItem<'a>],
    unit: Option<&'a str>,
}

impl<'a> Default for StatusBar<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> StatusBar<'a> {
    pub fn new() -> Self {
        Self {
            sketch: false,
            dot: None,
            text: "",
            hint: None,
            projection: None,
            items: &[],
            unit: None,
        }
    }
    pub fn sketch(mut self, sketch: bool) -> Self {
        self.sketch = sketch;
        self
    }
    pub fn dot(mut self, dot: Option<Color32>) -> Self {
        self.dot = dot;
        self
    }
    pub fn text(mut self, text: &'a str) -> Self {
        self.text = text;
        self
    }
    pub fn hint(mut self, hint: Option<&'a str>) -> Self {
        self.hint = hint;
        self
    }
    /// `Some(true)` = Persp, `Some(false)` = Ortho, `None` hides the switch.
    pub fn projection(mut self, projection: Option<bool>) -> Self {
        self.projection = projection;
        self
    }
    pub fn items(mut self, items: &'a [StatusItem<'a>]) -> Self {
        self.items = items;
        self
    }
    pub fn unit(mut self, unit: Option<&'a str>) -> Self {
        self.unit = unit;
        self
    }

    pub fn show(self, ui: &mut Ui) -> StatusOutput {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), H_STATUSBAR), Sense::hover());
        let (fill, line) = if self.sketch {
            (t.toolbar_sketch, t.sketch_chrome_border)
        } else {
            (t.statusbar, t.border)
        };
        let p = ui.painter();
        p.rect_filled(rect, 0.0, fill);
        p.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, line));
        let mut out = StatusOutput::default();
        let mut x = rect.right() - 10.0;
        if let Some(unit) = self.unit {
            x -= text_right(ui, x, rect, unit, mono_font(), l.theme.ink5) + 10.0;
        }
        for it in self.items.iter().rev() {
            x -= item_right(ui, x, rect, it, &l) + 10.0;
        }
        if let Some(persp) = self.projection {
            let (clicked, w) = segmented(ui, x, rect, persp, &l);
            out.projection = clicked;
            x -= w + 10.0;
        }
        self.paint_left(ui, rect, x, &l);
        out
    }

    /// Left cluster, elided so it never crosses `limit` (left edge of the right cluster).
    fn paint_left(&self, ui: &Ui, rect: Rect, limit: f32, l: &Look) {
        let t = &l.theme;
        let p = ui.painter();
        let cy = rect.center().y;
        let mut x = rect.left() + 10.0;
        if self.sketch {
            let font = FontId::new(metrics::SECONDARY, fonts::medium());
            x += left_text(p, "Sketch", font, t.sel_text, (x, cy), limit) + 10.0;
        }
        if let Some(c) = self.dot.filter(|_| x + 7.0 <= limit) {
            p.circle_filled(Pos2::new(x + 3.5, cy), 3.5, c);
            x += 7.0 + 6.0;
        }
        for (s, color) in [(Some(self.text), t.ink3), (self.hint, t.ink5)] {
            let Some(s) = s.filter(|s| !s.is_empty()) else {
                continue;
            };
            let font = FontId::proportional(metrics::SECONDARY);
            x += left_text(p, s, font, color, (x, cy), limit) + 10.0;
        }
    }
}

/// Paints vertically centred text at `(x, cy)` elided to `limit`; returns the width used.
fn left_text(
    p: &Painter,
    s: &str,
    font: FontId,
    c: Color32,
    (x, cy): (f32, f32),
    limit: f32,
) -> f32 {
    let room = limit - x;
    if room <= 0.0 {
        return 0.0;
    }
    let g = fitted(p, s, font, c, room);
    let w = g.size().x;
    p.galley(Pos2::new(x, cy - g.size().y / 2.0), g, c);
    w
}

fn mono_font() -> FontId {
    FontId::new(11.5, fonts::mono())
}

/// Paints right-aligned at `right`; returns the width used.
fn text_right(ui: &Ui, right: f32, rect: Rect, s: &str, font: FontId, color: Color32) -> f32 {
    let g = ui.painter().layout_no_wrap(s.to_owned(), font, color);
    let w = g.size().x;
    ui.painter().galley(
        Pos2::new(right - w, rect.center().y - g.size().y / 2.0),
        g,
        color,
    );
    w
}

fn item_right(ui: &Ui, right: f32, rect: Rect, it: &StatusItem, l: &Look) -> f32 {
    let t = &l.theme;
    let font = if it.mono {
        mono_font()
    } else {
        FontId::proportional(11.5)
    };
    let value = text_right(ui, right, rect, it.value, font.clone(), t.ink4);
    match it.label {
        Some(label) => {
            let lw = text_right(ui, right - value - 4.0, rect, label, font, t.ink5);
            value + 4.0 + lw
        }
        None => value,
    }
}

/// Persp | Ortho segmented switch ending at `right`; returns (click, total width).
fn segmented(ui: &mut Ui, right: f32, rect: Rect, persp: bool, l: &Look) -> (Option<bool>, f32) {
    let t = &l.theme;
    let labels = [("Persp", true), ("Ortho", false)];
    let galleys: Vec<Arc<Galley>> = labels
        .iter()
        .map(|(s, on)| {
            let family = if *on == persp {
                fonts::semibold()
            } else {
                egui::FontFamily::Proportional
            };
            ui.painter().layout_no_wrap(
                (*s).to_owned(),
                FontId::new(metrics::SECONDARY, family),
                Color32::PLACEHOLDER,
            )
        })
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 20.0).collect();
    let total = widths.iter().sum::<f32>() + 4.0;
    let outer = Rect::from_center_size(
        Pos2::new(right - total / 2.0, rect.center().y),
        vec2(total, 24.0),
    );
    ui.painter()
        .rect_filled(outer, CornerRadius::same(metrics::RADIUS_MD), t.well);
    let mut x = outer.left() + 2.0;
    let mut clicked = None;
    for ((g, w), (label, is_persp)) in galleys.into_iter().zip(widths).zip(labels) {
        let seg = Rect::from_min_size(Pos2::new(x, outer.top() + 2.0), vec2(w, 20.0));
        let r = ui.interact(seg, ui.scope_id().with(("proj", label)), Sense::click());
        r.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::RadioButton, true, is_persp == persp, label)
        });
        paint_segment(ui.painter(), seg, g, is_persp == persp, r.has_focus(), l);
        if r.clicked() {
            clicked = Some(is_persp);
        }
        x += w;
    }
    (clicked, total)
}

fn paint_segment(p: &Painter, seg: Rect, g: Arc<Galley>, active: bool, focus: bool, l: &Look) {
    let t = &l.theme;
    if active {
        let cr = CornerRadius::same(metrics::RADIUS_SM);
        p.add(metrics::shadow(Elevation::Pill, t, l.dark).as_shape(seg, cr));
        p.rect_filled(seg, cr, t.surface);
    }
    if focus {
        let (fw, c) = metrics::focus_ring(t);
        p.rect_stroke(
            seg.expand(fw * 0.5),
            CornerRadius::same(metrics::RADIUS_MD),
            Stroke::new(fw, c),
            StrokeKind::Outside,
        );
    }
    let color = if active { t.ink } else { t.ink4 };
    let size: Vec2 = g.size();
    p.galley(seg.center() - size / 2.0, g, color);
}
