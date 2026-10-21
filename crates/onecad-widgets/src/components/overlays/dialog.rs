//! Dialog (c-dialog): modal confirm / destroy / failure / input card.
//! Pure painting and layout; the host maps the returned `DialogOutcome` to app state.
//! Enter triggers the default button, Escape or a backdrop click reports `dismissed`.

use egui::{
    Align, Color32, Context, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Modifiers, Rect,
    RichText, Sense, Stroke, Ui, Vec2, vec2,
};

use crate::components::button::{Button, ButtonKind};
use crate::fonts;
use crate::icon::{self, IconStyle};
use crate::metrics::{BODY, Elevation, RADIUS_LG, RADIUS_MD, SECONDARY, SECTION, TITLE, shadow};
use crate::theme::look;

const PAD: i8 = 20;
const WELL: f32 = 36.0;
const GAP: f32 = 14.0;
const ROW_H: f32 = 30.0;
const MAX_ROWS: usize = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogIntent {
    #[default]
    Decide,
    Destroy,
    Failure,
    Input,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogOutcome {
    pub primary: bool,
    pub secondary: bool,
    pub tertiary: bool,
    pub dismissed: bool,
    /// Area rect of the card, for layout checks.
    pub rect: Rect,
}

impl Default for DialogOutcome {
    fn default() -> Self {
        Self {
            primary: false,
            secondary: false,
            tertiary: false,
            dismissed: false,
            rect: Rect::NOTHING,
        }
    }
}

pub struct Dialog<'a> {
    intent: DialogIntent,
    icon: Option<&'a str>,
    title: &'a str,
    body: Option<&'a str>,
    primary: Option<(&'a str, bool)>,
    secondary: Option<(&'a str, bool)>,
    tertiary: Option<&'a str>,
    focus_secondary: bool,
    disabled: bool,
    width: f32,
}

impl<'a> Dialog<'a> {
    pub fn new(intent: DialogIntent, title: &'a str) -> Self {
        Self {
            intent,
            icon: None,
            title,
            body: None,
            primary: None,
            secondary: None,
            tertiary: None,
            focus_secondary: false,
            disabled: false,
            width: 420.0,
        }
    }
    pub fn icon(mut self, name: &'a str) -> Self {
        self.icon = Some(name);
        self
    }
    pub fn body(mut self, text: &'a str) -> Self {
        self.body = Some(text);
        self
    }
    /// Omit to show a secondary-only dialog.
    pub fn primary(mut self, label: &'a str, enabled: bool) -> Self {
        self.primary = Some((label, enabled));
        self
    }
    pub fn secondary(mut self, label: &'a str, enabled: bool) -> Self {
        self.secondary = Some((label, enabled));
        self
    }
    pub fn tertiary(mut self, label: &'a str) -> Self {
        self.tertiary = Some(label);
        self
    }
    /// Cancel is the default (Enter) button and takes initial focus.
    pub fn focus_secondary(mut self, on: bool) -> Self {
        self.focus_secondary = on;
        self
    }
    /// Disables every button, e.g. while a request is in flight.
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn show(self, ctx: &Context, id: Id, extra: impl FnOnce(&mut Ui)) -> DialogOutcome {
        let l = look(ctx);
        let frame = Frame::new()
            .fill(l.theme.surface)
            .stroke(Stroke::new(1.0, l.theme.border_subtle))
            .corner_radius(CornerRadius::same(RADIUS_LG))
            .shadow(shadow(Elevation::Card, &l.theme, l.dark))
            .inner_margin(Margin::same(PAD));
        // A focused text field owns Enter.
        let enter = !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let mut out = DialogOutcome::default();
        let resp = egui::Modal::new(id).frame(frame).show(ctx, |ui| {
            ui.set_width(self.width - 2.0 * f32::from(PAD) - 2.0);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            self.content(ui, extra);
            ui.add_space(16.0);
            self.footer(ui, enter, &mut out);
        });
        out.dismissed = resp.should_close();
        out.rect = resp.response.rect;
        out
    }

    fn has_well(&self) -> bool {
        self.intent != DialogIntent::Input && self.icon.is_some()
    }

    fn content(&self, ui: &mut Ui, extra: impl FnOnce(&mut Ui)) {
        let l = look(ui.ctx());
        let t = &l.theme;
        let total = ui.available_width();
        let col_w = if self.has_well() {
            total - WELL - GAP
        } else {
            total
        };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            if let (true, Some(name)) = (self.has_well(), self.icon) {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(WELL), Sense::hover());
                let (bg, tint) = match self.intent {
                    DialogIntent::Destroy => (t.danger_surface, t.danger_strong),
                    DialogIntent::Failure => (t.warn_surface, t.warn),
                    _ => (t.sel_soft, t.accent),
                };
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(RADIUS_MD), bg);
                let style = IconStyle {
                    tint,
                    bg,
                    grey: false,
                };
                let glyph = Rect::from_center_size(rect.center(), Vec2::splat(20.0));
                icon::paint(ui.painter(), glyph, name, l.dark, style, 1.0);
                ui.add_space(GAP);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.set_width(col_w);
                ui.label(
                    RichText::new(self.title)
                        .font(FontId::new(TITLE, fonts::semibold()))
                        .color(t.ink),
                );
                if let Some(body) = self.body {
                    ui.add_space(4.0);
                    ui.label(RichText::new(body).size(BODY).color(t.ink3));
                }
                extra(ui);
            });
        });
    }

    fn footer(&self, ui: &mut Ui, enter: bool, out: &mut DialogOutcome) {
        let l = look(ui.ctx());
        let t = &l.theme;
        let on = !self.disabled;
        let default_secondary = self.focus_secondary || self.primary.is_none();
        ui.horizontal(|ui| {
            ui.set_min_height(ROW_H);
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some(label) = self.tertiary {
                let colour = match self.intent {
                    DialogIntent::Decide => t.danger_strong,
                    DialogIntent::Failure => t.accent,
                    _ => t.ink3,
                };
                out.tertiary = link(ui, label, colour, on);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some((label, enabled)) = self.primary {
                    let text = hinted(label, !default_secondary);
                    let kind = if self.intent == DialogIntent::Destroy {
                        ButtonKind::Danger
                    } else {
                        ButtonKind::Primary
                    };
                    let b = Button::new(&text)
                        .kind(kind)
                        .enabled(enabled && on)
                        .min_width(64.0);
                    let r = ui.add(b);
                    named(&r, label, enabled && on);
                    out.primary = r.clicked();
                    if enter && !default_secondary && enabled && on {
                        out.primary = true;
                    }
                }
                if let Some((label, enabled)) = self.secondary {
                    let text = hinted(label, default_secondary);
                    let b = Button::new(&text).enabled(enabled && on).min_width(64.0);
                    let r = ui.add(b);
                    named(&r, label, enabled && on);
                    if self.focus_secondary {
                        ui.memory_mut(|m| {
                            if m.focused().is_none() {
                                m.request_focus(r.id);
                            }
                        });
                    }
                    out.secondary = r.clicked();
                    if enter && default_secondary && enabled && on {
                        out.secondary = true;
                    }
                }
            });
        });
    }
}

