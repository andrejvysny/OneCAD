//! Routed control contacts drive egui's default touch scrolling without canvas authoring.
use super::*;
use egui::{Rect, Sense, pos2, vec2};

const CONTROL: RouteContext<'static> = RouteContext {
    pencil_only_drawing: true,
    drag_authoring: true,
    over_control: &|_| true,
};

fn sample(contact: u64, kind: u32, phase: u32, time: f64, y: f64) -> TouchSample {
    TouchSample {
        contact_id: contact,
        kind,
        phase,
        provenance: ffi::SAMPLE_ACTUAL,
        timestamp_s: time,
        x_pt: 80.0,
        y_pt: y,
        tap_count: 1,
        ..Default::default()
    }
}

fn finger(input: &mut Input, phase: u32, time: f64, y: f64) {
    input
        .submit(
            &[sample(7, ffi::CONTACT_FINGER, phase, time, y)],
            0,
            &CONTROL,
        )
        .unwrap();
}

#[derive(Default)]
struct Seen {
    offset: f32,
    screen: bool,
    touching: bool,
    clicked: bool,
    dragged: bool,
    drag_stopped: bool,
    events: Vec<Event>,
}

fn raw(ctx: &egui::Context, input: &mut Input, time: f64) -> egui::RawInput {
    let drained = input.drain();
    if drained.cancelled {
        onecad_ui::host::cancel_contact(ctx);
    }
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 300.0))),
        events: drained.events,
        time: Some(time),
        ..Default::default()
    }
}

fn scroll_frame(ctx: &egui::Context, input: &mut Input, time: f64) -> Seen {
    let mut seen = Seen::default();
    let mut output = ctx.run_ui(raw(ctx, input, time), |ui| {
        let scroll = egui::ScrollArea::vertical()
            .max_height(180.0)
            .show(ui, |ui| {
                ui.allocate_exact_size(vec2(200.0, 1200.0), Sense::hover());
            });
        seen.offset = scroll.state.clamped_offset().y;
        (seen.screen, seen.touching) =
            ui.input(|input| (input.has_touch_screen(), input.any_touches()));
    });
    output.textures_delta.clear();
    seen
}

#[test]
fn finger_control_drag_scrolls_blank_content_with_default_egui_settings() {
    let (ctx, mut input) = (egui::Context::default(), Input::default());
    scroll_frame(&ctx, &mut input, 0.0);
    // egui registers its OnTouch drag widget in the pass after the first real touch.
    finger(&mut input, ffi::PHASE_BEGAN, 0.01, 140.0);
    scroll_frame(&ctx, &mut input, 0.01);
    finger(&mut input, ffi::PHASE_ENDED, 0.02, 140.0);
    scroll_frame(&ctx, &mut input, 0.02);
    finger(&mut input, ffi::PHASE_BEGAN, 0.1, 140.0);
    let began = scroll_frame(&ctx, &mut input, 0.1);
    finger(&mut input, ffi::PHASE_MOVED, 0.2, 100.0);
    scroll_frame(&ctx, &mut input, 0.2);
    finger(&mut input, ffi::PHASE_MOVED, 0.3, 50.0);
    let moved = scroll_frame(&ctx, &mut input, 0.3);
    assert!(
        moved.offset > 30.0,
        "default touch scrolling must move content: {}",
        moved.offset
    );
    assert!(began.screen && began.touching && moved.touching);
    finger(&mut input, ffi::PHASE_ENDED, 0.4, 50.0);
    assert!(!scroll_frame(&ctx, &mut input, 0.4).touching);
}

fn button_frame(ctx: &egui::Context, input: &mut Input, time: f64) -> Seen {
    let raw = raw(ctx, input, time);
    let mut seen = Seen {
        events: raw.events.clone(),
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| {
        let response = ui.interact(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 180.0)),
            egui::Id::unique("touch-control-button"),
            Sense::click_and_drag(),
        );
        seen.clicked = response.clicked();
        seen.dragged = response.dragged();
        seen.drag_stopped = response.drag_stopped();
        (seen.screen, seen.touching) =
            ui.input(|input| (input.has_touch_screen(), input.any_touches()));
    });
    output.textures_delta.clear();
    seen
}

fn touches(events: &[Event]) -> Vec<(u64, egui::TouchPhase)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Touch {
                device_id,
                id,
                phase,
                pos,
                force,
            } => {
                assert_eq!(*device_id, egui::TouchDeviceId(0));
                assert!(pos.is_finite());
                assert!(force.is_none());
                Some((id.0, *phase))
            }
            _ => None,
        })
        .collect()
}

fn pointer_edges(events: &[Event]) -> Vec<bool> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::PointerButton { pressed, .. } => Some(*pressed),
            _ => None,
        })
        .collect()
}

