use super::*;
use crate::diagnostics::FileAction;

fn document() -> DocumentState {
    DocumentState {
        document_id: "document".into(),
        runtime_session: "session".into(),
        revision: 9,
        dirty: true,
        ..Default::default()
    }
}

fn event(attempt_id: u64, error: Option<&str>) -> AutosaveEvent {
    AutosaveEvent {
        document_id: "document".into(),
        runtime_session: "session".into(),
        document_revision: attempt_id,
        attempt_id,
        path: "/tmp/recovery.onecad".into(),
        at_ms: attempt_id * 1000,
        error: error.map(str::to_owned),
    }
}

#[test]
fn checkpoint_failure_is_visible_and_success_preserves_other_failure_sources() {
    let mut slot = Slot::default();
    let current = document();
    let mut diagnostics = Diagnostics::default();
    diagnostics.runtime(&current.runtime_session, "");
    diagnostics.record(
        Source::File(FileAction::Save),
        State::Failed,
        "Explicit Save failed",
    );
    diagnostics.record(Source::Worker, State::Failed, "Worker failed");
    slot.record(event(1, None));
    slot.record(event(2, Some("Disk full")));
    deliver(&mut slot, &current, &mut diagnostics);
    assert!(diagnostics.contains(Source::Autosave, State::Failed, "Disk full"));
    assert!(diagnostics.contains(Source::Autosave, State::Failed, "00:00:01 UTC, revision 1"));
    assert!(diagnostics.contains(Source::Autosave, State::Failed, "Save remains available"));
    assert_failed_activity(&diagnostics);
    slot.record(event(3, None));
    deliver(&mut slot, &current, &mut diagnostics);
    assert!(!diagnostics.contains(Source::Autosave, State::Failed, ""));
    assert!(diagnostics.contains(Source::Autosave, State::Succeeded, "revision 3"));
    assert!(diagnostics.contains(
        Source::File(FileAction::Save),
        State::Failed,
        "Explicit Save failed"
    ));
    assert!(diagnostics.contains(Source::Worker, State::Failed, "Worker failed"));
    assert!(current.dirty);
}

#[test]
fn late_delivery_cannot_attribute_old_runtime_status_to_replacement() {
    let mut slot = Slot::default();
    let current = DocumentState {
        runtime_session: "replacement".into(),
        ..document()
    };
    slot.record(event(1, Some("Previous runtime failure")));
    assert!(slot.take_current(Some(&current)).is_none());
    let mut next = event(2, Some("Replacement failure"));
    next.runtime_session = current.runtime_session.clone();
    slot.record(next);
    slot.record(event(1, None));
    let (_, message) = slot.take_current(Some(&current)).unwrap();
    assert!(message.contains("Replacement failure"));
    assert!(message.contains("No successful recovery checkpoint observed"));
    assert!(slot.take_current(Some(&current)).is_none());
}

#[test]
fn delayed_success_updates_last_success_without_hiding_newer_failure() {
    let mut slot = Slot::default();
    slot.record(event(3, Some("New failure")));
    slot.take_current(Some(&document())).unwrap();
    slot.record(event(2, None));
    let (state, message) = slot.take_current(Some(&document())).unwrap();
    assert_eq!(state, State::Failed);
    assert!(message.contains("New failure"));
    assert!(message.contains("revision 2"));
    slot.record(event(1, Some("Old failure")));
    assert!(slot.take_current(Some(&document())).is_none());
}

#[test]
fn checkpoint_status_bounds_utf8_payload_and_rejects_other_document_identity() {
    let mut slot = Slot::default();
    let mut failure = event(1, Some(&"界".repeat(5000)));
    failure.path = "界".repeat(20000);
    slot.record(failure);
    assert!(slot.latest.as_ref().unwrap().error.as_ref().unwrap().len() <= 4096);
    assert!(slot.latest.as_ref().unwrap().path.len() <= 32768);
    let other = DocumentState {
        document_id: "other".into(),
        ..document()
    };
    assert!(slot.take_current(Some(&other)).is_none());
    assert!(slot.take_current(None).is_none());
    assert!(slot.take_current(Some(&document())).is_some());
}

fn assert_failed_activity(diagnostics: &Diagnostics) {
    let context = egui::Context::default();
    let mut visible = false;
    for _ in 0..2 {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 560.0),
                )),
                ..Default::default()
            },
            |_| diagnostics.show(&context),
        );
        output.textures_delta.clear();
        visible |= output.shapes.iter().any(|shape| {
            matches!(&shape.shape,
            egui::epaint::Shape::Text(text) if text.galley.text() == "Activity · Failed")
        });
    }
    assert!(visible);
}
