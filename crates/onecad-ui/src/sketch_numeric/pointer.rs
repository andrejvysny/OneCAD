use egui::{Pos2, Rect, Ui};

pub(super) fn canvas_position(ui: &Ui, rect: Rect, blocked: bool) -> Option<Pos2> {
    let position = ui.input(|input| input.pointer.hover_pos())?;
    if blocked || !crate::viewport_widgets::allows_pointer(ui, rect, position) {
        return None;
    }
    if ui.input(|input| input.pointer.primary_pressed()) {
        ui.ctx().memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
    }
    (!ui.ctx().text_edit_focused() && ui.input(|input| input.pointer.delta() != egui::Vec2::ZERO))
        .then_some(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_other_overlays_block_motion_until_canvas_takes_focus() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 560.0));
        let mut text = "80".to_owned();
        for frame in 0..4 {
            let position = egui::pos2(500.0 + frame as f32 * 10.0, 300.0);
            let mut events = vec![egui::Event::PointerMoved(position)];
            if frame == 2 {
                events.push(egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ui| {
                    let field = ui.text_edit_singleline(&mut text);
                    if frame == 0 {
                        field.request_focus()
                    }
                    let motion = canvas_position(ui, rect, frame == 3);
                    if frame == 2 {
                        assert_eq!(motion, Some(position))
                    } else {
                        assert_eq!(motion, None)
                    }
                },
            );
            output.textures_delta.clear();
        }
    }
}
