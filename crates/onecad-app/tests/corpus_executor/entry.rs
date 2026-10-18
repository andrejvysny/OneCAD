fn run_rollback_timeline(case: &CorpusCase) -> Result<(), String> {
    let expected: RollbackExpected = serde_json::from_value(case.expected.clone())
        .map_err(|e| format!("{} expected schema: {e}", case.id))?;
    let sketch = SketchId(stable_uuid("corpus-h-profile"));
    let record = |label: &str, distance: f64| {
        extrude_record_for(
            stable_uuid(label).as_u128(),
            sketch,
            distance,
            BooleanMode::NewBody,
            None,
        )
    };
    let op1 = RecordId(stable_uuid("op1"));
    let op2 = RecordId(stable_uuid("op2"));
    let op3 = RecordId(stable_uuid("op3"));
    let mut timeline = Timeline::new();
    timeline.insert_at_cursor(record("op1", 10.0));
    timeline.insert_at_cursor(record("op2", 5.0));
    if timeline.len() != expected.after_two_extrudes.op_count.unwrap_or_default()
        || timeline.cursor() != expected.after_two_extrudes.applied_op_count
    {
        return Err("afterTwoExtrudes cursor/count mismatch".into());
    }

    timeline.set_cursor(1);
    if timeline.cursor() != expected.after_rollback.applied_op_count || timeline.len() != 2 {
        return Err("rollback must preserve ops and move cursor to 1".into());
    }
    timeline.insert_at_cursor(record("op3", 2.5));
    let order = &expected.after_insert_at_cursor.order;
    if timeline.len() != expected.after_insert_at_cursor.op_count
        || timeline.cursor() != expected.after_insert_at_cursor.applied_op_count
        || timeline.index_of(op1) != Some(order.op1)
        || timeline.index_of(op2) != Some(order.op2)
        || timeline.index_of(op3) != Some(order.op3)
    {
        return Err(format!(
            "insert-at-cursor mismatch: order={:?}, cursor={}",
            timeline
                .records()
                .iter()
                .map(|record| record.record_id)
                .collect::<Vec<_>>(),
            timeline.cursor()
        ));
    }
    timeline.set_cursor(timeline.len());
    if timeline.cursor() != expected.full_regen.to_applied_count {
        return Err("full regen cursor must cover all three ops".into());
    }
    Ok(())
}

fn load_cases() -> (Vec<PathBuf>, Vec<CorpusCase>) {
    let classifications = manifest_classifications();
    let paths = case_paths();
    assert!(!paths.is_empty(), "corpus/cases must contain JSON files");
    let mut cases = Vec::new();
    let mut problems = Vec::new();
    for path in &paths {
        let raw = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let case: CorpusCase = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));
        let classification = classifications.get(&case.id).cloned().unwrap_or_default();
        problems.extend(check_case_shape(&case, &classification));
        cases.push(case);
    }
    for id in classifications.keys() {
        if !cases.iter().any(|c| &c.id == id) {
            problems.push(format!(
                "coverage manifest classifies {id}, which is not a corpus file"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "corpus structure/provenance problems:\n{}",
        problems.join("\n")
    );
    (paths, cases)
}

async fn execute_case(
    wm: &WorkerManager,
    case: &CorpusCase,
    epoch: WorkerEpoch,
) -> Result<WorkerEpoch, String> {
    let kind = CaseKind::try_from(case.id.as_str())?;
    match kind {
        CaseKind::SketchExtrudeBlind => run_sketch_extrude_blind(wm, case, epoch)
            .await
            .map(|()| epoch),
        CaseKind::ExtrudeEndConditions => run_extrude_end_conditions(wm, case, epoch).await,
        CaseKind::BooleanBodyIdentity => run_boolean_body_identity(wm, case, epoch).await,
        CaseKind::FilletSelectedEdge => run_fillet_selected_edge(wm, case, epoch)
            .await
            .map(|()| epoch),
        CaseKind::NamingAfterEdit => run_naming_after_upstream_edit(wm, epoch)
            .await
            .map(|()| epoch),
        CaseKind::SymmetricAmbiguity => run_symmetric_ambiguity(wm, epoch).await.map(|()| epoch),
        CaseKind::SketchSolver => run_sketch_solver(wm, case).await.map(|()| epoch),
        CaseKind::RollbackTimeline => run_rollback_timeline(case).map(|()| epoch),
        CaseKind::MultiRegion => run_loop_detection(wm, case).await.map(|()| epoch),
    }
}

fn assert_complete(paths: &[PathBuf], executed: &[String], failures: &[String]) {
    assert!(
        failures.is_empty(),
        "corpus executor failures:\n{}",
        failures.join("\n")
    );
    let seen: BTreeSet<&str> = executed.iter().map(String::as_str).collect();
    assert_eq!(seen.len(), paths.len(), "every case must be accounted for");
    assert_eq!(executed.len(), 9, "the frozen corpus gate is 9/9");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corpus_cases_execute_9_of_9() {
    let (paths, cases) = load_cases();
    let Some(bin) = real_worker() else {
        assert!(
            std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
            "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved"
        );
        eprintln!(
            "skip (execution half): no worker binary; {} cases shape-checked",
            cases.len()
        );
        return;
    };

    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "worker must connect"
    );

    let mut epoch = WorkerEpoch(1);
    let mut executed: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for case in &cases {
        epoch = match reset_document(&wm, &case.id, epoch).await {
            Ok(next) => next,
            Err(e) => {
                failures.push(format!("{}: {e}", case.id));
                continue;
            }
        };
        match execute_case(&wm, case, epoch).await {
            Ok(next_epoch) => {
                epoch = next_epoch;
                executed.push(case.id.clone());
            }
            Err(e) => failures.push(format!("{}: {e}", case.id)),
        }
    }

    wm.shutdown().await;
    assert_complete(&paths, &executed, &failures);
    eprintln!(
        "corpus executed: {executed:?} ({} of {}, no skips)",
        executed.len(),
        paths.len()
    );
}
