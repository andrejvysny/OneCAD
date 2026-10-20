//! A native re-finish must not split the live sketch from its timeline record.
use super::*;
use onecad_core::sketch::CurvePosition;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn existing_record_refresh_undo_redo_restores_live_sketch_atomically() {
    let Some(bin) = real_worker() else {
        return;
    };
    let worker = spawn_worker(bin).await;
    let mut runtime = runtime_over(&worker);
    let sketch = dimensioned_line();
    let id = sketch.id;
    runtime.apply(EditCommand::AddSketch { sketch }).unwrap();
    runtime.enter_sketch(id).await.unwrap();
    runtime.finish_sketch(id).await.unwrap();
    let depth = runtime.undo_depth();
    assert_width(&runtime, id, 80.0);
    runtime.enter_sketch(id).await.unwrap();
    runtime
        .set_sketch_dimension_with_outcome(id, cid(0x401), Scalar::new(90.0))
        .await
        .unwrap();
    runtime.cancel_sketch(id, false).await.unwrap();
    runtime.finish_sketch(id).await.unwrap();
    assert_eq!(
        runtime.undo_depth(),
        depth + 1,
        "one finished sketch edit, including record refresh"
    );
    assert_width(&runtime, id, 90.0);
    assert!(runtime.undo().is_some());
    assert_width(&runtime, id, 80.0);
    assert_eq!(runtime.redo_depth(), 1);
    assert!(runtime.redo().unwrap().is_some());
    assert_width(&runtime, id, 90.0);
    let depth = runtime.undo_depth();
    runtime.enter_sketch(id).await.unwrap();
    runtime.finish_sketch(id).await.unwrap();
    assert_eq!(
        runtime.undo_depth(),
        depth,
        "unchanged re-finish creates no history"
    );
    worker.shutdown().await;
}

fn dimensioned_line() -> Sketch {
    let mut sketch =
        Sketch::on_world_plane(SketchId(Uuid::from_u128(0x5A_10)), "Width", WorldPlane::XY);
    for entity in [
        SketchEntity::point(eid(P0), Vec2::new(0.0, 0.0).unwrap(), false, false),
        SketchEntity::point(eid(P1), Vec2::new(80.0, 0.0).unwrap(), false, false),
        SketchEntity::line(eid(L0), eid(P0), eid(P1), false),
    ] {
        sketch.add_entity(entity).unwrap();
    }
    for constraint in [
        Constraint::Fixed {
            id: cid(0x400),
            point: eid(P0),
            point_position: CurvePosition::Arbitrary,
            at: Vec2::new(0.0, 0.0).unwrap(),
        },
        Constraint::Horizontal {
            id: cid(C_H),
            line: eid(L0),
        },
        Constraint::Distance {
            id: cid(0x401),
            entity1: eid(P0),
            entity1_position: CurvePosition::Arbitrary,
            entity2: eid(P1),
            entity2_position: CurvePosition::Arbitrary,
            value: Scalar::new(80.0),
        },
    ] {
        sketch.add_constraint(constraint).unwrap();
    }
    sketch
}

fn assert_width(runtime: &DocumentRuntime, id: SketchId, width: f64) {
    let sketch = runtime.sketch_snapshot(id, "history regression").unwrap();
    let SketchEntity::Point { at, .. } = sketch.get_entity(eid(P1)).unwrap() else {
        panic!("endpoint");
    };
    assert!(
        (at.x - width).abs() < 1e-7 && at.y.abs() < 1e-7,
        "{at:?} expected width {width}"
    );
    assert_eq!(
        sketch
            .get_constraint(cid(0x401))
            .unwrap()
            .value()
            .unwrap()
            .value,
        width
    );
    let record = runtime
        .projection()
        .features
        .iter()
        .find_map(|feature| {
            let record = runtime
                .operation_record(feature.id.parse().unwrap())
                .unwrap();
            matches!(record.op, Operation::Known(KnownOperation::Sketch(_))).then_some(record)
        })
        .unwrap();
    let Operation::Known(KnownOperation::Sketch(params)) = record.op else {
        unreachable!()
    };
    let (_, entities, constraints) = onecad_app::worker::wire::sketch_wire(&sketch);
    assert_eq!(
        Value::Array(params.entities),
        entities,
        "timeline/live geometry must match"
    );
    assert_eq!(
        Value::Array(params.constraints),
        constraints,
        "timeline/live constraints must match"
    );
}

#[allow(dead_code)]
#[path = "../native_plate_support/host.rs"]
mod host;

