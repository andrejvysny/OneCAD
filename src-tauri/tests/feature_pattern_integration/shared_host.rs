use super::super::*;

const HOST_SKETCH: u128 = 0xF130;
const HOST_EXTRUDE: u128 = 0xF131;
const ADD_SKETCH: u128 = 0xF132;
const ADD_EXTRUDE: u128 = 0xF133;
const ADD_PATTERN: u128 = 0xF134;
const ADD_FILLET: u128 = 0xF135;

fn named_sketch(sketch: &Sketch, record: u128) -> OperationRecord {
    let mut result = sketch_record(sketch);
    result.record_id = rid(record);
    result
}

fn named_extrude(sketch: SketchId, record: u128, distance: f64) -> OperationRecord {
    let mut result = extrude_record(sketch, distance);
    result.record_id = rid(record);
    result
}

fn add_extrude(sketch: SketchId, distance: f64) -> OperationRecord {
    let mut result = named_extrude(sketch, ADD_EXTRUDE, distance);
    let Operation::Known(KnownOperation::Extrude(params)) = &mut result.op else {
        unreachable!()
    };
    params.boolean_mode = BooleanMode::Add;
    params.target_body = Some(body_of(HOST_EXTRUDE));
    result.inputs = result.op.derive_inputs();
    result
}

fn cut_extrude(sketch: SketchId, distance: f64) -> OperationRecord {
    let mut result = add_extrude(sketch, distance);
    result.record_id = rid(0xF143);
    let Operation::Known(KnownOperation::Extrude(params)) = &mut result.op else {
        unreachable!()
    };
    params.boolean_mode = BooleanMode::Cut;
    result.inputs = result.op.derive_inputs();
    result
}

