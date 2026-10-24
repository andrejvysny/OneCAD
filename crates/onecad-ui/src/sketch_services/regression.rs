//! Real-worker authoring and repair evidence; no GPU or appearance qualification.
use super::*;
use onecad_app::dto::{DocumentProjection, FeatureStatus};
use onecad_core::{document::variables::Scalar, ids::ConstraintId};
use onecad_interaction::sketch::{SketchInteraction, SketchPhase, SketchStatus};

use super::test_support as support;

const TEST: &str = "sketch_services::regression::upstream_width_edit_retains_authoring_and_closes_with_named_repair";
const SKETCH: &str = "5ced698c-2b07-48cc-b602-a06aac0463d2";
const WIDTH: &str = "9532dee4-5ed8-49f9-88e3-5b1d434c4136";

#[test]
fn upstream_width_edit_retains_authoring_and_closes_with_named_repair() {
    support::required_worker();
    if std::env::var_os("ONECAD_NATIVE_PLATE_TEST_PHASE").is_some() {
        let root =
            std::path::PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run(&root));
    } else {
        let root = tempfile::tempdir().unwrap();
        support::run_isolated_phase(root.path(), "qa-repair", TEST);
    }
}

async fn run(root: &std::path::Path) {
    let harness = support::Harness::new(&root.join("qa-repair-host")).await;
    let path = root.join("plate.onecad");
    std::fs::write(
        &path,
        include_bytes!("../../../onecad-app/tests/fixtures/qa_plate_hole.onecad"),
    )
    .unwrap();
    api::open_document(
        &harness.state,
        harness.host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let baseline = harness.settled(&harness.projection().await).await;
    let baseline_volume = volume(&harness).await;
    let mut interaction = enter(&harness).await;
    let receipt = api::set_sketch_dimension_controlled(
        &harness.state,
        harness.host.clone(),
        SKETCH.into(),
        WIDTH.parse().unwrap(),
        Scalar::new(70.0),
        Default::default(),
    )
    .await
    .unwrap();
    if receipt.regen_expected {
        harness.settled(&receipt.projection).await;
    }
    assert!(receipt.result.unwrap().accepted);
    assert_width(&harness, 70.0).await;
    finish_with_repair(&harness, &mut interaction).await;
    assert_eq!(
        interaction.session.as_ref().unwrap().phase,
        SketchPhase::Closed
    );
    assert_eq!(
        interaction.text_undo_route(false),
        onecad_interaction::UndoRoute::Model
    );
    assert!(
        interaction.dispatch(SketchAction::Discard).is_empty(),
        "closed backend offers no Discard"
    );
    let undo = api::undo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(undo.regen_expected);
    let restored = harness.settled(&undo.projection).await;
    assert_eq!(restored.features.len(), baseline.features.len());
    assert_width(&harness, 60.0).await;
    assert!(
        (volume(&harness).await - baseline_volume).abs() < 1e-6,
        "Undo restores plate, hole and boss solid"
    );
    harness.close().await;
}

async fn enter(harness: &support::Harness) -> SketchInteraction {
    let id = SKETCH.parse().unwrap();
    let sketch = api::get_sketch_snapshot(&harness.state, id).await.unwrap();
    let captured = api::preview_context(&harness.state).await.unwrap();
    let context = DocumentContext {
        document_id: captured.document_id,
        runtime_session: captured.runtime_session,
        revision: captured.revision.0,
        snapshot_id: captured.snapshot_id,
        scene: onecad_scene::SceneIdentity {
            session: 1,
            scene: 1,
        },
    };
    let mut interaction = SketchInteraction::default();
    interaction.dispatch(SketchAction::ContextChanged(Some(context.clone())));
    let effects = interaction.dispatch(SketchAction::Begin {
        sketch: Box::new(sketch.clone()),
        create: false,
        context: context.clone(),
    });
    let SketchEffect::EnterSketch { tag, .. } = &effects[0] else {
        panic!("expected entry")
    };
    let receipt = api::enter_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        SKETCH.into(),
        Some(false),
    )
    .await
    .unwrap();
    assert!(!receipt.regen_expected);
    interaction.dispatch(SketchAction::Entered {
        tag: tag.clone(),
        solve: Box::new(receipt.result.unwrap()),
        snapshot: SketchSnapshot {
            sketch: Box::new(sketch),
            document: context,
            history: None,
        },
    });
    interaction
}

