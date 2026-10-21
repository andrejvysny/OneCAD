use super::*;
use onecad_core::{
    document::{
        record::{OperationRecord, TransformBodyParams},
        variables::Scalar,
    },
    ids::RecordId,
};

fn copy_record() -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(0x307c00)),
        0,
        "Two copied bodies",
        Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
            targets: vec![target_body(), extra_body()],
            translate: [Scalar::new(150.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
            copy: true,
            extra: Default::default(),
        })),
    )
}

pub async fn modifier(harness: &Harness) {
    let mut record = copy_record();
    record.record_id = RecordId(Uuid::from_u128(0x307c01));
    record.name = "Modify original body".into();
    let Operation::Known(KnownOperation::TransformBody(params)) = &mut record.op else {
        unreachable!();
    };
    params.targets = vec![target_body()];
    params.translate[0] = Scalar::new(5.0);
    params.copy = false;
    record.inputs = record.op.derive_inputs();
    let id = record.record_id;
    let added = api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddOperation {
            record,
            at_cursor: true,
        },
    )
    .await
    .unwrap();
    harness.settled(&added).await;
    let deletion = plan(harness, DeleteTarget::Body(target_body())).await;
    assert_eq!(deletion.records.len(), 2, "original producer and modifier");
    assert_eq!(deletion.records[0].id, target_body().to_string());
    assert_eq!(deletion.records[1].id, id.to_string());
}

pub async fn hosted_sketch(harness: &Harness) {
    let mesh = api::get_mesh(
        &harness.state,
        target_body().to_string(),
        "coarse".into(),
        None,
        None,
    )
    .await
    .unwrap();
    let view = onecad_protocol::mesh::MeshView::decode(&mesh, Default::default()).unwrap();
    let context = api::preview_context(&harness.state).await.unwrap();
    let mut face = None;
    for index in 0..view.header().face_count as usize {
        let key = view.face_id(index).unwrap();
        let descriptor = harness
            .state
            .element_query()
            .query_element_by_topo_key(context.snapshot_id, target_body(), key)
            .await
            .unwrap()
            .unwrap();
        if descriptor.surface_type == 0 && descriptor.normal[2] > 0.99 {
            face = Some(key.to_owned());
            break;
        }
    }
    let id = SketchId(Uuid::from_u128(0x307b01));
    let prepared = api::prepare_sketch_on_face(
        &harness.state,
        api::preview_context(&harness.state).await.unwrap(),
        onecad_app::worker::wire::body_id_wire(target_body()),
        String::new(),
        face,
        id.to_string(),
        "Dependent face sketch".into(),
    )
    .await
    .unwrap();
    api::commit_sketch_on_face(&harness.state, prepared)
        .await
        .unwrap();
    let finished =
        api::finish_sketch_with_completion(&harness.state, harness.host.clone(), id.to_string())
            .await
            .unwrap();
    assert!(finished.result.is_ok());
    harness.settled(&finished.projection).await;
    delete(harness, DeleteTarget::Body(target_body()), 1, 1).await;
    assert_eq!(harness.projection().await.sketches.len(), 2);
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&undo).await;
    assert_eq!(harness.projection().await.sketches.len(), 3);
    delete(harness, DeleteTarget::Sketch(id), 0, 1).await;
    assert_state(harness, 5, 2, true).await;
}

pub async fn multioutput(harness: &Harness) {
    let id = copy_record().record_id;
    let added = api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddOperation {
            record: copy_record(),
            at_cursor: true,
        },
    )
    .await
    .unwrap();
    harness.settled(&added).await;
    let outputs = api::get_operation_record(&harness.state, id)
        .await
        .unwrap()
        .outputs;
    assert_eq!(outputs.len(), 2);
    assert_eq!(harness.projection().await.bodies.len(), 7);
    let deletion = plan(harness, DeleteTarget::Body(outputs[0])).await;
    assert_eq!(deletion.records.len(), 1);
    assert_eq!(deletion.records[0].id, id.to_string());
    assert_eq!(
        deletion.bodies.len(),
        2,
        "confirmation must show collateral output"
    );
    assert!(deletion.sketches.is_empty());
    let deleted = api::delete_document_item(&harness.state, harness.host.clone(), deletion)
        .await
        .unwrap();
    harness.settled(&deleted.projection).await;
    assert_state(harness, 5, 2, true).await;
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&undo).await;
    assert_eq!(harness.projection().await.bodies.len(), 7);
    let redo = api::redo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&redo).await;
    assert_state(harness, 5, 2, true).await;
}

pub async fn refused_transactions(harness: &Harness) {
    let proposal = plan(harness, DeleteTarget::Body(target_body())).await;
    let mut guard = harness.state.runtime.lock().await;
    let runtime = guard.as_mut().unwrap();
    let revision = runtime.revision();
    let depth = runtime.undo_depth();
    assert!(runtime
        .apply(EditCommand::DeleteBody {
            body: target_body()
        })
        .is_err());
    let mut commands: Vec<_> = proposal
        .records
        .iter()
        .rev()
        .map(|record| EditCommand::RemoveOperation {
            record: record.id.parse().unwrap(),
        })
        .collect();
    commands.push(EditCommand::DeleteBody {
        body: target_body(),
    });
    commands.push(EditCommand::DeleteSketch {
        sketch: SketchId::new(),
    });
    assert!(runtime
        .apply_transaction("Must roll back", commands)
        .is_err());
    assert_eq!(runtime.revision(), revision);
    assert_eq!(runtime.undo_depth(), depth);
    assert_eq!(runtime.projection().bodies.len(), 5);
    assert!(runtime
        .operation_record(proposal.records[0].id.parse().unwrap())
        .is_some());
    runtime.enter_sketch(fixture::id()).await.unwrap();
    assert!(runtime
        .plan_delete_document_item(DeleteTarget::Body(target_body()), proposal.expected)
        .is_err());
    runtime.cancel_sketch(fixture::id(), false).await.unwrap();
    runtime.finish_sketch(fixture::id()).await.unwrap();
}