#[test]
fn native_cancel_keep_finish_history_matches_model_undo_redo() {
    host::required_worker();
    if std::env::var_os("ONECAD_NATIVE_PLATE_TEST_PHASE").is_none() {
        let root = tempfile::Builder::new()
            .prefix("onecad-finish-refresh-")
            .tempdir()
            .unwrap();
        host::run_isolated_phase(
            root.path(),
            "refresh",
            "refresh::native_cancel_keep_finish_history_matches_model_undo_redo",
        );
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(native_flow(&root));
}

async fn native_flow(root: &std::path::Path) {
    use onecad_app::api;
    let (harness, id) = native_setup(root).await;
    let depth = harness.projection().await.undo_depth;
    native_edit(&harness, id).await;
    assert_eq!(harness.projection().await.undo_depth, depth + 1);
    native_width(&harness, id, 90.0).await;
    native_history(&harness, id).await;
    let depth = harness.projection().await.undo_depth;
    api::enter_sketch(
        &harness.state,
        harness.host.clone(),
        id.to_string(),
        Some(false),
    )
    .await
    .unwrap();
    native_finish(&harness, id).await;
    assert_eq!(harness.projection().await.undo_depth, depth);
    harness.close().await;
}

async fn native_finish(harness: &host::Harness, id: SketchId) {
    use onecad_app::api;
    let kept = api::cancel_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        id.to_string(),
        Some(false),
    )
    .await
    .unwrap();
    if kept.regen_expected {
        harness.settled(&kept.projection).await;
    }
    let finished =
        api::finish_sketch_with_completion(&harness.state, harness.host.clone(), id.to_string())
            .await
            .unwrap();
    finished.result.unwrap();
    if finished.regen_expected {
        harness.settled(&finished.projection).await;
    }
}

async fn native_width(harness: &host::Harness, id: SketchId, width: f64) {
    use onecad_app::api;
    let sketch = api::get_sketch_snapshot(&harness.state, id).await.unwrap();
    let SketchEntity::Point { at, .. } = sketch.get_entity(eid(P1)).unwrap() else {
        panic!("endpoint");
    };
    assert!((at.x - width).abs() < 1e-7 && at.y.abs() < 1e-7);
    assert_eq!(
        sketch
            .get_constraint(cid(0x401))
            .unwrap()
            .value()
            .unwrap()
            .value,
        width
    );
    let projection = harness.projection().await;
    let feature = projection
        .features
        .iter()
        .find(|feature| feature.op_type == "Sketch")
        .unwrap();
    let record = api::get_operation_record(&harness.state, feature.id.parse().unwrap())
        .await
        .unwrap();
    let Operation::Known(KnownOperation::Sketch(params)) = record.op else {
        panic!("Sketch record");
    };
    let (_, entities, constraints) = onecad_app::worker::wire::sketch_wire(&sketch);
    assert_eq!(
        Value::Array(params.entities),
        entities,
        "native timeline/live geometry"
    );
    assert_eq!(
        Value::Array(params.constraints),
        constraints,
        "native timeline/live dimensions"
    );
}

async fn native_setup(root: &std::path::Path) -> (host::Harness, SketchId) {
    use onecad_app::api;
    let harness = host::Harness::new(&root.join("host")).await;
    api::new_document(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(
        harness
            .state
            .live_worker()
            .unwrap()
            .wait_ready(Duration::from_secs(15))
            .await
    );
    let sketch = dimensioned_line();
    let id = sketch.id;
    api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddSketch { sketch },
    )
    .await
    .unwrap();
    api::enter_sketch(
        &harness.state,
        harness.host.clone(),
        id.to_string(),
        Some(false),
    )
    .await
    .unwrap();
    native_finish(&harness, id).await;
    (harness, id)
}

async fn native_edit(harness: &host::Harness, id: SketchId) {
    use onecad_app::api;
    api::enter_sketch(
        &harness.state,
        harness.host.clone(),
        id.to_string(),
        Some(false),
    )
    .await
    .unwrap();
    let edit = api::set_sketch_dimension_with_completion(
        &harness.state,
        harness.host.clone(),
        id.to_string(),
        cid(0x401),
        Scalar::new(90.0),
    )
    .await
    .unwrap();
    assert!(edit.result.unwrap().accepted);
    if edit.regen_expected {
        harness.settled(&edit.projection).await;
    }
    native_finish(harness, id).await;
}

async fn native_history(harness: &host::Harness, id: SketchId) {
    use onecad_app::api;
    let undo = api::undo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(undo.regen_expected);
    harness.settled(&undo.projection).await;
    native_width(harness, id, 80.0).await;
    let redo = api::redo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(redo.regen_expected);
    harness.settled(&redo.projection).await;
    native_width(harness, id, 90.0).await;
}
