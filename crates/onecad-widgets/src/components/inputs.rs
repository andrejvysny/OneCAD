//! Inputs (g05): NumberField (c-num), Select + Toggle, chips + option group (c-ref),
//! SearchField (c-search).

mod chips;
mod number;

pub use chips::{AddChip, ChipOutput, OptionIconGroup, ReferenceChip};
pub use number::{NumberField, ParsedValue, parse_value, step_value};

use egui::epaint::Shadow;
use egui::{
    Align, Color32, CornerRadius, FontFamily, FontId, Frame, IdSalt, Layout, Margin, Rect,
    Response, RichText, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2, Widget,
    pos2, vec2,
};

use crate::components::actions::common::{focus_ring, galley, opacity, sense};
use crate::components::lists::{MenuRow, menu_frame};
use crate::fonts;
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::metrics::{CONTROL, H_CHIP, H_FIELD, RADIUS_SM};
use crate::theme::{Theme, look};

/// Field chrome: fill + border, accent border and ring when focused or `ring` is set.
fn field_shape(
    t: &Theme,
    rect: Rect,
    radius: u8,
    fill: Color32,
    border: Option<Color32>,
    ring: Option<Color32>,
) -> Shape {
    let r = CornerRadius::same(radius);
    let mut shapes = vec![Shape::rect_filled(rect, r, fill)];
    if let Some(b) = border {
        shapes.push(Shape::rect_stroke(
            rect,
            r,
            Stroke::new(1.0, b),
            StrokeKind::Inside,
        ));
    }
    if let Some(c) = ring {
        let (w, _) = crate::metrics::focus_ring(t);
        shapes.push(Shape::rect_stroke(
            rect.expand(w * 0.5),
            CornerRadius::same(radius + 2),
            Stroke::new(w, c),
            StrokeKind::Outside,
        ));
    }
    Shape::Vec(shapes)
}

pub struct Select<'a> {
    id_salt: IdSalt,
    selected: &'a str,
    options: &'a [&'a str],
    icon: Option<&'a str>,
    width: f32,
    enabled: bool,
}

pub struct SelectOutput {
    pub response: Response,
    pub chosen: Option<usize>,
}

impl<'a> Select<'a> {
    pub fn new(
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        selected: &'a str,
        options: &'a [&'a str],
    ) -> Self {
        Self {
            id_salt: IdSalt::new(id_salt),
            selected,
            options,
            icon: None,
            width: 150.0,
            enabled: true,
        }
    }
    pub fn icon(mut self, icon: Option<&'a str>) -> Self {
        self.icon = icon;
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn show(self, ui: &mut Ui) -> SelectOutput {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (rect, _) = ui.allocate_exact_size(vec2(self.width, H_FIELD), Sense::hover());
        let id = ui.make_persistent_id(self.id_salt);
        let response = ui.interact(rect, id, sense(self.enabled, Sense::click()));
        let op = opacity(self.enabled);
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
        let hovered = response.hovered() && self.enabled;
        let ring = (response.has_focus() || open).then(|| crate::metrics::focus_ring(t).1);
        let border = if ring.is_some() {
            t.accent
        } else {
            t.border_strong
        };
        let fill = if hovered { t.hover } else { t.surface };
        let shape = field_shape(
            t,
            rect,
            RADIUS_SM,
            fill.gamma_multiply(op),
            Some(border.gamma_multiply(op)),
            ring,
        );
        let p = ui.painter();
        p.add(shape);
        let cy = rect.center().y;
        let mut x = rect.left() + 8.0;
        if let Some(name) = self.icon {
            let r = Rect::from_center_size(pos2(x + 8.0, cy), Vec2::splat(16.0));
            let style = IconStyle {
                tint: t.ink3,
                bg: fill,
                grey: !self.enabled,
            };
            icon::paint(p, r, name, l.dark, style, op);
            x += 16.0 + 6.0;
        }
        let g = galley(p, self.selected, CONTROL, FontFamily::Proportional);
        p.galley(pos2(x, cy - g.size().y / 2.0), g, t.ink2.gamma_multiply(op));
        let chev = pos2(rect.right() - 8.0 - 6.0, cy);
        glyph::chevron_down(p, chev, 14.0, t.ink5.gamma_multiply(op));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, self.enabled, self.selected)
        });
        let mut chosen = None;
        if self.enabled {
            egui::Popup::menu(&response)
                .frame(menu_frame(ui.ctx()))
                .width(self.width)
                .gap(4.0)
                .show(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, option) in self.options.iter().enumerate() {
                        let checked = *option == self.selected;
                        if ui
                            .add(MenuRow::new(None, option).checked(checked))
                            .clicked()
                        {
                            chosen = Some(i);
                        }
                    }
                });
        }
        SelectOutput { response, chosen }
    }
}