/// The ↵ hint is visual only; assistive tech and lookups use the plain label.
fn named(response: &egui::Response, label: &str, enabled: bool) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
}

fn hinted(label: &str, default: bool) -> String {
    if default {
        format!("{label} \u{21b5}")
    } else {
        label.to_owned()
    }
}

/// Text-only button used for the left-hand tertiary action.
fn link(ui: &mut Ui, label: &str, colour: Color32, enabled: bool) -> bool {
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), FontId::new(12.5, fonts::medium()), colour);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(vec2(galley.size().x + 8.0, ROW_H), sense);
    let opacity = if enabled { 1.0 } else { 0.4 };
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter()
        .galley(pos, galley, colour.gamma_multiply(opacity));
    if enabled && r.hovered() {
        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
        ui.painter().hline(
            pos.x..=pos.x + rect.width() - 8.0,
            rect.center().y + 7.0,
            Stroke::new(1.0, colour),
        );
    }
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    enabled && r.clicked()
}

fn well_box(t: &crate::theme::Theme) -> Frame {
    Frame::new()
        .fill(t.well)
        .stroke(Stroke::new(1.0, t.border_subtle))
        .corner_radius(CornerRadius::same(RADIUS_MD))
}

/// Bordered list of `(icon, name, kind)` rows; scrolls past six rows.
pub fn affected_list(ui: &mut Ui, rows: &[(&str, &str, &str)]) {
    ui.add_space(12.0);
    let l = look(ui.ctx());
    let t = &l.theme;
    let width = ui.available_width() - 2.0;
    well_box(t).show(ui, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let list = |ui: &mut Ui| {
            for (name_icon, name, kind) in rows {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                let p = ui.painter().with_clip_rect(rect);
                let glyph = Rect::from_center_size(
                    egui::pos2(rect.left() + 10.0 + 8.0, rect.center().y),
                    Vec2::splat(16.0),
                );
                let style = IconStyle {
                    tint: t.ink4,
                    bg: t.well,
                    grey: false,
                };
                icon::paint(&p, glyph, name_icon, l.dark, style, 1.0);
                let k = p.layout_no_wrap(
                    (*kind).to_owned(),
                    FontId::new(SECTION, fonts::mono()),
                    t.ink5,
                );
                let kx = rect.right() - 10.0 - k.size().x;
                let n = p.layout_no_wrap((*name).to_owned(), FontId::proportional(BODY), t.ink2);
                let nx = glyph.right() + 8.0;
                // Clip long names so they never run under the kind tag.
                let np = ui.painter().with_clip_rect(Rect::from_min_max(
                    rect.min,
                    egui::pos2(kx - 8.0, rect.max.y),
                ));
                np.galley(
                    egui::pos2(nx, rect.center().y - n.size().y / 2.0),
                    n,
                    t.ink2,
                );
                p.galley(
                    egui::pos2(kx, rect.center().y - k.size().y / 2.0),
                    k,
                    t.ink5,
                );
            }
        };
        if rows.len() > MAX_ROWS {
            egui::ScrollArea::vertical()
                .max_height(MAX_ROWS as f32 * ROW_H)
                .show(ui, list);
        } else {
            list(ui);
        }
    });
}

