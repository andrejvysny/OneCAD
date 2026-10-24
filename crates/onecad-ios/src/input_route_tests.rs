//! INPUT.md ownership rows as they reach egui: orbit, controls, Pencil-only
//! drawing, hover, tip double tap and document-switch cancellation.
use super::{actual, finger, presses, sample, submit, wheels};
use crate::ffi;
use crate::input::Input;
use egui::{Event, TouchPhase, pos2};
use onecad_interaction::touch::{CancelReason, RouteContext};

const ORBIT: RouteContext<'static> = RouteContext::CANVAS;

fn shift_wheels(events: &[Event]) -> bool {
    events.iter().all(|event| match event {
        Event::MouseWheel { modifiers, .. } => modifiers.shift,
        _ => true,
    })
}

#[test]
fn a_canvas_finger_orbits_as_a_shift_trackpad_wheel_from_its_slop_exit() {
    let mut input = Input::default();
    let route = |input: &mut Input, sample| input.submit(&[sample], 0, &ORBIT).unwrap();
    route(&mut input, finger(1, ffi::PHASE_BEGAN, 0.0, 10.0));
    route(&mut input, finger(1, ffi::PHASE_MOVED, 0.1, 30.0));
    for step in 1..=5 {
        route(
            &mut input,
            finger(
                1,
                ffi::PHASE_MOVED,
                0.1 + 0.01 * f64::from(step),
                30.0 + f64::from(step),
            ),
        );
    }
    route(&mut input, finger(1, ffi::PHASE_ENDED, 0.3, 35.0));
    let frame = input.drain();
    assert!(presses(&frame.events).is_empty(), "orbit never presses");
    assert!(shift_wheels(&frame.events));
    assert_eq!(
        wheels(&frame.events),
        [
            (TouchPhase::Start, 0.0),
            (TouchPhase::Move, 5.0),
            (TouchPhase::End, 0.0)
        ],
        "moves between frames merge; the start is at the exit point (no jump)"
    );
    assert_eq!(frame.events[0], Event::PointerMoved(pos2(30.0, 30.0)));
    assert!(frame.again, "the finger's pointer leaves next frame");
}

#[test]
fn a_finger_on_a_control_presses_at_once_and_pencil_only_drawing_keeps_fingers_off() {
    let controls = RouteContext {
        pencil_only_drawing: false,
        drag_authoring: false,
        over_control: &|point| point.y < 50.0,
    };
    let mut input = Input::default();
    input
        .submit(&[finger(1, ffi::PHASE_BEGAN, 0.0, 20.0)], 0, &controls)
        .unwrap();
    assert_eq!(
        presses(&input.drain().events),
        [true],
        "no slop wait on a control"
    );

    let pencil_only = RouteContext {
        pencil_only_drawing: true,
        drag_authoring: true,
        over_control: &|_| false,
    };
    let mut input = Input::default();
    for sample in [
        finger(2, ffi::PHASE_BEGAN, 0.0, 100.0),
        finger(2, ffi::PHASE_MOVED, 0.1, 140.0),
    ] {
        input.submit(&[sample], 0, &pencil_only).unwrap();
    }
    let events = input.drain().events;
    assert!(presses(&events).is_empty(), "fingers never author");
    assert!(!wheels(&events).is_empty(), "they orbit instead");
}

