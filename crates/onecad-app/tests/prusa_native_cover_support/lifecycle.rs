use super::*;

async fn volume(h: &Harness) -> f64 {
    api::query_mass_properties(&h.state, body_id_wire(BodyId(id(3))), None)
        .await
        .unwrap()
        .volume
}

async fn mutation_state(h: &Harness) -> (serde_json::Value, usize, usize, u64) {
    let projection = h.projection().await;
    let guard = h.state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    let records: Vec<_> = [
        2, 3, 11, 12, 21, 22, 31, 32, 41, 42, 51, 52, 61, 62, 71, 72, 101, 102, 111, 112, 121, 122,
        131, 132, 201, 202, 211, 212, 221, 222, 231, 232,
    ]
    .into_iter()
    .map(|number| rt.operation_record(RecordId(id(number))).unwrap())
    .collect();
    (
        serde_json::json!({"projection": projection, "records": records}),
        rt.undo_depth(),
        rt.redo_depth(),
        rt.persisted_generation(),
    )
}

pub(super) async fn exercise_history(h: &Harness) -> f64 {
    let before = volume(h).await;
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undo).await;
    assert!(
        volume(h).await > before + 1.0,
        "undo removes final through cut"
    );
    let redo = api::redo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&redo).await;
    assert!(
        (volume(h).await - before).abs() < 1e-6,
        "redo restores source-derived geometry"
    );
    let state = mutation_state(h).await;
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
            record: RecordId(id(232)),
            op: wrong_type,
        }],
        None,
    )
    .await;
    assert!(
        result.is_err(),
        "operation-type replacement must be refused"
    );
    assert_eq!(mutation_state(h).await, state);
    assert!((volume(h).await - before).abs() < 1e-6);
    before
}

pub(super) fn run_fresh_process(path: &std::path::Path, volume: f64) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "source_derived_cover_native_history",
            "--nocapture",
        ])
        .env("ONECAD_COVER_REOPEN", path)
        .env("ONECAD_COVER_VOLUME_BITS", volume.to_bits().to_string())
        .output()
        .unwrap();
    eprintln!(
        "fresh process stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    eprintln!(
        "fresh process stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "fresh process failed: {}",
        output.status
    );
}

async fn edit_last_cut(h: &Harness, distance: f64) -> EditCommand {
    let mut record = h
        .state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .operation_record(RecordId(id(232)))
        .unwrap();
    let Operation::Known(KnownOperation::Extrude(ref mut params)) = record.op else {
        panic!("final cut changed kind")
    };
    params.distance = Scalar::new(distance);
    EditCommand::UpdateOperationParams {
        record: record.record_id,
        op: record.op,
    }
}

pub(super) async fn reopen_and_edit(path: &std::path::Path) {
    let h = Harness::new(&path.parent().unwrap().join("fresh-cover-host")).await;
    api::open_document(
        &h.state,
        h.host.clone(),
        path.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(
        h.state
            .live_worker()
            .unwrap()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let projection = h.projection().await;
    h.settled(&projection).await;
    assert!(!h.projection().await.dirty);
    validate_cover(&h).await;
    let before = volume(&h).await;
    let bits: u64 = std::env::var("ONECAD_COVER_VOLUME_BITS")
        .unwrap()
        .parse()
        .unwrap();
    assert!((before - f64::from_bits(bits)).abs() < 1e-6);
    let result = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![edit_last_cut(&h, 0.5).await],
        None,
    )
    .await
    .unwrap();
    h.settled(&result.projection).await;
    assert!(
        h.projection().await.dirty && volume(&h).await > before + 1.0,
        "shorter cut changes material"
    );
    let undo = api::undo(&h.state, h.host.clone()).await.unwrap();
    h.settled(&undo).await;
    assert!((volume(&h).await - before).abs() < 1e-6);
    probe_zero_distance(&h, before).await;
    h.close().await;
}

async fn probe_zero_distance(h: &Harness, before: f64) {
    let result = api::apply_operation(
        &h.state,
        h.host.clone(),
        vec![edit_last_cut(h, 0.0).await],
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
                "zero-distance edit accepted: outcome={}, failed_steps={:?}",
                report.outcome, report.failed_steps
            );
            assert_eq!(report.failed_steps.len(), 1);
            assert_eq!(
                report.failed_steps[0].record_id,
                RecordId(id(232)).to_string()
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
