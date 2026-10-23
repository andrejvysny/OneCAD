//! NumberField (c-num) with expression/number parsing and arrow-key stepping.

use egui::{
    Align, FontId, Frame, Id, IdSalt, Key, Layout, Margin, Modifiers, Response, RichText, Sense,
    Shape, TextEdit, Ui, UiBuilder, Widget, pos2, vec2,
};

use super::field_shape;
use crate::components::actions::common::{galley, opacity};
use crate::fonts;
use crate::glyph;
use crate::metrics::{H_FIELD, MONO_VALUE, RADIUS_SM};
use crate::theme::look;

#[derive(Clone, Debug, PartialEq)]
pub enum ParsedValue {
    Number(f64),
    Expression(String),
    Invalid,
}

/// Parse field text: `=expr` is an expression (not evaluated), otherwise a number that may
/// use U+2212 for minus and spaces or thin spaces as digit grouping.
pub fn parse_value(text: &str) -> ParsedValue {
    let s = text.trim();
    if let Some(rest) = s.strip_prefix('=') {
        return ParsedValue::Expression(rest.to_owned());
    }
    let cleaned: String = s
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{2009}' | '\u{202f}' | '\u{a0}'))
        .map(|c| if c == '\u{2212}' { '-' } else { c })
        .collect();
    match cleaned.parse::<f64>() {
        Ok(v) if v.is_finite() => ParsedValue::Number(v),
        _ => ParsedValue::Invalid,
    }
}

/// One arrow-key step; shift multiplies by 10.
pub fn step_value(v: f64, step: f64, shift: bool, up: bool) -> f64 {
    let amount = if shift { step * 10.0 } else { step };
    if up { v + amount } else { v - amount }
}

fn decimal_places(text: &str) -> usize {
    let (mantissa, exponent) = text
        .split_once(['e', 'E'])
        .map_or((text, 0), |(value, exponent)| {
            (value, exponent.parse::<i32>().unwrap_or(0))
        });
    let fraction = mantissa.split_once('.').map_or(0, |(_, digits)| {
        digits.chars().filter(char::is_ascii_digit).count()
    });
    (fraction as i64 - i64::from(exponent)).clamp(0, 324) as usize
}

fn stepped_text(text: &str, step: f64, shift: bool, up: bool, decimals: usize) -> Option<String> {
    let ParsedValue::Number(value) = parse_value(text) else {
        return None;
    };
    let value = step_value(value, step, shift, up);
    if !value.is_finite() {
        return None;
    }
    // Display precision is a minimum, never a limit on the authored value.
    let precision = decimals
        .max(decimal_places(text.trim()))
        .max(decimal_places(&step.to_string()));
    Some(format!("{value:.precision$}"))
}

fn shown_text(text: &str, decimals: usize, focused: bool) -> String {
    if !focused && let ParsedValue::Number(value) = parse_value(text) {
        format!("{value:.decimals$}")
    } else {
        text.to_owned()
    }
}

/// Width reserved for the vector degree sign.
const DEGREE_W: f32 = 6.0;

pub struct NumberField<'a> {
    id_salt: IdSalt,
    text: &'a mut String,
    unit: Option<&'a str>,
    label: &'a str,
    width: f32,
    step: f64,
    decimals: usize,
    error: Option<&'a str>,
    enabled: bool,
    compact: bool,
    keep_focus: bool,
    step_keys: bool,
}

impl<'a> NumberField<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug, text: &'a mut String) -> Self {
        Self {
            id_salt: IdSalt::new(id_salt),
            text,
            unit: None,
            label: "Numeric expression",
            width: 150.0,
            step: 1.0,
            decimals: 2,
            error: None,
            enabled: true,
            compact: false,
            keep_focus: false,
            step_keys: true,
        }
    }
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = label;
        self
    }
    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = Some(unit);
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }
    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }
    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// Keep focus when Enter is pressed (the host handles Enter itself).
    pub fn keep_focus_on_enter(mut self, keep: bool) -> Self {
        self.keep_focus = keep;
        self
    }
    /// Gate Up/Down nudging, e.g. while an IME composition is active.
    pub fn step_keys(mut self, enabled: bool) -> Self {
        self.step_keys = enabled;
        self
    }
    /// 26px / r5 / 12px variant used inline in history rows.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// Up/Down while focused nudges a numeric value; true when the text changed.
    fn nudge(&mut self, ui: &Ui, id: Id) -> bool {
        if !self.enabled || !self.step_keys || !ui.memory(|m| m.has_focus(id)) {
            return false;
        }
        let shift = ui.input(|i| i.modifiers.shift);
        let mods = if shift {
            Modifiers::SHIFT
        } else {
            Modifiers::NONE
        };
        let mut changed = false;
        for (up, key) in [(true, Key::ArrowUp), (false, Key::ArrowDown)] {
            if ui.input_mut(|i| i.consume_key(mods, key))
                && let Some(text) = stepped_text(self.text, self.step, shift, up, self.decimals)
            {
                *self.text = text;
                changed = true;
            }
        }
        changed
    }
}

