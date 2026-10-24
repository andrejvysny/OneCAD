use super::*;
use egui::{Event, TouchPhase, pos2};

/// Fingers author once they leave the tap slop (a sketch tool is armed).
const AUTHOR: RouteContext<'static> = RouteContext {
    pencil_only_drawing: false,
    drag_authoring: true,
    over_control: &|_| false,
};

fn sample(contact: u64, phase: u32, provenance: u32, t: f64, x: f64) -> TouchSample {
    TouchSample {
        contact_id: contact,
        kind: ffi::CONTACT_PENCIL,
        phase,
        provenance,
        timestamp_s: t,
        x_pt: x,
        y_pt: x,
        tap_count: 1,
        ..Default::default()
    }
}

fn actual(contact: u64, phase: u32, t: f64, x: f64) -> TouchSample {
    sample(contact, phase, ffi::SAMPLE_ACTUAL, t, x)
}

fn finger(contact: u64, phase: u32, t: f64, x: f64) -> TouchSample {
    TouchSample {
        kind: ffi::CONTACT_FINGER,
        ..actual(contact, phase, t, x)
    }
}

/// Submits with the authoring context; `true` when movement was coalesced.
fn submit(input: &mut Input, samples: &[TouchSample], generation: u64) -> bool {
    input
        .submit(samples, generation, &AUTHOR)
        .expect("batch fits")
}

fn release(pos: Pos2) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn drained(events: Vec<Event>, again: bool, cancelled: bool) -> Drained {
    Drained {
        events,
        again,
        cancelled,
        pointer_kind: None,
        pencil_double_tap: None,
        activations: Vec::new(),
    }
}

fn presses(events: &[Event]) -> Vec<bool> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::PointerButton { pressed, .. } => Some(*pressed),
            _ => None,
        })
        .collect()
}

/// A finger that left the slop on the canvas: it owns the pointer (authoring).
fn finger_author(input: &mut Input, contact: u64) {
    submit(input, &[finger(contact, ffi::PHASE_BEGAN, 0.0, 3.0)], 0);
    submit(input, &[finger(contact, ffi::PHASE_MOVED, 0.05, 30.0)], 0);
}

#[test]
fn mixed_pointer_stroke_and_pencil_hover_do_not_label_the_stroke_as_pencil() {
    mixed_source_frame(ffi::CONTACT_INDIRECT_POINTER, ffi::CONTACT_PENCIL);
}

#[test]
fn mixed_pencil_stroke_and_pointer_hover_do_not_author_a_pencil_primitive() {
    mixed_source_frame(ffi::CONTACT_PENCIL, ffi::CONTACT_INDIRECT_POINTER);
}

fn mixed_source_frame(stroke: u32, hover: u32) {
    let mut input = Input::default();
    let samples = [
        TouchSample {
            kind: stroke,
            ..actual(1, ffi::PHASE_BEGAN, 0.0, 1.0)
        },
        TouchSample {
            kind: stroke,
            ..actual(1, ffi::PHASE_MOVED, 0.1, 20.0)
        },
        TouchSample {
            kind: stroke,
            ..actual(1, ffi::PHASE_ENDED, 0.2, 30.0)
        },
        TouchSample {
            kind: hover,
            ..actual(2, ffi::PHASE_HOVER_MOVED, 0.3, 40.0)
        },
    ];
    submit(&mut input, &samples, 0);
    let frame = input.drain();
    assert_eq!(presses(&frame.events), [true, false]);
    assert_eq!(
        frame.pointer_kind,
        Some(onecad_ui::host::PointerKind::Precise)
    );
    assert!(!frame.cancelled);
    submit(&mut input, &[actual(3, ffi::PHASE_BEGAN, 0.4, 50.0)], 0);
    assert_eq!(
        input.drain().pointer_kind,
        Some(onecad_ui::host::PointerKind::Pencil),
        "mixed provenance belongs only to its pending frame"
    );
    submit(&mut input, &[actual(3, ffi::PHASE_MOVED, 0.5, 60.0)], 0);
    assert_eq!(
        input.drain().pointer_kind,
        Some(onecad_ui::host::PointerKind::Pencil)
    );
}

