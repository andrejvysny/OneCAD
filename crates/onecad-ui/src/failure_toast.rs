//! Failure-only toast: a newly recorded failed outcome shows bottom-right for a few
//! seconds. Successes and in-progress states never surface; the full record stays in
//! `Diagnostics`.
use crate::diagnostics::{Diagnostics, Failure, summary};
use egui::{Align2, Context, Id, Order, RichText, vec2};
use onecad_widgets::{
    components::button::{Button, ButtonKind},
    metrics::{Elevation, shadow},
    theme::look,
};

const LIFETIME: f64 = 6.0;
const INSET: f32 = 12.0;

/// Which failure the toast last took up and when; it is shown once per failure id.
#[derive(Clone, Copy)]
struct Shown {
    id: u64,
    since: f64,
    dismissed: bool,
}

fn state_id() -> Id {
    Id::unique("failure-toast-state")
}

pub(crate) fn show(ctx: &Context, diagnostics: &Diagnostics) {
    let Some(failure) = diagnostics.latest_failure() else {
        return;
    };
    let now = ctx.input(|i| i.time);
    let mut shown = ctx
        .data(|d| d.get_temp::<Shown>(state_id()))
        .filter(|shown| shown.id == failure.id)
        .unwrap_or(Shown {
            id: failure.id,
            since: now,
            dismissed: false,
        });
    if !shown.dismissed && now - shown.since >= LIFETIME {
        shown.dismissed = true;
    }
    if !shown.dismissed {
        shown.dismissed = card(ctx, failure);
        let left = (LIFETIME - (now - shown.since)).max(0.0);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), shown));
}

/// Draws the toast; true when it was dismissed (body click or Copy).
fn card(ctx: &Context, failure: &Failure) -> bool {
    let l = look(ctx);
    let mut copy = false;
    let area = egui::Area::new(Id::unique("failure-toast"))
        .order(Order::Tooltip)
        .movable(false)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-INSET, -INSET))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(l.theme.tooltip)
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .shadow(shadow(Elevation::Tooltip, &l.theme, l.dark))
                .show(ui, |ui| {
                    ui.set_max_width(340.0);
                    let text = format!("{}: {}", failure.source.label(), summary(&failure.message));
                    ui.add(
                        egui::Label::new(
                            RichText::new(text).size(13.0).color(l.theme.tooltip_text),
                        )
                        .selectable(false),
                    );
                    copy = ui
                        .add(Button::new("Copy").kind(ButtonKind::Secondary))
                        .clicked();
                });
        });
    if copy {
        ctx.copy_text(failure.details.clone());
    }
    let body_click = ctx.input(|i| {
        i.pointer.any_click()
            && i.pointer
                .interact_pos()
                .is_some_and(|pos| area.response.rect.contains(pos))
    });
    copy || body_click
}

#[cfg(test)]
#[path = "failure_toast_tests.rs"]
mod tests;
