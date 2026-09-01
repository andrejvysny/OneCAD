//! Headless vendor-STEP → library-component ingest (Component Library WP-C2).
//!
//! A vendor ships a folder of STEP files; a usable library needs one *component*
//! per file. Between those two is everything this module does: read the file,
//! decide which of its solids are the part (a NEMA 17 STEP carries 67, most of
//! them internal), fuse the survivors into the single solid spec §9 requires, and
//! write the package.
//!
//! **Two entry points over one core.** [`ingest_components_at`] is the whole lane
//! and knows nothing about Tauri; `crate::api::ingest_components` drives it from
//! the UI with a defaults-only plan, and `onecad-library-ingest` drives it from a
//! tracked [`IngestPlan`] recipe where each part carries its own keep-list. The
//! recipe exists because a keep-list is a human judgement — which solids are the
//! exterior envelope of a stepper motor is not something a heuristic should be
//! guessing on the user's behalf.
//!
//! **The runtime is private and per-part.** Every part gets a FRESH blank
//! [`DocumentRuntime`] built over the caller's worker; nothing here can reach the
//! open document, and the caller is expected to hand over a worker of its own
//! (the worker is one session per process, so ingesting on the open document's
//! worker would trample the session the user is editing — the same reasoning
//! `library::rebake_document_component` records).
//!
//! **Nothing here panics on a bad file.** Every per-part failure is a
//! [`IngestStatusDto::Failed`] row with the reason; a keep-list that cannot be
//! fused, or a stick that is not a prism, is a [`IngestStatusDto::Refused`] row.
//! A batch of seven files always returns seven rows.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use onecad_core::document::record::{
    BooleanOp, BooleanParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, RecordId};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, Outcome, RegenRequest};

use onecad_library::package::{
    AttachmentSpec, ComponentPackage, Identity, Metadata, ParameterSpec, SourceSpec,
};
use onecad_library::{BlobComponentKind, EmbeddedComponentRequest, Library};

use crate::document_runtime::DocumentRuntime;
use crate::dto::{
    IngestComponentsReportDto, IngestPartKindDto, IngestPartResultDto, IngestStatusDto,
};
use crate::error::ApiError;
use crate::export::GeometryExporter;
use crate::worker::manager::SupervisorConfig;
use crate::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, StepImport, WorkerManager,
};

/// The MACHINE-READABLE marker on the "your keep-list is still several separate
/// solids" refusal.
///
/// Same discipline as `library::MULTI_SOLID_REFUSAL`: the caller (a recipe author
/// iterating on a keep-list, or the ingest dialog) has to tell this refusal apart
/// from every other one, and matching prose would break the moment the sentence is
/// reworded.
pub const DISJOINT_REFUSAL: &str = "INGEST_DISJOINT_AFTER_FUSE";

/// How long an ingest waits for its own worker to finish the OCW1 handshake.
/// Matches `library`'s re-bake lane: a cold process start plus OCCT's own init.
const INGEST_WORKER_READY_TIMEOUT: Duration = Duration::from_secs(20);

/// Product-name globs the interactive lane always drops. Vendor assemblies ship
/// their fasteners as separate sub-products (the SG90 carries two ISO 7045
/// screws), and a screw is never part of the envelope the user wanted.
pub const DEFAULT_DROP_NAMES: [&str; 3] = ["*ISO 7045*", "*ISO 4762*", "*DIN 912*"];

// ─────────────────────────────────────────────────────────────────────────────
// The plan (recipe) types
// ─────────────────────────────────────────────────────────────────────────────

/// Which package kind one part becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestKind {
    /// A vendor solid stored as-is.
    Embedded,
    /// WP-C's length-parametric canonical face — a stick.
    Profile,
}

impl IngestKind {
    fn dto(self) -> IngestPartKindDto {
        match self {
            Self::Embedded => IngestPartKindDto::Embedded,
            Self::Profile => IngestPartKindDto::Profile,
        }
    }
}

/// How a part's keep-list selects solids out of what the STEP reader recovered.
#[derive(Debug, Clone, PartialEq)]
pub enum KeepMode {
    /// Everything that survived `drop_names`.
    All,
    /// Ordinal solid indices, as printed by `onecad-library-ingest --inspect`.
    Indices(Vec<usize>),
    /// Product-name globs.
    Names(Vec<String>),
    /// Every solid at or above this fraction of the total volume.
    VolumeFraction(f64),
}

/// A part's solid selection.
#[derive(Debug, Clone, PartialEq)]
pub struct KeepSpec {
    pub mode: KeepMode,
    /// Product-name globs dropped BEFORE `mode` is applied, so an index list
    /// authored from `--inspect` still means what it said.
    pub drop_names: Vec<String>,
}

impl Default for KeepSpec {
    fn default() -> Self {
        Self {
            mode: KeepMode::All,
            drop_names: Vec::new(),
        }
    }
}

/// The `profile`-kind extras (spec §2.1 `[parameters].length`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileSpec {
    /// The package's declared default length — the stick the vendor ships.
    pub length_default: f64,
    /// Lower bound recorded on the parameter.
    pub length_min: Option<f64>,
    /// Forces the prism axis instead of letting the worker find it.
    pub axis_hint: Option<[f64; 3]>,
}

impl Default for ProfileSpec {
    fn default() -> Self {
        Self {
            length_default: 500.0,
            length_min: Some(1.0),
            axis_hint: None,
        }
    }
}

/// An authored override of the default identity seating frame.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentOverride {
    pub origin: [f64; 3],
    pub z: [f64; 3],
    pub x: [f64; 3],
    pub accepts: Vec<String>,
}

