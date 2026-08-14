fn check_lifecycle_claims(
    claims: &[LifecycleClaim],
    actual: &[BodyLifecycleEvent],
    what: &str,
) -> Result<(), String> {
    if claims.len() != actual.len() {
        return Err(format!(
            "{what}: expected {} lifecycle event(s), got {}",
            claims.len(),
            actual.len()
        ));
    }
    for (claim, event) in claims.iter().zip(actual) {
        let got = match event {
            BodyLifecycleEvent::Created { .. } => "created",
            BodyLifecycleEvent::Modified { .. } => "modified",
            BodyLifecycleEvent::Split { .. } => "split",
            BodyLifecycleEvent::Merged { .. } => "merged",
            BodyLifecycleEvent::Deleted { .. } => "deleted",
        };
        if claim.kind != got {
            return Err(format!("{what}: expected {}, got {got}", claim.kind));
        }
    }
    Ok(())
}

async fn run_feature_boolean_scenario(
    wm: &WorkerManager,
    label: &str,
    epoch: WorkerEpoch,
    job_base: u128,
    boolean_mode: BooleanMode,
    distance: f64,
    expected: &BooleanDirectionClaim,
) -> Result<(), String> {
    let base = prepare_boolean_base(wm, label, epoch, job_base).await?;
    let hosted = hosted_rectangle_sketch(
        &format!("{label}:hosted"),
        [-2.0, -2.0, 4.0, 4.0],
        base.body,
        base.face,
        base.anchor,
    );
    let mut records = base.records;
    records.push(sketch_record_for(job_base + 3, &hosted));
    records.push(extrude_record_for(
        job_base + 4,
        hosted.id,
        distance,
        boolean_mode,
        Some(base.body),
    ));
    let full = timeline(records);
    let (prepared, events) = run_plan(wm, plan_request_from(&full, job_base + 5, epoch, 2)).await?;
    if prepared.stopped_reason != StoppedReason::Completed {
        return Err(format!(
            "{label}: feature stopped: {:?}",
            prepared.stopped_reason
        ));
    }
    accept(wm, job_base + 5, epoch).await?;
    let final_events = events.values().next_back().map_or(&[][..], Vec::as_slice);
    check_lifecycle_claims(&expected.body_events, final_events, label)?;
    check_boolean_direction(wm, base.body, boolean_mode, expected, label).await
}

struct BooleanBase {
    records: Vec<OperationRecord>,
    body: BodyId,
    face: ElementId,
    anchor: Vec3,
}

async fn prepare_boolean_base(
    wm: &WorkerManager,
    label: &str,
    epoch: WorkerEpoch,
    job_base: u128,
) -> Result<BooleanBase, String> {
    let base_sketch = rectangle_sketch(&format!("{label}:base"), -10.0, -10.0, 20.0, 20.0);
    let base_record = job_base + 1;
    let body = body_of_record(RecordId(Uuid::from_u128(base_record)));
    let base_records = vec![
        sketch_record_for(job_base, &base_sketch),
        extrude_record_for(
            base_record,
            base_sketch.id,
            10.0,
            BooleanMode::NewBody,
            None,
        ),
    ];
    let base_timeline = timeline(base_records.clone());
    let (base, _) = run_plan(wm, plan_request(&base_timeline, job_base + 2, epoch)).await?;
    if base.stopped_reason != StoppedReason::Completed {
        return Err(format!("{label}: base stopped: {:?}", base.stopped_reason));
    }
    accept(wm, job_base + 2, epoch).await?;
    let mesh = mesh_for(wm, body, base.prepared_snapshot_id).await?;
    let view = validate_mesh_blob(&mesh).map_err(|e| format!("{label}: invalid MESH1: {e}"))?;
    let (face, anchor) = top_face(&view, &mesh)?;
    Ok(BooleanBase {
        records: base_records,
        body,
        face,
        anchor,
    })
}

async fn check_boolean_direction(
    wm: &WorkerManager,
    body: BodyId,
    boolean_mode: BooleanMode,
    expected: &BooleanDirectionClaim,
    label: &str,
) -> Result<(), String> {
    let volume = volume_of(wm, body).await?;
    let changed = match boolean_mode {
        BooleanMode::Add => volume > expected.base_volume_mm3 + 1.0,
        BooleanMode::Cut => volume < expected.base_volume_mm3 - 1.0,
        _ => false,
    };
    if !changed {
        return Err(format!(
            "{label}: boolean volume {volume} did not change from {} in expected direction",
            expected.base_volume_mm3
        ));
    }
    Ok(())
}

