//! Same-editor replacement must fence successful and failed assembly receipts.
use super::*;
use crate::{
    assembly_controls::{Action, Reply},
    bridge::{Command, Message},
    diagnostics::{Source, State},
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
        assert!(app.doc.diagnostics.latest_failure().is_none());
    }
    app.message(
        Message::Assembly {
            sequence: 8,
            session: current.clone(),
            result: Err("current request failed".into()),
            _permit: permit(),
        },
        &context,
    );
    assert!(!app.doc.assembly_controls.pending);
    assert!(app.doc.status.contains("current request failed"));
    assert!(app.doc.diagnostics.contains(
        Source::Assembly,
        State::Failed,
        "current request failed"
    ));
    assert_eq!(
        app.doc.diagnostics.latest_failure().unwrap().source,
        Source::Assembly
    );
    app.doc.assembly_controls.sequence = 9;
    app.doc.assembly_controls.pending = true;
    app.doc.assembly_controls.pending_session = Some(current.clone());
    app.message(
        Message::Assembly {
            sequence: 9,
            session: current,
            result: Err("stale preview: joint solve cancelled".into()),
            _permit: permit(),
        },
        &context,
    );
    assert!(app.doc.diagnostics.contains(
        Source::Assembly,
        State::Cancelled,
        "joint solve cancelled"
    ));
    assert!(app.doc.diagnostics.latest_failure().is_none());
    invalid_limits_reach_diagnostics(&mut app, &context, &mut frame).await;
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

fn invalid_joint() -> onecad_assembly::joint::Joint {
    use onecad_assembly::{
        joint::{Joint, JointId, JointKind, JointLimits, JointOrigin},
        math::{Frame, Vec3d},
        occurrence::OccurrenceId,
    };
    let frame = Frame {
        origin: Vec3d::ZERO,
        x: Vec3d::X,
        z: Vec3d::Z,
    };
    Joint {
        id: JointId(uuid::Uuid::from_u128(0x61)),
        name: "Invalid slider".into(),
        kind: JointKind::Slider,
        a: JointOrigin {
            occurrence: OccurrenceId(uuid::Uuid::from_u128(0x62)),
            frame,
        },
        b: JointOrigin {
            occurrence: OccurrenceId(uuid::Uuid::from_u128(0x63)),
            frame,
        },
        limits: Some(JointLimits { min: 1.0, max: 0.0 }),
        value: 0.0,
        suppressed: false,
    }
}

async fn invalid_limits_reach_diagnostics(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
) {
    known_joint_occurrences(app, context, frame).await;
    for attempt in 0..2 {
        crate::assembly_controls::submit(
            app,
            Action::SolveBound {
                joints: vec![invalid_joint()],
                drives: vec![],
                joint: invalid_joint().id,
                picks: vec![],
                unbound: vec![],
            },
        );
        assert!(app.doc.assembly_controls.pending);
        assert!(app.doc.diagnostics.contains(
            Source::Assembly,
            State::Pending,
            "Applying assembly change"
        ));
        assert!(app.doc.diagnostics.latest_failure().is_none());
        pump_until(app, context, frame, |app| {
            !app.doc.assembly_controls.pending
        })
        .await;
        let failure = app.doc.diagnostics.latest_failure().unwrap().clone();
        assert_eq!(failure.source, Source::Assembly);
        assert_eq!(
            failure.message,
            "invalid command: joint 00000000-0000-0000-0000-000000000061: invalid limits [1, 0]"
        );
        assert!(failure.details.contains("invalid limits [1, 0]"));
        assert_failure_toast(&app.doc.diagnostics);
        app.doc
            .diagnostics
            .observe("Unrelated informational status");
        assert!(app.doc.diagnostics.contains(
            Source::Assembly,
            State::Failed,
            "invalid limits [1, 0]"
        ));
        assert_eq!(app.doc.diagnostics.latest_failure(), Some(&failure));
        eprintln!("invalid limits attempt {attempt}: {}", failure.message);
    }
}

async fn known_joint_occurrences(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
) {
    use onecad_assembly::{
        math::RigidTransform,
        occurrence::{ComponentDefId, Occurrence, OccurrenceId},
    };
    use onecad_core::edit::AssemblyEdit;
    let definition = ComponentDefId(uuid::Uuid::from_u128(0x64));
    let mut edits = vec![AssemblyEdit::CreateDefinition {
        id: definition,
        name: "Joint diagnostic fixture".into(),
    }];
    for n in 0..2u128 {
        edits.push(AssemblyEdit::InsertOccurrence {
            occurrence: Occurrence {
                id: OccurrenceId(uuid::Uuid::from_u128(0x62 + n)),
                def: definition,
                parent: None,
                local: RigidTransform::IDENTITY,
                grounded: n == 0,
                name: format!("Joint diagnostic instance {n}"),
            },
        });
    }
    for edit in edits {
        crate::assembly_controls::submit(app, Action::Edit(edit));
        assert!(app.doc.assembly_controls.pending);
        pump_until(app, context, frame, |app| {
            !app.doc.assembly_controls.pending
        })
        .await;
        assert!(app.doc.diagnostics.latest_failure().is_none());
    }
}

fn assert_failure_toast(diagnostics: &crate::diagnostics::Diagnostics) {
    let context = egui::Context::default();
    let mut labels = Vec::new();
    for time in [0.0, 0.1] {
        let mut output = context.run_ui(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 560.0),
                )),
                ..Default::default()
            },
            |_| crate::failure_toast::show(&context, diagnostics),
        );
        output.textures_delta.clear();
        labels = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
    }
    assert!(
        labels.iter().any(|text| {
            text.contains("Assembly: invalid command: joint") && text.contains("invalid limits")
        }),
        "{labels:?}"
    );
}
