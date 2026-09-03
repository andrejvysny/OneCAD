/// Deterministic label → UUID. Corpus labels ("sk_1", "op_extrude_1") are
/// illustrative; the executor assigns stable seeds in encounter order.
struct Labels {
    seeds: BTreeMap<String, u128>,
    counter: u128,
}

impl Labels {
    fn new() -> Self {
        Self {
            seeds: BTreeMap::new(),
            counter: 0,
        }
    }

    fn uuid(&mut self, label: &str) -> Uuid {
        let counter = &mut self.counter;
        let seed = *self.seeds.entry(label.to_string()).or_insert_with(|| {
            *counter += 1;
            *counter
        });
        Uuid::from_u128(seed)
    }
}

/// The body a NewBody op mints: Rust derives it from the record id, so a case's
/// body LABEL resolves through the op that produced it (`bodyLabels`).
fn body_of_record(record: RecordId) -> BodyId {
    BodyId(record.0)
}

fn parse_sketch_profile(input: &Value, labels: &mut Labels) -> Option<SketchRegionRef> {
    let primary = input.get("primary")?;
    if primary.get("kind")?.as_str()? != "face" {
        return None;
    }
    // corpus element ids look like "<sketchLabel>.region.<region>" or "<body>.face.top"
    let element_id = primary.get("elementId")?.as_str()?;
    let sketch_label = element_id.split('.').next()?;
    if sketch_label.starts_with("body_") {
        return None; // a face-hosted profile: needs a promoted face, not a sketch
    }
    Some(SketchRegionRef {
        sketch: SketchId(labels.uuid(sketch_label)),
        region: RegionId::new(""), // V1 first-region fallback
        region_identity_version: None,
        region_anchor: None,
        extra: Default::default(),
    })
}

fn build_record(
    step: usize,
    op: &CorpusOp,
    case: &CorpusCase,
    labels: &mut Labels,
) -> Result<OperationRecord, String> {
    let record_id = RecordId(labels.uuid(&op.op_id));
    let mut params = op.params.clone();
    if let Some(Value::String(s)) = params.get("sketchId") {
        params["sketchId"] = json!(labels.uuid(s).to_string());
    }
    // A body LABEL becomes the id of the body its producing op mints. Without the
    // `bodyLabels` mapping the label is unresolvable, and quietly minting a fresh
    // uuid would make a Cut target a body that does not exist — which the worker
    // reports as REF_UNRESOLVED, i.e. a red test for the wrong reason.
    if let Some(Value::String(label)) = params.get("targetBodyId") {
        if !label.is_empty() {
            let producer = case
                .body_labels
                .get(label.as_str())
                .ok_or_else(|| format!("{}: body label {label} has no producing op", case.id))?;
            let body = body_of_record(RecordId(labels.uuid(producer)));
            params["targetBodyId"] = json!(body.0.to_string());
        }
    }
    // Empty end-condition face ids are documentation, not payload: forwarding them
    // would put an empty string where the worker's strict reader expects a ref.
    for key in ["targetFaceId", "targetFaceId2"] {
        if params.get(key).and_then(Value::as_str) == Some("") {
            params.as_object_mut().and_then(|m| m.remove(key));
        }
    }

    let operation = match op.op_type.as_str() {
        "Sketch" => {
            let p: SketchOpParams = serde_json::from_value(params)
                .map_err(|e| format!("Sketch params for {}: {e}", op.op_id))?;
            Operation::Known(KnownOperation::Sketch(p))
        }
        "Extrude" => {
            // A corpus op states only the fields its scenario is about; the rest are
            // the schema's defaults. Filling them here keeps the CASE readable and
            // stops a missing `extrudeMode2` from reading as a malformed case.
            if let Some(map) = params.as_object_mut() {
                for (key, default) in [
                    ("draftAngleDeg", json!(0.0)),
                    ("twoDirections", json!(false)),
                    ("extrudeMode2", json!("Blind")),
                    ("distance2", json!(0.0)),
                ] {
                    map.entry(key.to_string()).or_insert(default);
                }
            }
            let mut p: ExtrudeParams = serde_json::from_value(params)
                .map_err(|e| format!("Extrude params for {}: {e}", op.op_id))?;
            if let Some(input) = op.inputs.first() {
                if let Some(profile) = parse_sketch_profile(input, labels) {
                    p.profile = Some(profile);
                }
            }
            Operation::Known(KnownOperation::Extrude(p))
        }
        other => return Err(format!("unsupported opType in executor: {other}")),
    };

    Ok(OperationRecord::new(
        record_id,
        step as u32,
        &op.op_type,
        operation,
    ))
}

