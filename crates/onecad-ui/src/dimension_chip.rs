//! Shared compact numeric surfaces; placement and measurements are presentation-only.
use egui::{Pos2, Rect, Vec2};

pub(crate) const WIDTH: f32 = 194.0;

pub(crate) fn usable(ctx: &egui::Context, state: &crate::ui_state::UiState) -> Rect {
    let chrome = crate::layout::chrome(ctx.content_rect(), state);
    let mut bounds = chrome.viewport.shrink(8.0);
    bounds.min.y += 72.0;
    bounds.max.y -= 42.0;
    if state.inspector_open {
        bounds.max.x = chrome.inspector.left() - 8.0;
    }
    if state.explorer_open {
        bounds.min.x = chrome.explorer.right() + 8.0;
    }
    bounds
}

pub(crate) fn place(anchor: Pos2, size: Vec2, bounds: Rect) -> Pos2 {
    let preferred = anchor + egui::vec2(24.0, 20.0);
    let mut position = preferred;
    if position.x + size.x > bounds.right() {
        position.x = anchor.x - size.x - 24.0;
    }
    if position.y + size.y > bounds.bottom() {
        position.y = anchor.y - size.y - 20.0;
    }
    let position = egui::pos2(
        position
            .x
            .clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left())),
        position
            .y
            .clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top())),
    );
    if !Rect::from_min_size(position, size).contains(anchor) {
        return position;
    }
    let y = if anchor.y - size.y - 20.0 >= bounds.top() {
        anchor.y - size.y - 20.0
    } else {
        anchor.y + 20.0
    };
    egui::pos2(
        position.x,
        y.clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top())),
    )
}

pub(crate) fn frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::popup(style).inner_margin(6.0)
}

pub(crate) fn error(ui: &mut egui::Ui, error: Option<&String>) {
    if let Some(error) = error {
        ui.add(
            egui::Label::new(
                egui::RichText::new(error)
                    .small()
                    .color(ui.visuals().error_fg_color),
            )
            .truncate(),
        )
        .on_hover_text(error);
    }
}

pub(crate) fn witness(ctx: &egui::Context, id: egui::Id, anchor: Pos2, bounds: Rect) {
    let endpoint = bounds.clamp(anchor);
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, id))
        .line_segment(
            [anchor, endpoint],
            egui::Stroke::new(1.0, ctx.global_style().visuals.weak_text_color()),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chip_error_stays_compact_and_owns_pointer_in_both_themes() {
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            let ctx = egui::Context::default();
            ctx.set_visuals(visuals);
            ctx.set_pixels_per_point(2.0);
            let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 560.0));
            let safe = Rect::from_min_max(egui::pos2(268.0, 124.0), egui::pos2(594.0, 510.0));
            let position = place(safe.right_bottom(), egui::vec2(WIDTH, 104.0), safe);
            let mut text = "width * 2".to_owned();
            for frame_index in 0..3 {
                let mut output = ctx.run_ui(egui::RawInput {
                    screen_rect: Some(screen), focused: true,
                    events: vec![egui::Event::PointerMoved(position + egui::vec2(20.0, 15.0))],
                    ..Default::default()
                }, |ui| {
                    let chip = egui::Area::new(egui::Id::unique("chip-test"))
                        .order(egui::Order::Foreground).fixed_pos(position).show(ui.ctx(), |ui| {
                            frame(ui.style()).show(ui, |ui| {
                                ui.set_width(WIDTH - 12.0);
                                ui.add(egui::TextEdit::singleline(&mut text).desired_width(WIDTH - 14.0));
                                error(ui, Some(&"Invalid expression with a very long explanation that must not enlarge this field".to_owned()));
                                ui.small_button("Cancel").clicked();
                            });
                        });
                    assert!(chip.response.rect.width() <= WIDTH + 2.0);
                    assert!(chip.response.rect.height() <= 104.0);
                    assert!(safe.contains_rect(chip.response.rect));
                    if frame_index > 0 {
                        assert!(!crate::viewport_widgets::allows_pointer(ui, screen, position + egui::vec2(20.0, 15.0)));
                    }
                });
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn compact_chips_fit_small_viewport_edges_without_covering_anchor() {
        let bounds = Rect::from_min_max(egui::pos2(268.0, 124.0), egui::pos2(594.0, 510.0));
        let size = egui::vec2(WIDTH, 104.0);
        for anchor in [
            bounds.left_top(),
            bounds.right_top(),
            bounds.left_bottom(),
            bounds.right_bottom(),
            bounds.center(),
        ] {
            let chip = Rect::from_min_size(place(anchor, size, bounds), size);
            assert!(bounds.contains_rect(chip));
            assert!(!chip.contains(anchor));
        }
    }
}
