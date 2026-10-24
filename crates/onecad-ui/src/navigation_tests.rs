use super::*;
use egui::{Modifiers, vec2};

#[path = "navigation_activity_tests.rs"]
mod activity;

fn wheel(source: MouseWheelSource, phase: TouchPhase, delta: Vec2, shift: bool) -> Event {
    Event::MouseWheel {
        source,
        phase,
        delta,
        unit: MouseWheelUnit::Point,
        modifiers: Modifiers {
            shift,
            ..Modifiers::NONE
        },
    }
}

fn send(nav: &mut Navigation, event: &Event, time: f64) -> Option<Motion> {
    nav.event(event, time, 600.0)
}

fn zoom(motion: Option<Motion>, expected: f64) {
    let Some(Motion::Zoom(actual)) = motion else {
        panic!("expected zoom, got {motion:?}")
    };
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}

#[test]
fn explicit_sources_override_units_phases_and_small_fractional_deltas() {
    for unit in [
        MouseWheelUnit::Point,
        MouseWheelUnit::Line,
        MouseWheelUnit::Page,
    ] {
        let scale = match unit {
            MouseWheelUnit::Point => 1.0,
            MouseWheelUnit::Line => 16.0,
            MouseWheelUnit::Page => 600.0,
        };
        for source in [
            MouseWheelSource::Wheel,
            MouseWheelSource::Trackpad,
            MouseWheelSource::Momentum,
        ] {
            let mut nav = Navigation::default();
            let mut event = wheel(source, TouchPhase::Start, vec2(0.25, 0.5), false);
            if let Event::MouseWheel {
                unit: event_unit, ..
            } = &mut event
            {
                *event_unit = unit;
            }
            let result = send(&mut nav, &event, 1.0);
            if source == MouseWheelSource::Wheel {
                zoom(result, (-0.5_f64 * f64::from(scale) * 0.0015).exp());
            } else {
                assert_eq!(result, Some(Motion::Pan(vec2(0.25, 0.5) * scale)));
            }
        }
    }
}

#[test]
fn unknown_points_require_a_bracket_and_keep_endpoint_deltas() {
    let mut nav = Navigation::default();
    let event = |phase| wheel(MouseWheelSource::Unknown, phase, vec2(2.0, 3.0), false);
    zoom(
        send(&mut nav, &event(TouchPhase::Move), 1.0),
        (-3.0_f64 * 0.0015).exp(),
    );
    for (i, phase) in [TouchPhase::Start, TouchPhase::Move, TouchPhase::End]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            send(&mut nav, &event(phase), 2.0 + i as f64 * 0.01),
            Some(Motion::Pan(vec2(2.0, 3.0)))
        );
    }
    zoom(
        send(&mut nav, &event(TouchPhase::Move), 2.04),
        (-3.0_f64 * 0.0015).exp(),
    );
}

#[test]
fn unknown_line_and_page_start_do_not_begin_a_trackpad_bracket() {
    for unit in [MouseWheelUnit::Line, MouseWheelUnit::Page] {
        let mut nav = Navigation::default();
        let mut event = wheel(
            MouseWheelSource::Unknown,
            TouchPhase::Start,
            vec2(0.0, 1.0),
            true,
        );
        if let Event::MouseWheel {
            unit: event_unit, ..
        } = &mut event
        {
            *event_unit = unit;
        }
        assert!(matches!(send(&mut nav, &event, 1.0), Some(Motion::Zoom(_))));
        let plain = wheel(
            MouseWheelSource::Unknown,
            TouchPhase::Move,
            vec2(0.0, 1.0),
            false,
        );
        assert!(matches!(
            send(&mut nav, &plain, 1.01),
            Some(Motion::Zoom(_))
        ));
    }
}