async fn run_standalone_boolean(wm: &WorkerManager, epoch: WorkerEpoch) -> Result<(), String> {
    let a = rectangle_sketch("c:standalone:a", -10.0, -10.0, 20.0, 20.0);
    let b = rectangle_sketch("c:standalone:b", 5.0, -5.0, 10.0, 10.0);
    let target_record = 0xC203;
    let tool_record = 0xC205;
    let target = body_of_record(RecordId(Uuid::from_u128(target_record)));
    let tool = body_of_record(RecordId(Uuid::from_u128(tool_record)));
    let boolean_record = OperationRecord::new(
        RecordId(Uuid::from_u128(0xC206)),
        4,
        "Boolean",
        Operation::Known(KnownOperation::Boolean(BooleanParams {
            operation: BooleanOp::Union,
            target_body: target,
            tool_body: tool,
            extra: Default::default(),
        })),
    );
    let timeline = timeline([
        sketch_record_for(0xC202, &a),
        extrude_record_for(target_record, a.id, 10.0, BooleanMode::NewBody, None),
        sketch_record_for(0xC204, &b),
        extrude_record_for(tool_record, b.id, 10.0, BooleanMode::NewBody, None),
        boolean_record,
    ]);
    let (prepared, _) = run_plan(wm, plan_request(&timeline, 0xC207, epoch)).await?;
    if prepared.stopped_reason != StoppedReason::Completed {
        return Err(format!(
            "standalone Boolean stopped: {:?}",
            prepared.stopped_reason
        ));
    }
    let outputs = prepared
        .per_step
        .last()
        .ok_or("standalone Boolean has no result row")?;
    if outputs.body_ids != [target] {
        return Err(format!(
            "standalone Boolean must preserve target BodyId {target}, got {:?}",
            outputs.body_ids
        ));
    }
    accept(wm, 0xC207, epoch).await?;
    Ok(())
}

async fn run_boolean_body_identity(
    wm: &WorkerManager,
    case: &CorpusCase,
    mut epoch: WorkerEpoch,
) -> Result<WorkerEpoch, String> {
    let expected: BooleanExpected = serde_json::from_value(case.expected.clone())
        .map_err(|e| format!("{} expected schema: {e}", case.id))?;
    run_feature_boolean_scenario(
        wm,
        "corpus-c-add",
        epoch,
        0xC100,
        BooleanMode::Add,
        4.0,
        &expected.add_fuse,
    )
    .await?;
    epoch = reset_document(wm, "corpus-c-cut", epoch).await?;
    run_feature_boolean_scenario(
        wm,
        "corpus-c-cut",
        epoch,
        0xC120,
        BooleanMode::Cut,
        -4.0,
        &expected.cut,
    )
    .await?;
    epoch = reset_document(wm, "corpus-c-standalone", epoch).await?;
    run_standalone_boolean(wm, epoch).await?;
    Ok(epoch)
}

async fn run_fillet_selected_edge(
    wm: &WorkerManager,
    case: &CorpusCase,
    epoch: WorkerEpoch,
) -> Result<(), String> {
    let expected: FilletExpected = serde_json::from_value(case.expected.clone())
        .map_err(|e| format!("{} expected schema: {e}", case.id))?;
    let sketch = rectangle_sketch("corpus-d-box", -5.0, -5.0, 10.0, 10.0);
    let body_record = 0xD011;
    let body = body_of_record(RecordId(Uuid::from_u128(body_record)));
    let base_records = vec![
        sketch_record_for(0xD010, &sketch),
        extrude_record_for(body_record, sketch.id, 10.0, BooleanMode::NewBody, None),
    ];
    let base_timeline = timeline(base_records.clone());
    let (base, _) = run_plan(wm, plan_request(&base_timeline, 0xD012, epoch)).await?;
    if base.stopped_reason != StoppedReason::Completed {
        return Err(format!("fillet base stopped: {:?}", base.stopped_reason));
    }
    accept(wm, 0xD012, epoch).await?;
    let mesh = mesh_for(wm, body, base.prepared_snapshot_id).await?;
    let view = validate_mesh_blob(&mesh).map_err(|e| format!("fillet base MESH1: {e}"))?;
    let (edge, anchor) = vertical_edge(&view, &mesh)?;

    let mut records = base_records;
    records.push(fillet_record_for(0xD013, body, edge, anchor));
    let full = timeline(records);
    let (prepared, events) = run_plan(wm, plan_request_from(&full, 0xD014, epoch, 2)).await?;
    if prepared.stopped_reason != StoppedReason::Completed {
        return Err(format!("fillet stopped: {:?}", prepared.stopped_reason));
    }
    let actual = events.values().next_back().map_or(&[][..], Vec::as_slice);
    check_lifecycle_claims(&expected.fillet_result.body_events, actual, "corpus d")?;
    accept(wm, 0xD014, epoch).await?;
    let result_mesh = mesh_for(wm, body, prepared.prepared_snapshot_id).await?;
    let result = validate_mesh_blob(&result_mesh).map_err(|e| format!("fillet MESH1: {e}"))?;
    if result.face_count < 7 {
        return Err(format!(
            "fillet selected through MESH1 did not add a rolled face: {}",
            result.face_count
        ));
    }
    Ok(())
}

