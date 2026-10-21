//! TabClosePopover: "Save changes to X?" card anchored under a document tab.
//! Not modal (no scrim); the host decides what is blocked while it is open.
//! Escape reports `cancel`, Enter reports `save`.

use egui::{
    Align, Context, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Modifiers, Order, Rect,
    RichText, Stroke, Ui, Vec2, pos2,
};

use super::dialog::link;
use crate::components::button::{Button, ButtonKind};
use crate::fonts;
use crate::metrics::{Elevation, PANEL_TITLE, RADIUS_LG, SECONDARY, shadow};
use crate::theme::look;

const WIDTH: f32 = 352.0;
const PAD: i8 = 14;
const OFFSET: f32 = 6.0;
const EDGE: f32 = 8.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TabCloseOutcome {
    pub save: bool,
    pub discard: bool,
    pub cancel: bool,
}

pub struct TabClosePopover<'a> {
    title: &'a str,
    body: Option<&'a str>,
    anchor: Rect,
    saving: bool,
}

impl<'a> TabClosePopover<'a> {
    pub fn new(title: &'a str, anchor: Rect) -> Self {
        Self {
            title,
            body: None,
            anchor,
            saving: false,
        }
    }
    /// e.g. "3 changes since last save: Mate 4, Gear pair, Housing offset."
    pub fn body(mut self, body: Option<&'a str>) -> Self {
        self.body = body;
        self
    }
    /// Disables "Save and close" while a save is in flight.
    pub fn saving(mut self, saving: bool) -> Self {
        self.saving = saving;
        self
    }

    pub fn show(self, ctx: &Context, id: Id) -> TabCloseOutcome {
        let l = look(ctx);
        let t = &l.theme;
        let frame = Frame::new()
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.border_subtle))
            .corner_radius(CornerRadius::same(RADIUS_LG))
            .shadow(shadow(Elevation::Card, t, l.dark))
            .inner_margin(Margin::same(PAD));
        let screen = ctx.content_rect();
        let max_x = (screen.right() - WIDTH - EDGE).max(EDGE);
        let pos = pos2(
            self.anchor.left().clamp(EDGE, max_x),
            self.anchor.bottom() + OFFSET,
        );
        let enter = !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let mut out = TabCloseOutcome::default();
        crate::frame::floating(id, pos, Order::Foreground).show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_width(WIDTH - 2.0 * f32::from(PAD) - 2.0);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                self.content(ui);
                ui.add_space(14.0);
                self.footer(ui, &mut out);
            });
        });
        out.cancel |= escape;
        out.save |= enter && !self.saving;
        out
    }

    fn content(&self, ui: &mut Ui) {
        let l = look(ui.ctx());
        let t = &l.theme;
        let title = format!("Save changes to {}?", self.title);
        ui.label(
            RichText::new(title)
                .font(FontId::new(PANEL_TITLE, fonts::semibold()))
                .color(t.ink),
        );
        if let Some(body) = self.body {
            ui.add_space(4.0);
            ui.label(RichText::new(body).size(SECONDARY).color(t.ink3));
        }
    }

    fn footer(&self, ui: &mut Ui, out: &mut TabCloseOutcome) {
        let l = look(ui.ctx());
        ui.horizontal(|ui| {
            ui.set_min_height(30.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            out.discard = link(ui, "Don't save", l.theme.danger_strong, true);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let save = Button::new("Save and close")
                    .kind(ButtonKind::Primary)
                    .enabled(!self.saving);
                out.save = ui.add(save).clicked();
                out.cancel = ui.add(Button::new("Cancel")).clicked();
            });
        });
    }
}