#[test]
fn shift_latches_through_trackpad_end_and_momentum_start() {
    let mut nav = Navigation::default();
    let delta = vec2(10.0, -5.0);
    let cases = [
        (MouseWheelSource::Trackpad, TouchPhase::Start, true),
        (MouseWheelSource::Trackpad, TouchPhase::Move, false),
        (MouseWheelSource::Trackpad, TouchPhase::End, false),
        (MouseWheelSource::Momentum, TouchPhase::Start, false),
        (MouseWheelSource::Momentum, TouchPhase::Move, false),
        (MouseWheelSource::Momentum, TouchPhase::End, false),
    ];
    for (i, (source, phase, shift)) in cases.into_iter().enumerate() {
        assert_eq!(
            send(
                &mut nav,
                &wheel(source, phase, delta, shift),
                1.0 + i as f64 * 0.02
            ),
            Some(Motion::Orbit(delta * 0.6))
        );
    }
    let fresh = wheel(MouseWheelSource::Trackpad, TouchPhase::Start, delta, false);
    assert_eq!(send(&mut nav, &fresh, 1.11), Some(Motion::Pan(delta)));
}

#[test]
fn momentum_after_idle_takes_current_shift() {
    let mut nav = Navigation::default();
    for phase in [TouchPhase::Start, TouchPhase::End] {
        send(
            &mut nav,
            &wheel(MouseWheelSource::Trackpad, phase, vec2(0.0, 1.0), true),
            1.0,
        );
    }
    let momentum = wheel(
        MouseWheelSource::Momentum,
        TouchPhase::Start,
        vec2(0.0, 1.0),
        false,
    );
    assert_eq!(
        send(&mut nav, &momentum, 1.121),
        Some(Motion::Pan(vec2(0.0, 1.0)))
    );
}

#[test]
fn unknown_bracket_and_known_phase_less_input_relatch_after_idle() {
    for source in [MouseWheelSource::Unknown, MouseWheelSource::Trackpad] {
        let mut nav = Navigation::default();
        send(
            &mut nav,
            &wheel(source, TouchPhase::Start, vec2(0.0, 1.0), true),
            1.0,
        );
        let plain = wheel(source, TouchPhase::Move, vec2(0.0, 1.0), false);
        assert_eq!(
            send(&mut nav, &plain, 1.01),
            Some(Motion::Orbit(vec2(0.0, 0.6)))
        );
        assert_eq!(
            send(&mut nav, &plain, 1.2),
            Some(Motion::Pan(vec2(0.0, 1.0)))
        );
    }
}

#[test]
fn wheel_events_never_contaminate_trackpad_latch() {
    let mut nav = Navigation::default();
    send(
        &mut nav,
        &wheel(
            MouseWheelSource::Trackpad,
            TouchPhase::Start,
            vec2(0.0, 1.0),
            true,
        ),
        1.0,
    );
    for phase in [
        TouchPhase::Start,
        TouchPhase::Move,
        TouchPhase::End,
        TouchPhase::Cancel,
    ] {
        send(
            &mut nav,
            &wheel(MouseWheelSource::Wheel, phase, vec2(0.0, 0.5), false),
            1.01,
        );
    }
    let pad = wheel(
        MouseWheelSource::Trackpad,
        TouchPhase::Move,
        vec2(0.0, 1.0),
        false,
    );
    assert_eq!(
        send(&mut nav, &pad, 1.02),
        Some(Motion::Orbit(vec2(0.0, 0.6)))
    );
}

#[test]
fn cancel_focus_loss_and_pointer_departure_clear_latches() {
    for reset in [
        Event::PointerGone,
        Event::WindowFocused(false),
        wheel(
            MouseWheelSource::Unknown,
            TouchPhase::Cancel,
            vec2(3.0, 4.0),
            false,
        ),
    ] {
        let mut nav = Navigation::default();
        let start = wheel(
            MouseWheelSource::Unknown,
            TouchPhase::Start,
            vec2(0.0, 1.0),
            true,
        );
        send(&mut nav, &start, 1.0);
        assert_eq!(send(&mut nav, &reset, 1.01), None);
        let plain = wheel(
            MouseWheelSource::Unknown,
            TouchPhase::Move,
            vec2(0.0, 1.0),
            false,
        );
        assert!(matches!(
            send(&mut nav, &plain, 1.02),
            Some(Motion::Zoom(_))
        ));
    }
}

#[test]
fn mouse_normalization_horizontal_fallback_and_vertical_precedence() {
    for (unit, scale) in [
        (MouseWheelUnit::Point, 1.0),
        (MouseWheelUnit::Line, 16.0),
        (MouseWheelUnit::Page, 600.0),
    ] {
        for (delta, axis) in [(vec2(2.0, 0.0), 2.0), (vec2(2.0, -1.0), -1.0)] {
            let mut event = wheel(MouseWheelSource::Wheel, TouchPhase::Move, delta, true);
            if let Event::MouseWheel {
                unit: event_unit, ..
            } = &mut event
            {
                *event_unit = unit;
            }
            zoom(
                send(&mut Navigation::default(), &event, 1.0),
                (-axis * scale * 0.0015_f64).exp().clamp(0.2, 5.0),
            );
        }
    }
}

