use super::*;
use crate::input::Input;
use egui::Event;

fn sample(contact: u64, kind: u32, phase: u32, t: f64, x: f64) -> TouchSample {
    TouchSample {
        contact_id: contact,
        kind,
        phase,
        provenance: ffi::SAMPLE_ACTUAL,
        timestamp_s: t,
        x_pt: x,
        y_pt: x,
        tap_count: 1,
        ..Default::default()
    }
}
fn finger(contact: u64, phase: u32, t: f64, x: f64) -> TouchSample {
    sample(contact, ffi::CONTACT_FINGER, phase, t, x)
}

#[test]
fn ios_policy_is_strict_in_all_canvas_modes() {
    for authoring in [false, true] {
        let route = route_context(authoring, &|_| false);
        assert!(
            route.pencil_only_drawing,
            "iOS canvas routing must require Pencil"
        );
        let mut input = Input::default();
        input
            .submit(
                &[
                    finger(1, ffi::PHASE_BEGAN, 0.0, 200.0),
                    finger(1, ffi::PHASE_ENDED, 0.1, 202.0),
                ],
                1,
                &route,
            )
            .unwrap();
        let drained = input.drain();
        assert!(
            drained.events.is_empty(),
            "canvas finger tap cannot become a click: {drained:?}"
        );
        input
            .submit(
                &[
                    finger(2, ffi::PHASE_BEGAN, 1.0, 200.0),
                    finger(2, ffi::PHASE_MOVED, 1.1, 220.0),
                    finger(2, ffi::PHASE_MOVED, 1.2, 230.0),
                    finger(2, ffi::PHASE_ENDED, 1.3, 230.0),
                ],
                1,
                &route,
            )
            .unwrap();
        let drained = input.drain();
        assert!(
            !drained
                .events
                .iter()
                .any(|event| matches!(event, Event::PointerButton { .. })),
            "finger orbit must not author: {drained:?}"
        );
        assert!(
            drained
                .events
                .iter()
                .any(|event| matches!(event, Event::MouseWheel { .. })),
            "finger drag must retain navigation: {drained:?}"
        );
    }
}

#[test]
fn strict_ios_controls_pencil_and_indirect_pointer_remain_usable() {
    for authoring in [false, true] {
        let route = route_context(authoring, &|point| point.x < 100.0);
        let mut input = Input::default();
        input
            .submit(
                &[
                    finger(1, ffi::PHASE_BEGAN, 0.0, 10.0),
                    finger(1, ffi::PHASE_ENDED, 0.1, 12.0),
                ],
                1,
                &route,
            )
            .unwrap();
        assert_eq!(
            buttons(&input.drain().events),
            [true, false],
            "finger controls must remain usable"
        );
        for kind in [ffi::CONTACT_PENCIL, ffi::CONTACT_INDIRECT_POINTER] {
            let mut input = Input::default();
            input
                .submit(
                    &[
                        sample(1, kind, ffi::PHASE_BEGAN, 1.0, 200.0),
                        sample(1, kind, ffi::PHASE_MOVED, 1.1, 220.0),
                        sample(1, kind, ffi::PHASE_ENDED, 1.2, 220.0),
                    ],
                    1,
                    &route,
                )
                .unwrap();
            assert_eq!(
                buttons(&input.drain().events),
                [true, false],
                "precise source {kind} must still author"
            );
        }
    }
}
fn buttons(events: &[Event]) -> Vec<bool> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::PointerButton { pressed, .. } => Some(*pressed),
            _ => None,
        })
        .collect()
}
