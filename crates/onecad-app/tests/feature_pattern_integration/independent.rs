use super::super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_pattern_survives_edit_history_and_reopen() {
    let Some(binary) = real_worker() else { return };
    let worker = spawn_worker(binary.clone()).await;
    let mut rt = runtime(&worker);
    let sketch_id = SketchId(Uuid::from_u128(SKETCH));
    let sketch = rectangle(sketch_id);
    add(&mut rt, sketch_record(&sketch));
    rt.apply(EditCommand::AddSketch { sketch }).unwrap();
    add(&mut rt, extrude_record(sketch_id, 10.0));
    let source = regen(&mut rt).await;
    published(&source, "source extrude");
    assert_clean(&rt, &source, &[rid(SKETCH), rid(EXTRUDE)]);
    let source_body = body_of(EXTRUDE);
    let (edge, anchor) = promote_straight_edge(
        &mut rt,
        &worker,
        SnapshotId(source.snapshot_id),
        source_body,
    )
    .await;
    add(
        &mut rt,
        fillet_record(FILLET, source_body, edge, anchor, 1.0),
    );
    let filleted = regen(&mut rt).await;
    published(&filleted, "source fillet");
    assert_clean(&rt, &filleted, &[rid(SKETCH), rid(EXTRUDE), rid(FILLET)]);
    let (edge_2, anchor_2) = promote_straight_edge(
        &mut rt,
        &worker,
        SnapshotId(filleted.snapshot_id),
        source_body,
    )
    .await;
    add(
        &mut rt,
        fillet_record(FILLET_2, source_body, edge_2, anchor_2, 0.5),
    );
    let twice_filleted = regen(&mut rt).await;
    published(&twice_filleted, "second source fillet");
    assert_clean(
        &rt,
        &twice_filleted,
        &[rid(SKETCH), rid(EXTRUDE), rid(FILLET), rid(FILLET_2)],
    );
    add(&mut rt, pattern_record());

    let first = regen_from(&mut rt, 4).await;
    let first_snapshot = published(&first, "feature pattern");
    assert_clean(
        &rt,
        &first,
        &[
            rid(SKETCH),
            rid(EXTRUDE),
            rid(FILLET),
            rid(FILLET_2),
            rid(PATTERN),
        ],
    );
    let children = [0, 1].map(|k| BodyId(split_child_uuid(rid(PATTERN).0, k)));
    let (source_volume, source_min, _, source_faces) = metrics(&mut rt, source_body).await;
    assert!(
        source_volume < 2_399.0 && source_faces >= 7,
        "source is filleted"
    );
    assert_eq!(first_snapshot.bodies.len(), 3);
    assert!(first_snapshot
        .bodies
        .iter()
        .any(|body| body.body == source_body));
    for (ordinal, child) in children.into_iter().enumerate() {
        assert!(first_snapshot.bodies.iter().any(|body| body.body == child));
        let meta = rt.body_meta(child).expect("pattern child metadata");
        let split = meta.split_of.expect("split origin");
        assert_eq!(
            (meta.created_by, split.op, split.k),
            (rid(PATTERN), rid(PATTERN), ordinal)
        );
        let (volume, min, _, faces) = metrics(&mut rt, child).await;
        assert!((volume - source_volume).abs() < 1.0 && faces == source_faces);
        assert!((min[0] - (source_min[0] + 40.0 * (ordinal + 1) as f32)).abs() < 0.1);
    }
    let (old_volume, _, _, _) = metrics(&mut rt, children[0]).await;

    rt.apply(EditCommand::UpdateOperationParams {
        record: rid(EXTRUDE),
        op: extrude_record(sketch_id, 16.0).op,
    })
    .unwrap();
    let edited = regen(&mut rt).await;
    let edited_snapshot = published(&edited, "edited source");
    assert_clean(&rt, &edited, &[rid(FILLET), rid(FILLET_2), rid(PATTERN)]);
    assert!(children
        .iter()
        .all(|child| edited_snapshot.bodies.iter().any(|b| b.body == *child)));
    let (edited_volume, _, _, _) = metrics(&mut rt, children[0]).await;
    assert!(edited_volume > old_volume * 1.4);
    assert!(rt.undo().is_some());
    let undone = regen(&mut rt).await;
    published(&undone, "undo source edit");
    assert_clean(&rt, &undone, &[rid(FILLET), rid(FILLET_2), rid(PATTERN)]);
    assert!((metrics(&mut rt, children[0]).await.0 - old_volume).abs() < 1.0);
    assert!(rt.redo().unwrap().is_some());
    let redone = regen(&mut rt).await;
    published(&redone, "redo source edit");
    assert_clean(&rt, &redone, &[rid(FILLET), rid(FILLET_2), rid(PATTERN)]);
    assert!((metrics(&mut rt, children[0]).await.0 - edited_volume).abs() < 1.0);

    rt.apply(EditCommand::SetOperationSuppression {
        record: rid(FILLET),
        suppressed: true,
        cascade: false,
    })
    .unwrap();
    published(&regen(&mut rt).await, "suppressed source prefix");
    assert_eq!(row_status(&rt, rid(PATTERN)), FeatureStatus::NeedsRepair);
    assert!(children
        .iter()
        .all(|child| !rt.head_body_ids().contains(child)));
    rt.apply(EditCommand::SetOperationSuppression {
        record: rid(FILLET),
        suppressed: false,
        cascade: false,
    })
    .unwrap();
    let restored = regen(&mut rt).await;
    published(&restored, "unsuppressed source");
    assert_clean(&rt, &restored, &[rid(FILLET), rid(FILLET_2), rid(PATTERN)]);

    rt.apply(EditCommand::RemoveOperation {
        record: rid(FILLET),
    })
    .unwrap();
    published(&regen(&mut rt).await, "deleted source prefix");
    assert_eq!(row_status(&rt, rid(PATTERN)), FeatureStatus::NeedsRepair);
    assert!(rt.undo().is_some(), "undo source deletion");
    let undeleted = regen(&mut rt).await;
    published(&undeleted, "undo source deletion");
    assert_clean(&rt, &undeleted, &[rid(FILLET), rid(FILLET_2), rid(PATTERN)]);

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("feature-pattern.onecad");
    rt.save(&path, save_meta()).expect("save pattern");
    let json = document_json(&path);
    let record = json["timeline"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["recordId"] == rid(PATTERN).to_string())
        .expect("persisted pattern");
    assert!(
        record["params"].get("sourceOps").is_none(),
        "derived sourceOps persisted"
    );
    let params = rt
        .operation_params(rid(PATTERN))
        .expect("public pattern params");
    assert!(
        params.get("sourceOps").is_none(),
        "public getter leaked worker-only sourceOps"
    );
    worker.shutdown().await;

    let reopened_worker = spawn_worker(binary).await;
    let mut reopened = open_runtime(&reopened_worker, &path);
    let reopened_report = regen(&mut reopened).await;
    let reopened_snapshot = published(&reopened_report, "fresh-worker reopen");
    assert_clean(
        &reopened,
        &reopened_report,
        &[rid(FILLET), rid(FILLET_2), rid(PATTERN)],
    );
    assert_eq!(reopened_snapshot.bodies.len(), 3);
    for child in children {
        assert!(reopened_snapshot
            .bodies
            .iter()
            .any(|body| body.body == child));
        assert!((metrics(&mut reopened, child).await.0 - edited_volume).abs() < 1.0);
    }
    reopened_worker.shutdown().await;
}
