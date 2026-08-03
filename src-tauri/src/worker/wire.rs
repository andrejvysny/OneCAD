//! Wire translation between the core [`GeometryEngine`] domain types and the
//! OCW1 SCHEMA JSON — the seam the [`WorkerManager`](super::manager::WorkerManager)
//! speaks over [`ProtocolClient`](onecad_protocol::client::ProtocolClient).
//!
//! Pure functions (no async, no IO). They map:
//!
//! * a [`PlanRequest`] → `ExecutePlan.args` (SCHEMA §7.2), each op serialized to
//!   the `{opType, opId, inputs, params, determinism}` wire shape (§7.3);
//! * a streamed `planStep` `event` payload → [`PlanStepEvent`] — the key boundary
//!   is `bodyEvents`/`elementMapDelta` `bodyId` strings **`body_<opId>` →
//!   `BodyId(opId uuid)`** (R-WP10 inherited flag; adoption re-checks
//!   `body.as_uuid() ∈ known_ops` in [`validate_created`](super::validate_created));
//! * a terminal `PlanPrepared` result → [`PlanPrepared`];
//! * lifecycle/accept/head/tessellate args + results.
//!
//! `NeedsRepair` payloads are parsed as **state** (SCHEMA §8/§9) into the step's
//! `needs_repair`, never mapped to an [`EngineError`]. `scoringVersion` rides
//! through verbatim (the `RepairItem` already carries the optional field).

use std::collections::BTreeMap;

use serde_json::{json, Value};
use uuid::Uuid;

use onecad_core::document::body::BodyLifecycleEvent;
use onecad_core::document::record::{ExtrudeMode, KnownOperation, Operation};
use onecad_core::document::refs::{AnchorIntent, ElementKind, ElementRef};
use onecad_core::document::repair::RepairItem;
use onecad_core::ids::{
    BodyId, DocumentRevision, ElementId, EntityId, JobId, SnapshotId, TopoKey, WorkerEpoch,
};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    AcceptResult, AcquireRequest, BodySelector, CheckpointArtifact, CheckpointArtifacts,
    CheckpointEnvelope, Diagnostic, ElementMapDelta, ElementMapEntry, EngineError,
    HistoryPrefixHash, OpFailureCode, OpenSessionRequest, PlanPrepared, PlanRequest, PlanStepEvent,
    PlannedOp, RefResolution, ResolveOutcome, ResolveRequest, RestoreRequest, SessionMode,
    Severity, Signature, StepResult, StepSignatures, StepStatus, StoppedReason, TessellateRequest,
    WorkerElementEvidence, WorkerHead, ARTIFACT_SCHEMA_VERSION,
};
use onecad_core::sketch::WorldPlane;
use onecad_core::sketch::{
    Constraint, CurvePosition, FaceFrame, ProjectedEntity, ProjectionPayload, Sketch,
    SketchAttachment, SketchEntity, SketchPlane,
};

use onecad_protocol::messages::{BinSection, ErrorCode, ErrorObject};

use crate::dto::{
    DragSolveDto, PreviewTrianglesDto, SketchRegionDto, SketchSolveStatus, SketchUpsertDto,
};

use super::{lod_str, StepInspection};

// ─────────────────────────────────────────────────────────────────────────────
// BodyId ↔ wire (`body_<opId>`)
// ─────────────────────────────────────────────────────────────────────────────

// ── Split-child BodyId representation (M5a, SCHEMA §2 / §14) ──────────────────
//
// A NewBody id is `body_<opId>` and maps 1:1 to `BodyId(opId uuid)`. A boolean SPLIT
// mints `body_<opId>:<k>` (deterministic `k`-ordering; see the worker `ordered_solids`).
// `BodyId` is a `Uuid` newtype, so a `:<k>` child cannot reuse the opId uuid — it maps
// to a **deterministic derived uuid** `uuid5(SPLIT_NS, "<opId>:<k>")` (pure function ⇒
// replay-stable + persistence-stable: the doc stores the derived uuid, a from-0 replay
// re-mints the SAME id). Because that uuid5 is one-way, `body_id_wire` cannot rebuild
// the `body_<opId>:<k>` string from the uuid alone; the [`split_interner`] records
// `derived → "body_<opId>:<k>"` at [`parse_body_id`] time (the worker ALWAYS mints a
// child, so Rust parses it, BEFORE Rust ever renders it back over the wire — tessellate,
// export, a downstream op input). The interner is a bounded, append-only, deterministic
// map (identical strings ⇒ identical uuids ⇒ identical strings), so it is self-healing
// across reopen and cannot be cross-contaminated between documents/tests.

use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

use onecad_core::document::body::split_child_uuid;

/// `derived split-child BodyId uuid → its "body_<opId>:<k>" wire string`. See the
/// section header for why the reverse map is needed + why it is sound. Repopulated
/// on document open / checkpoint restore from the persisted `BodyMeta.split_of`
/// ([`intern_split_child`]) so a downstream reference to a child renders correctly in
/// a **fresh process** — before the plan compiles, when nothing has been parsed yet.
fn split_interner() -> &'static RwLock<HashMap<Uuid, String>> {
    static INTERNER: OnceLock<RwLock<HashMap<Uuid, String>>> = OnceLock::new();
    INTERNER.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Interns split child `k` of op `op`: computes the deterministic derived uuid
/// (core [`split_child_uuid`]) and records `derived → "body_<op>:<k>"` so
/// [`body_id_wire`] renders the exact form the worker keys its BodyStore by. Returns
/// the child [`BodyId`]. Called at document open / checkpoint restore from the
/// persisted `split_of` so the interner is warm before any plan compiles (the
/// cross-process fix).
pub fn intern_split_child(op: Uuid, k: usize) -> BodyId {
    let derived = split_child_uuid(op, k);
    if let Ok(mut map) = split_interner().write() {
        map.insert(derived, format!("body_{op}:{k}"));
    }
    BodyId(derived)
}

/// Clears the split-id interner — TEST SUPPORT ONLY (simulates a fresh process so a
/// test can prove the persisted-`split_of` re-intern path, not a warm interner).
#[doc(hidden)]
pub fn clear_split_interner_for_test() {
    if let Ok(mut map) = split_interner().write() {
        map.clear();
    }
}

/// The interned `body_<opId>:<k>` wire string for a split-child [`BodyId`], if it is
/// one (i.e. it was produced by [`parse_body_id`]). `None` for a plain `body_<opId>`.
#[must_use]
pub fn split_wire(body: BodyId) -> Option<String> {
    split_interner().read().ok()?.get(&body.0).cloned()
}

/// The `(opId, ordinal)` a split-child [`BodyId`] was derived from, if it is one.
/// Used by adoption ([`validate_created`](super::validate_created)) to re-check the
/// opId against the plan + the `k`-contiguity.
#[must_use]
pub fn split_parts(body: BodyId) -> Option<(Uuid, usize)> {
    let s = split_wire(body)?;
    let rest = s.strip_prefix("body_")?;
    let (op, k) = rest.split_once(':')?;
    Some((Uuid::parse_str(op).ok()?, k.parse().ok()?))
}

/// The wire form of a [`BodyId`]: `body_<opId>:<k>` for a split child (from the
/// interner), else `body_<uuid>` (SCHEMA §2 — a NewBody id is `body_<opId>`, the
/// `opId` being the Rust-minted record-id uuid).
#[must_use]
pub fn body_id_wire(body: BodyId) -> String {
    if let Some(s) = split_wire(body) {
        return s;
    }
    format!("body_{}", body.0)
}

/// Parses a worker `body_<opId>` (or split `body_<opId>:<k>`) string back to a core
/// [`BodyId`] (SCHEMA §2, D1). A plain id maps to `BodyId(opId uuid)`; a split child
/// maps to the deterministic derived uuid [`split_child_uuid`] and INTERNS the reverse
/// mapping so [`body_id_wire`] can rebuild the exact `body_<opId>:<k>` string.
///
/// # Errors
/// A human reason on a missing prefix, a non-uuid opId, or a non-integer ordinal.
pub fn parse_body_id(s: &str) -> Result<BodyId, String> {
    let op = s
        .strip_prefix("body_")
        .ok_or_else(|| format!("bodyId {s:?} missing 'body_' prefix (D1)"))?;
    if let Some((op_str, k_str)) = op.split_once(':') {
        let op_uuid = Uuid::parse_str(op_str)
            .map_err(|e| format!("split-child bodyId {s:?} opId is not a uuid: {e}"))?;
        let k: usize = k_str
            .parse()
            .map_err(|e| format!("split-child bodyId {s:?} ordinal is not an integer: {e}"))?;
        // Interns the CANONICAL reconstruction so `body_id_wire` always emits the exact
        // form the worker keys its BodyStore by (lowercase uuid + ordinal).
        return Ok(intern_split_child(op_uuid, k));
    }
    Uuid::parse_str(op)
        .map(BodyId)
        .map_err(|e| format!("bodyId {s:?} opId is not a uuid: {e}"))
}

/// The wire `jobId` (SCHEMA §2 `u64`) for a core [`JobId`].
///
/// **Collision-safety invariant:** a `JobId` is minted from a strictly-monotonic
/// per-document `u64` counter via `Uuid::from_u128(u128::from(counter))` (see
/// `DocumentRuntime::next_job_id`), so the uuid's full 128-bit value equals the
/// counter and always fits in the low 64 bits. Truncating to `u64` here is
/// therefore lossless and collision-free per connection — two distinct jobs never
/// map to the same wire id. The `debug_assert` pins that invariant at the
/// truncation site: a `JobId` with any high bits set would be a mis-minted id.
#[must_use]
pub fn job_id_wire(job: JobId) -> u64 {
    debug_assert_eq!(
        job.0.as_u128() >> 64,
        0,
        "JobId must be minted from a monotonic u64 counter (no high bits) so the wire \
         truncation is collision-free"
    );
    job.0.as_u128() as u64
}

// ─────────────────────────────────────────────────────────────────────────────
// ExecutePlan args (SCHEMA §7.2 / §7.3)
// ─────────────────────────────────────────────────────────────────────────────

/// Builds the `ExecutePlan.args` for a fenced [`PlanRequest`] (SCHEMA §7.2).
#[must_use]
pub fn execute_plan_args(req: &PlanRequest) -> Value {
    let ops: Vec<Value> = req.ops.iter().map(wire_op).collect();
    let prefix: Vec<Value> = req
        .prefix_hashes
        .iter()
        .map(|h| Value::String(h.as_str().to_string()))
        .collect();
    let mut args = json!({
        "jobId": job_id_wire(req.job_id),
        "documentRevision": req.document_revision.0,
        "workerEpoch": req.worker_epoch.0,
        "expectedBaseHash": req.expected_base_hash.as_str(),
        "prefixHashes": prefix,
        "policyVersions": {
            "quantizationVersion": req.policy_versions.quantization,
            "solverPolicyVersion": req.policy_versions.solver_policy,
            "descriptorVersion": req.policy_versions.descriptor,
            "resolverVersion": req.policy_versions.resolver,
            "signatureVersion": req.policy_versions.signature,
        },
        "targetStep": req.target_step,
        "ops": ops,
    });
    if let Some(cp) = &req.base_checkpoint {
        args["baseCheckpoint"] =
            json!({ "stepIndex": cp.step_index, "checkpointId": cp.checkpoint_id.as_str() });
    }
    if let Some(t) = &req.artifacts.tessellate {
        args["artifacts"] =
            json!({ "tessellate": { "lod": lod_str(t.lod), "includeEdges": t.include_edges } });
    }
    args
}

/// One op in `ExecutePlan.ops` (SCHEMA §7.3): `{opType, opId, inputs, params,
/// determinism}`. `opType`/`params` come from the typed [`Operation`] (same split
/// the planner hashes over); `inputs`/`determinism` serialize their core structs.
///
/// `params` is rendered through [`to_wire_body_form`] so every body-bearing field
/// (`targetBodyId`/`toolBodyId`/`axis.bodyId`/…) crosses the wire in the worker's
/// `body_<uuid>` form: the core serde emits a [`BodyId`] transparently as a bare
/// uuid, but the worker keys its `BodyStore` by `body_<opId>`, so a bare uuid would
/// never resolve (REF_UNRESOLVED / "target body not found").
fn wire_op(op: &PlannedOp) -> Value {
    let mut wire = lower_operation(&op.operation, &op.inputs, &op.record_id.to_string());
    wire["stepIndex"] = json!(op.step_index);
    wire["determinism"] = serde_json::to_value(&op.determinism).unwrap_or(Value::Null);
    wire
}

/// Canonical worker operation for drag preview.
///
/// Preview and ExecutePlan MUST lower the typed core [`Operation`] through this
/// same path. In particular, this lifts `profile` to flat `sketchId`/`regionId`,
/// rewrites every body-bearing UUID to `body_<uuid>`, and derives the same
/// semantic `inputs[]` a committed operation carries.
#[must_use]
pub fn preview_wire_op(operation: &Operation, op_id: &str) -> Value {
    lower_operation(operation, &operation.derive_inputs(), op_id)
}

/// Shared typed-operation lowering used by both ExecutePlan and PreviewOp.
fn lower_operation(
    operation: &Operation,
    inputs: &onecad_core::document::record::OperationInputs,
    op_id: &str,
) -> Value {
    let op_val = serde_json::to_value(operation).unwrap_or(Value::Null);
    let (op_type, mut params) = split_operation(operation, op_val);
    to_wire_body_form(&mut params);
    lift_profile_to_params(&mut params);
    inject_import_path(operation, &mut params);
    json!({
        "opType": op_type,
        "opId": op_id,
        "inputs": wire_op_inputs(operation, inputs),
        "params": params,
    })
}

fn split_operation(operation: &Operation, op_val: Value) -> (Value, Value) {
    match operation {
        Operation::Known(_) => (
            op_val.get("opType").cloned().unwrap_or(Value::Null),
            op_val.get("params").cloned().unwrap_or(Value::Null),
        ),
        Operation::Opaque(_) => {
            let mut obj = op_val.as_object().cloned().unwrap_or_default();
            let op_type = obj.remove("opType").unwrap_or(Value::Null);
            (op_type, Value::Object(obj))
        }
    }
}

/// Lifts the Rust-core `profile` (`{sketchId, regionId}`, a `SketchRegionRef`) to
/// **top-level** `params.sketchId` + `params.regionId` and drops the `profile`
/// wrapper (SCHEMA §7.3 carries no `profile`).
///
/// The worker reads the profile source there: `ExtrudeOp`/`RevolveOp` `find_sketch`
/// selects the sketch by `params.sketchId` (else `last_sketch_id`), and
/// `build_profile_face` selects the region by matching `params.regionId` against each
/// detected region's normative FNV id (SCHEMA §7.4). This is what closes the
/// multi-region / multi-sketch profile-binding gap (M2 flag `last_sketch_id` +
/// first-region fallback): a non-empty `regionId` now picks a specific region; an
/// empty/absent one keeps the first-region fallback. Ops without a `profile` object
/// are untouched.
///
/// **Sweep note:** this also fires on a `Sweep` op's `profile` (it has the same
/// `SketchRegionRef` shape). Sweep has no worker handler yet; when one lands it MUST
/// read the lifted top-level `params.sketchId`/`params.regionId` (as Extrude/Revolve
/// do), NOT a nested `params.profile`, which this lift removes.
fn lift_profile_to_params(params: &mut Value) {
    let Some(map) = params.as_object_mut() else {
        return;
    };
    let Some(profile) = map.remove("profile") else {
        return;
    };
    let Some(pobj) = profile.as_object() else {
        return;
    };
    if let Some(sid) = pobj.get("sketchId") {
        map.insert("sketchId".into(), sid.clone());
    }
    // Only forward a non-empty regionId — an empty one keeps the worker's
    // first-region fallback (backward compat with placeholder/legacy region ids).
    if let Some(rid) = pobj
        .get("regionId")
        .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
    {
        map.insert("regionId".into(), rid.clone());
    }
}

/// Injects the **wire-only, NON-hashed** `params.path` an `ImportStep` op needs
/// (SCHEMA §7.3 / §7.8): the core params are a content-address POINTER
/// (`sourceSha256`), and Rust materializes the blob to a temp file the worker reads.
///
/// The path is resolved from the process-wide
/// [`import path registry`](crate::imports) that
/// [`DocumentRuntime`](crate::document_runtime::DocumentRuntime) populates when it
/// materializes a document's import blobs. It is deliberately absent from
/// [`ImportStepParams`](onecad_core::document::record::ImportStepParams) — the
/// planner hashes the CORE params, so two lowerings of the same record under
/// different temp roots produce the same
/// [`history_prefix_hash`](onecad_core::regen::planner::history_prefix_hash) and
/// different wire JSON. An absolute path in the hash would make every document
/// machine-specific.
///
/// **A missing blob lowers an EMPTY path on purpose.** The worker then fails that
/// one step with `OP_FAILED` ("no source path supplied"), which is a recoverable,
/// named, per-record failure — exactly the blast radius `io::imports` designs for.
/// Refusing to lower (or panicking) would take down the whole plan, i.e. every
/// unrelated feature in the document, over one un-materialized blob.
fn inject_import_path(operation: &Operation, params: &mut Value) {
    let Operation::Known(KnownOperation::ImportStep(p)) = operation else {
        return;
    };
    let Some(map) = params.as_object_mut() else {
        return;
    };
    let path = crate::imports::resolve_blob_path(&p.source_sha256);
    if path.is_empty() {
        tracing::warn!(
            sha = %p.source_sha256,
            source = %p.source_name,
            "importStep: no materialized blob for this source — lowering an empty path \
             (the worker will fail THIS step with OP_FAILED, not the plan)"
        );
    }
    map.insert("path".into(), Value::String(path));
}

