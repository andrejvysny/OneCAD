//! Application receipts retain scheduled work before a host captures sketch geometry.
#[allow(dead_code)]
mod native_plate_support;

use native_plate_support::{profile, Harness};
use onecad_app::{api, error::ApiError};
use onecad_core::{
    document::{record::OperationRecord, variables::Scalar},
    edit::{EditCommand, SketchEditOp},
    ids::{ConstraintId, EntityId, SketchId},
    math::Vec2,
    sketch::{Sketch, SketchEntity, WorldPlane},
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[test]
fn sketch_completion_receipts_cover_rebuilds_noops_and_recorded_refusals() {
    native_plate_support::required_worker();
    if let Ok(phase) = std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run(&root, &phase));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    for phase in ["dependent", "empty", "refusal"] {
        native_plate_support::run_isolated_phase(
            root.path(),
            phase,
            "sketch_completion_receipts_cover_rebuilds_noops_and_recorded_refusals",
        );
    }
}

async fn run(root: &Path, phase: &str) {
    let harness = Harness::new(&root.join(format!("{phase}-host"))).await;
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
    match phase {
        "dependent" => dependent_profile(&harness).await,
        "empty" => empty_profile(&harness).await,
        "refusal" => recorded_refusal(&harness).await,
        _ => panic!("unknown sketch completion phase"),
    }
    harness.close().await;
}

async fn settle<T>(harness: &Harness, completion: api::CommandCompletion<T>) -> T {
    if completion.regen_expected {
        harness.settled(&completion.projection).await;
    }
    completion
        .result
        .unwrap_or_else(|error| panic!("{error:?}"))
}

async fn enter(harness: &Harness, id: SketchId, created: bool) {
    let receipt = api::enter_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        id.to_string(),
        Some(created),
    )
    .await
    .unwrap();
    assert!(
        !receipt.regen_expected,
        "a solved entry changes no geometry"
    );
    settle(harness, receipt).await;
}

async fn change_width(harness: &Harness, width: f64) {
    let receipt = api::sketch_upsert_with_completion(
        &harness.state,
        harness.host.clone(),
        profile::sketch_id().to_string(),
        vec![SketchEditOp::SetDimension {
            constraint: ConstraintId(Uuid::from_u128(0x273209)),
            value: Scalar::new(width),
        }],
    )
    .await
    .unwrap();
    assert!(
        receipt.regen_expected,
        "the dependent extrusion must rebuild"
    );
    settle(harness, receipt).await;
    let sketch = api::get_sketch_snapshot(&harness.state, profile::sketch_id())
        .await
        .unwrap();
    let dimension = sketch
        .constraints()
        .iter()
        .find(|constraint| constraint.id() == ConstraintId(Uuid::from_u128(0x273209)))
        .unwrap();
    assert_eq!(dimension.value().unwrap().value, width);
}

async fn assert_volume(harness: &Harness, width: f64) {
    let projection = harness.projection().await;
    assert_eq!(projection.bodies.len(), 1);
    let body = projection.bodies.keys().next().unwrap().parse().unwrap();
    let properties = api::query_mass_properties(
        &harness.state,
        onecad_app::worker::wire::body_id_wire(body),
        None,
    )
    .await
    .unwrap();
    let expected = width * 50.0 * 6.0;
    assert!(
        (properties.volume - expected).abs() < 1e-6,
        "width {width}: expected volume {expected}, got {}",
        properties.volume
    );
}

async fn dependent_profile(harness: &Harness) {
    let regions = profile::create(harness).await;
    let extrusion = api::apply_operation(
        &harness.state,
        harness.host.clone(),
        vec![EditCommand::AddOperation {
            record: OperationRecord::new(
                profile::extrude_id(),
                1,
                "Completion dependent extrusion",
                profile::extrude(&regions, 6.0),
            ),
            at_cursor: true,
        }],
        None,
    )
    .await
    .unwrap();
    harness.settled(&extrusion.projection).await;
    assert_volume(harness, 80.0).await;
    enter(harness, profile::sketch_id(), false).await;
    change_width(harness, 100.0).await;
    // The existing session authors geometry before finish refreshes its record.
    assert_volume(harness, 80.0).await;
    let keep = api::cancel_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        profile::sketch_id().to_string(),
        Some(false),
    )
    .await
    .unwrap();
    assert!(!keep.regen_expected);
    assert!(!settle(harness, keep).await.discarded);
    finish(harness, profile::sketch_id()).await;
    assert_volume(harness, 100.0).await;
    dependent_discard(harness).await;
}

