//! Create-component wizard (Final 12a): centred modal, UI facade over mock selection data.
use crate::{app::App, library_pages::kit, ui_state::UiState};
use egui::{
    Align, Align2, Color32, FontFamily, FontId, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2,
};
use onecad_widgets::{
    components::button::{Button, ButtonKind},
    fonts,
    theme::look,
};

mod steps;

const SIZE: egui::Vec2 = vec2(760.0, 560.0);
const HEADER_H: f32 = 88.0;
const FOOTER_H: f32 = 68.0;
pub const STEPS: [&str; 4] = ["Source", "Details", "Parameters", "Save"];
pub const CATEGORIES: [&str; 3] = ["Brackets", "Fasteners", "Enclosures"];
pub const DESTINATIONS: [&str; 2] = ["My components", "This project"];
/// Mock dimensions the Parameters step offers: name, value.
pub const DIMENSIONS: [(&str, &str); 3] = [
    ("width", "60 mm"),
    ("height", "40 mm"),
    ("thickness", "4 mm"),
];

#[derive(Clone, Debug)]
pub struct WizardState {
    pub step: usize,
    pub name: String,
    pub category: usize,
    pub save_to: usize,
    pub description: String,
    /// 0 Static, 1 Parametric.
    pub kind: usize,
    pub body: bool,
    pub sketches: bool,
    pub exposed: [bool; 3],
    pub retaken: bool,
    pub saved: bool,
}

impl WizardState {
    pub fn new() -> Self {
        Self {
            step: 0,
            name: "Mounting bracket".into(),
            category: 0,
            save_to: 0,
            description: "L-bracket with two slotted mounting holes per leg.".into(),
            kind: 1,
            body: true,
            sketches: true,
            exposed: [true, true, false],
            retaken: false,
            saved: false,
        }
    }

    pub fn primary_label(&self) -> String {
        match (self.step, self.saved) {
            (3, false) => "Save component".into(),
            (3, true) => "Done".into(),
            (step, _) => format!("Next: {}", STEPS[step + 1]),
        }
    }

    /// Primary footer action; true when the wizard should close.
    pub fn advance(&mut self) -> bool {
        if self.step < 3 {
            self.step += 1;
            false
        } else if !self.saved {
            self.saved = true;
            false
        } else {
            true
        }
    }

    pub fn back(&mut self) {
        if !self.saved {
            self.step = self.step.saturating_sub(1);
        }
    }

    pub fn exposed_count(&self) -> usize {
        if self.kind == 1 {
            self.exposed.iter().filter(|e| **e).count()
        } else {
            0
        }
    }
}

impl Default for WizardState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn open(ui: &mut UiState) {
    ui.library.wizard = Some(WizardState::new());
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    show_state(&mut app.ui, ctx);
}

pub(crate) fn show_state(state: &mut UiState, ctx: &egui::Context) {
    let Some(mut w) = state.library.wizard.take() else {
        return;
    };
    let mut close = false;
    let response = egui::Modal::new(egui::Id::unique("create-component-wizard"))
        .frame(kit::modal_frame(ctx))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let (rect, _) = ui.allocate_exact_size(SIZE, Sense::hover());
            let header = Rect::from_min_size(rect.min, vec2(SIZE.x, HEADER_H));
            let footer = Rect::from_min_max(pos2(rect.left(), rect.bottom() - FOOTER_H), rect.max);
            let body = Rect::from_min_max(
                pos2(rect.left() + 22.0, header.bottom() + 20.0),
                pos2(rect.right() - 22.0, footer.top() - 12.0),
            );
            close |= header_row(ui, header, &w);
            ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                steps::show(ui, &mut w)
            });
            close |= footer_row(ui, footer, &mut w);
        });
    if close || response.should_close() {
        return;
    }
    state.library.wizard = Some(w);
}

