//! Typed history completion receipts distinguish metadata edits from worker work.
// Reuse the isolated real-worker host without duplicating its workflow helpers.
#[allow(dead_code)]
mod native_plate_support;

use native_plate_support::{profile, Harness};
use onecad_app::{api, dto::DocumentProjection};
use onecad_core::{
    document::record::OperationRecord,
    edit::{EditCommand, VisibilityTarget},
};
use std::path::{Path, PathBuf};

#[test]
fn history_receipts_report_only_requested_regeneration() {
    native_plate_support::required_worker();
    if std::env::var_os("ONECAD_NATIVE_PLATE_TEST_PHASE").is_some() {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run(&root));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    native_plate_support::run_isolated_phase(
        root.path(),
        "history",
        "history_receipts_report_only_requested_regeneration",
    );
}

async fn run(root: &Path) {
    let harness = Harness::new(&root.join("history-host")).await;
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(
        harness
            .state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    assert!(
        !api::undo_with_outcome(&harness.state, harness.host.clone())
            .await
            .unwrap()
            .regen_expected
    );
    let region = profile::create(&harness).await;
    metadata_history(&harness).await;
    let response = api::apply_operation(
        &harness.state,
        harness.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                profile::extrude_id(),
                1,
                "History extrusion",
                profile::extrude(&region, 6.0),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    let committed = harness.settled(&response.projection).await;
    assert_eq!(committed.bodies.len(), 1);
    geometry_history(&harness, &committed).await;
    harness.close().await;
}

async fn metadata_history(harness: &Harness) {
    let key = profile::sketch_id().to_string();
    let before = harness.projection().await;
    for (command, undo_regenerates) in [
        (
            EditCommand::RenameSketch {
                sketch: profile::sketch_id(),
                name: "Renamed profile".into(),
            },
            true,
        ),
        (
            EditCommand::SetVisibility {
                target: VisibilityTarget::Sketch(profile::sketch_id()),
                visible: false,
            },
            false,
        ),
    ] {
        metadata_roundtrip(harness, command, undo_regenerates).await;
    }
    assert_ne!(
        harness.projection().await.sketches[&key],
        before.sketches[&key]
    );
}

async fn metadata_roundtrip(harness: &Harness, command: EditCommand, undo_regenerates: bool) {
    let key = profile::sketch_id().to_string();
    let prior = harness.projection().await;
    let snapshot = harness.snapshot_id();
    let edited = api::apply_edit_command(&harness.state, harness.host.clone(), command)
        .await
        .unwrap();
    let undo = api::undo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    // Rename's whole-sketch inverse conservatively requests regeneration;
    // visibility's dedicated inverse does not. Hosts must use this receipt.
    assert_eq!(undo.regen_expected, undo_regenerates);
    assert!(undo.projection.revision > edited.revision);
    assert_eq!(undo.projection.sketches[&key], prior.sketches[&key]);
    if undo.regen_expected {
        harness.settled(&undo.projection).await;
    }
    let redo = api::redo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(
        !redo.regen_expected,
        "metadata redo must not await a worker"
    );
    assert!(redo.projection.revision > undo.projection.revision);
    assert_eq!(redo.projection.sketches[&key], edited.sketches[&key]);
    if !undo_regenerates {
        legacy_projection_contract(harness).await;
        assert_eq!(harness.snapshot_id(), snapshot);
    }
}

async fn legacy_projection_contract(harness: &Harness) {
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert_eq!(undo, harness.projection().await);
    let redo = api::redo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert_eq!(redo, harness.projection().await);
    for projection in [undo, redo] {
        let value = serde_json::to_value(projection).unwrap();
        assert!(value.get("documentId").is_some());
        assert!(value.get("projection").is_none());
        assert!(value.get("regenExpected").is_none());
        assert!(value.get("regen_expected").is_none());
    }
}

async fn geometry_history(harness: &Harness, committed: &DocumentProjection) {
    let undo = api::undo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(undo.regen_expected, "geometry undo requested regeneration");
    let undone = harness.settled(&undo.projection).await;
    assert!(undone.bodies.is_empty());
    assert_eq!(undone.undo_depth + 1, committed.undo_depth);
    let redo = api::redo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(redo.regen_expected, "geometry redo requested regeneration");
    let redone = harness.settled(&redo.projection).await;
    assert_eq!(redone.bodies, committed.bodies);
    assert_eq!(redone.undo_depth, committed.undo_depth);
    let noop = api::redo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(!noop.regen_expected);
    assert_eq!(noop.projection, redone);
}
