//! Same-editor replacement must fence successful and failed assembly receipts.
use super::*;
use crate::{
    assembly_controls::Reply,
    bridge::{Command, Message},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn permit() -> tokio::sync::OwnedSemaphorePermit {
    Arc::new(tokio::sync::Semaphore::new(1))
        .try_acquire_owned()
        .unwrap()
}

fn report() -> onecad_app::joint_execution::JointSolveReport {
    onecad_app::joint_execution::JointSolveReport {
        iterations: 1,
        max_residual: 0.0,
        remaining_dof: 7,
        redundant_constraints: 0,
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn replacement_fences_queued_assembly_success_failure_and_prior_report() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = Box::pin(create_app(area.path())).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    let old = app
        .doc
        .document_state
        .as_ref()
        .unwrap()
        .runtime_session
        .clone();
    let cancellation = Arc::new(AtomicBool::new(false));
    let state = &mut app.doc.assembly_controls;
    state.sequence = 7;
    state.pending = true;
    state.pending_session = Some(old.clone());
    state.cancellation = Some(cancellation.clone());
    state.report = Some(report());
    app.doc
        .bridge
        .submit(Command::Document(Request::New))
        .unwrap();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc
            .document_state
            .as_ref()
            .is_some_and(|state| state.runtime_session != old)
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|display| display.context.runtime_session != old)
    })
    .await;
    assert!(cancellation.load(Ordering::Acquire));
    assert!(!app.doc.assembly_controls.pending);
    assert!(app.doc.assembly_controls.report.is_none());
    assert_eq!(app.doc.assembly_controls.sequence, 7);
    let current = app
        .doc
        .document_state
        .as_ref()
        .unwrap()
        .runtime_session
        .clone();
    app.doc.assembly_controls.sequence = 8;
    app.doc.assembly_controls.pending = true;
    app.doc.assembly_controls.pending_session = Some(current.clone());
    app.doc.status = "Current document".into();
    for result in [
        Ok(Reply {
            report: Some(report()),
            session: old.clone(),
            capture_error: None,
            changed: true,
        }),
        Err("old request failed".into()),
    ] {
        app.message(
            Message::Assembly {
                sequence: 8,
                session: old.clone(),
                result,
                _permit: permit(),
            },
            &context,
        );
        assert!(app.doc.assembly_controls.pending);
        assert!(app.doc.assembly_controls.report.is_none());
        assert_eq!(app.doc.status, "Current document");
    }
    app.message(
        Message::Assembly {
            sequence: 8,
            session: current,
            result: Err("current request failed".into()),
            _permit: permit(),
        },
        &context,
    );
    assert!(!app.doc.assembly_controls.pending);
    assert!(app.doc.status.contains("current request failed"));
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}
