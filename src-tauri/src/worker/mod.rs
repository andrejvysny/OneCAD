//! The geometry-backend seam the app layer drives regen against.
//!
//! R-WP10 owns **no worker lifecycle** (that is R-WP11). It codes against the
//! [`GeometryEngine`] trait (core-owned, transport-agnostic) plus a small
//! [`MeshProvider`] seam for the bulk MESH1 bytes the core trait does not carry.
//! The two together are a [`Backend`]:
//!
//! * `FakeEngine` / `onecad-worker-stub` implement [`Backend`] in tests;
//! * R-WP11's `WorkerManager` (`tokio::process` + `ProtocolClient`) implements it
//!   over the real C++ sidecar and slots into [`AppState`](crate::state::AppState)
//!   with zero changes here (that is the seam).
//!
//! [`AdoptingEngine`] wraps any [`GeometryEngine`] to enforce **D1** (the
//! approved cross-track decision): NewBody `BodyId`s are worker-minted
//! deterministic `body_<opId>`; Rust adopts them from `planStep` `bodyEvents` and
//! rejects a prepared plan on malformation / collision (see [`validate_created`]).
//!
//! [`AppState`]: crate::state::AppState

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use uuid::Uuid;

use onecad_core::document::body::BodyLifecycleEvent;
use onecad_core::document::record::Operation;
use onecad_core::ids::{BodyId, DocumentId, JobId, SnapshotId, WorkerEpoch};
use onecad_core::regen::{
    AcceptResult, AcquireRequest, CheckpointArtifacts, EngineError, Fencing, GeometryEngine, Lod,
    OpenSessionRequest, PlanEvent, PlanRequest, RefResolution, ResolveRequest, RestoreRequest,
    RestoreResult, TessellateRequest, TessellateResult, WorkerElementEvidence, WorkerHead,
};

pub mod manager;
pub mod wire;

pub use manager::{RestartHook, SupervisorConfig, WorkerLifecycle, WorkerManager, WorkerState};

/// The default dev-tree worker binary, relative to `src-tauri/`.
pub const DEV_WORKER_PATH: &str = "../worker/build/onecad-worker";

/// The `ONECAD_WORKER_PATH` override env var (highest precedence).
pub const WORKER_PATH_ENV: &str = "ONECAD_WORKER_PATH";

/// The sidecar binary's basename as Tauri drops it beside the main executable in
/// a bundled app (`externalBin` strips the target triple at install time).
pub const BUNDLED_WORKER_NAME: &str = "onecad-worker";

/// Resolves the worker binary path (SCHEMA-agnostic packaging seam) over a fixed
/// precedence chain:
///
/// 1. `ONECAD_WORKER_PATH` override — if it names a file that exists;
/// 2. **release builds**: `<exe_dir>/onecad-worker` — where Tauri places the
///    `externalBin` sidecar in a bundled app — then the dev fallback;
/// 3. **debug builds**: the dev fallback `../worker/build/onecad-worker` FIRST,
///    then `<exe_dir>/onecad-worker`. Tauri dev copies the staged
///    `src-tauri/binaries/` sidecar beside the debug executable, and that staged
///    copy drifts stale silently (it is only refreshed by
///    `scripts/build-worker.sh`) — the dev-tree build is the source of truth.
///
/// Returns `None` when no candidate exists on disk, so the app keeps the
/// [`PendingBackend`] fallback rather than spawning a missing binary.
#[must_use]
pub fn resolve_worker_path() -> Option<PathBuf> {
    let env_override = std::env::var_os(WORKER_PATH_ENV).map(PathBuf::from);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    resolve_worker_path_from(
        env_override,
        exe_dir,
        Path::new(DEV_WORKER_PATH),
        cfg!(debug_assertions),
    )
}

/// The pure resolution core behind [`resolve_worker_path`], factored out so the
/// precedence chain is unit-testable without touching the process-global
/// environment or the real executable location.
///
/// `env_override` always wins when it exists. `prefer_dev` (debug builds) swaps
/// the remaining two rungs: `dev_fallback` before `<exe_dir>/onecad-worker`
/// (with a `.exe` suffix on Windows); release order is bundled-then-dev.
fn resolve_worker_path_from(
    env_override: Option<PathBuf>,
    exe_dir: Option<PathBuf>,
    dev_fallback: &Path,
    prefer_dev: bool,
) -> Option<PathBuf> {
    if let Some(path) = env_override {
        if path.exists() {
            return Some(path);
        }
    }
    let bundled = exe_dir.map(|dir| {
        let name = if cfg!(windows) {
            "onecad-worker.exe"
        } else {
            BUNDLED_WORKER_NAME
        };
        dir.join(name)
    });
    let bundled = bundled.filter(|p| p.exists());
    let dev = dev_fallback.exists().then(|| dev_fallback.to_path_buf());

    if prefer_dev {
        if let Some(dev) = dev {
            // Debug-only observability: a staged sidecar beside the exe is being
            // shadowed by the dev-tree build — say so, or a stale staged copy
            // "works" invisibly the day this order changes.
            if let Some(shadowed) = &bundled {
                tracing::warn!(
                    dev = %dev.display(),
                    shadowed = %shadowed.display(),
                    "worker resolve: dev-tree build preferred over staged sidecar (debug build)"
                );
            }
            return Some(dev);
        }
        return bundled;
    }
    bundled.or(dev)
}

