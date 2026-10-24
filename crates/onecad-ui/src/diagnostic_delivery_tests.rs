//! Native-host callback coverage; this does not qualify physical input or file dialogs.
use super::*;
use crate::diagnostics::{FileAction, Source, State};
use onecad_core::{
    document::Document,
    ids::{ConstraintId, DocumentId, EntityId},
    io::container::{ContainerCaches, ContainerWriter, SaveMeta},
    sketch::Constraint,
};
use onecad_interaction::sketch::{SketchAction, SketchPhase, SketchStatus};

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn file_failure_callbacks_remain_visible_until_retry_or_document_replacement() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    let destination = area.path().join("qa/documents/Diagnostics.onecad");
    ContainerWriter::save(
        &destination,
        &Document::new(DocumentId::new()),
        &ContainerCaches::none(),
        &SaveMeta::default(),
    )
    .unwrap();
    app.document_request(Request::Open(Some(destination.clone())));
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.doc.documents.running
    })
    .await;
    assert!(app.doc.status.contains("opened"), "{}", app.doc.status);
    let before = app
        .doc
        .document_state
        .as_ref()
        .unwrap()
        .runtime_session
        .clone();
    let original = std::fs::read(&destination).unwrap();
    let read_only =
        super::document_lifecycle_tests::ReadOnlyDirectory::new(destination.parent().unwrap());
    app.document_request(Request::Save { save_as: false });
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.doc.documents.running
    })
    .await;
    assert!(
        app.doc
            .diagnostics
            .contains(Source::File(FileAction::Save), State::Failed, "")
    );
    assert!(
        app.doc.status.contains("Permission denied (os error 13)"),
        "{}",
        app.doc.status
    );
    assert_eq!(std::fs::read(&destination).unwrap(), original);
    drop(read_only);
    app.document_request(Request::Open(Some(area.path().join("missing.onecad"))));
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.doc.documents.running
    })
    .await;
    assert!(app.doc.diagnostics.contains(
        Source::File(FileAction::Open),
        State::Failed,
        "No such file"
    ));
    // Export uses an OS dialog. Inject its failure at the real application delivery callback.
    app.doc.documents.running = true;
    app.doc.documents.diagnostic_action = Some(FileAction::Export);
    app.document_finished(Err("Export destination is read-only".into()));
    app.doc.status = "Profile selected".into();
    assert!(app.doc.diagnostics.contains(
        Source::File(FileAction::Export),
        State::Failed,
        "read-only"
    ));
    assert!(
        app.doc
            .diagnostics
            .contains(Source::File(FileAction::Save), State::Failed, "")
    );
    app.document_request(Request::New);
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.doc.documents.running
    })
    .await;
    assert_ne!(
        app.doc.document_state.as_ref().unwrap().runtime_session,
        before
    );
    assert!(
        !app.doc
            .diagnostics
            .contains(Source::File(FileAction::Save), State::Failed, "")
    );
    assert!(
        !app.doc
            .diagnostics
            .contains(Source::File(FileAction::Export), State::Failed, "")
    );
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn rejected_sketch_mutation_and_worker_failure_reach_editor_diagnostics() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    app.new_sketch();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
    let key = app.doc.sketch.session.as_ref().unwrap().key.clone();
    let mut stale = app.doc.sketch.session.as_ref().unwrap().request.clone();
    stale.sequence += 1;
    let permit = std::sync::Arc::new(tokio::sync::Semaphore::new(1))
        .acquire_owned()
        .await
        .unwrap();
    app.message(
        crate::bridge::Message::Sketch {
            delivery: Box::new(crate::sketch_services::Delivery {
                tag: Some(stale.clone()),
                display: None,
                actions: vec![SketchAction::Failed {
                    tag: stale,
                    message: "Stale sketch failure".into(),
                }],
                error: Some("Stale sketch failure".into()),
                _lease: None,
            }),
            _permit: permit,
        },
        &context,
    );
    assert!(!app.doc.status.contains("Stale sketch failure"));
    assert!(
        !app.doc
            .diagnostics
            .contains(Source::Sketch, State::Failed, "Stale sketch failure")
    );
    app.sketch_dispatch(SketchAction::AddConstraints {
        key,
        constraints: vec![Constraint::Horizontal {
            id: ConstraintId::new(),
            line: EntityId::new(),
        }],
    });
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.sketch.session.as_ref().is_some_and(|session| {
            matches!(
                session.status,
                SketchStatus::Refused(_) | SketchStatus::Failed { .. }
            )
        })
    })
    .await;
    assert!(
        app.doc
            .diagnostics
            .contains(Source::Sketch, State::Rejected, "")
            || app
                .doc
                .diagnostics
                .contains(Source::Sketch, State::Failed, "")
    );
    assert!(
        app.doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .authoritative
            .entities()
            .is_empty()
    );
    app.sketch_dispatch(SketchAction::Discard);
    pump_until(&mut app, &context, &mut frame, |app| !app.sketch_active()).await;
    // Exercise the authoritative worker-failure delivery slot without killing a process.
    *app.doc.bridge.shared.failure.lock().unwrap() = Some("Worker transport disconnected".into());
    logic_tick(&mut app, &context, &mut frame, false);
    assert!(app.doc.diagnostics.contains(
        Source::Worker,
        State::Failed,
        "Worker transport disconnected"
    ));
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn stale_same_runtime_model_terminal_does_not_replace_current_diagnostic() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    let tag = onecad_interaction::RequestTag {
        context: app.doc.interaction.context.clone().unwrap(),
        record_id: onecad_core::ids::RecordId::new(),
        gesture: 1,
        sequence: 2,
    };
    app.doc.interaction.status = onecad_interaction::CommitStatus::Pending(tag.clone());
    let mut stale = tag;
    stale.sequence -= 1;
    app.doc.status = "Current feature pending".into();
    let permit = std::sync::Arc::new(tokio::sync::Semaphore::new(1))
        .acquire_owned()
        .await
        .unwrap();
    app.message(
        crate::bridge::Message::Terminal {
            command: "Apply",
            tag: Some(stale),
            result: Err("Stale model failure".into()),
            _permit: permit,
        },
        &context,
    );
    assert_eq!(app.doc.status, "Current feature pending");
    assert!(
        !app.doc
            .diagnostics
            .contains(Source::Model, State::Rejected, "Stale model failure")
    );
    app.doc.interaction.status = onecad_interaction::CommitStatus::Idle;
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}
