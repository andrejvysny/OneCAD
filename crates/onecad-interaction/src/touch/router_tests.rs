//! INPUT.md ownership table rows, hover, double tap and cancellation.
use super::test_support::*;
use super::*;
use Phase::*;

fn press(contact: ContactId, owner: Owner, kind: ContactKind, x: f64, y: f64) -> RoutedEvent {
    RoutedEvent::Press {
        contact,
        owner,
        kind,
        position: p(x, y),
        tap_count: 1,
    }
}

#[test]
fn pencil_authors_anywhere_with_one_press_and_one_release() {
    let mut pipe = Pipeline::default();
    let events = pipe.submit(
        &[
            pencil(1, Began, 0.0, 50.0, 50.0),
            pencil(1, Moved, 0.1, 60.0, 50.0),
            pencil(1, Ended, 0.2, 70.0, 50.0),
        ],
        &with_controls(),
    );
    assert_eq!(kinds(&events), ["press", "move", "release"]);
    assert_eq!(
        events[0],
        press(1, Owner::Author, ContactKind::Pencil, 50.0, 50.0)
    );
}

#[test]
fn finger_starting_over_a_control_is_a_control_pointer() {
    let mut pipe = Pipeline::default();
    let events = pipe.submit(
        &[
            finger(1, Began, 0.0, 10.0, 10.0),
            finger(1, Moved, 0.1, 40.0, 10.0),
            finger(1, Ended, 0.9, 40.0, 10.0),
        ],
        &with_controls(),
    );
    assert_eq!(kinds(&events), ["press", "move", "release"]);
    assert_eq!(
        events[0],
        press(1, Owner::Control, ContactKind::Finger, 10.0, 10.0)
    );
}

#[test]
fn finger_tap_on_canvas_presses_and_releases_at_its_begin_position() {
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    assert!(pipe
        .submit(&[finger(1, Began, 0.0, 200.0, 200.0)], &ctx)
        .is_empty());
    let events = pipe.submit(
        &[
            finger(1, Moved, 0.1, 205.0, 203.0),
            finger(1, Ended, 0.3, 206.0, 204.0),
        ],
        &ctx,
    );
    assert_eq!(kinds(&events), ["press", "release"]);
    assert_eq!(
        events[0],
        press(1, Owner::Tap, ContactKind::Finger, 200.0, 200.0)
    );
    assert!(
        matches!(events[1], RoutedEvent::Release { position, .. } if position == p(200.0, 200.0))
    );
}

