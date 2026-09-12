use super::super::*;

const REVOLVE_RECORD: u128 = 0xF120;
const REVOLVE_PATTERN: u128 = 0xF121;
const REVOLVE_FILLET: u128 = 0xF122;

fn revolve(sketch: SketchId, axis_sketch: SketchId, angle: f64) -> OperationRecord {
    OperationRecord::new(
        rid(REVOLVE_RECORD),
        0,
        "Revolve",
        Operation::Known(KnownOperation::Revolve(RevolveParams {
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(""),
                region_identity_version: None,
                region_anchor: None,
                extra: Default::default(),
            }),
            angle_deg: Scalar::new(angle),
            axis: Some(AxisRef::SketchLine {
                sketch: axis_sketch,
                line: EntityId(Uuid::from_u128(0x30)),
                extra: Default::default(),
            }),
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            extra: Default::default(),
        })),
    )
}

fn pattern(source_record_ids: Vec<RecordId>) -> OperationRecord {
    OperationRecord::new(
        rid(REVOLVE_PATTERN),
        0,
        "FeaturePattern",
        Operation::Known(KnownOperation::FeaturePattern(FeaturePatternParams {
            source_record_ids,
            layout: FeaturePatternLayout::Linear {
                direction: Vec3::new_unchecked(1.0, 0.0, 0.0),
                spacing: Scalar::new(40.0),
            },
            count: 3,
            semantics_version: 1,
            extra: Default::default(),
        })),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sketch_line_revolve_pattern_survives_edit_and_reopen() {
    let Some(binary) = real_worker() else { return };
    let worker = spawn_worker(binary.clone()).await;
    let mut rt = runtime(&worker);
    let sketch_id = SketchId(Uuid::from_u128(SKETCH));
    let sketch = rectangle(sketch_id);
    add(&mut rt, sketch_record(&sketch));
    rt.apply(EditCommand::AddSketch { sketch }).unwrap();
    add(&mut rt, revolve(sketch_id, sketch_id, 180.0));
    let source = regen(&mut rt).await;
    assert_clean(&rt, &source, &[rid(SKETCH), rid(REVOLVE_RECORD)]);
    let source_body = body_of(REVOLVE_RECORD);
    let (edge, anchor) = promote_straight_edge(
        &mut rt,
        &worker,
        SnapshotId(source.snapshot_id),
        source_body,
    )
    .await;
    add(
        &mut rt,
        fillet_record(REVOLVE_FILLET, source_body, edge, anchor, 0.5),
    );
    let modified = regen(&mut rt).await;
    assert_clean(&rt, &modified, &[rid(REVOLVE_FILLET)]);
    add(
        &mut rt,
        pattern(vec![rid(SKETCH), rid(REVOLVE_RECORD), rid(REVOLVE_FILLET)]),
    );
    let first = regen_from(&mut rt, 3).await;
    assert_clean(&rt, &first, &[rid(REVOLVE_PATTERN)]);
    let child = BodyId(split_child_uuid(rid(REVOLVE_PATTERN).0, 0));
    let first_volume = metrics(&mut rt, child).await.0;

    rt.apply(EditCommand::UpdateOperationParams {
        record: rid(REVOLVE_RECORD),
        op: revolve(sketch_id, sketch_id, 270.0).op,
    })
    .unwrap();
    let edited = regen(&mut rt).await;
    assert_clean(
        &rt,
        &edited,
        &[
            rid(REVOLVE_RECORD),
            rid(REVOLVE_FILLET),
            rid(REVOLVE_PATTERN),
        ],
    );
    let edited_volume = metrics(&mut rt, child).await.0;
    assert!(edited_volume > first_volume * 1.4);
    assert!(rt.undo().is_some());
    let undone = regen(&mut rt).await;
    assert_clean(&rt, &undone, &[rid(REVOLVE_FILLET), rid(REVOLVE_PATTERN)]);
    assert!(rt.redo().unwrap().is_some());
    let redone = regen(&mut rt).await;
    assert_clean(&rt, &redone, &[rid(REVOLVE_FILLET), rid(REVOLVE_PATTERN)]);

    rt.apply(EditCommand::SetOperationSuppression {
        record: rid(REVOLVE_RECORD),
        suppressed: true,
        cascade: false,
    })
    .unwrap();
    published(&regen(&mut rt).await, "suppressed Revolve source");
    assert_eq!(
        row_status(&rt, rid(REVOLVE_PATTERN)),
        FeatureStatus::NeedsRepair
    );
    rt.apply(EditCommand::SetOperationSuppression {
        record: rid(REVOLVE_RECORD),
        suppressed: false,
        cascade: false,
    })
    .unwrap();
    let restored = regen(&mut rt).await;
    assert_clean(&rt, &restored, &[rid(REVOLVE_FILLET), rid(REVOLVE_PATTERN)]);
    rt.apply(EditCommand::RemoveOperation {
        record: rid(REVOLVE_RECORD),
    })
    .unwrap();
    published(&regen(&mut rt).await, "deleted Revolve source");
    assert_eq!(
        row_status(&rt, rid(REVOLVE_PATTERN)),
        FeatureStatus::NeedsRepair
    );
    assert!(rt.undo().is_some());
    let undeleted = regen(&mut rt).await;
    assert_clean(
        &rt,
        &undeleted,
        &[rid(REVOLVE_FILLET), rid(REVOLVE_PATTERN)],
    );

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("revolve-feature-pattern.onecad");
    rt.save(&path, save_meta()).unwrap();
    worker.shutdown().await;
    let reopened_worker = spawn_worker(binary).await;
    let mut reopened = open_runtime(&reopened_worker, &path);
    let report = regen(&mut reopened).await;
    assert_clean(
        &reopened,
        &report,
        &[
            rid(REVOLVE_RECORD),
            rid(REVOLVE_FILLET),
            rid(REVOLVE_PATTERN),
        ],
    );
    assert!((metrics(&mut reopened, child).await.0 - edited_volume).abs() < 1.0);
    reopened_worker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn separate_profile_and_axis_sketches_are_remapped() {
    let Some(binary) = real_worker() else { return };
    let worker = spawn_worker(binary).await;
    let mut rt = runtime(&worker);
    let profile_id = SketchId(Uuid::from_u128(SKETCH));
    let axis_id = SketchId(Uuid::from_u128(0xF123));
    let profile = rectangle(profile_id);
    add(&mut rt, sketch_record(&profile));
    rt.apply(EditCommand::AddSketch { sketch: profile })
        .unwrap();
    let axis = rectangle(axis_id);
    let mut axis_record = sketch_record(&axis);
    axis_record.record_id = rid(0xF123);
    add(&mut rt, axis_record);
    rt.apply(EditCommand::AddSketch { sketch: axis }).unwrap();
    add(&mut rt, revolve(profile_id, axis_id, -180.0));
    let source = regen(&mut rt).await;
    assert_clean(
        &rt,
        &source,
        &[rid(SKETCH), rid(0xF123), rid(REVOLVE_RECORD)],
    );
    add(
        &mut rt,
        pattern(vec![rid(SKETCH), rid(0xF123), rid(REVOLVE_RECORD)]),
    );
    let report = regen_from(&mut rt, 3).await;
    assert_clean(&rt, &report, &[rid(REVOLVE_PATTERN)]);
    let snapshot = published(&report, "two-sketch Revolve pattern");
    assert_eq!(snapshot.bodies.len(), 3);
    worker.shutdown().await;
}