#[test]
fn control_tap_delivers_one_click_and_a_balanced_same_contact_touch() {
    let (ctx, mut input) = (egui::Context::default(), Input::default());
    button_frame(&ctx, &mut input, 0.0);
    finger(&mut input, ffi::PHASE_BEGAN, 0.1, 90.0);
    let start = button_frame(&ctx, &mut input, 0.1);
    assert!(start.touching && start.screen && !start.clicked);
    assert_eq!(touches(&start.events), [(7, egui::TouchPhase::Start)]);
    assert_eq!(pointer_edges(&start.events), [true]);
    finger(&mut input, ffi::PHASE_ENDED, 0.2, 90.0);
    let end = button_frame(&ctx, &mut input, 0.2);
    assert!(end.clicked && !end.touching);
    assert_eq!(touches(&end.events), [(7, egui::TouchPhase::End)]);
    assert_eq!(pointer_edges(&end.events), [false]);
    assert!(!button_frame(&ctx, &mut input, 0.3).clicked);
}

#[test]
fn delivered_control_cancellation_and_focus_loss_clear_touches_without_a_click_or_release() {
    for cancel in 0..3 {
        let (ctx, mut input) = (egui::Context::default(), Input::default());
        button_frame(&ctx, &mut input, 0.0);
        finger(&mut input, ffi::PHASE_BEGAN, 0.1, 90.0);
        assert!(button_frame(&ctx, &mut input, 0.1).touching);
        finger(&mut input, ffi::PHASE_MOVED, 0.2, 100.0);
        match cancel {
            0 => finger(&mut input, ffi::PHASE_CANCELLED, 0.3, 100.0),
            1 => input.cancel_all(CancelReason::SystemGesture),
            _ => input.focus_lost(),
        }
        let end = button_frame(&ctx, &mut input, 0.3);
        assert!(!end.touching && !end.clicked && !end.drag_stopped);
        assert_eq!(pointer_edges(&end.events), Vec::<bool>::new());
        assert!(matches!(
            end.events.as_slice(),
            [
                Event::Touch {
                    phase: egui::TouchPhase::Cancel,
                    ..
                },
                Event::PointerGone
            ]
        ));
        assert_eq!(touches(&end.events), [(7, egui::TouchPhase::Cancel)]);
        finger(&mut input, ffi::PHASE_ENDED, 0.4, 100.0);
        let late = button_frame(&ctx, &mut input, 0.4);
        assert!(!late.clicked && !late.touching && touches(&late.events).is_empty());
    }
}

#[test]
fn undelivered_control_cancel_purges_touch_start_and_moves_with_pointer_events() {
    let (ctx, mut input) = (egui::Context::default(), Input::default());
    finger(&mut input, ffi::PHASE_BEGAN, 0.1, 90.0);
    finger(&mut input, ffi::PHASE_MOVED, 0.2, 100.0);
    input.cancel_all(CancelReason::DocumentSwitch);
    let seen = button_frame(&ctx, &mut input, 0.3);
    assert!(seen.events.is_empty());
    assert!(!seen.screen && !seen.touching && !seen.clicked && !seen.drag_stopped);
}

#[test]
fn cancelled_control_drag_does_not_click_or_report_a_completed_drag() {
    let (ctx, mut input) = (egui::Context::default(), Input::default());
    button_frame(&ctx, &mut input, 0.0);
    finger(&mut input, ffi::PHASE_BEGAN, 0.1, 130.0);
    button_frame(&ctx, &mut input, 0.1);
    finger(&mut input, ffi::PHASE_MOVED, 0.2, 60.0);
    assert!(button_frame(&ctx, &mut input, 0.2).dragged);
    input.cancel_all(CancelReason::SystemGesture);
    let cancelled = button_frame(&ctx, &mut input, 0.3);
    assert!(
        !cancelled.touching && !cancelled.clicked && !cancelled.drag_stopped && !cancelled.dragged
    );
    finger(&mut input, ffi::PHASE_BEGAN, 0.4, 90.0);
    finger(&mut input, ffi::PHASE_ENDED, 0.5, 90.0);
    assert!(
        button_frame(&ctx, &mut input, 0.5).clicked,
        "next control tap remains usable"
    );
}

#[test]
fn owner_is_retained_when_a_control_finger_crosses_into_the_canvas() {
    let mut input = Input::default();
    finger(&mut input, ffi::PHASE_BEGAN, 0.1, 130.0);
    for (phase, time) in [(ffi::PHASE_MOVED, 0.2), (ffi::PHASE_ENDED, 0.3)] {
        input
            .submit(
                &[sample(7, ffi::CONTACT_FINGER, phase, time, 400.0)],
                0,
                &RouteContext::CANVAS,
            )
            .unwrap();
    }
    let events = input.drain().events;
    assert_eq!(
        touches(&events),
        [
            (7, egui::TouchPhase::Start),
            (7, egui::TouchPhase::Move),
            (7, egui::TouchPhase::End)
        ]
    );
    assert_eq!(pointer_edges(&events), [true, false]);
}

#[test]
fn pencil_and_indirect_pointer_keep_pointer_events_without_egui_touches() {
    for kind in [ffi::CONTACT_PENCIL, ffi::CONTACT_INDIRECT_POINTER] {
        let mut input = Input::default();
        for (phase, time, y) in [
            (ffi::PHASE_BEGAN, 0.1, 130.0),
            (ffi::PHASE_MOVED, 0.2, 60.0),
            (ffi::PHASE_ENDED, 0.3, 60.0),
        ] {
            input
                .submit(&[sample(7, kind, phase, time, y)], 0, &CONTROL)
                .unwrap();
        }
        let events = input.drain().events;
        assert!(touches(&events).is_empty());
        assert_eq!(pointer_edges(&events), [true, false]);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::PointerMoved(_)))
                .count(),
            3
        );
    }
}