pub async fn replacement_and_epoch(harness: &Harness, path: &Path) {
    let old = plan(harness, DeleteTarget::Body(extra_body())).await;
    api::open_document(
        &harness.state,
        harness.host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    harness.settled(&harness.projection().await).await;
    let before = harness.projection().await;
    assert!(
        api::delete_document_item(&harness.state, harness.host.clone(), old)
            .await
            .is_err()
    );
    assert_eq!(harness.projection().await.revision, before.revision);
    let old = plan(harness, DeleteTarget::Body(extra_body())).await;
    harness
        .state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .on_worker_restart(onecad_core::ids::WorkerEpoch(
            old.expected.worker_epoch.0 + 1,
        ));
    let before = harness.projection().await;
    assert!(
        api::delete_document_item(&harness.state, harness.host.clone(), old)
            .await
            .is_err()
    );
    assert_eq!(harness.projection().await.revision, before.revision);
}

pub async fn clear_all(root: &Path) {
    let harness = super::harness(&root.join("clear-host")).await;
    build(&harness).await;
    delete(&harness, DeleteTarget::Sketch(fixture::id()), 4, 1).await;
    let deletion = plan(&harness, DeleteTarget::Sketch(extra_sketch())).await;
    let deleted = api::delete_document_item(&harness.state, harness.host.clone(), deletion)
        .await
        .unwrap();
    assert!(deleted.regen_expected);
    harness.settled(&deleted.projection).await;
    assert_state(&harness, 0, 0, false).await;
    let undo = api::undo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&undo).await;
    assert_state(&harness, 1, 1, false).await;
    let redo = api::redo(&harness.state, harness.host.clone())
        .await
        .unwrap();
    harness.settled(&redo).await;
    assert_state(&harness, 0, 0, false).await;
    recreate_after_empty(&harness).await;
    save(&harness, &root.join("empty-deleted.onecad")).await;
    harness.close().await;
    stale_clear(root).await;
}

async fn recreate_after_empty(harness: &Harness) {
    let context = api::preview_context(&harness.state).await.unwrap();
    assert_eq!(context.snapshot_id.0, 0);
    assert_eq!(
        context.expected_base_hash,
        onecad_core::regen::HistoryPrefixHash::empty()
    );
    let mut sketch = fixture::profile(SketchPlane::xy());
    sketch.id = SketchId(Uuid::from_u128(0x307b02));
    let id = sketch.id;
    api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddSketch { sketch },
    )
    .await
    .unwrap();
    api::enter_sketch(&harness.state, harness.host.clone(), id.to_string(), None)
        .await
        .unwrap();
    let finished =
        api::finish_sketch_with_completion(&harness.state, harness.host.clone(), id.to_string())
            .await
            .unwrap();
    let regions = finished.result.unwrap();
    harness.settled(&finished.projection).await;
    let mut record = fixture::extrude(&regions.regions[0], regions.region_identity_version, 10);
    let Operation::Known(KnownOperation::Extrude(params)) = &mut record.op else {
        unreachable!();
    };
    params.profile.as_mut().unwrap().sketch = id;
    record.inputs = record.op.derive_inputs();
    let committed = api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddOperation {
            record,
            at_cursor: true,
        },
    )
    .await
    .unwrap();
    harness.settled(&committed).await;
    assert_eq!(harness.projection().await.bodies.len(), 1);
    let context = api::preview_context(&harness.state).await.unwrap();
    assert_ne!(context.snapshot_id.0, 0);
    let deletion = plan(harness, DeleteTarget::Sketch(id)).await;
    let deleted = api::delete_document_item(&harness.state, harness.host.clone(), deletion)
        .await
        .unwrap();
    harness.settled(&deleted.projection).await;
    assert_state(harness, 0, 0, false).await;
    api::preview_context(&harness.state).await.unwrap();
}

async fn stale_clear(root: &Path) {
    let harness = super::harness(&root.join("stale-clear-host")).await;
    build(&harness).await;
    delete(&harness, DeleteTarget::Sketch(fixture::id()), 4, 1).await;
    let proposal = plan(&harness, DeleteTarget::Sketch(extra_sketch())).await;
    let epoch = proposal.expected.worker_epoch;
    let prepared = {
        let mut guard = harness.state.runtime.lock().await;
        let runtime = guard.as_mut().unwrap();
        runtime.delete_document_item(&proposal).unwrap();
        let prepared = runtime
            .begin_regen(RegenRequest::ToEnd { from: 0 })
            .unwrap();
        runtime
            .apply(EditCommand::AddSketch {
                sketch: fixture::blank(SketchPlane::xy()),
            })
            .unwrap();
        prepared
    };
    let driven = prepared.drive(CancelToken::new()).await;
    {
        let mut guard = harness.state.runtime.lock().await;
        let runtime = guard.as_mut().unwrap();
        let report = runtime.finish_regen(driven);
        assert!(matches!(report.outcome, Outcome::Superseded));
        assert_eq!(
            runtime.fencing().get().1,
            epoch,
            "stale clear must not reset or adopt an epoch"
        );
        let report = runtime
            .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
            .await;
        assert!(matches!(report.outcome, Outcome::Published(_)));
    }
    assert!(
        api::preview_context(&harness.state)
            .await
            .unwrap()
            .worker_epoch
            .0
            > epoch.0
    );
    harness.close().await;
}
