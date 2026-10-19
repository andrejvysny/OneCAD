//! Painting helpers shared by the action, list and input components.

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Galley, Painter, Rect, Sense, Stroke, StrokeKind,
};

use crate::theme::Theme;

pub(crate) fn sense(enabled: bool, click: Sense) -> Sense {
    if enabled { click } else { Sense::hover() }
}

/// Disabled controls render at 40% (Button's rule).
pub(crate) fn opacity(enabled: bool) -> f32 {
    if enabled { 1.0 } else { 0.4 }
}

/// Focus ring: the 3px accent halo from `metrics::focus_ring`, drawn outside `rect`.
pub(crate) fn focus_ring(p: &Painter, t: &Theme, rect: Rect, radius: u8) {
    let (w, c) = crate::metrics::focus_ring(t);
    p.rect_stroke(
        rect.expand(w * 0.5),
        CornerRadius::same(radius + 2),
        Stroke::new(w, c),
        StrokeKind::Outside,
    );
}

/// 1px border drawn inside `rect` (CSS `inset 0 0 0 1px`).
pub(crate) fn inset(p: &Painter, rect: Rect, radius: u8, color: Color32) {
    p.rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, color),
        StrokeKind::Inside,
    );
}

/// Single-line galley in the placeholder colour; the colour is supplied at paint time.
pub(crate) fn galley(p: &Painter, text: &str, size: f32, family: FontFamily) -> Arc<Galley> {
    p.layout_no_wrap(
        text.to_owned(),
        FontId::new(size, family),
        Color32::PLACEHOLDER,
    )
}

/// Single-line galley ellipsised to `max_w`.
pub(crate) fn fitted(
    p: &Painter,
    text: &str,
    font: FontId,
    colour: Color32,
    max_w: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, colour);
    job.wrap = TextWrapping {
        max_width: max_w.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    p.layout_job(job)
}

/// Run `f` in a headless egui frame with the OneCAD look installed.
#[cfg(test)]
pub(crate) fn headless(dark: bool, events: Vec<egui::Event>, f: impl FnMut(&mut egui::Ui)) {
    let ctx = egui::Context::default();
    crate::fonts::install(&ctx);
    if let Ok(t) = crate::theme::load(dark) {
        crate::theme::install(&ctx, t, dark);
    }
    let raw = |events| egui::RawInput {
        events,
        screen_rect: Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 600.0),
        )),
        ..Default::default()
    };
    // Fonts registered by `install` apply from the next pass.
    // Unapplied texture deltas panic on drop in debug builds.
    ctx.run_ui(raw(Vec::new()), |_| {}).textures_delta.clear();
    ctx.run_ui(raw(events), f).textures_delta.clear();
}