/// Rewrites every body-bearing field of `value` — a key exactly `"bodyId"` or ending
/// in `"BodyId"` (`targetBodyId`, `toolBodyId`, `sourceBodyId`, …) — whose value is a
/// bare [`Uuid`] string into the worker's `body_<uuid>` wire form (SCHEMA §2),
/// recursing through nested objects and arrays.
///
/// The core serde emits a [`BodyId`] transparently as a **bare uuid** (the frozen
/// v2 file schema); this wire layer owns rendering it as `body_<uuid>`, the id form
/// the worker's `BodyStore` is keyed by (a NewBody body is `body_<opId>`).
/// **Idempotent** — an already-prefixed value (`body_…`) fails the uuid parse and is
/// left untouched, as is the empty string (`""` = "no body", the NewBody case).
/// **Scoped to body keys only** — `sketchId`/`opId`/`elementId`/`edgeIds`/… never
/// match, so a non-body id is never rewritten. **`intent` subtrees are skipped
/// wholesale**: an [`ElementRef`]'s `intent` is worker-authored frozen evidence
/// (descriptor + metadata) that must round-trip verbatim — any body reference the
/// worker ever puts there is already in wire form, and the wire layer must not
/// rewrite worker-owned bytes (independent-review NOTE, 2026-07-19).
fn to_wire_body_form(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if key == "intent" {
                    continue;
                }
                if key == "bodyId" || key.ends_with("BodyId") {
                    if let Value::String(s) = v {
                        if let Ok(u) = Uuid::parse_str(s) {
                            *s = body_id_wire(BodyId(u));
                        }
                    }
                }
                // `TransformBody.targets` is the one body-bearing param that is an
                // ARRAY of ids rather than a `*BodyId` scalar (SCHEMA §7.3
                // `"targets": ["body_1", …]`), so it needs its own arm. Same
                // idempotent, uuid-parse-gated rewrite, applied per element.
                if key == "targets" {
                    if let Value::Array(items) = v {
                        for item in items.iter_mut() {
                            if let Value::String(s) = item {
                                if let Ok(u) = Uuid::parse_str(s) {
                                    *s = body_id_wire(BodyId(u));
                                }
                            }
                        }
                    }
                }
                to_wire_body_form(v);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(to_wire_body_form),
        _ => {}
    }
}

/// The SCHEMA §7.3 `inputs[]` semantic-ref ARRAY for an op, built from its typed
/// params. (The derived [`OperationInputs`](onecad_core::document::record::OperationInputs)
/// `{bodies,sketches,elements}` id-view drives the Rust dependency graph; the WIRE
/// must carry per-op semantic refs `{primary:{bodyId,elementId,kind}, anchor, intent}`
/// the worker's ops resolve through the ladder — §7.3, e.g. Fillet edges / Boolean
/// bodies / Extrude ToFace targets.)
///
/// `bodyId` is rendered in the worker's `body_<uuid>` wire form (SCHEMA §2). The
/// extrude *profile* rides in `params` (`sketchId`/first region) — the worker reads
/// it there — so a Blind/NewBody extrude carries no `inputs`.
fn wire_op_inputs(
    operation: &Operation,
    inputs: &onecad_core::document::record::OperationInputs,
) -> Value {
    let refs: Vec<Value> = match operation {
        Operation::Known(KnownOperation::Fillet(p)) => {
            edge_input_refs(&p.edges, &p.edge_ids, &inputs.bodies)
        }
        Operation::Known(KnownOperation::Chamfer(p)) => {
            edge_input_refs(&p.edges, &p.edge_ids, &inputs.bodies)
        }
        Operation::Known(KnownOperation::Boolean(p)) => {
            vec![body_input_ref(p.target_body), body_input_ref(p.tool_body)]
        }
        // Shell: one semantic ref per removed (open) face. ShellParams carries only
        // bare ElementIds, so these are element-only face refs (mirrors Fillet's
        // bare-`edge_ids` fallback); the worker resolves each through the ladder or its
        // partition-tracked binding (§10). The shelled body rides in `params`.
        Operation::Known(KnownOperation::Shell(p)) => {
            face_input_refs(&p.open_faces, &inputs.bodies)
        }
        // Linear/Circular pattern + MirrorBody: a whole-body ref to the SOURCE body
        // (the axis/plane/spacing ride in `params`; §7.3). Mirrors Boolean's body refs;
        // the source id is also in `params.sourceBodyId` (auto-covered by
        // `to_wire_body_form`), so this is the graph-visible input echo.
        Operation::Known(KnownOperation::LinearPattern(p)) => {
            p.source_body.map(body_input_ref).into_iter().collect()
        }
        Operation::Known(KnownOperation::CircularPattern(p)) => {
            p.source_body.map(body_input_ref).into_iter().collect()
        }
        Operation::Known(KnownOperation::MirrorBody(p)) => {
            p.source_body.map(body_input_ref).into_iter().collect()
        }
        // TransformBody: one whole-body ref per target, in `targets` order —
        // `inputs[]` MIRRORS `params.targets` (SCHEMA §7.3). The order is
        // load-bearing for `copy: true`: the ordinal `k` in `body_<opId>:<k>` is
        // the target's index in `targets`.
        Operation::Known(KnownOperation::TransformBody(p)) => {
            p.targets.iter().copied().map(body_input_ref).collect()
        }
        Operation::Known(KnownOperation::Extrude(p)) => {
            let mut v = Vec::new();
            if p.mode == ExtrudeMode::ToFace {
                if let Some(f) = &p.target_face {
                    v.push(element_ref_wire(f));
                }
            }
            if p.two_directions && p.mode2 == ExtrudeMode::ToFace {
                if let Some(f) = &p.target_face2 {
                    v.push(element_ref_wire(f));
                }
            }
            v
        }
        _ => Vec::new(),
    };
    Value::Array(refs)
}

/// Fillet/Chamfer edge refs: prefer the typed per-edge [`ElementRef`]s (they carry
/// the operated body + anchor/descriptor evidence). Fall back to bare `edge_ids`
/// (element-only), attaching the operated body — the op's graph-view `bodies[0]`
/// (`FilletParams` derives the operated body from its edge refs; SCHEMA §7.3) — as
/// `primary.bodyId` in the worker's `body_<uuid>` form so `FilletChamferOp`'s
/// `target_body_of()` can bind the body. With no body input the ref stays
/// element-only (a clear worker-side "requires body input" error, not a silent miss).
fn edge_input_refs(edges: &[ElementRef], edge_ids: &[ElementId], bodies: &[BodyId]) -> Vec<Value> {
    if !edges.is_empty() {
        return edges.iter().map(element_ref_wire).collect();
    }
    edge_ids
        .iter()
        .map(|id| {
            let mut primary = serde_json::Map::new();
            if let Some(b) = bodies.first() {
                primary.insert("bodyId".into(), json!(body_id_wire(*b)));
            }
            primary.insert("elementId".into(), json!(id.as_str()));
            primary.insert("kind".into(), json!("edge"));
            json!({ "primary": Value::Object(primary) })
        })
        .collect()
}

/// Shell open-face refs: a bare `{primary:{bodyId, elementId, kind:"face"}}` per open
/// face id. `ShellParams` carries only bare [`ElementId`]s (no typed per-face ref), so
/// no descriptor/anchor rides — this mirrors [`edge_input_refs`]'s element-only fallback.
/// The shelled body — the op's graph-view `bodies[0]` (`ShellParams` derives it from
/// `params.targetBodyId`, SCHEMA §7.3) — is attached as `primary.bodyId` in the worker's
/// `body_<uuid>` form so `ShellOp` can group + resolve the faces (ladder §10, or the
/// partition-tracked binding). With no body input the ref stays element-only.
fn face_input_refs(face_ids: &[ElementId], bodies: &[BodyId]) -> Vec<Value> {
    face_ids
        .iter()
        .map(|id| {
            let mut primary = serde_json::Map::new();
            if let Some(b) = bodies.first() {
                primary.insert("bodyId".into(), json!(body_id_wire(*b)));
            }
            primary.insert("elementId".into(), json!(id.as_str()));
            primary.insert("kind".into(), json!("face"));
            json!({ "primary": Value::Object(primary) })
        })
        .collect()
}

/// A whole-body semantic ref (boolean target/tool). `elementId` == the body id, as
/// the worker keys its body records by the `body_<uuid>` id (D1).
fn body_input_ref(b: BodyId) -> Value {
    json!({ "primary": { "bodyId": body_id_wire(b), "elementId": body_id_wire(b), "kind": "body" } })
}

/// Render an [`ElementRef`] to the SCHEMA §7.3 semantic-ref JSON (`{primary, intent,
/// anchor}`), with body-bearing fields in the worker's `body_<uuid>` wire form.
///
/// Serializes the typed ref directly (rather than hand-rolling the object), so the
/// `#[serde(flatten)] extra` forward-compat maps that a hand-rolled builder would
/// drop survive; `primary.kind` serializes lowercase (`face`/`edge`/`vertex`) via
/// [`ElementKind`]'s serde derive. [`to_wire_body_form`] then rewrites `primary.bodyId`
/// (a bare uuid from core serde) into `body_<uuid>` — the form the worker reads in
/// `OpCommon`/`FilletChamferOp`/`ExtrudeOp::resolve_to_face`.
fn element_ref_wire(r: &ElementRef) -> Value {
    let mut v = serde_json::to_value(r).unwrap_or_else(|_| json!({}));
    to_wire_body_form(&mut v);
    v
}

// ─────────────────────────────────────────────────────────────────────────────
// planStep event → PlanStepEvent (SCHEMA §7.2)
// ─────────────────────────────────────────────────────────────────────────────

/// Parses one `planStep` event payload into a core [`PlanStepEvent`].
///
/// # Errors
/// A human reason on a malformed `bodyId` / `elementMapDelta` / `needsRepair`
/// payload (surfaced by the caller as `PROTOCOL_ERROR`).
pub fn parse_plan_step(payload: &Value, fallback_step: usize) -> Result<PlanStepEvent, String> {
    let step_index = payload
        .get("stepIndex")
        .and_then(Value::as_u64)
        .map_or(fallback_step, |s| s as usize);
    Ok(PlanStepEvent {
        step_index,
        body_events: parse_body_events(payload.get("bodyEvents"))?,
        element_map_delta: parse_element_delta(payload.get("elementMapDelta"))?,
        needs_repair: parse_needs_repair(payload.get("needsRepair"), step_index)?,
        signatures: parse_signatures(payload.get("signatures")),
        diagnostics: parse_diagnostics(payload.get("diagnostics")),
    })
}

fn parse_body_events(v: Option<&Value>) -> Result<Vec<BodyLifecycleEvent>, String> {
    let arr = v.and_then(Value::as_array).cloned().unwrap_or_default();
    arr.iter().map(parse_body_event).collect()
}

fn parse_body_event(ev: &Value) -> Result<BodyLifecycleEvent, String> {
    let kind = ev.get("kind").and_then(Value::as_str).unwrap_or("");
    let body = || body_field(ev, "bodyId");
    match kind {
        "created" => Ok(BodyLifecycleEvent::Created { body: body()? }),
        "modified" => Ok(BodyLifecycleEvent::Modified { body: body()? }),
        "deleted" => Ok(BodyLifecycleEvent::Deleted { body: body()? }),
        "split" => Ok(BodyLifecycleEvent::Split {
            parent: body_field(ev, "parent")?,
            children: body_array(ev.get("children"))?,
        }),
        "merged" => Ok(BodyLifecycleEvent::Merged {
            inputs: body_array(ev.get("inputs"))?,
            winner: body_field(ev, "winner")?,
        }),
        other => Err(format!("unknown bodyEvent kind {other:?}")),
    }
}

fn body_field(ev: &Value, key: &str) -> Result<BodyId, String> {
    parse_body_id(ev.get(key).and_then(Value::as_str).unwrap_or(""))
}

fn body_array(v: Option<&Value>) -> Result<Vec<BodyId>, String> {
    str_array(v).iter().map(|s| parse_body_id(s)).collect()
}

fn parse_element_delta(v: Option<&Value>) -> Result<ElementMapDelta, String> {
    let get = |k: &str| v.and_then(|d| d.get(k));
    Ok(ElementMapDelta {
        added: parse_entries(get("added"))?,
        relabeled: parse_entries(get("relabeled"))?,
        removed: str_array(get("removed"))
            .into_iter()
            .map(ElementId::new)
            .collect(),
    })
}

fn parse_entries(v: Option<&Value>) -> Result<Vec<ElementMapEntry>, String> {
    let arr = v.and_then(Value::as_array).cloned().unwrap_or_default();
    arr.iter()
        .map(|e| {
            Ok(ElementMapEntry {
                element_id: ElementId::new(
                    e.get("elementId").and_then(Value::as_str).unwrap_or(""),
                ),
                topo_key: TopoKey::new(e.get("topoKey").and_then(Value::as_str).unwrap_or("")),
                kind: parse_kind(e.get("kind").and_then(Value::as_str).unwrap_or("face")),
                body: body_field(e, "bodyId")?,
            })
        })
        .collect()
}

/// Parses `needsRepair[]` **state** (SCHEMA §9), injecting the step index each
/// item omits (it is implicit from the enclosing `planStep`). `scoringVersion`
/// rides through as the `RepairItem`'s optional field.
fn parse_needs_repair(v: Option<&Value>, step: usize) -> Result<Vec<RepairItem>, String> {
    let arr = v.and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let mut obj = item;
        if let Some(map) = obj.as_object_mut() {
            map.entry("stepIndex".to_string()).or_insert(json!(step));
        }
        out.push(serde_json::from_value(obj).map_err(|e| format!("needsRepair parse: {e}"))?);
    }
    Ok(out)
}

fn parse_signatures(v: Option<&Value>) -> StepSignatures {
    let sig = |k: &str| {
        Signature::new(
            v.and_then(|s| s.get(k))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        )
    };
    StepSignatures {
        geometry: sig("geometry"),
        body_lifecycle: sig("bodyLifecycle"),
        referenced_binding: sig("referencedBinding"),
    }
}

