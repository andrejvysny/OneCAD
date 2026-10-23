use super::*;
use touch::{Overlay, Point, RoutedEvent};

fn begin(input: &mut Input) {
    input.set_prediction_context(Default::default());
    input.apply_all(vec![RoutedEvent::Press {
        contact: 7,
        kind: ContactKind::Pencil,
        owner: touch::Owner::Author,
        position: Point::new(1.0, 2.0),
        tap_count: 1,
    }]);
}

fn predict(input: &mut Input, at: f64) {
    input.apply_all(vec![RoutedEvent::Overlay(Overlay::Prediction {
        contact: 7,
        points: vec![Point::new(at, at)],
    })]);
}

#[test]
fn replacing_and_rendering_predictions_never_changes_accepted_events() {
    let mut input = Input::default();
    begin(&mut input);
    let accepted = input.drain();
    predict(&mut input, 10.0);
    predict(&mut input, 20.0);
    let snapshot = input.prediction_snapshot().unwrap();
    assert_eq!(snapshot.contact, 7);
    assert_eq!(snapshot.anchor, egui::pos2(1.0, 2.0));
    assert_eq!(snapshot.points, vec![egui::pos2(20.0, 20.0)]);
    assert!(input.drain().events.is_empty());
    assert_eq!(accepted.events.len(), 2);
    input.apply_all(vec![RoutedEvent::Move {
        contact: 7,
        owner: touch::Owner::Author,
        position: Point::new(3.0, 4.0),
    }]);
    assert!(
        input.prediction_snapshot().is_none(),
        "confirmed move clears without replacement prediction"
    );
    assert_eq!(
        input.drain().events,
        vec![egui::Event::PointerMoved(egui::pos2(3.0, 4.0))]
    );
}

#[test]
fn cancellation_and_presentation_change_drop_old_snapshot() {
    let mut input = Input::default();
    begin(&mut input);
    predict(&mut input, 9.0);
    let mut changed = onecad_ui::host::PresentationContext::default();
    changed.owner.tab = 2;
    input.set_prediction_context(changed);
    assert!(input.prediction_snapshot().is_none());
    predict(&mut input, 10.0);
    input.focus_lost();
    assert!(input.prediction_snapshot().is_none());
}

#[test]
fn same_tab_home_cancels_live_contact_and_late_end_cannot_release() {
    let mut shown = crate::frame::ShownDocument::default();
    let mut owner = onecad_ui::host::OwnerIdentity {
        tab: 7,
        editor_visible: true,
    };
    assert!(!shown.switched(owner));
    let mut input = Input::default();
    begin(&mut input);
    input.drain();
    predict(&mut input, 9.0);
    owner.editor_visible = false;
    assert!(shown.switched(owner));
    input.cancel_all(CancelReason::DocumentSwitch);
    assert!(!shown.switched(owner));
    input.apply_all(vec![RoutedEvent::Release {
        contact: 7,
        kind: ContactKind::Pencil,
        owner: touch::Owner::Author,
        position: Point::new(10.0, 10.0),
    }]);
    assert_eq!(input.drain().events, vec![Event::PointerGone]);
    assert!(input.prediction_snapshot().is_none());
}