fn pattern() -> OperationRecord {
    OperationRecord::new(
        rid(ADD_PATTERN),
        0,
        "FeaturePattern",
        Operation::Known(KnownOperation::FeaturePattern(FeaturePatternParams {
            source_record_ids: vec![rid(ADD_SKETCH), rid(ADD_EXTRUDE)],
            layout: FeaturePatternLayout::Linear {
                direction: Vec3::new_unchecked(1.0, 0.0, 0.0),
                spacing: Scalar::new(2.0),
            },
            count: 3,
            semantics_version: 1,
            extra: Default::default(),
        })),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extrude_add_fillet_pattern_keeps_one_host_through_edit_and_reopen() {
    let Some(binary) = real_worker() else { return };
    let worker = spawn_worker(binary.clone()).await;
    let mut rt = runtime(&worker);
    let host_sketch_id = SketchId(Uuid::from_u128(HOST_SKETCH));
    let host_sketch = rectangle_sized(host_sketch_id, 100.0, 100.0);
    let mut host_record = named_sketch(&host_sketch, HOST_SKETCH);
    let Operation::Known(KnownOperation::Sketch(params)) = &mut host_record.op else {
        unreachable!()
    };
    params.plane.origin = Vec3::new_unchecked(20.0, -20.0, 0.0);
    add(&mut rt, host_record);
    rt.apply(EditCommand::AddSketch {
        sketch: host_sketch,
    })
    .unwrap();
    add(&mut rt, named_extrude(host_sketch_id, HOST_EXTRUDE, 5.0));
    let host_source = regen(&mut rt).await;
    assert_clean(&rt, &host_source, &[rid(HOST_EXTRUDE)]);
    let host = body_of(HOST_EXTRUDE);
    let base_volume = metrics(&mut rt, host).await.0;

    let add_sketch_id = SketchId(Uuid::from_u128(ADD_SKETCH));
    let add_sketch = asymmetric_triangle(add_sketch_id);
    add(&mut rt, named_sketch(&add_sketch, ADD_SKETCH));
    rt.apply(EditCommand::AddSketch { sketch: add_sketch })
        .unwrap();
    add(&mut rt, add_extrude(add_sketch_id, 10.0));
    let source = regen_from(&mut rt, 2).await;
    assert_clean(&rt, &source, &[rid(ADD_SKETCH), rid(ADD_EXTRUDE)]);
    let unfilleted_volume = metrics(&mut rt, host).await.0;
    published(&source, "shared Add source");
    let source_snapshot = SnapshotId(source.snapshot_id);
    let (edge, anchor) =
        promote_straight_edge_above(&mut rt, &worker, source_snapshot, host, 5.0).await;
    add(
        &mut rt,
        fillet_record(ADD_FILLET, host, edge.clone(), anchor, 0.5),
    );
    let filleted = regen_from(&mut rt, 4).await;
    assert_clean(&rt, &filleted, &[rid(ADD_FILLET)]);
    let seed_volume = metrics(&mut rt, host).await.0;
    assert!(seed_volume < unfilleted_volume);
    let mut add_pattern = pattern();
    let Operation::Known(KnownOperation::FeaturePattern(params)) = &mut add_pattern.op else {
        unreachable!()
    };
    params.source_record_ids.push(rid(ADD_FILLET));
    params.layout = FeaturePatternLayout::Linear {
        direction: Vec3::new_unchecked(-1.0, 0.0, 0.0),
        spacing: Scalar::new(25.0),
    };
    add(&mut rt, add_pattern);
    let patterned = regen_from(&mut rt, 5).await;
    assert_clean(&rt, &patterned, &[rid(ADD_PATTERN)]);
    assert_eq!(published(&patterned, "shared Add").bodies.len(), 1);
    assert_eq!(rt.head_body_ids(), vec![host]);
    let patterned_volume = metrics(&mut rt, host).await.0;
    let expected_volume = base_volume + 3.0 * (seed_volume - base_volume);
    assert!((patterned_volume - expected_volume).abs() < 1.0);

    rt.apply(EditCommand::UpdateOperationParams {
        record: rid(ADD_FILLET),
        op: fillet_record(ADD_FILLET, host, edge, anchor, 0.75).op,
    })
    .unwrap();
    let edited = regen_from(&mut rt, 4).await;
    assert_clean(&rt, &edited, &[rid(ADD_FILLET), rid(ADD_PATTERN)]);
    let edited_volume = metrics(&mut rt, host).await.0;
    assert!(edited_volume < patterned_volume);
    assert!(rt.undo().is_some());
    let undone = regen_from(&mut rt, 4).await;
    assert_clean(&rt, &undone, &[rid(ADD_PATTERN)]);
    assert!(rt.redo().unwrap().is_some());
    let redone = regen_from(&mut rt, 4).await;
    assert_clean(&rt, &redone, &[rid(ADD_PATTERN)]);

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("shared-add-fillet-feature-pattern.onecad");
    rt.save(&path, save_meta()).unwrap();
    worker.shutdown().await;
    let reopened_worker = spawn_worker(binary).await;
    let mut reopened = open_runtime(&reopened_worker, &path);
    let report = clean_regen(&mut reopened).await;
    assert_clean(&reopened, &report, &[rid(ADD_FILLET), rid(ADD_PATTERN)]);
    assert_eq!(reopened.head_body_ids(), vec![host]);
    assert!((metrics(&mut reopened, host).await.0 - edited_volume).abs() < 1.0);
    reopened_worker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extrude_add_pattern_keeps_one_host_through_source_edit() {
    let Some(binary) = real_worker() else { return };
    let worker = spawn_worker(binary).await;
    let mut rt = runtime(&worker);
    let host_sketch_id = SketchId(Uuid::from_u128(HOST_SKETCH));
    let host_sketch = rectangle(host_sketch_id);
    add(&mut rt, named_sketch(&host_sketch, HOST_SKETCH));
    rt.apply(EditCommand::AddSketch {
        sketch: host_sketch,
    })
    .unwrap();
    add(&mut rt, named_extrude(host_sketch_id, HOST_EXTRUDE, 5.0));
    let add_sketch_id = SketchId(Uuid::from_u128(ADD_SKETCH));
    let add_sketch = rectangle(add_sketch_id);
    add(&mut rt, named_sketch(&add_sketch, ADD_SKETCH));
    rt.apply(EditCommand::AddSketch { sketch: add_sketch })
        .unwrap();
    add(&mut rt, add_extrude(add_sketch_id, 10.0));
    let source = regen(&mut rt).await;
    assert_clean(&rt, &source, &[rid(ADD_SKETCH), rid(ADD_EXTRUDE)]);
    let host = body_of(HOST_EXTRUDE);
    let seed_volume = metrics(&mut rt, host).await.0;
    add(&mut rt, pattern());
    let patterned = regen_from(&mut rt, 4).await;
    assert_clean(&rt, &patterned, &[rid(ADD_PATTERN)]);
    assert_eq!(rt.head_body_ids(), vec![host]);
    let patterned_volume = metrics(&mut rt, host).await.0;
    assert!(patterned_volume > seed_volume);
    rt.apply(EditCommand::UpdateOperationParams {
        record: rid(ADD_EXTRUDE),
        op: add_extrude(add_sketch_id, 14.0).op,
    })
    .unwrap();
    let edited = regen(&mut rt).await;
    assert_clean(&rt, &edited, &[rid(ADD_EXTRUDE), rid(ADD_PATTERN)]);
    assert!(metrics(&mut rt, host).await.0 > patterned_volume);
    worker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extrude_cut_pattern_keeps_one_host_through_edit_and_reopen() {
    let Some(binary) = real_worker() else { return };
    let worker = spawn_worker(binary.clone()).await;
    let mut rt = runtime(&worker);
    let host_sketch_id = SketchId(Uuid::from_u128(0xF140));
    let host_sketch = rectangle_sized(host_sketch_id, 100.0, 80.0);
    add(&mut rt, named_sketch(&host_sketch, 0xF140));
    rt.apply(EditCommand::AddSketch {
        sketch: host_sketch,
    })
    .unwrap();
    let mut host_extrude = named_extrude(host_sketch_id, HOST_EXTRUDE, 10.0);
    host_extrude.record_id = rid(HOST_EXTRUDE);
    add(&mut rt, host_extrude);

    let cut_sketch_id = SketchId(Uuid::from_u128(0xF142));
    let cut_sketch = rectangle(cut_sketch_id);
    add(&mut rt, named_sketch(&cut_sketch, 0xF142));
    rt.apply(EditCommand::AddSketch { sketch: cut_sketch })
        .unwrap();
    add(&mut rt, cut_extrude(cut_sketch_id, 10.0));
    let source = regen(&mut rt).await;
    assert_clean(&rt, &source, &[rid(0xF142), rid(0xF143)]);
    let host = body_of(HOST_EXTRUDE);
    let seed_volume = metrics(&mut rt, host).await.0;

    let mut cut_pattern = pattern();
    cut_pattern.record_id = rid(0xF144);
    let Operation::Known(KnownOperation::FeaturePattern(params)) = &mut cut_pattern.op else {
        unreachable!()
    };
    params.source_record_ids = vec![rid(0xF142), rid(0xF143)];
    params.layout = FeaturePatternLayout::Linear {
        direction: Vec3::new_unchecked(-1.0, 0.0, 0.0),
        spacing: Scalar::new(25.0),
    };
    add(&mut rt, cut_pattern);
    let patterned = regen_from(&mut rt, 4).await;
    assert_clean(&rt, &patterned, &[rid(0xF144)]);
    assert_eq!(published(&patterned, "shared Cut").bodies.len(), 1);
    assert_eq!(rt.head_body_ids(), vec![host]);
    let patterned_volume = metrics(&mut rt, host).await.0;
    let replica_delta = seed_volume - patterned_volume;
    assert!((replica_delta - 4_800.0).abs() < 1.0, "{replica_delta}");

    rt.apply(EditCommand::UpdateOperationParams {
        record: rid(0xF143),
        op: cut_extrude(cut_sketch_id, 5.0).op,
    })
    .unwrap();
    let edited = regen(&mut rt).await;
    assert_clean(&rt, &edited, &[rid(0xF143), rid(0xF144)]);
    let edited_volume = metrics(&mut rt, host).await.0;
    assert!(edited_volume > patterned_volume);
    assert!(rt.undo().is_some());
    let undone = regen(&mut rt).await;
    assert_clean(&rt, &undone, &[rid(0xF144)]);
    assert!(rt.redo().unwrap().is_some());
    let redone = regen(&mut rt).await;
    assert_clean(&rt, &redone, &[rid(0xF144)]);

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("shared-cut-feature-pattern.onecad");
    rt.save(&path, save_meta()).unwrap();
    worker.shutdown().await;
    let reopened_worker = spawn_worker(binary).await;
    let mut reopened = open_runtime(&reopened_worker, &path);
    let report = regen(&mut reopened).await;
    assert_clean(&reopened, &report, &[rid(0xF143), rid(0xF144)]);
    assert_eq!(reopened.head_body_ids(), vec![host]);
    assert!((metrics(&mut reopened, host).await.0 - edited_volume).abs() < 1.0);
    reopened_worker.shutdown().await;
}
