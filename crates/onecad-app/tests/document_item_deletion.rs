//! Real-worker deletion qualification, including persisted absence and one-step history.
#[path = "document_item_deletion/additional.rs"]
mod additional;
#[path = "sketch_finish_stability/fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[allow(dead_code)]
mod native_plate_support;

use native_plate_support::Harness;
use onecad_app::{
    api::{self, DeleteTarget},
    dto::SketchRegionDto,
};
use onecad_core::{
    document::record::{KnownOperation, Operation},
    edit::{EditCommand, SketchEditOp},
    ids::{BodyId, SketchId},
    regen::{CancelToken, Outcome, RegenRequest},
    sketch::{Sketch, SketchPlane},
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn extra_sketch() -> SketchId {
    SketchId(Uuid::from_u128(0x307b00))
}
fn extra_body() -> BodyId {
    BodyId(Uuid::from_u128(0x307a04))
}
fn target_body() -> BodyId {
    BodyId(Uuid::from_u128(0x307a00))
}

#[test]
fn deletion_preserves_independent_branches_history_and_fresh_reopen() {
    native_plate_support::required_worker();
    if let Ok(phase) = std::env::var("ONECAD_NATIVE_PLATE_TEST_PHASE") {
        let root = PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                match phase.as_str() {
                    "create" => create(&root).await,
                    "reopen" => reopen(&root).await,
                    _ => panic!("unknown phase {phase}"),
                }
            });
        return;
    }
    let root = tempfile::tempdir().unwrap();
    for phase in ["create", "reopen"] {
        native_plate_support::run_isolated_phase(
            root.path(),
            phase,
            "deletion_preserves_independent_branches_history_and_fresh_reopen",
        );
    }
}

async fn harness(root: &Path) -> Harness {
    let harness = Harness::new(root).await;
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
    harness
}

async fn build(harness: &Harness) {
    let mut guard = harness.state.runtime.lock().await;
    let runtime = guard.as_mut().unwrap();
    runtime
        .apply(EditCommand::AddSketch {
            sketch: fixture::blank(SketchPlane::xy()),
        })
        .unwrap();
    runtime.enter_sketch(fixture::id()).await.unwrap();
    runtime
        .sketch_upsert(
            fixture::id(),
            fixture::operations(&fixture::profile(SketchPlane::xy())),
        )
        .await
        .unwrap();
    let finished = runtime.finish_sketch(fixture::id()).await.unwrap();
    for (index, region) in finished.regions.iter().enumerate() {
        runtime
            .apply(EditCommand::AddOperation {
                record: fixture::extrude(region, finished.region_identity_version, index),
                at_cursor: true,
            })
            .unwrap();
    }
    let mut independent = fixture::profile(SketchPlane::xz());
    independent.id = extra_sketch();
    independent.name = "Independent branch".into();
    runtime
        .apply(EditCommand::AddSketch {
            sketch: independent,
        })
        .unwrap();
    let independent = runtime.finish_sketch(extra_sketch()).await.unwrap();
    let region = independent
        .regions
        .iter()
        .find(|r| r.outer_loop.len() == 5)
        .unwrap();
    let mut record = fixture::extrude(region, independent.region_identity_version, 4);
    let Operation::Known(KnownOperation::Extrude(params)) = &mut record.op else {
        unreachable!();
    };
    params.profile.as_mut().unwrap().sketch = extra_sketch();
    record.inputs = record.op.derive_inputs();
    runtime
        .apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .unwrap();
    let report = runtime
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(report.outcome, Outcome::Published(_)));
    assert!(report.failed_steps.is_empty(), "{:?}", report.failed_steps);
    assert_eq!(runtime.projection().bodies.len(), 5);
}

async fn plan(harness: &Harness, target: DeleteTarget) -> api::DeletionPlan {
    let context = api::preview_context(&harness.state).await.unwrap();
    api::plan_delete_document_item(&harness.state, target, context)
        .await
        .unwrap()
}

async fn delete(harness: &Harness, target: DeleteTarget, bodies: usize, sketches: usize) {
    let plan = plan(harness, target).await;
    assert_eq!(plan.bodies.len(), bodies);
    assert_eq!(plan.sketches.len(), sketches);
    assert!(!plan
        .bodies
        .iter()
        .any(|body| body.id == extra_body().to_string()));
    let before = harness.projection().await;
    let completion = api::delete_document_item(&harness.state, harness.host.clone(), plan)
        .await
        .unwrap();
    assert!(completion.result.is_ok() && completion.regen_expected);
    assert_eq!(completion.projection.undo_depth, before.undo_depth + 1);
    harness.settled(&completion.projection).await;
}

