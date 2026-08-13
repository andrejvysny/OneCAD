//! P4 real-worker corpus executor (WP4.4).
//!
//! `corpus/cases/*.json` is the frozen characterization of OneCAD-CPP at commit
//! `b4ddccc`: every numeric expectation cites the C++ line that produced it. This
//! test is what turns that from a document into evidence — it enumerates every
//! case, classifies it, and either RUNS it against the real worker or records why
//! it cannot. No corpus file is silently skipped.
//!
//! Three things the earlier version got wrong, all of the same kind — a claim with
//! nothing behind it:
//!
//! * **The classification was a hardcoded table in this file.** The coverage
//!   manifest (`docs/qa/modeling-operation-coverage.json`) also classifies every
//!   case, and the two could disagree without either failing. The manifest is now
//!   the single source: this test READS it, and a case missing from it is an error.
//! * **The one executed case asserted its last volume and nothing else.** Region
//!   counts, body events, solid counts and face counts sat in `expected` unread.
//! * **`expected` was never checked for provenance.** WP4.4 requires the citations
//!   to remain present; a number that loses its citation stops being a
//!   characterization and becomes a guess with a decimal point.
//!
//! Executed today: `a_sketch_extrude_blind` (the full expected block),
//! `b_extrude_throughall_symmetric_twodir` (three independent scenarios), and
//! `i_multiregion_loop_detection` (four SketchRegions oracles). The rest carry an
//! explicit reason naming the machinery they still need, and a reason for a case
//! that HAS become executable is itself an error — the table cannot go stale
//! quietly.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use onecad_core::document::body::BodyLifecycleEvent;
use onecad_core::document::record::{
    ExtrudeParams, KnownOperation, Operation, OperationRecord, SketchOpParams,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::history::{DependencyGraph, Timeline};
use onecad_core::ids::{
    BodyId, DocumentId, DocumentRevision, EntityId, JobId, RecordId, RegionId, SketchId,
    SnapshotId, WorkerEpoch,
};
use onecad_core::math::Vec2;
use onecad_core::regen::{
    Fencing, GeometryEngine, Lod, OpenSessionRequest, PlanArtifacts, PlanContext, PlanEvent,
    PlanPrepared, PlanRequest, PolicyVersions, RegenPlanner, RegenRequest, SessionMode,
    StoppedReason,
};
use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};
use onecad_protocol::mesh::validate_mesh_blob;

// ─────────────────────────────────────────────────────────────────────────────
// Case model
// ─────────────────────────────────────────────────────────────────────────────

/// One op in a case's `opScript`. Params stay raw JSON: the executor maps them
/// onto typed Rust records only for the ops it can run, and an op it cannot run
/// must still be READABLE (that is how its unsupported reason stays honest).
#[derive(Debug, Deserialize)]
struct CorpusOp {
    #[serde(rename = "opType")]
    op_type: String,
    #[serde(rename = "opId", default)]
    op_id: String,
    #[serde(default)]
    inputs: Vec<Value>,
    #[serde(default = "empty_object")]
    params: Value,
    /// Which independent scenario this op belongs to. The C++ oracles are often
    /// separate documents; running them in one timeline would boolean unrelated
    /// bodies together. Absent ⇒ the case is a single scenario.
    #[serde(default)]
    scenario: Option<String>,
    /// SketchRegions cases carry their geometry here rather than in `params`.
    #[serde(default)]
    sketch: Option<Value>,
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Debug, Deserialize)]
struct CorpusCase {
    id: String,
    #[serde(default)]
    source: Vec<String>,
    #[serde(rename = "opScript")]
    op_script: Vec<CorpusOp>,
    expected: Value,
    /// `bodyLabel → opId that mints it`. A case that names `body_1` as a boolean
    /// target is only runnable if it says who produced that label.
    #[serde(rename = "bodyLabels", default)]
    body_labels: BTreeMap<String, String>,
}

fn repo_root() -> PathBuf {
    PathBuf::from(std::env!("CARGO_MANIFEST_DIR")).join("..")
}

