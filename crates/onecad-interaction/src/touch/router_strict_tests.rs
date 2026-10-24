//! Pencil-only canvas policy preserves controls and precise authors.
use super::*;

#[test]
fn strict_pencil_canvas_taps_never_press_or_release() {
    for drag_authoring in [false, true] {
        for tap_count in [1, 2] {
            let mut pipe = Pipeline::default();
            let ctx = RouteContext {
                pencil_only_drawing: true,
                drag_authoring,
                over_control: &|_| false,
            };
            let mut begin = finger(1, Began, 0.0, 200.0, 200.0);
            begin.tap_count = tap_count;
            let events = pipe.submit(&[begin, finger(1, Ended, 0.1, 202.0, 200.0)], &ctx);
            assert!(events.is_empty(), "strict finger tap emitted {events:?}");
            assert_eq!(pipe.router.owner(1), None);
        }
    }
}

fn strict_controls() -> RouteContext<'static> {
    RouteContext {
        pencil_only_drawing: true,
        drag_authoring: true,
        over_control: &|point| point.x < 100.0,
    }
}

#[test]
fn strict_controls_and_precise_contacts_keep_their_pointer_streams() {
    let ctx = strict_controls();
    let mut pipe = Pipeline::default();
    let events = pipe.submit(
        &[
            finger(1, Began, 0.0, 10.0, 10.0),
            finger(1, Moved, 0.1, 40.0, 10.0),
            finger(1, Ended, 0.2, 40.0, 10.0),
        ],
        &ctx,
    );
    assert_eq!(kinds(&events), ["press", "move", "release"]);
    assert!(matches!(
        events[0],
        RoutedEvent::Press {
            owner: Owner::Control,
            ..
        }
    ));
    for kind in [ContactKind::Pencil, ContactKind::IndirectPointer] {
        let mut pipe = Pipeline::default();
        let samples = [
            sample(2, kind, Began, Provenance::Actual, 1.0, 200.0, 200.0),
            sample(2, kind, Moved, Provenance::Actual, 1.1, 220.0, 200.0),
            sample(2, kind, Ended, Provenance::Actual, 1.2, 220.0, 200.0),
        ];
        let events = pipe.submit(&samples, &ctx);
        assert_eq!(kinds(&events), ["press", "move", "release"]);
        assert!(
            matches!(events[0], RoutedEvent::Press { owner: Owner::Author, kind: source, .. } if source == kind)
        );
    }
}

#[test]
fn strict_pencil_precedence_retains_control_activation_and_blocks_navigation() {
    let ctx = strict_controls();
    let mut pipe = Pipeline::default();
    pipe.submit(
        &[
            finger(1, Began, 0.0, 200.0, 200.0),
            finger(1, Moved, 0.1, 220.0, 200.0),
        ],
        &ctx,
    );
    let events = pipe.submit(&[pencil(2, Began, 0.2, 300.0, 300.0)], &ctx);
    assert!(matches!(
        events.as_slice(),
        [
            RoutedEvent::Orbit {
                phase: GesturePhase::End,
                ..
            },
            RoutedEvent::Press {
                owner: Owner::Author,
                kind: ContactKind::Pencil,
                ..
            }
        ]
    ));
    let events = pipe.submit(
        &[
            finger(3, Began, 0.3, 10.0, 10.0),
            finger(3, Ended, 0.4, 10.0, 10.0),
            finger(1, Ended, 0.4, 220.0, 200.0),
        ],
        &ctx,
    );
    assert_eq!(
        events,
        [RoutedEvent::Activate {
            contact: 3,
            position: p(10.0, 10.0)
        }]
    );
    assert_eq!(pipe.router.native_navigation(), (false, vec![]));
    let events = pipe.submit(&[pencil(2, Cancelled, 0.5, 300.0, 300.0)], &ctx);
    assert_eq!(
        events,
        [RoutedEvent::Cancel {
            contact: 2,
            reason: CancelReason::System
        }]
    );
    assert_eq!(pipe.router.native_navigation(), (true, vec![]));
}

#[test]
fn strict_multitouch_cancellation_never_yields_a_canvas_pointer() {
    let mut pipe = Pipeline::default();
    let ctx = strict_controls();
    let events = pipe.submit(
        &[
            finger(1, Began, 0.0, 200.0, 200.0),
            finger(1, Moved, 0.1, 220.0, 200.0),
            finger(2, Began, 0.2, 400.0, 400.0),
        ],
        &ctx,
    );
    assert!(events.iter().any(|event| matches!(
        event,
        RoutedEvent::MultiTouch {
            phase: GesturePhase::Start,
            ..
        }
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        RoutedEvent::Press { .. } | RoutedEvent::Release { .. }
    )));
    let events = pipe.cancel_all(CancelReason::System);
    assert!(events.iter().any(|event| matches!(
        event,
        RoutedEvent::MultiTouch {
            phase: GesturePhase::Cancel,
            ..
        }
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        RoutedEvent::Press { .. } | RoutedEvent::Release { .. }
    )));
    assert_eq!(pipe.router.owner(1), None);
    assert_eq!(pipe.router.owner(2), None);
}