fn header_row(ui: &mut Ui, rect: Rect, w: &WizardState) -> bool {
    let t = look(ui.ctx()).theme;
    let mut close = false;
    let p = ui.painter();
    let font = FontId::new(17.0, fonts::semibold());
    p.text(
        pos2(rect.left() + 22.0, rect.top() + 28.0),
        Align2::LEFT_CENTER,
        "Create component",
        font,
        t.ink,
    );
    p.hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0, t.border),
    );
    let x = Rect::from_min_size(
        pos2(rect.right() - 22.0 - 28.0, rect.top() + 14.0),
        vec2(28.0, 28.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(x), |ui| {
        close = kit::icon_button(ui, "close", 28.0, false).clicked();
    });
    stepper(ui, pos2(rect.left() + 22.0, rect.top() + 62.0), w.step);
    close
}

/// Done = green check, current = accent circle, todo = grey circle.
fn stepper(ui: &mut Ui, origin: egui::Pos2, current: usize) {
    let l = look(ui.ctx());
    let t = &l.theme;
    let mut x = origin.x;
    for (i, name) in STEPS.iter().enumerate() {
        let c = pos2(x + 11.0, origin.y);
        let p = ui.painter();
        let (fill, ink, family) = match i.cmp(&current) {
            std::cmp::Ordering::Less => (
                t.ok_text.gamma_multiply(0.15),
                t.ok_text,
                FontFamily::Proportional,
            ),
            std::cmp::Ordering::Equal => (t.accent, t.on_accent, fonts::semibold()),
            std::cmp::Ordering::Greater => (t.well, t.ink5, FontFamily::Proportional),
        };
        p.circle_filled(c, 11.0, fill);
        if i < current {
            kit::paint_icon(
                p,
                Rect::from_center_size(c, vec2(14.0, 14.0)),
                "check",
                Color32::TRANSPARENT,
                t.ok_text,
            );
        } else {
            p.text(
                c,
                Align2::CENTER_CENTER,
                i + 1,
                FontId::new(11.5, fonts::semibold()),
                ink,
            );
        }
        let label_ink = if i == current { t.ink } else { t.ink4 };
        let g = p.layout_no_wrap((*name).to_owned(), FontId::new(12.5, family), label_ink);
        let width = g.size().x;
        p.galley(pos2(c.x + 11.0 + 8.0, c.y - g.size().y / 2.0), g, label_ink);
        x = c.x + 11.0 + 8.0 + width;
        if i + 1 < STEPS.len() {
            p.hline(
                x + 12.0..=x + 44.0,
                c.y,
                egui::Stroke::new(1.0, t.border_strong),
            );
            x += 56.0;
        }
    }
}

fn footer_row(ui: &mut Ui, rect: Rect, w: &mut WizardState) -> bool {
    let t = look(ui.ctx()).theme;
    let mut close = false;
    ui.painter()
        .hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, t.border));
    ui.scope_builder(
        UiBuilder::new().max_rect(rect.shrink2(vec2(22.0, 0.0))),
        |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.set_height(rect.height());
                let back = Button::new("Back")
                    .kind(ButtonKind::Ghost)
                    .enabled(w.step > 0 && !w.saved);
                if ui.add(back).clicked() {
                    w.back();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let label = w.primary_label();
                    if ui
                        .add(Button::new(&label).kind(ButtonKind::Primary))
                        .clicked()
                    {
                        close |= w.advance();
                    }
                    if !w.saved && ui.add(Button::new("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        },
    );
    close
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wizard_walks_all_steps_then_saves_and_closes() {
        let mut ui = UiState::default();
        open(&mut ui);
        let w = ui.library.wizard.as_mut().expect("open sets the wizard");
        assert_eq!(w.primary_label(), "Next: Details");
        assert!(!w.advance());
        assert_eq!(w.primary_label(), "Next: Parameters");
        assert!(!w.advance() && !w.advance());
        assert_eq!((w.step, w.primary_label().as_str()), (3, "Save component"));
        assert!(!w.advance());
        assert!(w.saved && w.primary_label() == "Done");
        w.back();
        assert_eq!(w.step, 3);
        assert!(w.advance());
    }

    #[test]
    fn back_stops_at_source_and_static_exposes_nothing() {
        let mut w = WizardState::new();
        w.back();
        assert_eq!(w.step, 0);
        assert_eq!(w.exposed_count(), 2);
        w.kind = 0;
        assert_eq!(w.exposed_count(), 0);
    }
}