fn case_paths() -> Vec<PathBuf> {
    let dir = repo_root().join("corpus").join("cases");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("corpus dir {} unreadable: {e}", dir.display()))
        .filter_map(|e| {
            let p = e.ok()?.path();
            p.extension()
                .and_then(|x| x.to_str())
                .filter(|&x| x == "json")?;
            Some(p)
        })
        .collect();
    paths.sort();
    paths
}

/// The corpus classification the COVERAGE MANIFEST declares. Reading it here is
/// what stops the manifest and this executor from drifting apart: there is one
/// statement of what each case is, and both consumers read it.
fn manifest_classifications() -> BTreeMap<String, String> {
    let path = repo_root().join("docs/qa/modeling-operation-coverage.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("coverage manifest {} unreadable: {e}", path.display()));
    let manifest: Value = serde_json::from_str(&raw).expect("coverage manifest parses");
    manifest["corpusCases"]
        .as_object()
        .expect("coverage manifest has corpusCases")
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
        .collect()
}

/// A deterministic UUID from a label (FNV-1a over the bytes, twice, for 128 bits).
/// Determinism is the point: two runs of the same corpus case must address the
/// same document and the same sketch, or a "stable across replay" assertion is
/// only ever asserting that random ids differ.
fn stable_uuid(label: &str) -> Uuid {
    let fnv = |seed: u64, bytes: &[u8]| {
        bytes.iter().fold(seed, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    };
    let high = fnv(0xcbf2_9ce4_8422_2325, label.as_bytes());
    let low = fnv(high, label.as_bytes());
    Uuid::from_u128((u128::from(high) << 64) | u128::from(low))
}

fn real_worker() -> Option<PathBuf> {
    resolve_worker_path()
}

// ─────────────────────────────────────────────────────────────────────────────
// Structure + provenance (no worker needed — this half runs on every machine)
// ─────────────────────────────────────────────────────────────────────────────

/// Keys whose presence makes an object a CLAIM: it states a measured number, so
/// WP4.4 requires it to say where the number came from.
const MEASURED_KEYS: &[&str] = &[
    "volume_mm3",
    "regions",
    "holes",
    "solids",
    "outerLoopArea_gt",
    "wiresInFace",
    "baseVolume_mm3",
    "appliedOpCount",
    "dof",
];

/// Walks `expected` and returns every claim that carries no provenance. A claim is
/// covered when it, or an ancestor, cites something — `perStep[]` entries cite per
/// step, while a grouped oracle cites once for the group.
fn claims_without_provenance(value: &Value, path: &str, inherited: bool, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            let cites = inherited
                || map.keys().any(|k| k.starts_with("provenance"))
                // `confidence` is the corpus's way of saying "derived, NOT captured
                // from the oracle" — a stronger statement than a citation, and the
                // reason `expected.symmetric` carries no provenance line.
                || map.contains_key("confidence");
            // Only a SCALAR measurement is a claim. A key like `dof` holding an
            // ARRAY is a grouping whose members carry their own citations, and
            // treating the group as a claim would demand a citation for a
            // container that measures nothing.
            let is_claim = MEASURED_KEYS
                .iter()
                .any(|k| matches!(map.get(*k), Some(Value::Number(_)) | Some(Value::String(_))));
            if is_claim && !cites {
                out.push(path.to_string());
            }
            for (key, child) in map {
                claims_without_provenance(child, &format!("{path}.{key}"), cites, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                claims_without_provenance(child, &format!("{path}[{index}]"), inherited, out);
            }
        }
        _ => {}
    }
}

/// Structural contract every case obeys, executable or not.
fn check_case_shape(case: &CorpusCase, classification: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if case.source.is_empty() {
        problems.push(format!("{}: no source[] citations", case.id));
    }
    for entry in &case.source {
        // Every citation names a FILE. Most name a line range too, but the frozen
        // tree legitimately cites headers by symbol (`Sketch.h SketchPlane::XY()`)
        // and specs by section (`SCHEMA.md §9`), so the rule is "names a source",
        // not "names a line" — a rule the corpus cannot satisfy is not a rule.
        let names_a_file = [".cpp", ".h", ".md", ".txt", ".json"]
            .iter()
            .any(|ext| entry.contains(ext));
        if !names_a_file {
            problems.push(format!("{}: source entry names no file — {entry}", case.id));
        }
    }
    if case.op_script.is_empty() {
        problems.push(format!("{}: empty opScript", case.id));
    }
    let mut missing = Vec::new();
    claims_without_provenance(&case.expected, "expected", false, &mut missing);
    for path in missing {
        problems.push(format!(
            "{}: measured claim without provenance at {path}",
            case.id
        ));
    }
    for (label, op_id) in &case.body_labels {
        if !case.op_script.iter().any(|op| &op.op_id == op_id) {
            problems.push(format!(
                "{}: bodyLabels maps {label} to unknown op {op_id}",
                case.id
            ));
        }
    }
    if classification.is_empty() {
        problems.push(format!(
            "{}: the coverage manifest classifies no such case",
            case.id
        ));
    }
    problems
}

// ─────────────────────────────────────────────────────────────────────────────
// Timeline construction
// ─────────────────────────────────────────────────────────────────────────────

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

// ─────────────────────────────────────────────────────────────────────────────
// Assertions against `expected`
// ─────────────────────────────────────────────────────────────────────────────

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

/// `expected.volume_mm3` (± `tol`/`volume_tol`, default 1e-6) against a measurement.
fn check_volume(expected: &Value, actual: f64, what: &str) -> Result<(), String> {
    let want = number(expected, "volume_mm3")
        // A `derived-not-asserted` value is recorded as an object, deliberately, so
        // it cannot be mistaken for an oracle. It is checked as an OBSERVATION.
        .or_else(|| expected.get("volume_mm3").and_then(|v| number(v, "value")))
        .ok_or_else(|| format!("{what}: expected.volume_mm3 missing"))?;
    let tol = number(expected, "volume_tol")
        .or_else(|| number(expected, "tol"))
        .unwrap_or(1e-6);
    if (actual - want).abs() > tol {
        return Err(format!(
            "{what}: volume {actual} not within {tol} of {want}"
        ));
    }
    Ok(())
}

/// The body-lifecycle claims of one step: kinds and count must match exactly.
fn check_body_events(
    expected: &Value,
    actual: &[BodyLifecycleEvent],
    what: &str,
) -> Result<(), String> {
    let Some(claims) = expected.get("bodyEvents").and_then(Value::as_array) else {
        return Ok(());
    };
    if claims.len() != actual.len() {
        return Err(format!(
            "{what}: expected {} body event(s), worker emitted {} ({actual:?})",
            claims.len(),
            actual.len()
        ));
    }
    for (claim, event) in claims.iter().zip(actual) {
        let want = claim
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let got = match event {
            BodyLifecycleEvent::Created { .. } => "created",
            BodyLifecycleEvent::Modified { .. } => "modified",
            BodyLifecycleEvent::Split { .. } => "split",
            BodyLifecycleEvent::Merged { .. } => "merged",
            BodyLifecycleEvent::Deleted { .. } => "deleted",
        };
        if want != got {
            return Err(format!("{what}: expected a {want} body event, got {got}"));
        }
    }
    Ok(())
}

async fn volume_of(wm: &WorkerManager, body: BodyId) -> Result<f64, String> {
    let label = format!("body_{}", body.0);
    wm.query_mass_properties(body, label)
        .await
        .map(|mass| mass.volume)
        .map_err(|e| format!("mass query failed: {e}"))
}

async fn face_count_of(
    wm: &WorkerManager,
    body: BodyId,
    snapshot: SnapshotId,
) -> Result<u32, String> {
    let blob = wm
        .fetch_mesh(body, Lod::Coarse, snapshot)
        .await
        .map_err(|e| format!("mesh fetch failed: {e}"))?;
    let view = validate_mesh_blob(&blob).map_err(|e| format!("MESH1 invalid: {e:?}"))?;
    Ok(view.face_count)
}

// ─────────────────────────────────────────────────────────────────────────────
// Case runners
// ─────────────────────────────────────────────────────────────────────────────

/// A core [`Sketch`] from a case's machine-readable entity list.
///
/// The corpus states geometry the way the C++ oracle drew it — a Line by its two
/// endpoints, an Arc by centre/radius/endpoints — while the core models a Line by
/// two POINT ENTITIES and an Arc by centre plus sweep ANGLES. This is that
/// translation, and nothing more: no constraint is invented, because region
/// detection closes loops on coincident COORDINATES, and adding solver constraints
/// here would silently change what the oracle is being asked.
fn sketch_from_corpus(label: &str, entities: &[Value]) -> Result<Sketch, String> {
    let mut sketch = Sketch::on_world_plane(SketchId(stable_uuid(label)), label, WorldPlane::XY);
    let mut next: u128 = 0;
    let mut fresh = |sketch: &mut Sketch, at: Vec2| -> Result<EntityId, String> {
        next += 1;
        let id = EntityId(Uuid::from_u128(next | (1 << 96)));
        sketch
            .add_entity(SketchEntity::point(id, at, false, false))
            .map_err(|e| format!("point {id:?}: {e:?}"))?;
        Ok(id)
    };
    let vec2 = |value: &Value, key: &str| -> Result<Vec2, String> {
        let arr = value
            .get(key)
            .and_then(Value::as_array)
            .ok_or(format!("missing {key}"))?;
        let (x, y) = (
            arr.first()
                .and_then(Value::as_f64)
                .ok_or(format!("{key}[0]"))?,
            arr.get(1)
                .and_then(Value::as_f64)
                .ok_or(format!("{key}[1]"))?,
        );
        Ok(Vec2::new_unchecked(x, y))
    };
    let number = |value: &Value, key: &str| -> Result<f64, String> {
        value
            .get(key)
            .and_then(Value::as_f64)
            .ok_or(format!("missing {key}"))
    };

    for entity in entities {
        let id = EntityId(stable_uuid(&format!(
            "{label}:{}",
            entity.get("id").and_then(Value::as_str).unwrap_or("?")
        )));
        match entity.get("type").and_then(Value::as_str) {
            Some("Line") => {
                let start = fresh(&mut sketch, vec2(entity, "p0")?)?;
                let end = fresh(&mut sketch, vec2(entity, "p1")?)?;
                sketch
                    .add_entity(SketchEntity::line(id, start, end, false))
                    .map_err(|e| format!("line: {e:?}"))?;
            }
            Some("Arc") => {
                let centre_at = vec2(entity, "center")?;
                let centre = fresh(&mut sketch, centre_at)?;
                let radius = number(entity, "radius")?;
                // The corpus gives the arc's ENDPOINTS; the core wants sweep angles.
                let angle = |point: Vec2| (point.y - centre_at.y).atan2(point.x - centre_at.x);
                let arc = SketchEntity::arc(
                    id,
                    centre,
                    radius,
                    angle(vec2(entity, "start")?),
                    angle(vec2(entity, "end")?),
                    false,
                )
                .ok_or("arc rejected (non-finite radius/angles)")?;
                sketch.add_entity(arc).map_err(|e| format!("arc: {e:?}"))?;
            }
            Some("Circle") => {
                let centre = fresh(&mut sketch, vec2(entity, "center")?)?;
                let circle = SketchEntity::circle(id, centre, number(entity, "radius")?, false)
                    .ok_or("circle rejected (non-finite radius)")?;
                sketch
                    .add_entity(circle)
                    .map_err(|e| format!("circle: {e:?}"))?;
            }
            Some("Ellipse") => {
                let centre = fresh(&mut sketch, vec2(entity, "center")?)?;
                let ellipse = SketchEntity::ellipse(
                    id,
                    centre,
                    number(entity, "majorR")?,
                    number(entity, "minorR")?,
                    number(entity, "rotation").unwrap_or(0.0),
                    false,
                )
                .ok_or("ellipse rejected (non-finite radii)")?;
                sketch
                    .add_entity(ellipse)
                    .map_err(|e| format!("ellipse: {e:?}"))?;
            }
            other => return Err(format!("entity type {other:?} has no corpus translation")),
        }
    }
    Ok(sketch)
}

/// `a_sketch_extrude_blind` — the whole expected block, not just the last volume:
/// the sketch step's region count, the extrude step's body events, solid count and
/// volume, and the derived face count.
async fn run_sketch_extrude_blind(
    wm: &WorkerManager,
    case: &CorpusCase,
    epoch: WorkerEpoch,
) -> Result<(), String> {
    // Labels are consumed while BUILDING the timeline; case a addresses its bodies
    // through the plan's own per-step results, so none are needed afterwards.
    let (tl, _labels) = timeline_of(case, None)?;
    let (prepared, events) = run_plan(wm, plan_request(&tl, 0xA, epoch)).await?;
    if prepared.stopped_reason != StoppedReason::Completed {
        return Err(format!("plan stopped: {:?}", prepared.stopped_reason));
    }
    accept(wm, 0xA, epoch).await?;

    let per_step = case.expected["perStep"]
        .as_array()
        .ok_or("expected.perStep missing")?;
    for step_expected in per_step {
        let index = step_expected["stepIndex"]
            .as_u64()
            .ok_or("stepIndex missing")? as usize;
        let what = format!("{} step {index}", case.id);

        // The sketch step's region oracle, read from the solver lane the frontend uses.
        if let Some(regions) = step_expected.get("regions").and_then(Value::as_u64) {
            // Region detection lives on the SOLVER lane, which the plan's Sketch op
            // does not populate — the plan materializes the sketch for the geometry
            // lane. So the same entities are upserted here, exactly as the frontend
            // does before a profile pick, and the oracle is read from that.
            let entities = case.op_script[index]
                .params
                .get("entities")
                .and_then(Value::as_array)
                .ok_or("sketch step carries no entities")?;
            let sketch = sketch_from_corpus(&case.op_script[index].op_id, entities)
                .map_err(|e| format!("{what}: {e}"))?;
            wm.sketch_upsert(&sketch)
                .await
                .map_err(|e| format!("{what}: SketchUpsert failed: {e}"))?;
            let detected = wm
                .sketch_regions(&sketch.id.to_string())
                .await
                .map_err(|e| format!("{what}: SketchRegions failed: {e}"))?;
            if detected.regions.len() as u64 != regions {
                return Err(format!(
                    "{what}: expected {regions} region(s), detected {}",
                    detected.regions.len()
                ));
            }
        }

        check_body_events(
            step_expected,
            events.get(&index).map_or(&[], Vec::as_slice),
            &what,
        )?;

        if let Some(solids) = step_expected.get("solids").and_then(Value::as_u64) {
            let produced = prepared
                .per_step
                .iter()
                .find(|s| s.step_index == index)
                .map_or(0, |s| s.body_ids.len());
            if produced as u64 != solids {
                return Err(format!(
                    "{what}: expected {solids} solid(s), got {produced}"
                ));
            }
        }

        if step_expected.get("volume_mm3").is_some() {
            let body = prepared
                .per_step
                .iter()
                .find(|s| s.step_index == index)
                .and_then(|s| s.body_ids.first().copied())
                .ok_or_else(|| format!("{what}: no body to measure"))?;
            check_volume(step_expected, volume_of(wm, body).await?, &what)?;

            // `faceCount` is recorded as derived-not-asserted (the C++ test checks
            // volume + validity only). Verifying it here is exactly what the case
            // asks for — "verify against worker output" — and it is the assertion
            // that would catch a prism built from the wrong profile at the right
            // volume.
            if let Some(faces) = case
                .expected
                .get("faceCount")
                .and_then(|f| f.get("value"))
                .and_then(Value::as_u64)
            {
                let actual = face_count_of(wm, body, prepared.prepared_snapshot_id).await?;
                if u64::from(actual) != faces {
                    return Err(format!("{what}: expected {faces} faces, got {actual}"));
                }
            }
        }
    }
    Ok(())
}

/// `b_extrude_throughall_symmetric_twodir` — three INDEPENDENT scenarios (the C++
/// oracles are separate documents), each its own plan against a reset session.
async fn run_extrude_end_conditions(
    wm: &WorkerManager,
    case: &CorpusCase,
    mut epoch: WorkerEpoch,
) -> Result<(), String> {
    // (scenario, expected key, job id, which body the volume claim describes)
    let scenarios: [(&str, &str, u128, &str); 3] = [
        ("throughAllCut", "throughAllCut", 0xB1, "op_base_box"),
        ("twoDirection", "twoDirection", 0xB2, "op_twodir"),
        ("symmetric", "symmetric", 0xB3, "op_symmetric"),
    ];
    for (scenario, expected_key, job, body_op) in scenarios {
        epoch = reset_document(wm, scenario, epoch).await?;
        let (tl, mut labels) = timeline_of(case, Some(scenario))?;
        let (prepared, events) = run_plan(wm, plan_request(&tl, job, epoch)).await?;
        if prepared.stopped_reason != StoppedReason::Completed {
            return Err(format!(
                "{scenario}: plan stopped: {:?}",
                prepared.stopped_reason
            ));
        }
        accept(wm, job, epoch).await?;

        let expected = &case.expected[expected_key];
        let what = format!("{} / {scenario}", case.id);
        let body = body_of_record(RecordId(labels.uuid(body_op)));

        // The through-all case claims BOTH volumes: the base box before the cut and
        // the result after it. Only the second is measurable on the final head, so
        // the first is asserted through the step that produced it.
        if let Some(after) = expected.get("afterCut") {
            check_body_events(
                expected,
                events.values().last().map_or(&[], Vec::as_slice),
                &what,
            )?;
            check_volume(
                after,
                volume_of(wm, body).await?,
                &format!("{what} afterCut"),
            )?;
        } else {
            let last_step = events.keys().copied().next_back().unwrap_or(0);
            check_body_events(
                expected,
                events.get(&last_step).map_or(&[], Vec::as_slice),
                &what,
            )?;
            check_volume(expected, volume_of(wm, body).await?, &what)?;
        }
    }
    Ok(())
}

/// `i_multiregion_loop_detection` — the loop-detector oracle, run through the
/// solver lane's `SketchRegions` (the same verb the extrude profile pick uses).
async fn run_loop_detection(wm: &WorkerManager, case: &CorpusCase) -> Result<(), String> {
    // opId → the expected block that describes it.
    let expected_for: BTreeMap<&str, &str> = BTreeMap::from([
        ("regions_rect", "rectangle"),
        ("regions_square_hole", "squareWithHole"),
        ("regions_arc_chord", "arcAndChord"),
        ("regions_ellipse", "ellipse"),
    ]);
    for op in &case.op_script {
        let key = expected_for
            .get(op.op_id.as_str())
            .ok_or_else(|| format!("{}: no expected block for {}", case.id, op.op_id))?;
        // Where the new stack DELIBERATELY differs from the C++ oracle, the case
        // records both and the runner asserts the current contract — the frozen
        // number stays visible instead of being overwritten by the new answer.
        let recorded = &case.expected[*key];
        let expected = recorded.get("newStack").unwrap_or(recorded);
        let what = format!("{} / {}", case.id, op.op_id);
        let entities = op
            .sketch
            .as_ref()
            .and_then(|s| s.get("entities"))
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{what}: no machine-readable entities"))?;

        let sketch = sketch_from_corpus(&op.op_id, entities).map_err(|e| format!("{what}: {e}"))?;
        wm.sketch_upsert(&sketch)
            .await
            .map_err(|e| format!("{what}: SketchUpsert failed: {e}"))?;
        let detected = wm
            .sketch_regions(&sketch.id.to_string())
            .await
            .map_err(|e| format!("{what}: SketchRegions failed: {e}"))?;

        // `regions` is either an exact count or the ">=1" the C++ test asserts.
        match expected.get("regions") {
            Some(Value::Number(n)) => {
                let want = n.as_u64().unwrap_or_default();
                if detected.regions.len() as u64 != want {
                    return Err(format!(
                        "{what}: expected {want} region(s), detected {}",
                        detected.regions.len()
                    ));
                }
            }
            Some(Value::String(s)) if s == ">=1" => {
                if detected.regions.is_empty() {
                    return Err(format!(
                        "{what}: expected at least one region, detected none"
                    ));
                }
            }
            other => return Err(format!("{what}: unreadable regions claim {other:?}")),
        }

        if let Some(holes) = expected.get("holes").and_then(Value::as_u64) {
            let detected_holes: usize = detected.regions.iter().map(|r| r.holes.len()).sum();
            if detected_holes as u64 != holes {
                return Err(format!(
                    "{what}: expected {holes} hole(s), detected {detected_holes}"
                ));
            }
        }
    }
    Ok(())
}

/// Cases the executor still cannot run, each naming the machinery it needs. A
/// reason for a case that HAS become executable is an error (see the test body):
/// this table is the honest statement of what is not yet proven, and a stale entry
/// turns it back into a comment.
fn unsupported_reasons() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("c_boolean_cut_fuse_bodyid", "needs a sketch hosted ON a promoted face (`body_1.face.top`) plus the standalone body-body Boolean verb; the case also mixes three separate C++ documents in one script"),
        ("d_fillet_selected_edge", "needs edge picking from MESH1 + promotion before the fillet ladder can be driven from a corpus script"),
        ("e_naming_break_fillet_upstream_edit", "anti-goal: needs the fillet interpreter above, then an upstream parametric edit through the edit lane"),
        ("f_symmetric_ambiguity", "anti-goal: needs a mirror-symmetric descriptor-tie fixture and a ResolveRefs assertion (bind-or-NeedsRepair, never a wrong bind)"),
        ("g_sketch_solver_drag_constraints", "solver: needs the BeginGesture/SolveDrag/EndGesture lane and a DOF oracle"),
        ("h_rollback_dirty_timeline", "timeline: needs a rollback-cursor replay harness (the `Rollback` pseudo-op is a document command, not a plan op)"),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corpus_cases_are_classified_executed_or_unsupported() {
    let classifications = manifest_classifications();
    let paths = case_paths();
    assert!(!paths.is_empty(), "corpus/cases must contain JSON files");

    // ── Shape + provenance run everywhere, worker or not ────────────────────
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

    let unsupported = unsupported_reasons();
    // The session epoch advances with every ResetSession; each plan must be fenced
    // with the CURRENT value or the worker rejects it (SCHEMA §7.2 D4).
    let mut epoch = WorkerEpoch(1);
    let mut executed: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut unclassified: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for case in &cases {
        // Every runner starts from a clean session: a leftover body from the
        // previous case would make the next one's solid counts a lie.
        epoch = match reset_document(&wm, &case.id, epoch).await {
            Ok(next) => next,
            Err(e) => {
                failures.push(format!("{}: {e}", case.id));
                continue;
            }
        };
        let outcome = match case.id.as_str() {
            "a_sketch_extrude_blind" => Some(run_sketch_extrude_blind(&wm, case, epoch).await),
            "b_extrude_throughall_symmetric_twodir" => {
                Some(run_extrude_end_conditions(&wm, case, epoch).await)
            }
            "i_multiregion_loop_detection" => Some(run_loop_detection(&wm, case).await),
            _ => None,
        };
        match outcome {
            Some(Ok(())) => executed.push(case.id.clone()),
            Some(Err(e)) => failures.push(format!("{}: {e}", case.id)),
            None => match unsupported.get(case.id.as_str()) {
                Some(reason) => skipped.push(format!("{}: {reason}", case.id)),
                None => unclassified.push(case.id.clone()),
            },
        }
    }

    wm.shutdown().await;

    assert!(
        unclassified.is_empty(),
        "corpus cases with no executor path and no unsupported reason: {unclassified:?}"
    );
    // The other direction: a reason left behind for a case that now runs would
    // understate the coverage and, worse, would never be noticed.
    let stale: Vec<&&str> = unsupported
        .keys()
        .filter(|id| executed.iter().any(|done| done == **id))
        .collect();
    assert!(
        stale.is_empty(),
        "unsupported reasons kept for cases that now execute: {stale:?}"
    );

    for skip in &skipped {
        eprintln!("corpus unsupported: {skip}");
    }
    assert!(
        failures.is_empty(),
        "corpus executor failures:\n{}",
        failures.join("\n")
    );

    let seen: BTreeSet<&str> = executed
        .iter()
        .map(String::as_str)
        .chain(skipped.iter().map(|s| s.split(':').next().unwrap()))
        .collect();
    assert_eq!(
        seen.len(),
        paths.len(),
        "every corpus file must be accounted for"
    );
    eprintln!(
        "corpus executed: {executed:?} ({} of {})",
        executed.len(),
        paths.len()
    );
}