// ─────────────────────────────────────────────────────────────────────────────
// Mesh bytes seam
// ─────────────────────────────────────────────────────────────────────────────

/// Supplies the raw MESH1 bytes for a body/LOD in a published snapshot.
///
/// The core [`GeometryEngine::tessellate`] returns only mesh *handles* (identity
/// + integrity); the bytes stream on the bulk lane and are assembled by the
/// transport (R-WP11's `WorkerManager`). This seam lets the app-layer mesh cache
/// pull the assembled blob without the core trait carrying bulk payloads.
#[async_trait]
pub trait MeshProvider: Send + Sync {
    /// Fetches the MESH1 blob for `body` at `lod` in `snapshot`.
    async fn fetch_mesh(
        &self,
        body: BodyId,
        lod: Lod,
        snapshot: SnapshotId,
    ) -> Result<Vec<u8>, EngineError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Solver-lane seam (SCHEMA §7.4) — separate from GeometryEngine
// ─────────────────────────────────────────────────────────────────────────────

/// The sketch **solver lane** (SCHEMA §7.4) — a dedicated seam distinct from the
/// OCCT-lane [`GeometryEngine`] because the worker runs PlaneGCS on a separate
/// thread/actor: drags must **never queue behind** an `ExecutePlan` (plan "Solver
/// lane in V1"). The transport ([`ProtocolClient`](onecad_protocol::client::ProtocolClient))
/// already multiplexes concurrent in-flight requests, so a `SolveDrag` frame goes
/// out and resolves while a plan is mid-flight.
///
/// **Latest-wins** is a client-side contract: the caller fires the newest
/// `SolveDrag` (monotonic `seq`) without awaiting each serially and tolerates a
/// `superseded`/`CANCELLED` terminal for a stale `seq` (SCHEMA §7.4) — it simply
/// drops that response's positions.
#[async_trait]
pub trait SolverEngine: Send + Sync {
    /// `SketchUpsert` (SCHEMA §7.4) — sync the authoritative sketch + report dof/state.
    async fn sketch_upsert(
        &self,
        sketch: &onecad_core::sketch::Sketch,
    ) -> Result<crate::dto::SketchUpsertDto, EngineError>;

    /// `BeginGesture` (SCHEMA §7.4) — open a drag gesture on a point.
    async fn begin_gesture(
        &self,
        sketch_id: &str,
        sketch_revision: u64,
        gesture_id: u64,
        drag_point: onecad_core::ids::EntityId,
        solver_policy_hash: &str,
    ) -> Result<crate::dto::BeginGestureDto, EngineError>;

    /// `SolveDrag` (SCHEMA §7.4) — one latest-wins incremental solve.
    async fn solve_drag(
        &self,
        gesture_id: u64,
        seq: u64,
        drag_point: onecad_core::ids::EntityId,
        target: [f64; 2],
    ) -> Result<crate::dto::DragSolveDto, EngineError>;

    /// `EndGesture` (SCHEMA §7.4) — pointer-up final exact solve; carries the
    /// changed positions the caller applies as one undo command.
    async fn end_gesture(
        &self,
        sketch_id: &str,
        gesture_id: u64,
        final_target: Option<[f64; 2]>,
    ) -> Result<crate::dto::SketchUpsertDto, EngineError>;

    /// `SketchRegions` (SCHEMA §7.4) — closed profile regions for extrude/preview.
    async fn sketch_regions(
        &self,
        sketch_id: &str,
    ) -> Result<Vec<crate::dto::SketchRegionDto>, EngineError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Face-geometry seam (SCHEMA §7.5 `QueryElement`)
// ─────────────────────────────────────────────────────────────────────────────

/// Reads a bound element's geometric evidence out of a published snapshot.
///
/// A separate seam rather than a [`GeometryEngine`] method for the same reason
/// [`MeshProvider`] is: the core trait models the REGEN contract (plan → snapshot),
/// and this is a read-only side query that neither mutates nor fences anything.
/// The verb itself (`QueryElement`) has existed on the worker since W-WP6 but had
/// no Rust caller at all.
///
/// MODEL-OPS W2 uses it to derive a sketch plane from a picked face: the element
/// descriptor carries `surfaceType`, `center` and `normal`
/// (`worker/src/elementmap/ElementMapPartition.cpp descriptor_to_json`), and for a
/// PLANAR face the bounding-box centre lies on the plane, so `{center, normal}`
/// defines it exactly.
#[async_trait]
pub trait ElementQuery: Send + Sync {
    /// `QueryElement` **by ElementId** (SCHEMA §7.5) — the element's current
    /// binding + descriptor. `Ok(None)` when the element is not present.
    ///
    /// IMPORTANT — what "not present" means here. The worker's element-map
    /// partition mints entries **on demand**, and the only thing that mints one
    /// is an OP resolving it as an input (`PlanExecutor resolve_input_refs`).
    /// `AcquireElementIds` returns *evidence*; RUST mints the id and the worker
    /// never hears about it. So an id that has been promoted but not yet CONSUMED
    /// by an operation is legitimately absent from the partition, and this lookup
    /// returns `None` for it. Use [`ElementQuery::query_element_by_topo_key`] for
    /// a fresh pick — the two together are the read ladder (see `api::element_info`).
    async fn query_element(
        &self,
        snapshot: SnapshotId,
        body: BodyId,
        element: &str,
    ) -> Result<Option<crate::dto::ElementInfoDto>, EngineError>;