impl Widget for NumberField<'_> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = l.theme.clone();
        let id = ui.make_persistent_id(self.id_salt);
        let (height, radius, size) = if self.compact {
            (26.0, 5, 12.0)
        } else {
            (H_FIELD, RADIUS_SM, MONO_VALUE)
        };
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let (rect, _) = ui.allocate_exact_size(vec2(self.width, height), Sense::hover());
            let bg = ui.painter().add(Shape::Noop);
            if self.enabled
                && ui
                    .interact(rect, id.with("field"), Sense::click())
                    .clicked()
            {
                ui.memory_mut(|m| m.request_focus(id));
            }
            let native_changed = crate::text_session::before(ui, id, self.text, self.enabled);
            let nudged = self.nudge(ui, id);
            let op = opacity(self.enabled);
            let is_expr = self.text.trim_start().starts_with('=');
            let colour = if is_expr { t.expression } else { t.ink2 }.gamma_multiply(op);
            let inner = rect.shrink2(vec2(8.0, 0.0));
            let degree = self.unit == Some("\u{b0}");
            let unit = self
                .unit
                .filter(|_| !degree)
                .map(|u| galley(ui.painter(), u, size, fonts::mono()));
            let unit_w = match &unit {
                Some(g) => g.size().x + 6.0,
                None if degree => DEGREE_W + 6.0,
                None => 0.0,
            };
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(inner)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            let mut shown = shown_text(self.text, self.decimals, ui.memory(|m| m.has_focus(id)));
            let mut edit = TextEdit::singleline(&mut shown)
                .id(id)
                .font(FontId::new(size, fonts::mono()))
                .text_color(colour)
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .desired_width(inner.width() - unit_w)
                .interactive(self.enabled);
            if self.keep_focus {
                edit = edit.return_key(None);
            }
            let mut output = edit.show(&mut child);
            if output.response.changed() {
                *self.text = shown;
            }
            crate::text_session::after(
                &child,
                self.text,
                &output,
                self.label,
                crate::text_session::MAX_UTF16,
            );
            if native_changed {
                output.response.mark_changed();
            }
            let mut response = output.response.response;
            if nudged {
                response.mark_changed();
            }
            if degree {
                let c = pos2(inner.right() - DEGREE_W / 2.0, rect.center().y - 2.0);
                glyph::degree(ui.painter(), c, size, t.ink5.gamma_multiply(op));
            }
            if let Some(g) = unit {
                let pos = pos2(
                    inner.right() - g.size().x,
                    rect.center().y - g.size().y / 2.0,
                );
                ui.painter().galley(pos, g, t.ink5.gamma_multiply(op));
            }
            let (border, ring) = match (self.error.is_some(), response.has_focus()) {
                (true, _) => (t.danger_strong, Some(t.danger_strong.gamma_multiply(0.12))),
                (false, true) => (t.accent, Some(crate::metrics::focus_ring(&t).1)),
                (false, false) => (t.border_strong, None),
            };
            let fill = t.surface.gamma_multiply(op);
            let shape = field_shape(
                &t,
                rect,
                radius,
                fill,
                Some(border.gamma_multiply(op)),
                ring,
            );
            ui.painter().set(bg, shape);
            if let Some(msg) = self.error {
                let rt = RichText::new(msg).size(11.0).color(t.danger_strong);
                ui.add_sized(vec2(self.width, 0.0), egui::Label::new(rt).wrap());
            }
            response
        })
        .inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(
        ctx: &egui::Context,
        text: &mut String,
        focus: bool,
        events: Vec<egui::Event>,
    ) -> (bool, String) {
        let mut changed = false;
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                let id = ui.make_persistent_id(egui::IdSalt::new("value"));
                ui.memory_mut(|memory| {
                    if focus {
                        memory.request_focus(id);
                    } else {
                        memory.surrender_focus(id);
                    }
                });
                changed = ui.add(NumberField::new("value", text)).changed();
            },
        );
        out.textures_delta.clear();
        let update = out.platform_output.accesskit_update.unwrap();
        let value = update
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.role() == egui::accesskit::Role::TextInput)
                    .then(|| node.value().unwrap().to_owned())
            })
            .expect("accessible input");
        (changed, value)
    }

    #[test]
    fn rounded_idle_value_focus_and_blur_preserve_bound_precision_and_accessible_text() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let source = "50.00019918212638";
        let mut text = source.to_owned();
        for (focus, expected) in [(false, "50.00"), (true, source), (false, "50.00")] {
            let (changed, shown) = draw(&ctx, &mut text, focus, Vec::new());
            assert!(!changed);
            assert_eq!(text, source);
            assert_eq!(shown, expected);
        }
    }

    #[test]
    fn typed_precision_and_expressions_survive_display_rounding() {
        for source in ["3.141592653589793", "=width / 3"] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut text = String::new();
            draw(&ctx, &mut text, true, Vec::new());
            let (changed, shown) = draw(
                &ctx,
                &mut text,
                true,
                vec![egui::Event::Text(source.into())],
            );
            assert!(changed);
            assert_eq!(shown, source);
            assert_eq!(text, source);
            let expected = if source.starts_with('=') {
                source
            } else {
                "3.14"
            };
            let (changed, shown) = draw(&ctx, &mut text, false, Vec::new());
            assert!(!changed);
            assert_eq!(shown, expected);
            assert_eq!(text, source);
            let (changed, shown) = draw(&ctx, &mut text, true, Vec::new());
            assert!(!changed);
            assert_eq!(shown, source);
            assert_eq!(text, source);
        }
    }

    #[test]
    fn parse_value_table() {
        let cases = [
            ("14.00", ParsedValue::Number(14.0)),
            ("  -3 ", ParsedValue::Number(-3.0)),
            ("\u{2212}3", ParsedValue::Number(-3.0)),
            ("3 093.14", ParsedValue::Number(3093.14)),
            ("3\u{2009}093.14", ParsedValue::Number(3093.14)),
            ("=d1/2", ParsedValue::Expression("d1/2".into())),
            ("", ParsedValue::Invalid),
            ("abc", ParsedValue::Invalid),
            ("inf", ParsedValue::Invalid),
            ("1e999", ParsedValue::Invalid),
        ];
        for (text, want) in cases {
            assert_eq!(parse_value(text), want, "{text:?}");
        }
    }

    #[test]
    fn step_value_table() {
        assert_eq!(step_value(14.0, 1.0, false, true), 15.0);
        assert_eq!(step_value(14.0, 1.0, false, false), 13.0);
        assert_eq!(step_value(14.0, 0.5, true, true), 19.0);
        assert_eq!(step_value(0.0, 2.0, true, false), -20.0);
    }

    #[test]
    fn stepping_preserves_authored_precision_and_modifiers() {
        for initial in ["0.1234", "-0.1234", "1.234e-3"] {
            for shift in [false, true] {
                let mut text = initial.to_owned();
                for _ in 0..10 {
                    text = stepped_text(&text, 1.0, shift, true, 2).unwrap();
                }
                for _ in 0..10 {
                    text = stepped_text(&text, 1.0, shift, false, 2).unwrap();
                }
                assert_eq!(parse_value(&text), parse_value(initial));
            }
        }
        assert_eq!(
            stepped_text("0.1234", 0.00001, false, true, 2).as_deref(),
            Some("0.12341")
        );
        assert_eq!(stepped_text("=width / 2", 1.0, false, true, 2), None);
        assert_eq!(stepped_text("bad", 1.0, false, true, 2), None);
        assert_eq!(stepped_text("1e308", 1e308, false, true, 2), None);
    }
}