#[test]
fn ctrl_always_zooms_and_pinch_is_reciprocal() {
    for source in [
        MouseWheelSource::Wheel,
        MouseWheelSource::Trackpad,
        MouseWheelSource::Unknown,
    ] {
        let mut event = wheel(source, TouchPhase::Start, vec2(0.0, 10.0), true);
        if let Event::MouseWheel { modifiers, .. } = &mut event {
            modifiers.ctrl = true;
        }
        zoom(
            send(&mut Navigation::default(), &event, 1.0),
            (-0.1_f64).exp(),
        );
    }
    for (input, expected) in [
        (2.0, 0.5),
        (0.5, 2.0),
        (f32::MIN_POSITIVE, 5.0),
        (f32::MAX, 0.2),
    ] {
        zoom(
            send(&mut Navigation::default(), &Event::Zoom(input), 1.0),
            expected,
        );
    }
}

#[test]
fn rejects_nonfinite_deltas_times_page_heights_and_invalid_pinches() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for delta in [vec2(value, 1.0), vec2(1.0, value)] {
            let event = wheel(MouseWheelSource::Trackpad, TouchPhase::Start, delta, false);
            assert_eq!(send(&mut Navigation::default(), &event, 1.0), None);
        }
    }
    for factor in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            send(&mut Navigation::default(), &Event::Zoom(factor), 1.0),
            None
        );
    }
    for time in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        assert_eq!(
            send(&mut Navigation::default(), &Event::Zoom(2.0), time),
            None
        );
    }
    for height in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(
            Navigation::default().event(&Event::Zoom(2.0), 1.0, height),
            None
        );
    }
}

#[test]
fn extreme_finite_zoom_saturates_and_pan_overflow_is_rejected() {
    for (delta, expected) in [(f32::MAX, 0.2), (-f32::MAX, 5.0)] {
        let event = Event::MouseWheel {
            unit: MouseWheelUnit::Page,
            delta: vec2(delta, delta),
            source: MouseWheelSource::Wheel,
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        zoom(Navigation::default().event(&event, 1.0, f32::MAX), expected);
        let Event::MouseWheel {
            unit,
            delta,
            modifiers,
            phase,
            ..
        } = event
        else {
            unreachable!()
        };
        let pad = Event::MouseWheel {
            unit,
            delta,
            modifiers,
            phase,
            source: MouseWheelSource::Trackpad,
        };
        assert_eq!(Navigation::default().event(&pad, 1.0, f32::MAX), None);
    }
}

#[test]
fn backward_pad_time_drops_event_and_clears_bracket() {
    let mut nav = Navigation::default();
    send(
        &mut nav,
        &wheel(
            MouseWheelSource::Unknown,
            TouchPhase::Start,
            vec2(0.0, 1.0),
            true,
        ),
        2.0,
    );
    let plain = wheel(
        MouseWheelSource::Unknown,
        TouchPhase::Move,
        vec2(0.0, 1.0),
        false,
    );
    assert_eq!(send(&mut nav, &plain, 1.0), None);
    assert!(matches!(
        send(&mut nav, &plain, 2.01),
        Some(Motion::Zoom(_))
    ));
}

#[test]
fn unrelated_events_do_not_reset_or_generate_navigation() {
    let mut nav = Navigation::default();
    send(
        &mut nav,
        &wheel(
            MouseWheelSource::Trackpad,
            TouchPhase::Start,
            vec2(0.0, 1.0),
            true,
        ),
        1.0,
    );
    assert_eq!(
        send(&mut nav, &Event::PointerMoved(egui::pos2(10.0, 20.0)), 1.01),
        None
    );
    let pad = wheel(
        MouseWheelSource::Trackpad,
        TouchPhase::Move,
        vec2(0.0, 1.0),
        false,
    );
    assert_eq!(
        send(&mut nav, &pad, 1.02),
        Some(Motion::Orbit(vec2(0.0, 0.6)))
    );
}
