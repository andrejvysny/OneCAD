use super::*;
use onecad_core::document::record::{HoleParams, HoleType};

fn face_pick(key: String) -> StagePick {
    StagePick {
        body: body_a(),
        kind: ElementKind::Face,
        address: StageAddress::TopoKey(key),
    }
}

fn hole_record(face: ElementRef, point: Vec3) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(0x315F)),
        0,
        "Anchored Hole",
        Operation::Known(KnownOperation::Hole(HoleParams {
            target_body: body_a(),
            face,
            point,
            points: None,
            hole_type: HoleType::Simple,
            diameter: Scalar::new(6.0),
            depth: None,
            cb_diameter: None,
            cb_depth: None,
            cs_diameter: None,
            cs_angle_deg: None,
            thread: None,
            result_policy_version: Some(2),
            extra: Default::default(),
        })),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hole_stage_anchors_both_addresses_without_mutating_document() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = box_state(&wm).await;
    let dir = tempfile::tempdir().unwrap();
    let before = saved_elements(&state, &dir.path().join("before.onecad")).await;
    let scope = api::preview_context(&state).await.unwrap();
    let observed = observe(&state).await;
    let key = top_face_key(&body_mesh(&state).await);
    let (face, point) = api::stage_hole(&state, &scope, face_pick(key))
        .await
        .unwrap();
    assert_eq!(face.anchor.as_ref().unwrap().world_point, point);
    assert!((point.z - 25.0).abs() < 1e-8);
    let id = ref_id(&face);
    let (again, same_point) = api::stage_hole(
        &state,
        &scope,
        StagePick {
            body: body_a(),
            kind: ElementKind::Face,
            address: StageAddress::Element(id.as_str().into()),
        },
    )
    .await
    .unwrap();
    assert!(api::stage_hole(
        &state,
        &scope,
        StagePick {
            body: BodyId::new(),
            kind: ElementKind::Face,
            address: StageAddress::Element(id.as_str().into()),
        }
    )
    .await
    .is_err());
    assert_eq!(ref_id(&again), id);
    assert_eq!(same_point, point);
    let now = observe(&state).await;
    assert_eq!((now.0, now.1, now.3), (observed.0, observed.1, observed.3));
    assert_eq!(api::preview_context(&state).await.unwrap(), scope);
    let after = saved_elements(&state, &dir.path().join("after.onecad")).await;
    assert_eq!(document_json(&before), document_json(&after));
    {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .unwrap()
            .apply(EditCommand::AddOperation {
                record: hole_record(face, point),
                at_cursor: true,
            })
            .unwrap();
    }
    let adopted = saved_elements(&state, &dir.path().join("adopted.onecad")).await;
    let entry = adopted.elements.get(&id).unwrap();
    assert_eq!(entry.anchor.as_ref().unwrap().world_point, point);
    assert!(entry.descriptor.is_some());
    assert_eq!(
        published(
            &regen(state.runtime.lock().await.as_mut().unwrap()).await,
            "Hole"
        ),
        0
    );
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hole_stage_rejects_wrong_address_body_kind_and_obsolete_head() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = box_state(&wm).await;
    let blob = body_mesh(&state).await;
    let scope = api::preview_context(&state).await.unwrap();
    let key = top_face_key(&blob);
    let observed = observe(&state).await;
    let mut wrong_body = face_pick(key.clone());
    wrong_body.body = BodyId::new();
    for pick in [
        face_pick("missing-face".into()),
        wrong_body,
        edge_pick(&top_x_edges(&blob)[0].0),
    ] {
        assert!(api::stage_hole(&state, &scope, pick).await.is_err());
        assert_eq!(observe(&state).await, observed);
    }
    for stale in [
        PreviewContext {
            revision: DocumentRevision(scope.revision.0 + 1),
            ..scope.clone()
        },
        PreviewContext {
            snapshot_id: SnapshotId(scope.snapshot_id.0 + 1),
            ..scope.clone()
        },
        PreviewContext {
            worker_epoch: WorkerEpoch(scope.worker_epoch.0 + 1),
            ..scope.clone()
        },
        PreviewContext {
            runtime_session: Uuid::new_v4().to_string(),
            ..scope.clone()
        },
    ] {
        assert!(matches!(
            api::stage_hole(&state, &stale, face_pick(key.clone())).await,
            Err(ApiError::StalePreview(_))
        ));
        assert_eq!(observe(&state).await, observed);
    }
    *state.runtime.lock().await = Some(box_runtime(&state, &wm).await);
    assert!(matches!(
        api::stage_hole(&state, &scope, face_pick(key)).await,
        Err(ApiError::StalePreview(_))
    ));
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_anchored_hole_survives_plate_width_60_to_70() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = box_state(&wm).await;
    let sketch = SketchId(Uuid::from_u128(0x315));
    {
        let mut guard = state.runtime.lock().await;
        let runtime = guard.as_mut().unwrap();
        for record in [
            sketch_record(SKETCH_A, &rect_sketch(sketch, 0x3150_0000, 60.0, 40.0)),
            extrude_record(sketch, 4.0),
        ] {
            runtime
                .apply(EditCommand::UpdateOperationParams {
                    record: record.record_id,
                    op: record.op,
                })
                .unwrap();
        }
        assert_eq!(published(&regen(runtime).await, "60 x 40 plate"), 0);
    }
    let scope = api::preview_context(&state).await.unwrap();
    let key = top_face_key(&body_mesh(&state).await);
    let (face, point) = api::stage_hole(&state, &scope, face_pick(key))
        .await
        .unwrap();
    {
        let mut guard = state.runtime.lock().await;
        let runtime = guard.as_mut().unwrap();
        runtime
            .apply(EditCommand::AddOperation {
                record: hole_record(face, point),
                at_cursor: true,
            })
            .unwrap();
        assert_eq!(published(&regen(runtime).await, "fresh Hole"), 0);
        let wider = sketch_record(SKETCH_A, &rect_sketch(sketch, 0x3150_0000, 70.0, 40.0));
        runtime
            .apply(EditCommand::UpdateOperationParams {
                record: wider.record_id,
                op: wider.op,
            })
            .unwrap();
        assert_eq!(
            published(&regen(runtime).await, "60 to 70 width with Hole"),
            0
        );
        assert!(runtime
            .projection()
            .features
            .iter()
            .any(|feature| feature.label == "Anchored Hole"
                && feature.status == onecad_app::dto::FeatureStatus::Ok));
    }
    let expected = 70.0 * 40.0 * 4.0 - std::f64::consts::PI * 9.0 * 4.0;
    assert!((mesh_volume(&body_mesh(&state).await) - expected).abs() < 1.0);
    wm.shutdown().await;
}
