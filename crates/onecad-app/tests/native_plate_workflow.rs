//! ONECAD-273 service regression using a synthetic, fully constrained 80×50 mm
//! plate. Dimensions and 6→8 mm extrusion are test inputs, not approved product
//! requirements. This proves shared application/worker behavior, not native input.
//!
//! Required: ONECAD_WORKER_PATH=<real binary>. Optional:
//! ONECAD_NATIVE_PLATE_FIXTURE=<explicit QA .onecad output>. The fixture contains
//! only the finished constrained profile, ready for native region selection.
//! Each workflow phase runs in a fresh test-host process and worker process.
mod native_plate_support;
use native_plate_support::{
    geometry::{assert_mesh, assert_plate},
    profile::{self, EDITED_DEPTH, FIRST_DEPTH},
    Harness,
};
use onecad_app::{
    api,
    dto::{DocumentProjection, FinishSketchDto},
};
use onecad_core::{
    document::record::OperationRecord, edit::EditCommand, ids::SnapshotId, regen::Lod,
};
use std::path::{Path, PathBuf};

const PHASE: &str = "ONECAD_NATIVE_PLATE_TEST_PHASE";
const QA_ROOT: &str = "ONECAD_NATIVE_PLATE_TEST_ROOT";

#[test]
fn synthetic_plate_native_workflow() {
    native_plate_support::required_worker();
    if let Ok(phase) = std::env::var(PHASE) {
        let root = PathBuf::from(std::env::var_os(QA_ROOT).expect("child QA root required"));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            match phase.as_str() {
                "create" => create_edit_save(&root).await,
                "reopen" => reopen_edit(&root).await,
                _ => panic!("unknown workflow phase: {phase}"),
            }
        });
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("onecad-273-plate-")
        .tempdir()
        .unwrap();
    for phase in ["create", "reopen"] {
        native_plate_support::run_isolated_phase(
            root.path(),
            phase,
            "synthetic_plate_native_workflow",
        );
    }
    assert!(root.path().join("edited-plate.onecad").is_file());
}

async fn create_edit_save(root: &Path) {
    let harness = Harness::new(&root.join("create-host")).await;
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    let worker = harness.state.live_worker().expect("real worker required");
    assert!(worker.wait_ready(std::time::Duration::from_secs(15)).await);
    let region = profile::create(&harness).await;
    std::fs::write(
        root.join("original-region-id.txt"),
        &region.regions[0].region_id,
    )
    .unwrap();
    let fixture = save_profile_fixture(&harness, root).await;
    let before = harness.projection().await;
    assert!(before.bodies.is_empty());
    assert!(before.sketches[&profile::sketch_id().to_string()].visible);
    let preview_body = preview_unchanged(&harness, &region).await;
    let first = commit(&harness, &region, FIRST_DEPTH, false).await;
    assert_eq!(
        first.undo_depth,
        before.undo_depth + 1,
        "extrude commits one history step"
    );
    assert!(
        first.bodies.contains_key(&preview_body),
        "preview and commit preserve body identity"
    );
    let first_mesh = assert_plate(&harness, &first, FIRST_DEPTH).await;
    let edited = commit(&harness, &region, EDITED_DEPTH, true).await;
    assert_eq!(
        edited.undo_depth,
        first.undo_depth + 1,
        "numeric edit commits one history step"
    );
    assert_plate(&harness, &edited, EDITED_DEPTH).await;
    undo_redo(&harness, &first, &edited, &first_mesh).await;
    save(&harness, &root.join("edited-plate.onecad")).await;
    harness.close().await;
    eprintln!(
        "synthetic constrained profile fixture: {}",
        fixture.display()
    );
}