async fn run_naming_after_upstream_edit(
    wm: &WorkerManager,
    epoch: WorkerEpoch,
) -> Result<(), String> {
    let (body, edge, anchor) = prepare_initial_naming_fillet(wm, epoch).await?;
    let after = rectangle_sketch("corpus-e-profile", -7.0, -5.0, 14.0, 10.0);
    let edited = timeline([
        sketch_record_for(0xE010, &after),
        extrude_record_for(0xE011, after.id, 5.0, BooleanMode::NewBody, None),
        fillet_record_for(0xE013, body, edge, anchor),
    ]);
    let mut request = plan_request(&edited, 0xE015, epoch);
    request.edited_from = Some(0);
    let (prepared, _) = run_plan(wm, request).await?;
    match prepared.stopped_reason {
        StoppedReason::Completed | StoppedReason::NeedsRepair => {}
        other => return Err(format!("upstream edit stopped unsafely: {other:?}")),
    }
    accept(wm, 0xE015, epoch).await?;
    check_edited_fillet(wm, body, &prepared).await?;
    Ok(())
}

async fn prepare_initial_naming_fillet(
    wm: &WorkerManager,
    epoch: WorkerEpoch,
) -> Result<(BodyId, ElementId, Vec3), String> {
    let before = rectangle_sketch("corpus-e-profile", -5.0, -5.0, 10.0, 10.0);
    let body_record = 0xE011;
    let body = body_of_record(RecordId(Uuid::from_u128(body_record)));
    let base_records = vec![
        sketch_record_for(0xE010, &before),
        extrude_record_for(body_record, before.id, 5.0, BooleanMode::NewBody, None),
    ];
    let base_timeline = timeline(base_records);
    let (base, _) = run_plan(wm, plan_request(&base_timeline, 0xE012, epoch)).await?;
    if base.stopped_reason != StoppedReason::Completed {
        return Err(format!("naming base stopped: {:?}", base.stopped_reason));
    }
    accept(wm, 0xE012, epoch).await?;
    let mesh = mesh_for(wm, body, base.prepared_snapshot_id).await?;
    let view = validate_mesh_blob(&mesh).map_err(|e| format!("naming base MESH1: {e}"))?;
    let (edge, anchor) = vertical_edge(&view, &mesh)?;

    let initial = timeline([
        sketch_record_for(0xE010, &before),
        extrude_record_for(body_record, before.id, 5.0, BooleanMode::NewBody, None),
        fillet_record_for(0xE013, body, edge.clone(), anchor),
    ]);
    let (filleted, _) = run_plan(wm, plan_request_from(&initial, 0xE014, epoch, 2)).await?;
    if filleted.stopped_reason != StoppedReason::Completed {
        return Err(format!(
            "naming initial fillet stopped: {:?}",
            filleted.stopped_reason
        ));
    }
    accept(wm, 0xE014, epoch).await?;
    Ok((body, edge, anchor))
}

async fn check_edited_fillet(
    wm: &WorkerManager,
    body: BodyId,
    prepared: &PlanPrepared,
) -> Result<(), String> {
    if prepared.stopped_reason == StoppedReason::Completed {
        let result_mesh = mesh_for(wm, body, prepared.prepared_snapshot_id).await?;
        let result =
            validate_mesh_blob(&result_mesh).map_err(|e| format!("edited fillet MESH1: {e}"))?;
        if result.face_count < 7 {
            return Err(format!(
                "clean topology rebind did not preserve fillet: {} faces",
                result.face_count
            ));
        }
    } else if prepared.last_valid_step != Some(1) {
        return Err(format!(
            "NeedsRepair must publish predecessor step 1, got {:?}",
            prepared.last_valid_step
        ));
    }
    Ok(())
}
