//! Real-worker qualification for a sketch visit over a full rolling model stack.
//! Active visits qualify solved sketch state and scheduled replay. Solid geometry
//! is qualified after Finish/Discard, when the timeline sketch record is current.
#[allow(dead_code)]
mod native_plate_support;

use native_plate_support::{profile, Harness};
use onecad_app::{
    api,
    dto::{SketchHistoryScope, SketchSessionDto, SketchSolveStatus},
};
use onecad_core::{
    document::{record::OperationRecord, variables::Scalar},
    edit::{EditCommand, HistoryDirection, SketchEditOp, VisibilityTarget, UNDO_CAP},
    ids::{ConstraintId, SnapshotId},
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[test]
fn full_history_constrained_solid_finish_discard_and_reopen() {
    native_plate_support::required_worker();
    if let Ok(phase) = std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                match phase.as_str() {
                    "create" => create(&root).await,
                    "reopen" => reopen(&root).await,
                    _ => panic!("unknown history phase: {phase}"),
                }
            });
        return;
    }
    let root = tempfile::tempdir().unwrap();
    for phase in ["create", "reopen"] {
        native_plate_support::run_isolated_phase(
            root.path(),
            phase,
            "full_history_constrained_solid_finish_discard_and_reopen",
        );
    }
}

async fn create(root: &Path) {
    let harness = Harness::new(&root.join("create-host")).await;
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
    let region = profile::create(&harness).await;
    std::fs::write(root.join("region.txt"), &region.regions[0].region_id).unwrap();
    let committed = api::apply_operation(
        &harness.state,
        harness.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                profile::extrude_id(),
                1,
                "History solid",
                profile::extrude(&region, 6.0),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    harness.settled(&committed.projection).await;
    assert_solid(&harness, 80.0).await;
    finish_partial_visit(&harness).await;
    discard_partial_visit(&harness).await;
    save(&harness, root).await;
    harness.close().await;
}

async fn save(harness: &Harness, root: &Path) {
    let snapshot = sketch(harness).await;
    std::fs::write(
        root.join("sketch.json"),
        serde_json::to_vec(&serde_json::json!({
            "entities": snapshot.entities,
            "constraints": snapshot.constraints,
        }))
        .unwrap(),
    )
    .unwrap();
    let saved = api::save_document(
        &harness.state,
        harness.host.clone(),
        Some(root.join("history.onecad").to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    assert!(saved.clean);
}

async fn fill_model_history(harness: &Harness) {
    for index in 0..UNDO_CAP {
        api::apply_edit_command(
            &harness.state,
            harness.host.clone(),
            EditCommand::SetVisibility {
                target: VisibilityTarget::Sketch(profile::sketch_id()),
                visible: index % 2 == 0,
            },
        )
        .await
        .unwrap();
    }
    assert_eq!(harness.projection().await.undo_depth, UNDO_CAP);
}

async fn enter(harness: &Harness) {
    let completion = api::enter_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        profile::sketch_id().to_string(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        completion.result.unwrap().status,
        SketchSolveStatus::FullyConstrained
    );
    if completion.regen_expected {
        harness.settled(&completion.projection).await;
    }
}

async fn sketch(harness: &Harness) -> SketchSessionDto {
    api::get_sketch(&harness.state, profile::sketch_id().to_string())
        .await
        .unwrap()
}

async fn set_width(harness: &Harness, width: f64) {
    let solve = api::sketch_upsert(
        &harness.state,
        harness.host.clone(),
        profile::sketch_id().to_string(),
        vec![SketchEditOp::SetDimension {
            constraint: ConstraintId(Uuid::from_u128(0x273209)),
            value: Scalar::with_expr(width, format!("{width}mm")),
        }],
    )
    .await
    .unwrap();
    assert_eq!(solve.status, SketchSolveStatus::FullyConstrained);
    assert_eq!(solve.dof, 0);
    assert!(solve.conflicting.is_empty());
    harness.settled(&harness.projection().await).await;
    assert_width(&sketch(harness).await, width);
    assert_expression(harness, width).await;
}

async fn history(harness: &Harness, direction: HistoryDirection, width: f64) {
    let scope = {
        let guard = harness.state.runtime.lock().await;
        let runtime = guard.as_ref().unwrap();
        SketchHistoryScope {
            sketch_id: profile::sketch_id(),
            edit_session: runtime
                .get_sketch(profile::sketch_id())
                .unwrap()
                .edit_session
                .unwrap(),
            runtime_session: runtime.runtime_session().to_string(),
            revision: runtime.revision().0,
            snapshot_id: runtime.head_snapshot_id().unwrap_or(SnapshotId(0)),
        }
    };
    let completion =
        api::sketch_history_with_completion(&harness.state, harness.host.clone(), scope, direction)
            .await
            .unwrap();
    assert!(completion.result.unwrap().moved);
    assert!(
        completion.regen_expected,
        "downstream solid must regenerate"
    );
    harness.settled(&completion.projection).await;
    assert_width(&sketch(harness).await, width);
    assert_expression(harness, width).await;
}

async fn finish_partial_visit(harness: &Harness) {
    fill_model_history(harness).await;
    enter(harness).await;
    let baseline = sketch(harness).await;
    set_width(harness, 90.0).await;
    let first = sketch(harness).await;
    assert_ne!(
        first.entities, baseline.entities,
        "dimension solve must move coordinates"
    );
    assert_same_ids(&baseline, &first);
    set_width(harness, 100.0).await;
    history(harness, HistoryDirection::Undo, 90.0).await;
    history(harness, HistoryDirection::Redo, 100.0).await;
    history(harness, HistoryDirection::Undo, 90.0).await;
    let finish = api::finish_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        profile::sketch_id().to_string(),
    )
    .await
    .unwrap();
    assert_eq!(finish.result.unwrap().regions.len(), 1);
    if finish.regen_expected {
        harness.settled(&finish.projection).await;
    }
    assert_eq!(harness.projection().await.redo_depth, 0);
    let kept = sketch(harness).await;
    assert!(kept.edit_session.is_none());
    assert_same_ids(&baseline, &kept);
    assert_width(&kept, 90.0);
    assert_expression(harness, 90.0).await;
    assert_solid(harness, 90.0).await;
}

async fn discard_partial_visit(harness: &Harness) {
    fill_model_history(harness).await;
    enter(harness).await;
    let baseline = sketch(harness).await;
    set_width(harness, 95.0).await;
    set_width(harness, 105.0).await;
    history(harness, HistoryDirection::Undo, 95.0).await;
    let completion = api::cancel_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        profile::sketch_id().to_string(),
        Some(true),
    )
    .await
    .unwrap();
    assert!(completion.result.unwrap().discarded);
    assert!(completion.regen_expected);
    harness.settled(&completion.projection).await;
    let discarded = sketch(harness).await;
    assert_same_ids(&baseline, &discarded);
    assert_eq!(discarded.constraints, baseline.constraints);
    assert_width(&discarded, 90.0);
    assert_expression(harness, 90.0).await;
    assert_solid(harness, 90.0).await;
    let projection = harness.projection().await;
    assert_eq!((projection.undo_depth, projection.redo_depth), (198, 0));
}

fn assert_width(sketch: &SketchSessionDto, width: f64) {
    let point = sketch
        .entities
        .as_array()
        .unwrap()
        .iter()
        .find(|entity| entity["id"] == Uuid::from_u128(0x273101).to_string())
        .unwrap();
    assert!((point["at"][0].as_f64().unwrap() - width).abs() < 1e-7);
    assert!(point["at"][1].as_f64().unwrap().abs() < 1e-7);
}

async fn assert_expression(harness: &Harness, width: f64) {
    let guard = harness.state.runtime.lock().await;
    let sketch = guard
        .as_ref()
        .unwrap()
        .sketch_snapshot(profile::sketch_id(), "history test")
        .unwrap();
    let constraint = sketch
        .constraints()
        .iter()
        .find(|constraint| constraint.id() == ConstraintId(Uuid::from_u128(0x273209)))
        .unwrap();
    let onecad_core::sketch::Constraint::HorizontalDistance { value, .. } = constraint else {
        panic!("width constraint kind changed");
    };
    assert_eq!(value.expr.as_deref(), Some(format!("{width}mm").as_str()));
}

fn assert_same_ids(before: &SketchSessionDto, after: &SketchSessionDto) {
    for (before, after) in [
        (&before.entities, &after.entities),
        (&before.constraints, &after.constraints),
    ] {
        let ids = |value: &serde_json::Value| {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(before), ids(after));
    }
}

async fn assert_solid(harness: &Harness, width: f64) {
    let projection = harness.projection().await;
    assert_eq!(projection.bodies.len(), 1);
    let id = profile::extrude_id().to_string();
    assert!(
        projection.bodies.contains_key(&id),
        "solid identity must stay stable"
    );
    let wire_id = onecad_app::worker::wire::body_id_wire(id.parse().unwrap());
    let topology = api::query_body_topology(&harness.state, wire_id.clone())
        .await
        .unwrap();
    assert_eq!((topology.solid_count, topology.face_count), (1, 6));
    let mass = api::query_mass_properties(&harness.state, wire_id, None)
        .await
        .unwrap();
    assert!(
        (mass.volume - width * profile::HEIGHT * 6.0).abs() < 1e-5,
        "width {width}: volume {} must equal {}",
        mass.volume,
        width * profile::HEIGHT * 6.0
    );
}

async fn reopen(root: &Path) {
    let harness = Harness::new(&root.join("reopen-host")).await;
    api::open_document(
        &harness.state,
        harness.host.clone(),
        root.join("history.onecad").to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let opened = harness.projection().await;
    harness.settled(&opened).await;
    assert!(!harness.projection().await.dirty);
    let snapshot = sketch(&harness).await;
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("sketch.json")).unwrap()).unwrap();
    assert_eq!(snapshot.entities, saved["entities"]);
    assert_eq!(snapshot.constraints, saved["constraints"]);
    assert_width(&snapshot, 90.0);
    assert_expression(&harness, 90.0).await;
    assert_solid(&harness, 90.0).await;
    let regions = api::get_sketch_regions(&harness.state, profile::sketch_id().to_string())
        .await
        .unwrap();
    assert_eq!(
        regions.regions[0].region_id,
        std::fs::read_to_string(root.join("region.txt")).unwrap()
    );
    harness.close().await;
}
