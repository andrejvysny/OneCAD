//! The input queue driven through a real egui context the way `frame.rs` does:
//! a cancelled contact never clicks a button or stops a drag.
use super::{finger, nav};
use crate::ffi;
use crate::input::Input;
use egui::{Rect, Sense, pos2, vec2};
use onecad_interaction::touch::{CancelReason, RouteContext};

/// The button row is a control; fingers author on the drag area (a sketch tool).
const ROUTE: RouteContext<'static> = RouteContext {
    pencil_only_drawing: false,
    drag_authoring: true,
    over_control: &|point| point.y < 100.0,
};

fn submit(input: &mut Input, sample: ffi::TouchSample) {
    input.submit(&[sample], 0, &ROUTE).expect("batch fits");
}

#[derive(Default)]
struct Seen {
    clicked: bool,
    drag_stopped: bool,
    dragged: bool,
}

/// One frame: a button at (0,0)-(200,100) and a drag area at (0,150)-(200,250).
fn frame(ctx: &egui::Context, input: &mut Input, time: f64) -> Seen {
    let drained = input.drain();
    if drained.cancelled {
        onecad_ui::host::cancel_contact(ctx);
    }
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 300.0))),
        time: Some(time),
        events: drained.events,
        ..Default::default()
    };
    let mut seen = Seen::default();
    let mut output = ctx.run_ui(raw, |ui| {
        let button = ui.interact(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 100.0)),
            ui.scope_id().with("button"),
            Sense::click(),
        );
        let area = ui.interact(
            Rect::from_min_size(pos2(0.0, 150.0), vec2(200.0, 100.0)),
            ui.scope_id().with("area"),
            Sense::drag(),
        );
        seen.clicked = button.clicked() || area.clicked();
        seen.drag_stopped = button.drag_stopped() || area.drag_stopped();
        seen.dragged = area.dragged();
    });
    output.textures_delta.clear();
    seen
}

fn contact(phase: u32, t: f64, x: f64, y: f64) -> ffi::TouchSample {
    ffi::TouchSample {
        y_pt: y,
        ..finger(1, phase, t, x)
    }
}

#[test]
fn a_quick_tap_clicks_but_a_cancelled_one_does_not() {
    let ctx = egui::Context::default();
    let mut input = Input::default();
    frame(&ctx, &mut input, 0.0);
    submit(&mut input, contact(ffi::PHASE_BEGAN, 0.01, 50.0, 40.0));
    submit(&mut input, contact(ffi::PHASE_ENDED, 0.02, 50.0, 40.0));
    assert!(
        frame(&ctx, &mut input, 0.02).clicked,
        "the harness sees clicks"
    );
    frame(&ctx, &mut input, 0.03);

    for cancel in [ffi::PHASE_CANCELLED, ffi::PHASE_ENDED] {
        submit(&mut input, contact(ffi::PHASE_BEGAN, 1.0, 50.0, 40.0));
        assert!(!frame(&ctx, &mut input, 1.0).clicked);
        if cancel == ffi::PHASE_ENDED {
            input.cancel_all(CancelReason::SystemGesture);
        } else {
            submit(&mut input, contact(cancel, 1.05, 50.0, 40.0));
        }
        assert!(!frame(&ctx, &mut input, 1.05).clicked);
        assert!(!frame(&ctx, &mut input, 1.1).clicked);
        submit(&mut input, contact(ffi::PHASE_BEGAN, 1.2, 50.0, 40.0));
        submit(&mut input, contact(ffi::PHASE_ENDED, 1.21, 50.0, 40.0));
        assert!(
            frame(&ctx, &mut input, 1.21).clicked,
            "the next tap still clicks"
        );
        frame(&ctx, &mut input, 1.3);
    }
}

#[test]
fn a_cancelled_drag_never_reports_drag_stopped() {
    let ctx = egui::Context::default();
    let mut input = Input::default();
    frame(&ctx, &mut input, 0.0);
    submit(&mut input, contact(ffi::PHASE_BEGAN, 0.01, 50.0, 200.0));
    frame(&ctx, &mut input, 0.01);
    submit(&mut input, contact(ffi::PHASE_MOVED, 0.05, 90.0, 210.0));
    assert!(frame(&ctx, &mut input, 0.05).dragged);
    // A second finger starts a pinch: the authoring finger is cancelled.
    input.navigation(&nav(ffi::PHASE_BEGAN, 0.0, 1.0));
    for t in [0.06, 0.07, 0.08] {
        let seen = frame(&ctx, &mut input, t);
        assert!(!seen.dragged && !seen.drag_stopped && !seen.clicked);
    }
}
