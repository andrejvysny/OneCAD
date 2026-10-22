use super::*;
use egui::{Event, TouchPhase, pos2};

fn sample(contact: u64, phase: u32, provenance: u32, t: f64, x: f64) -> TouchSample {
    TouchSample {
        contact_id: contact,
        kind: ffi::CONTACT_PENCIL,
        phase,
        provenance,
        timestamp_s: t,
        x_pt: x,
        y_pt: x,
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

#[test]
fn coalesced_samples_replay_in_order_without_the_actual_duplicate_or_predictions() {
    let mut input = Input::default();
    input.submit(&[actual(7, ffi::PHASE_BEGAN, 1.0, 5.0)], 0);
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
    assert!(!input.submit(&batch, 0));
    let events = input.drain().events;
    assert_eq!(
        events,
        vec![
            Event::PointerMoved(pos2(10.0, 10.0)),
            Event::PointerMoved(pos2(20.0, 20.0))
        ]
    );
    assert_eq!(
        input.prediction(7).map(|p| p.position),
        Some(pos2(30.0, 30.0))
    );
}

#[test]
fn predictions_are_kept_for_the_owner_only_and_end_with_it() {
    let mut input = Input::default();
    let predicted = |contact| sample(contact, ffi::PHASE_MOVED, ffi::SAMPLE_PREDICTED, 0.2, 9.0);
    input.submit(&[actual(1, ffi::PHASE_BEGAN, 0.0, 1.0), predicted(1)], 0);
    input.submit(&[finger(2, ffi::PHASE_BEGAN, 0.1, 5.0), predicted(2)], 0);
    assert!(input.prediction(1).is_some());
    assert!(
        input.prediction(2).is_none(),
        "a non-owner keeps no prediction"
    );
    input.submit(&[actual(1, ffi::PHASE_ENDED, 0.3, 2.0), predicted(1)], 0);
    assert!(
        input.prediction(1).is_none(),
        "an ended contact keeps no prediction"
    );
}

#[test]
fn finger_release_commits_before_the_pointer_leaves_in_a_later_frame() {
    let mut input = Input::default();
    input.submit(
        &[
            finger(1, ffi::PHASE_BEGAN, 0.0, 4.0),
            finger(1, ffi::PHASE_ENDED, 0.1, 4.0),
        ],
        0,
    );
    let first = input.drain();
    assert!(first.again && !first.events.contains(&Event::PointerGone));
    assert_eq!(first.events.last(), Some(&release(pos2(4.0, 4.0))));
    assert_eq!(
        input.drain(),
        drained(vec![Event::PointerGone], false, false)
    );
    assert_eq!(input.drain(), Drained::default());
}

#[test]
fn cancellation_never_releases_and_ignores_other_contacts() {
    let mut input = Input::default();
    input.submit(
        &[
            actual(1, ffi::PHASE_BEGAN, 0.0, 3.0),
            actual(2, ffi::PHASE_BEGAN, 0.0, 50.0),
            actual(2, ffi::PHASE_MOVED, 0.1, 60.0),
        ],
        0,
    );
    assert_eq!(input.drain().events.len(), 2, "only the owner presses");
    input.submit(&[actual(1, ffi::PHASE_MOVED, 0.2, 8.0)], 0);
    input.cancel_all();
    assert_eq!(
        input.drain(),
        drained(vec![Event::PointerGone], false, true)
    );
}

#[test]
fn a_contact_cancelled_before_any_frame_never_reaches_egui() {
    let mut input = Input::default();
    input.push_event(Event::Text("a".into()));
    input.submit(&[finger(1, ffi::PHASE_BEGAN, 0.0, 3.0)], 0);
    input.submit(&[finger(1, ffi::PHASE_CANCELLED, 0.1, 3.0)], 0);
    assert_eq!(
        input.drain(),
        drained(vec![Event::Text("a".into())], false, false)
    );
}

#[test]
fn stale_generation_cancels_instead_of_committing() {
    let mut input = Input::default();
    input.submit(&[actual(1, ffi::PHASE_BEGAN, 0.0, 1.0)], 4);
    input.drain();
    input.submit(&[actual(1, ffi::PHASE_ENDED, 0.1, 9.0)], 3);
    assert_eq!(
        input.drain(),
        drained(vec![Event::PointerGone], false, true)
    );
}

#[test]
fn a_pencil_takes_the_pointer_from_a_finger() {
    let mut input = Input::default();
    input.submit(&[finger(1, ffi::PHASE_BEGAN, 0.0, 3.0)], 0);
    input.drain();
    input.submit(&[actual(2, ffi::PHASE_BEGAN, 0.1, 40.0)], 0);
    let frame = input.drain();
    assert!(frame.cancelled);
    assert_eq!(frame.events[0], Event::PointerGone);
    assert_eq!(presses(&frame.events), [true]);
    input.submit(&[finger(1, ffi::PHASE_ENDED, 0.2, 3.0)], 0);
    input.submit(&[actual(2, ffi::PHASE_ENDED, 0.3, 40.0)], 0);
    assert_eq!(
        presses(&input.drain().events),
        [false],
        "only the Pencil releases"
    );
}

#[test]
fn overflow_coalesces_movement_but_keeps_terminal_events() {
    let mut input = Input::default();
    input.submit(&[actual(1, ffi::PHASE_BEGAN, 0.0, 0.0)], 0);
    let moves: Vec<_> = (1..=PENDING_MAX)
        .map(|i| actual(1, ffi::PHASE_MOVED, i as f64, i as f64))
        .collect();
    assert!(input.submit(&moves, 0));
    let last = PENDING_MAX as f64 + 1.0;
    input.submit(&[actual(1, ffi::PHASE_ENDED, last, last)], 0);
    let events = input.drain().events;
    assert!(events.len() <= PENDING_MAX + 2);
    assert_eq!(presses(&events), [true, false], "press and release survive");
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
fn navigation_cancels_a_finger_owner_and_is_ignored_under_a_pencil() {
    let mut input = Input::default();
    input.submit(&[finger(1, ffi::PHASE_BEGAN, 0.0, 3.0)], 0);
    input.drain();
    input.navigation(&nav(ffi::PHASE_BEGAN, 0.0, 1.0));
    let frame = input.drain();
    assert!(frame.cancelled && frame.events[0] == Event::PointerGone);
    assert!(presses(&frame.events).is_empty());
    input.navigation(&nav(ffi::PHASE_ENDED, 0.0, 1.0));
    input.drain();
    input.drain();

    input.submit(&[actual(2, ffi::PHASE_BEGAN, 1.0, 3.0)], 0);
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
