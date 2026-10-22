//! Viewport starts obey chrome ownership; captured gestures retain their releases.

pub fn gesture_allowed(ui: &egui::Ui) -> bool {
    ui.is_enabled()
        && !egui::Popup::is_any_open(ui.ctx())
        && ui.input(|input| input.focused && !input.key_pressed(egui::Key::Escape))
}

/// Which input drives the primary pointer; picking widens the edge aperture
/// for a finger. Desktop never sets it, so mouse picking keeps its 8 points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerKind {
    /// Mouse, trackpad pointer or Pencil.
    #[default]
    Precise,
    Finger,
}

/// Edge pick aperture per input kind, in points. Scene picking measures in
/// logical pixels, which are egui points, so no further scaling applies.
pub const PRECISE_EDGE_TOLERANCE_PT: f64 = onecad_scene::EDGE_PICK_LOGICAL_PIXELS;
pub const FINGER_EDGE_TOLERANCE_PT: f64 = 22.0;

fn pointer_kind_id() -> egui::Id {
    egui::Id::unique("onecad-pointer-kind")
}

pub fn set_pointer_kind(ctx: &egui::Context, kind: PointerKind) {
    ctx.data_mut(|data| data.insert_temp(pointer_kind_id(), kind));
}

fn pencil_double_tap_id() -> egui::Id {
    egui::Id::unique("onecad-pencil-double-tap")
}

/// INPUT.md: a Pencil tip's second tap (`tap_count` 2) promotes like a double
/// click even when egui's click timing does not count it. Set per press.
pub fn set_pencil_double_tap(ctx: &egui::Context, double: bool) {
    ctx.data_mut(|data| data.insert_temp(pencil_double_tap_id(), double));
}

pub fn pencil_double_tap(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(pencil_double_tap_id()))
        .unwrap_or(false)
}

pub fn edge_tolerance(ctx: &egui::Context) -> f64 {
    match ctx.data(|data| data.get_temp::<PointerKind>(pointer_kind_id())) {
        Some(PointerKind::Finger) => FINGER_EDGE_TOLERANCE_PT,
        Some(PointerKind::Precise) | None => PRECISE_EDGE_TOLERANCE_PT,
    }
}

#[derive(Clone, Copy)]
struct Canvas {
    rect: egui::Rect,
    layer: egui::LayerId,
    pass: u64,
}

fn canvas_id() -> egui::Id {
    egui::Id::unique("onecad-touch-canvas")
}

/// Records where the CAD canvas was drawn this pass, for touch routing between
/// frames.
pub fn capture_canvas(ctx: &egui::Context, rect: egui::Rect, layer: egui::LayerId) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(canvas_id(), Canvas { rect, layer, pass }));
}

/// INPUT.md `over_control`, from the last completed pass: a point is a control
/// unless it is on the CAD canvas with no higher egui layer over it. Without a
/// canvas that pass (start screen, no document) everything is a control.
pub fn over_control(ctx: &egui::Context, position: egui::Pos2) -> bool {
    let current = ctx.cumulative_pass_nr();
    let canvas = ctx
        .data(|data| data.get_temp::<Canvas>(canvas_id()))
        .filter(|canvas| canvas.pass + 1 >= current);
    !canvas.is_some_and(|canvas| {
        canvas.rect.contains(position)
            && ctx
                .layer_id_at(position)
                .is_none_or(|layer| layer == canvas.layer)
    })
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
    fn finger_presses_widen_the_edge_aperture_and_mouse_keeps_eight_points() {
        let ctx = egui::Context::default();
        assert_eq!(edge_tolerance(&ctx), 8.0, "desktop default");
        set_pointer_kind(&ctx, PointerKind::Finger);
        assert_eq!(edge_tolerance(&ctx), FINGER_EDGE_TOLERANCE_PT);
        set_pointer_kind(&ctx, PointerKind::Precise);
        assert_eq!(edge_tolerance(&ctx), 8.0);
    }

    #[test]
    fn a_pencil_double_tap_mark_lasts_until_the_next_press_sets_it() {
        let ctx = egui::Context::default();
        assert!(!pencil_double_tap(&ctx), "desktop never sets it");
        set_pencil_double_tap(&ctx, true);
        assert!(pencil_double_tap(&ctx));
        set_pencil_double_tap(&ctx, false);
        assert!(!pencil_double_tap(&ctx));
    }

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
    fn touch_controls_are_chrome_or_anything_without_a_canvas_last_pass() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let canvas = egui::Rect::from_min_max(egui::pos2(0.0, 50.0), egui::pos2(800.0, 600.0));
        let chrome = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 200.0));
        let run = |draw_canvas: bool| {
            let raw = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                egui::Area::new(egui::Id::unique("chrome"))
                    .fixed_pos(chrome.min)
                    .show(ui.ctx(), |ui| {
                        ui.allocate_exact_size(chrome.size(), egui::Sense::click());
                    });
                egui::CentralPanel::default().show(ui, |ui| {
                    if draw_canvas {
                        capture_canvas(ui.ctx(), canvas, ui.layer_id());
                    }
                });
            });
            output.textures_delta.clear();
        };
        run(true);
        run(true);
        assert!(over_control(&ctx, chrome.center()), "floating chrome");
        assert!(!over_control(&ctx, egui::pos2(600.0, 400.0)), "bare canvas");
        assert!(over_control(&ctx, egui::pos2(600.0, 20.0)), "title bar");
        run(false);
        assert!(
            over_control(&ctx, egui::pos2(600.0, 400.0)),
            "no canvas drawn last pass"
        );
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