/// One-line reassurance under a destructive list.
pub fn undo_hint(ui: &mut Ui, text: &str) {
    let l = look(ui.ctx());
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        let style = IconStyle {
            tint: l.theme.ink5,
            bg: l.theme.surface,
            grey: false,
        };
        icon::paint(ui.painter(), rect, "undo", l.dark, style, 1.0);
        ui.label(RichText::new(text).size(SECONDARY).color(l.theme.ink5));
    });
}

/// "Technical details" disclosure with a monospace block.
pub fn details(ui: &mut Ui, open: &mut bool, text: &str) {
    let l = look(ui.ctx());
    let t = &l.theme;
    ui.add_space(8.0);
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let label = ui.painter().layout_no_wrap(
        "Technical details".to_owned(),
        FontId::new(SECONDARY, fonts::medium()),
        t.ink4,
    );
    let pos = egui::pos2(rect.left(), rect.center().y - label.size().y / 2.0);
    let right = pos.x + label.size().x + 6.0;
    ui.painter().galley(pos, label, t.ink4);
    let style = IconStyle {
        tint: t.ink4,
        bg: t.surface,
        grey: false,
    };
    let chev = Rect::from_min_size(egui::pos2(right, rect.center().y - 6.0), Vec2::splat(12.0));
    icon::paint(ui.painter(), chev, "chevronDown", l.dark, style, 1.0);
    if r.clicked() {
        *open = !*open;
    }
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Technical details"));
    if *open {
        ui.add_space(6.0);
        let width = ui.available_width() - 2.0;
        well_box(t).inner_margin(Margin::same(10)).show(ui, |ui| {
            ui.set_width(width - 20.0);
            ui.label(
                RichText::new(text)
                    .font(FontId::new(SECTION + 0.5, fonts::mono()))
                    .color(t.ink3),
            );
        });
    }
}

#[cfg(test)]
#[path = "dialog_tests.rs"]
mod tests;