async fn dependent_discard(harness: &Harness) {
    enter(harness, profile::sketch_id(), false).await;
    noop_upsert(harness, profile::sketch_id()).await;
    change_width(harness, 120.0).await;
    assert_volume(harness, 100.0).await;
    let discard = api::cancel_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        profile::sketch_id().to_string(),
        Some(true),
    )
    .await
    .unwrap();
    assert!(discard.regen_expected, "discard must rebuild the old solid");
    assert!(settle(harness, discard).await.discarded);
    assert_volume(harness, 100.0).await;
    enter(harness, profile::sketch_id(), false).await;
    let noop = api::cancel_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        profile::sketch_id().to_string(),
        Some(true),
    )
    .await
    .unwrap();
    assert!(!noop.regen_expected, "edit-free discard schedules nothing");
    assert!(settle(harness, noop).await.discarded);
}

async fn noop_upsert(harness: &Harness, id: SketchId) {
    let before = harness.projection().await;
    let noop = api::sketch_upsert_with_completion(
        &harness.state,
        harness.host.clone(),
        id.to_string(),
        Vec::new(),
    )
    .await
    .unwrap();
    assert!(!noop.regen_expected);
    assert_eq!(noop.projection, before);
    settle(harness, noop).await;
}

async fn create_empty(harness: &Harness, id: SketchId) {
    let receipt = api::apply_edit_command_with_completion(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddSketch {
            sketch: Sketch::on_world_plane(id, "Empty receipt sketch", WorldPlane::XY),
        },
    )
    .await
    .unwrap();
    assert!(!receipt.regen_expected);
    assert_eq!(receipt.result.as_ref().unwrap(), &receipt.projection);
    settle(harness, receipt).await;
    enter(harness, id, true).await;
    noop_upsert(harness, id).await;
}

async fn empty_profile(harness: &Harness) {
    let id = SketchId(Uuid::from_u128(0x275001));
    create_empty(harness, id).await;
    let discard = api::cancel_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        id.to_string(),
        Some(true),
    )
    .await
    .unwrap();
    assert!(!discard.regen_expected);
    assert!(!discard.projection.sketches.contains_key(&id.to_string()));
    assert!(settle(harness, discard).await.discarded);
    create_empty(harness, id).await;
    finish(harness, id).await;
    let noop =
        api::finish_sketch_with_completion(&harness.state, harness.host.clone(), id.to_string())
            .await
            .unwrap();
    assert!(!noop.regen_expected, "unchanged finish schedules nothing");
    assert!(settle(harness, noop).await.regions.is_empty());
}

async fn finish(harness: &Harness, id: SketchId) {
    let receipt =
        api::finish_sketch_with_completion(&harness.state, harness.host.clone(), id.to_string())
            .await
            .unwrap();
    assert!(
        receipt.regen_expected,
        "a new or changed record must rebuild"
    );
    settle(harness, receipt).await;
}

fn overlapping_sketch(id: SketchId) -> Sketch {
    let mut sketch = Sketch::on_world_plane(id, "Refused overlap", WorldPlane::XY);
    for (index, (x, y)) in [
        (0., 0.),
        (20., 0.),
        (10., 0.),
        (40., 0.),
        (40., 20.),
        (0., 20.),
    ]
    .into_iter()
    .enumerate()
    {
        sketch
            .add_entity(SketchEntity::point(
                EntityId(Uuid::from_u128(0x275100 + index as u128)),
                Vec2::new_unchecked(x, y),
                false,
                false,
            ))
            .unwrap();
    }
    for (index, (a, b)) in [(0, 1), (2, 3), (3, 4), (4, 5), (5, 0)]
        .into_iter()
        .enumerate()
    {
        sketch
            .add_entity(SketchEntity::line(
                EntityId(Uuid::from_u128(0x275110 + index as u128)),
                EntityId(Uuid::from_u128(0x275100 + a)),
                EntityId(Uuid::from_u128(0x275100 + b)),
                false,
            ))
            .unwrap();
    }
    sketch
}

async fn recorded_refusal(harness: &Harness) {
    let id = SketchId(Uuid::from_u128(0x275002));
    api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddSketch {
            sketch: overlapping_sketch(id),
        },
    )
    .await
    .unwrap();
    let receipt =
        api::finish_sketch_with_completion(&harness.state, harness.host.clone(), id.to_string())
            .await
            .unwrap();
    assert!(
        receipt.regen_expected,
        "regions refused after record commit"
    );
    assert_eq!(receipt.projection.features.len(), 1);
    let error = receipt.result.unwrap_err();
    assert_overlap_error(&error);
    harness.terminal(&receipt.projection).await;
    assert_eq!(
        harness.projection().await.features.len(),
        1,
        "the accepted record remains"
    );
    let legacy = api::finish_sketch(&harness.state, harness.host.clone(), id.to_string())
        .await
        .unwrap_err();
    assert_overlap_error(&legacy);
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        serde_json::to_value(legacy).unwrap()
    );
}

fn assert_overlap_error(error: &ApiError) {
    let ApiError::OpFailed {
        message,
        diagnostics,
    } = error
    else {
        panic!("expected regions refusal, got {error:?}");
    };
    assert!(message.contains("finishSketch: "), "{message}");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.reason_code.as_deref()
            == Some("SKETCH_PROFILE_OVERLAPPING_CURVES")));
}
