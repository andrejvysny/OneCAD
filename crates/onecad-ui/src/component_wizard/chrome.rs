//! Wizard chrome: the modal frame, stepper, footer and the pick banner.
use super::{
    model::{LAST, Phase, STEPS, Saved, WizardState},
    steps,
};
use crate::library_pages::kit;
use egui::{
    Align, Align2, Color32, FontFamily, FontId, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2,
};
use onecad_app::dto::VariableDto;
use onecad_widgets::{
    components::button::{Button, ButtonKind},
    fonts,
    theme::look,
};

const SIZE: egui::Vec2 = vec2(760.0, 600.0);
const HEADER_H: f32 = 88.0;
const FOOTER_H: f32 = 68.0;

pub(crate) enum Outcome {
    Stay,
    Close,
    OpenInLibrary(Saved),
}

pub(crate) fn modal(
    ctx: &egui::Context,
    w: &mut WizardState,
    variables: &[VariableDto],
) -> Outcome {
    let mut outcome = Outcome::Stay;
    let response = egui::Modal::new(egui::Id::unique("create-component-wizard"))
        .frame(kit::modal_frame(ctx))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let (rect, _) = ui.allocate_exact_size(SIZE, Sense::hover());
            let header = Rect::from_min_size(rect.min, vec2(SIZE.x, HEADER_H));
            let footer = Rect::from_min_max(pos2(rect.left(), rect.bottom() - FOOTER_H), rect.max);
            let body = Rect::from_min_max(
                pos2(rect.left() + 22.0, header.bottom() + 16.0),
                pos2(rect.right() - 22.0, footer.top() - 10.0),
            );
            if header_row(ui, header, w) {
                outcome = Outcome::Close;
            }
            ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| steps::show(ui, w, variables));
            });
            if let Some(next) = footer_row(ui, footer, w, variables) {
                outcome = next;
            }
        });
    if response.should_close() && !w.busy() {
        return Outcome::Close;
    }
    outcome
}

/// While picking the modal steps aside so the body is clickable.
pub(crate) fn pick_banner(ctx: &egui::Context, w: &mut WizardState) -> Outcome {
    let t = look(ctx).theme;
    egui::Area::new(egui::Id::unique("component-pick-banner"))
        .anchor(Align2::CENTER_TOP, vec2(0.0, 72.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.surface)
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let text = format!(
                            "Click a planar face, a cylinder or a circular edge of {} to name \"{}\"",
                            w.body_name, w.new_attachment
                        );
                        kit::label(ui, &text, 12.5, FontFamily::Proportional, t.ink);
                        if ui.add(Button::new("Cancel pick")).clicked() {
                            w.cancel_pick();
                        }
                    });
                });
        });
    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    Outcome::Stay
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

fn footer_row(
    ui: &mut Ui,
    rect: Rect,
    w: &mut WizardState,
    variables: &[VariableDto],
) -> Option<Outcome> {
    let t = look(ui.ctx()).theme;
    let mut outcome = None;
    ui.painter()
        .hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, t.border));
    ui.scope_builder(
        UiBuilder::new().max_rect(rect.shrink2(vec2(22.0, 0.0))),
        |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.set_height(rect.height());
                let saved = matches!(w.phase, Phase::Saved(_));
                let back = Button::new("Back")
                    .kind(ButtonKind::Ghost)
                    .enabled(w.step > 0 && w.phase == Phase::Editing);
                if ui.add(back).clicked() {
                    w.back();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    outcome = primary(ui, w, variables);
                    if !saved && !w.busy() && ui.add(Button::new("Cancel")).clicked() {
                        outcome = Some(Outcome::Close);
                    }
                    if saved && ui.add(Button::new("Done")).clicked() {
                        outcome = Some(Outcome::Close);
                    }
                });
            });
        },
    );
    outcome
}

/// The primary button, with the reason it is off written beside it.
fn primary(ui: &mut Ui, w: &mut WizardState, variables: &[VariableDto]) -> Option<Outcome> {
    if let Phase::Saved(saved) = &w.phase {
        let open = ui.add(Button::new("Open in library").kind(ButtonKind::Primary));
        return open
            .clicked()
            .then(|| Outcome::OpenInLibrary(saved.clone()));
    }
    let last = w.step == LAST;
    let blocker = if last {
        w.save_blocker()
    } else {
        w.blocker(w.step)
    };
    let label = match (last, w.union_offer, w.phase.clone()) {
        (_, _, Phase::Saving) => "Saving…".to_owned(),
        (true, true, _) => "Union solids and save".to_owned(),
        (true, false, _) => "Save component".to_owned(),
        (false, ..) => format!("Next: {}", STEPS[w.step + 1]),
    };
    let enabled = blocker.is_none() && w.phase == Phase::Editing;
    let button = Button::new(&label)
        .kind(ButtonKind::Primary)
        .enabled(enabled);
    let response = ui.add(button);
    if let Some(reason) = &blocker {
        let warn = look(ui.ctx()).theme.warn;
        kit::label(ui, reason, 12.0, FontFamily::Proportional, warn);
    }
    if response.clicked() {
        if last {
            w.union_solids |= w.union_offer;
            w.request_save(variables);
        } else {
            w.advance();
        }
    }
    None
}