async fn save_profile_fixture(harness: &Harness, root: &Path) -> PathBuf {
    let fixture = std::env::var_os("ONECAD_NATIVE_PLATE_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("constrained-plate-80x50.onecad"));
    if let Some(parent) = fixture.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    save(harness, &fixture).await;
    fixture
}

async fn preview_unchanged(harness: &Harness, region: &FinishSketchDto) -> String {
    let before = harness.projection().await;
    let context = api::preview_context(&harness.state).await.unwrap();
    let snapshot = harness.snapshot_id();
    assert_eq!(context.snapshot_id.0, snapshot);
    let event_count = harness.document_event_count();
    let preview = api::preview_operation(
        &harness.state,
        profile::extrude(region, FIRST_DEPTH),
        profile::extrude_id(),
        Some(profile::sketch_id().to_string()),
        Some(SnapshotId(snapshot)),
        Lod::Fine,
    )
    .await
    .unwrap();
    assert_eq!(preview.snapshot_id, snapshot);
    assert_eq!(preview.bodies.len(), 1);
    assert!(preview.needs_repair.is_empty());
    assert_mesh(&preview.bodies[0].mesh, FIRST_DEPTH);
    assert_eq!(
        harness.projection().await,
        before,
        "preview leaves document/history unchanged"
    );
    assert_eq!(harness.snapshot_id(), snapshot);
    assert_eq!(
        harness.document_event_count(),
        event_count,
        "preview publishes no document mutation"
    );
    assert_eq!(
        api::preview_context(&harness.state).await.unwrap(),
        context,
        "preview leaves worker head snapshot/hash/epoch unchanged"
    );
    preview.bodies[0].body_id.clone()
}

async fn commit(
    harness: &Harness,
    region: &FinishSketchDto,
    depth: f64,
    edit: bool,
) -> DocumentProjection {
    let operation = profile::extrude(region, depth);
    let command = if edit {
        EditCommand::UpdateOperationParams {
            record: profile::extrude_id(),
            op: operation,
        }
    } else {
        EditCommand::AddOperation {
            record: OperationRecord::new(
                profile::extrude_id(),
                1,
                "Synthetic plate extrusion",
                operation,
            ),
            at_cursor: true,
        }
    };
    let effects = (!edit).then(|| api::OperationEffects {
        hide_sketch_ids: vec![profile::sketch_id().to_string()],
    });
    let response =
        api::apply_operation(&harness.state, harness.host.clone(), vec![command], effects)
            .await
            .unwrap();
    harness.settled(&response.projection).await
}

async fn undo_redo(
    harness: &Harness,
    first: &DocumentProjection,
    edited: &DocumentProjection,
    first_mesh: &[u8],
) {
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    let undo = harness.settled(&undo).await;
    assert_eq!(undo.undo_depth, first.undo_depth);
    assert_eq!(undo.redo_depth, 1);
    assert_eq!(assert_plate(harness, &undo, FIRST_DEPTH).await, first_mesh);
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    let undo = harness.settled(&undo).await;
    assert!(
        undo.bodies.is_empty(),
        "second undo removes the initial extrusion"
    );
    assert!(
        undo.sketches[&profile::sketch_id().to_string()].visible,
        "same undo restores automatic sketch visibility"
    );
    for depth in [FIRST_DEPTH, EDITED_DEPTH] {
        let redo = api::redo(&harness.state, harness.host.clone())
            .await
            .unwrap();
        let redo = harness.settled(&redo).await;
        assert_plate(harness, &redo, depth).await;
    }
    let projection = harness.projection().await;
    assert_eq!(projection.undo_depth, edited.undo_depth);
    assert_eq!(projection.redo_depth, 0);
}

async fn reopen_edit(root: &Path) {
    let harness = Harness::new(&root.join("reopen-host")).await;
    api::open_document(
        &harness.state,
        harness.host.clone(),
        root.join("edited-plate.onecad")
            .to_string_lossy()
            .into_owned(),
        None,
    )
    .await
    .unwrap();
    let initial = harness.projection().await;
    assert!(!initial.dirty, "loaded saved document starts clean");
    let reopened = harness.settled(&initial).await;
    assert!(!reopened.dirty, "replay does not create an unsaved edit");
    assert_plate(&harness, &reopened, EDITED_DEPTH).await;
    let regions = api::get_sketch_regions(&harness.state, profile::sketch_id().to_string())
        .await
        .unwrap();
    assert_persisted_operation(&harness, root, &regions, &reopened).await;
    let before = harness.projection().await;
    let edited = commit(&harness, &regions, FIRST_DEPTH, true).await;
    assert_eq!(edited.undo_depth, before.undo_depth + 1);
    assert_plate(&harness, &edited, FIRST_DEPTH).await;
    save(&harness, &root.join("edited-plate.onecad")).await;
    harness.close().await;
}

async fn assert_persisted_operation(
    harness: &Harness,
    root: &Path,
    regions: &FinishSketchDto,
    reopened: &DocumentProjection,
) {
    let original_region = std::fs::read_to_string(root.join("original-region-id.txt")).unwrap();
    assert_eq!(regions.regions.len(), 1);
    assert_eq!(
        regions.regions[0].region_id, original_region,
        "profile identity survives process restart"
    );
    // SCHEMA §7.4 new authoring uses cell-v3; persisted identity must stay frozen.
    assert_eq!(regions.region_identity_version, 3);
    let record = api::get_operation_record(&harness.state, profile::extrude_id())
        .await
        .unwrap();
    assert_eq!(record.record_id, profile::extrude_id());
    assert_eq!(record.step_index, 1);
    assert!(!record.suppressed);
    assert_eq!(
        record.op,
        profile::extrude(regions, EDITED_DEPTH),
        "persisted profile ref and every extrusion parameter survive unchanged"
    );
    assert_eq!(record.inputs, record.op.derive_inputs());
    assert!(
        reopened
            .bodies
            .contains_key(&profile::extrude_id().to_string()),
        "output body identity survives restart"
    );
}

async fn save(harness: &Harness, path: &Path) {
    let saved = api::save_document(
        &harness.state,
        harness.host.clone(),
        Some(path.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean, "save must persist the current revision");
    assert!(path.is_file());
    assert!(!harness.projection().await.dirty);
}
