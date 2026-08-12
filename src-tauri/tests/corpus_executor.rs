//! P4 real-worker corpus executor.
//!
//! Enumerates every `corpus/cases/*.json`, classifies each case, and either
//! executes it against the real C++ worker or records an explicit unsupported
//! reason. No corpus file is silently skipped.
//!
//! The executor is intentionally conservative: it currently runs the simple
//! sketch → Blind NewBody golden (`a_sketch_extrude_blind`) end-to-end and
//! documents why the remaining cases need additional interpreter work.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use onecad_core::document::record::{
    ExtrudeParams, KnownOperation, Operation, OperationRecord, SketchOpParams,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::history::{DependencyGraph, Timeline};
use onecad_core::ids::{BodyId, DocumentRevision, JobId, RecordId, RegionId, SketchId};
use onecad_core::regen::{
    Fencing, GeometryEngine, PlanArtifacts, PlanContext, PlanEvent, PlanPrepared, PlanRequest,
    PolicyVersions, RegenPlanner, RegenRequest,
};

use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{resolve_worker_path, ElementQuery, WorkerManager};

/// One op in a corpus case's `opScript`. Inputs are kept as raw JSON because the
/// executor maps them onto Rust typed refs only for the ops it knows how to run.
#[derive(Debug, Deserialize)]
struct CorpusOp {
    #[serde(rename = "opType")]
    op_type: String,
    #[serde(rename = "opId", default)]
    op_id: String,
    #[serde(default)]
    inputs: Vec<serde_json::Value>,
    #[serde(default = "empty_object")]
    params: serde_json::Value,
}

fn empty_object() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Deserialize)]
struct CorpusCase {
    id: String,
    #[serde(rename = "opScript")]
    op_script: Vec<CorpusOp>,
    expected: serde_json::Value,
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(std::env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("corpus")
        .join("cases")
}

fn case_paths() -> Vec<PathBuf> {
    let dir = corpus_dir();
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("corpus dir {} unreadable: {e}", dir.display()))
        .filter_map(|e| {
            let e = e.ok()?;
            let p = e.path();
            p.extension()
                .and_then(|x| x.to_str())
                .filter(|&x| x == "json")?;
            Some(p)
        })
        .collect();
    paths.sort();
    paths
}

fn real_worker() -> Option<PathBuf> {
    resolve_worker_path()
}

fn require_worker_or_skip() -> Option<PathBuf> {
    let bin = real_worker()?;
    Some(bin)
}

/// Deterministic label → UUID mapping. Labels in the corpus ("sk_1", "body_1")
/// are illustrative; the executor assigns stable numeric seeds in encounter order.
fn label_uuid(label: &str, labels: &mut BTreeMap<String, u128>, counter: &mut u128) -> Uuid {
    let seed = *labels.entry(label.to_string()).or_insert_with(|| {
        *counter += 1;
        *counter
    });
    Uuid::from_u128(seed)
}

fn label_to_string(label: &str, labels: &mut BTreeMap<String, u128>, counter: &mut u128) -> String {
    label_uuid(label, labels, counter).to_string()
}

fn parse_sketch_profile(
    input: &serde_json::Value,
    labels: &mut BTreeMap<String, u128>,
    counter: &mut u128,
) -> Option<SketchRegionRef> {
    let primary = input.get("primary")?;
    if primary.get("kind")?.as_str()? != "face" {
        return None;
    }
    let element_id = primary.get("elementId")?.as_str()?;
    // corpus element ids look like "<sketchLabel>.region.<region>" or "<body>.face.top"
    let parts: Vec<&str> = element_id.split('.').collect();
    let sketch_label = parts.first()?;
    if sketch_label.starts_with("body_") {
        return None;
    }
    let sketch = SketchId(label_uuid(sketch_label, labels, counter));
    Some(SketchRegionRef {
        sketch,
        region: RegionId::new(""), // V1 first-region fallback
        region_identity_version: None,
        extra: Default::default(),
    })
}