    /// `QueryElement` **by `{topoKey, bodyId}`** — the SCHEMA §7.5 second form.
    ///
    /// A `TopoKey` is snapshot-scoped ordinal evidence rather than a durable
    /// identity, so this is only sound when the caller passes the snapshot its
    /// pick was made against (Invariant 4). It resolves against the body shape
    /// directly instead of the partition, which is exactly why it answers for a
    /// just-picked element that no operation has referenced yet.
    async fn query_element_by_topo_key(
        &self,
        snapshot: SnapshotId,
        body: BodyId,
        topo_key: &str,
    ) -> Result<Option<crate::dto::ElementInfoDto>, EngineError>;

    /// `QueryMassProperties` (SCHEMA §7.5; WP-C1) — exact `GProp` volume, surface
    /// area, centroid and principal inertia frame for ONE body.
    ///
    /// On this trait rather than a ninth backend facet because SCHEMA files it
    /// under §7.5 beside `QueryElement`, and it is the same KIND of call: a
    /// read-only, non-fencing, non-minting lookup against a copy of the worker
    /// head, served by the same `WorkerManager`. The only difference is what it
    /// addresses — a whole body instead of one of its sub-shapes.
    ///
    /// Unlike the two lookups above there is no `Option`: an unknown body is a
    /// caller mistake and comes back as a loud `REF_UNRESOLVED` error. "That
    /// element is gone" is a real answer for a snapshot-scoped pick; "that body
    /// is gone" has no partial reading to report, and answering `None` would let
    /// a UI silently show nothing where it should show a failure.
    async fn query_mass_properties(
        &self,
        body: BodyId,
        body_id_label: String,
    ) -> Result<crate::dto::MassPropertiesDto, EngineError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Host-face boundary projection seam (SCHEMA §7.6 `ProjectFaceBoundary`)
// ─────────────────────────────────────────────────────────────────────────────

/// Projects a picked planar face's boundary into a sketch plane's UV.
///
/// A separate seam beside [`ElementQuery`] for the same reason: this is a
/// **read-only side query** that neither mutates nor fences anything (no
/// prepare/accept/discard, no element-map minting), so it does not belong on the
/// regen-shaped [`GeometryEngine`] trait. Like the §7.5 identity verbs it reads a
/// copy of the head, so a stale or absent reference answers `Ok(None)` —
/// "that face is gone" is an ANSWER, not an error.
///
/// # The two-round-trip handshake
///
/// The plane is INPUT and AUTHORITATIVE (SCHEMA §7.6): Rust owns the basis and
/// the worker expresses every coordinate in it. But Rust cannot build a basis
/// before it knows a point that is genuinely ON the face plane — an element
/// descriptor's `center` is an axis-aligned **bbox** centre and sits OFF-plane
/// for a tilted face, which would extrude a sliver. So:
///
/// 1. [`project_face_boundary_frame`](FaceBoundaryProjection::project_face_boundary_frame)
///    (`frameOnly`) returns the kernel-exact `gp_Pln` origin + orientation-corrected
///    normal;
/// 2. Rust builds the deterministic, lock-tested basis
///    ([`plane_from_point_normal`](onecad_core::sketch::plane_from_point_normal));
/// 3. [`project_face_boundary`](FaceBoundaryProjection::project_face_boundary)
///    runs the real projection in THAT basis and echoes `exact` back as a
///    tripwire the caller compares.
///
/// Callers MUST NOT hold the `DocumentRuntime` lock across either call — both are
/// worker round-trips, and holding the single writer across worker IO is the
/// anti-pattern R-WP11 fixed for regen.
#[async_trait]
pub trait FaceBoundaryProjection: Send + Sync {
    /// `ProjectFaceBoundary` with `frameOnly:true` (SCHEMA §7.6) — round-trip 1.
    /// `Ok(None)` when the reference does not resolve in the current head.
    async fn project_face_boundary_frame(
        &self,
        snapshot: SnapshotId,
        body: BodyId,
        address: wire::FaceAddress<'_>,
    ) -> Result<Option<onecad_core::sketch::FaceFrame>, EngineError>;