pub struct Toggle<'a> {
    on: &'a mut bool,
    enabled: bool,
}

impl<'a> Toggle<'a> {
    pub fn new(on: &'a mut bool) -> Self {
        Self { on, enabled: true }
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

const KNOB_SHADOW: Shadow = Shadow {
    offset: [0, 1],
    blur: 2,
    spread: 0,
    color: Color32::from_black_alpha(77),
};

impl Widget for Toggle<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (rect, mut response) =
            ui.allocate_exact_size(vec2(30.0, 18.0), sense(self.enabled, Sense::click()));
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        let k = ui.ctx().animate_bool_with_time(response.id, *self.on, 0.12);
        let op = opacity(self.enabled);
        let p = ui.painter();
        let track = t.toggle_off.lerp_to_gamma(t.accent, k);
        p.rect_filled(rect, CornerRadius::same(9), track.gamma_multiply(op));
        let left = rect.left() + 2.0 + 7.0;
        let knob = Rect::from_center_size(
            pos2(left + (rect.width() - 18.0) * k, rect.center().y),
            Vec2::splat(14.0),
        );
        p.add(KNOB_SHADOW.as_shape(knob, CornerRadius::same(7)));
        p.rect_filled(knob, CornerRadius::same(7), t.on_accent.gamma_multiply(op));
        if response.has_focus() {
            focus_ring(p, t, rect, 9);
        }
        let on = *self.on;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::CheckBox, self.enabled, on, "Toggle")
        });
        response
    }
}

pub struct SearchField<'a> {
    id_salt: IdSalt,
    text: &'a mut String,
    placeholder: &'a str,
    shortcut: Option<&'a str>,
    width: f32,
    enabled: bool,
}