fn build_record(
    step: usize,
    op: &CorpusOp,
    labels: &mut BTreeMap<String, u128>,
    counter: &mut u128,
) -> Result<OperationRecord, String> {
    let record_id = RecordId(label_uuid(&op.op_id, labels, counter));

    let mut params = op.params.clone();
    if let Some(serde_json::Value::String(s)) = params.get("sketchId") {
        params["sketchId"] = json!(label_to_string(s, labels, counter));
    }
    if let Some(serde_json::Value::String(s)) = params.get("targetBodyId") {
        if !s.is_empty() {
            params["targetBodyId"] = json!(label_to_string(s, labels, counter));
        }
    }

    let operation = match op.op_type.as_str() {
        "Sketch" => {
            let p: SketchOpParams = serde_json::from_value(params)
                .map_err(|e| format!("Sketch params for {}: {e}", op.op_id))?;
            Operation::Known(KnownOperation::Sketch(p))
        }
        "Extrude" => {
            let mut p: ExtrudeParams = serde_json::from_value(params)
                .map_err(|e| format!("Extrude params for {}: {e}", op.op_id))?;
            if let Some(input) = op.inputs.first() {
                if let Some(profile) = parse_sketch_profile(input, labels, counter) {
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

fn timeline_from_corpus(case: &CorpusCase) -> Result<Timeline, String> {
    let mut tl = Timeline::new();
    let mut labels: BTreeMap<String, u128> = BTreeMap::new();
    let mut counter: u128 = 0;
    for (step, op) in case.op_script.iter().enumerate() {
        let rec = build_record(step, op, &mut labels, &mut counter)?;
        tl.insert_at_cursor(rec);
    }
    Ok(tl)
}

fn plan_request(tl: &Timeline, job: u128) -> PlanRequest {
    let ctx = PlanContext {
        policy_versions: PolicyVersions::default(),
        occt_fingerprint: String::new(),
    };
    RegenPlanner::plan(
        tl,
        &DependencyGraph::new(),
        &[],
        RegenRequest::ToEnd { from: 0 },
        &ctx,
    )
    .into_request(
        JobId(Uuid::from_u128(job)),
        DocumentRevision(0),
        onecad_core::ids::WorkerEpoch(1),
        PolicyVersions::default(),
        PlanArtifacts { tessellate: None },
    )
}

async fn run_plan(wm: &WorkerManager, plan: PlanRequest) -> Result<PlanPrepared, String> {
    let mut rx = wm.execute_plan(plan).await;
    let mut prepared = None;
    let mut failed = None;
    while let Some(ev) = rx.recv().await {
        match ev {
            PlanEvent::Prepared(p) => prepared = Some(p),
            PlanEvent::Failed(e) => failed = Some(e),
            PlanEvent::Step(_) => {}
        }
    }
    match (prepared, failed) {
        (Some(p), _) => Ok(p),
        (None, Some(e)) => Err(format!("plan failed: {e}")),
        (None, None) => Err("plan produced no terminal event".into()),
    }
}

async fn volume_of_first_body(
    wm: &WorkerManager,
    prepared: &PlanPrepared,
) -> Result<(BodyId, f64), String> {
    let body = prepared
        .per_step
        .iter()
        .flat_map(|s| &s.body_ids)
        .copied()
        .next()
        .ok_or("no body produced")?;
    let label = format!("body_{}", body.0);
    let mass = wm
        .query_mass_properties(body, label)
        .await
        .map_err(|e| format!("mass query failed: {e}"))?;
    Ok((body, mass.volume))
}

fn assert_volume(expected: &serde_json::Value, actual: f64) -> Result<(), String> {
    let value = expected
        .get("volume_mm3")
        .and_then(|v| v.as_f64())
        .ok_or("expected.volume_mm3 missing")?;
    let tol = expected
        .get("volume_tol")
        .and_then(|v| v.as_f64())
        .unwrap_or(1e-6);
    if (actual - value).abs() > tol {
        return Err(format!("volume {actual} not within {tol} of {value}"));
    }
    Ok(())
}

async fn execute_a_sketch_extrude_blind(
    wm: &WorkerManager,
    case: &CorpusCase,
) -> Result<(), String> {
    let tl = timeline_from_corpus(case)?;
    let plan = plan_request(&tl, 0xA);
    let prepared = run_plan(wm, plan).await?;
    if prepared.stopped_reason != onecad_core::regen::StoppedReason::Completed {
        return Err(format!("plan stopped: {:?}", prepared.stopped_reason));
    }

    // Accept so the scratch snapshot becomes the worker head that mass queries read.
    wm.accept_prepared(
        JobId(Uuid::from_u128(0xA)),
        Fencing {
            document_revision: DocumentRevision(0),
            worker_epoch: onecad_core::ids::WorkerEpoch(1),
        },
    )
    .await
    .map_err(|e| format!("accept failed: {e}"))?;

    let (_, volume) = volume_of_first_body(wm, &prepared).await?;
    let last_step = case
        .expected
        .get("perStep")
        .and_then(|a| a.as_array())
        .and_then(|a| a.last());
    if let Some(step_expected) = last_step {
        assert_volume(step_expected, volume)?;
    }
    Ok(())
}

/// Explicit reasons for corpus cases the executor does not yet run.
fn unsupported_reasons() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("b_extrude_throughall_symmetric_twodir", "executor does not yet author ToNext/ToFace/Symmetric/two-direction extrudes"),
        ("c_boolean_cut_fuse_bodyid", "executor does not yet map face-hosted extrude inputs and standalone body-body booleans"),
        ("d_fillet_selected_edge", "executor does not yet pick edges and run the fillet ladder"),
        ("e_naming_break_fillet_upstream_edit", "anti-goal identity case: needs fillet + upstream-edit interpreter"),
        ("f_symmetric_ambiguity", "anti-goal identity case: needs mirror-symmetric descriptor-tie setup"),
        ("g_sketch_solver_drag_constraints", "solver case: needs sketch gesture lane integration"),
        ("h_rollback_dirty_timeline", "timeline/rollback case: needs dirty-cursor replay harness"),
        ("i_multiregion_loop_detection", "solver case: needs sketch region-count oracle integration"),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corpus_cases_are_classified_executed_or_unsupported() {
    let Some(bin) = require_worker_or_skip() else {
        if std::env::var("ONECAD_REQUIRE_WORKER").as_deref() == Ok("1") {
            panic!("ONECAD_REQUIRE_WORKER=1 but no worker binary resolved");
        }
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };

    let paths = case_paths();
    assert!(!paths.is_empty(), "corpus/cases must contain JSON files");

    let unsupported = unsupported_reasons();
    let mut unclassified: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut executed: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    // Spawn one worker for the whole enumeration.
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "worker must connect"
    );

    for path in &paths {
        let raw = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let case: CorpusCase = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));

        match case.id.as_str() {
            "a_sketch_extrude_blind" => match execute_a_sketch_extrude_blind(&wm, &case).await {
                Ok(()) => executed.push(case.id),
                Err(e) => failures.push(format!("{}: {e}", case.id)),
            },
            id => {
                if let Some(reason) = unsupported.get(id) {
                    skipped.push(format!("{id}: {reason}"));
                } else {
                    unclassified.push(id.to_string());
                }
            }
        }
    }

    wm.shutdown().await;

    if !unclassified.is_empty() {
        panic!(
            "corpus cases are unclassified (no executor path and no unsupported reason): {:?}",
            unclassified
        );
    }

    for skip in &skipped {
        eprintln!("corpus unsupported: {skip}");
    }

    if !failures.is_empty() {
        panic!("corpus executor failures:\n{}", failures.join("\n"));
    }

    // Sanity: we really did enumerate the expected set.
    let seen: HashSet<&str> = executed
        .iter()
        .map(String::as_str)
        .chain(skipped.iter().map(|s| s.split(':').next().unwrap()))
        .collect();
    assert_eq!(
        seen.len(),
        paths.len(),
        "every corpus file must be accounted for"
    );
}
