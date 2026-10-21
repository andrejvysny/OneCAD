use super::*;
use egui::{Event, pos2};

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

fn release(pos: Pos2) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }
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
    let (events, _) = input.drain();
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
fn finger_release_commits_before_the_pointer_leaves_in_a_later_frame() {
    let mut input = Input::default();
    let mut began = actual(1, ffi::PHASE_BEGAN, 0.0, 4.0);
    began.kind = ffi::CONTACT_FINGER;
    let mut ended = actual(1, ffi::PHASE_ENDED, 0.1, 4.0);
    ended.kind = ffi::CONTACT_FINGER;
    input.submit(&[began, ended], 0);
    let (events, again) = input.drain();
    assert!(again && !events.contains(&Event::PointerGone));
    assert_eq!(events.last(), Some(&release(pos2(4.0, 4.0))));
    assert_eq!(input.drain(), (vec![Event::PointerGone], false));
    assert_eq!(input.drain(), (Vec::new(), false));
}

#[test]
fn cancellation_leaves_before_releasing_and_ignores_other_contacts() {
    let mut input = Input::default();
    input.submit(
        &[
            actual(1, ffi::PHASE_BEGAN, 0.0, 3.0),
            actual(2, ffi::PHASE_BEGAN, 0.0, 50.0),
            actual(2, ffi::PHASE_MOVED, 0.1, 60.0),
        ],
        0,
    );
    let (events, _) = input.drain();
    assert_eq!(events.len(), 2, "only the owner presses: {events:?}");
    input.cancel_all();
    let (events, again) = input.drain();
    assert_eq!(events, vec![Event::PointerGone, release(pos2(3.0, 3.0))]);
    assert!(!again);
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
    let (events, _) = input.drain();
    assert!(events.len() <= PENDING_MAX + 2);
    let buttons = events
        .iter()
        .filter(|event| matches!(event, Event::PointerButton { .. }))
        .count();
    assert_eq!(buttons, 2, "press and release survive coalescing");
}

#[test]
fn stale_generation_cancels_instead_of_committing() {
    let mut input = Input::default();
    input.submit(&[actual(1, ffi::PHASE_BEGAN, 0.0, 1.0)], 4);
    input.drain();
    input.submit(&[actual(1, ffi::PHASE_ENDED, 0.1, 9.0)], 3);
    let (events, _) = input.drain();
    assert_eq!(events, vec![Event::PointerGone, release(pos2(1.0, 1.0))]);
}

#[test]
fn navigation_becomes_trackpad_pan_and_zoom_over_its_anchor() {
    let mut input = Input::default();
    let mut nav = ffi::Navigation {
        phase: ffi::PHASE_BEGAN,
        anchor_x_pt: 100.0,
        anchor_y_pt: 200.0,
        scale_delta: 1.0,
        ..Default::default()
    };
    assert!(input.navigation(&nav));
    nav.phase = ffi::PHASE_MOVED;
    (nav.pan_dx_pt, nav.scale_delta) = (3.0, 1.5);
    assert!(input.navigation(&nav));
    nav.phase = ffi::PHASE_ENDED;
    (nav.pan_dx_pt, nav.scale_delta) = (0.0, 1.0);
    assert!(input.navigation(&nav));
    let (events, again) = input.drain();
    let phases: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            Event::MouseWheel { phase, source, .. } => {
                assert_eq!(*source, egui::MouseWheelSource::Trackpad);
                Some(*phase)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        phases,
        [TouchPhase::Start, TouchPhase::Move, TouchPhase::End]
    );
    assert!(events.contains(&Event::Zoom(1.5)));
    assert_eq!(events[0], Event::PointerMoved(pos2(100.0, 200.0)));
    assert!(again, "the pointer leaves only after the gesture's frame");
    nav.phase = 99;
    assert!(!input.navigation(&nav));
}

#[test]
fn text_composition_commits_through_ime_and_return_is_enter() {
    let mut input = Input::default();
    input.set_marked("ni");
    input.insert_text("你");
    input.set_marked("");
    input.insert_text("\n");
    input.delete_backward();
    let (events, _) = input.drain();
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
            (egui::Key::Backspace, true),
            (egui::Key::Backspace, false)
        ]
    );
}