/// One part of an [`IngestPlan`] — one STEP file, one component package.
#[derive(Debug, Clone, PartialEq)]
pub struct IngestPart {
    /// Absolute path to the STEP file.
    pub file: PathBuf,
    pub kind: IngestKind,
    pub id: String,
    pub version: String,
    pub name: String,
    pub standard: Option<String>,
    /// Kept VERBATIM, `{length}` placeholders and all — the configurator
    /// substitutes them, not the ingest.
    pub designation: Option<String>,
    pub category: Vec<String>,
    pub tags: Vec<String>,
    pub keep: KeepSpec,
    pub profile: ProfileSpec,
    pub attachment: Option<AttachmentOverride>,
}

/// A parsed ingest recipe.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IngestPlan {
    pub parts: Vec<IngestPart>,
}

/// The catalog defaults a defaults-only plan is built from
/// ([`plan_from_paths`]) and a recipe's `[defaults]` table falls back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestDefaults {
    pub vendor: String,
    pub category: Vec<String>,
    pub tags: Vec<String>,
    pub version: String,
}

impl Default for IngestDefaults {
    fn default() -> Self {
        Self {
            vendor: "vendor".to_string(),
            category: vec!["imported".to_string()],
            tags: Vec::new(),
            version: "1.0.0".to_string(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Recipe parsing
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct RawRecipe {
    #[serde(default)]
    defaults: RawDefaults,
    #[serde(default)]
    part: Vec<RawPart>,
}

#[derive(Default, serde::Deserialize)]
struct RawDefaults {
    vendor: Option<String>,
    category: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    version: Option<String>,
}

#[derive(serde::Deserialize)]
struct RawPart {
    file: String,
    kind: Option<String>,
    id: String,
    name: Option<String>,
    #[allow(dead_code)]
    vendor: Option<String>,
    version: Option<String>,
    standard: Option<String>,
    designation: Option<String>,
    category: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    profile: Option<RawProfile>,
    keep: Option<RawKeep>,
    attachment: Option<RawAttachment>,
}

#[derive(serde::Deserialize)]
struct RawProfile {
    length_default: Option<f64>,
    length_min: Option<f64>,
    axis_hint: Option<[f64; 3]>,
}

#[derive(serde::Deserialize)]
struct RawKeep {
    mode: Option<String>,
    indices: Option<Vec<usize>>,
    names: Option<Vec<String>>,
    min_fraction: Option<f64>,
    drop_names: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
struct RawAttachment {
    origin: Option<[f64; 3]>,
    z: Option<[f64; 3]>,
    x: Option<[f64; 3]>,
    accepts: Option<Vec<String>>,
}

/// Parses an `ingest.toml` recipe. `base_dir` is the directory the recipe lives
/// in — every `part.file` is resolved relative to it, so a recipe is movable with
/// its STEP files.
///
/// # Errors
/// A prose message naming the offending part: malformed TOML, an id that is not a
/// lowercase dotted namespace, an unknown `kind` or `keep.mode`, or a keep mode
/// missing the field it selects by.
pub fn parse_recipe(toml_str: &str, base_dir: &Path) -> Result<IngestPlan, String> {
    let raw: RawRecipe = toml::from_str(toml_str).map_err(|e| format!("ingest recipe: {e}"))?;
    let defaults = IngestDefaults {
        vendor: raw.defaults.vendor.unwrap_or_else(|| "vendor".to_string()),
        category: raw
            .defaults
            .category
            .unwrap_or_else(|| vec!["imported".to_string()]),
        tags: raw.defaults.tags.unwrap_or_default(),
        version: raw.defaults.version.unwrap_or_else(|| "1.0.0".to_string()),
    };
    let parts = raw
        .part
        .into_iter()
        .map(|p| part_from_raw(p, &defaults, base_dir))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(IngestPlan { parts })
}

fn part_from_raw(p: RawPart, d: &IngestDefaults, base_dir: &Path) -> Result<IngestPart, String> {
    validate_component_id(&p.id)?;
    let kind = match p.kind.as_deref().unwrap_or("embedded") {
        "embedded" => IngestKind::Embedded,
        "profile" => IngestKind::Profile,
        other => {
            return Err(format!(
                "part `{}`: unknown kind {other:?} (expected `embedded` or `profile`)",
                p.id
            ))
        }
    };
    let name = p.name.unwrap_or_else(|| p.id.clone());
    let raw_profile = p.profile.unwrap_or(RawProfile {
        length_default: None,
        length_min: None,
        axis_hint: None,
    });
    let profile = ProfileSpec {
        length_default: raw_profile.length_default.unwrap_or(500.0),
        length_min: raw_profile.length_min.or(Some(1.0)),
        axis_hint: raw_profile.axis_hint,
    };
    if kind == IngestKind::Profile && !profile.length_default.is_finite() {
        return Err(format!(
            "part `{}`: profile.length_default must be finite",
            p.id
        ));
    }
    Ok(IngestPart {
        file: base_dir.join(&p.file),
        kind,
        id: p.id.clone(),
        version: p.version.unwrap_or_else(|| d.version.clone()),
        name,
        standard: p.standard,
        designation: p.designation,
        category: p.category.unwrap_or_else(|| d.category.clone()),
        tags: p.tags.unwrap_or_else(|| d.tags.clone()),
        keep: keep_from_raw(p.keep, &p.id)?,
        profile,
        attachment: p.attachment.map(|a| AttachmentOverride {
            origin: a.origin.unwrap_or([0.0; 3]),
            z: a.z.unwrap_or([0.0, 0.0, 1.0]),
            x: a.x.unwrap_or([1.0, 0.0, 0.0]),
            accepts: a.accepts.unwrap_or_else(|| vec!["plane".to_string()]),
        }),
    })
}

fn keep_from_raw(raw: Option<RawKeep>, id: &str) -> Result<KeepSpec, String> {
    let Some(raw) = raw else {
        return Ok(KeepSpec::default());
    };
    let mode = match raw.mode.as_deref().unwrap_or("all") {
        "all" => KeepMode::All,
        "indices" => {
            KeepMode::Indices(raw.indices.ok_or_else(|| {
                format!("part `{id}`: keep.mode = \"indices\" needs keep.indices")
            })?)
        }
        "names" => KeepMode::Names(
            raw.names
                .ok_or_else(|| format!("part `{id}`: keep.mode = \"names\" needs keep.names"))?,
        ),
        "volume_fraction" => KeepMode::VolumeFraction(raw.min_fraction.ok_or_else(|| {
            format!("part `{id}`: keep.mode = \"volume_fraction\" needs keep.min_fraction")
        })?),
        other => {
            return Err(format!(
                "part `{id}`: unknown keep.mode {other:?} (expected all | indices | names | \
                 volume_fraction)"
            ))
        }
    };
    Ok(KeepSpec {
        mode,
        drop_names: raw.drop_names.unwrap_or_default(),
    })
}

/// `^[a-z0-9]+(\.[a-z0-9-]+)+$` — a lowercase dotted namespace, at least two
/// segments. Stricter than `onecad-library`'s own path-safety check (which also
/// admits `_`) because these ids are minted, not migrated.
fn validate_component_id(id: &str) -> Result<(), String> {
    let mut segments = id.split('.');
    let ok_first = segments.next().is_some_and(|s| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    });
    let mut count = 0usize;
    let ok_rest = segments.all(|s| {
        count += 1;
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    if ok_first && ok_rest && count >= 1 {
        return Ok(());
    }
    Err(format!(
        "component id {id:?} must be a lowercase dotted namespace, \
         e.g. `onecad.vendor.rollco.rp4030`"
    ))
}

/// Builds the defaults-only plan the interactive `ingest_components` command
/// runs: every path becomes an `embedded` part under `defaults`, keeping every
/// solid except the fastener sub-products.
#[must_use]
pub fn plan_from_paths(paths: &[String], defaults: &IngestDefaults) -> IngestPlan {
    let parts = paths
        .iter()
        .map(|p| {
            let path = PathBuf::from(p);
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "component".to_string());
            IngestPart {
                file: path,
                kind: IngestKind::Embedded,
                id: format!("onecad.vendor.{}.{}", slug(&defaults.vendor), slug(&stem)),
                version: defaults.version.clone(),
                name: stem,
                standard: None,
                designation: None,
                category: defaults.category.clone(),
                tags: defaults.tags.clone(),
                keep: KeepSpec {
                    mode: KeepMode::All,
                    drop_names: DEFAULT_DROP_NAMES
                        .iter()
                        .map(|s| (*s).to_string())
                        .collect(),
                },
                profile: ProfileSpec::default(),
                attachment: None,
            }
        })
        .collect();
    IngestPlan { parts }
}

/// Lowercase `[a-z0-9-]`, runs of anything else collapsed to one `-`. Empty in ⇒
/// `"part"`, so a slug is always a legal id segment.
fn slug(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "part".to_string()
    } else {
        trimmed.to_string()
    }
}

/// `*`-only glob match, case-insensitive — enough for the product-name patterns a
/// keep-list uses (`"*ISO 7045*"`), and deliberately not a regex.
fn glob_match(pattern: &str, text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    let mut cursor = 0usize;
    let segments: Vec<&str> = pattern.split('*').collect();
    let last = segments.len() - 1;
    for (i, seg) in segments.iter().enumerate() {
        if seg.is_empty() {
            continue;
        }
        if i == 0 && !text[cursor..].starts_with(seg) {
            return false;
        }
        if i == last && !pattern.ends_with('*') {
            return text[cursor..].ends_with(seg) && text.len() - cursor >= seg.len();
        }
        match text[cursor..].find(seg) {
            Some(at) => cursor += at + seg.len(),
            None => return false,
        }
    }
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// The ingest
// ─────────────────────────────────────────────────────────────────────────────

/// One solid the STEP reader recovered, as measured before any keep-list runs.
#[derive(Debug, Clone)]
pub struct SolidRow {
    /// Ordinal index — what a `keep.indices` entry names.
    pub index: usize,
    pub body: BodyId,
    /// The STEP product name, when the file carried one.
    pub name: String,
    /// Exact `QueryMassProperties` volume, mm³.
    pub volume_mm3: f64,
    pub face_count: u32,
    pub solid_count: u32,
    /// Axis-aligned `(min, max)` extents in mm, read from the MESH1 header the
    /// regen already produced. TESSELLATION bounds, not `Bnd_Box` — a couple of
    /// chord-tolerance microns out, which is irrelevant for the one thing it is
    /// for: telling a 42 mm motor can apart from a 5 mm shaft when the vendor
    /// file carries no usable product names.
    pub bbox: Option<([f32; 3], [f32; 3])>,
}

/// What one part's STEP file contained, before any package was written — the
/// table `onecad-library-ingest --inspect` prints so a keep-list can be authored.
#[derive(Debug, Clone)]
pub struct InspectedPart {
    pub path: PathBuf,
    pub solids: Result<Vec<SolidRow>, String>,
    pub import_ms: u64,
}

/// Owns an ingest's OWN worker process and retires it on every exit path.
///
/// Same guard as `library`'s re-bake lane, and for the same reason: a leaked
/// sidecar would outlive the call and hold an OCCT process for the rest of the
/// session. Never installed into `AppState`'s slots, so the open document's
/// session, fencing tokens and epoch never see it.
pub struct EphemeralWorker(pub WorkerManager);

impl Drop for EphemeralWorker {
    fn drop(&mut self) {
        self.0.retire();
    }
}

/// The supervision policy an ingest worker MUST run under.
///
/// **Not the production policy.** SCHEMA §8's ping 5 s × 2 misses is tuned for an
/// interactive session, where ten seconds of silence really does mean a hung
/// kernel. An ingest's unit of work is a whole vendor assembly — reading and
/// tessellating the 67-solid NEMA 17 file keeps the worker busy past that budget
/// in one uninterrupted call — and the interactive heuristic then SIGKILLs it
/// mid-import, turning every remaining part of the batch into "worker not
/// connected". So the batch lane gets a batch budget (30 s × 4). Everything else
/// is production policy, restart and flap behaviour included: a worker that
/// genuinely dies still dies.
///
/// Public because a test that drives [`ingest_components_at`] over its own
/// `WorkerManager` needs the SAME policy — spawning that worker under production
/// supervision reproduces exactly the kill this exists to prevent.
#[must_use]
pub fn ingest_supervisor_config(binary: PathBuf) -> SupervisorConfig {
    SupervisorConfig {
        ping_interval: Duration::from_secs(30),
        ping_timeout: Duration::from_secs(30),
        max_missed_pings: 4,
        ..SupervisorConfig::production(binary)
    }
}

/// Spawns a worker of the ingest's own, under [`ingest_supervisor_config`], and
/// waits out its handshake.
///
/// # Errors
/// [`ApiError::Internal`] when no worker binary resolves, [`ApiError::Worker`]
/// when it never becomes ready.
pub async fn spawn_ingest_worker() -> Result<EphemeralWorker, ApiError> {
    let bin = resolve_worker_path().ok_or_else(|| {
        ApiError::Internal(
            "ingestComponents: no geometry worker binary resolved — an ingest needs one".into(),
        )
    })?;
    let worker = EphemeralWorker(WorkerManager::spawn(ingest_supervisor_config(bin)));
    if !worker.0.wait_ready(INGEST_WORKER_READY_TIMEOUT).await {
        return Err(ApiError::Worker(
            "ingestComponents: the ingest worker never became ready".into(),
        ));
    }
    Ok(worker)
}

/// Reads every part of `plan` and reports its solids WITHOUT writing anything.
/// The `--inspect` lane: this is how a keep-list is authored.
pub async fn inspect_components_at(
    worker: &WorkerManager,
    plan: &IngestPlan,
) -> Vec<InspectedPart> {
    let mut out = Vec::with_capacity(plan.parts.len());
    for part in &plan.parts {
        let started = Instant::now();
        let solids = match import_part(worker, &part.file).await {
            Ok((rt, rows)) => {
                drop(rt);
                Ok(rows)
            }
            Err(e) => Err(e.to_string()),
        };
        out.push(InspectedPart {
            path: part.file.clone(),
            solids,
            import_ms: started.elapsed().as_millis() as u64,
        });
    }
    out
}

/// Runs `plan` against `library_root`, writing one package per successful part.
///
/// Never `Err`: a batch of seven files always answers with seven rows, because a
/// single unreadable file must not cost the other six their result.
pub async fn ingest_components_at(
    worker: &WorkerManager,
    library_root: &Path,
    plan: &IngestPlan,
) -> IngestComponentsReportDto {
    let mut parts = Vec::with_capacity(plan.parts.len());
    for part in &plan.parts {
        parts.push(ingest_one(worker, library_root, part).await);
    }
    IngestComponentsReportDto {
        parts,
        library_root: library_root.to_string_lossy().into_owned(),
    }
}

/// One part, start to finish. Classifies its own outcome so the batch loop above
/// never has to decide what an error means.
async fn ingest_one(
    worker: &WorkerManager,
    library_root: &Path,
    part: &IngestPart,
) -> IngestPartResultDto {
    let started = Instant::now();
    let row = |status, message: Option<String>| IngestPartResultDto {
        path: part.file.to_string_lossy().into_owned(),
        id: Some(part.id.clone()),
        version: Some(part.version.clone()),
        kind: Some(part.kind.dto()),
        status,
        message,
        solids_found: None,
        solids_kept: None,
        face_count: None,
        import_ms: None,
    };

    let (runtime, solids) = match import_part(worker, &part.file).await {
        Ok(v) => v,
        Err(e) => return row(IngestStatusDto::Failed, Some(e.to_string())),
    };
    let import_ms = started.elapsed().as_millis() as u64;
    let found = solids.len();

    let kept = select_kept(&solids, &part.keep);
    if kept.is_empty() {
        let mut r = row(
            IngestStatusDto::Refused,
            Some(format!(
                "keep-list selected no solid out of {found} (drop_names removed \
                 {} of them)",
                found - solids_after_drop(&solids, &part.keep.drop_names).len()
            )),
        );
        r.solids_found = Some(found);
        r.solids_kept = Some(0);
        r.import_ms = Some(import_ms);
        return r;
    }

    let outcome = match part.kind {
        IngestKind::Embedded => {
            save_embedded(worker, library_root, part, &runtime, &solids, &kept).await
        }
        IngestKind::Profile => save_profile(worker, library_root, part, &runtime, &kept).await,
    };
    let mut r = match outcome {
        Ok(face_count) => {
            let mut r = row(IngestStatusDto::Ok, None);
            r.face_count = Some(face_count);
            r
        }
        Err(PartRefusal::Refused(m)) => row(IngestStatusDto::Refused, Some(m)),
        Err(PartRefusal::Failed(m)) => row(IngestStatusDto::Failed, Some(m)),
    };
    r.solids_found = Some(found);
    r.solids_kept = Some(kept.len());
    r.import_ms = Some(import_ms);
    r
}

/// A per-part non-success. `Refused` is the ingest working and saying no;
/// `Failed` is something not cooperating.
enum PartRefusal {
    Refused(String),
    Failed(String),
}

impl From<ApiError> for PartRefusal {
    fn from(e: ApiError) -> Self {
        Self::Failed(e.to_string())
    }
}

/// Reads one STEP file into a FRESH blank runtime and measures every solid it
/// produced. The runtime is returned so the caller can go on authoring against
/// exactly these body ids.
///
/// Returned already wrapped in a `Mutex<Option<…>>` because that is the shape
/// `library::extract_prism_profile_at` takes — the profile leg reuses that
/// function verbatim rather than re-deriving its head fence, and this runtime is
/// local to one part, so the lock is never contended.
async fn import_part(
    worker: &WorkerManager,
    file: &Path,
) -> Result<(tokio::sync::Mutex<Option<DocumentRuntime>>, Vec<SolidRow>), ApiError> {
    let step_import: Arc<dyn StepImport> = Arc::new(worker.clone());
    let prepared = crate::imports::prepare_import(&*step_import, file).await?;

    let engine: Arc<dyn GeometryEngine> = Arc::new(worker.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(worker.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(worker.clone());
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    rt.add_import_record(&prepared, false)?;
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    if let Some(failed) = report.failed_steps.first() {
        return Err(ApiError::OpFailed {
            message: format!("importing {}: {}", file.display(), failed.message),
            diagnostics: failed.diagnostics.clone(),
        });
    }
    if !matches!(report.outcome, Outcome::Published(_)) {
        return Err(ApiError::OpFailed {
            message: format!(
                "importing {} did not publish ({})",
                file.display(),
                report.outcome_str()
            ),
            diagnostics: Vec::new(),
        });
    }

    let rows = measure_bodies(worker, &mut rt).await?;
    Ok((tokio::sync::Mutex::new(Some(rt)), rows))
}

/// Per-body measurements. Volume and the two counts are EXACT (§7.5
/// `QueryMassProperties` + `QueryBodyTopology`, straight off the BRep); the
/// extents come from the MESH1 header the regen already published, which costs no
/// extra round trip and is only ever read by a human authoring a keep-list.
async fn measure_bodies(
    worker: &WorkerManager,
    rt: &mut DocumentRuntime,
) -> Result<Vec<SolidRow>, ApiError> {
    let mut rows = Vec::new();
    for (index, body) in rt.head_body_ids().into_iter().enumerate() {
        let label = body.to_string();
        let mass = ElementQuery::query_mass_properties(worker, body, label.clone()).await?;
        let topo = ElementQuery::query_body_topology(worker, body, label).await?;
        let name = rt.body_meta(body).map(|m| m.name).unwrap_or_default();
        let bbox = mesh_bounds(rt, body).await;
        rows.push(SolidRow {
            index,
            body,
            name,
            volume_mm3: mass.volume,
            face_count: topo.face_count,
            solid_count: topo.solid_count,
            bbox,
        });
    }
    Ok(rows)
}

/// The published mesh's declared bounds, or `None` when no mesh was cached for
/// this body at the coarse LOD.
async fn mesh_bounds(rt: &mut DocumentRuntime, body: BodyId) -> Option<([f32; 3], [f32; 3])> {
    let blob = rt.get_mesh(body, Lod::Coarse, None).await?;
    let view = onecad_protocol::mesh::validate_mesh_blob(&blob).ok()?;
    Some((view.bbox_min, view.bbox_max))
}

/// `drop_names` applied, `mode` not yet.
fn solids_after_drop<'a>(solids: &'a [SolidRow], drop_names: &[String]) -> Vec<&'a SolidRow> {
    solids
        .iter()
        .filter(|s| !drop_names.iter().any(|p| glob_match(p, &s.name)))
        .collect()
}

/// The keep-list, resolved to the solids it selects. `drop_names` runs FIRST so
/// an index list authored from `--inspect` keeps naming the same solids.
fn select_kept<'a>(solids: &'a [SolidRow], keep: &KeepSpec) -> Vec<&'a SolidRow> {
    let survivors = solids_after_drop(solids, &keep.drop_names);
    match &keep.mode {
        KeepMode::All => survivors,
        KeepMode::Indices(want) => survivors
            .into_iter()
            .filter(|s| want.contains(&s.index))
            .collect(),
        KeepMode::Names(globs) => survivors
            .into_iter()
            .filter(|s| globs.iter().any(|g| glob_match(g, &s.name)))
            .collect(),
        KeepMode::VolumeFraction(min) => {
            let total: f64 = solids.iter().map(|s| s.volume_mm3).sum();
            if total <= 0.0 {
                return Vec::new();
            }
            survivors
                .into_iter()
                .filter(|s| s.volume_mm3 / total >= *min)
                .collect()
        }
    }
}

/// The `embedded` leg: fuse the kept solids into one, bake it, write the package.
async fn save_embedded(
    worker: &WorkerManager,
    library_root: &Path,
    part: &IngestPart,
    runtime: &tokio::sync::Mutex<Option<DocumentRuntime>>,
    all: &[SolidRow],
    kept: &[&SolidRow],
) -> Result<u32, PartRefusal> {
    let body = fuse_kept(runtime, all, kept).await?;
    let topo = ElementQuery::query_body_topology(worker, body, body.to_string())
        .await
        .map_err(ApiError::from)?;
    if topo.solid_count != 1 {
        return Err(PartRefusal::Refused(disjoint_message(
            topo.solid_count,
            kept,
        )));
    }

    let scratch = scratch_dir(&part.id)?;
    let geometry_path = scratch.join("geometry.xbf");
    let exporter: Arc<dyn GeometryExporter> = Arc::new(worker.clone());
    let baked = exporter
        .export_geometry(&geometry_path.to_string_lossy(), &[body], "xbf", false)
        .await
        .map_err(ApiError::from);
    let saved = match baked {
        Ok(baked) if baked.solid_count == 1 => std::fs::read(&geometry_path)
            .map_err(|e| PartRefusal::Failed(format!("read baked geometry: {e}")))
            .and_then(|bytes| {
                write_package(
                    library_root,
                    part,
                    BlobComponentKind::Embedded,
                    bytes,
                    baked.codec,
                    Some(baked.format),
                    BTreeMap::new(),
                )
            }),
        Ok(baked) => Err(PartRefusal::Refused(disjoint_message(
            baked.solid_count as u32,
            kept,
        ))),
        Err(e) => Err(e.into()),
    };
    let _ = std::fs::remove_dir_all(&scratch);
    saved.map(|()| topo.face_count)
}

/// The `profile` leg: one stick, probed for prism-ness, stored as its canonical
/// end-cap face.
async fn save_profile(
    worker: &WorkerManager,
    library_root: &Path,
    part: &IngestPart,
    runtime: &tokio::sync::Mutex<Option<DocumentRuntime>>,
    kept: &[&SolidRow],
) -> Result<u32, PartRefusal> {
    // A profile is ONE stick. Fusing first would hide a keep-list that selected
    // two different extrusions, and the canonical face would then be whichever
    // end cap the fuse happened to leave at the axis minimum.
    let [stick] = kept else {
        return Err(PartRefusal::Refused(format!(
            "a `profile` part must be exactly one solid before any fuse; the keep-list \
             selected {}",
            kept.len()
        )));
    };
    if stick.solid_count != 1 {
        return Err(PartRefusal::Refused(format!(
            "a `profile` part must be exactly one solid; solid {} carries {}",
            stick.index, stick.solid_count
        )));
    }

    let scratch = scratch_dir(&part.id)?;
    let face_path = scratch.join("profile.brep");
    let exporter: Arc<dyn GeometryExporter> = Arc::new(worker.clone());
    let answer = crate::library::extract_prism_profile_at(
        runtime,
        exporter,
        &stick.body.to_string(),
        Some(&face_path.to_string_lossy()),
    )
    .await;

    let saved = match answer {
        Ok(crate::dto::PrismProfileAnswerDto::Refused(r)) => Err(PartRefusal::Refused(format!(
            "{}: {} (volumeRatio {:.6})",
            r.code,
            r.message,
            r.volume_ratio.unwrap_or(f64::NAN)
        ))),
        Ok(crate::dto::PrismProfileAnswerDto::Prism(prism)) => match prism.bake {
            None => Err(PartRefusal::Failed(
                "ExtractPrismProfile measured the prism but wrote no face".into(),
            )),
            Some(bake) => std::fs::read(&face_path)
                .map_err(|e| PartRefusal::Failed(format!("read canonical face: {e}")))
                .and_then(|bytes| {
                    write_package(
                        library_root,
                        part,
                        BlobComponentKind::Profile,
                        bytes,
                        bake.codec,
                        Some(bake.format),
                        profile_parameters(part),
                    )
                }),
        },
        Err(e) => Err(e.into()),
    };
    let _ = std::fs::remove_dir_all(&scratch);
    saved.map(|()| stick.face_count)
}

/// The package's declared `length` (spec §2.1 `[parameters]`) — the stick the
/// vendor ships, so an instance that overrides nothing places at that length.
fn profile_parameters(part: &IngestPart) -> BTreeMap<String, ParameterSpec> {
    let mut spec = ParameterSpec::free_variable("length", part.profile.length_default);
    spec.min = part.profile.length_min;
    BTreeMap::from([("length".to_string(), spec)])
}

/// Unions every kept solid onto the largest of them.
///
/// The largest is the target on purpose: `BooleanOp` MODIFIES the target in place
/// when the result is a single solid, so anchoring on the envelope keeps the
/// result's body id stable across the whole chain, and a chain that splits stops
/// looking like a successful fuse.
async fn fuse_kept(
    runtime: &tokio::sync::Mutex<Option<DocumentRuntime>>,
    all: &[SolidRow],
    kept: &[&SolidRow],
) -> Result<BodyId, PartRefusal> {
    let target = kept
        .iter()
        .max_by(|a, b| a.volume_mm3.total_cmp(&b.volume_mm3))
        .expect("the caller refused an empty keep-list");
    if kept.len() == 1 {
        return Ok(target.body);
    }
    let dropped: HashSet<BodyId> = all
        .iter()
        .map(|s| s.body)
        .filter(|b| !kept.iter().any(|k| k.body == *b))
        .collect();

    let mut guard = runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| PartRefusal::Failed("ingest runtime was closed".into()))?;
    // The FIRST fuse's dirty floor, so the regen below does not replay the import
    // it is built on. Re-reading a 7 MB assembly and re-tessellating all 67 of its
    // bodies per part is what pushed the worker past its ping budget.
    let mut fuse_from: Option<usize> = None;
    for tool in kept.iter().filter(|k| k.body != target.body) {
        let op = Operation::Known(KnownOperation::Boolean(BooleanParams {
            operation: BooleanOp::Union,
            target_body: target.body,
            tool_body: tool.body,
            extra: Default::default(),
        }));
        let outcome = rt
            .apply(EditCommand::AddOperation {
                record: OperationRecord::new(RecordId::new(), 0, "Fuse", op),
                at_cursor: true,
            })
            .map_err(|e| PartRefusal::Failed(format!("authoring the fuse chain: {e}")))?;
        if fuse_from.is_none() {
            fuse_from = outcome.dirty.map(|d| d.from);
        }
    }
    let report = rt
        .run_regen(
            RegenRequest::ToEnd {
                from: fuse_from.unwrap_or(0),
            },
            CancelToken::new(),
        )
        .await;
    if let Some(failed) = report.failed_steps.first() {
        return Err(PartRefusal::Refused(format!(
            "{DISJOINT_REFUSAL}: the fuse chain failed at {}: {}",
            failed.record_id, failed.message
        )));
    }
    // A regen that did not publish says NOTHING about how many solids remain —
    // the head is whatever it was before. Counting survivors here would report a
    // torn-down worker as a disjoint keep-list, which is the one refusal a recipe
    // author would act on by rewriting a keep-list that was never wrong.
    if !matches!(report.outcome, Outcome::Published(_)) {
        return Err(PartRefusal::Failed(format!(
            "the fuse did not publish ({}){}",
            report.outcome_str(),
            report
                .failure_message()
                .map(|m| format!(": {m}"))
                .unwrap_or_default()
        )));
    }
    let survivors: Vec<BodyId> = rt
        .head_body_ids()
        .into_iter()
        .filter(|b| !dropped.contains(b))
        .collect();
    match survivors.as_slice() {
        [one] => Ok(*one),
        many => Err(PartRefusal::Refused(disjoint_message(
            many.len() as u32,
            kept,
        ))),
    }
}

fn disjoint_message(solids: u32, kept: &[&SolidRow]) -> String {
    let names: Vec<String> = kept
        .iter()
        .map(|s| format!("{}:{}", s.index, s.name))
        .collect();
    format!(
        "{DISJOINT_REFUSAL}: {solids} solids remain; kept=[{}]",
        names.join(", ")
    )
}

/// Alongside the authoring bake's scratch (same process-scoped temp root, same
/// "worker hand-off material, not user data" status).
fn scratch_dir(id: &str) -> Result<PathBuf, PartRefusal> {
    let dir =
        std::env::temp_dir().join(format!("onecad-ingest-{}-{}", std::process::id(), slug(id)));
    std::fs::create_dir_all(&dir)
        .map_err(|e| PartRefusal::Failed(format!("ingest scratch dir: {e}")))?;
    Ok(dir)
}

fn write_package(
    library_root: &Path,
    part: &IngestPart,
    kind: BlobComponentKind,
    geometry: Vec<u8>,
    geometry_codec: String,
    geometry_format: Option<u32>,
    parameters: BTreeMap<String, ParameterSpec>,
) -> Result<(), PartRefusal> {
    let attachments = match &part.attachment {
        None => BTreeMap::new(),
        Some(a) => {
            let frame = onecad_library::package::AttachmentFrame {
                origin: a.origin,
                z: a.z,
                x: a.x,
            }
            .orthonormalized()
            .ok_or_else(|| {
                PartRefusal::Failed(format!(
                    "part `{}`: attachment frame is degenerate — `z` and `x` must be \
                     non-zero, finite, and not parallel",
                    part.id
                ))
            })?;
            BTreeMap::from([(
                "base".to_string(),
                AttachmentSpec {
                    on: "body".to_string(),
                    accepts: a.accepts.clone(),
                    frame: Some(frame),
                },
            )])
        }
    };
    let package = ComponentPackage {
        identity: Identity {
            id: part.id.clone(),
            version: part.version.clone(),
            // Recomputed by the save over what actually lands on disk.
            revision: format!("sha256:{}", "0".repeat(64)),
        },
        metadata: Metadata {
            name: part.name.clone(),
            standard: part.standard.clone(),
            designation: part.designation.clone(),
            category: part.category.clone(),
            tags: part.tags.clone(),
            unit: "mm".to_string(),
        },
        // Overwritten by the save with the blob source it produces.
        source: SourceSpec::Generator {
            generator: String::new(),
            generator_version: 0,
        },
        parameters,
        attachments,
    };
    let mut library = Library::open(library_root)
        .map_err(|e| PartRefusal::Failed(format!("open library: {e}")))?;
    library
        .save_embedded_component(EmbeddedComponentRequest {
            package,
            kind,
            geometry,
            geometry_codec,
            geometry_format,
            preview_png: None,
        })
        .map(|_| ())
        // A taken `id@version` is the library's own policy answer, not a crash.
        .map_err(|e| PartRefusal::Refused(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECIPE: &str = r#"
[defaults]
vendor = "acme"
category = ["imported"]
version = "2.1.0"
tags = ["vendor"]

[[part]]
file = "sticks/rp4030.step"
kind = "profile"
id = "onecad.vendor.rollco.rp4030"
name = "Rollco RP4030 20x20"
standard = "Rollco RP"
designation = "RP4030 20x20 L={length}"
category = ["profiles", "aluminium"]
tags = ["20x20"]
[part.profile]
length_default = 500.0
length_min = 1.0

[[part]]
file = "nema17.step"
id = "onecad.vendor.generic.nema17"
[part.keep]
mode = "indices"
indices = [0, 2]
drop_names = ["*ISO 7045*"]
"#;

    #[test]
    fn recipe_defaults_fall_through_to_a_part_that_declares_none() {
        let plan = parse_recipe(RECIPE, Path::new("/steps")).expect("parse");
        let nema = &plan.parts[1];
        assert_eq!(nema.version, "2.1.0", "version falls back to [defaults]");
        assert_eq!(nema.category, vec!["imported".to_string()]);
        assert_eq!(nema.tags, vec!["vendor".to_string()]);
        assert_eq!(nema.kind, IngestKind::Embedded, "kind defaults to embedded");
        assert_eq!(nema.name, nema.id, "name defaults to the id");
        assert_eq!(nema.file, Path::new("/steps/nema17.step"));
        assert_eq!(nema.keep.mode, KeepMode::Indices(vec![0, 2]));
        assert_eq!(nema.keep.drop_names, vec!["*ISO 7045*".to_string()]);
    }

    #[test]
    fn an_explicit_part_overrides_every_default_and_keeps_its_designation_verbatim() {
        let plan = parse_recipe(RECIPE, Path::new("/steps")).expect("parse");
        let stick = &plan.parts[0];
        assert_eq!(stick.kind, IngestKind::Profile);
        assert_eq!(stick.category, vec!["profiles", "aluminium"]);
        assert_eq!(
            stick.designation.as_deref(),
            Some("RP4030 20x20 L={length}"),
            "the placeholder is the configurator's, not the ingest's"
        );
        assert_eq!(stick.profile.length_default, 500.0);
        assert_eq!(stick.profile.length_min, Some(1.0));
        assert_eq!(stick.keep.mode, KeepMode::All, "no [part.keep] ⇒ keep all");
    }

    #[test]
    fn a_malformed_id_is_refused_at_parse_time() {
        for bad in [
            "NoCaps.thing",
            "single",
            "trailing.",
            "under_score.thing",
            "space thing.x",
        ] {
            let toml = format!("[[part]]\nfile = \"a.step\"\nid = \"{bad}\"\n");
            assert!(
                parse_recipe(&toml, Path::new("/x")).is_err(),
                "id {bad:?} must be refused"
            );
        }
        assert!(parse_recipe(
            "[[part]]\nfile = \"a.step\"\nid = \"onecad.vendor.rollco.rp4030-5\"\n",
            Path::new("/x")
        )
        .is_ok());
    }

    #[test]
    fn an_unknown_kind_or_keep_mode_is_refused_rather_than_defaulted() {
        let bad_kind = "[[part]]\nfile = \"a.step\"\nid = \"a.b\"\nkind = \"solid\"\n";
        let err = parse_recipe(bad_kind, Path::new("/x")).expect_err("unknown kind");
        assert!(err.contains("unknown kind"), "{err}");

        let bad_mode =
            "[[part]]\nfile = \"a.step\"\nid = \"a.b\"\n[part.keep]\nmode = \"biggest\"\n";
        let err = parse_recipe(bad_mode, Path::new("/x")).expect_err("unknown keep.mode");
        assert!(err.contains("unknown keep.mode"), "{err}");

        let no_indices =
            "[[part]]\nfile = \"a.step\"\nid = \"a.b\"\n[part.keep]\nmode = \"indices\"\n";
        assert!(parse_recipe(no_indices, Path::new("/x")).is_err());
    }

    fn row(index: usize, name: &str, volume: f64) -> SolidRow {
        SolidRow {
            index,
            body: BodyId::new(),
            name: name.to_string(),
            volume_mm3: volume,
            face_count: 6,
            solid_count: 1,
            bbox: None,
        }
    }

    #[test]
    fn drop_names_glob_runs_before_the_mode_and_is_case_insensitive() {
        let solids = vec![
            row(0, "Housing", 1000.0),
            row(1, "ISO 7045 - M2 x 8", 5.0),
            row(2, "iso 7045 - M2 x 12", 5.0),
            row(3, "Output gear", 20.0),
        ];
        let keep = KeepSpec {
            mode: KeepMode::All,
            drop_names: vec!["*ISO 7045*".to_string()],
        };
        let kept = select_kept(&solids, &keep);
        assert_eq!(
            kept.iter().map(|s| s.index).collect::<Vec<_>>(),
            vec![0, 3],
            "both screws drop regardless of case"
        );
    }

    #[test]
    fn volume_fraction_keeps_only_the_solids_above_the_threshold() {
        let solids = vec![
            row(0, "Case", 900.0),
            row(1, "Shaft", 90.0),
            row(2, "Washer", 10.0),
        ];
        let kept = select_kept(
            &solids,
            &KeepSpec {
                mode: KeepMode::VolumeFraction(0.05),
                drop_names: Vec::new(),
            },
        );
        assert_eq!(kept.iter().map(|s| s.index).collect::<Vec<_>>(), vec![0, 1]);
    }

    #[test]
    fn a_defaults_only_plan_slugs_the_file_stem_into_the_id() {
        let plan = plan_from_paths(
            &["/vendor/CAD_RP4030_5 20x20 (Normal).step".to_string()],
            &IngestDefaults {
                vendor: "Rollco AB".to_string(),
                ..Default::default()
            },
        );
        assert_eq!(
            plan.parts[0].id,
            "onecad.vendor.rollco-ab.cad-rp4030-5-20x20-normal"
        );
        assert!(validate_component_id(&plan.parts[0].id).is_ok());
        assert_eq!(plan.parts[0].kind, IngestKind::Embedded);
        assert_eq!(
            plan.parts[0].keep.drop_names.len(),
            DEFAULT_DROP_NAMES.len()
        );
    }

    /// The one nested object `ingest_components` receives over the wire. `paths`
    /// and `libraryRoot` are FLAT command arguments (tauri camelCases arg names),
    /// so this is the whole serde surface of the request.
    #[test]
    fn the_frontend_defaults_payload_deserializes() {
        let full: crate::dto::IngestDefaultsDto = serde_json::from_str(
            r#"{"vendor":"rollco","category":["profiles"],"tags":["t-slot"]}"#,
        )
        .expect("the shape tauriClient.ingestComponents sends");
        assert_eq!(full.vendor, "rollco");
        assert_eq!(full.category, vec!["profiles".to_string()]);

        // `tags` is optional on the TS side (`tags?: string[]`).
        let lean: crate::dto::IngestDefaultsDto =
            serde_json::from_str(r#"{"vendor":"acme","category":[]}"#).expect("tags omitted");
        assert!(lean.tags.is_empty());
    }

    #[test]
    fn glob_match_anchors_when_the_pattern_does() {
        assert!(glob_match("*ISO 7045*", "Part ISO 7045 screw"));
        assert!(!glob_match("*ISO 7045*", "Part ISO 4762 screw"));
        assert!(glob_match("housing*", "Housing left"));
        assert!(!glob_match("housing", "Housing left"));
        assert!(glob_match("*left", "Housing left"));
        assert!(glob_match("*", "anything"));
    }
}
