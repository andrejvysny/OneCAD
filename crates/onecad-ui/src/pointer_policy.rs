//! Viewport starts obey chrome ownership; captured gestures retain their releases.

pub fn gesture_allowed(ui: &egui::Ui) -> bool {
    ui.is_enabled()
        && !egui::Popup::is_any_open(ui.ctx())
        && ui.input(|input| input.focused && !input.key_pressed(egui::Key::Escape))
}

pub fn allows_pointer(ui: &egui::Ui, rect: egui::Rect, position: egui::Pos2) -> bool {
    gesture_allowed(ui)
        && rect.contains(position)
        && ui
            .ctx()
            .layer_id_at(position)
            .is_none_or(|layer| layer == ui.layer_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_layer_chrome_blocks_hover_and_new_gestures() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let chrome = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 200.0));
        for frame in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect),
                    focused: true,
                    ..Default::default()
                },
                |ui| {
                    egui::Area::new(egui::Id::unique("panel"))
                        .fixed_pos(chrome.min)
                        .show(ui.ctx(), |ui| {
                            ui.allocate_exact_size(chrome.size(), egui::Sense::hover());
                        });
                    egui::CentralPanel::default().show(ui, |ui| {
                        if frame > 0 {
                            assert!(!allows_pointer(ui, rect, chrome.center()));
                            assert!(allows_pointer(ui, rect, egui::pos2(600.0, 400.0)));
                            assert!(gesture_allowed(ui));
                        }
                    });
                },
            );
            output.textures_delta.clear();
        }
    }

    #[test]
    fn focus_loss_escape_and_popup_cancel_gesture_permission() {
        for condition in 0..3 {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    focused: condition != 0,
                    events: if condition == 1 {
                        vec![egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        }]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                },
                |ui| {
                    if condition == 2 {
                        egui::Popup::open_id(ui.ctx(), egui::Id::unique("popup"));
                    }
                    assert!(!gesture_allowed(ui));
                },
            );
            output.textures_delta.clear();
        }
    }
}