    /// `ProjectFaceBoundary` with the caller's authoritative `plane` — round-trip 2.
    /// `Ok(None)` when the reference does not resolve in the current head.
    async fn project_face_boundary(
        &self,
        snapshot: SnapshotId,
        body: BodyId,
        address: wire::FaceAddress<'_>,
        plane: &onecad_core::sketch::SketchPlane,
        scope: wire::ProjectionScope,
    ) -> Result<Option<onecad_core::sketch::ProjectionPayload>, EngineError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// STEP import preflight seam (SCHEMA §7.8 `InspectStep`)
// ─────────────────────────────────────────────────────────────────────────────

/// One `InspectStep` result (SCHEMA §7.8) — a read-only probe of a STEP file.
///
/// `geometry_bytes` is `Some` only when the caller asked for the conversion lane
/// (`include_geometry`): the healed, mm-normalized, **UNSCALED** result serialized
/// in `geometry_codec`, whose solid order IS the §7.3 ordinal order.
/// `geometry_codec` + `geometry_format` name that byte form and pin its binary
/// format version (both reported unconditionally, so a caller that defers fetching
/// the bytes still learns what to pin).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepInspection {
    /// Number of solids the heal pipeline recovered (= the number of bodies the
    /// §7.3 `ImportStep` op will mint).
    pub solid_count: usize,
    /// The file's declared length unit (`"MM"`, `"INCH"`, …), display-only.
    pub source_unit: String,
    /// Axis-aligned bounds over every solid, `(min, max)` in mm.
    pub bbox: ([f64; 3], [f64; 3]),
    /// STEP product names, ONE PER ORDINAL SOLID where recoverable (an entry is
    /// empty when the file carried no name for that solid). A length that differs
    /// from the minted body count makes the whole list unusable, so the worker
    /// pads rather than truncates — a name is best-effort evidence, never identity.
    pub product_names: Vec<String>,
    /// The codec `geometry_bytes` is serialized in, and the value to author into
    /// `ImportStepParams::source_codec` (SCHEMA §7.8 `geometryCodec`; `"xbf"`
    /// today). Empty when the worker reported none.
    pub geometry_codec: String,
    /// The binary format version of `geometry_bytes` (and the value to pin in
    /// `ImportStepParams::brep_format`).
    pub geometry_format: u32,
    /// Advisory findings from the read (`STEP_SEWN`, `STEP_HEALED`, …) as
    /// `(code, message)`. The probe itself succeeded — a failure is an error.
    pub diagnostics: Vec<(String, String)>,
    /// The healed geometry bytes in `geometry_codec`, when `include_geometry` was
    /// requested.
    pub geometry_bytes: Option<Vec<u8>>,
}

/// Probes a STEP file without touching any session state (SCHEMA §7.8
/// `InspectStep`).
///
/// A separate seam beside [`FaceBoundaryProjection`] for the same reason: the
/// probe neither fences, prepares, nor publishes anything, so it does not belong on
/// the regen-shaped [`GeometryEngine`] trait. It is the **conversion lane** for the
/// converted-primary replay policy (W0 decision): Rust probes the user's file once
/// at import-command time, persists the returned geometry bytes alongside the STEP
/// source in the container, and authors the `ImportStep` record against them — so
/// replay never re-parses STEP.
///
/// A malformed file comes back as a recoverable `OP_FAILED`-class [`EngineError`],
/// never a protocol tear-down (SCHEMA §7.8). Callers MUST NOT hold the
/// `DocumentRuntime` lock across this call — it is a worker round-trip, and holding
/// the single writer across worker IO is the anti-pattern R-WP11 fixed for regen.
#[async_trait]
pub trait StepImport: Send + Sync {
    /// `InspectStep` (SCHEMA §7.8). `path` is a Rust-owned path the worker reads;
    /// `include_geometry` additionally requests the healed replay bytes in the
    /// response's `geometry` bin section.
    ///
    /// # Errors
    /// [`EngineError`] on a disconnected worker, a malformed response, or an
    /// unreadable / invalid STEP file (recoverable `OpFailed`).
    async fn inspect_step(
        &self,
        path: &Path,
        include_geometry: bool,
    ) -> Result<StepInspection, EngineError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Drag-time preview seam (SCHEMA §7.6 `PreviewOp`)
// ─────────────────────────────────────────────────────────────────────────────

/// Runs ONE candidate op against a throwaway copy of the worker's head and
/// returns the resulting MESH1 blobs.
///
/// Deliberately NOT a [`GeometryEngine`] method: the core trait models the regen
/// contract (plan → prepared snapshot → accept), and a preview participates in
/// none of it — it never fences, never prepares, never commits, and is invisible
/// to the head hash/snapshot/epoch (see `worker/src/session/PreviewOp.h`).
///
/// Callers MUST NOT hold the `DocumentRuntime` lock across this call: it is a
/// worker round-trip on the kernel lane, and holding the single writer across
/// worker IO is the anti-pattern R-WP11 fixed for regen.
#[async_trait]
pub trait PreviewEngine: Send + Sync {
    /// `PreviewOp` (SCHEMA §7.6). `operation` is the typed core candidate; the
    /// implementation lowers it through the same wire mapper as ExecutePlan.
    /// `sketch_id` names the profile sketch to seed from the committed store.
    async fn preview_op(
        &self,
        operation: Operation,
        op_id: String,
        sketch_id: Option<String>,
        expected_snapshot: Option<SnapshotId>,
        lod: Lod,
    ) -> Result<crate::dto::PreviewResultDto, EngineError>;
}

/// The full geometry backend: a [`GeometryEngine`] plus its [`MeshProvider`].
/// Blanket-implemented, so any type that is both is a `Backend`.
pub trait Backend: GeometryEngine + MeshProvider {}
impl<T: GeometryEngine + MeshProvider> Backend for T {}

/// The wire string for a [`Lod`] (`"coarse"`/`"medium"`/`"fine"`; SCHEMA §7.6).
#[must_use]
pub fn lod_str(lod: Lod) -> &'static str {
    match lod {
        Lod::Coarse => "coarse",
        Lod::Medium => "medium",
        Lod::Fine => "fine",
    }
}

/// Parses a wire LOD string; unknown ⇒ `Coarse` (the safe default tier).
#[must_use]
pub fn lod_from_str(s: &str) -> Lod {
    match s {
        "medium" => Lod::Medium,
        "fine" => Lod::Fine,
        _ => Lod::Coarse,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// D1 — worker-minted BodyId adoption
// ─────────────────────────────────────────────────────────────────────────────

/// Validates one step's `created` body events against the D1 adoption rule.
///
/// A NewBody id is worker-minted deterministic `body_<opId>`; a boolean SPLIT mints
/// `body_<opId>:<k>` (M5a). Adoption accepts a `created` body iff:
///
/// * **NewBody** (`body_<opId>`) — `body.as_uuid() ∈ known_ops` (the string maps to
///   `BodyId(opId.uuid)` at the wire boundary);
/// * **split child** (`body_<opId>:<k>`) — the id parses (via the [`wire`]
///   interner) to a `(opId, k)` whose `opId ∈ known_ops`, and the op's ordinals are
///   **contiguous from 0** across the step (the worker emits all of one op's children
///   in one step);
/// * and in both cases it is **unique** — neither an already-present session body
///   (`existing`) nor a duplicate `created` id in this plan (`seen`).
///
/// A malformed / non-contiguous / colliding id returns `Err(message)`; the caller
/// rejects the whole prepared plan (never silently adopts).
///
/// # Errors
/// A human-readable reason on malformation/collision/non-contiguity (surfaced as
/// `PROTOCOL_ERROR`).
pub fn validate_created(
    events: &[BodyLifecycleEvent],
    known_ops: &HashSet<Uuid>,
    existing: &HashSet<BodyId>,
    seen: &mut HashSet<BodyId>,
) -> Result<(), String> {
    // Split-child ordinals seen in THIS step's events, grouped by producing op — a
    // step emits all of one op's children together, so `k`-contiguity is per-step.
    let mut split_ks: std::collections::HashMap<Uuid, Vec<usize>> =
        std::collections::HashMap::new();
    for ev in events {
        let BodyLifecycleEvent::Created { body } = ev else {
            continue;
        };
        // Uniqueness first (both id forms share the `BodyId` uuid space).
        if existing.contains(body) || !seen.insert(*body) {
            return Err(format!(
                "worker-minted id {body} collides with an existing/duplicate body (D1)"
            ));
        }
        match crate::worker::wire::split_parts(*body) {
            Some((op, k)) => {
                if !known_ops.contains(&op) {
                    return Err(format!(
                        "split child (op {op}, k={k}) does not match any known opId (D1 malformation)"
                    ));
                }
                split_ks.entry(op).or_default().push(k);
            }
            None => {
                if !known_ops.contains(&body.as_uuid()) {
                    return Err(format!(
                        "worker-minted NewBody id {body} does not match any known opId (D1 malformation)"
                    ));
                }
            }
        }
    }
    // Split ordinals must be contiguous 0..n per op (no gaps, no holes).
    for (op, mut ks) in split_ks {
        ks.sort_unstable();
        if ks.iter().enumerate().any(|(i, &k)| i != k) {
            return Err(format!(
                "split children of op {op} are not contiguous from 0 (got {ks:?}, D1)"
            ));
        }
    }
    Ok(())
}

/// Wraps a [`GeometryEngine`] to enforce D1 body-id adoption on the `execute_plan`
/// stream. Every other verb delegates unchanged to the inner engine.
///
/// On a malformed / colliding `created` id the wrapper converts the terminal
/// `PlanPrepared` into a `PlanEvent::Failed(PROTOCOL_ERROR)`, so the executor
/// **discards** the scratch job (rejecting the prepared plan) rather than
/// publishing worker-minted ids Rust cannot adopt.
pub struct AdoptingEngine {
    inner: Arc<dyn GeometryEngine>,
    known_ops: HashSet<Uuid>,
    existing: HashSet<BodyId>,
}

impl AdoptingEngine {
    /// Wraps `inner`, validating `created` ids against the plan's `known_ops`
    /// (op record-id UUIDs) and the scratch base's `existing` bodies.
    #[must_use]
    pub fn new(
        inner: Arc<dyn GeometryEngine>,
        known_ops: HashSet<Uuid>,
        existing: HashSet<BodyId>,
    ) -> Self {
        Self {
            inner,
            known_ops,
            existing,
        }
    }
}

#[async_trait]
impl GeometryEngine for AdoptingEngine {
    async fn execute_plan(&self, request: PlanRequest) -> mpsc::Receiver<PlanEvent> {
        let mut inner_rx = self.inner.execute_plan(request).await;
        let (tx, rx) = mpsc::channel(256);
        let known = self.known_ops.clone();
        let existing = self.existing.clone();
        tokio::spawn(async move {
            let mut seen: HashSet<BodyId> = HashSet::new();
            let mut violation: Option<String> = None;
            while let Some(ev) = inner_rx.recv().await {
                // Validate `created` ids on each step until a violation is latched.
                if violation.is_none() {
                    if let PlanEvent::Step(step) = &ev {
                        if let Err(msg) =
                            validate_created(&step.body_events, &known, &existing, &mut seen)
                        {
                            violation = Some(msg);
                        }
                    }
                }
                // Reject a prepared plan that violated adoption: the executor
                // discards the scratch instead of publishing un-adoptable ids.
                if matches!(ev, PlanEvent::Prepared(_)) {
                    if let Some(msg) = violation.take() {
                        let _ = tx
                            .send(PlanEvent::Failed(EngineError::Protocol { message: msg }))
                            .await;
                        return;
                    }
                }
                if tx.send(ev).await.is_err() {
                    return;
                }
            }
        });
        rx
    }

    async fn open_session(&self, req: OpenSessionRequest) -> Result<WorkerHead, EngineError> {
        self.inner.open_session(req).await
    }
    async fn close_session(
        &self,
        document_id: DocumentId,
        worker_epoch: WorkerEpoch,
    ) -> Result<(), EngineError> {
        self.inner.close_session(document_id, worker_epoch).await
    }
    async fn reset(
        &self,
        document_id: DocumentId,
        worker_epoch: WorkerEpoch,
    ) -> Result<WorkerEpoch, EngineError> {
        self.inner.reset(document_id, worker_epoch).await
    }
    async fn accept_prepared(
        &self,
        job_id: JobId,
        fencing: Fencing,
    ) -> Result<AcceptResult, EngineError> {
        self.inner.accept_prepared(job_id, fencing).await
    }
    async fn discard_prepared(&self, job_id: JobId) -> Result<(), EngineError> {
        self.inner.discard_prepared(job_id).await
    }
    async fn get_worker_head(&self) -> Result<WorkerHead, EngineError> {
        self.inner.get_worker_head().await
    }
    async fn tessellate(&self, req: TessellateRequest) -> Result<TessellateResult, EngineError> {
        self.inner.tessellate(req).await
    }
    async fn save_checkpoint(&self, step_index: usize) -> Result<CheckpointArtifacts, EngineError> {
        self.inner.save_checkpoint(step_index).await
    }
    async fn restore_checkpoint(&self, req: RestoreRequest) -> Result<RestoreResult, EngineError> {
        self.inner.restore_checkpoint(req).await
    }
    async fn acquire_element_ids(
        &self,
        req: AcquireRequest,
    ) -> Result<Vec<WorkerElementEvidence>, EngineError> {
        self.inner.acquire_element_ids(req).await
    }
    async fn resolve_refs(&self, req: ResolveRequest) -> Result<Vec<RefResolution>, EngineError> {
        self.inner.resolve_refs(req).await
    }
    async fn cancel(&self, job_id: JobId) -> Result<(), EngineError> {
        self.inner.cancel(job_id).await
    }
    async fn ping(&self) -> Result<(), EngineError> {
        self.inner.ping().await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Placeholder backend (production boot before R-WP11 wires the real worker)
// ─────────────────────────────────────────────────────────────────────────────

/// A [`Backend`] that fails every geometry call, so the app boots and the webview
/// loads before R-WP11 spawns the real worker. Every regen surfaces a
/// `PROTOCOL_ERROR` (recoverable — the session stays editable).
#[derive(Debug, Default)]
pub struct PendingBackend;

impl PendingBackend {
    fn not_ready() -> EngineError {
        EngineError::Protocol {
            message: "worker not started (R-WP11 wires the real sidecar)".into(),
        }
    }
}

#[async_trait]
impl GeometryEngine for PendingBackend {
    async fn execute_plan(&self, _request: PlanRequest) -> mpsc::Receiver<PlanEvent> {
        let (tx, rx) = mpsc::channel(1);
        let _ = tx.send(PlanEvent::Failed(Self::not_ready())).await;
        rx
    }
    async fn open_session(&self, _req: OpenSessionRequest) -> Result<WorkerHead, EngineError> {
        Err(Self::not_ready())
    }
    async fn close_session(&self, _d: DocumentId, _e: WorkerEpoch) -> Result<(), EngineError> {
        Ok(())
    }
    async fn reset(&self, _d: DocumentId, e: WorkerEpoch) -> Result<WorkerEpoch, EngineError> {
        Ok(WorkerEpoch(e.0 + 1))
    }
    async fn accept_prepared(&self, _j: JobId, _f: Fencing) -> Result<AcceptResult, EngineError> {
        Err(Self::not_ready())
    }
    async fn discard_prepared(&self, _j: JobId) -> Result<(), EngineError> {
        Ok(())
    }
    async fn get_worker_head(&self) -> Result<WorkerHead, EngineError> {
        Err(Self::not_ready())
    }
    async fn tessellate(&self, _r: TessellateRequest) -> Result<TessellateResult, EngineError> {
        Err(Self::not_ready())
    }
    async fn save_checkpoint(&self, _s: usize) -> Result<CheckpointArtifacts, EngineError> {
        Err(Self::not_ready())
    }
    async fn restore_checkpoint(&self, _r: RestoreRequest) -> Result<RestoreResult, EngineError> {
        Err(Self::not_ready())
    }
    async fn acquire_element_ids(
        &self,
        _r: AcquireRequest,
    ) -> Result<Vec<WorkerElementEvidence>, EngineError> {
        Err(Self::not_ready())
    }
    async fn resolve_refs(&self, _r: ResolveRequest) -> Result<Vec<RefResolution>, EngineError> {
        Err(Self::not_ready())
    }
    async fn cancel(&self, _j: JobId) -> Result<(), EngineError> {
        Ok(())
    }
    async fn ping(&self) -> Result<(), EngineError> {
        Err(Self::not_ready())
    }
}

#[async_trait]
impl MeshProvider for PendingBackend {
    async fn fetch_mesh(
        &self,
        _body: BodyId,
        _lod: Lod,
        _snapshot: SnapshotId,
    ) -> Result<Vec<u8>, EngineError> {
        Err(Self::not_ready())
    }
}

#[async_trait]
impl PreviewEngine for PendingBackend {
    async fn preview_op(
        &self,
        _operation: Operation,
        _op_id: String,
        _sketch_id: Option<String>,
        _expected_snapshot: Option<SnapshotId>,
        _lod: Lod,
    ) -> Result<crate::dto::PreviewResultDto, EngineError> {
        Err(Self::not_ready())
    }
}

#[async_trait]
impl ElementQuery for PendingBackend {
    async fn query_element(
        &self,
        _snapshot: SnapshotId,
        _body: BodyId,
        _element: &str,
    ) -> Result<Option<crate::dto::ElementInfoDto>, EngineError> {
        Err(Self::not_ready())
    }

    async fn query_element_by_topo_key(
        &self,
        _snapshot: SnapshotId,
        _body: BodyId,
        _topo_key: &str,
    ) -> Result<Option<crate::dto::ElementInfoDto>, EngineError> {
        Err(Self::not_ready())
    }

    async fn query_mass_properties(
        &self,
        _body: BodyId,
        _body_id_label: String,
    ) -> Result<crate::dto::MassPropertiesDto, EngineError> {
        Err(Self::not_ready())
    }
}

#[async_trait]
impl FaceBoundaryProjection for PendingBackend {
    async fn project_face_boundary_frame(
        &self,
        _snapshot: SnapshotId,
        _body: BodyId,
        _address: wire::FaceAddress<'_>,
    ) -> Result<Option<onecad_core::sketch::FaceFrame>, EngineError> {
        Err(Self::not_ready())
    }

    async fn project_face_boundary(
        &self,
        _snapshot: SnapshotId,
        _body: BodyId,
        _address: wire::FaceAddress<'_>,
        _plane: &onecad_core::sketch::SketchPlane,
        _scope: wire::ProjectionScope,
    ) -> Result<Option<onecad_core::sketch::ProjectionPayload>, EngineError> {
        Err(Self::not_ready())
    }
}

#[async_trait]
impl StepImport for PendingBackend {
    async fn inspect_step(
        &self,
        _path: &Path,
        _include_geometry: bool,
    ) -> Result<StepInspection, EngineError> {
        Err(Self::not_ready())
    }
}

#[async_trait]
impl SolverEngine for PendingBackend {
    async fn sketch_upsert(
        &self,
        _sketch: &onecad_core::sketch::Sketch,
    ) -> Result<crate::dto::SketchUpsertDto, EngineError> {
        Err(Self::not_ready())
    }
    async fn begin_gesture(
        &self,
        _sketch_id: &str,
        _sketch_revision: u64,
        _gesture_id: u64,
        _drag_point: onecad_core::ids::EntityId,
        _solver_policy_hash: &str,
    ) -> Result<crate::dto::BeginGestureDto, EngineError> {
        Err(Self::not_ready())
    }
    async fn solve_drag(
        &self,
        _gesture_id: u64,
        _seq: u64,
        _drag_point: onecad_core::ids::EntityId,
        _target: [f64; 2],
    ) -> Result<crate::dto::DragSolveDto, EngineError> {
        Err(Self::not_ready())
    }
    async fn end_gesture(
        &self,
        _sketch_id: &str,
        _gesture_id: u64,
        _final_target: Option<[f64; 2]>,
    ) -> Result<crate::dto::SketchUpsertDto, EngineError> {
        Err(Self::not_ready())
    }
    async fn sketch_regions(
        &self,
        _sketch_id: &str,
    ) -> Result<Vec<crate::dto::SketchRegionDto>, EngineError> {
        Err(Self::not_ready())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(n: u128) -> BodyId {
        BodyId(Uuid::from_u128(n))
    }

    #[test]
    fn resolve_prefers_env_override_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let over = dir.path().join("custom-worker");
        std::fs::write(&over, b"x").unwrap();
        // A bundled sidecar also exists, but the override wins.
        let exe_dir = dir.path().join("bundle");
        std::fs::create_dir(&exe_dir).unwrap();
        std::fs::write(exe_dir.join(BUNDLED_WORKER_NAME), b"x").unwrap();

        // Env wins in BOTH build modes.
        for prefer_dev in [false, true] {
            let got = resolve_worker_path_from(
                Some(over.clone()),
                Some(exe_dir.clone()),
                Path::new("/nonexistent/dev/onecad-worker"),
                prefer_dev,
            );
            assert_eq!(got, Some(over.clone()));
        }
    }

    #[test]
    fn resolve_falls_through_missing_env_override_to_bundled() {
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("bundle");
        std::fs::create_dir(&exe_dir).unwrap();
        let bundled = exe_dir.join(BUNDLED_WORKER_NAME);
        std::fs::write(&bundled, b"x").unwrap();

        let got = resolve_worker_path_from(
            Some(dir.path().join("does-not-exist")),
            Some(exe_dir),
            Path::new("/nonexistent/dev/onecad-worker"),
            false,
        );
        assert_eq!(got, Some(bundled));
    }

    #[test]
    fn resolve_falls_through_to_dev_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("bundle"); // empty — no sidecar beside exe
        std::fs::create_dir(&exe_dir).unwrap();
        let dev = dir.path().join("dev-onecad-worker");
        std::fs::write(&dev, b"x").unwrap();

        let got = resolve_worker_path_from(None, Some(exe_dir), &dev, false);
        assert_eq!(got, Some(dev));
    }

    #[test]
    fn resolve_prefer_dev_wins_over_bundled_in_debug() {
        // The stale-sidecar drift class: BOTH exist; prefer_dev picks the
        // dev-tree build, release order picks the staged sidecar.
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("bundle");
        std::fs::create_dir(&exe_dir).unwrap();
        let bundled = exe_dir.join(BUNDLED_WORKER_NAME);
        std::fs::write(&bundled, b"x").unwrap();
        let dev = dir.path().join("dev-onecad-worker");
        std::fs::write(&dev, b"x").unwrap();

        let debug = resolve_worker_path_from(None, Some(exe_dir.clone()), &dev, true);
        assert_eq!(debug, Some(dev.clone()));
        let release = resolve_worker_path_from(None, Some(exe_dir), &dev, false);
        assert_eq!(release, Some(bundled));
    }

    #[test]
    fn resolve_prefer_dev_falls_back_to_bundled_when_dev_missing() {
        let dir = tempfile::tempdir().unwrap();
        let exe_dir = dir.path().join("bundle");
        std::fs::create_dir(&exe_dir).unwrap();
        let bundled = exe_dir.join(BUNDLED_WORKER_NAME);
        std::fs::write(&bundled, b"x").unwrap();

        let got = resolve_worker_path_from(
            None,
            Some(exe_dir),
            Path::new("/nonexistent/dev/onecad-worker"),
            true,
        );
        assert_eq!(got, Some(bundled));
    }

    #[test]
    fn resolve_returns_none_when_no_candidate_exists() {
        let dir = tempfile::tempdir().unwrap();
        for prefer_dev in [false, true] {
            let got = resolve_worker_path_from(
                Some(dir.path().join("missing-override")),
                Some(dir.path().join("empty-bundle")),
                &dir.path().join("missing-dev"),
                prefer_dev,
            );
            assert_eq!(got, None);
        }
    }

    #[test]
    fn adoption_accepts_deterministic_new_body_id() {
        let op = Uuid::from_u128(0x10);
        let known: HashSet<Uuid> = [op].into_iter().collect();
        let mut seen = HashSet::new();
        let ev = BodyLifecycleEvent::Created {
            body: BodyId(op), // body.uuid == opId (deterministic body_<opId>)
        };
        assert!(validate_created(&[ev], &known, &HashSet::new(), &mut seen).is_ok());
    }

    #[test]
    fn adoption_rejects_unknown_op_id() {
        let known: HashSet<Uuid> = [Uuid::from_u128(0x10)].into_iter().collect();
        let mut seen = HashSet::new();
        let ev = BodyLifecycleEvent::Created { body: body(0xBAD) };
        let err = validate_created(&[ev], &known, &HashSet::new(), &mut seen).unwrap_err();
        assert!(err.contains("malformation"), "{err}");
    }

    #[test]
    fn adoption_accepts_contiguous_split_children() {
        let op = Uuid::from_u128(0x10);
        let known: HashSet<Uuid> = [op].into_iter().collect();
        // Parse the worker's split-child wire ids so the interner maps them.
        let c0 = crate::worker::wire::parse_body_id(&format!("body_{op}:0")).unwrap();
        let c1 = crate::worker::wire::parse_body_id(&format!("body_{op}:1")).unwrap();
        let mut seen = HashSet::new();
        let events = vec![
            BodyLifecycleEvent::Deleted { body: BodyId(op) },
            BodyLifecycleEvent::Created { body: c0 },
            BodyLifecycleEvent::Created { body: c1 },
        ];
        assert!(validate_created(&events, &known, &HashSet::new(), &mut seen).is_ok());
    }

    #[test]
    fn adoption_rejects_non_contiguous_split_children() {
        let op = Uuid::from_u128(0x10);
        let known: HashSet<Uuid> = [op].into_iter().collect();
        let c0 = crate::worker::wire::parse_body_id(&format!("body_{op}:0")).unwrap();
        let c2 = crate::worker::wire::parse_body_id(&format!("body_{op}:2")).unwrap(); // gap at 1
        let mut seen = HashSet::new();
        let events = vec![
            BodyLifecycleEvent::Created { body: c0 },
            BodyLifecycleEvent::Created { body: c2 },
        ];
        let err = validate_created(&events, &known, &HashSet::new(), &mut seen).unwrap_err();
        assert!(err.contains("contiguous"), "{err}");
    }

    #[test]
    fn adoption_rejects_split_child_of_unknown_op() {
        let op = Uuid::from_u128(0x10);
        let other = Uuid::from_u128(0xBAD);
        let known: HashSet<Uuid> = [op].into_iter().collect();
        let child = crate::worker::wire::parse_body_id(&format!("body_{other}:0")).unwrap();
        let mut seen = HashSet::new();
        let err = validate_created(
            &[BodyLifecycleEvent::Created { body: child }],
            &known,
            &HashSet::new(),
            &mut seen,
        )
        .unwrap_err();
        assert!(err.contains("malformation"), "{err}");
    }

    #[test]
    fn adoption_rejects_duplicate_and_existing_collision() {
        let op = Uuid::from_u128(0x10);
        let known: HashSet<Uuid> = [op].into_iter().collect();
        // Duplicate within one plan.
        let mut seen = HashSet::new();
        let dup = vec![
            BodyLifecycleEvent::Created { body: BodyId(op) },
            BodyLifecycleEvent::Created { body: BodyId(op) },
        ];
        assert!(validate_created(&dup, &known, &HashSet::new(), &mut seen)
            .unwrap_err()
            .contains("collides"));
        // Collision with an existing session body.
        let mut seen2 = HashSet::new();
        let existing: HashSet<BodyId> = [BodyId(op)].into_iter().collect();
        let ev = BodyLifecycleEvent::Created { body: BodyId(op) };
        assert!(validate_created(&[ev], &known, &existing, &mut seen2)
            .unwrap_err()
            .contains("collides"));
    }
}