#[test]
fn focus_loss_clears_modifiers_but_contact_cancellation_preserves_them() {
    let mut input = Input::default();
    let key = ffi::KeyEvent {
        modifiers: ffi::MOD_SHIFT | ffi::MOD_COMMAND,
        ..Default::default()
    };
    input.key(&key);
    input.focus_lost();
    assert_eq!(input.modifiers, egui::Modifiers::NONE);
    input.insert_text("\n");
    input.insert_text("\t");
    assert!(input.drain().events.iter().all(|event| match event {
        Event::Key { modifiers, .. } => *modifiers == egui::Modifiers::NONE,
        _ => true,
    }));
    input.key(&key);
    input.cancel_all(CancelReason::DocumentSwitch);
    assert!(input.modifiers.shift && input.modifiers.command);
}

#[test]
fn coalesced_samples_replay_in_order_without_the_actual_duplicate_or_predictions() {
    let mut input = Input::default();
    submit(&mut input, &[actual(7, ffi::PHASE_BEGAN, 1.0, 5.0)], 0);
    input.drain();
    let batch = [
        actual(7, ffi::PHASE_MOVED, 1.2, 20.0),
        sample(7, ffi::PHASE_MOVED, ffi::SAMPLE_COALESCED, 1.1, 10.0),
        sample(7, ffi::PHASE_MOVED, ffi::SAMPLE_COALESCED, 1.2, 20.0),
        sample(7, ffi::PHASE_MOVED, ffi::SAMPLE_PREDICTED, 1.3, 30.0),
        sample(
            7,
            ffi::PHASE_MOVED,
            ffi::SAMPLE_ESTIMATION_UPDATE,
            1.0,
            99.0,
        ),
    ];
    assert!(!submit(&mut input, &batch, 0));
    let events = input.drain().events;
    assert_eq!(
        events,
        vec![
            Event::PointerMoved(pos2(10.0, 10.0)),
            Event::PointerMoved(pos2(20.0, 20.0))
        ]
    );
    assert_eq!(input.prediction(7), Some(pos2(30.0, 30.0)));
}

#[test]
fn predictions_are_kept_for_the_author_only_and_end_with_it() {
    let mut input = Input::default();
    let predicted = |contact| sample(contact, ffi::PHASE_MOVED, ffi::SAMPLE_PREDICTED, 0.2, 9.0);
    submit(
        &mut input,
        &[actual(1, ffi::PHASE_BEGAN, 0.0, 1.0), predicted(1)],
        0,
    );
    submit(
        &mut input,
        &[finger(2, ffi::PHASE_BEGAN, 0.1, 5.0), predicted(2)],
        0,
    );
    assert!(input.prediction(1).is_some());
    assert!(
        input.prediction(2).is_none(),
        "a non-authoring contact keeps no prediction"
    );
    submit(
        &mut input,
        &[actual(1, ffi::PHASE_ENDED, 0.3, 2.0), predicted(1)],
        0,
    );
    assert!(
        input.prediction(1).is_none(),
        "an ended contact keeps no prediction"
    );
}

#[test]
fn a_finger_tap_clicks_and_the_pointer_leaves_in_a_later_frame() {
    let mut input = Input::default();
    submit(
        &mut input,
        &[
            finger(1, ffi::PHASE_BEGAN, 0.0, 4.0),
            finger(1, ffi::PHASE_ENDED, 0.1, 4.0),
        ],
        0,
    );
    let first = input.drain();
    assert!(first.again && !first.events.contains(&Event::PointerGone));
    assert_eq!(presses(&first.events), [true, false]);
    assert_eq!(first.events.last(), Some(&release(pos2(4.0, 4.0))));
    assert_eq!(
        first.pointer_kind,
        Some(onecad_ui::host::PointerKind::Finger)
    );
    assert_eq!(
        input.drain(),
        drained(vec![Event::PointerGone], false, false)
    );
    assert_eq!(input.drain(), Drained::default());
}