async fn finish_with_repair(harness: &support::Harness, interaction: &mut SketchInteraction) {
    let effects = interaction.dispatch(SketchAction::Finish);
    let SketchEffect::FinishSketch { tag, .. } = &effects[0] else {
        panic!("expected Finish")
    };
    let keep = api::cancel_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        SKETCH.into(),
        Some(false),
    )
    .await
    .unwrap();
    assert!(!keep.regen_expected);
    assert!(!keep.result.unwrap().discarded);
    let receipt =
        api::finish_sketch_with_completion(&harness.state, harness.host.clone(), SKETCH.into())
            .await
            .unwrap();
    assert!(
        receipt.result.is_ok(),
        "authoring accepted before downstream geometry"
    );
    assert!(receipt.regen_expected);
    let completion = harness.terminal(&receipt.projection).await;
    assert_eq!(completion.outcome, "published");
    assert!(
        completion.failed_steps.is_empty(),
        "repair is distinct from kernel failure"
    );
    let projection = harness.projection().await;
    let hole = projection
        .features
        .iter()
        .find(|feature| feature.op_type == "Hole")
        .unwrap();
    assert_eq!(hole.status, FeatureStatus::NeedsRepair);
    assert_eq!(completion.repair_steps, vec![hole.id.clone()]);
    repair_evidence(harness).await;
    let completed = crate::sketch_completion::finish_authoring(receipt, async { Ok(completion) })
        .await
        .unwrap();
    let Reply::Finished(result, warning) = finish_reply(completed, &projection).unwrap() else {
        panic!("accepted repair must close")
    };
    let warning = warning.expect("repair cannot be healthy success");
    assert!(
        warning.contains("Hole")
            && warning.contains("changes retained")
            && warning.contains("Undo available"),
        "{warning}"
    );
    interaction.dispatch(SketchAction::Finished {
        tag: tag.clone(),
        result,
        warning: Some(warning),
    });
    let SketchStatus::Finished {
        kept_reason: Some(reason),
        result,
    } = &interaction.session.as_ref().unwrap().status
    else {
        panic!("closed warning required")
    };
    assert_eq!(
        crate::sketch_feedback::finished(Some(reason), result).1,
        crate::diagnostics::State::Warning
    );
}

async fn repair_evidence(harness: &support::Harness) {
    let guard = harness.state.runtime.lock().await;
    let items = guard.as_ref().unwrap().repair_items();
    eprintln!(
        "QA-01 exact RepairItem: {}",
        serde_json::to_string(items).unwrap()
    );
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].step_index, 2);
    assert_eq!(
        items[0].reason,
        onecad_core::document::repair::RepairReason::LowConfidence
    );
    assert_eq!(
        items[0].ladder_failed,
        onecad_core::document::repair::LadderLevel::Descriptor
    );
    assert!(!items[0].candidates.is_empty());
}

async fn assert_width(harness: &support::Harness, expected: f64) {
    let sketch = api::get_sketch_snapshot(&harness.state, SKETCH.parse().unwrap())
        .await
        .unwrap();
    let constraint = WIDTH.parse::<ConstraintId>().unwrap();
    assert_eq!(
        sketch
            .constraints()
            .iter()
            .find(|item| item.id() == constraint)
            .unwrap()
            .value()
            .unwrap()
            .value,
        expected
    );
}

async fn volume(harness: &support::Harness) -> f64 {
    let projection: DocumentProjection = harness.projection().await;
    assert_eq!(projection.bodies.len(), 1);
    let body = projection.bodies.keys().next().unwrap().parse().unwrap();
    api::query_mass_properties(
        &harness.state,
        onecad_app::worker::wire::body_id_wire(body),
        None,
    )
    .await
    .unwrap()
    .volume
}