/// Ops of one scenario, in script order. `None` selects a case with no scenarios.
fn scenario_ops<'a>(case: &'a CorpusCase, scenario: Option<&str>) -> Vec<&'a CorpusOp> {
    case.op_script
        .iter()
        .filter(|op| match scenario {
            Some(name) => op.scenario.as_deref() == Some(name),
            None => true,
        })
        .collect()
}

fn timeline_of(case: &CorpusCase, scenario: Option<&str>) -> Result<(Timeline, Labels), String> {
    let mut tl = Timeline::new();
    let mut labels = Labels::new();
    for (step, op) in scenario_ops(case, scenario).into_iter().enumerate() {
        tl.insert_at_cursor(build_record(step, op, case, &mut labels)?);
    }
    Ok((tl, labels))
}

fn plan_request(tl: &Timeline, job: u128, epoch: WorkerEpoch) -> PlanRequest {
    plan_request_from(tl, job, epoch, 0)
}

fn plan_request_from(tl: &Timeline, job: u128, epoch: WorkerEpoch, from: usize) -> PlanRequest {
    let ctx = PlanContext {
        policy_versions: PolicyVersions::default(),
        occt_fingerprint: String::new(),
    };
    RegenPlanner::plan(
        tl,
        &DependencyGraph::new(),
        &[],
        RegenRequest::ToEnd { from },
        &ctx,
    )
    .into_request(
        JobId(Uuid::from_u128(job)),
        DocumentRevision(0),
        epoch,
        PolicyVersions::default(),
        PlanArtifacts { tessellate: None },
    )
}

/// Runs a plan and returns the prepare plus the per-step body events, which are
/// the only channel the lifecycle claims (`created` / `modified`) can be read from.
async fn run_plan(
    wm: &WorkerManager,
    plan: PlanRequest,
) -> Result<(PlanPrepared, BTreeMap<usize, Vec<BodyLifecycleEvent>>), String> {
    let mut rx = wm.execute_plan(plan).await;
    let mut prepared = None;
    let mut failed = None;
    let mut events: BTreeMap<usize, Vec<BodyLifecycleEvent>> = BTreeMap::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            PlanEvent::Prepared(p) => prepared = Some(p),
            PlanEvent::Failed(e) => failed = Some(e),
            PlanEvent::Step(step) => {
                events.insert(step.step_index, step.body_events);
            }
        }
    }
    match (prepared, failed) {
        (Some(p), _) => Ok((p, events)),
        (None, Some(e)) => Err(format!("plan failed: {e}")),
        (None, None) => Err("plan produced no terminal event".into()),
    }
}

async fn accept(wm: &WorkerManager, job: u128, epoch: WorkerEpoch) -> Result<(), String> {
    wm.accept_prepared(
        JobId(Uuid::from_u128(job)),
        Fencing {
            document_revision: DocumentRevision(0),
            worker_epoch: epoch,
        },
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("accept failed: {e}"))
}

/// A fresh worker DOCUMENT for the next scenario. Cases and scenarios must not see
/// each other's bodies — a leftover body makes the next solid count a lie — and
/// `ResetSession` is how the protocol says to get one (SCHEMA §7.1). It BUMPS the
/// worker epoch, and every subsequent plan must be fenced with the new value, so
/// the epoch is threaded rather than assumed.
async fn reset_document(
    wm: &WorkerManager,
    label: &str,
    epoch: WorkerEpoch,
) -> Result<WorkerEpoch, String> {
    let document = DocumentId(stable_uuid(label));
    // ResetSession drops the session entirely (SCHEMA §7.1) — it bumps the epoch
    // and leaves NO open session, so the document has to be opened again before a
    // plan can run. Skipping that is an "ExecutePlan: no open session" that reads
    // like a harness bug rather than a missing verb.
    let next = wm
        .reset(document, epoch)
        .await
        .map_err(|e| format!("reset for {label} failed: {e}"))?;
    wm.open_session(OpenSessionRequest {
        document_id: document,
        document_revision: DocumentRevision(0),
        worker_epoch: next,
        mode: SessionMode::Determinism,
    })
    .await
    .map_err(|e| format!("open session for {label} failed: {e}"))?;
    Ok(next)
}