#[test]
fn cancellation_never_releases_and_ignores_other_contacts() {
    let mut input = Input::default();
    submit(
        &mut input,
        &[
            actual(1, ffi::PHASE_BEGAN, 0.0, 3.0),
            actual(2, ffi::PHASE_BEGAN, 0.0, 50.0),
            actual(2, ffi::PHASE_MOVED, 0.1, 60.0),
        ],
        0,
    );
    let first = input.drain();
    assert_eq!(first.events.len(), 2, "only the first Pencil presses");
    assert_eq!(
        first.pointer_kind,
        Some(onecad_ui::host::PointerKind::Pencil)
    );
    submit(&mut input, &[actual(1, ffi::PHASE_MOVED, 0.2, 8.0)], 0);
    input.cancel_all(CancelReason::SystemGesture);
    assert_eq!(
        input.drain(),
        drained(vec![Event::PointerGone], false, true)
    );
}

#[test]
fn a_contact_cancelled_before_any_frame_never_reaches_egui() {
    for author in [finger, actual] {
        let mut input = Input::default();
        input.push_event(Event::Text("a".into()));
        submit(&mut input, &[author(1, ffi::PHASE_BEGAN, 0.0, 3.0)], 0);
        submit(&mut input, &[author(1, ffi::PHASE_MOVED, 0.05, 40.0)], 0);
        submit(&mut input, &[author(1, ffi::PHASE_CANCELLED, 0.1, 40.0)], 0);
        let frame = input.drain();
        assert_eq!(frame.events, vec![Event::Text("a".into())]);
        assert!(!frame.cancelled && !frame.again);
    }
}

#[test]
fn stale_generation_cancels_instead_of_committing() {
    let mut input = Input::default();
    submit(&mut input, &[actual(1, ffi::PHASE_BEGAN, 0.0, 1.0)], 4);
    input.drain();
    submit(&mut input, &[actual(1, ffi::PHASE_ENDED, 0.1, 9.0)], 3);
    assert_eq!(
        input.drain(),
        drained(vec![Event::PointerGone], false, true)
    );
}

#[test]
fn a_pencil_takes_the_pointer_from_an_authoring_finger() {
    let mut input = Input::default();
    finger_author(&mut input, 1);
    assert_eq!(presses(&input.drain().events), [true]);
    submit(&mut input, &[actual(2, ffi::PHASE_BEGAN, 0.1, 40.0)], 0);
    let frame = input.drain();
    assert!(frame.cancelled);
    assert_eq!(frame.events[0], Event::PointerGone);
    assert_eq!(presses(&frame.events), [true]);
    submit(&mut input, &[finger(1, ffi::PHASE_ENDED, 0.2, 3.0)], 0);
    submit(&mut input, &[actual(2, ffi::PHASE_ENDED, 0.3, 40.0)], 0);
    assert_eq!(
        presses(&input.drain().events),
        [false],
        "only the Pencil releases"
    );
}

#[test]
fn overflow_coalesces_movement_but_keeps_terminal_events() {
    let mut input = Input::default();
    submit(&mut input, &[actual(1, ffi::PHASE_BEGAN, 0.0, 0.0)], 0);
    let mut overflowed = false;
    for chunk in 0..PENDING_MAX / BATCH_MAX {
        let moves: Vec<_> = (1..=BATCH_MAX)
            .map(|i| {
                let at = (chunk * BATCH_MAX + i) as f64;
                actual(1, ffi::PHASE_MOVED, at, at)
            })
            .collect();
        overflowed |= submit(&mut input, &moves, 0);
    }
    assert!(overflowed);
    let last = PENDING_MAX as f64 + 1.0;
    submit(&mut input, &[actual(1, ffi::PHASE_ENDED, last, last)], 0);
    let events = input.drain().events;
    assert!(events.len() <= PENDING_MAX + 2);
    assert_eq!(presses(&events), [true, false], "press and release survive");
    let oversized = vec![actual(1, ffi::PHASE_MOVED, 0.0, 0.0); BATCH_MAX + 1];
    assert!(input.submit(&oversized, 0, &AUTHOR).is_err());
}

fn nav(phase: u32, dx: f64, scale: f64) -> ffi::Navigation {
    ffi::Navigation {
        phase,
        anchor_x_pt: 100.0,
        anchor_y_pt: 200.0,
        pan_dx_pt: dx,
        scale_delta: scale,
        ..Default::default()
    }
}