impl<'a> SearchField<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug, text: &'a mut String) -> Self {
        Self {
            id_salt: IdSalt::new(id_salt),
            text,
            placeholder: "",
            shortcut: None,
            width: 220.0,
            enabled: true,
        }
    }
    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }
    /// With a shortcut the field is the bordered command-search look; without it, the
    /// borderless filter look.
    pub fn shortcut(mut self, shortcut: Option<&'a str>) -> Self {
        self.shortcut = shortcut;
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl Widget for SearchField<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let id = ui.make_persistent_id(self.id_salt);
        let (rect, _) = ui.allocate_exact_size(vec2(self.width, H_CHIP), Sense::hover());
        let bg = ui.painter().add(Shape::Noop);
        if self.enabled
            && ui
                .interact(rect, id.with("field"), Sense::click())
                .clicked()
        {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let op = opacity(self.enabled);
        let command = self.shortcut.is_some();
        let (pad, gap, icon_size) = if command {
            (10.0, 8.0, 15.0)
        } else {
            (9.0, 7.0, 14.0)
        };
        let fill = if command { t.surface } else { t.hover };
        let hint = if command { t.ink5 } else { t.ink6 };
        let ic = Rect::from_center_size(
            pos2(rect.left() + pad + icon_size / 2.0, rect.center().y),
            Vec2::splat(icon_size),
        );
        glyph::search(
            ui.painter(),
            ic.center(),
            icon_size,
            hint.gamma_multiply(op),
        );
        let key = self
            .shortcut
            .map(|k| galley(ui.painter(), k, 11.0, fonts::mono()));
        let key_w = key.as_ref().map_or(0.0, |g| g.size().x + gap);
        let x0 = ic.right() + gap;
        let field = Rect::from_min_max(
            pos2(x0, rect.top()),
            pos2(rect.right() - pad - key_w, rect.bottom()),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(field)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let response = child.add(
            TextEdit::singleline(self.text)
                .id(id)
                .font(FontId::new(CONTROL, FontFamily::Proportional))
                .text_color(t.ink2.gamma_multiply(op))
                .hint_text(RichText::new(self.placeholder.to_owned()).color(hint))
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .desired_width(field.width())
                .interactive(self.enabled),
        );
        if let Some(g) = key {
            let pos = pos2(
                rect.right() - pad - g.size().x,
                rect.center().y - g.size().y / 2.0,
            );
            ui.painter().galley(pos, g, t.ink5.gamma_multiply(op));
        }
        let focused = response.has_focus();
        let border = match (command, focused) {
            (_, true) => Some(t.accent),
            (true, false) => Some(t.border_strong),
            (false, false) => None,
        };
        let ring = focused.then(|| crate::metrics::focus_ring(t).1);
        let shape = field_shape(
            t,
            rect,
            RADIUS_SM,
            fill.gamma_multiply(op),
            border.map(|b| b.gamma_multiply(op)),
            ring,
        );
        ui.painter().set(bg, shape);
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::actions::common::headless;
    use crate::components::tooltip::Tip;

    #[test]
    fn number_error_wraps_inside_feature_card_at_both_display_scales() {
        use crate::components::overlays::FeatureCard;

        const ERROR: &str = "Distance cannot resolve because the referenced feature is unavailable. Select a valid persistent reference before applying this operation.";
        for scale in [1.0, 2.0] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx, crate::theme::load(false).unwrap(), false);
            ctx.set_pixels_per_point(scale);
            let mut text = "0.1234".to_owned();
            for _ in 0..4 {
                let mut field_width = 0.0;
                let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
                    FeatureCard::new("numeric-bounds", "extrude", "Extrude")
                        .width(280.0)
                        .show(ui, |rows| {
                            rows.row("Extrusion distance", |ui| {
                                field_width = ui.available_width();
                                ui.add(
                                    NumberField::new("distance", &mut text)
                                        .width(field_width)
                                        .error(Some(ERROR)),
                                );
                            });
                        });
                    assert!(ui.min_rect().width() <= 281.0, "{:?}", ui.min_rect());
                });
                frame.textures_delta.clear();
                let error = frame
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.job.text == ERROR => {
                            Some(text)
                        }
                        _ => None,
                    })
                    .expect("error remains visible");
                assert!(error.galley.rows.len() > 1);
                assert!(
                    error.galley.size().x <= field_width + 1.0,
                    "scale={scale}: {} > {field_width}",
                    error.galley.size().x
                );
            }
        }
    }

    #[test]
    fn every_component_renders_in_both_themes() {
        for dark in [false, true] {
            headless(dark, Vec::new(), |ui| {
                let mut a = "14.00".to_owned();
                let mut b = "=d1/2".to_owned();
                let mut c = "-3".to_owned();
                ui.add(NumberField::new("a", &mut a).unit("mm"));
                ui.add(NumberField::new("b", &mut b).unit("mm"));
                ui.add(
                    NumberField::new("c", &mut c)
                        .unit("mm")
                        .error(Some("Distance must be greater than 0")),
                );
                ui.add(NumberField::new("d", &mut a).enabled(false).compact(true));
                let out = Select::new("s", "Distance", &["Distance", "Through all"])
                    .icon(Some("union"))
                    .show(ui);
                assert!(out.chosen.is_none());
                let (mut on, mut off) = (true, false);
                ui.add(Toggle::new(&mut on));
                ui.add(Toggle::new(&mut off).enabled(false));
                let chip = ReferenceChip::new("2 edges")
                    .icon(Some("sketchEntity"))
                    .removable(true)
                    .show(ui);
                assert!(!chip.remove);
                ui.add(AddChip::new());
                let mut sel = 0;
                let opts = [("union", Tip::new("Join")), ("subtract", Tip::new("Cut"))];
                ui.add(OptionIconGroup::new(&mut sel, &opts));
                let mut q = String::new();
                ui.add(
                    SearchField::new("q1", &mut q)
                        .placeholder("Search commands")
                        .shortcut(Some("⌘K")),
                );
                ui.add(
                    SearchField::new("q2", &mut q)
                        .placeholder("Filter")
                        .width(200.0),
                );
            });
        }
    }
}
