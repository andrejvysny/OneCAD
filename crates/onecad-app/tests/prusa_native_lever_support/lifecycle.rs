use super::*;

pub(super) async fn assert_full_geometry(h: &Harness) {
    let bytes = api::get_mesh(
        &h.state,
        BodyId(id(3)).to_string(),
        "fine".into(),
        None,
        None,
    )
    .await
    .unwrap();
    let mesh = MeshView::decode(&bytes, MeshLimits::default()).unwrap();
    assert!(mesh.header().triangle_count > 0);
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for tri in 0..mesh.header().triangle_count as usize {
        for vertex in mesh.triangle(tri).unwrap() {
            let p = mesh.position(vertex as usize).unwrap();
            for i in 0..3 {
                low[i] = low[i].min(f64::from(p[i]));
                high[i] = high[i].max(f64::from(p[i]));
            }
        }
    }
    eprintln!("fine mesh bounds: {low:?} to {high:?}");
    let diagnostic = source_diagnostic();
    let bounds = &diagnostic["boundsMm"];
    for i in 0..3 {
        assert!(
            (low[i] - bounds[0][i].as_f64().unwrap()).abs() <= 0.15,
            "axis {i} low {}",
            low[i]
        );
        assert!(
            (high[i] - bounds[1][i].as_f64().unwrap()).abs() <= 0.15,
            "axis {i} high {}",
            high[i]
        );
    }
}

async fn volume(h: &Harness) -> f64 {
    api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume
}

pub(super) async fn exercise_history(h: &Harness) {
    let before = volume(h).await;
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undo).await;
    assert!(
        volume(h).await > before + 1.0,
        "undo removes transverse cut"
    );
    let redo = api::redo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&redo).await;
    assert!(
        (volume(h).await - before).abs() < 1e-6,
        "redo restores original geometry"
    );
    let revision = h.projection().await.revision;
    let wrong_type = Operation::Known(KnownOperation::Revolve(RevolveParams {
        profile: None,
        angle_deg: Scalar::new(360.0),
        axis: None,
        boolean_mode: BooleanMode::Cut,
        target_body: Some(BodyId(id(3))),
        extra: Default::default(),
    }));
    let result = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::UpdateOperationParams {
            record: RecordId(id(32)),
            op: wrong_type,
        }],
        None,
    )
    .await;
    assert!(result.is_err(), "changing operation kind must be refused");
    assert_eq!(h.projection().await.revision, revision);
    assert!((volume(h).await - before).abs() < 1e-6);
}

pub(super) async fn reopen_and_edit(path: &std::path::Path) {
    let h = Harness::new(&path.parent().unwrap().join("fresh-host")).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    let worker = h.state.live_worker().unwrap();
    assert!(worker.wait_ready(std::time::Duration::from_secs(15)).await);
    let projection = h.projection().await;
    h.settled(&projection).await;
    assert!(!h.projection().await.dirty);
    assert_full_geometry(&h).await;
    let before = volume(&h).await;
    let mut record = h
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .operation_record(RecordId(id(32)))
        .unwrap();
    let Operation::Known(KnownOperation::Extrude(ref mut params)) = record.op else {
        panic!("transverse cut changed kind")
    };
    params.distance = Scalar::new(4.0);
    let result = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::UpdateOperationParams {
            record: record.record_id,
            op: record.op,
        }],
        None,
    )
    .await
    .unwrap();
    h.settled(&result.projection).await;
    assert!(h.projection().await.dirty);
    eprintln!("fresh-process edited volume: {}", volume(&h).await);
    assert!(
        volume(&h).await > before + 1.0,
        "shortened cut changes material"
    );
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undo).await;
    assert!((volume(&h).await - before).abs() < 1e-6);
    probe_zero_distance(&h, before).await;
    h.close().await;
}

async fn probe_zero_distance(h: &Harness, before: f64) {
    let mut record = h
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .operation_record(RecordId(id(32)))
        .unwrap();
    let Operation::Known(KnownOperation::Extrude(ref mut params)) = record.op else {
        panic!("transverse cut changed kind")
    };
    params.distance = Scalar::new(0.0);
    let result = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![EditCommand::UpdateOperationParams {
            record: record.record_id,
            op: record.op,
        }],
        None,
    )
    .await;
    match result {
        Err(error) => {
            eprintln!("zero-distance same-kind edit refused: {error}");
            assert!((volume(h).await - before).abs() < 1e-6);
        }
        Ok(response) => {
            let report = h.terminal(&response.projection).await;
            eprintln!(
                "zero-distance same-kind edit accepted; regen outcome={}, failed_steps={:?}",
                report.outcome, report.failed_steps
            );
            assert_eq!(report.failed_steps.len(), 1);
            assert_eq!(
                report.failed_steps[0].record_id,
                RecordId(id(32)).to_string()
            );
            assert!(
                report.failed_steps[0]
                    .message
                    .contains("Extrude distance too small")
            );
            let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
            h.settled(&undo).await;
            assert!((volume(h).await - before).abs() < 1e-6);
        }
    }
}