fn wheels(events: &[Event]) -> Vec<(TouchPhase, f32)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::MouseWheel {
                phase,
                source,
                delta,
                ..
            } => {
                assert_eq!(*source, egui::MouseWheelSource::Trackpad);
                Some((*phase, delta.x))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn navigation_becomes_trackpad_pan_and_zoom_over_its_anchor() {
    let mut input = Input::default();
    assert!(input.navigation(&nav(ffi::PHASE_BEGAN, 0.0, 1.0)));
    assert!(input.navigation(&nav(ffi::PHASE_MOVED, 3.0, 1.5)));
    assert!(input.navigation(&nav(ffi::PHASE_ENDED, 0.0, 1.0)));
    let frame = input.drain();
    assert_eq!(
        wheels(&frame.events),
        [
            (TouchPhase::Start, 0.0),
            (TouchPhase::Move, 3.0),
            (TouchPhase::End, 0.0)
        ]
    );
    assert!(frame.events.contains(&Event::Zoom(1.5)));
    assert_eq!(frame.events[0], Event::PointerMoved(pos2(100.0, 200.0)));
    assert!(
        frame.again,
        "the pointer leaves only after the gesture's frame"
    );
    assert!(!input.navigation(&nav(99, 0.0, 1.0)));
}

#[test]
fn navigation_moves_between_frames_merge() {
    let mut input = Input::default();
    input.navigation(&nav(ffi::PHASE_BEGAN, 0.0, 1.0));
    input.drain();
    for _ in 0..100 {
        input.navigation(&nav(ffi::PHASE_MOVED, 2.0, 1.01));
    }
    let events = input.drain().events;
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(wheels(&events), [(TouchPhase::Move, 200.0)]);
    let zoom = events.iter().find_map(|event| match event {
        Event::Zoom(zoom) => Some(*zoom),
        _ => None,
    });
    assert!(zoom.is_some_and(|zoom| (zoom - 1.01f32.powi(100)).abs() < 1e-3));
}

#[test]
fn navigation_cancels_an_authoring_finger_and_is_ignored_under_a_pencil() {
    let mut input = Input::default();
    finger_author(&mut input, 1);
    input.drain();
    input.navigation(&nav(ffi::PHASE_BEGAN, 0.0, 1.0));
    let frame = input.drain();
    assert!(frame.cancelled && frame.events[0] == Event::PointerGone);
    assert!(presses(&frame.events).is_empty());
    input.navigation(&nav(ffi::PHASE_ENDED, 0.0, 1.0));
    input.drain();
    input.drain();

    submit(&mut input, &[actual(2, ffi::PHASE_BEGAN, 1.0, 3.0)], 0);
    input.drain();
    for phase in [ffi::PHASE_BEGAN, ffi::PHASE_MOVED, ffi::PHASE_ENDED] {
        assert!(input.navigation(&nav(phase, 5.0, 2.0)));
    }
    assert_eq!(input.drain(), Drained::default(), "the camera holds still");
}

#[test]
fn text_composition_commits_through_ime_and_return_and_tab_are_keys() {
    let mut input = Input::default();
    input.set_marked("ni");
    input.insert_text("你");
    input.set_marked("");
    input.insert_text("\n");
    input.insert_text("\t");
    input.delete_backward();
    let events = input.drain().events;
    assert!(matches!(
        &events[0],
        Event::Ime(egui::ImeEvent::Preedit { text, .. }) if text == "ni"
    ));
    assert_eq!(events[1], Event::Ime(egui::ImeEvent::Commit("你".into())));
    let keys: Vec<_> = events[2..]
        .iter()
        .map(|event| match event {
            Event::Key { key, pressed, .. } => (*key, *pressed),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        keys,
        [
            (egui::Key::Enter, true),
            (egui::Key::Enter, false),
            (egui::Key::Tab, true),
            (egui::Key::Tab, false),
            (egui::Key::Backspace, true),
            (egui::Key::Backspace, false)
        ]
    );
}

#[path = "input_egui_tests.rs"]
mod egui_level;

#[path = "input_route_tests.rs"]
mod route;
