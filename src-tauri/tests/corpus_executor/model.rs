// ─────────────────────────────────────────────────────────────────────────────
// Case model
// ─────────────────────────────────────────────────────────────────────────────

/// One op in a case's `opScript`. Params remain frozen JSON; specialized runners
/// map them onto typed records and typed assertion models.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaseKind {
    SketchExtrudeBlind,
    ExtrudeEndConditions,
    BooleanBodyIdentity,
    FilletSelectedEdge,
    NamingAfterEdit,
    SymmetricAmbiguity,
    SketchSolver,
    RollbackTimeline,
    MultiRegion,
}

impl TryFrom<&str> for CaseKind {
    type Error = String;

    fn try_from(id: &str) -> Result<Self, Self::Error> {
        match id {
            "a_sketch_extrude_blind" => Ok(Self::SketchExtrudeBlind),
            "b_extrude_throughall_symmetric_twodir" => Ok(Self::ExtrudeEndConditions),
            "c_boolean_cut_fuse_bodyid" => Ok(Self::BooleanBodyIdentity),
            "d_fillet_selected_edge" => Ok(Self::FilletSelectedEdge),
            "e_naming_break_fillet_upstream_edit" => Ok(Self::NamingAfterEdit),
            "f_symmetric_ambiguity" => Ok(Self::SymmetricAmbiguity),
            "g_sketch_solver_drag_constraints" => Ok(Self::SketchSolver),
            "h_rollback_dirty_timeline" => Ok(Self::RollbackTimeline),
            "i_multiregion_loop_detection" => Ok(Self::MultiRegion),
            _ => Err(format!("unknown corpus case id {id}")),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleClaim {
    kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BooleanDirectionClaim {
    #[serde(rename = "baseVolume_mm3")]
    base_volume_mm3: f64,
    body_events: Vec<LifecycleClaim>,
}

#[derive(Debug, Deserialize)]
struct BooleanExpected {
    add_fuse: BooleanDirectionClaim,
    cut: BooleanDirectionClaim,
}

#[derive(Debug, Deserialize)]
struct FilletResultClaim {
    #[serde(rename = "bodyEvents")]
    body_events: Vec<LifecycleClaim>,
}

#[derive(Debug, Deserialize)]
struct FilletExpected {
    fillet_result: FilletResultClaim,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum VolumeValue {
    Asserted(f64),
    Observed { value: f64 },
}

impl VolumeValue {
    fn value(&self) -> f64 {
        match self {
            Self::Asserted(value) | Self::Observed { value } => *value,
        }
    }
}

#[derive(Debug, Deserialize)]
struct VolumeClaim {
    volume_mm3: Option<VolumeValue>,
    #[serde(default, alias = "volume_tol")]
    tol: Option<f64>,
    #[serde(rename = "bodyEvents", default)]
    body_events: Vec<LifecycleClaim>,
    #[serde(rename = "afterCut")]
    after_cut: Option<Box<VolumeClaim>>,
}

impl VolumeClaim {
    fn value(&self) -> Result<f64, String> {
        self.volume_mm3
            .as_ref()
            .map(VolumeValue::value)
            .ok_or_else(|| "volume_mm3 missing".into())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StepClaim {
    step_index: usize,
    regions: Option<u64>,
    solids: Option<u64>,
    volume_mm3: Option<VolumeValue>,
    volume_tol: Option<f64>,
    #[serde(default)]
    body_events: Vec<LifecycleClaim>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SketchExtrudeExpected {
    per_step: Vec<StepClaim>,
    face_count: DerivedCount,
}

#[derive(Debug, Deserialize)]
struct DerivedCount {
    value: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EndConditionExpected {
    through_all_cut: VolumeClaim,
    two_direction: VolumeClaim,
    symmetric: VolumeClaim,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CountClaim {
    Exact(u64),
    AtLeastOne(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegionClaim {
    regions: CountClaim,
    holes: Option<u64>,
    new_stack: Option<Box<RegionClaim>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegionsExpected {
    rectangle: RegionClaim,
    square_with_hole: RegionClaim,
    arc_and_chord: RegionClaim,
    ellipse: RegionClaim,
}

#[derive(Debug, Deserialize)]
struct DofClaim {
    state: String,
    dof: u32,
    #[serde(rename = "overConstrained")]
    over_constrained: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DragClaim {
    id: String,
    status: String,
    tol: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SolverExpected {
    dof: Vec<DofClaim>,
    drags: Vec<DragClaim>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorClaim {
    #[serde(default)]
    op_count: Option<usize>,
    applied_op_count: usize,
}

#[derive(Debug, Deserialize)]
struct OrderClaim {
    op1: usize,
    op2: usize,
    op3: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InsertClaim {
    op_count: usize,
    order: OrderClaim,
    applied_op_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FullRegenClaim {
    to_applied_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RollbackExpected {
    after_two_extrudes: CursorClaim,
    after_rollback: CursorClaim,
    after_insert_at_cursor: InsertClaim,
    full_regen: FullRegenClaim,
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