#[test]
fn canvas_orbit_multi_touch_and_ignored_fingers_never_become_egui_touches() {
    let canvas = RouteContext {
        pencil_only_drawing: true,
        drag_authoring: true,
        over_control: &|_| false,
    };
    for mode in 0..3 {
        let mut input = Input::default();
        if mode == 2 {
            input
                .submit(
                    &[sample(1, ffi::CONTACT_PENCIL, ffi::PHASE_BEGAN, 0.0, 200.0)],
                    0,
                    &canvas,
                )
                .unwrap();
            input.drain();
        }
        input
            .submit(
                &[sample(7, ffi::CONTACT_FINGER, ffi::PHASE_BEGAN, 0.1, 130.0)],
                0,
                &canvas,
            )
            .unwrap();
        if mode == 1 {
            input
                .submit(
                    &[sample(8, ffi::CONTACT_FINGER, ffi::PHASE_BEGAN, 0.2, 140.0)],
                    0,
                    &canvas,
                )
                .unwrap();
        }
        for (phase, time) in [(ffi::PHASE_MOVED, 0.3), (ffi::PHASE_ENDED, 0.4)] {
            input
                .submit(
                    &[sample(7, ffi::CONTACT_FINGER, phase, time, 50.0)],
                    0,
                    &canvas,
                )
                .unwrap();
        }
        let events = input.drain().events;
        assert!(touches(&events).is_empty());
        assert!(pointer_edges(&events).is_empty());
        if mode == 0 {
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, Event::MouseWheel { .. }))
            );
        }
    }
}

#[test]
fn pointerless_control_activation_beside_pencil_never_starts_an_egui_touch() {
    let mut input = Input::default();
    input
        .submit(
            &[sample(1, ffi::CONTACT_PENCIL, ffi::PHASE_BEGAN, 0.0, 200.0)],
            0,
            &CONTROL,
        )
        .unwrap();
    input.drain();
    finger(&mut input, ffi::PHASE_BEGAN, 0.1, 90.0);
    finger(&mut input, ffi::PHASE_ENDED, 0.2, 90.0);
    let drained = input.drain();
    assert_eq!(drained.activations, [pos2(80.0, 90.0)]);
    assert!(touches(&drained.events).is_empty());
    assert!(pointer_edges(&drained.events).is_empty());
}

fn pressure(input: &mut Input) {
    let mut overflowed = false;
    for chunk in 0..PENDING_MAX / BATCH_MAX {
        let batch: Vec<_> = (1..=BATCH_MAX)
            .map(|index| {
                let step = chunk * BATCH_MAX + index;
                sample(
                    7,
                    ffi::CONTACT_FINGER,
                    ffi::PHASE_MOVED,
                    step as f64,
                    step as f64,
                )
            })
            .collect();
        overflowed |= input.submit(&batch, 0, &CONTROL).unwrap();
    }
    assert!(
        overflowed,
        "queue pressure must exercise disposable-event eviction"
    );
}

#[test]
fn pressure_preserves_touch_start_end_and_pointer_edges_then_clears_touch_state() {
    let (ctx, mut input) = (egui::Context::default(), Input::default());
    finger(&mut input, ffi::PHASE_BEGAN, 0.0, 130.0);
    pressure(&mut input);
    finger(&mut input, ffi::PHASE_ENDED, PENDING_MAX as f64 + 1.0, 50.0);
    let seen = button_frame(&ctx, &mut input, PENDING_MAX as f64 + 1.0);
    assert!(seen.events.len() <= PENDING_MAX);
    assert_eq!(pointer_edges(&seen.events), [true, false]);
    let touches = touches(&seen.events);
    assert_eq!(touches.first(), Some(&(7, egui::TouchPhase::Start)));
    assert_eq!(touches.last(), Some(&(7, egui::TouchPhase::End)));
    assert_eq!(
        touches
            .iter()
            .filter(|(_, phase)| *phase != egui::TouchPhase::Move)
            .count(),
        2
    );
    assert!(!seen.touching);
}

#[test]
fn pressure_cannot_drop_a_delivered_contact_cancel_or_turn_it_into_a_release() {
    let (ctx, mut input) = (egui::Context::default(), Input::default());
    finger(&mut input, ffi::PHASE_BEGAN, 0.0, 130.0);
    assert!(button_frame(&ctx, &mut input, 0.0).touching);
    pressure(&mut input);
    input.focus_lost();
    let seen = button_frame(&ctx, &mut input, PENDING_MAX as f64 + 1.0);
    assert_eq!(touches(&seen.events), [(7, egui::TouchPhase::Cancel)]);
    assert!(pointer_edges(&seen.events).is_empty());
    assert!(!seen.touching && !seen.clicked && !seen.drag_stopped);
}
