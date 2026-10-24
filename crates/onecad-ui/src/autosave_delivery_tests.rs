use super::*;
use crate::diagnostics::{FileAction, Source, State};
use onecad_app::autosave::AutosaveEvent;

fn outcome(
    current: &onecad_app::dto::DocumentState,
    attempt_id: u64,
    error: Option<&str>,
) -> AutosaveEvent {
    AutosaveEvent {
        document_id: current.document_id.clone(),
        runtime_session: current.runtime_session.clone(),
        document_revision: current.revision,
        attempt_id,
        path: "/tmp/recovery.onecad".into(),
        at_ms: attempt_id * 1000,
        error: error.map(str::to_owned),
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn actual_autosave_sink_and_host_poll_report_failure_success_and_fence_replacement() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    let current = app.doc.document_state.clone().unwrap();
    let dirty = app.document_dirty();
    app.doc.diagnostics.record(
        Source::File(FileAction::Save),
        State::Failed,
        "Explicit Save failed",
    );
    app.doc
        .bridge
        .emit_autosave_for_test(outcome(&current, 1000, Some("Disk full")));
    app.poll(&context);
    assert!(app.doc.diagnostics.contains(
        Source::Autosave,
        State::Failed,
        "Save remains available"
    ));
    app.doc
        .bridge
        .emit_autosave_for_test(outcome(&current, 1001, None));
    app.poll(&context);
    assert!(app.doc.diagnostics.contains(
        Source::Autosave,
        State::Succeeded,
        "Recovery checkpoint updated"
    ));
    assert!(
        !app.doc
            .diagnostics
            .contains(Source::Autosave, State::Failed, "")
    );
    assert!(app.doc.diagnostics.contains(
        Source::File(FileAction::Save),
        State::Failed,
        "Explicit Save failed"
    ));
    assert_eq!(app.document_dirty(), dirty);
    app.document_request(Request::New);
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.doc.documents.running
            && app
                .doc
                .document_state
                .as_ref()
                .is_some_and(|state| state.runtime_session != current.runtime_session)
    })
    .await;
    app.doc
        .bridge
        .emit_autosave_for_test(outcome(&current, 1002, Some("Stale runtime failure")));
    app.poll(&context);
    assert!(!app.doc.diagnostics.contains(
        Source::Autosave,
        State::Failed,
        "Stale runtime failure"
    ));
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}