async fn stale_and_tampered_refuse(harness: &Harness) {
    let mut tampered = plan(harness, DeleteTarget::Body(target_body())).await;
    let before = harness.projection().await;
    tampered.records.clear();
    assert!(
        api::delete_document_item(&harness.state, harness.host.clone(), tampered)
            .await
            .is_err()
    );
    assert_eq!(harness.projection().await.revision, before.revision);
    let stale = plan(harness, DeleteTarget::Body(target_body())).await;
    api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::RenameSketch {
            sketch: extra_sketch(),
            name: "Renamed independent branch".into(),
        },
    )
    .await
    .unwrap();
    let before = harness.projection().await;
    assert!(
        api::delete_document_item(&harness.state, harness.host.clone(), stale)
            .await
            .is_err()
    );
    assert_eq!(harness.projection().await.revision, before.revision);
    let mut hash = plan(harness, DeleteTarget::Body(target_body())).await;
    hash.expected.expected_base_hash.0 = "not-the-worker-head".into();
    assert!(
        api::delete_document_item(&harness.state, harness.host.clone(), hash)
            .await
            .is_err()
    );
}

async fn assert_state(harness: &Harness, bodies: usize, sketches: usize, target_exists: bool) {
    let projection = harness.projection().await;
    assert_eq!(projection.bodies.len(), bodies);
    assert_eq!(projection.sketches.len(), sketches);
    let context = api::preview_context(&harness.state).await.unwrap();
    for body in projection.bodies.keys() {
        let bytes = api::get_mesh(
            &harness.state,
            body.clone(),
            "fine".into(),
            None,
            Some(context.runtime_session.clone()),
        )
        .await
        .unwrap();
        let mesh = onecad_protocol::mesh::validate_mesh_blob(&bytes).expect(
            "every published body must have a checked Fine mesh after deletion/history/reopen",
        );
        assert!(mesh.vertex_count > 0 && mesh.triangle_count > 0);
    }
    assert_eq!(api::preview_context(&harness.state).await.unwrap(), context);
    if bodies == 0 {
        assert!(
            api::query_mass_properties(
                &harness.state,
                onecad_app::worker::wire::body_id_wire(extra_body()),
                None
            )
            .await
            .is_err(),
            "the empty model must also clear the real worker head"
        );
        return;
    }
    assert!(projection.bodies.contains_key(&extra_body().to_string()));
    assert_eq!(
        projection.bodies.contains_key(&target_body().to_string()),
        target_exists
    );
    let topology = api::query_body_topology(
        &harness.state,
        onecad_app::worker::wire::body_id_wire(extra_body()),
    )
    .await
    .unwrap();
    assert_eq!((topology.solid_count, topology.face_count), (1, 7));
    let mass = api::query_mass_properties(
        &harness.state,
        onecad_app::worker::wire::body_id_wire(extra_body()),
        None,
    )
    .await
    .unwrap();
    assert!((mass.volume - 475.0 * 6.0).abs() < 1e-6);
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
    assert!(saved.clean);
}

async fn create(root: &Path) {
    let harness = harness(&root.join("create-host")).await;
    build(&harness).await;
    additional::modifier(&harness).await;
    additional::hosted_sketch(&harness).await;
    additional::multioutput(&harness).await;
    additional::refused_transactions(&harness).await;
    stale_and_tampered_refuse(&harness).await;
    let sketch_authority = api::get_sketch_snapshot(&harness.state, fixture::id())
        .await
        .unwrap();
    delete(&harness, DeleteTarget::Body(target_body()), 1, 0).await;
    assert_state(&harness, 4, 2, false).await;
    assert_eq!(
        api::get_sketch_snapshot(&harness.state, fixture::id())
            .await
            .unwrap(),
        sketch_authority
    );
    save(&harness, &root.join("body-deleted.onecad")).await;
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&undo).await;
    assert_state(&harness, 5, 2, true).await;
    let redo = api::redo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&redo).await;
    assert_state(&harness, 4, 2, false).await;
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&undo).await;
    delete(&harness, DeleteTarget::Sketch(fixture::id()), 4, 1).await;
    assert_state(&harness, 1, 1, false).await;
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&undo).await;
    assert_state(&harness, 5, 2, true).await;
    let redo = api::redo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&redo).await;
    assert_state(&harness, 1, 1, false).await;
    save(&harness, &root.join("sketch-deleted.onecad")).await;
    additional::replacement_and_epoch(&harness, &root.join("sketch-deleted.onecad")).await;
    harness.close().await;
    additional::clear_all(root).await;
}

async fn reopen(root: &Path) {
    for (name, bodies, sketches) in [
        ("body-deleted", 4, 2),
        ("sketch-deleted", 1, 1),
        ("empty-deleted", 0, 0),
    ] {
        let harness = Harness::new(&root.join(format!("reopen-{name}"))).await;
        api::open_document(
            &harness.state,
            harness.host.clone(),
            root.join(format!("{name}.onecad"))
                .to_string_lossy()
                .into_owned(),
            None,
        )
        .await
        .unwrap();
        harness.settled(&harness.projection().await).await;
        assert_state(&harness, bodies, sketches, false).await;
        assert!(!harness.projection().await.dirty);
        harness.close().await;
    }
}