fn parse_diagnostics(v: Option<&Value>) -> Vec<Diagnostic> {
    v.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|d| Diagnostic {
                    severity: match d.get("severity").and_then(Value::as_str) {
                        Some("error") => Severity::Error,
                        Some("info") => Severity::Info,
                        _ => Severity::Warning,
                    },
                    code: d
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    message: d
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────
// PlanPrepared (SCHEMA §7.2)
// ─────────────────────────────────────────────────────────────────────────────

/// Parses a terminal `PlanPrepared` result. `job` is the [`JobId`] Rust sent (the
/// executor checks the prepare is for *this* job), not re-parsed from the wire.
///
/// # Errors
/// A human reason on a missing `preparedSnapshotId` or a malformed `bodyIds`.
pub fn parse_plan_prepared(job: JobId, result: &Value) -> Result<PlanPrepared, String> {
    let prepared_snapshot_id = SnapshotId(
        result
            .get("preparedSnapshotId")
            .and_then(Value::as_u64)
            .ok_or("PlanPrepared missing preparedSnapshotId")?,
    );
    let last_valid_step = match result.get("lastValidStep") {
        Some(Value::Number(n)) => n.as_u64().map(|v| v as usize),
        _ => None,
    };
    let stopped_reason = match result.get("stoppedReason").and_then(Value::as_str) {
        Some("opFailed") => StoppedReason::OpFailed,
        Some("needsRepair") => StoppedReason::NeedsRepair,
        _ => StoppedReason::Completed,
    };
    Ok(PlanPrepared {
        job_id: job,
        prepared_snapshot_id,
        last_valid_step,
        stopped_reason,
        per_step: parse_per_step(result.get("perStepResults"))?,
        history_prefix_hash: HistoryPrefixHash::new(
            result
                .get("historyPrefixHash")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
    })
}

fn parse_per_step(v: Option<&Value>) -> Result<Vec<StepResult>, String> {
    let arr = v.and_then(Value::as_array).cloned().unwrap_or_default();
    arr.iter()
        .map(|r| {
            Ok(StepResult {
                step_index: r.get("stepIndex").and_then(Value::as_u64).unwrap_or(0) as usize,
                status: match r.get("status").and_then(Value::as_str) {
                    Some("needsRepair") => StepStatus::NeedsRepair,
                    Some("opFailed") => StepStatus::OpFailed,
                    _ => StepStatus::Ok,
                },
                body_ids: body_array(r.get("bodyIds"))?,
                message: r
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Lifecycle / accept / head / tessellate
// ─────────────────────────────────────────────────────────────────────────────

/// `OpenSession.args` (SCHEMA §7.1).
#[must_use]
pub fn open_session_args(req: &OpenSessionRequest) -> Value {
    json!({
        "documentId": req.document_id.to_string(),
        "documentRevision": req.document_revision.0,
        "workerEpoch": req.worker_epoch.0,
        "mode": match req.mode { SessionMode::Fast => "fast", SessionMode::Determinism => "determinism" },
    })
}

/// Parses an `OpenSession` result head (SCHEMA §7.1); `epoch` is the epoch Rust
/// opened with.
#[must_use]
pub fn parse_open_session(result: &Value, epoch: WorkerEpoch) -> WorkerHead {
    let head = result.get("workerHead");
    WorkerHead {
        document_revision: DocumentRevision(u64_at(head, "documentRevision")),
        worker_epoch: epoch,
        snapshot_id: SnapshotId(u64_at(head, "snapshotId")),
        history_prefix_hash: HistoryPrefixHash::empty(),
        has_scratch: false,
    }
}

/// Parses a `GetWorkerHead` result (SCHEMA §7.1).
#[must_use]
pub fn parse_worker_head(result: &Value) -> WorkerHead {
    WorkerHead {
        document_revision: DocumentRevision(
            result
                .get("documentRevision")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        worker_epoch: WorkerEpoch(
            result
                .get("workerEpoch")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        snapshot_id: SnapshotId(
            result
                .get("snapshotId")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        history_prefix_hash: HistoryPrefixHash::new(
            result
                .get("historyPrefixHash")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        has_scratch: result
            .get("hasScratch")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// Parses an `AcceptPrepared` result (SCHEMA §7.2).
#[must_use]
pub fn parse_accept(result: &Value) -> AcceptResult {
    AcceptResult {
        snapshot_id: SnapshotId(
            result
                .get("snapshotId")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        document_revision: DocumentRevision(
            result
                .get("documentRevision")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
    }
}

/// `Tessellate.args` (SCHEMA §7.6).
#[must_use]
pub fn tessellate_args(req: &TessellateRequest) -> Value {
    let bodies = match &req.bodies {
        BodySelector::All => json!("all"),
        BodySelector::Ids(ids) => json!(ids.iter().map(|b| body_id_wire(*b)).collect::<Vec<_>>()),
    };
    json!({ "bodyIds": bodies, "lod": lod_str(req.lod), "includeEdges": req.include_edges })
}

/// `ExportStep.args` (SCHEMA §7.8).
#[must_use]
pub fn export_step_args(path: &str, bodies: &[BodyId], schema: &str) -> Value {
    json!({
        "path": path,
        "bodyIds": bodies.iter().map(|b| body_id_wire(*b)).collect::<Vec<_>>(),
        "schema": schema,
    })
}

/// `ExportStl.args` (SCHEMA §7.8): `{path, bodyIds, binary, lod}`.
#[must_use]
pub fn export_stl_args(path: &str, bodies: &[BodyId], binary: bool, lod: &str) -> Value {
    json!({
        "path": path,
        "bodyIds": bodies.iter().map(|b| body_id_wire(*b)).collect::<Vec<_>>(),
        "binary": binary,
        "lod": lod,
    })
}

/// `ExportObj.args` (SCHEMA §7.8): `{path, bodyIds, lod}`.
#[must_use]
pub fn export_obj_args(path: &str, bodies: &[BodyId], lod: &str) -> Value {
    json!({
        "path": path,
        "bodyIds": bodies.iter().map(|b| body_id_wire(*b)).collect::<Vec<_>>(),
        "lod": lod,
    })
}

/// `InspectStep.args` (SCHEMA §7.8): `{path, includeGeometry}`.
#[must_use]
pub fn inspect_step_args(path: &str, include_geometry: bool) -> Value {
    json!({ "path": path, "includeGeometry": include_geometry })
}

/// Parses an `InspectStep` result (SCHEMA §7.8) into a [`StepInspection`], WITHOUT
/// the `geometry` bin payload (the caller attaches it from the response tail).
///
/// Missing/mistyped scalars fall back to their neutral values (`0` / `""` / an
/// all-zero bbox) rather than failing: the probe already SUCCEEDED at the worker,
/// and the fields this parser tolerates losing are advisory display evidence. The
/// load-bearing ones are `solidCount`, `geometryCodec` and `geometryFormat`, and
/// all three are validated by the caller against what it actually does with them (a
/// zero `solidCount` means no bodies; an unknown codec or a format the worker
/// cannot read fails loudly at authoring / replay).
#[must_use]
pub fn parse_inspect_step(result: &Value) -> StepInspection {
    let vec3 = |v: Option<&Value>| -> [f64; 3] {
        let a = v.and_then(Value::as_array);
        let get = |i: usize| {
            a.and_then(|a| a.get(i))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        };
        [get(0), get(1), get(2)]
    };
    let bbox = result.get("bbox");
    StepInspection {
        solid_count: result
            .get("solidCount")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(usize::MAX as u64) as usize,
        source_unit: result
            .get("sourceUnit")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        bbox: (
            vec3(bbox.and_then(|b| b.get("min"))),
            vec3(bbox.and_then(|b| b.get("max"))),
        ),
        product_names: result
            .get("productNames")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|v| v.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default(),
        geometry_codec: result
            .get("geometryCodec")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        geometry_format: result
            .get("geometryFormat")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        diagnostics: result
            .get("diagnostics")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|d| {
                        (
                            d.get("code")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .into(),
                            d.get("message")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .into(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        geometry_bytes: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Checkpoints (SCHEMA §7.7)
// ─────────────────────────────────────────────────────────────────────────────

/// `SaveCheckpoint.args` (SCHEMA §7.7): `{stepIndex}`.
#[must_use]
pub fn save_checkpoint_args(step_index: usize) -> Value {
    json!({ "stepIndex": step_index })
}

/// `RestoreCheckpoint.args` (SCHEMA §7.7 + `workerEpoch` for D4 fencing +
/// `stepIndex` so the worker keys its in-session retained checkpoint).
#[must_use]
pub fn restore_checkpoint_args(req: &RestoreRequest, worker_epoch: WorkerEpoch) -> Value {
    json!({
        "checkpointId": req.checkpoint.checkpoint_id.as_str(),
        "stepIndex": req.checkpoint.step_index,
        "expectedHistoryPrefixHash": req.expected_history_prefix_hash.as_str(),
        "workerEpoch": worker_epoch.0,
    })
}

/// Extracts the bytes of a named `bin` section from a resp's binary tail.
fn extract_bin_section(
    name: Option<&Value>,
    sections: &[BinSection],
    tail: &[u8],
) -> Result<Vec<u8>, String> {
    let name = name
        .and_then(Value::as_str)
        .ok_or("checkpoint artifact missing 'bin' name")?;
    let sec = sections
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("checkpoint bin section {name:?} missing"))?;
    let (start, end) = (sec.off as usize, (sec.off + sec.len) as usize);
    tail.get(start..end)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| format!("checkpoint bin section {name:?} out of range"))
}

/// Parses a `SaveCheckpoint` resp + its binary tail into core
/// [`CheckpointArtifacts`] (SCHEMA §7.7). Envelope version axes come from the
/// current policy (`occt_fingerprint` + version 1s), so a later regen validates the
/// checkpoint against the running worker's fingerprint (Invariant 7).
pub fn parse_save_checkpoint(
    result: &Value,
    sections: &[BinSection],
    tail: &[u8],
    occt_fingerprint: &str,
) -> Result<CheckpointArtifacts, String> {
    let step = result.get("stepIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
    let history_prefix_hash = HistoryPrefixHash::new(
        result
            .get("historyPrefixHash")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    );
    let signatures = parse_signatures(result.get("signatures"));
    let mut artifacts = Vec::new();
    if let Some(arr) = result.get("artifacts").and_then(Value::as_array) {
        for a in arr {
            let body = parse_body_id(a.get("bodyId").and_then(Value::as_str).unwrap_or(""))?;
            let bytes = extract_bin_section(a.get("bin"), sections, tail)?;
            let content_hash = a
                .get("contentHash")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let envelope = CheckpointEnvelope {
                artifact_schema_version: ARTIFACT_SCHEMA_VERSION,
                body,
                step,
                history_prefix_hash: history_prefix_hash.clone(),
                brep_content_hash: content_hash.clone(),
                occt_fingerprint: occt_fingerprint.to_string(),
                descriptor_version: 1,
                resolver_version: 1,
                quantization_version: 1,
                signature_version: 1,
                codec: a
                    .get("codec")
                    .and_then(Value::as_str)
                    .unwrap_or("brep-bintools")
                    .to_string(),
                size: bytes.len() as u64,
                content_hash,
            };
            artifacts.push(CheckpointArtifact { envelope, bytes });
        }
    }
    let element_map_partition = result
        .get("elementMapPartition")
        .and_then(|p| extract_bin_section(p.get("bin"), sections, tail).ok())
        .unwrap_or_default();
    Ok(CheckpointArtifacts {
        step,
        artifacts,
        element_map_partition,
        signatures,
        history_prefix_hash,
    })
}

/// The `(restored, drift_detected, snapshot_id)` a `RestoreCheckpoint` resp reports.
#[must_use]
pub fn parse_restore_checkpoint(result: &Value) -> (bool, bool, u64) {
    (
        result
            .get("restored")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        result
            .get("driftDetected")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        result
            .get("snapshotId")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Error mapping (SCHEMA §8) — NeedsRepair is NEVER here.
// ─────────────────────────────────────────────────────────────────────────────

/// Maps a wire [`ErrorObject`] to the core [`EngineError`] taxonomy (SCHEMA §8).
#[must_use]
pub fn map_error(err: &ErrorObject) -> EngineError {
    // The RAW wire terminal, before the engine taxonomy folds `retriable` away
    // (SCHEMA §8) — the only place a worker error's own code/retriable is visible.
    tracing::debug!(
        code = ?err.code,
        retriable = err.retriable,
        message = %err.message,
        "worker error frame"
    );
    let op = |code| EngineError::OpFailed {
        code,
        recoverable: true,
        message: err.message.clone(),
    };
    match err.code {
        ErrorCode::OpFailed => op(OpFailureCode::OpFailed),
        ErrorCode::RefUnresolved => op(OpFailureCode::RefUnresolved),
        ErrorCode::GeometryInvalid => op(OpFailureCode::GeometryInvalid),
        ErrorCode::Unsupported => op(OpFailureCode::Unsupported),
        ErrorCode::StalePreview => op(OpFailureCode::StalePreview),
        ErrorCode::Cancelled => EngineError::Cancelled,
        ErrorCode::ProtocolError => EngineError::Protocol {
            message: err.message.clone(),
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Solver lane — Sketch → SCHEMA §7.4 wire (the Rust `WireSketch` translator)
// ─────────────────────────────────────────────────────────────────────────────

/// Translates a core [`Sketch`] into the `(plane, entities, constraints)` wire
/// JSON the worker's `WireSketch::translate` consumes (SCHEMA §7.3 entity /
/// constraint shapes, §7.4 solver lane).
///
/// The core model references points **by id** (a [`Line`](SketchEntity::Line)
/// stores its two endpoint ids, an [`Arc`](SketchEntity::Arc)/[`Circle`](SketchEntity::Circle)/
/// [`Ellipse`](SketchEntity::Ellipse) its center id); this maps 1:1 onto the
/// worker's `p0Ref`/`p1Ref` line form and (for arc/circle/ellipse) an inlined
/// center coordinate resolved from the center point.
///
/// An ellipse is **not** registered with PlaneGCS by the worker (deliberate
/// legacy parity), so an ellipse-bearing sketch reports naively-counted DOF —
/// see SCHEMA §7.4. It still forms regions and extrudes with a true
/// `Geom_Ellipse` boundary.
#[must_use]
pub fn sketch_wire(sketch: &Sketch) -> (Value, Value, Value) {
    let plane = json!({
        "kind": plane_kind_str(sketch),
        "origin": [sketch.plane.origin.x, sketch.plane.origin.y, sketch.plane.origin.z],
        "xAxis": [sketch.plane.x_axis.x, sketch.plane.x_axis.y, sketch.plane.x_axis.z],
        "yAxis": [sketch.plane.y_axis.x, sketch.plane.y_axis.y, sketch.plane.y_axis.z],
        "normal": [sketch.plane.normal.x, sketch.plane.normal.y, sketch.plane.normal.z],
    });
    let entities: Vec<Value> = sketch
        .entities()
        .iter()
        .filter_map(|e| wire_entity(sketch, e))
        .collect();
    let constraints: Vec<Value> = sketch.constraints().iter().map(wire_constraint).collect();
    (plane, Value::Array(entities), Value::Array(constraints))
}

/// `SketchUpsert.args` (SCHEMA §7.4) for a core [`Sketch`].
#[must_use]
pub fn sketch_upsert_args(sketch: &Sketch) -> Value {
    let (plane, entities, constraints) = sketch_wire(sketch);
    json!({
        "sketchId": sketch.id.to_string(),
        "plane": plane,
        "entities": entities,
        "constraints": constraints,
    })
}

/// `BeginGesture.args` (SCHEMA §7.4). `drag_point` is the point entity being
/// dragged — its wire handle is its uuid (points register under their id).
///
/// `solver_policy_hash` is RESERVED wire plumbing: always `""` today (the sole
/// caller, `DocumentRuntime::begin_gesture`, hard-codes it) and never read by the
/// worker's `SolverLane::on_begin`. Kept for future solver-policy pinning
/// (client/worker solver-version mismatch guard) — do not remove; the wire slot
/// is part of the §7.4 args shape. Tracked backlog item, no consumer yet.
#[must_use]
pub fn begin_gesture_args(
    sketch_id: &str,
    sketch_revision: u64,
    gesture_id: u64,
    drag_point: EntityId,
    solver_policy_hash: &str,
) -> Value {
    json!({
        "sketchId": sketch_id,
        "sketchRevision": sketch_revision,
        "gestureId": gesture_id,
        "solverPolicyHash": solver_policy_hash,
        "drag": { "pointId": drag_point.to_string() },
        "pointId": drag_point.to_string(),
    })
}

/// `SolveDrag.args` (SCHEMA §7.4) — latest-wins incremental solve.
#[must_use]
pub fn solve_drag_args(gesture_id: u64, seq: u64, drag_point: EntityId, target: [f64; 2]) -> Value {
    json!({
        "gestureId": gesture_id,
        "seq": seq,
        "pointId": drag_point.to_string(),
        "target": [target[0], target[1]],
    })
}

/// `EndGesture.args` (SCHEMA §7.4) — pointer-up final exact solve.
#[must_use]
pub fn end_gesture_args(gesture_id: u64, final_target: Option<[f64; 2]>) -> Value {
    let mut args = json!({ "gestureId": gesture_id });
    if let Some(t) = final_target {
        args["commit"] = json!({ "finalTarget": [t[0], t[1]] });
    }
    args
}

/// `SketchRegions.args` (SCHEMA §7.4).
#[must_use]
pub fn sketch_regions_args(sketch_id: &str) -> Value {
    json!({ "sketchId": sketch_id })
}

/// Parses a `SketchUpsert`/`EndGesture` solve result into a [`SketchUpsertDto`].
/// `EndGesture` also carries a `positions` map (changed points since the gesture
/// began); `SketchUpsert` carries none (identity solve).
///
/// `SketchUpsert` reports the solve `state` (the four PascalCase tokens) directly;
/// `EndGesture` reports a drag `status` (`success`|`partial`|`conflicting`) + `dof`
/// instead, so the solve status is **derived** (`conflicting` ⇒ `Conflicting`, else
/// `dof == 0` ⇒ `FullyConstrained` else `UnderConstrained`).
#[must_use]
pub fn parse_sketch_upsert(sketch_id: &str, result: &Value) -> SketchUpsertDto {
    let dof = parse_dof(result);
    let status = if let Some(state) = result.get("state").and_then(Value::as_str) {
        SketchSolveStatus::parse(state)
    } else {
        match result.get("status").and_then(Value::as_str) {
            Some("conflicting") => SketchSolveStatus::Conflicting,
            _ if dof == 0 => SketchSolveStatus::FullyConstrained,
            _ => SketchSolveStatus::UnderConstrained,
        }
    };
    SketchUpsertDto {
        sketch_id: sketch_id.to_string(),
        sketch_revision: result
            .get("sketchRevision")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        dof,
        status,
        // SketchUpsert + EndGesture both carry `conflicting` (SCHEMA §7.4); absent
        // (legacy worker) ⇒ empty, strict back/forward compat.
        conflicting: str_array(result.get("conflicting")),
        solved_positions: parse_positions(result.get("positions")),
    }
}

/// Parses a `SolveDrag` result into a [`DragSolveDto`]. A stale `seq` may come
/// back `status:"superseded"` (latest-wins) — the caller tolerates it and drops
/// the (empty) positions.
#[must_use]
pub fn parse_solve_drag(result: &Value) -> DragSolveDto {
    let status = result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("success")
        .to_string();
    DragSolveDto {
        gesture_id: result.get("gestureId").and_then(Value::as_u64).unwrap_or(0),
        seq: result.get("seq").and_then(Value::as_u64).unwrap_or(0),
        superseded: status == "superseded",
        status,
        dof: parse_dof(result),
        conflicting: str_array(result.get("conflicting")),
        positions: parse_positions(result.get("positions")),
        solve_micros: result
            .get("solveMicros")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

/// Parses a `SketchRegions` result + its response binary tail into region DTOs.
/// Malformed metadata or geometry is a protocol failure, never an empty/partial
/// successful result.
///
/// # Errors
/// A protocol-contract reason for missing fields, invalid bins, or invalid
/// triangulation.
pub fn parse_sketch_regions(
    expected_sketch_id: &str,
    result: &Value,
    bin_sections: &[BinSection],
    tail: &[u8],
) -> Result<Vec<SketchRegionDto>, String> {
    validate_sketch_region_header(expected_sketch_id, result)?;
    let regions = result
        .get("regions")
        .and_then(Value::as_array)
        .ok_or("SketchRegions: missing/invalid regions array")?;
    if regions.is_empty() {
        if bin_sections.is_empty() && tail.is_empty() {
            return Ok(Vec::new());
        }
        return Err("SketchRegions: empty regions must have empty binary data".into());
    }

    let sections = validate_bin_sections("SketchRegions", bin_sections, tail)?;
    let mut region_ids = HashSet::with_capacity(regions.len());
    let mut referenced_bins = HashSet::with_capacity(regions.len());
    let mut parsed = Vec::with_capacity(regions.len());
    for (index, region) in regions.iter().enumerate() {
        let (dto, bin) = parse_sketch_region(index, region, &sections, tail)?;
        if !region_ids.insert(dto.region_id.clone()) {
            return Err(format!(
                "SketchRegions: duplicate regionId {:?}",
                dto.region_id
            ));
        }
        if !referenced_bins.insert(bin.clone()) {
            return Err(format!("SketchRegions: binary section {bin:?} reused"));
        }
        parsed.push(dto);
    }
    reject_unreferenced_sections("SketchRegions", &sections, &referenced_bins)?;
    Ok(parsed)
}

fn validate_sketch_region_header(expected_sketch_id: &str, result: &Value) -> Result<(), String> {
    let sketch_id = result
        .get("sketchId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("SketchRegions: missing/invalid sketchId")?;
    if sketch_id != expected_sketch_id {
        return Err(format!(
            "SketchRegions: sketchId mismatch; expected {expected_sketch_id:?}, got {sketch_id:?}"
        ));
    }
    result
        .get("sketchRevision")
        .and_then(Value::as_u64)
        .ok_or("SketchRegions: missing/invalid sketchRevision")?;
    Ok(())
}

fn parse_sketch_region(
    index: usize,
    region: &Value,
    sections: &HashMap<&str, &BinSection>,
    tail: &[u8],
) -> Result<(SketchRegionDto, String), String> {
    let context = format!("SketchRegions: region[{index}]");
    let region_id = required_nonempty_string(region.get("regionId"), &context, "regionId")?;
    let outer_loop =
        required_nonempty_string_array(region.get("outerLoop"), &context, "outerLoop")?;
    let holes = parse_region_holes(region.get("holes"), &context)?;
    let (triangles, bin) = parse_preview_triangles(
        region.get("previewTriangles"),
        holes.len(),
        sections,
        tail,
        &context,
    )?;
    Ok((
        SketchRegionDto {
            region_id,
            outer_loop,
            holes,
            preview_triangles: Some(triangles),
        },
        bin,
    ))
}

fn parse_region_holes(value: Option<&Value>, context: &str) -> Result<Vec<Vec<String>>, String> {
    let holes = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{context}: missing/invalid holes array"))?;
    holes
        .iter()
        .enumerate()
        .map(|(index, hole)| {
            required_nonempty_string_array(Some(hole), context, &format!("holes[{index}]"))
        })
        .collect()
}

fn required_nonempty_string_array(
    value: Option<&Value>,
    context: &str,
    field: &str,
) -> Result<Vec<String>, String> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{context}: missing/invalid {field} array"))?;
    if values.is_empty() {
        return Err(format!("{context}: {field} must not be empty"));
    }
    values
        .iter()
        .map(|value| required_nonempty_string(Some(value), context, field))
        .collect()
}

fn required_nonempty_string(
    value: Option<&Value>,
    context: &str,
    field: &str,
) -> Result<String, String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{context}: missing/invalid {field} string"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Element identity (SCHEMA §7.5) — AcquireElementIds / ResolveRefs
// ─────────────────────────────────────────────────────────────────────────────

/// `AcquireElementIds.args` (SCHEMA §7.5) — promote TopoKeys to persistent ids.
#[must_use]
pub fn acquire_element_ids_args(req: &AcquireRequest) -> Value {
    let picks: Vec<Value> = req
        .picks
        .iter()
        .map(|p| {
            let mut o = json!({ "topoKey": p.topo_key.as_str() });
            if let Some(anchor) = &p.anchor {
                o["anchor"] = anchor_to_wire(anchor);
            }
            o
        })
        .collect();
    json!({
        "snapshotId": req.snapshot_id.0,
        "bodyId": body_id_wire(req.body),
        "picks": picks,
    })
}

/// `QueryElement` args (SCHEMA §7.5) — look an element up inside a snapshot.
#[must_use]
pub fn query_element_args(snapshot: SnapshotId, body: BodyId, element: &str) -> Value {
    json!({
        "snapshotId": snapshot.0,
        // The worker's BodyStore is keyed `body_<uuid>` — the same wire-form rule
        // every other body-bearing param follows (M2-R).
        "bodyId": body_id_wire(body),
        "elementId": element,
    })
}

/// `QueryElement` args in the SCHEMA §7.5 **`{topoKey, bodyId}`** form.
///
/// `elementId` is deliberately ABSENT, not empty: the worker branches on the
/// PRESENCE of the key and returns early on that path, so including it would
/// shadow the topoKey lookup entirely.
#[must_use]
pub fn query_element_by_topo_key_args(snapshot: SnapshotId, body: BodyId, topo_key: &str) -> Value {
    json!({
        "snapshotId": snapshot.0,
        "bodyId": body_id_wire(body),
        "topoKey": topo_key,
    })
}

/// Parses a `QueryElement` result. `None` when the element is absent from the
/// snapshot (`present: false`), so a stale pick reads as "gone", not as a face at
/// the origin.
#[must_use]
pub fn parse_query_element(result: &Value) -> Option<crate::dto::ElementInfoDto> {
    if result.get("present").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let d = result.get("descriptor");
    let vec3 = |key: &str, fallback: [f64; 3]| -> [f64; 3] {
        d.and_then(|d| d.get(key))
            .and_then(Value::as_array)
            .filter(|a| a.len() >= 3)
            .map(|a| {
                let g = |i: usize| a[i].as_f64().unwrap_or(0.0);
                [g(0), g(1), g(2)]
            })
            .unwrap_or(fallback)
    };
    let str_at = |key: &str| {
        result
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let num_at = |key: &str, fallback: f64| -> f64 {
        d.and_then(|d| d.get(key))
            .and_then(Value::as_f64)
            .unwrap_or(fallback)
    };
    Some(crate::dto::ElementInfoDto {
        element_id: str_at("elementId"),
        topo_key: str_at("topoKey"),
        body_id: str_at("bodyId"),
        kind: str_at("kind"),
        surface_type: d
            .and_then(|d| d.get("surfaceType"))
            .and_then(Value::as_i64)
            // -1, not 0: 0 IS `GeomAbs_Plane`, so a missing descriptor must never
            // read as "this is a plane".
            .unwrap_or(-1),
        // Same rule for the curve type: 0 IS `GeomAbs_Line`.
        curve_type: d
            .and_then(|d| d.get("curveType"))
            .and_then(Value::as_i64)
            .unwrap_or(-1),
        center: vec3("center", [0.0, 0.0, 0.0]),
        normal: vec3("normal", [0.0, 0.0, 1.0]),
        has_normal: d
            .and_then(|d| d.get("hasNormal"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        // The worker has always emitted these (ElementMapPartition::descriptor_to_json);
        // MEASURE V1a is the first consumer, so they stop being dropped here.
        // 0.0 is the honest absence value for both — a measurement UI shows
        // "0 mm²" rather than a fabricated size.
        size: num_at("size", 0.0),
        magnitude: num_at("magnitude", 0.0),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Host-face boundary projection (SCHEMA §7.6 `ProjectFaceBoundary`)
// ─────────────────────────────────────────────────────────────────────────────

/// The addressing rung this request uses (SCHEMA §7.6): `elementId`, **else**
/// `{bodyId, topoKey}`.
///
/// The two are mutually exclusive on the wire, not merely preferred: the worker
/// branches on the PRESENCE of `elementId` (`FaceProjection.cpp resolve_seed`) and
/// answers `present:false` when that branch misses, so shipping both would let a
/// stale elementId shadow a perfectly good topoKey.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceAddress<'a> {
    /// The `elementId` branch — partition entry → `(bodyId, topoKey)` → sub-shape.
    ElementId(&'a str),
    /// The `{bodyId, topoKey}` branch — resolves against the body shape directly.
    TopoKey(&'a str),
}

impl FaceAddress<'_> {
    /// Renders this rung into `args`, writing exactly ONE of the two keys.
    fn write_into(self, args: &mut Value) {
        match self {
            Self::ElementId(id) => args["elementId"] = Value::String(id.to_string()),
            Self::TopoKey(key) => args["topoKey"] = Value::String(key.to_string()),
        }
    }
}

/// Which faces the projection covers (SCHEMA §7.6 `scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectionScope {
    /// The seed face alone.
    FaceOnly,
    /// The seed face plus every other face of the same body coplanar with the
    /// supplied plane (the wire default).
    #[default]
    CoplanarBody,
}

impl ProjectionScope {
    /// The wire token.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::FaceOnly => "faceOnly",
            Self::CoplanarBody => "coplanarBody",
        }
    }
}

/// `ProjectFaceBoundary` args for the **first** handshake round-trip (SCHEMA §7.6
/// `frameOnly`): the caller has no basis yet, so it asks only for the kernel-exact
/// frame. `plane`/`scope` are IGNORED in this mode and are deliberately not sent.
#[must_use]
pub fn project_face_boundary_frame_args(
    snapshot: SnapshotId,
    body: BodyId,
    address: FaceAddress<'_>,
) -> Value {
    let mut args = json!({
        "snapshotId": snapshot.0,
        // The worker's BodyStore is keyed `body_<uuid>` — the same wire-form rule
        // every other body-bearing param follows (M2-R). Required by BOTH rungs:
        // the elementId branch overwrites it from the partition entry, the topoKey
        // branch resolves against it.
        "bodyId": body_id_wire(body),
        "frameOnly": true,
    });
    address.write_into(&mut args);
    args
}

/// `ProjectFaceBoundary` args for the **second** round-trip: the real projection,
/// expressed in the basis Rust built from the `frameOnly` frame.
///
/// `plane` is INPUT and AUTHORITATIVE (SCHEMA §7.6) — every returned `at` and every
/// arc/circle centre is in ITS UV, and `coplanarBody` membership is tested against
/// IT, not against the seed face's own frame. `options` is omitted so the worker's
/// documented defaults apply.
#[must_use]
pub fn project_face_boundary_args(
    snapshot: SnapshotId,
    body: BodyId,
    address: FaceAddress<'_>,
    plane: &SketchPlane,
    scope: ProjectionScope,
) -> Value {
    let mut args = json!({
        "snapshotId": snapshot.0,
        "bodyId": body_id_wire(body),
        "frameOnly": false,
        "plane": {
            "origin": [plane.origin.x, plane.origin.y, plane.origin.z],
            "xAxis": [plane.x_axis.x, plane.x_axis.y, plane.x_axis.z],
            "yAxis": [plane.y_axis.x, plane.y_axis.y, plane.y_axis.z],
            "normal": [plane.normal.x, plane.normal.y, plane.normal.z],
        },
        "scope": scope.wire(),
    });
    address.write_into(&mut args);
    args
}

/// Reads a `[x, y, z]` wire vector, rejecting a wrong shape or a non-finite
/// component (SCHEMA §4).
fn projection_vec3(value: Option<&Value>, what: &str) -> Result<Vec3, String> {
    let a = value
        .and_then(Value::as_array)
        .filter(|a| a.len() == 3)
        .ok_or_else(|| format!("ProjectFaceBoundary: {what} must be a 3-number array"))?;
    let g = |i: usize| -> Result<f64, String> {
        a[i].as_f64()
            .ok_or_else(|| format!("ProjectFaceBoundary: {what}[{i}] is not a number"))
    };
    Vec3::new(g(0)?, g(1)?, g(2)?)
        .ok_or_else(|| format!("ProjectFaceBoundary: {what} has a non-finite component"))
}

/// Parses the mandatory `exact` frame (SCHEMA §7.6 — always present when
/// `present`, in BOTH modes).
fn parse_exact_frame(result: &Value) -> Result<FaceFrame, String> {
    let exact = result
        .get("exact")
        .ok_or_else(|| "ProjectFaceBoundary: present:true without an `exact` frame".to_string())?;
    Ok(FaceFrame {
        origin: projection_vec3(exact.get("origin"), "exact.origin")?,
        normal: projection_vec3(exact.get("normal"), "exact.normal")?,
    })
}

/// True when the result says the reference did not resolve (`present:false`).
///
/// `present` is required; a response missing it is malformed, and treating that
/// as "absent" would silently swallow a protocol break.
fn projection_present(result: &Value) -> Result<bool, String> {
    result
        .get("present")
        .and_then(Value::as_bool)
        .ok_or_else(|| "ProjectFaceBoundary: result is missing `present`".to_string())
}

/// Parses a `frameOnly` result. `Ok(None)` = `present:false` (a stale or absent
/// reference is an ANSWER, not an error — SCHEMA §7.6).
///
/// # Errors
/// A human-readable reason when the frame is missing or malformed (the caller
/// surfaces it as `PROTOCOL_ERROR`; a malformed frame must never be defaulted —
/// a fabricated `(0,0,0)/(0,0,1)` would sketch on the world XY plane and look
/// plausible).
pub fn parse_project_face_boundary_frame(result: &Value) -> Result<Option<FaceFrame>, String> {
    if !projection_present(result)? {
        return Ok(None);
    }
    parse_exact_frame(result).map(Some)
}

/// Resolves a response-local `p<N>` ref (SCHEMA §7.6) to an index into this
/// response's `points[]`. Refs are 0-based, scoped to ONE response, and are NOT
/// `ElementId`s — a ref that does not resolve is a protocol break, not a miss.
fn parse_point_ref(entity: &Value, field: &str, points: usize) -> Result<usize, String> {
    let raw = entity
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("ProjectFaceBoundary: entity is missing {field}"))?;
    let index: usize = raw
        .strip_prefix('p')
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("ProjectFaceBoundary: {field} {raw:?} is not a `p<N>` ref"))?;
    if index >= points {
        return Err(format!(
            "ProjectFaceBoundary: {field} {raw:?} is out of range ({points} points in this response)"
        ));
    }
    Ok(index)
}

/// Reads a required finite number off an entity.
fn projection_number(entity: &Value, field: &str) -> Result<f64, String> {
    entity
        .get(field)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("ProjectFaceBoundary: entity {field} must be a finite number"))
}

/// Parses a full `ProjectFaceBoundary` result into the pure core payload
/// [`ProjectionPayload`] the translator consumes. `Ok(None)` = `present:false`.
///
/// Every `p<N>` ref is resolved and bounds-checked HERE, so the pure translator
/// downstream never has to reason about wire shape.
///
/// # Errors
/// A human-readable reason for any malformed field. There is no lenient path: a
/// dropped boundary curve would produce an open profile that silently fails to
/// form a region later, far from the cause.
pub fn parse_project_face_boundary(result: &Value) -> Result<Option<ProjectionPayload>, String> {
    if !projection_present(result)? {
        return Ok(None);
    }
    let exact = parse_exact_frame(result)?;

    let mut points = Vec::new();
    for (i, p) in result
        .get("points")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let at = p
            .get("at")
            .and_then(Value::as_array)
            .filter(|a| a.len() == 2)
            .ok_or_else(|| {
                format!("ProjectFaceBoundary: points[{i}].at must be a 2-number array")
            })?;
        let g = |k: usize| -> Result<f64, String> {
            at[k]
                .as_f64()
                .ok_or_else(|| format!("ProjectFaceBoundary: points[{i}].at[{k}] is not a number"))
        };
        points.push(
            Vec2::new(g(0)?, g(1)?)
                .ok_or_else(|| format!("ProjectFaceBoundary: points[{i}].at is non-finite"))?,
        );
    }

    let mut entities = Vec::new();
    for e in result
        .get("entities")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let kind = e
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "ProjectFaceBoundary: entity is missing `type`".to_string())?;
        entities.push(match kind {
            "Line" => ProjectedEntity::Line {
                p0: parse_point_ref(e, "p0Ref", points.len())?,
                p1: parse_point_ref(e, "p1Ref", points.len())?,
            },
            "Circle" => ProjectedEntity::Circle {
                center: parse_point_ref(e, "centerRef", points.len())?,
                radius: projection_number(e, "radius")?,
            },
            "Arc" => ProjectedEntity::Arc {
                center: parse_point_ref(e, "centerRef", points.len())?,
                radius: projection_number(e, "radius")?,
                // Already the CCW-ordered pair (SCHEMA §7.6) — carried verbatim.
                start_angle: projection_number(e, "startAngle")?,
                end_angle: projection_number(e, "endAngle")?,
                ccw: e.get("ccw").and_then(Value::as_bool).unwrap_or(true),
            },
            other => {
                return Err(format!(
                    "ProjectFaceBoundary: unknown entity type {other:?} (SCHEMA §7.6 emits only \
                     Line/Circle/Arc — every other curve falls back to a Line polyline)"
                ))
            }
        });
    }

    Ok(Some(ProjectionPayload {
        exact,
        has_closed_boundary: result
            .get("hasClosedBoundary")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        face_count: result
            .get("faceCount")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .try_into()
            .unwrap_or(u32::MAX),
        points,
        entities,
    }))
}

/// Parses an `AcquireElementIds` result into worker evidence (Rust then mints the
/// ids via [`mint_element_ids`](onecad_core::regen::mint_element_ids)). A worker
/// `elementId` (echoed existing binding) rides through as `existing`. `fallback_body`
/// backs a malformed/absent `bodyId`.
#[must_use]
pub fn parse_acquire_evidence(result: &Value, fallback_body: BodyId) -> Vec<WorkerElementEvidence> {
    result
        .get("ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|e| WorkerElementEvidence {
                    topo_key: TopoKey::new(e.get("topoKey").and_then(Value::as_str).unwrap_or("")),
                    body: e
                        .get("bodyId")
                        .and_then(Value::as_str)
                        .and_then(|s| parse_body_id(s).ok())
                        .unwrap_or(fallback_body),
                    kind: parse_kind(e.get("kind").and_then(Value::as_str).unwrap_or("face")),
                    anchor: e
                        .get("anchor")
                        .and_then(|a| serde_json::from_value::<AnchorIntent>(a.clone()).ok()),
                    descriptor: e.get("descriptor").cloned(),
                    existing: e
                        .get("elementId")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(ElementId::new),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `ResolveRefs.args` (SCHEMA §7.5) — dry-run ladder for repair dialogs.
///
/// Each ref is rendered through [`element_ref_wire`] so its `primary.bodyId` crosses
/// in the worker's `body_<uuid>` form (SCHEMA §2) — the worker's `BodyStore` is keyed
/// `body_<opId>`, so a bare core-serde uuid would miss (`referenced body not found`,
/// the same body-form class as the M2 op-`params` defect; this path went un-exercised
/// against a real body until the M4a re-resolve gate).
#[must_use]
pub fn resolve_refs_args(req: &ResolveRequest) -> Value {
    let refs: Vec<Value> = req
        .refs
        .iter()
        .map(|r| {
            let mut o = element_ref_wire(&r.element);
            if let Some(map) = o.as_object_mut() {
                map.insert("refId".to_string(), json!(r.ref_id));
            }
            o
        })
        .collect();
    json!({ "snapshotId": req.snapshot_id.0, "refs": refs })
}

/// Parses a `ResolveRefs` result into core [`RefResolution`]s (SCHEMA §7.5).
///
/// `autoBind` carries the Rust-minted `elementId` in its own slot (empty when the
/// resolved element is unminted) plus the bound `topoKey` as evidence (SCHEMA §9 —
/// M4a autoBind-conformance fix; the worker no longer puts the topoKey in the
/// elementId slot). `unchanged` echoes the ref's bound `elementId`. `needsRepair`
/// carries the full [`RepairItem`] evidence.
#[must_use]
pub fn parse_resolve_refs(result: &Value) -> Vec<RefResolution> {
    result
        .get("resolutions")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_one_resolution).collect())
        .unwrap_or_default()
}

/// Reads a non-empty string field as an [`ElementId`] (empty/absent ⇒ `None`).
fn opt_element_id(r: &Value, key: &str) -> Option<ElementId> {
    r.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ElementId::new)
}

fn parse_one_resolution(r: &Value) -> Option<RefResolution> {
    let ref_id = r.get("refId").and_then(Value::as_str)?.to_string();
    let outcome = match r.get("outcome").and_then(Value::as_str)? {
        "autoBind" => {
            // SCHEMA §7.5 strict slot: `elementId` is the bound Rust-minted id (empty
            // ⇒ None here). The bound `topoKey` rides as EVIDENCE (SCHEMA §9). A
            // one-release tolerance: a legacy worker that emitted only `topoKey` (no
            // `elementId`) still parses — the topoKey lands as evidence, elementId None.
            ResolveOutcome::AutoBind {
                element_id: opt_element_id(r, "elementId").unwrap_or_else(|| ElementId::new("")),
                score: r.get("score").and_then(Value::as_f64).unwrap_or(0.0),
                margin: r.get("margin").and_then(Value::as_f64).unwrap_or(0.0),
                topo_key: r
                    .get("topoKey")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(TopoKey::new),
            }
        }
        "unchanged" => ResolveOutcome::Unchanged {
            element_id: opt_element_id(r, "elementId"),
        },
        "needsRepair" => {
            let mut obj = r.get("needsRepair").cloned().unwrap_or_else(|| json!({}));
            if let Some(map) = obj.as_object_mut() {
                map.entry("stepIndex".to_string()).or_insert(json!(0));
                map.entry("refId".to_string()).or_insert(json!(ref_id));
            }
            ResolveOutcome::NeedsRepair(serde_json::from_value::<RepairItem>(obj).ok()?)
        }
        _ => return None,
    };
    Some(RefResolution { ref_id, outcome })
}

// ─────────────────────────────────────────────────────────────────────────────
// Solver / identity helpers
// ─────────────────────────────────────────────────────────────────────────────

fn plane_kind_str(sketch: &Sketch) -> &'static str {
    match &sketch.attachment {
        SketchAttachment::World { plane } => match plane {
            WorldPlane::XY => "XY",
            WorldPlane::XZ => "XZ",
            WorldPlane::YZ => "YZ",
        },
        // Datum / host-face frames carry a resolved custom basis.
        _ => "custom",
    }
}

/// The `[x, y]` position of a point entity (for inlining arc/circle centers).
fn point_pos(sketch: &Sketch, id: EntityId) -> Option<[f64; 2]> {
    match sketch.get_entity(id)? {
        SketchEntity::Point { at, .. } => Some([at.x, at.y]),
        _ => None,
    }
}

/// Adds `"referenceLocked": true` to an already-built entity object, and NOTHING
/// when the flag is false (SCHEMA §7.3: optional, default `false`). Omitting the
/// false case keeps the wire byte-identical for every sketch authored before the
/// flag existed — which is all of them, since nothing produces locked geometry yet.
fn tag_reference_locked(mut v: Value, e: &SketchEntity) -> Value {
    if e.is_reference_locked() {
        v["referenceLocked"] = json!(true);
    }
    v
}

fn wire_entity(sketch: &Sketch, e: &SketchEntity) -> Option<Value> {
    Some(tag_reference_locked(
        match e {
            SketchEntity::Point {
                id,
                at,
                construction,
                ..
            } => json!({
                "id": id.to_string(), "type": "Point",
                "at": [at.x, at.y], "construction": construction,
            }),
            SketchEntity::Line {
                id,
                start,
                end,
                construction,
                ..
            } => json!({
                "id": id.to_string(), "type": "Line",
                "p0Ref": start.to_string(), "p1Ref": end.to_string(), "construction": construction,
            }),
            SketchEntity::Circle {
                id,
                center,
                radius,
                construction,
                ..
            } => {
                let c = point_pos(sketch, *center)?;
                // `centerRef` carries the center point's uuid alongside the inlined coords
                // so re-entry hydration can re-own the center Point (BUG-5). The worker
                // ignores the extra key; the frontend consumes it (`sketchWireMap.ts`).
                json!({
                    "id": id.to_string(), "type": "Circle",
                    "center": c, "centerRef": center.to_string(),
                    "radius": radius, "construction": construction,
                })
            }
            SketchEntity::Arc {
                id,
                center,
                radius,
                start_angle,
                end_angle,
                construction,
                ..
            } => {
                let c = point_pos(sketch, *center)?;
                json!({
                    "id": id.to_string(), "type": "Arc",
                    "center": c, "centerRef": center.to_string(), "radius": radius,
                    "startAngle": start_angle, "endAngle": end_angle, "construction": construction,
                })
            }
            SketchEntity::Ellipse {
                id,
                center,
                major_r,
                minor_r,
                rotation,
                construction,
                ..
            } => {
                let c = point_pos(sketch, *center)?;
                // Same shape as Circle: inlined center coords + the informational
                // `centerRef` uuid so re-entry hydration re-owns the center Point.
                // `rotation` is RADIANS on the wire (core stores radians too — the
                // UI converts at its own boundary via `angleUnits.ts`).
                json!({
                    "id": id.to_string(), "type": "Ellipse",
                    "center": c, "centerRef": center.to_string(),
                    "majorR": major_r, "minorR": minor_r, "rotation": rotation,
                    "construction": construction,
                })
            }
        },
        e,
    ))
}

fn wire_constraint(c: &Constraint) -> Value {
    let s = |id: &EntityId| id.to_string();
    match c {
        Constraint::Coincident {
            point1,
            point2,
            point1_position,
            point2_position,
            ..
        } => {
            let mut v = json!({
                "id": cid(c), "type": "Coincident", "entities": [s(point1), s(point2)],
            });
            // `positions` is emitted ONLY when a side names an arc endpoint. A
            // plain point-to-point Coincident (every one authored before W0b)
            // keeps its exact previous wire shape — and an `Arbitrary` slot is
            // spelled `""`, not `"Arbitrary"`, because the worker's
            // `WireIndex::resolve_point` treats an EMPTY role as "the entity is
            // itself the point" (a literal "arbitrary" role would miss the
            // handle map and only survive on its fallback).
            if !point1_position.is_arbitrary() || !point2_position.is_arbitrary() {
                v["positions"] = json!([
                    curve_position_role(*point1_position),
                    curve_position_role(*point2_position),
                ]);
            }
            v
        }
        Constraint::Horizontal { line, .. } => {
            json!({ "id": cid(c), "type": "Horizontal", "entities": [s(line)] })
        }
        Constraint::Vertical { line, .. } => {
            json!({ "id": cid(c), "type": "Vertical", "entities": [s(line)] })
        }
        Constraint::Fixed { point, .. } => {
            json!({ "id": cid(c), "type": "Fixed", "entities": [s(point)] })
        }
        Constraint::Midpoint { point, line, .. } => {
            json!({ "id": cid(c), "type": "Midpoint", "entities": [s(point), s(line)] })
        }
        Constraint::OnCurve {
            point,
            curve,
            position,
            ..
        } => json!({
            "id": cid(c), "type": "OnCurve",
            "entities": [s(point), s(curve)],
            "positions": ["", curve_position_str(*position)],
        }),
        Constraint::Parallel { line1, line2, .. } => {
            json!({ "id": cid(c), "type": "Parallel", "entities": [s(line1), s(line2)] })
        }
        Constraint::Perpendicular { line1, line2, .. } => {
            json!({ "id": cid(c), "type": "Perpendicular", "entities": [s(line1), s(line2)] })
        }
        Constraint::Tangent {
            entity1, entity2, ..
        } => json!({ "id": cid(c), "type": "Tangent", "entities": [s(entity1), s(entity2)] }),
        Constraint::Concentric {
            entity1, entity2, ..
        } => json!({ "id": cid(c), "type": "Concentric", "entities": [s(entity1), s(entity2)] }),
        Constraint::Equal {
            entity1, entity2, ..
        } => json!({ "id": cid(c), "type": "Equal", "entities": [s(entity1), s(entity2)] }),
        Constraint::Distance {
            entity1,
            entity2,
            value,
            ..
        } => json!({
            "id": cid(c), "type": "Distance",
            "entities": [s(entity1), s(entity2)], "value": value.value,
        }),
        Constraint::HorizontalDistance {
            point1,
            point2,
            value,
            ..
        } => json!({
            "id": cid(c), "type": "HorizontalDistance",
            "entities": [s(point1), s(point2)], "value": value.value,
        }),
        Constraint::VerticalDistance {
            point1,
            point2,
            value,
            ..
        } => json!({
            "id": cid(c), "type": "VerticalDistance",
            "entities": [s(point1), s(point2)], "value": value.value,
        }),
        Constraint::Angle {
            line1,
            line2,
            value,
            ..
        } => json!({
            "id": cid(c), "type": "Angle",
            "entities": [s(line1), s(line2)], "value": value.value,
        }),
        Constraint::Radius { entity, value, .. } => json!({
            "id": cid(c), "type": "Radius", "entities": [s(entity)], "value": value.value,
        }),
        Constraint::Diameter { entity, value, .. } => json!({
            "id": cid(c), "type": "Diameter", "entities": [s(entity)], "value": value.value,
        }),
        Constraint::Symmetric {
            point1,
            point2,
            axis,
            ..
        } => json!({
            "id": cid(c), "type": "Symmetric", "entities": [s(point1), s(point2), s(axis)],
        }),
    }
}

fn cid(c: &Constraint) -> String {
    c.id().to_string()
}

fn curve_position_str(p: CurvePosition) -> &'static str {
    match p {
        CurvePosition::Start => "Start",
        CurvePosition::End => "End",
        CurvePosition::Arbitrary => "Arbitrary",
    }
}

/// Same tokens, but `Arbitrary` renders as the EMPTY role — the spelling a
/// `positions` slot uses when the operand is a point entity in its own right.
fn curve_position_role(p: CurvePosition) -> &'static str {
    match p {
        CurvePosition::Arbitrary => "",
        other => curve_position_str(other),
    }
}

fn anchor_to_wire(anchor: &AnchorIntent) -> Value {
    serde_json::to_value(anchor).unwrap_or_else(|_| json!({}))
}

fn parse_dof(result: &Value) -> u32 {
    result
        .get("dof")
        .and_then(Value::as_i64)
        .map(|d| d.max(0) as u32)
        .unwrap_or(0)
}

/// Parses a solver `positions` map (`{handle: [x, y]}`), keyed by the point
/// entity id (the wire handle for a point).
fn parse_positions(v: Option<&Value>) -> BTreeMap<String, [f64; 2]> {
    let Some(obj) = v.and_then(Value::as_object) else {
        return BTreeMap::new();
    };
    obj.iter()
        .filter_map(|(k, xy)| {
            let a = xy.as_array()?;
            let x = a.first()?.as_f64()?;
            let y = a.get(1)?.as_f64()?;
            Some((k.clone(), [x, y]))
        })
        .collect()
}

/// Validates the frame-level binary section table shared by region and mesh
/// parsers. Alignment gaps are permitted; duplicate names, overlap, overflow,
/// and out-of-tail sections are not.
pub(super) fn validate_bin_sections<'a>(
    context: &str,
    sections: &'a [BinSection],
    tail: &[u8],
) -> Result<HashMap<&'a str, &'a BinSection>, String> {
    if sections.is_empty() && !tail.is_empty() {
        return Err(format!(
            "{context}: binary tail present without named sections"
        ));
    }
    let mut by_name = HashMap::with_capacity(sections.len());
    let mut intervals = Vec::with_capacity(sections.len());
    for section in sections {
        if section.name.is_empty() {
            return Err(format!("{context}: empty binary section name"));
        }
        if by_name.insert(section.name.as_str(), section).is_some() {
            return Err(format!(
                "{context}: duplicate binary section {:?}",
                section.name
            ));
        }
        let start = section.off as usize;
        let end = start
            .checked_add(section.len as usize)
            .ok_or_else(|| format!("{context}: binary section range overflow"))?;
        if end > tail.len() {
            return Err(format!(
                "{context}: binary section {:?} out of bounds",
                section.name
            ));
        }
        intervals.push((start, end, section.name.as_str()));
    }
    intervals.sort_unstable_by_key(|interval| interval.0);
    for pair in intervals.windows(2) {
        if pair[1].0 < pair[0].1 {
            return Err(format!(
                "{context}: binary sections {:?} and {:?} overlap",
                pair[0].2, pair[1].2
            ));
        }
    }
    Ok(by_name)
}

pub(super) fn reject_unreferenced_sections(
    context: &str,
    sections: &HashMap<&str, &BinSection>,
    referenced: &HashSet<String>,
) -> Result<(), String> {
    let mut unreferenced: Vec<&str> = sections
        .keys()
        .copied()
        .filter(|name| !referenced.contains(*name))
        .collect();
    unreferenced.sort_unstable();
    if unreferenced.is_empty() && referenced.len() == sections.len() {
        return Ok(());
    }
    Err(format!(
        "{context}: unreferenced/unexpected binary sections {unreferenced:?}"
    ))
}

/// Decodes one required region `previewTriangles` section (f32 xyz vertices then
/// u32 indices) into planar `(u,v)` positions + triangle indices.
fn parse_preview_triangles(
    v: Option<&Value>,
    hole_count: usize,
    sections: &HashMap<&str, &BinSection>,
    tail: &[u8],
    context: &str,
) -> Result<(PreviewTrianglesDto, String), String> {
    let pt = v
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{context}: missing/invalid previewTriangles object"))?;
    if pt.get("format").and_then(Value::as_str) != Some("f32xyz+u32idx") {
        return Err(format!("{context}: invalid previewTriangles format"));
    }
    let section_name = required_nonempty_string(pt.get("bin"), context, "previewTriangles.bin")?;
    let vertex_count = required_nonzero_count(pt.get("vertexCount"), context, "vertexCount")?;
    let triangle_count = required_nonzero_count(pt.get("triangleCount"), context, "triangleCount")?;
    let holes_subtracted = required_holes_subtracted(pt.get("holesSubtracted"), context)?;
    if holes_subtracted as usize != hole_count {
        return Err(format!(
            "{context}: holesSubtracted {holes_subtracted} != holes.len() {hole_count}"
        ));
    }
    let section = sections
        .get(section_name.as_str())
        .copied()
        .ok_or_else(|| format!("{context}: missing binary section {section_name:?}"))?;
    let expected_len = preview_triangle_payload_len(vertex_count, triangle_count, context)?;
    if section.len as usize != expected_len {
        return Err(format!(
            "{context}: binary section length {} != expected {expected_len}",
            section.len
        ));
    }
    let start = section.off as usize;
    let bytes = tail
        .get(start..start + expected_len)
        .ok_or_else(|| format!("{context}: binary section out of bounds"))?;
    let positions = decode_region_positions(bytes, vertex_count, context)?;
    let indices = decode_region_indices(bytes, vertex_count, triangle_count, context)?;
    Ok((
        PreviewTrianglesDto {
            positions,
            indices,
            holes_subtracted,
        },
        section_name,
    ))
}

fn required_nonzero_count(
    value: Option<&Value>,
    context: &str,
    field: &str,
) -> Result<usize, String> {
    let raw = value
        .and_then(Value::as_u64)
        .filter(|count| *count > 0)
        .ok_or_else(|| format!("{context}: missing/invalid nonzero {field}"))?;
    usize::try_from(raw).map_err(|_| format!("{context}: {field} exceeds usize"))
}

fn required_holes_subtracted(value: Option<&Value>, context: &str) -> Result<u32, String> {
    let raw = value
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{context}: missing/invalid holesSubtracted"))?;
    u32::try_from(raw).map_err(|_| format!("{context}: holesSubtracted exceeds u32"))
}

fn preview_triangle_payload_len(
    vertex_count: usize,
    triangle_count: usize,
    context: &str,
) -> Result<usize, String> {
    let vertex_bytes = vertex_count
        .checked_mul(12)
        .ok_or_else(|| format!("{context}: vertex byte count overflow"))?;
    let index_bytes = triangle_count
        .checked_mul(3)
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(|| format!("{context}: index byte count overflow"))?;
    vertex_bytes
        .checked_add(index_bytes)
        .ok_or_else(|| format!("{context}: triangle payload length overflow"))
}

fn decode_region_positions(
    bytes: &[u8],
    vertex_count: usize,
    context: &str,
) -> Result<Vec<f64>, String> {
    let mut positions = Vec::with_capacity(vertex_count * 2);
    for index in 0..vertex_count {
        let base = index * 12;
        let xyz = [
            read_f32(bytes, base, context)?,
            read_f32(bytes, base + 4, context)?,
            read_f32(bytes, base + 8, context)?,
        ];
        if xyz.iter().any(|value| !value.is_finite()) {
            return Err(format!("{context}: vertex[{index}] is non-finite"));
        }
        if xyz[2] != 0.0 {
            return Err(format!("{context}: vertex[{index}] is not plane-local"));
        }
        positions.extend([f64::from(xyz[0]), f64::from(xyz[1])]);
    }
    Ok(positions)
}

fn decode_region_indices(
    bytes: &[u8],
    vertex_count: usize,
    triangle_count: usize,
    context: &str,
) -> Result<Vec<u32>, String> {
    let index_count = triangle_count * 3;
    let index_base = vertex_count * 12;
    let mut indices = Vec::with_capacity(index_count);
    for position in 0..index_count {
        let index = read_u32(bytes, index_base + position * 4, context)?;
        if index as u64 >= vertex_count as u64 {
            return Err(format!(
                "{context}: triangle index {index} outside vertexCount {vertex_count}"
            ));
        }
        indices.push(index);
    }
    Ok(indices)
}

fn read_f32(bytes: &[u8], offset: usize, context: &str) -> Result<f32, String> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{context}: truncated f32"))?
        .try_into()
        .map_err(|_| format!("{context}: invalid f32 width"))?;
    Ok(f32::from_le_bytes(raw))
}

fn read_u32(bytes: &[u8], offset: usize, context: &str) -> Result<u32, String> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{context}: truncated u32"))?
        .try_into()
        .map_err(|_| format!("{context}: invalid u32 width"))?;
    Ok(u32::from_le_bytes(raw))
}

// ─────────────────────────────────────────────────────────────────────────────
// Small helpers
// ─────────────────────────────────────────────────────────────────────────────

fn parse_kind(s: &str) -> ElementKind {
    match s {
        "edge" => ElementKind::Edge,
        "vertex" => ElementKind::Vertex,
        _ => ElementKind::Face,
    }
}

fn str_array(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn u64_at(v: Option<&Value>, key: &str) -> u64 {
    v.and_then(|o| o.get(key))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MEASURE V1a: `magnitude` / `size` / `curveType` used to be DROPPED here.
    #[test]
    fn parse_query_element_keeps_the_measurable_descriptor_fields() {
        let result = json!({
            "elementId": "el_7",
            "topoKey": "f:22",
            "bodyId": "body_1",
            "kind": "face",
            "present": true,
            "descriptor": {
                "center": [1.0, 2.0, 3.0],
                "normal": [0.0, 0.0, 1.0],
                "hasNormal": true,
                "surfaceType": 0,
                "curveType": 0,
                "size": 44.72,
                "magnitude": 800.0,
            },
        });
        let info = parse_query_element(&result).expect("present element");
        assert_eq!(info.element_id, "el_7");
        assert_eq!(info.kind, "face");
        assert_eq!(info.surface_type, 0);
        assert_eq!(info.curve_type, 0);
        assert!((info.magnitude - 800.0).abs() < 1e-9);
        assert!((info.size - 44.72).abs() < 1e-9);
    }

    /// A descriptor that carries no curve type must read as −1, NOT 0 —
    /// `GeomAbs_Line` is 0, so a defaulted 0 would claim "this is a line"
    /// (the same trap `surface_type` already guards against for `GeomAbs_Plane`).
    #[test]
    fn parse_query_element_missing_type_ordinals_are_negative_one() {
        let result = json!({
            "elementId": "el_9",
            "kind": "edge",
            "descriptor": { "magnitude": 40.0 },
        });
        let info = parse_query_element(&result).expect("present element");
        assert_eq!(info.surface_type, -1);
        assert_eq!(info.curve_type, -1);
        assert!((info.magnitude - 40.0).abs() < 1e-9);
        assert!((info.size - 0.0).abs() < 1e-9);
    }

    /// `present: false` (a stale pick after an edit) stays `None`, so a vanished
    /// element can never be measured as a zero-area face at the origin.
    #[test]
    fn parse_query_element_absent_is_none() {
        assert!(parse_query_element(&json!({ "elementId": "el_9", "present": false })).is_none());
    }

    #[test]
    fn body_id_round_trips_through_wire() {
        let b = BodyId(Uuid::from_u128(0x4a1));
        let wire = body_id_wire(b);
        assert!(wire.starts_with("body_"));
        assert_eq!(parse_body_id(&wire).unwrap(), b);
    }

    #[test]
    fn job_id_wire_is_lossless_for_counter_minted_ids() {
        // JobId collision-safety invariant: counter-minted ids (u128::from(u64))
        // truncate to u64 losslessly, so distinct counters ⇒ distinct wire ids.
        for counter in [0u64, 1, 2, 88, u32::MAX as u64, u64::MAX] {
            let job = JobId(Uuid::from_u128(u128::from(counter)));
            assert_eq!(job_id_wire(job), counter, "wire id must equal the counter");
        }
        // A large counter and its successor never collide on the wire.
        let a = JobId(Uuid::from_u128(u128::from(u64::MAX - 1)));
        let b = JobId(Uuid::from_u128(u128::from(u64::MAX)));
        assert_ne!(job_id_wire(a), job_id_wire(b));
    }

    #[test]
    fn parse_body_id_rejects_bad_shapes() {
        assert!(parse_body_id("body_not-a-uuid").is_err());
        assert!(parse_body_id("7").is_err(), "missing prefix");
        assert!(
            parse_body_id("body_00000000-0000-0000-0000-000000000001:x").is_err(),
            "non-integer ordinal"
        );
        assert!(
            parse_body_id("body_not-a-uuid:0").is_err(),
            "split child with non-uuid opId"
        );
    }

    // Serializes the interner-mutating tests: the split interner is a process-global,
    // and `clear_split_interner_for_test` wipes it, so two such tests must not race.
    static INTERNER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn split_child_body_id_round_trips_and_is_deterministic() {
        let _guard = INTERNER_TEST_LOCK.lock().unwrap();
        // A split child parses to a DERIVED uuid (not the opId), and body_id_wire
        // rebuilds the EXACT `body_<opId>:<k>` string via the interner (SCHEMA §2).
        let op = Uuid::from_u128(0xABCD);
        let s0 = format!("body_{op}:0");
        let s1 = format!("body_{op}:1");
        let child0 = parse_body_id(&s0).unwrap();
        let child1 = parse_body_id(&s1).unwrap();
        assert_ne!(child0, child1, "distinct ordinals → distinct ids");
        assert_ne!(child0, BodyId(op), "child id is NOT the opId uuid");
        assert_eq!(
            body_id_wire(child0),
            s0,
            "round-trips to the exact wire form"
        );
        assert_eq!(body_id_wire(child1), s1);
        // Deterministic (pure function of opId + k) — a re-parse yields the same id.
        assert_eq!(parse_body_id(&s0).unwrap(), child0);
        // split_parts recovers (opId, k) for adoption.
        assert_eq!(split_parts(child0), Some((op, 0)));
        assert_eq!(split_parts(child1), Some((op, 1)));
        assert_eq!(
            split_parts(BodyId(op)),
            None,
            "a plain NewBody id is not a split child"
        );
    }

    #[test]
    fn split_child_reinterns_after_fresh_process_simulation() {
        let _guard = INTERNER_TEST_LOCK.lock().unwrap();
        // A worker mint parses the child + interns it (the warm-process path).
        let op = Uuid::from_u128(0xF00D);
        let child = parse_body_id(&format!("body_{op}:1")).unwrap();
        assert_eq!(body_id_wire(child), format!("body_{op}:1"));

        // Simulate a FRESH PROCESS: nothing parsed yet, interner empty.
        clear_split_interner_for_test();
        // PRE-FIX behavior (the gap): body_id_wire MISSES and renders the bare derived
        // uuid — an id the worker never minted ⇒ REF_UNRESOLVED for any downstream ref.
        assert_eq!(
            body_id_wire(child),
            format!("body_{}", child.0),
            "pre-fix: a cold interner renders the wrong (bare-uuid) id"
        );
        assert_ne!(body_id_wire(child), format!("body_{op}:1"));

        // THE FIX: re-intern from the persisted split_of (opId, k) — exactly what
        // DocumentRuntime does at open — and the render is correct again.
        let rederived = intern_split_child(op, 1);
        assert_eq!(
            rederived, child,
            "re-intern derives the SAME BodyId (deterministic)"
        );
        assert_eq!(
            body_id_wire(child),
            format!("body_{op}:1"),
            "fixed: the persisted split_of repopulates the interner"
        );
        clear_split_interner_for_test(); // leave the interner clean for other tests
    }

    #[test]
    fn high_ordinal_child_full_identity_round_trip() {
        let _guard = INTERNER_TEST_LOCK.lock().unwrap();
        // WP-0: ordinal 300 exceeds the deleted 256-probe cap. The whole chain —
        // parse → registry origin stamp → fresh-process re-intern → wire render —
        // must hold for ANY ordinal, or a >256-solid op (multi-solid STEP import)
        // silently loses bodies on reopen.
        use onecad_core::document::body::{BodyLifecycleEvent, BodyRegistry, SplitOrigin};
        use onecad_core::ids::RecordId;
        let op = Uuid::from_u128(0xBEEF);
        let k = 300;
        let child = parse_body_id(&format!("body_{op}:{k}")).unwrap();

        // Core registry stamps the exact origin (this is what document.json persists).
        let by = RecordId(op);
        let mut reg = BodyRegistry::new();
        reg.fold(0, by, BodyLifecycleEvent::Created { body: child });
        let origin = reg.get(child).expect("registered").split_of;
        assert_eq!(
            origin,
            Some(SplitOrigin { op: by, k }),
            "origin exact at k=300"
        );

        // Fresh process: re-intern from the persisted origin → exact wire form.
        clear_split_interner_for_test();
        let o = origin.unwrap();
        let rederived = intern_split_child(o.op.as_uuid(), o.k);
        assert_eq!(
            rederived, child,
            "persisted origin re-derives the same BodyId"
        );
        assert_eq!(body_id_wire(child), format!("body_{op}:{k}"));
        clear_split_interner_for_test();
    }

    #[test]
    fn plan_step_parses_created_body_and_delta() {
        let op = Uuid::from_u128(0x10);
        let payload = json!({
            "stepIndex": 3,
            "bodyEvents": [ { "kind": "created", "bodyId": format!("body_{op}") } ],
            "elementMapDelta": {
                "added": [ { "elementId": "el_1", "topoKey": "f:2", "kind": "face", "bodyId": format!("body_{op}") } ],
                "removed": ["el_9"], "relabeled": []
            },
            "needsRepair": [],
            "signatures": { "geometry": "aa", "bodyLifecycle": "bb", "referencedBinding": "cc" },
            "diagnostics": [ { "severity": "warning", "code": "X", "message": "m" } ]
        });
        let step = parse_plan_step(&payload, 0).unwrap();
        assert_eq!(step.step_index, 3);
        assert!(
            matches!(step.body_events[0], BodyLifecycleEvent::Created { body } if body == BodyId(op))
        );
        assert_eq!(step.element_map_delta.added[0].body, BodyId(op));
        assert_eq!(step.element_map_delta.removed[0], ElementId::new("el_9"));
        assert_eq!(step.signatures.geometry.as_str(), "aa");
        assert_eq!(step.diagnostics.len(), 1);
    }

    #[test]
    fn plan_step_rejects_unadoptable_body_id() {
        let payload = json!({
            "stepIndex": 0,
            "bodyEvents": [ { "kind": "created", "bodyId": "body_bogus" } ]
        });
        assert!(parse_plan_step(&payload, 0).is_err());
    }

    #[test]
    fn needs_repair_injects_step_index_and_keeps_scoring_version() {
        let payload = json!({
            "stepIndex": 5,
            "needsRepair": [ {
                "refId": "op_5.input0", "ladderFailed": "descriptor", "reason": "ambiguous",
                "scoringVersion": 1, "candidates": []
            } ]
        });
        let step = parse_plan_step(&payload, 0).unwrap();
        assert_eq!(step.needs_repair.len(), 1);
        assert_eq!(step.needs_repair[0].step_index, 5);
        assert_eq!(step.needs_repair[0].scoring_version, Some(1));
    }

    #[test]
    fn plan_prepared_parses_terminal() {
        let job = JobId(Uuid::from_u128(7));
        let op = Uuid::from_u128(0x10);
        let result = json!({
            "planPrepared": true, "preparedSnapshotId": 5013, "lastValidStep": 6,
            "stoppedReason": "completed",
            "perStepResults": [ { "stepIndex": 6, "status": "ok", "bodyIds": [ format!("body_{op}") ] } ],
            "historyPrefixHash": "9c4d"
        });
        let p = parse_plan_prepared(job, &result).unwrap();
        assert_eq!(p.job_id, job);
        assert_eq!(p.prepared_snapshot_id, SnapshotId(5013));
        assert_eq!(p.last_valid_step, Some(6));
        assert_eq!(p.stopped_reason, StoppedReason::Completed);
        assert_eq!(p.per_step[0].body_ids[0], BodyId(op));
        assert_eq!(p.history_prefix_hash.as_str(), "9c4d");
    }

    #[test]
    fn plan_prepared_base_only_last_valid_null() {
        let job = JobId(Uuid::from_u128(1));
        let result = json!({
            "preparedSnapshotId": 1, "lastValidStep": null, "stoppedReason": "needsRepair",
            "perStepResults": [], "historyPrefixHash": "e3b0"
        });
        let p = parse_plan_prepared(job, &result).unwrap();
        assert_eq!(p.last_valid_step, None);
        assert_eq!(p.stopped_reason, StoppedReason::NeedsRepair);
    }

    #[test]
    fn error_mapping_keeps_needs_repair_out() {
        let e = map_error(&ErrorObject {
            code: ErrorCode::ProtocolError,
            message: "boom".into(),
            detail: None,
            retriable: false,
        });
        assert!(matches!(e, EngineError::Protocol { .. }));
        let e = map_error(&ErrorObject {
            code: ErrorCode::OpFailed,
            message: "x".into(),
            detail: None,
            retriable: false,
        });
        assert!(matches!(
            e,
            EngineError::OpFailed {
                recoverable: true,
                ..
            }
        ));
    }
}

#[cfg(test)]
mod solver_wire_tests {
    use super::*;
    use onecad_core::document::variables::Scalar;
    use onecad_core::ids::{ConstraintId, SketchId};
    use onecad_core::math::Vec2;
    use onecad_core::regen::Pick;
    use onecad_core::sketch::{Constraint, Sketch, SketchEntity, WorldPlane};

    fn eid(n: u128) -> EntityId {
        EntityId(Uuid::from_u128(n))
    }
    fn cid(n: u128) -> ConstraintId {
        ConstraintId(Uuid::from_u128(n))
    }

    fn valid_region_response() -> (Value, Vec<BinSection>, Vec<u8>) {
        let mut tail = Vec::new();
        for xyz in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for value in xyz {
                tail.extend_from_slice(&value.to_le_bytes());
            }
        }
        for index in [0_u32, 1, 2] {
            tail.extend_from_slice(&index.to_le_bytes());
        }
        let sections = vec![BinSection {
            name: "region:r0".into(),
            off: 0,
            len: tail.len() as u32,
        }];
        let result = json!({
            "sketchId": "sk_1",
            "sketchRevision": 1,
            "regions": [{
                "regionId": "r0",
                "outerLoop": ["outer"],
                "holes": [],
                "previewTriangles": {
                    "format": "f32xyz+u32idx",
                    "bin": "region:r0",
                    "vertexCount": 3,
                    "triangleCount": 1,
                    "holesSubtracted": 0
                }
            }]
        });
        (result, sections, tail)
    }

    fn assert_region_parse_error(result: &Value, sections: &[BinSection], tail: &[u8]) {
        assert!(
            parse_sketch_regions("sk_1", result, sections, tail).is_err(),
            "malformed SketchRegions response must fail closed"
        );
    }

    /// A point-referenced line + a circle (center inlined) + two constraints,
    /// translated to the worker `WireSketch` shapes (SCHEMA §7.3/§7.4).
    #[test]
    fn sketch_wire_maps_topology_to_worker_shapes() {
        let sid = SketchId(Uuid::from_u128(1));
        let (p0, p1, c) = (eid(0x10), eid(0x11), eid(0x12));
        let (line, circle) = (eid(0x20), eid(0x21));
        let mut sk = Sketch::on_world_plane(sid, "S", WorldPlane::XY);
        sk.add_entity(SketchEntity::point(
            p0,
            Vec2::new_unchecked(0.0, 0.0),
            false,
            false,
        ))
        .unwrap();
        sk.add_entity(SketchEntity::point(
            p1,
            Vec2::new_unchecked(40.0, 0.0),
            false,
            false,
        ))
        .unwrap();
        sk.add_entity(SketchEntity::point(
            c,
            Vec2::new_unchecked(10.0, 10.0),
            false,
            false,
        ))
        .unwrap();
        sk.add_entity(SketchEntity::line(line, p0, p1, false))
            .unwrap();
        sk.add_entity(SketchEntity::circle(circle, c, 3.0, false).unwrap())
            .unwrap();
        sk.add_constraint(Constraint::Horizontal { id: cid(1), line })
            .unwrap();
        sk.add_constraint(Constraint::Distance {
            id: cid(2),
            entity1: p0,
            entity2: p1,
            value: Scalar::new(40.0),
        })
        .unwrap();

        let (plane, entities, constraints) = sketch_wire(&sk);
        // Named plane keeps the non-standard XY basis.
        assert_eq!(plane["kind"], "XY");
        assert_eq!(plane["xAxis"], json!([0.0, 1.0, 0.0]));

        let ents = entities.as_array().unwrap();
        // Line references its endpoints by id (p0Ref/p1Ref) — the point-ref form.
        let l = ents
            .iter()
            .find(|e| e["id"] == json!(line.to_string()))
            .unwrap();
        assert_eq!(l["type"], "Line");
        assert_eq!(l["p0Ref"], json!(p0.to_string()));
        assert_eq!(l["p1Ref"], json!(p1.to_string()));
        // Circle inlines its center coordinate.
        let ci = ents
            .iter()
            .find(|e| e["id"] == json!(circle.to_string()))
            .unwrap();
        assert_eq!(ci["type"], "Circle");
        assert_eq!(ci["center"], json!([10.0, 10.0]));
        assert_eq!(ci["radius"], json!(3.0));
        // BUG-5: the center point uuid rides alongside the inlined coords so re-entry
        // hydration can re-own the center Point (frontend `sketchWireMap.ts`).
        assert_eq!(ci["centerRef"], json!(c.to_string()));

        let cons = constraints.as_array().unwrap();
        let h = cons
            .iter()
            .find(|c| c["type"] == json!("Horizontal"))
            .unwrap();
        assert_eq!(h["entities"], json!([line.to_string()]));
        let d = cons
            .iter()
            .find(|c| c["type"] == json!("Distance"))
            .unwrap();
        assert_eq!(d["entities"], json!([p0.to_string(), p1.to_string()]));
        assert_eq!(d["value"], json!(40.0));
    }

    /// An ellipse renders exactly like a circle plus the three shape scalars —
    /// inlined `center`, informational `centerRef`, `majorR`/`minorR`/`rotation`
    /// verbatim from the core fields (SCHEMA §7.3). Before W3 this entity was
    /// dropped on the floor, so an ellipse never reached the worker at all.
    #[test]
    fn sketch_wire_renders_ellipse_with_inline_center_and_scalars() {
        let sid = SketchId(Uuid::from_u128(2));
        let (c, ellipse) = (eid(0x30), eid(0x31));
        let mut sk = Sketch::on_world_plane(sid, "S", WorldPlane::XY);
        sk.add_entity(SketchEntity::point(
            c,
            Vec2::new_unchecked(-2.5, 4.0),
            false,
            false,
        ))
        .unwrap();
        sk.add_entity(SketchEntity::ellipse(ellipse, c, 6.0, 3.0, 0.25, false).unwrap())
            .unwrap();

        let (_, entities, _) = sketch_wire(&sk);
        let ents = entities.as_array().unwrap();
        assert_eq!(
            ents.len(),
            2,
            "the center Point and the Ellipse both render"
        );
        let e = ents
            .iter()
            .find(|e| e["id"] == json!(ellipse.to_string()))
            .expect("the ellipse is no longer skipped");
        assert_eq!(e["type"], "Ellipse");
        assert_eq!(e["center"], json!([-2.5, 4.0]));
        assert_eq!(e["centerRef"], json!(c.to_string()));
        assert_eq!(e["majorR"], json!(6.0));
        assert_eq!(e["minorR"], json!(3.0));
        // Radians on the wire, not degrees.
        assert_eq!(e["rotation"], json!(0.25));
        assert_eq!(e["construction"], json!(false));
    }

    /// `referenceLocked` is emitted ONLY when true (SCHEMA §7.3), on all five
    /// kinds. The omission is the load-bearing half: every sketch authored
    /// before the flag existed must still marshal byte-identically, and with
    /// zero producers that is every sketch in existence.
    #[test]
    fn sketch_wire_emits_reference_locked_only_when_set() {
        let sid = SketchId(Uuid::from_u128(9));
        let mut sk = Sketch::on_world_plane(sid, "S", WorldPlane::XY);
        let (p0, p1, c) = (eid(0x50), eid(0x51), eid(0x52));
        for (id, locked) in [(p0, false), (p1, false), (c, true)] {
            sk.add_entity(SketchEntity::point(
                id,
                Vec2::new_unchecked(0.0, 0.0),
                false,
                locked,
            ))
            .unwrap();
        }
        let (line, arc, circle, ellipse) = (eid(0x53), eid(0x54), eid(0x55), eid(0x56));
        sk.add_entity(SketchEntity::line(line, p0, p1, false))
            .unwrap();
        sk.add_entity(
            SketchEntity::arc(arc, c, 5.0, 0.0, 1.0, false)
                .unwrap()
                .with_reference_locked(true),
        )
        .unwrap();
        sk.add_entity(
            SketchEntity::circle(circle, c, 5.0, false)
                .unwrap()
                .with_reference_locked(true),
        )
        .unwrap();
        sk.add_entity(
            SketchEntity::ellipse(ellipse, c, 6.0, 3.0, 0.0, false)
                .unwrap()
                .with_reference_locked(true),
        )
        .unwrap();

        let (_, entities, _) = sketch_wire(&sk);
        let ents = entities.as_array().unwrap();
        let at = |id: EntityId| {
            ents.iter()
                .find(|e| e["id"] == json!(id.to_string()))
                .unwrap()
                .clone()
        };
        for id in [c, arc, circle, ellipse] {
            assert_eq!(at(id)["referenceLocked"], json!(true), "{id} is locked");
        }
        for id in [p0, p1, line] {
            assert!(
                at(id).get("referenceLocked").is_none(),
                "{id} is free, so the key must be ABSENT (not `false`)"
            );
        }
    }

    /// The construction flag rides through to the wire, where the worker's
    /// synthesized center Point inherits it. (The "center missing" arm of
    /// `wire_entity` is unreachable through the public API — core rejects a
    /// dangling center with `DanglingEntityRef` at `add_entity` — so the `?` on
    /// `point_pos` is a defensive belt, matching circle/arc.)
    #[test]
    fn sketch_wire_ellipse_carries_construction_flag() {
        let sid = SketchId(Uuid::from_u128(3));
        let (c, ellipse) = (eid(0x40), eid(0x41));
        let mut sk = Sketch::on_world_plane(sid, "S", WorldPlane::XY);
        sk.add_entity(SketchEntity::point(
            c,
            Vec2::new_unchecked(0.0, 0.0),
            true,
            false,
        ))
        .unwrap();
        sk.add_entity(SketchEntity::ellipse(ellipse, c, 5.0, 5.0, 0.0, true).unwrap())
            .unwrap();
        let (_, entities, _) = sketch_wire(&sk);
        let ents = entities.as_array().unwrap();
        let e = ents
            .iter()
            .find(|e| e["id"] == json!(ellipse.to_string()))
            .unwrap();
        assert_eq!(e["construction"], json!(true));
        // A circular ellipse (major == minor) still renders both scalars — the
        // worker's `addEllipse` normalization is a no-op here.
        assert_eq!(e["majorR"], json!(5.0));
        assert_eq!(e["minorR"], json!(5.0));
    }

    /// W0b: an arc-endpoint Coincident carries `positions`; a plain
    /// point-to-point one must stay byte-identical to the pre-W0b wire (the
    /// worker parses `positions` per slot, so a spurious array would change how
    /// every existing Coincident resolves).
    #[test]
    fn coincident_emits_positions_only_for_arc_endpoints() {
        let (p, arc) = (eid(0x50), eid(0x51));

        let plain = wire_constraint(&Constraint::Coincident {
            id: ConstraintId(Uuid::from_u128(1)),
            point1: p,
            point2: arc,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        });
        assert_eq!(
            plain,
            json!({
                "id": ConstraintId(Uuid::from_u128(1)).to_string(),
                "type": "Coincident",
                "entities": [p.to_string(), arc.to_string()],
            }),
            "a point-to-point Coincident renders exactly as it always has"
        );

        let welded = wire_constraint(&Constraint::Coincident {
            id: ConstraintId(Uuid::from_u128(2)),
            point1: p,
            point2: arc,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::End,
        });
        // The Arbitrary slot is the EMPTY role, not "Arbitrary" — see
        // `curve_position_role`.
        assert_eq!(welded["positions"], json!(["", "End"]));
        assert_eq!(welded["entities"], json!([p.to_string(), arc.to_string()]));

        let both = wire_constraint(&Constraint::Coincident {
            id: ConstraintId(Uuid::from_u128(3)),
            point1: arc,
            point2: arc,
            point1_position: CurvePosition::Start,
            point2_position: CurvePosition::End,
        });
        assert_eq!(both["positions"], json!(["Start", "End"]));
    }

    /// The core serde form is likewise unchanged for the default positions —
    /// this is what keeps the frozen sketch fixtures byte-identical.
    #[test]
    fn coincident_serde_skips_default_positions() {
        let plain = Constraint::Coincident {
            id: ConstraintId(Uuid::from_u128(1)),
            point1: eid(0x50),
            point2: eid(0x51),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        };
        let v = serde_json::to_value(&plain).unwrap();
        assert!(v.get("point1Position").is_none());
        assert!(v.get("point2Position").is_none());
        // …and a document written before the fields existed still loads.
        let legacy = json!({
            "kind": "coincident",
            "id": ConstraintId(Uuid::from_u128(1)).to_string(),
            "point1": eid(0x50).to_string(),
            "point2": eid(0x51).to_string(),
        });
        assert_eq!(serde_json::from_value::<Constraint>(legacy).unwrap(), plain);

        let welded = Constraint::Coincident {
            id: ConstraintId(Uuid::from_u128(2)),
            point1: eid(0x50),
            point2: eid(0x51),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Start,
        };
        let v = serde_json::to_value(&welded).unwrap();
        assert!(
            v.get("point1Position").is_none(),
            "default slot stays absent"
        );
        assert_eq!(
            v["point2Position"],
            json!("start"),
            "camelCase field, camelCase value"
        );
    }

    #[test]
    fn solve_drag_parses_superseded_and_positions() {
        let ok = json!({
            "gestureId": 51, "seq": 129, "status": "success", "dof": 1,
            "conflicting": [], "positions": { "e3.start": [42.0, 19.5] }, "solveMicros": 1840
        });
        let d = parse_solve_drag(&ok);
        assert_eq!(d.seq, 129);
        assert!(!d.superseded);
        assert_eq!(d.positions["e3.start"], [42.0, 19.5]);
        assert_eq!(d.dof, 1);

        let stale =
            json!({ "gestureId": 51, "seq": 3, "status": "superseded", "dof": 1, "positions": {} });
        let d = parse_solve_drag(&stale);
        assert!(
            d.superseded,
            "a stale seq resolves superseded (latest-wins)"
        );
        assert!(d.positions.is_empty());
    }

    #[test]
    fn sketch_upsert_parses_state_and_end_gesture_derives_status() {
        // SketchUpsert carries `state`.
        let up = json!({ "sketchId": "sk_1", "sketchRevision": 4, "dof": 2, "state": "UnderConstrained" });
        let d = parse_sketch_upsert("sk_1", &up);
        assert_eq!(d.sketch_revision, 4);
        assert_eq!(d.dof, 2);
        assert_eq!(d.status, SketchSolveStatus::UnderConstrained);
        // EndGesture carries `status` (drag status) + dof; the DTO derives the solve
        // status from dof (0 ⇒ FullyConstrained).
        let end = json!({ "gestureId": 51, "status": "success", "dof": 0,
            "positions": { "00000000-0000-0000-0000-000000000010": [1.0, 2.0] }, "sketchRevision": 5 });
        let d = parse_sketch_upsert("sk_1", &end);
        assert_eq!(d.status, SketchSolveStatus::FullyConstrained);
        assert_eq!(d.sketch_revision, 5);
        assert_eq!(d.solved_positions.len(), 1);
    }

    #[test]
    fn sketch_upsert_parses_conflicting_present_and_absent() {
        // Conflicting upsert: the constraint ids ride the `conflicting` field.
        let up = json!({ "sketchId": "sk_1", "sketchRevision": 3, "dof": 0,
            "state": "Conflicting", "conflicting": ["c-a", "c-b"] });
        let d = parse_sketch_upsert("sk_1", &up);
        assert_eq!(d.status, SketchSolveStatus::Conflicting);
        assert_eq!(d.conflicting, vec!["c-a".to_string(), "c-b".to_string()]);
        // Absent field (legacy worker / clean sketch) ⇒ empty, no panic.
        let clean = json!({ "sketchId": "sk_1", "sketchRevision": 4, "dof": 2, "state": "UnderConstrained" });
        assert!(parse_sketch_upsert("sk_1", &clean).conflicting.is_empty());
        // EndGesture carries it too (same parser).
        let end = json!({ "gestureId": 51, "status": "conflicting", "dof": 0,
            "conflicting": ["c-x"], "positions": {}, "sketchRevision": 6 });
        assert_eq!(
            parse_sketch_upsert("sk_1", &end).conflicting,
            vec!["c-x".to_string()]
        );
    }

    #[test]
    fn sketch_regions_preserve_holes_subtracted_evidence() {
        let mut tail = Vec::new();
        for xyz in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for value in xyz {
                tail.extend_from_slice(&value.to_le_bytes());
            }
        }
        for index in [0_u32, 1, 2] {
            tail.extend_from_slice(&index.to_le_bytes());
        }
        let sections = vec![BinSection {
            name: "region:r0".into(),
            off: 0,
            len: tail.len() as u32,
        }];
        let result = json!({
            "sketchId": "sk_1",
            "sketchRevision": 1,
            "regions": [{
                "regionId": "r0",
                "outerLoop": ["outer"],
                "holes": [["inner"]],
                "previewTriangles": {
                    "format": "f32xyz+u32idx",
                    "bin": "region:r0",
                    "vertexCount": 3,
                    "triangleCount": 1,
                    "holesSubtracted": 1
                }
            }]
        });

        let regions = parse_sketch_regions("sk_1", &result, &sections, &tail).unwrap();
        let triangles = regions[0].preview_triangles.as_ref().unwrap();
        assert_eq!(triangles.holes_subtracted, 1);
        assert_eq!(triangles.indices, vec![0, 1, 2]);
    }

    #[test]
    fn sketch_regions_require_response_and_region_structure() {
        let (result, sections, tail) = valid_region_response();
        for field in ["sketchId", "sketchRevision", "regions"] {
            let mut malformed = result.clone();
            malformed.as_object_mut().unwrap().remove(field);
            assert_region_parse_error(&malformed, &sections, &tail);
        }
        let mut wrong_sketch = result.clone();
        wrong_sketch["sketchId"] = json!("sk_other");
        assert_region_parse_error(&wrong_sketch, &sections, &tail);

        for field in ["regionId", "outerLoop", "holes", "previewTriangles"] {
            let mut malformed = result.clone();
            malformed["regions"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert_region_parse_error(&malformed, &sections, &tail);
        }
        let empty = json!({"sketchId": "sk_1", "sketchRevision": 1, "regions": []});
        assert!(parse_sketch_regions("sk_1", &empty, &[], &[])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sketch_regions_reject_invalid_loops_holes_and_duplicates() {
        let (result, sections, tail) = valid_region_response();
        for invalid_outer in [json!([]), json!([""])] {
            let mut malformed = result.clone();
            malformed["regions"][0]["outerLoop"] = invalid_outer;
            assert_region_parse_error(&malformed, &sections, &tail);
        }
        for invalid_holes in [json!([[]]), json!(["not-an-array"]), json!([[""]])] {
            let mut malformed = result.clone();
            malformed["regions"][0]["holes"] = invalid_holes;
            assert_region_parse_error(&malformed, &sections, &tail);
        }
        let mut duplicate_region = result.clone();
        let copy = duplicate_region["regions"][0].clone();
        duplicate_region["regions"]
            .as_array_mut()
            .unwrap()
            .push(copy);
        assert_region_parse_error(&duplicate_region, &sections, &tail);

        let mut reused_bin = result.clone();
        let mut second = reused_bin["regions"][0].clone();
        second["regionId"] = json!("r1");
        reused_bin["regions"].as_array_mut().unwrap().push(second);
        assert_region_parse_error(&reused_bin, &sections, &tail);
    }

    #[test]
    fn sketch_regions_reject_invalid_triangle_metadata() {
        let (result, sections, tail) = valid_region_response();
        for (field, value) in [
            ("format", json!("other")),
            ("vertexCount", json!(0)),
            ("triangleCount", json!(0)),
            ("holesSubtracted", json!(1)),
        ] {
            let mut malformed = result.clone();
            malformed["regions"][0]["previewTriangles"][field] = value;
            assert_region_parse_error(&malformed, &sections, &tail);
        }
        for field in ["bin", "vertexCount", "triangleCount", "holesSubtracted"] {
            let mut malformed = result.clone();
            malformed["regions"][0]["previewTriangles"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert_region_parse_error(&malformed, &sections, &tail);
        }
        let mut wrong_count = result.clone();
        wrong_count["regions"][0]["previewTriangles"]["triangleCount"] = json!(2);
        assert_region_parse_error(&wrong_count, &sections, &tail);
    }

    #[test]
    fn sketch_regions_reject_invalid_binary_geometry() {
        let (result, sections, tail) = valid_region_response();
        let mut bad_index = tail.clone();
        bad_index[36..40].copy_from_slice(&3_u32.to_le_bytes());
        assert_region_parse_error(&result, &sections, &bad_index);
        let mut nonfinite = tail.clone();
        nonfinite[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_region_parse_error(&result, &sections, &nonfinite);
        let mut off_plane = tail.clone();
        off_plane[8..12].copy_from_slice(&1_f32.to_le_bytes());
        assert_region_parse_error(&result, &sections, &off_plane);

        let mut short = sections.clone();
        short[0].len -= 1;
        assert_region_parse_error(&result, &short, &tail);
        let mut out_of_bounds = sections.clone();
        out_of_bounds[0].off = tail.len() as u32;
        assert_region_parse_error(&result, &out_of_bounds, &tail);
    }

    #[test]
    fn sketch_regions_reject_bad_section_tables_but_allow_alignment_gaps() {
        let (result, sections, tail) = valid_region_response();
        let mut duplicate = sections.clone();
        duplicate.push(sections[0].clone());
        assert_region_parse_error(&result, &duplicate, &tail);
        let mut overlap = sections.clone();
        overlap.push(BinSection {
            name: "extra".into(),
            off: 4,
            len: 4,
        });
        assert_region_parse_error(&result, &overlap, &tail);
        let mut tail_with_extra = tail.clone();
        tail_with_extra.extend_from_slice(&[0; 4]);
        let mut unexpected = sections.clone();
        unexpected.push(BinSection {
            name: "extra".into(),
            off: tail.len() as u32,
            len: 4,
        });
        assert_region_parse_error(&result, &unexpected, &tail_with_extra);

        let mut aligned_tail = vec![0; 4];
        aligned_tail.extend_from_slice(&tail);
        let mut aligned = sections;
        aligned[0].off = 4;
        assert!(parse_sketch_regions("sk_1", &result, &aligned, &aligned_tail).is_ok());
    }

    #[test]
    fn acquire_args_and_evidence_round_trip() {
        let body = BodyId(Uuid::from_u128(0x3));
        let req = AcquireRequest {
            snapshot_id: SnapshotId(5012),
            body,
            picks: vec![Pick {
                topo_key: TopoKey::new("f:22"),
                anchor: None,
            }],
        };
        let args = acquire_element_ids_args(&req);
        assert_eq!(args["snapshotId"], 5012);
        assert_eq!(args["bodyId"], json!(body_id_wire(body)));
        assert_eq!(args["picks"][0]["topoKey"], "f:22");

        // Worker echoes evidence (existing id present ⇒ carried through).
        let result = json!({ "ids": [
            { "topoKey": "f:22", "kind": "face", "bodyId": body_id_wire(body), "elementId": "el_00000000000004a1", "descriptor": {} },
            { "topoKey": "e:3", "kind": "edge", "bodyId": body_id_wire(body), "elementId": "" }
        ]});
        let ev = parse_acquire_evidence(&result, body);
        assert_eq!(ev.len(), 2);
        assert_eq!(
            ev[0].existing.as_ref().unwrap().as_str(),
            "el_00000000000004a1"
        );
        assert_eq!(ev[0].kind, onecad_core::document::refs::ElementKind::Face);
        assert!(ev[1].existing.is_none(), "empty elementId ⇒ Rust mints");
        assert_eq!(ev[1].kind, onecad_core::document::refs::ElementKind::Edge);
    }

    #[test]
    fn resolve_refs_parses_all_three_outcomes() {
        let result = json!({ "resolutions": [
            // M4a shape: `elementId` in its own slot (Rust-minted), `topoKey` = evidence.
            { "refId": "op_5.input0", "outcome": "autoBind", "elementId": "el_top", "topoKey": "f:1", "score": 0.94, "margin": 0.31 },
            { "refId": "op_5.input1", "outcome": "unchanged", "elementId": "el_9", "topoKey": "f:2" },
            { "refId": "op_5.input2", "outcome": "needsRepair",
              "needsRepair": { "refId": "op_5.input2", "ladderFailed": "descriptor", "reason": "ambiguous", "candidates": [] } },
            // Unminted dry-run autoBind: empty elementId, topoKey still rides as evidence.
            { "refId": "op_5.input3", "outcome": "autoBind", "elementId": "", "topoKey": "f:3", "score": 0.9, "margin": 0.2 }
        ]});
        let res = parse_resolve_refs(&result);
        assert_eq!(res.len(), 4);
        match &res[0].outcome {
            ResolveOutcome::AutoBind {
                element_id,
                score,
                topo_key,
                ..
            } => {
                assert_eq!(element_id.as_str(), "el_top", "elementId in its own slot");
                assert!((score - 0.94).abs() < 1e-9);
                assert_eq!(
                    topo_key.as_ref().map(TopoKey::as_str),
                    Some("f:1"),
                    "topoKey rides as evidence"
                );
            }
            other => panic!("expected AutoBind, got {other:?}"),
        }
        assert!(matches!(
            &res[1].outcome,
            ResolveOutcome::Unchanged { element_id } if element_id.as_ref().map(ElementId::as_str) == Some("el_9")
        ));
        assert!(matches!(res[2].outcome, ResolveOutcome::NeedsRepair(_)));
        // Unminted autoBind: empty elementId, topoKey preserved as evidence.
        match &res[3].outcome {
            ResolveOutcome::AutoBind {
                element_id,
                topo_key,
                ..
            } => {
                assert!(element_id.as_str().is_empty(), "unminted ⇒ empty elementId");
                assert_eq!(topo_key.as_ref().map(TopoKey::as_str), Some("f:3"));
            }
            other => panic!("expected AutoBind, got {other:?}"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BodyId wire-form rendering (M2 code-review defects 1–6) — the params/inputs body
// fields must cross the wire as `body_<uuid>` (SCHEMA §2), never a bare core-serde
// uuid (which the worker's BodyStore, keyed `body_<opId>`, would never resolve).
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod body_wire_tests {
    use super::*;
    use onecad_core::document::record::{
        BooleanMode, BooleanOp, BooleanParams, DeterminismSettings, ExtrudeParams, FilletParams,
        OperationInputs, RevolveParams,
    };
    use onecad_core::document::refs::{AxisRef, ElementKind, ElementRef, Extra, PrimaryRef};
    use onecad_core::document::variables::Scalar;
    use onecad_core::ids::RecordId;

    fn planned(op: Operation, inputs: OperationInputs) -> PlannedOp {
        PlannedOp {
            record_id: RecordId(Uuid::from_u128(0xE0)),
            step_index: 1,
            operation: op,
            inputs,
            determinism: DeterminismSettings::default(),
        }
    }

    fn extrude_cut(target: BodyId) -> ExtrudeParams {
        ExtrudeParams {
            profile: None,
            distance: Scalar::new(5.0),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::Cut,
            target_body: Some(target),
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn to_wire_body_form_rewrites_only_body_keys() {
        let u = Uuid::from_u128(0xABC);
        let bare = u.to_string();
        let mut v = json!({
            "targetBodyId": bare,
            "toolBodyId": bare,
            "sourceBodyId": bare,
            "bodyId": bare,
            "sketchId": bare,        // not a body key
            "elementId": "el_1",
            "bodyIdList": bare,      // contains but does not END with BodyId
            "nested": { "axis": { "bodyId": bare } },
            "already": { "targetBodyId": format!("body_{u}") },  // idempotent
            "empty": { "targetBodyId": "" },                     // NewBody
            // Worker-authored frozen evidence: round-trips verbatim, never rewritten.
            "intent": { "bodyId": bare, "descriptor": { "refBodyId": bare } },
        });
        to_wire_body_form(&mut v);
        let want = format!("body_{u}");
        for k in ["targetBodyId", "toolBodyId", "sourceBodyId", "bodyId"] {
            assert_eq!(v[k], json!(want), "{k} → body_<uuid>");
        }
        assert_eq!(v["sketchId"], json!(bare), "sketchId is not a body key");
        assert_eq!(
            v["bodyIdList"],
            json!(bare),
            "suffix match only (not substring)"
        );
        assert_eq!(v["nested"]["axis"]["bodyId"], json!(want), "recurses");
        assert_eq!(
            v["already"]["targetBodyId"],
            json!(want),
            "already-prefixed is left as-is (idempotent)"
        );
        assert_eq!(v["empty"]["targetBodyId"], json!(""), "empty stays empty");
        assert_eq!(
            v["intent"],
            json!({ "bodyId": bare, "descriptor": { "refBodyId": bare } }),
            "intent subtree (worker-authored evidence) round-trips verbatim"
        );
    }

    #[test]
    fn wire_op_boolean_renders_body_wire_form() {
        let target = BodyId(Uuid::from_u128(0x11));
        let tool = BodyId(Uuid::from_u128(0x22));
        let op = Operation::Known(KnownOperation::Boolean(BooleanParams {
            operation: BooleanOp::Cut,
            target_body: target,
            tool_body: tool,
            extra: Default::default(),
        }));
        let inputs = op.derive_inputs();
        let w = wire_op(&planned(op, inputs));
        assert_eq!(w["params"]["targetBodyId"], json!(body_id_wire(target)));
        assert_eq!(w["params"]["toolBodyId"], json!(body_id_wire(tool)));
        // inputs[] whole-body refs carry the same wire form + kind "body".
        assert_eq!(
            w["inputs"][0]["primary"]["bodyId"],
            json!(body_id_wire(target))
        );
        assert_eq!(w["inputs"][0]["primary"]["kind"], json!("body"));
        assert_eq!(
            w["inputs"][1]["primary"]["bodyId"],
            json!(body_id_wire(tool))
        );
    }

    #[test]
    fn wire_op_extrude_cut_renders_target_body_wire_form() {
        let target = BodyId(Uuid::from_u128(0x33));
        let op = Operation::Known(KnownOperation::Extrude(extrude_cut(target)));
        let inputs = op.derive_inputs();
        let w = wire_op(&planned(op, inputs));
        assert_eq!(w["params"]["targetBodyId"], json!(body_id_wire(target)));
    }

    #[test]
    fn element_ref_wire_body_form_kind_and_extra_survive() {
        let body = BodyId(Uuid::from_u128(0x44));
        let mut extra = Extra::new();
        extra.insert("alienRefKey".into(), json!({ "keep": true }));
        let r = ElementRef {
            primary: Some(PrimaryRef {
                body,
                element: ElementId::new("el_9"),
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            intent: None,
            anchor: None,
            extra,
        };
        let w = element_ref_wire(&r);
        assert_eq!(w["primary"]["bodyId"], json!(body_id_wire(body)));
        assert_eq!(w["primary"]["elementId"], json!("el_9"));
        assert_eq!(
            w["primary"]["kind"],
            json!("face"),
            "kind lowercases via derive"
        );
        assert_eq!(
            w["alienRefKey"],
            json!({ "keep": true }),
            "serde-flattened extra preserved (hand-rolled builder dropped it)"
        );
    }

    #[test]
    fn edge_input_refs_bare_fallback_attaches_operated_body() {
        let body = BodyId(Uuid::from_u128(0x55));
        let ids = vec![ElementId::new("e:5"), ElementId::new("e:6")];
        let with_body = edge_input_refs(&[], &ids, &[body]);
        assert_eq!(
            with_body[0]["primary"]["bodyId"],
            json!(body_id_wire(body)),
            "bare fallback attaches the operated body (defect 5)"
        );
        assert_eq!(with_body[0]["primary"]["elementId"], json!("e:5"));
        assert_eq!(with_body[0]["primary"]["kind"], json!("edge"));
        // No body input ⇒ element-only (a clear worker-side "requires body input").
        let no_body = edge_input_refs(&[], &ids, &[]);
        assert!(no_body[0]["primary"].get("bodyId").is_none());
        assert_eq!(no_body[0]["primary"]["elementId"], json!("e:5"));
    }

    #[test]
    fn wire_op_fillet_bare_edge_ids_carries_operated_body() {
        let body = BodyId(Uuid::from_u128(0x66));
        let op = Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(2.0),
            edge_ids: vec![ElementId::new("e:14")],
            edges: vec![],
            chain_tangent_edges: false,
            extra: Default::default(),
        }));
        // The graph-view carries the operated body at bodies[0] (as the plan would
        // when the fillet's body input is known), exercising the bare-fallback attach.
        let mut inputs = OperationInputs::default();
        inputs.bodies.push(body);
        inputs.elements.push(ElementId::new("e:14"));
        let w = wire_op(&planned(op, inputs));
        assert_eq!(
            w["inputs"][0]["primary"]["bodyId"],
            json!(body_id_wire(body))
        );
        assert_eq!(w["inputs"][0]["primary"]["kind"], json!("edge"));
    }

    #[test]
    fn wire_op_lifts_profile_to_sketch_and_region_for_extrude_and_revolve() {
        use onecad_core::document::refs::SketchRegionRef;
        use onecad_core::ids::{RegionId, SketchId};

        let sid = SketchId(Uuid::from_u128(0x5c));
        let region = "r_0123456789abcdef";
        let profile = SketchRegionRef {
            sketch: sid,
            region: RegionId::new(region),
            extra: Default::default(),
        };

        // Extrude: profile {sketchId, regionId} is lifted to top-level params; the
        // core-only `profile` wrapper (SCHEMA §7.3 has none) is dropped.
        let ex = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(profile.clone()),
            distance: Scalar::new(5.0),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        }));
        let w = wire_op(&planned(ex.clone(), ex.derive_inputs()));
        assert_eq!(w["params"]["sketchId"], json!(sid.to_string()));
        assert_eq!(w["params"]["regionId"], json!(region));
        assert!(
            w["params"].get("profile").is_none(),
            "the core-only `profile` wrapper is dropped from the wire"
        );

        // Revolve: same lift (the frontend Revolve WP now sends a profile too; the
        // worker's RevolveOp reads params.sketchId/regionId via the SAME find_sketch +
        // build_profile_face path as Extrude).
        let rev = Operation::Known(KnownOperation::Revolve(RevolveParams {
            profile: Some(profile),
            angle_deg: Scalar::new(360.0),
            axis: None,
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            extra: Default::default(),
        }));
        let w = wire_op(&planned(rev.clone(), rev.derive_inputs()));
        assert_eq!(w["params"]["sketchId"], json!(sid.to_string()));
        assert_eq!(w["params"]["regionId"], json!(region));
        assert!(w["params"].get("profile").is_none());

        // An EMPTY regionId is NOT forwarded (keeps the worker's first-region fallback).
        let ex_empty = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch: sid,
                region: RegionId::new(""),
                extra: Default::default(),
            }),
            distance: Scalar::new(5.0),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        }));
        let w = wire_op(&planned(ex_empty.clone(), ex_empty.derive_inputs()));
        assert_eq!(w["params"]["sketchId"], json!(sid.to_string()));
        assert!(
            w["params"].get("regionId").is_none(),
            "empty regionId is not forwarded (first-region fallback)"
        );
    }

    /// PreviewOp and ExecutePlan MUST lower the SAME typed `Operation` to the SAME
    /// wire op — for EVERY op the frontend can open a preview session on, not just
    /// Extrude (`src/ipc/previewOps.ts` `OP_BUILDERS`). Both sides go through
    /// `lower_operation`, so this is a regression fence around that single seam:
    /// any future opType-specific branch on either path breaks it here first.
    #[test]
    fn preview_and_commit_share_profile_and_body_lowering() {
        use onecad_core::document::record::{ChamferParams, ShellParams};
        use onecad_core::document::refs::SketchRegionRef;
        use onecad_core::ids::{RegionId, SketchId};

        let target = BodyId(Uuid::from_u128(0x88));
        let tool = BodyId(Uuid::from_u128(0x89));
        let edge_body = BodyId(Uuid::from_u128(0x8a));
        let profile = SketchRegionRef {
            sketch: SketchId(Uuid::from_u128(0x99)),
            region: RegionId::new("r_non_first"),
            extra: Default::default(),
        };
        let edge_ref = |body: BodyId, element: &str| ElementRef {
            primary: Some(PrimaryRef {
                body,
                element: ElementId::new(element),
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: None,
            extra: Default::default(),
        };

        let mut extrude = extrude_cut(target);
        extrude.profile = Some(profile.clone());

        let cases: Vec<(&str, Operation)> = vec![
            (
                "Extrude",
                Operation::Known(KnownOperation::Extrude(extrude)),
            ),
            (
                "Revolve",
                Operation::Known(KnownOperation::Revolve(RevolveParams {
                    profile: Some(profile.clone()),
                    angle_deg: Scalar::new(90.0),
                    axis: Some(AxisRef::Element {
                        body: target,
                        edge: ElementId::new("e:2"),
                        extra: Default::default(),
                    }),
                    boolean_mode: BooleanMode::Cut,
                    target_body: Some(target),
                    extra: Default::default(),
                })),
            ),
            (
                "Fillet",
                Operation::Known(KnownOperation::Fillet(FilletParams {
                    radius: Scalar::new(2.0),
                    edge_ids: vec![ElementId::new("e:14"), ElementId::new("e:15")],
                    edges: vec![edge_ref(edge_body, "e:14"), edge_ref(edge_body, "e:15")],
                    chain_tangent_edges: true,
                    extra: Default::default(),
                })),
            ),
            (
                "Chamfer",
                Operation::Known(KnownOperation::Chamfer(ChamferParams {
                    radius: Scalar::new(1.0),
                    edge_ids: vec![ElementId::new("e:16")],
                    edges: vec![edge_ref(edge_body, "e:16")],
                    chain_tangent_edges: true,
                    extra: Default::default(),
                })),
            ),
            (
                "Shell",
                Operation::Known(KnownOperation::Shell(ShellParams {
                    thickness: Scalar::new(1.5),
                    open_faces: vec![ElementId::new("el_f1")],
                    target_body: Some(target),
                    extra: Default::default(),
                })),
            ),
            (
                "Boolean",
                Operation::Known(KnownOperation::Boolean(BooleanParams {
                    operation: BooleanOp::Cut,
                    target_body: target,
                    tool_body: tool,
                    extra: Default::default(),
                })),
            ),
        ];

        for (name, operation) in cases {
            let planned = planned(operation.clone(), operation.derive_inputs());
            let committed = wire_op(&planned);
            let previewed = preview_wire_op(&operation, &planned.record_id.to_string());
            assert_eq!(previewed["opType"], json!(name), "{name}: opType tag");
            for key in ["opType", "opId", "inputs", "params"] {
                assert_eq!(
                    previewed[key], committed[key],
                    "{name}: PreviewOp and ExecutePlan diverged at {key}"
                );
            }
        }

        // Extrude keeps its named pins (EXTRUDE-REGION-PARITY): the boolean target
        // renders in worker body form, a non-first region survives, and the
        // core-only `profile` wrapper is gone.
        let mut params = extrude_cut(target);
        params.profile = Some(profile);
        let operation = Operation::Known(KnownOperation::Extrude(params));
        let previewed = preview_wire_op(&operation, "op-1");
        assert_eq!(
            previewed["params"]["targetBodyId"],
            json!(body_id_wire(target))
        );
        assert_eq!(previewed["params"]["regionId"], json!("r_non_first"));
        assert!(previewed["params"].get("profile").is_none());
    }

    #[test]
    fn wire_op_revolve_renders_axis_and_target_body_wire_form() {
        let axis_body = BodyId(Uuid::from_u128(0x77));
        let target = BodyId(Uuid::from_u128(0x88));
        let op = Operation::Known(KnownOperation::Revolve(RevolveParams {
            profile: None,
            angle_deg: Scalar::new(90.0),
            axis: Some(AxisRef::Element {
                body: axis_body,
                edge: ElementId::new("e:2"),
                extra: Default::default(),
            }),
            boolean_mode: BooleanMode::Cut,
            target_body: Some(target),
            extra: Default::default(),
        }));
        let inputs = op.derive_inputs();
        let w = wire_op(&planned(op, inputs));
        // Both body-bearing params fields (edge-axis body + boolean target) → wire form
        // via to_wire_body_form; the worker reads them from params (no inputs branch).
        assert_eq!(
            w["params"]["axis"]["bodyId"],
            json!(body_id_wire(axis_body)),
            "edge-axis body → wire form (defect 4)"
        );
        assert_eq!(w["params"]["targetBodyId"], json!(body_id_wire(target)));
    }
}