#[test]
fn a_long_press_within_the_slop_does_nothing() {
    let mut pipe = Pipeline::default();
    let events = pipe.submit(
        &[
            finger(1, Began, 0.0, 200.0, 200.0),
            finger(1, Ended, 0.6, 201.0, 200.0),
        ],
        &with_controls(),
    );
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn finger_leaving_the_slop_authors_from_its_begin_position_while_drag_authoring() {
    let mut pipe = Pipeline::default();
    let events = pipe.submit(
        &[
            finger(1, Began, 0.0, 200.0, 200.0),
            finger(1, Moved, 0.1, 204.0, 200.0),
            finger(1, Moved, 0.2, 220.0, 200.0),
            finger(1, Ended, 0.3, 230.0, 200.0),
        ],
        &authoring(),
    );
    assert_eq!(kinds(&events), ["press", "move", "release"]);
    assert_eq!(
        events[0],
        press(1, Owner::Author, ContactKind::Finger, 200.0, 200.0)
    );
}

#[test]
fn finger_orbits_from_the_slop_exit_point_without_a_jump() {
    for ctx in [
        with_controls(),
        RouteContext {
            pencil_only_drawing: true,
            drag_authoring: true,
            over_control: &|_| false,
        },
    ] {
        let mut pipe = Pipeline::default();
        let events = pipe.submit(
            &[
                finger(1, Began, 0.0, 200.0, 200.0),
                finger(1, Moved, 0.1, 210.0, 200.0),
                finger(1, Moved, 0.2, 215.0, 202.0),
                finger(1, Ended, 0.3, 215.0, 202.0),
            ],
            &ctx,
        );
        let orbit: Vec<_> = events
            .iter()
            .map(|event| match event {
                RoutedEvent::Orbit { phase, delta, .. } => (*phase, *delta),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            orbit,
            [
                (GesturePhase::Start, p(0.0, 0.0)),
                (GesturePhase::Move, p(5.0, 2.0)),
                (GesturePhase::End, p(0.0, 0.0))
            ]
        );
    }
}

#[test]
fn canvas_finger_is_ignored_while_a_pencil_authors_but_controls_still_work() {
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    pipe.submit(&[pencil(1, Began, 0.0, 300.0, 300.0)], &ctx);
    let events = pipe.submit(
        &[
            finger(2, Began, 0.1, 200.0, 200.0),
            finger(2, Moved, 0.2, 260.0, 200.0),
            finger(2, Ended, 0.3, 260.0, 200.0),
            finger(3, Began, 0.1, 10.0, 10.0),
            finger(3, Moved, 0.15, 12.0, 10.0),
            finger(3, Ended, 0.2, 12.0, 10.0),
        ],
        &ctx,
    );
    // The control is activated without taking the Pencil's pointer.
    assert_eq!(
        events,
        [RoutedEvent::Activate {
            contact: 3,
            position: p(10.0, 10.0)
        }]
    );
    assert_eq!(pipe.router.owner(1), Some(Owner::Author));
    // Dragged off the control, or cancelled, it activates nothing.
    let events = pipe.submit(
        &[
            finger(4, Began, 0.4, 10.0, 10.0),
            finger(4, Moved, 0.5, 40.0, 10.0),
            finger(4, Ended, 0.6, 40.0, 10.0),
            finger(5, Began, 0.4, 20.0, 10.0),
            finger(5, Cancelled, 0.5, 20.0, 10.0),
        ],
        &ctx,
    );
    assert!(events.is_empty(), "{events:?}");
    let released = pipe.submit(&[pencil(1, Ended, 0.7, 300.0, 300.0)], &ctx);
    assert_eq!(kinds(&released), ["release"], "the Pencil stroke is intact");
}

#[test]
fn a_pencil_takes_the_pointer_from_an_authoring_finger() {
    let mut pipe = Pipeline::default();
    let ctx = authoring();
    pipe.submit(
        &[
            finger(1, Began, 0.0, 200.0, 200.0),
            finger(1, Moved, 0.1, 240.0, 200.0),
        ],
        &ctx,
    );
    let events = pipe.submit(&[pencil(2, Began, 0.2, 300.0, 300.0)], &ctx);
    assert_eq!(
        events[0],
        RoutedEvent::Cancel {
            contact: 1,
            reason: CancelReason::PencilPrecedence
        }
    );
    assert_eq!(
        events[1],
        press(2, Owner::Author, ContactKind::Pencil, 300.0, 300.0)
    );
    let ended = pipe.submit(
        &[
            finger(1, Ended, 0.3, 240.0, 200.0),
            pencil(2, Ended, 0.4, 300.0, 300.0),
        ],
        &ctx,
    );
    assert_eq!(kinds(&ended), ["release"], "only the Pencil releases");
}

#[test]
fn a_second_finger_turns_canvas_fingers_into_multi_touch() {
    let mut pipe = Pipeline::default();
    let ctx = authoring();
    pipe.submit(
        &[
            finger(1, Began, 0.0, 200.0, 200.0),
            finger(1, Moved, 0.1, 240.0, 200.0),
        ],
        &ctx,
    );
    let events = pipe.submit(&[finger(2, Began, 0.2, 400.0, 400.0)], &ctx);
    assert_eq!(
        events,
        [
            RoutedEvent::Cancel {
                contact: 1,
                reason: CancelReason::MultiTouch
            },
            RoutedEvent::MultiTouch {
                contact: 2,
                phase: GesturePhase::Start
            }
        ]
    );
    // An orbiting finger ends in place when a second finger lands.
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    pipe.submit(
        &[
            finger(1, Began, 0.0, 200.0, 200.0),
            finger(1, Moved, 0.1, 240.0, 200.0),
        ],
        &ctx,
    );
    let events = pipe.submit(&[finger(2, Began, 0.2, 400.0, 400.0)], &ctx);
    assert_eq!(kinds(&events), ["orbit", "multitouch", "multitouch"]);
}

#[test]
fn the_remaining_finger_after_multi_touch_is_ignored_until_lifted() {
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    pipe.submit(
        &[
            finger(1, Began, 0.0, 200.0, 200.0),
            finger(2, Began, 0.0, 300.0, 300.0),
        ],
        &ctx,
    );
    let events = pipe.submit(&[finger(2, Ended, 0.1, 300.0, 300.0)], &ctx);
    assert_eq!(
        events,
        [
            RoutedEvent::MultiTouch {
                contact: 2,
                phase: GesturePhase::End
            },
            RoutedEvent::MultiTouch {
                contact: 1,
                phase: GesturePhase::End
            }
        ]
    );
    assert_eq!(pipe.router.owner(1), Some(Owner::Ignored));
    let later = pipe.submit(
        &[
            finger(1, Moved, 0.2, 260.0, 200.0),
            finger(1, Ended, 0.25, 260.0, 200.0),
        ],
        &ctx,
    );
    assert!(later.is_empty(), "{later:?}");
}

#[test]
fn an_ignored_finger_does_not_rejoin_multi_touch_when_another_lands() {
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    pipe.submit(
        &[
            finger(1, Began, 0.0, 200.0, 200.0),
            finger(2, Began, 0.0, 300.0, 300.0),
            finger(2, Ended, 0.1, 300.0, 300.0),
        ],
        &ctx,
    );
    assert_eq!(pipe.router.owner(1), Some(Owner::Ignored));
    let events = pipe.submit(
        &[
            finger(3, Began, 0.2, 400.0, 400.0),
            finger(3, Ended, 0.25, 400.0, 400.0),
        ],
        &ctx,
    );
    assert_eq!(kinds(&events), ["press", "release"], "{events:?}");
    assert_eq!(pipe.router.owner(1), Some(Owner::Ignored));
    let ended = pipe.submit(&[finger(1, Ended, 0.3, 200.0, 200.0)], &ctx);
    assert!(ended.is_empty(), "{ended:?}");
}

#[test]
fn hover_moves_without_a_button_and_is_suppressed_while_authoring() {
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    let hover = |t, x, phase| pencil(9, phase, t, x, 10.0);
    let events = pipe.submit(&[hover(0.0, 10.0, HoverMoved)], &ctx);
    assert_eq!(
        events,
        [RoutedEvent::Hover {
            kind: ContactKind::Pencil,
            position: p(10.0, 10.0)
        }]
    );
    pipe.submit(&[pencil(1, Began, 0.1, 20.0, 10.0)], &ctx);
    assert!(pipe
        .submit(&[hover(0.2, 30.0, HoverMoved)], &ctx)
        .is_empty());
    // Hover exit clears hover only: the stroke still releases normally.
    pipe.submit(&[hover(0.3, 30.0, HoverExited)], &ctx);
    let end = pipe.submit(&[pencil(1, Ended, 0.4, 25.0, 10.0)], &ctx);
    assert_eq!(kinds(&end), ["release"]);
    // Fingers cannot hover.
    assert!(pipe
        .submit(&[finger(5, HoverMoved, 0.5, 1.0, 1.0)], &ctx)
        .is_empty());
}

#[test]
fn hover_exit_follows_hover() {
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    pipe.submit(&[pencil(9, HoverMoved, 0.0, 10.0, 10.0)], &ctx);
    let events = pipe.submit(&[pencil(9, HoverExited, 0.1, 10.0, 10.0)], &ctx);
    assert_eq!(
        events,
        [RoutedEvent::HoverExit {
            kind: ContactKind::Pencil
        }]
    );
    assert_eq!(pipe.router.pencil_position(), Some(p(10.0, 10.0)));
}

#[test]
fn pencil_tip_double_tap_carries_its_tap_count() {
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    pipe.submit(
        &[
            pencil(1, Began, 0.0, 200.0, 200.0),
            pencil(1, Ended, 0.05, 200.0, 200.0),
        ],
        &ctx,
    );
    let second = Sample {
        tap_count: 2,
        ..pencil(2, Began, 0.2, 200.0, 200.0)
    };
    let events = pipe.submit(&[second, pencil(2, Ended, 0.25, 200.0, 200.0)], &ctx);
    assert!(matches!(
        events[0],
        RoutedEvent::Press {
            tap_count: 2,
            owner: Owner::Author,
            ..
        }
    ));
    assert_eq!(
        kinds(&events),
        ["press", "release"],
        "no extra synthesized click"
    );
}

#[test]
fn a_system_cancelled_contact_never_releases() {
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    pipe.submit(&[pencil(1, Began, 0.0, 200.0, 200.0)], &ctx);
    let events = pipe.submit(&[pencil(1, Cancelled, 0.1, 210.0, 200.0)], &ctx);
    assert_eq!(
        events,
        [RoutedEvent::Cancel {
            contact: 1,
            reason: CancelReason::System
        }]
    );
    assert!(pipe
        .submit(&[pencil(1, Ended, 0.2, 210.0, 200.0)], &ctx)
        .is_empty());
}

#[test]
fn cancel_all_cancels_pressed_contacts_and_ends_orbits() {
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    pipe.submit(
        &[
            pencil(1, Began, 0.0, 300.0, 300.0),
            finger(2, Began, 0.0, 10.0, 10.0),
        ],
        &ctx,
    );
    let events = pipe.cancel_all(CancelReason::TextFocus);
    assert_eq!(
        events,
        [RoutedEvent::Cancel {
            contact: 1,
            reason: CancelReason::TextFocus
        }],
        "the control finger never held the pointer, so only the Pencil cancels"
    );
}

#[test]
fn predictions_reach_the_host_only_as_overlays_of_an_authoring_contact() {
    let mut pipe = Pipeline::default();
    let ctx = with_controls();
    let predicted = |c, t, x| {
        with(
            Provenance::Predicted,
            sample(
                c,
                ContactKind::Pencil,
                Moved,
                Provenance::Predicted,
                t,
                x,
                0.0,
            ),
        )
    };
    let events = pipe.submit(
        &[pencil(1, Began, 0.0, 200.0, 0.0), predicted(1, 0.1, 999.0)],
        &ctx,
    );
    assert_eq!(
        events.last(),
        Some(&RoutedEvent::Overlay(Overlay::Prediction {
            contact: 1,
            points: vec![p(999.0, 0.0)]
        }))
    );
    let finger_prediction = with(Provenance::Predicted, finger(2, Moved, 0.1, 999.0, 0.0));
    let events = pipe.submit(
        &[finger(2, Began, 0.0, 300.0, 0.0), finger_prediction],
        &ctx,
    );
    assert!(events.is_empty(), "an undecided finger shows no prediction");
    let end = pipe.submit(&[pencil(1, Ended, 0.2, 210.0, 0.0)], &ctx);
    assert!(end.contains(&RoutedEvent::Overlay(Overlay::Prediction {
        contact: 1,
        points: vec![]
    })));
}

#[test]
fn native_navigation_is_refused_under_a_pencil_and_takes_canvas_fingers_otherwise() {
    let mut pipe = Pipeline::default();
    let ctx = authoring();
    pipe.submit(&[pencil(1, Began, 0.0, 300.0, 300.0)], &ctx);
    assert_eq!(pipe.router.native_navigation(), (false, vec![]));
    let mut pipe = Pipeline::default();
    pipe.submit(
        &[
            finger(1, Began, 0.0, 200.0, 200.0),
            finger(1, Moved, 0.1, 240.0, 200.0),
        ],
        &ctx,
    );
    let (allowed, events) = pipe.router.native_navigation();
    assert!(allowed);
    assert_eq!(
        events,
        [RoutedEvent::Cancel {
            contact: 1,
            reason: CancelReason::MultiTouch
        }]
    );
}
