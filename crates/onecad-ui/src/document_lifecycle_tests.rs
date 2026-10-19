//! Lifecycle decisions must survive a real scene-admission refusal.
use super::*;
use crate::{bridge::Command, document_flow::apply_close_choice};
use onecad_core::{
    document::{
        Document,
        variables::{Scalar, Unit, Variable},
    },
    ids::{DocumentId, VariableId},
    io::{
        container::{ContainerCaches, ContainerWriter, SaveMeta},
        recovery,
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn recovered_fixture(root: &Path) -> (String, PathBuf) {
    let paths =
        QaPaths::from_args(["--qa-root".into(), root.join("qa").into_os_string()].into_iter())
            .unwrap();
    let id = DocumentId::new();
    let mut document = Document::new(id);
    for index in 0..1025 {
        document.variables.upsert(Variable {
            id: VariableId::new(),
            name: format!("value_{index}"),
            value: Scalar::new(index as f64),
            unit: Unit::Mm,
        });
    }
    let destination = paths.root.join("documents/Saved metadata.onecad");
    let autosave = recovery::autosave_path(&paths.data, id);
    fs::create_dir_all(autosave.parent().unwrap()).unwrap();
    ContainerWriter::save(
        &autosave,
        &document,
        &ContainerCaches::none(),
        &SaveMeta {
            app_version: "lifecycle-regression".into(),
            created: "2026-10-19T00:00:00Z".into(),
            modified: "2026-10-19T00:00:00Z".into(),
            ..Default::default()
        },
    )
    .unwrap();
    fs::copy(&autosave, &destination).unwrap();
    recovery::write_marker(
        &paths.data,
        &recovery::SessionMarker {
            document_id: id,
            pid: i32::MAX as u32,
            opened_path: Some(destination.clone()),
            last_autosave: "2026-10-19T00:00:00Z".into(),
            title: Some("Recovered metadata".into()),
        },
    )
    .unwrap();
    (id.to_string(), destination)
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn recovered_document_remains_saveable_and_protected_after_capture_refusal() {
    let area = tempfile::tempdir().unwrap();
    let (id, destination) = recovered_fixture(area.path());
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.bridge.shared.recovery.lock().unwrap().offers.len() == 1
    })
    .await;
    app.bridge.shared.recovery.lock().unwrap().busy = true;
    logic_tick(&mut app, &context, &mut frame, true);
    assert!(app.close_prompt);
    assert!(!app.stopping);
    assert!(app.documents.confirm.is_none());
    app.bridge
        .submit(Command::Recovery {
            document_id: id.clone(),
            choice: crate::startup_recovery::Choice::Restore,
        })
        .unwrap();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.document_state
            .as_ref()
            .is_some_and(|state| state.document_id == id)
            && app
                .status
                .contains("Native scene object count budget exceeded")
    })
    .await;
    assert!(matches!(app.documents.confirm, Some(Pending::Quit)));
    apply_close_choice(&mut app, Some(2));
    assert!(app.display.is_none());
    assert!(!app.bridge.valid());
    assert!(crate::bars::save_allowed(&app));
    assert!(app.document_dirty());
    assert_eq!(
        app.document_state.as_ref().unwrap().title,
        "Recovered metadata"
    );
    assert_eq!(
        app.document_state.as_ref().unwrap().path.as_deref(),
        destination.to_str()
    );
    check_prompts_and_failed_save(&mut app, &context, &mut frame, &destination).await;
    app.document_request(Request::Save { save_as: false });
    pump_until(&mut app, &context, &mut frame, |app| !app.documents.running).await;
    assert!(!app.document_dirty());
    assert!(app.display.is_none());
    assert!(!app.bridge.valid());
    let state = app.document_state.as_ref().unwrap();
    assert_eq!(state.title, "Saved metadata");
    assert_eq!(state.path.as_deref(), destination.to_str());
    assert!(destination.is_file());
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

async fn check_prompts_and_failed_save(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
    destination: &Path,
) {
    logic_tick(app, context, frame, true);
    assert!(matches!(app.documents.confirm, Some(Pending::Quit)));
    apply_close_choice(app, Some(2));
    assert!(app.documents.confirm.is_none());
    assert!(!app.stopping);
    app.document_request(Request::New);
    assert!(matches!(
        app.documents.confirm,
        Some(Pending::Document(Request::New))
    ));
    apply_close_choice(app, Some(2));
    fs::remove_file(destination).unwrap();
    fs::create_dir(destination).unwrap();
    app.document_request(Request::Save { save_as: false });
    pump_until(app, context, frame, |app| !app.documents.running).await;
    assert!(app.document_dirty());
    assert_eq!(
        app.document_state.as_ref().unwrap().title,
        "Recovered metadata"
    );
    assert_eq!(
        app.document_state.as_ref().unwrap().path.as_deref(),
        destination.to_str()
    );
    fs::remove_dir(destination).unwrap();
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn late_clean_display_cannot_replace_authoritative_dirty_lifecycle() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| app.display.is_some()).await;
    let old = app.display.clone().unwrap();
    app.bridge
        .shared
        .document_state
        .lock()
        .unwrap()
        .publish(onecad_app::dto::DocumentState {
            document_id: "replacement".into(),
            runtime_session: "replacement".into(),
            revision: 9,
            dirty: true,
            title: "Unsaved replacement".into(),
            path: Some("/replacement.onecad".into()),
        });
    app.sync_document_state();
    app.adopt_display(old);
    assert!(app.document_dirty());
    assert_eq!(
        app.document_state.as_ref().unwrap().title,
        "Unsaved replacement"
    );
    assert_eq!(
        app.document_state.as_ref().unwrap().path.as_deref(),
        Some("/replacement.onecad")
    );
    assert!(crate::bars::save_allowed(&app));
    logic_tick(&mut app, &context, &mut frame, true);
    assert!(matches!(app.documents.confirm, Some(Pending::Quit)));
    apply_close_choice(&mut app, Some(2));
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn discard_quit_cleans_recovery_while_normal_stop_retains_dirty_recovery() {
    for discard in [false, true] {
        let area = tempfile::tempdir().unwrap();
        let (id, _) = recovered_fixture(area.path());
        let autosave = recovery::autosave_path(&area.path().join("qa/data"), id.parse().unwrap());
        let (mut app, context) = create_app(area.path()).await;
        let mut frame = eframe::Frame::_new_kittest();
        pump_until(&mut app, &context, &mut frame, |app| {
            app.bridge.shared.recovery.lock().unwrap().offers.len() == 1
        })
        .await;
        app.bridge
            .submit(Command::Recovery {
                document_id: id.clone(),
                choice: crate::startup_recovery::Choice::Restore,
            })
            .unwrap();
        pump_until(&mut app, &context, &mut frame, |app| {
            app.document_state
                .as_ref()
                .is_some_and(|state| state.document_id == id)
                && app
                    .status
                    .contains("Native scene object count budget exceeded")
        })
        .await;
        assert!(autosave.is_file());
        if discard {
            logic_tick(&mut app, &context, &mut frame, true);
            assert!(matches!(app.documents.confirm, Some(Pending::Quit)));
            apply_close_choice(&mut app, Some(1));
        } else {
            app.stop();
        }
        pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
        assert_eq!(autosave.exists(), !discard);
    }
}