#[test]
fn hover_moves_the_pointer_without_a_button_and_exit_only_clears_it() {
    let mut input = Input::default();
    let hover = |phase, x| sample(5, phase, ffi::SAMPLE_ACTUAL, 0.0, x);
    submit(&mut input, &[hover(ffi::PHASE_HOVER_MOVED, 12.0)], 0);
    let frame = input.drain();
    assert_eq!(frame.events, vec![Event::PointerMoved(pos2(12.0, 12.0))]);
    assert_eq!(
        frame.pointer_kind,
        Some(onecad_ui::host::PointerKind::Pencil)
    );
    submit(&mut input, &[actual(1, ffi::PHASE_BEGAN, 0.1, 20.0)], 0);
    submit(&mut input, &[hover(ffi::PHASE_HOVER_MOVED, 90.0)], 0);
    assert_eq!(
        presses(&input.drain().events),
        [true],
        "no hover under a press"
    );
    submit(&mut input, &[actual(1, ffi::PHASE_ENDED, 0.2, 20.0)], 0);
    submit(&mut input, &[hover(ffi::PHASE_HOVER_MOVED, 30.0)], 0);
    submit(&mut input, &[hover(ffi::PHASE_HOVER_EXITED, 30.0)], 0);
    let frame = input.drain();
    assert_eq!(presses(&frame.events), [false]);
    assert!(frame.again && !frame.cancelled, "exit clears hover only");
    assert_eq!(input.drain().events, vec![Event::PointerGone]);
}

#[test]
fn a_control_tapped_beside_a_pencil_stroke_is_activated_without_the_pointer() {
    let controls = RouteContext {
        pencil_only_drawing: false,
        drag_authoring: true,
        over_control: &|point| point.y < 50.0,
    };
    let mut input = Input::default();
    let route = |input: &mut Input, sample| input.submit(&[sample], 0, &controls).unwrap();
    route(&mut input, actual(1, ffi::PHASE_BEGAN, 0.0, 200.0));
    route(&mut input, finger(2, ffi::PHASE_BEGAN, 0.05, 20.0));
    route(&mut input, actual(1, ffi::PHASE_MOVED, 0.1, 220.0));
    route(&mut input, finger(2, ffi::PHASE_ENDED, 0.15, 20.0));
    let frame = input.drain();
    assert_eq!(presses(&frame.events), [true], "only the Pencil presses");
    assert_eq!(frame.activations, vec![pos2(20.0, 20.0)]);
    route(&mut input, actual(1, ffi::PHASE_ENDED, 0.2, 240.0));
    let frame = input.drain();
    assert_eq!(presses(&frame.events), [false], "the stroke still releases");
    assert!(
        frame.activations.is_empty(),
        "each activation is drained once"
    );
}

#[test]
fn a_pencil_tip_double_tap_is_marked_for_promotion_and_a_finger_one_is_not() {
    let mut input = Input::default();
    let tap = |contact, kind, phase, t| ffi::TouchSample {
        kind,
        tap_count: 2,
        ..actual(contact, phase, t, 200.0)
    };
    submit(
        &mut input,
        &[tap(1, ffi::CONTACT_PENCIL, ffi::PHASE_BEGAN, 0.0)],
        0,
    );
    assert_eq!(input.drain().pencil_double_tap, Some(true));
    submit(
        &mut input,
        &[tap(1, ffi::CONTACT_PENCIL, ffi::PHASE_ENDED, 0.05)],
        0,
    );
    assert_eq!(
        input.drain().pencil_double_tap,
        None,
        "only a press sets it"
    );
    let finger_tap = [
        tap(2, ffi::CONTACT_FINGER, ffi::PHASE_BEGAN, 0.3),
        tap(2, ffi::CONTACT_FINGER, ffi::PHASE_ENDED, 0.35),
    ];
    submit(&mut input, &finger_tap, 0);
    assert_eq!(
        input.drain().pencil_double_tap,
        Some(false),
        "a finger double tap is egui's to count"
    );
}

#[test]
fn a_document_switch_cancels_a_live_stroke_without_a_release() {
    let mut input = Input::default();
    submit(&mut input, &[actual(1, ffi::PHASE_BEGAN, 0.0, 20.0)], 0);
    submit(&mut input, &[actual(1, ffi::PHASE_MOVED, 0.1, 40.0)], 0);
    input.drain();
    input.cancel_all(CancelReason::DocumentSwitch);
    let frame = input.drain();
    assert_eq!(frame.events, vec![Event::PointerGone]);
    assert!(frame.cancelled, "the editor forgets the press");
    submit(&mut input, &[actual(1, ffi::PHASE_ENDED, 0.2, 60.0)], 0);
    assert!(presses(&input.drain().events).is_empty(), "no late release");
}
