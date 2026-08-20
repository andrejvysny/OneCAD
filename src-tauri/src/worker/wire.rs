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

use onecad_core::document::body::{BodyHealth, BodyLifecycleEvent};
use onecad_core::document::record::{
    ExtrudeMode, FrozenPlacement, KnownOperation, OffsetDistanceType, Operation,
};
use onecad_core::document::refs::{AnchorIntent, AxisRef, ElementKind, ElementRef};
use onecad_core::document::repair::RepairItem;
use onecad_core::ids::{
    BodyId, DocumentRevision, ElementId, EntityId, JobId, SnapshotId, TopoKey, WorkerEpoch,
};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    AcceptResult, AcquireRequest, BindElementIdsRequest, BodySelector, CheckpointArtifact,
    CheckpointArtifacts, CheckpointEnvelope, Diagnostic, ElementMapDelta, ElementMapEntry,
    EngineError, HistoryPrefixHash, Lod, OpFailureCode, OpenSessionRequest, PlanPrepared,
    PlanRequest, PlanStepEvent, PlannedOp, PreparedMeshRef, RefResolution, ResolveOutcome,
    ResolveRequest, RestoreRequest, SessionMode, Severity, Signature, StepResult, StepSignatures,
    StepStatus, StoppedReason, TessellateRequest, WorkerElementEvidence, WorkerHead,
    ARTIFACT_SCHEMA_VERSION,
};
use onecad_core::sketch::WorldPlane;
use onecad_core::sketch::{
    Constraint, CurvePosition, FaceFrame, ProjectedEntity, ProjectionPayload, Sketch,
    SketchAttachment, SketchEntity, SketchPlane,
};

use onecad_protocol::messages::{BinSection, ErrorCode, ErrorObject};

use crate::dto::{
    CurveParamsDto, DragSolveDto, PreviewTrianglesDto, SketchRegionDto, SketchSolveStatus,
    SketchUpsertDto,
};

use super::{lod_from_str, lod_str, StepInspection};

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
use std::sync::{Arc, OnceLock, RwLock};

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
    // OPTIONAL `editedFrom` (SCHEMA §7.2). OMITTED when there is no edit context —
    // absence is "no claim", and a `null` would be a claim of a different kind.
    if let Some(edited_from) = req.edited_from {
        args["editedFrom"] = json!(edited_from);
    }
    // OPTIONAL `checkpointFallbackReplay` (SCHEMA §7.2), same omission rule: the
    // key appears ONLY when true, so an ordinary plan is byte-identical to what it
    // was before this field existed and `false` never has to be spelled out.
    if req.checkpoint_fallback_replay {
        args["checkpointFallbackReplay"] = json!(true);
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
    strip_sketch_host_face(operation, &mut params);
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

/// Drops the core-only `hostFace` key from `Sketch` op params (VF-B5a).
///
/// `SketchOpParams::host_face` is the RECORD's copy of a face-hosted sketch's
/// attachment: it exists so `derive_inputs` can declare the dependency (and the
/// §7.3 edit-safety gate can see it), and it participates in the planner's
/// prefix hash. It is NOT part of the §7.3 `Sketch` wire params — the SCHEMA
/// states the attachment is core-owned and never crosses the wire, and the worker
/// stores raw sketch params verbatim, so emitting it would put a core-only field
/// into every plan frame for no consumer. Legacy/world/datum records carry `None`
/// and never render the key at all, so this only bites newly stamped host-face
/// records. (Rust remains the sole hash authority — the planner hashes the CORE
/// params, so this omission cannot move a hash.)
fn strip_sketch_host_face(operation: &Operation, params: &mut Value) {
    if !matches!(operation, Operation::Known(KnownOperation::Sketch(_))) {
        return;
    }
    if let Some(map) = params.as_object_mut() {
        map.remove("hostFace");
    }
}

/// Lifts the Rust-core `profile` (`{sketchId, regionId, regionIdentityVersion?}`)
/// to **top-level** `params.sketchId` + `params.regionId` + optional version and drops the `profile`
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
    if let Some(version) = pobj.get("regionIdentityVersion") {
        map.insert("regionIdentityVersion".into(), version.clone());
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
/// (WP-3.2) The same treatment reaches a placed component whose source is one of
/// the blob-backed kinds (`embedded` / `document`): the record carries the content
/// address, the worker needs the bytes, and the path is injected at
/// `params.source.path` — nested, because that is where the rest of the source
/// pointer already lives. `generator` sources are untouched: they carry no blob.
fn inject_import_path(operation: &Operation, params: &mut Value) {
    match operation {
        Operation::Known(KnownOperation::ImportStep(p)) => {
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
        Operation::Known(KnownOperation::PlaceComponent(p)) => {
            inject_component_source_path(&p.source, params, "placeComponent");
        }
        Operation::Known(KnownOperation::DetachComponent(p)) => {
            inject_component_source_path(&p.source, params, "detachComponent");
        }
        _ => {}
    }
}

/// Injects `params.source.path` for a blob-backed component source. Same three
/// rules as the `ImportStep` lane: wire-only, never hashed, and an unmaterialized
/// blob lowers an EMPTY path so exactly that one step fails.
fn inject_component_source_path(
    source: &onecad_core::document::record::ComponentSourceRef,
    params: &mut Value,
    op_label: &str,
) {
    let Some(blob) = source.blob_ref() else {
        return; // generator source — no bytes to point at
    };
    let Some(source_map) = params
        .as_object_mut()
        .and_then(|m| m.get_mut("source"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let path = crate::imports::resolve_blob_path(blob.sha256);
    if path.is_empty() {
        tracing::warn!(
            sha = %blob.sha256,
            op = op_label,
            "component source: no materialized blob — lowering an empty path (the worker will \
             fail THIS step with OP_FAILED, not the plan)"
        );
    }
    source_map.insert("path".into(), Value::String(path));
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
        // Typed Revolve body-edge axes use the normal semantic-input route so
        // descriptor stamping, pre-op ladder resolution and repair provenance
        // all agree on `<opId>.input0`. A legacy axis remains byte-identical.
        Operation::Known(KnownOperation::Revolve(p)) => match p.axis.as_ref() {
            Some(AxisRef::Element {
                edge_ref: Some(edge_ref),
                ..
            }) => vec![element_ref_wire(edge_ref)],
            _ => Vec::new(),
        },
        Operation::Known(KnownOperation::Fillet(p)) => {
            edge_input_refs(&p.edges, &p.edge_ids, &inputs.bodies)
        }
        Operation::Known(KnownOperation::Chamfer(p)) => {
            edge_input_refs(&p.edges, &p.edge_ids, &inputs.bodies)
        }
        Operation::Known(KnownOperation::Boolean(p)) => {
            vec![body_input_ref(p.target_body), body_input_ref(p.tool_body)]
        }
        // Shell: one semantic ref per removed face. New records carry typed refs
        // with descriptor/anchor evidence; legacy bare-id records retain the safe
        // element-only fallback. The shelled body rides in `params`.
        Operation::Known(KnownOperation::Shell(p)) => {
            face_input_refs(&p.faces, &p.open_faces, &inputs.bodies)
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
        // Hole: `[semanticRef(host body), semanticRef(host face)]` in that order
        // (SCHEMA §7.3). The face ref is the TYPED one from `params.face` — it
        // carries descriptor + anchor evidence, so `HoleOp` resolves it through the
        // ladder exactly like a Fillet edge (and `NeedsRepair`s rather than guessing
        // when the face is gone). `params.targetBodyId` also carries the host id
        // (auto-covered by `to_wire_body_form`); this is the graph-visible echo.
        Operation::Known(KnownOperation::Hole(p)) => {
            vec![body_input_ref(p.target_body), element_ref_wire(&p.face)]
        }
        // Gear: the placement FACE only. Unlike every other op here a gear has
        // NO host body — it MINTS one (D1) — so there is no body slot to echo,
        // and a FRAME placement contributes no ref at all. The face resolves
        // through the ladder exactly like a Hole seat (SCHEMA §7.3).
        Operation::Known(KnownOperation::Gear(p)) => {
            p.placement.face.iter().map(element_ref_wire).collect()
        }
        // OffsetFace: the operative faces in stored order, then the `Total`
        // opposite face LAST when present (SCHEMA §7.3 — the slot order is
        // NORMATIVE and is mirrored verbatim by `KnownOperation::element_refs_mut`,
        // `document_runtime::element_ref_input` and `InputPath::OffsetFaceFace` /
        // `OffsetFaceOpposite`).
        //
        // Typed refs ONLY — deliberately no bare-`faceIds` fallback like
        // `edge_input_refs`'. OffsetFace is new in v2 and its session validation
        // requires the `faces`/`faceIds` lockstep, so a bare-id-only record cannot
        // be authored; synthesizing element-only refs for one would add slots the
        // three mirroring tables do not have, which is the H9 silent-mis-repair
        // hazard. `params.targetBodyId` carries the operated body.
        Operation::Known(KnownOperation::OffsetFace(p)) => p
            .faces
            .iter()
            .chain(p.opposite_face.iter())
            .map(element_ref_wire)
            .collect(),
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
        // PlaceComponent: NO wire input, even when `mate` is present (P3
        // WP-3.1, spec §5.5 — deliberate change from the pre-WP-3.1 shape,
        // which put `mate.target` here). The worker's generic
        // `resolve_input_refs` pre-flight treats ANY unresolved `inputs[]`
        // entry as blocking — a genuinely correct rule for a face/edge an op
        // structurally NEEDS (Hole can't drill nowhere), but wrong for a
        // mate: an unresolvable target must still let the component publish
        // at its frozen `placement` (spec: "never drop it, never silently
        // move it"), not skip the op entirely. `ComponentOp.cpp`'s
        // `resolve_mate_reseat` now owns mate resolution completely,
        // in-process, with exactly that non-blocking contract — putting the
        // ref here too would silently reintroduce the drop-on-NeedsRepair
        // bug this WP exists to prevent. `element_refs_mut`'s manual-repair
        // rebind surface is unaffected — it is a separate mechanism.
        Operation::Known(KnownOperation::PlaceComponent(_)) => Vec::new(),
        // DetachComponent: no mate, no identity — the record re-describes
        // geometry directly, so it has no topological input at all.
        Operation::Known(KnownOperation::DetachComponent(_)) => Vec::new(),
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

/// Shell open-face refs: prefer typed evidence. Legacy bare-id records retain an
/// element-only fallback, attaching the operated body when known.
fn face_input_refs(faces: &[ElementRef], face_ids: &[ElementId], bodies: &[BodyId]) -> Vec<Value> {
    if !faces.is_empty() {
        return faces.iter().map(element_ref_wire).collect();
    }
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
pub fn parse_plan_step(payload: &Value, envelope_step: usize) -> Result<PlanStepEvent, String> {
    let step_index = payload
        .get("stepIndex")
        .and_then(Value::as_u64)
        .and_then(|s| usize::try_from(s).ok())
        .ok_or("planStep missing or invalid stepIndex")?;
    if step_index != envelope_step {
        return Err(format!(
            "planStep payload stepIndex {step_index} != envelope stepIndex {envelope_step}"
        ));
    }
    Ok(PlanStepEvent {
        step_index,
        body_events: parse_body_events(payload.get("bodyEvents"))?,
        body_rank_keys: parse_body_rank_keys(payload.get("bodyEvents")),
        body_health: parse_body_health(payload.get("bodyEvents"))?,
        element_map_delta: parse_element_delta(payload.get("elementMapDelta"))?,
        needs_repair: parse_needs_repair(payload.get("needsRepair"), step_index)?,
        signatures: parse_signatures(payload.get("signatures")),
        diagnostics: parse_diagnostics(payload.get("diagnostics")),
        mate_placement: parse_mate_placement(payload.get("matePlacement")),
    })
}

/// SCHEMA §7.2 `matePlacement` (Component Library P3 WP-3.1, spec §5.5) —
/// OPTIONAL, absent on every step but a reseated `PlaceComponent`.
/// **Deliberately infallible, same reasoning as `parse_body_rank_keys`**:
/// this is a derived-geometry echo, not execution input — a malformed
/// payload here means "no reseat happened" (dropped), never a
/// `PROTOCOL_ERROR` that tears down an otherwise valid regen step.
fn parse_mate_placement(v: Option<&Value>) -> Option<Box<FrozenPlacement>> {
    serde_json::from_value(v?.clone()).ok().map(Box::new)
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

fn parse_body_health(v: Option<&Value>) -> Result<BTreeMap<BodyId, BodyHealth>, String> {
    let mut out = BTreeMap::new();
    let Some(events) = v.and_then(Value::as_array) else {
        return Ok(out);
    };
    for event in events {
        let Some(raw) = event.get("health") else {
            continue;
        };
        let health = match raw.as_str() {
            Some("healthy") => BodyHealth::Healthy,
            Some("quarantined") => BodyHealth::Quarantined,
            _ => return Err("bodyEvent health must be healthy or quarantined".into()),
        };
        let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
        if kind != "created" && kind != "modified" {
            return Err("bodyEvent health is valid only on created/modified events".into());
        }
        out.insert(body_field(event, "bodyId")?, health);
    }
    Ok(out)
}

/// Collects the OPTIONAL SCHEMA §7.2 `bodyEvents[].rankKey` evidence (VF-B6) into a
/// `bodyId → [i64; 5]` map.
///
/// **Deliberately infallible.** `rankKey` is diagnostic identity evidence, not
/// execution input: a malformed or missing key means "no claim" and is dropped, and
/// the tripwire simply does not fire for that body. Escalating it to a
/// `PROTOCOL_ERROR` would let a bad *diagnostic* tear down an otherwise valid regen —
/// the exact opposite of the failure bias this tripwire exists to enforce. Body ids
/// that fail to parse are also skipped here; `parse_body_events` already rejects the
/// frame for those on the authoritative path.
fn parse_body_rank_keys(v: Option<&Value>) -> BTreeMap<BodyId, [i64; 5]> {
    let mut out = BTreeMap::new();
    let Some(arr) = v.and_then(Value::as_array) else {
        return out;
    };
    for ev in arr {
        let Some(nums) = ev.get("rankKey").and_then(Value::as_array) else {
            continue;
        };
        if nums.len() != 5 {
            continue;
        }
        let mut key = [0i64; 5];
        let mut ok = true;
        for (slot, n) in key.iter_mut().zip(nums) {
            match n.as_i64() {
                Some(x) => *slot = x,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        // `rankKey` rides `created`/`modified` events, whose body is `bodyId`.
        if let (true, Ok(body)) = (ok, body_field(ev, "bodyId")) {
            out.insert(body, key);
        }
    }
    out
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
///
/// **An EMPTY `anchor` object is read as "no anchor" (H9).** `PlanExecutor.cpp`
/// renders `{"anchor": {}}` for a ref that was authored without one, but
/// `AnchorIntent::world_point` is a REQUIRED field, so serde rejected the item —
/// turning "this reference broke and carries no evidence", the most ordinary
/// unrepairable case there is, into a fatal `Protocol` error that tore the worker
/// down instead of surfacing a deterministic NeedsRepair the panel could act on.
/// (Found by `hole_ops::hole_host_face_rebind_moves_the_drill_to_the_repicked_face`,
/// which blinds a hole's face ref on purpose.) `anchor` is OPTIONAL evidence, so
/// a tolerant read here masks nothing: an anchor with no world point carries no
/// information the ladder could have used. Structural fields stay strict.
fn parse_needs_repair(v: Option<&Value>, step: usize) -> Result<Vec<RepairItem>, String> {
    let arr = v.and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let mut obj = item;
        if let Some(map) = obj.as_object_mut() {
            map.entry("stepIndex".to_string()).or_insert(json!(step));
            let anchorless = map
                .get("anchor")
                .is_some_and(|a| a.is_null() || a.get("worldPoint").is_none());
            if anchorless {
                map.remove("anchor");
            }
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
    let Some(array) = v.and_then(Value::as_array) else {
        if v.is_some_and(|value| !value.is_null()) {
            tracing::warn!("worker diagnostics ignored: expected array");
        }
        return Vec::new();
    };
    array
        .iter()
        .take(64)
        .filter_map(|value| {
            let Some(object) = value.as_object() else {
                tracing::warn!("worker diagnostic ignored: expected object");
                return None;
            };
            let (Some(code), Some(message)) = (
                object.get("code").and_then(Value::as_str),
                object.get("message").and_then(Value::as_str),
            ) else {
                tracing::warn!("worker diagnostic ignored: malformed required fields");
                return None;
            };
            if code.len() > 128 || message.len() > 4096 {
                tracing::warn!("worker diagnostic ignored: text exceeds bounds");
                return None;
            }
            let severity = match object.get("severity").and_then(Value::as_str) {
                Some("error") => Severity::Error,
                Some("info") => Severity::Info,
                Some("warning") => Severity::Warning,
                _ => {
                    tracing::warn!("worker diagnostic ignored: malformed severity");
                    return None;
                }
            };
            let stage = object
                .get("stage")
                .and_then(Value::as_str)
                .filter(|stage| stage.len() <= 64)
                .map(str::to_owned);
            if object.contains_key("stage") && stage.is_none() {
                tracing::warn!("worker diagnostic stage ignored: malformed or oversized");
            }
            // SCHEMA §7.2 `diagnostics[].reasonCode` — optional, ≤64 bytes.
            let reason_code = object
                .get("reasonCode")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty() && reason.len() <= 64)
                .map(str::to_owned);
            if object.contains_key("reasonCode") && reason_code.is_none() {
                tracing::warn!("worker diagnostic reasonCode ignored: malformed or oversized");
            }
            let evidence = object.get("evidence").and_then(|evidence| {
                if evidence.is_object() && evidence.to_string().len() <= 65_536 {
                    Some(evidence.clone())
                } else {
                    tracing::warn!("worker diagnostic evidence ignored: malformed or oversized");
                    None
                }
            });
            Some(Diagnostic {
                severity,
                code: code.to_owned(),
                message: message.to_owned(),
                stage,
                reason_code,
                evidence,
            })
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// PlanPrepared (SCHEMA §7.2)
// ─────────────────────────────────────────────────────────────────────────────

/// Parses a terminal `PlanPrepared` result, ignoring any inline mesh artifacts.
///
/// # Errors
/// A human reason on a missing `preparedSnapshotId` or a malformed `bodyIds`.
pub fn parse_plan_prepared(job: JobId, result: &Value) -> Result<PlanPrepared, String> {
    parse_plan_prepared_with_artifacts(job, result, None, &[])
}

/// Parses a terminal `PlanPrepared` result, lifting the optional inline
/// `artifacts.tessellate` meshes out of the resp's binary tail (SCHEMA §7.2).
/// `job` is the [`JobId`] Rust sent (the executor checks the prepare is for *this*
/// job), not re-parsed from the wire.
///
/// The artifacts are a **cache fill only** — a missing or malformed one degrades
/// to the `Tessellate` pull and never fails the plan (see
/// [`parse_plan_artifact_meshes`]).
///
/// # Errors
/// A human reason on a missing `preparedSnapshotId` or a malformed `bodyIds`.
/// Never on an artifact defect.
pub fn parse_plan_prepared_with_artifacts(
    job: JobId,
    result: &Value,
    sections: Option<&[BinSection]>,
    tail: &[u8],
) -> Result<PlanPrepared, String> {
    let prepared_snapshot_id = SnapshotId(
        result
            .get("preparedSnapshotId")
            .and_then(Value::as_u64)
            .ok_or("PlanPrepared missing preparedSnapshotId")?,
    );
    let last_valid_step = match result.get("lastValidStep") {
        Some(Value::Null) => None,
        Some(Value::Number(n)) => Some(
            n.as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .ok_or("PlanPrepared invalid lastValidStep")?,
        ),
        Some(_) => return Err("PlanPrepared invalid lastValidStep".into()),
        None => return Err("PlanPrepared missing lastValidStep".into()),
    };
    let stopped_reason = match result.get("stoppedReason") {
        Some(Value::String(reason)) if reason == "completed" => StoppedReason::Completed,
        Some(Value::String(reason)) if reason == "opFailed" => StoppedReason::OpFailed,
        Some(Value::String(reason)) if reason == "needsRepair" => StoppedReason::NeedsRepair,
        Some(Value::String(reason)) => {
            return Err(format!("PlanPrepared unknown stoppedReason {reason:?}"));
        }
        Some(_) => return Err("PlanPrepared invalid stoppedReason".into()),
        None => return Err("PlanPrepared missing stoppedReason".into()),
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
        artifact_meshes: parse_plan_artifact_meshes(result, sections, tail)
            .into_iter()
            .map(|m| PreparedMeshRef {
                body: m.body,
                lod: m.lod,
                bytes: m.bytes,
            })
            .collect(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Inline tessellate artifacts on the ExecutePlan terminal (SCHEMA §7.2)
// ─────────────────────────────────────────────────────────────────────────────

/// The most inline artifact geometry one `ExecutePlan` terminal may contribute
/// (256 MiB). A frame that large is already pathological; the cap keeps a
/// misbehaving worker from making Rust allocate the whole tail into the cache.
const ARTIFACT_INGEST_CAP: usize = 256 * 1024 * 1024;

/// One MESH1 blob lifted from an `ExecutePlan` terminal's inline tessellate
/// artifact — a body's mesh the worker already computed while preparing.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedMesh {
    pub body: BodyId,
    pub lod: Lod,
    pub bytes: Arc<Vec<u8>>,
}

/// Lifts `result.artifacts.tessellate.meshes[]` (SCHEMA §7.2, §7.6-shaped handles)
/// out of the terminal resp's binary tail.
///
/// **This never fails a plan.** The artifacts are an optimization: they let Rust
/// seed its mesh cache instead of issuing one `Tessellate` round-trip per body.
/// Every absence and every defect therefore degrades to "not cached" — the pull
/// path still serves that body (Invariant 7: a cache is acceleration, never a
/// correctness input):
///
/// * no `artifacts` key at all (an idempotent cached re-return deliberately omits
///   it, and a request without `artifacts.tessellate` never gets one) ⇒ empty;
/// * a bad section table ⇒ empty (all of it, since offsets can no longer be trusted);
/// * one malformed handle (unparseable body id, wrong `snapshotId`, missing/OOB
///   `bin`, `totalBytes`/`sha256` mismatch, invalid MESH1 header) ⇒ `warn` + skip
///   **that** handle; the rest are still ingested;
/// * past [`ARTIFACT_INGEST_CAP`] total bytes ⇒ `warn` once and stop.
#[must_use]
pub fn parse_plan_artifact_meshes(
    result: &Value,
    sections: Option<&[BinSection]>,
    tail: &[u8],
) -> Vec<PreparedMesh> {
    parse_artifact_meshes_capped(result, sections, tail, ARTIFACT_INGEST_CAP)
}

/// [`parse_plan_artifact_meshes`] with an explicit ingest cap, so the truncation
/// rule is testable without allocating a quarter-gigabyte fixture.
fn parse_artifact_meshes_capped(
    result: &Value,
    sections: Option<&[BinSection]>,
    tail: &[u8],
    cap: usize,
) -> Vec<PreparedMesh> {
    let handles = result
        .get("artifacts")
        .and_then(|a| a.get("tessellate"))
        .and_then(|t| t.get("meshes"))
        .and_then(Value::as_array);
    let Some(handles) = handles else {
        // A retransmitted (idempotent cached) prepare, or a plan whose request carried
        // no tessellate rider. Logged with `handles = 0` so "why did this regen still
        // pull every mesh?" is answerable from the lane alone.
        log_artifact_parse(0, 0, tail.len());
        return Vec::new();
    };
    // The prepared scratch snapshot every handle must belong to. A handle stamped
    // with a different snapshot is evidence of a worker mixing preparations, which
    // would cache the WRONG geometry under this publish's generation.
    let prepared_snapshot = result.get("preparedSnapshotId").and_then(Value::as_u64);

    let by_name =
        match validate_bin_sections("PlanPrepared artifacts", sections.unwrap_or(&[]), tail) {
            Ok(map) => map,
            Err(msg) => {
                tracing::warn!(
                    handles = handles.len(),
                    "PlanPrepared tessellate artifacts dropped (section table invalid): {msg} \
                 — falling back to Tessellate pulls"
                );
                log_artifact_parse(0, handles.len(), tail.len());
                return Vec::new();
            }
        };

    let mut meshes = Vec::with_capacity(handles.len());
    let mut ingested = 0usize;
    for handle in handles {
        match artifact_mesh(handle, prepared_snapshot, &by_name, tail) {
            Ok(mesh) => {
                if ingested.saturating_add(mesh.bytes.len()) > cap {
                    tracing::warn!(
                        cap,
                        ingested,
                        kept = meshes.len(),
                        total = handles.len(),
                        "PlanPrepared tessellate artifacts exceeded the ingest cap — \
                         remaining bodies fall back to Tessellate pulls"
                    );
                    break;
                }
                ingested += mesh.bytes.len();
                meshes.push(mesh);
            }
            Err(msg) => tracing::warn!(
                "PlanPrepared tessellate artifact skipped ({msg}) — that body falls back \
                 to a Tessellate pull"
            ),
        }
    }
    log_artifact_parse(meshes.len(), handles.len(), tail.len());
    meshes
}

/// The one observability line for the artifact lane, emitted on EVERY terminal
/// prepare — once per regen, never on a drag-frequency path. `artifact_meshes = 0`
/// with `handles = 0` is the retransmit / no-rider shape; a large `tail_bytes` is
/// the §5.2 inline-past-`chunkSize` case worth noticing.
fn log_artifact_parse(artifact_meshes: usize, handles: usize, tail_bytes: usize) {
    tracing::info!(
        artifact_meshes,
        handles,
        tail_bytes,
        "PlanPrepared: inline tessellate artifacts parsed"
    );
}

/// Decodes and verifies ONE §7.6 mesh handle against the resp tail.
fn artifact_mesh(
    handle: &Value,
    prepared_snapshot: Option<u64>,
    by_name: &HashMap<&str, &BinSection>,
    tail: &[u8],
) -> Result<PreparedMesh, String> {
    let body_str = handle
        .get("bodyId")
        .and_then(Value::as_str)
        .ok_or("mesh handle missing bodyId")?;
    let body = parse_body_id(body_str)?;
    // `format` is informational; only MESH1 is transportable here (Invariant 5).
    match handle.get("format").and_then(Value::as_str) {
        None | Some("MESH1") => {}
        Some(other) => {
            return Err(format!(
                "mesh handle {body_str:?} format {other:?} != MESH1"
            ))
        }
    }
    if let (Some(want), Some(got)) = (
        prepared_snapshot,
        handle.get("snapshotId").and_then(Value::as_u64),
    ) {
        if want != got {
            return Err(format!(
                "mesh handle {body_str:?} snapshotId {got} != preparedSnapshotId {want}"
            ));
        }
    }
    // Inline only: an ExecutePlan artifact never streams (the worker appends the
    // blob to this very resp's tail), so a `streamId` handle has no frames to join.
    let name = handle
        .get("bin")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("mesh handle {body_str:?} has no inline `bin` section"))?;
    let section = by_name
        .get(name)
        .ok_or_else(|| format!("mesh handle {body_str:?} names absent bin section {name:?}"))?;
    let start = section.off as usize;
    let end = start
        .checked_add(section.len as usize)
        .ok_or_else(|| format!("mesh handle {body_str:?} bin section range overflow"))?;
    let blob = tail
        .get(start..end)
        .ok_or_else(|| format!("mesh handle {body_str:?} bin section {name:?} out of range"))?;

    let total = handle.get("totalBytes").and_then(Value::as_u64);
    let sha = handle.get("sha256").and_then(Value::as_str);
    verify_mesh(blob, total, sha).map_err(|e| match e {
        EngineError::Protocol { message } => format!("mesh handle {body_str:?}: {message}"),
        other => format!("mesh handle {body_str:?}: {other:?}"),
    })?;

    Ok(PreparedMesh {
        body,
        // Unknown ⇒ Coarse, matching the pull path's tolerant reading. The rider
        // Rust attaches is Coarse, so this is the identity in practice.
        lod: lod_from_str(
            handle
                .get("lod")
                .and_then(Value::as_str)
                .unwrap_or("coarse"),
        ),
        bytes: Arc::new(blob.to_vec()),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// MESH1 integrity (shared by the Tessellate pull and the inline plan artifacts)
// ─────────────────────────────────────────────────────────────────────────────

/// Reconciles a manifest-level and a resp-level copy of an integrity field: both
/// present and unequal ⇒ error; otherwise the present value (manifest preferred),
/// or `None` when neither carries it (F5).
///
/// # Errors
/// [`EngineError::Protocol`] when both copies are present and disagree.
pub(super) fn reconcile_field<T: PartialEq + std::fmt::Display>(
    manifest: Option<T>,
    resp: Option<T>,
    field: &str,
) -> Result<Option<T>, EngineError> {
    match (manifest, resp) {
        (Some(m), Some(r)) => {
            if m != r {
                return Err(EngineError::Protocol {
                    message: format!("mesh {field} mismatch: manifest {m} != resp {r}"),
                });
            }
            Ok(Some(m))
        }
        (Some(m), None) => Ok(Some(m)),
        (None, r) => Ok(r),
    }
}

/// Verifies a MESH1 blob's declared size + SHA-256 (when declared) and validates
/// its header (Invariant 5 forward-verbatim).
///
/// # Errors
/// [`EngineError::Protocol`] on a length mismatch, a digest mismatch, or an
/// invalid MESH1 header.
pub(super) fn verify_mesh(
    blob: &[u8],
    total: Option<u64>,
    sha: Option<&str>,
) -> Result<(), EngineError> {
    if let Some(t) = total {
        if blob.len() as u64 != t {
            return Err(EngineError::Protocol {
                message: "MESH1 assembled length != totalBytes".into(),
            });
        }
    }
    if let Some(want) = sha {
        if crate::imports::sha256_hex(blob) != want {
            return Err(EngineError::Protocol {
                message: "MESH1 SHA-256 mismatch (corrupt stream)".into(),
            });
        }
    }
    onecad_protocol::mesh::validate_mesh_blob(blob).map_err(|e| EngineError::Protocol {
        message: format!("MESH1 header invalid: {e}"),
    })?;
    Ok(())
}

fn parse_per_step(v: Option<&Value>) -> Result<Vec<StepResult>, String> {
    let arr = v
        .and_then(Value::as_array)
        .ok_or("PlanPrepared missing or invalid perStepResults")?;
    arr.iter()
        .map(|r| {
            let step_index = r
                .get("stepIndex")
                .and_then(Value::as_u64)
                .and_then(|s| usize::try_from(s).ok())
                .ok_or("PlanPrepared perStepResults missing or invalid stepIndex")?;
            let status = match r.get("status") {
                Some(Value::String(status)) if status == "ok" => StepStatus::Ok,
                Some(Value::String(status)) if status == "needsRepair" => StepStatus::NeedsRepair,
                Some(Value::String(status)) if status == "opFailed" => StepStatus::OpFailed,
                Some(Value::String(status)) => {
                    return Err(format!(
                        "PlanPrepared perStepResults unknown status {status:?}"
                    ));
                }
                Some(_) => return Err("PlanPrepared perStepResults invalid status".into()),
                None => return Err("PlanPrepared perStepResults missing status".into()),
            };
            Ok(StepResult {
                step_index,
                status,
                body_ids: body_array(r.get("bodyIds"))?,
                message: r
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                diagnostics: parse_diagnostics(r.get("diagnostics")),
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
///
/// `bodyNames` / `bodyColors` / `faceColors` are ADDITIVE (DI-5) and each is omitted
/// when empty, so an attribute-less export is byte-identical to the pre-DI-5 request
/// the worker's older behaviour is pinned against.
#[must_use]
pub fn export_step_args(
    path: &str,
    bodies: &[BodyId],
    schema: &str,
    attributes: &crate::export::StepExportAttributes,
) -> Value {
    let mut args = json!({
        "path": path,
        "bodyIds": bodies.iter().map(|b| body_id_wire(*b)).collect::<Vec<_>>(),
        "schema": schema,
    });
    let obj = args.as_object_mut().expect("json! built an object");
    if !attributes.body_names.is_empty() {
        obj.insert("bodyNames".into(), json!(attributes.body_names));
    }
    if !attributes.body_colors.is_empty() {
        obj.insert("bodyColors".into(), json!(attributes.body_colors));
    }
    if !attributes.face_colors.is_empty() {
        obj.insert("faceColors".into(), json!(attributes.face_colors));
    }
    args
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

/// `ExportGeometry.args` (SCHEMA §7.8): `{path, bodyIds, codec, union}`.
///
/// `codec` is one of the §7.3 REPLAY codecs (`"brep"` / `"xbf"`) — this verb bakes
/// a live body into the same byte form `ImportStep` reads back, which is what a
/// Component Library `embedded` / `document` source is made of (spec §2.1).
///
/// `union_solids` is the opt-in multi-solid fuse (WP-F1.2): the worker fuses a
/// body that flattens to more than one solid BEFORE writing, and refuses when the
/// fused result is still not one solid. Always sent explicitly so the wire never
/// depends on the worker's default.
#[must_use]
pub fn export_geometry_args(
    path: &str,
    bodies: &[BodyId],
    codec: &str,
    union_solids: bool,
) -> Value {
    json!({
        "path": path,
        "bodyIds": bodies.iter().map(|b| body_id_wire(*b)).collect::<Vec<_>>(),
        "codec": codec,
        "union": union_solids,
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
///
/// The epoch comes from the request (i.e. from the plan), not from the manager: a
/// restore and the plan it seeds must fence against ONE epoch (VF-B4).
#[must_use]
pub fn restore_checkpoint_args(req: &RestoreRequest) -> Value {
    json!({
        "checkpointId": req.checkpoint.checkpoint_id.as_str(),
        "stepIndex": req.checkpoint.step_index,
        "expectedHistoryPrefixHash": req.expected_history_prefix_hash.as_str(),
        "workerEpoch": req.worker_epoch.0,
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
    let diagnostic_value = match err.detail.as_ref() {
        Some(detail) if detail.is_object() => detail.get("diagnostics"),
        Some(_) => {
            tracing::warn!("worker error detail ignored: expected object");
            None
        }
        None => None,
    };
    let diagnostics = parse_diagnostics(diagnostic_value);
    let op = |code| EngineError::OpFailed {
        code,
        recoverable: true,
        message: err.message.clone(),
        diagnostics: diagnostics.clone(),
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

/// What the pointer grabbed (SCHEMA §7.4 `BeginGesture.drag.kind`). The tokens are
/// camelCase and matched EXACTLY by the worker's `parse_drag_kind` — an unknown one
/// is `OP_FAILED` there, never degraded to `point` (degrading would move a handle
/// the user never grabbed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragKind {
    /// One point handle — the pre-SP-2 gesture, wire-identical to it.
    Point,
    /// An arc endpoint: reshapes radius + the swept angle, center held.
    ArcEnd,
    /// A curve's radius parameter (moves no point).
    Radius,
    /// Every point an entity owns (translates the whole entity).
    EntityBody,
}

impl DragKind {
    /// The exact wire token.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::ArcEnd => "arcEnd",
            Self::Radius => "radius",
            Self::EntityBody => "entityBody",
        }
    }

    /// Parses a wire token STRICTLY — an unknown token is `None`, and the caller
    /// MUST reject it rather than fall back to [`Point`](Self::Point) (§7.4).
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "point" => Some(Self::Point),
            "arcEnd" => Some(Self::ArcEnd),
            "radius" => Some(Self::Radius),
            "entityBody" => Some(Self::EntityBody),
            _ => None,
        }
    }
}

/// The resolved `BeginGesture.drag` target (SCHEMA §7.4).
///
/// `role` is a `&'static str` on purpose: the only legal values are the five §7.4
/// tokens, so the api layer must MAP a client string onto one (rejecting anything
/// else) instead of forwarding it — an unresolvable role is `REF_UNRESOLVED` at the
/// worker, and a typo must fail at the boundary, not silently ride the wire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureTarget {
    /// What the pointer grabbed.
    pub kind: DragKind,
    /// The grabbed entity (the point itself for [`DragKind::Point`]).
    pub entity: EntityId,
    /// `start`|`end`|`center`|`p0`|`p1`; REQUIRED for [`DragKind::ArcEnd`].
    pub role: Option<&'static str>,
    /// Pointer-down position in sketch-plane coordinates. The worker derives the
    /// per-kind offset from it ONCE, at `BeginGesture`, so a gesture cannot drift.
    pub grab: Option<[f64; 2]>,
}

impl GestureTarget {
    /// The plain point drag — the default when a client sends no `target`, and the
    /// one kind that keeps the pre-SP-2 wire form byte for byte.
    #[must_use]
    pub const fn point(entity: EntityId) -> Self {
        Self {
            kind: DragKind::Point,
            entity,
            role: None,
            grab: None,
        }
    }
}

/// `BeginGesture.args` (SCHEMA §7.4). `drag_point` is the point entity being
/// dragged — its wire handle is its uuid (points register under their id).
///
/// `target` names WHAT the pointer grabbed. [`DragKind::Point`] emits the
/// pre-SP-2 form **verbatim** (`drag.pointId` + the bare top-level `pointId`, no
/// `kind` key): §7.4 resolves those first and lets `pointId` win on the point
/// path, so the legacy request stays byte-identical and `target.entity` is not
/// consulted there. Every other kind emits `drag.{kind, entity[, role][, grab]}`
/// and drops both legacy keys — a `pointId` riding along with `kind: "radius"`
/// would be ignored by the worker anyway, and emitting it would only invite a
/// reader to degrade the gesture to a point drag.
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
    target: &GestureTarget,
    solver_policy_hash: &str,
) -> Value {
    if target.kind == DragKind::Point {
        return json!({
            "sketchId": sketch_id,
            "sketchRevision": sketch_revision,
            "gestureId": gesture_id,
            "solverPolicyHash": solver_policy_hash,
            "drag": { "pointId": drag_point.to_string() },
            "pointId": drag_point.to_string(),
        });
    }
    let mut drag = json!({
        "kind": target.kind.wire(),
        "entity": target.entity.to_string(),
    });
    if let Some(role) = target.role {
        drag["role"] = json!(role);
    }
    if let Some([x, y]) = target.grab {
        drag["grab"] = json!([x, y]);
    }
    json!({
        "sketchId": sketch_id,
        "sketchRevision": sketch_revision,
        "gestureId": gesture_id,
        "solverPolicyHash": solver_policy_hash,
        "drag": drag,
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
        // Additive since SP-2 (§7.4): a pre-SP-2 worker omits it ⇒ empty.
        solved_curves: parse_curves(result.get("curves")),
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
        curves: parse_curves(result.get("curves")),
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
) -> Result<crate::dto::FinishSketchDto, String> {
    let region_identity_version = validate_sketch_region_header(expected_sketch_id, result)?;
    let regions = result
        .get("regions")
        .and_then(Value::as_array)
        .ok_or("SketchRegions: missing/invalid regions array")?;
    if regions.is_empty() {
        if bin_sections.is_empty() && tail.is_empty() {
            return Ok(crate::dto::FinishSketchDto {
                region_identity_version,
                regions: Vec::new(),
            });
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
    Ok(crate::dto::FinishSketchDto {
        region_identity_version,
        regions: parsed,
    })
}

fn validate_sketch_region_header(expected_sketch_id: &str, result: &Value) -> Result<u32, String> {
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
    let version = result
        .get("regionIdentityVersion")
        .and_then(Value::as_u64)
        .ok_or("SketchRegions: missing/invalid regionIdentityVersion")?;
    if !matches!(version, 2 | 3) {
        return Err(format!(
            "SketchRegions: unsupported regionIdentityVersion {version}; expected 2 or 3"
        ));
    }
    Ok(version as u32)
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
// Element identity (SCHEMA §7.5) — Acquire / Bind / Resolve
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

/// `BindElementIds.args` (SCHEMA §7.5) — snapshot-fenced, all-or-nothing
/// installation of Rust-minted ids into the authoritative worker head.
#[must_use]
pub fn bind_element_ids_args(req: &BindElementIdsRequest) -> Value {
    let bindings: Vec<Value> = req
        .bindings
        .iter()
        .map(|binding| {
            let mut value = json!({
                "bodyId": body_id_wire(binding.body),
                "topoKey": binding.topo_key.as_str(),
                "elementId": binding.element_id.as_str(),
                "kind": element_kind_str(binding.kind),
            });
            if let Some(anchor) = &binding.anchor {
                value["anchor"] = anchor_to_wire(anchor);
            }
            value
        })
        .collect();
    json!({ "snapshotId": req.snapshot_id.0, "bindings": bindings })
}

/// Validates the worker's exact, order-preserving binding echo. A successful
/// response is the promotion commit point: every id is now directly queryable.
pub fn validate_bind_element_ids_result(
    req: &BindElementIdsRequest,
    result: &Value,
) -> Result<(), String> {
    let bound = result
        .get("bound")
        .and_then(Value::as_array)
        .ok_or_else(|| "BindElementIds: missing bound array".to_string())?;
    if bound.len() != req.bindings.len() {
        return Err(format!(
            "BindElementIds: expected {} bindings, got {}",
            req.bindings.len(),
            bound.len()
        ));
    }
    for (index, (actual, expected)) in bound.iter().zip(&req.bindings).enumerate() {
        let matches = actual.get("bodyId").and_then(Value::as_str)
            == Some(body_id_wire(expected.body).as_str())
            && actual.get("topoKey").and_then(Value::as_str) == Some(expected.topo_key.as_str())
            && actual.get("elementId").and_then(Value::as_str)
                == Some(expected.element_id.as_str())
            && actual.get("kind").and_then(Value::as_str) == Some(element_kind_str(expected.kind));
        if !matches {
            return Err(format!("BindElementIds: mismatched bound[{index}]"));
        }
    }
    Ok(())
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
// Interactive surface classification (SCHEMA §7.5 `ClassifyElement`; Component
// Library WP-0.1)
// ─────────────────────────────────────────────────────────────────────────────

/// `ClassifyElement` args, addressed by ElementId (SCHEMA §7.5).
///
/// No `snapshotId` — unlike `QueryElement`'s pick-time addressing (Invariant
/// 4), this verb serves a continuously re-issued LIVE hover query and always
/// reads the current head, same as `QueryMassProperties`.
#[must_use]
pub fn classify_element_args(body: BodyId, element: &str) -> Value {
    json!({
        "bodyId": body_id_wire(body),
        "elementId": element,
    })
}

/// `ClassifyElement` args, addressed by `{bodyId, topoKey}` — the form a live
/// raycast pick naturally has BEFORE any ElementId promotion.
#[must_use]
pub fn classify_element_by_topo_key_args(body: BodyId, topo_key: &str) -> Value {
    json!({
        "bodyId": body_id_wire(body),
        "topoKey": topo_key,
    })
}

/// Parses a `ClassifyElement` result. `None` when the reference does not
/// resolve against the current head (`present: false`) — an ANSWER, not an
/// error, matching `ProjectFaceBoundary`'s convention.
#[must_use]
pub fn parse_classify_element(result: &Value) -> Option<crate::dto::ClassifyElementDto> {
    if result.get("present").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let str_at = |key: &str| {
        result
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let vec3 = |o: &Value, key: &str| -> Option<[f64; 3]> {
        o.get(key)
            .and_then(Value::as_array)
            .filter(|a| a.len() == 3)
            .map(|a| {
                let g = |i: usize| a[i].as_f64().unwrap_or(0.0);
                [g(0), g(1), g(2)]
            })
    };
    let frame = result
        .get("frame")
        .map(|f| crate::dto::ClassifyElementFrameDto {
            // A malformed origin is a PROTOCOL break, never a fabricated (0,0,0):
            // that would seat a mate at the world origin instead of failing loudly.
            origin: vec3(f, "origin").unwrap_or([0.0, 0.0, 0.0]),
            normal: vec3(f, "normal"),
            axis: vec3(f, "axis"),
            radius: f.get("radius").and_then(Value::as_f64),
        });
    Some(crate::dto::ClassifyElementDto {
        kind: str_at("kind"),
        surface_type: str_at("surfaceType"),
        curve_type: str_at("curveType"),
        frame,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Mass properties (SCHEMA §7.5 `QueryMassProperties`; WP-C1)
// ─────────────────────────────────────────────────────────────────────────────

/// `QueryMassProperties` args (SCHEMA §7.5).
///
/// `snapshotId` is deliberately ABSENT: the verb reads the worker's current head
/// copy and a body id is durable across snapshots, so there is nothing for a
/// snapshot stamp to disambiguate. Sending one would imply a per-snapshot lookup
/// the worker does not perform.
#[must_use]
pub fn query_mass_properties_args(body: BodyId) -> Value {
    json!({ "bodyId": body_id_wire(body) })
}

/// Parses a `QueryMassProperties` result into a [`crate::dto::MassPropertiesDto`].
///
/// `body_id` is the CALLER's own string rather than anything off the wire: the
/// worker keys its store by `body_<uuid>` and the frontend speaks its own form,
/// and echoing the caller's is what keeps the DTO addressable by the id that was
/// asked about.
///
/// # Errors
/// Returns the offending field name when a required number/array is missing or
/// malformed. A mass reading has no honest default — a fabricated `0` would
/// render as a real "0 mm³" measurement — so this never falls back.
pub fn parse_mass_properties(
    body_id: String,
    result: &Value,
) -> Result<crate::dto::MassPropertiesDto, String> {
    let num = |key: &str| -> Result<f64, String> {
        result
            .get(key)
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("QueryMassProperties: missing/invalid {key:?}"))
    };
    let vec3 = |value: Option<&Value>, what: &str| -> Result<[f64; 3], String> {
        let a = value
            .and_then(Value::as_array)
            .filter(|a| a.len() == 3)
            .ok_or_else(|| format!("QueryMassProperties: {what} is not a 3-array"))?;
        let mut out = [0.0f64; 3];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = a[i]
                .as_f64()
                .ok_or_else(|| format!("QueryMassProperties: {what}[{i}] is not a number"))?;
        }
        Ok(out)
    };

    let axes_raw = result
        .get("principalAxes")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 3)
        .ok_or_else(|| "QueryMassProperties: principalAxes is not a 3-array".to_string())?;
    let mut principal_axes = [[0.0f64; 3]; 3];
    for (i, row) in principal_axes.iter_mut().enumerate() {
        *row = vec3(Some(&axes_raw[i]), &format!("principalAxes[{i}]"))?;
    }

    Ok(crate::dto::MassPropertiesDto {
        body_id,
        volume: num("volume")?,
        surface_area: num("surfaceArea")?,
        centroid: vec3(result.get("centroid"), "centroid")?,
        principal_moments: vec3(result.get("principalMoments"), "principalMoments")?,
        principal_axes,
    })
}

/// `QueryBodyTopology` args (SCHEMA §7.5).
#[must_use]
pub fn query_body_topology_args(body: BodyId) -> Value {
    json!({ "bodyId": body_id_wire(body) })
}

/// Parses exact BRep topology counts. No default is honest here: a malformed
/// result must not masquerade as an empty body.
pub fn parse_body_topology(
    body_id: String,
    result: &Value,
) -> Result<crate::dto::BodyTopologyDto, String> {
    let count = |key: &str| {
        result
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| format!("QueryBodyTopology: missing/invalid {key:?}"))
    };
    Ok(crate::dto::BodyTopologyDto {
        body_id,
        solid_count: count("solidCount")?,
        face_count: count("faceCount")?,
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

// ─────────────────────────────────────────────────────────────────────────────
// Edge-op authoring handshake (`PrepareEdgeOp`)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeOpMode {
    Fillet,
    Chamfer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeOpPick<'a> {
    pub body: Option<BodyId>,
    pub address: FaceAddress<'a>,
}

#[must_use]
pub fn prepare_edge_op_args(
    snapshot: SnapshotId,
    mode: EdgeOpMode,
    picks: &[EdgeOpPick<'_>],
    chain_tangent_edges: bool,
) -> Value {
    let picked: Vec<Value> = picks
        .iter()
        .map(|pick| {
            let mut entry = json!({});
            if let Some(body) = pick.body {
                entry["bodyId"] = Value::String(body_id_wire(body));
            }
            pick.address.write_into(&mut entry);
            entry
        })
        .collect();
    json!({
        "snapshotId": snapshot.0,
        "mode": match mode { EdgeOpMode::Fillet => "Fillet", EdgeOpMode::Chamfer => "Chamfer" },
        "pickedEdges": picked,
        "chainTangentEdges": chain_tangent_edges,
    })
}

pub fn parse_prepare_edge_op(result: &Value) -> Result<crate::dto::PrepareEdgeOpDto, String> {
    let snapshot_id = result
        .get("snapshotId")
        .and_then(Value::as_u64)
        .ok_or_else(|| "PrepareEdgeOp: result is missing `snapshotId`".to_string())?;
    let target_body_id = result
        .get("targetBodyId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let refusal = parse_edge_op_refusal(result.get("refusal"))?;
    let edges = match result.get("edges") {
        Some(Value::Array(items)) => items
            .iter()
            .map(parse_edge_op_evidence)
            .collect::<Result<Vec<_>, _>>()?,
        None | Some(Value::Null) => Vec::new(),
        Some(_) => return Err("PrepareEdgeOp: `edges` must be an array".into()),
    };
    if refusal.is_none() && (target_body_id.is_empty() || edges.is_empty()) {
        return Err("PrepareEdgeOp: accepted result has no body or closure".into());
    }
    Ok(crate::dto::PrepareEdgeOpDto {
        snapshot_id,
        target_body_id,
        edges,
        refusal,
    })
}

fn parse_edge_op_evidence(v: &Value) -> Result<crate::dto::EdgeOpEvidenceDto, String> {
    let topo_key = v
        .get("topoKey")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| "PrepareEdgeOp: edge is missing `topoKey`".to_string())?;
    Ok(crate::dto::EdgeOpEvidenceDto {
        topo_key: topo_key.to_string(),
        picked: v.get("picked").and_then(Value::as_bool).unwrap_or(false),
        element_id: None,
        body_id: None,
        kind: None,
        anchor: v.get("anchor").filter(|value| !value.is_null()).cloned(),
        descriptor: v
            .get("descriptor")
            .filter(|value| !value.is_null())
            .cloned(),
    })
}

fn parse_edge_op_refusal(
    value: Option<&Value>,
) -> Result<Option<crate::dto::EdgeOpRefusalDto>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let code = value
        .get("code")
        .and_then(Value::as_str)
        .filter(|code| !code.is_empty())
        .ok_or_else(|| "PrepareEdgeOp: refusal is missing `code`".to_string())?;
    Ok(Some(crate::dto::EdgeOpRefusalDto {
        code: code.to_string(),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        edges: str_array(value.get("edges")),
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge-op feasible range (SCHEMA §7.6 `AnalyzeEdgeOpRange`)
// ─────────────────────────────────────────────────────────────────────────────

/// The optional mm window and build budget a caller may put on the search.
/// Both are HINTS the worker clamps; neither is echoed back unchanged, which is
/// why the response carries its own `searchedRange`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EdgeOpRangeRequest {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub probe_budget: Option<u32>,
}

#[must_use]
pub fn analyze_edge_op_range_args(
    snapshot: SnapshotId,
    mode: EdgeOpMode,
    picks: &[EdgeOpPick<'_>],
    chain_tangent_edges: bool,
    request: EdgeOpRangeRequest,
) -> Value {
    let picked: Vec<Value> = picks
        .iter()
        .map(|pick| {
            let mut entry = json!({});
            if let Some(body) = pick.body {
                entry["bodyId"] = Value::String(body_id_wire(body));
            }
            pick.address.write_into(&mut entry);
            entry
        })
        .collect();
    let mut args = json!({
        "snapshotId": snapshot.0,
        "mode": match mode { EdgeOpMode::Fillet => "Fillet", EdgeOpMode::Chamfer => "Chamfer" },
        "pickedEdges": picked,
        "chainTangentEdges": chain_tangent_edges,
    });
    // Omitted, not defaulted: an absent `range` means "the kernel's own
    // body-derived bracket", and there is no number Rust could put here that
    // says that.
    let mut range = json!({});
    if let Some(min) = request.min {
        range["min"] = json!(min);
    }
    if let Some(max) = request.max {
        range["max"] = json!(max);
    }
    if !range.as_object().is_some_and(serde_json::Map::is_empty) {
        args["range"] = range;
    }
    if let Some(budget) = request.probe_budget {
        args["probeBudget"] = json!(budget);
    }
    args
}

/// The two vocabularies a consumer BRANCHES on. Both are validated fail-closed:
/// an unrecognised rung means the frontend cannot know what it is allowed to
/// enforce, and guessing a clamp from an unknown confidence is exactly the
/// silent-wrong-answer failure this verb exists to remove. A loud protocol error
/// is the safe direction.
const EDGE_OP_RANGE_CONFIDENCE: [&str; 5] =
    ["none", "nonMonotonic", "lowerOnly", "bracketed", "coarse"];
const EDGE_OP_RANGE_STOPPED: [&str; 3] = ["converged", "budgetExhausted", "deadline"];

pub fn parse_analyze_edge_op_range(result: &Value) -> Result<crate::dto::EdgeOpRangeDto, String> {
    let snapshot_id = result
        .get("snapshotId")
        .and_then(Value::as_u64)
        .ok_or_else(|| "AnalyzeEdgeOpRange: result is missing `snapshotId`".to_string())?;
    let mode = result
        .get("mode")
        .and_then(Value::as_str)
        .filter(|m| *m == "Fillet" || *m == "Chamfer")
        .ok_or_else(|| "AnalyzeEdgeOpRange: result is missing a known `mode`".to_string())?;
    let confidence = result
        .get("confidence")
        .and_then(Value::as_str)
        .filter(|value| EDGE_OP_RANGE_CONFIDENCE.contains(value))
        .ok_or_else(|| "AnalyzeEdgeOpRange: unknown `confidence`".to_string())?;
    let stopped_reason = result
        .get("stoppedReason")
        .and_then(Value::as_str)
        .filter(|value| EDGE_OP_RANGE_STOPPED.contains(value))
        .ok_or_else(|| "AnalyzeEdgeOpRange: unknown `stoppedReason`".to_string())?;
    let lower_bound = optional_f64(result.get("lowerBound"));
    let best_known_max = optional_f64(result.get("bestKnownMax"));
    let proven_upper_bound = optional_f64(result.get("provenUpperBound"));
    // The §7.6 normative invariant, checked at the boundary rather than trusted.
    // Every consumer downstream clamps with these three; a violation here would
    // author a ceiling below its own floor.
    if let (Some(lower), Some(best)) = (lower_bound, best_known_max) {
        if lower > best {
            return Err("AnalyzeEdgeOpRange: lowerBound exceeds bestKnownMax".into());
        }
    }
    if let (Some(best), Some(upper)) = (best_known_max, proven_upper_bound) {
        if best >= upper {
            return Err("AnalyzeEdgeOpRange: bestKnownMax is not below provenUpperBound".into());
        }
    }
    let feasible_intervals = match result.get("feasibleIntervals") {
        Some(Value::Array(items)) => items
            .iter()
            .map(parse_edge_op_range_interval)
            .collect::<Result<Vec<_>, _>>()?,
        None | Some(Value::Null) => Vec::new(),
        Some(_) => return Err("AnalyzeEdgeOpRange: `feasibleIntervals` must be an array".into()),
    };
    Ok(crate::dto::EdgeOpRangeDto {
        snapshot_id,
        mode: mode.to_string(),
        target_body_id: result
            .get("targetBodyId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        edges: str_array(result.get("edges")),
        searched_range: crate::dto::EdgeOpRangeWindowDto {
            min: optional_f64(result.get("searchedRange").and_then(|w| w.get("min")))
                .unwrap_or(0.0),
            max: optional_f64(result.get("searchedRange").and_then(|w| w.get("max")))
                .unwrap_or(0.0),
        },
        lower_bound,
        best_known_max,
        proven_upper_bound,
        feasible_intervals,
        intervals_truncated: result
            .get("intervalsTruncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        limiting_entities: result
            .get("limitingEntities")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(parse_edge_op_limiting).collect())
            .unwrap_or_default(),
        confidence: confidence.to_string(),
        monotonic_observed: result
            .get("monotonicObserved")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        probes_used: result
            .get("probesUsed")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .try_into()
            .unwrap_or(u32::MAX),
        budget_exhausted: result
            .get("budgetExhausted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        stopped_reason: stopped_reason.to_string(),
        refusal: parse_edge_op_refusal(result.get("refusal"))?,
    })
}

/// `null` and "absent" both mean UNPROVEN, and neither may become a number: 0.0
/// is a radius a caller could act on.
fn optional_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|v| v.is_finite())
}

fn parse_edge_op_range_interval(v: &Value) -> Result<crate::dto::EdgeOpRangeIntervalDto, String> {
    let lower = optional_f64(v.get("lower"))
        .ok_or_else(|| "AnalyzeEdgeOpRange: interval is missing `lower`".to_string())?;
    let upper = optional_f64(v.get("upper"))
        .ok_or_else(|| "AnalyzeEdgeOpRange: interval is missing `upper`".to_string())?;
    if lower > upper {
        return Err("AnalyzeEdgeOpRange: interval is inverted".into());
    }
    Ok(crate::dto::EdgeOpRangeIntervalDto { lower, upper })
}

fn parse_edge_op_limiting(v: &Value) -> Option<crate::dto::EdgeOpLimitingEntityDto> {
    let topo_key = v
        .get("topoKey")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())?;
    Some(crate::dto::EdgeOpLimitingEntityDto {
        topo_key: topo_key.to_string(),
        kind: v
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("edge")
            .to_string(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// OffsetFace authoring handshake (SCHEMA §7.6 `PrepareOffsetFace`)
// ─────────────────────────────────────────────────────────────────────────────

/// One `pickedFaces[]` entry (SCHEMA §7.6 `PrepareOffsetFace`): the same two
/// mutually-exclusive addressing rungs [`FaceAddress`] models, plus the `bodyId`
/// the `topoKey` rung resolves against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OffsetFacePick<'a> {
    /// The picked face's body. Required by the `topoKey` rung; the `elementId`
    /// rung overwrites it from the partition entry, so it is optional there.
    pub body: Option<BodyId>,
    pub address: FaceAddress<'a>,
}

/// `PrepareOffsetFace.args` (SCHEMA §7.6) — the READ-ONLY half of the OffsetFace
/// authoring transaction.
///
/// Rust sends picks and reads back TopoKey EVIDENCE; it never asks the worker for
/// an `ElementId` and the worker never mints one (Invariant 2). `snapshotId` is a
/// real fence here, unlike the advisory §7.5 reads: the closure this returns is
/// about to be FROZEN into a document record, so a stale head must be refused
/// rather than answered.
#[must_use]
pub fn prepare_offset_face_args(
    snapshot: SnapshotId,
    picks: &[OffsetFacePick<'_>],
    chain_tangent_faces: bool,
    distance_type: OffsetDistanceType,
) -> Value {
    let picked: Vec<Value> = picks
        .iter()
        .map(|p| {
            let mut entry = json!({});
            if let Some(b) = p.body {
                entry["bodyId"] = Value::String(body_id_wire(b));
            }
            p.address.write_into(&mut entry);
            entry
        })
        .collect();
    json!({
        "snapshotId": snapshot.0,
        "pickedFaces": picked,
        "chainTangentFaces": chain_tangent_faces,
        "distanceType": offset_distance_type_wire(distance_type),
    })
}

/// The SCHEMA §7.3/§7.6 wire spelling of an [`OffsetDistanceType`].
///
/// Goes through the core serde rather than a hand-written table so the two can
/// never drift: the params `distanceType` and this verb's argument are the SAME
/// enumeration, and a divergence would refuse a perfectly good handshake.
fn offset_distance_type_wire(t: OffsetDistanceType) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "Offset".into())
}

/// Parses a `PrepareOffsetFace` result into the frontend DTO (SCHEMA §7.6).
///
/// **A `refusal` is an ANSWER, not an error** — `crossBody`, `chainMismatch`,
/// `unsupportedSurface` and friends all arrive with `ok:true` and are handed to
/// the caller intact, so the tool can explain itself instead of failing blank.
/// A structurally malformed result IS an error: the caller is about to freeze
/// this closure into a record, and a defaulted `targetBodyId` or a silently empty
/// `faces` list would freeze the WRONG one.
///
/// `descriptor` and `anchor` ride through as opaque `serde_json::Value`s — they
/// are worker-owned evidence the core never interprets (Invariant 2), and Rust
/// re-emits them verbatim when it authors the typed refs.
///
/// # Errors
/// A human reason naming the malformed field.
pub fn parse_prepare_offset_face(
    result: &Value,
) -> Result<crate::dto::PrepareOffsetFaceDto, String> {
    let snapshot_id = result
        .get("snapshotId")
        .and_then(Value::as_u64)
        .ok_or_else(|| "PrepareOffsetFace: result is missing `snapshotId`".to_string())?;
    let target_body_id = result
        .get("targetBodyId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let refusal = parse_offset_refusal(result.get("refusal"))?;
    // A refusal answers INSTEAD of a closure, so the shape checks below are only
    // meaningful on the accepting path.
    if refusal.is_none() && target_body_id.is_empty() {
        return Err("PrepareOffsetFace: accepted result carries no `targetBodyId`".into());
    }
    let faces = match result.get("faces") {
        Some(Value::Array(items)) => items
            .iter()
            .map(parse_offset_face_evidence)
            .collect::<Result<Vec<_>, _>>()?,
        None | Some(Value::Null) => Vec::new(),
        Some(_) => return Err("PrepareOffsetFace: `faces` must be an array".into()),
    };
    if refusal.is_none() && faces.is_empty() {
        return Err("PrepareOffsetFace: accepted result carries an empty `faces` closure".into());
    }
    let opposite_face = match result.get("oppositeFace") {
        None | Some(Value::Null) => None,
        Some(v) => Some(parse_offset_face_evidence(v)?),
    };
    let dims = result.get("currentDims");
    Ok(crate::dto::PrepareOffsetFaceDto {
        snapshot_id,
        target_body_id,
        faces,
        opposite_face,
        current_dims: crate::dto::OffsetCurrentDimsDto {
            radius: offset_finite(dims, "radius"),
            thickness: offset_finite(dims, "thickness"),
        },
        refusal,
    })
}

/// A `currentDims` reading, present only when the worker could measure it. A
/// non-finite number is dropped rather than surfaced — an absent seed makes the
/// tool ask for a value, a `NaN` seed would render as one.
fn offset_finite(dims: Option<&Value>, key: &str) -> Option<f64> {
    dims?
        .get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
}

fn parse_offset_face_evidence(v: &Value) -> Result<crate::dto::OffsetFaceEvidenceDto, String> {
    let topo_key = v
        .get("topoKey")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "PrepareOffsetFace: a face entry is missing `topoKey`".to_string())?;
    Ok(crate::dto::OffsetFaceEvidenceDto {
        topo_key: topo_key.to_string(),
        picked: v.get("picked").and_then(Value::as_bool).unwrap_or(false),
        anchor: v.get("anchor").filter(|a| !a.is_null()).cloned(),
        descriptor: v.get("descriptor").filter(|d| !d.is_null()).cloned(),
    })
}

fn parse_offset_refusal(
    v: Option<&Value>,
) -> Result<Option<crate::dto::OffsetFaceRefusalDto>, String> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let code = v
        .get("code")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "PrepareOffsetFace: `refusal` is missing `code`".to_string())?;
    Ok(Some(crate::dto::OffsetFaceRefusalDto {
        code: code.to_string(),
        message: v
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        faces: str_array(v.get("faces")),
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

/// Validates the SCHEMA §7.5 echo before a resolution may be trusted (the
/// `BindElementIds` discipline, applied to the dry-run lane).
///
/// The load-bearing check is `snapshotId`: a client caches a candidate set by
/// `{revision, snapshotId, refId}` and promotes its TopoKeys ONLY against that
/// snapshot. Accepting a resolution computed on some OTHER snapshot and filing it
/// under the requested one is exactly the stale-candidate mis-bind the §7.5 rule
/// exists to prevent, so a mismatch is a protocol error, not a warning.
///
/// `refId` is checked in ORDER: the response preserves request order, and a
/// re-ordered response would silently attach one ref's evidence to another's dialog.
pub fn validate_resolve_refs_result(req: &ResolveRequest, result: &Value) -> Result<(), String> {
    let resolutions = result
        .get("resolutions")
        .and_then(Value::as_array)
        .ok_or_else(|| "ResolveRefs: missing resolutions array".to_string())?;
    if resolutions.len() != req.refs.len() {
        return Err(format!(
            "ResolveRefs: expected {} resolutions, got {}",
            req.refs.len(),
            resolutions.len()
        ));
    }
    let mut echoed_revision = None;
    for (index, (actual, expected)) in resolutions.iter().zip(&req.refs).enumerate() {
        if actual.get("refId").and_then(Value::as_str) != Some(expected.ref_id.as_str()) {
            return Err(format!(
                "ResolveRefs: mismatched refId at resolutions[{index}]"
            ));
        }
        match actual.get("snapshotId").and_then(Value::as_u64) {
            Some(echoed) if echoed == req.snapshot_id.0 => {}
            Some(echoed) => {
                return Err(format!(
                    "ResolveRefs: resolutions[{index}] echoed snapshot {echoed}, requested {}",
                    req.snapshot_id.0
                ))
            }
            None => {
                return Err(format!(
                    "ResolveRefs: resolutions[{index}] carries no snapshotId echo"
                ))
            }
        }
        let revision = actual
            .get("revision")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("ResolveRefs: resolutions[{index}] carries no revision echo"))?;
        if let Some(first) = echoed_revision {
            if revision != first {
                return Err(format!(
                    "ResolveRefs: resolutions[{index}] revision {revision} != {first}"
                ));
            }
        } else {
            echoed_revision = Some(revision);
        }
        validate_resolution_body(index, expected, actual)?;
    }
    Ok(())
}

fn validate_resolution_body(
    index: usize,
    expected: &onecad_core::regen::ResolveRef,
    actual: &Value,
) -> Result<(), String> {
    let outcome = actual
        .get("outcome")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("ResolveRefs: resolutions[{index}] missing outcome"))?;
    if !matches!(outcome, "unchanged" | "autoBind" | "needsRepair") {
        return Err(format!(
            "ResolveRefs: resolutions[{index}] unknown outcome {outcome:?}"
        ));
    }
    let echoed = actual.get("bodyId").and_then(Value::as_str);
    if let Some(primary) = &expected.element.primary {
        let wanted = body_id_wire(primary.body);
        if let Some(body) = echoed {
            if body != wanted {
                return Err(format!(
                    "ResolveRefs: resolutions[{index}] bodyId {body:?} != primary {wanted:?}"
                ));
            }
        } else if !missing_body_refusal(actual, outcome) {
            return Err(format!(
                "ResolveRefs: resolutions[{index}] omitted bodyId outside missing-body refusal"
            ));
        }
    } else if echoed.is_none() && !missing_body_refusal(actual, outcome) {
        return Err(format!(
            "ResolveRefs: resolutions[{index}] omitted bodyId outside missing-body refusal"
        ));
    }
    Ok(())
}

fn missing_body_refusal(actual: &Value, outcome: &str) -> bool {
    let repair = actual.get("needsRepair");
    outcome == "needsRepair"
        && repair.and_then(|r| r.get("reason")).and_then(Value::as_str) == Some("no-candidates")
        && repair
            .and_then(|r| r.get("candidates"))
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
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
    // SCHEMA §7.5 echo. Validated in `validate_resolve_refs_result` before any of
    // this is trusted; parsed here so the caller never has to re-derive it.
    let snapshot_id = SnapshotId(r.get("snapshotId").and_then(Value::as_u64).unwrap_or(0));
    let revision = r.get("revision").and_then(Value::as_u64).unwrap_or(0);
    let body_id = r
        .get("bodyId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .and_then(|s| parse_body_id(s).ok());
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
                // The worker emits `"anchor": {}` for a ref that carried no anchor
                // (`missing_body_resolution`, and any ladder refusal on such a ref).
                // An empty object is NOT a valid `AnchorIntent` — `worldPoint` is
                // required — so deserializing the whole `RepairItem` failed and the
                // `.ok()?` below DROPPED the resolution entirely: the response came
                // back with one refusal and the caller saw an empty vec. That is a
                // repair-panel defect as much as a re-bind one (a dialog with real
                // candidates rendering as "nothing to resolve"), and it is invisible
                // because both halves are silent. An absent anchor is `None`.
                if map.get("anchor").is_some_and(|a| {
                    a.as_object().is_some_and(serde_json::Map::is_empty) || a.is_null()
                }) {
                    map.remove("anchor");
                }
            }
            ResolveOutcome::NeedsRepair(serde_json::from_value::<RepairItem>(obj).ok()?)
        }
        _ => return None,
    };
    Some(RefResolution {
        ref_id,
        outcome,
        snapshot_id,
        revision,
        body_id,
    })
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
        Constraint::Fixed {
            point,
            point_position,
            ..
        } => {
            let mut v = json!({ "id": cid(c), "type": "Fixed", "entities": [s(point)] });
            if let Some(p) = point_roles(&[*point_position]) {
                v["positions"] = p;
            }
            v
        }
        Constraint::Midpoint {
            point,
            point_position,
            line,
            ..
        } => {
            let mut v =
                json!({ "id": cid(c), "type": "Midpoint", "entities": [s(point), s(line)] });
            if let Some(p) = point_roles(&[*point_position, CurvePosition::Arbitrary]) {
                v["positions"] = p;
            }
            v
        }
        // `positions[0]` = which point of `point`; `positions[1]` = where on the
        // CURVE it is pinned. Two different concepts sharing one array — never
        // conflate them (SCHEMA §7.3, SKETCH-V2 P3).
        Constraint::OnCurve {
            point,
            point_position,
            curve,
            position,
            ..
        } => json!({
            "id": cid(c), "type": "OnCurve",
            "entities": [s(point), s(curve)],
            "positions": [curve_position_role(*point_position), curve_position_str(*position)],
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
            entity1_position,
            entity2,
            entity2_position,
            value,
            ..
        } => {
            let mut v = json!({
                "id": cid(c), "type": "Distance",
                "entities": [s(entity1), s(entity2)], "value": value.value,
            });
            if let Some(p) = point_roles(&[*entity1_position, *entity2_position]) {
                v["positions"] = p;
            }
            v
        }
        Constraint::HorizontalDistance {
            point1,
            point1_position,
            point2,
            point2_position,
            value,
            ..
        } => {
            let mut v = json!({
                "id": cid(c), "type": "HorizontalDistance",
                "entities": [s(point1), s(point2)], "value": value.value,
            });
            if let Some(p) = point_roles(&[*point1_position, *point2_position]) {
                v["positions"] = p;
            }
            v
        }
        Constraint::VerticalDistance {
            point1,
            point1_position,
            point2,
            point2_position,
            value,
            ..
        } => {
            let mut v = json!({
                "id": cid(c), "type": "VerticalDistance",
                "entities": [s(point1), s(point2)], "value": value.value,
            });
            if let Some(p) = point_roles(&[*point1_position, *point2_position]) {
                v["positions"] = p;
            }
            v
        }
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
            point1_position,
            point2,
            point2_position,
            axis,
            ..
        } => {
            let mut v = json!({
                "id": cid(c), "type": "Symmetric", "entities": [s(point1), s(point2), s(axis)],
            });
            if let Some(p) =
                point_roles(&[*point1_position, *point2_position, CurvePosition::Arbitrary])
            {
                v["positions"] = p;
            }
            v
        }
        // SNAP P3. Both slots are POINTS, so the `positions` discipline is
        // exactly `Coincident`'s: emitted only when a side names an arc
        // endpoint, absent otherwise.
        Constraint::HorizontalPoints {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } => {
            let mut v = json!({
                "id": cid(c), "type": "HorizontalPoints", "entities": [s(point1), s(point2)],
            });
            if let Some(p) = point_roles(&[*point1_position, *point2_position]) {
                v["positions"] = p;
            }
            v
        }
        Constraint::VerticalPoints {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } => {
            let mut v = json!({
                "id": cid(c), "type": "VerticalPoints", "entities": [s(point1), s(point2)],
            });
            if let Some(p) = point_roles(&[*point1_position, *point2_position]) {
                v["positions"] = p;
            }
            v
        }
    }
}

fn cid(c: &Constraint) -> String {
    c.id().to_string()
}

/// The `positions` array for a constraint whose point slots MAY name an
/// owner+role, or `None` when every slot is `Arbitrary`.
///
/// Emitting nothing in the all-arbitrary case is what keeps every constraint
/// authored before SKETCH-V2 P3 byte-identical on the wire — the same
/// discipline `Coincident` has used since W0b.
fn point_roles(roles: &[CurvePosition]) -> Option<Value> {
    if roles.iter().all(CurvePosition::is_arbitrary) {
        return None;
    }
    Some(Value::Array(
        roles
            .iter()
            .map(|r| Value::String(curve_position_role(*r).to_string()))
            .collect(),
    ))
}

fn curve_position_str(p: CurvePosition) -> &'static str {
    match p {
        CurvePosition::Start => "Start",
        CurvePosition::End => "End",
        CurvePosition::Center => "Center",
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

fn element_kind_str(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Face => "face",
        ElementKind::Edge => "edge",
        ElementKind::Vertex => "vertex",
    }
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

/// Parses a solver `curves` map (`{entityId: {radius?, startAngle?, endAngle?}}`)
/// — SCHEMA §7.4's additive companion to `positions`, carrying what a drag changed
/// that is NOT a point coordinate (a radius drag moves no point; an arcEnd drag
/// reshapes the arc; a Tangent propagates a plain point drag into a neighbour's
/// radius).
///
/// The key is absent on a pre-SP-2 worker ⇒ empty map (optional/additive, exactly
/// like `conflicting`). Members are individually optional: an absent member is
/// UNCHANGED, never zero. Finiteness is NOT enforced here (mirroring
/// [`parse_positions`]); the typed apply step drops non-finite values.
fn parse_curves(v: Option<&Value>) -> BTreeMap<String, CurveParamsDto> {
    let Some(obj) = v.and_then(Value::as_object) else {
        return BTreeMap::new();
    };
    obj.iter()
        .filter_map(|(k, members)| {
            let m = members.as_object()?;
            Some((
                k.clone(),
                CurveParamsDto {
                    radius: m.get("radius").and_then(Value::as_f64),
                    start_angle: m.get("startAngle").and_then(Value::as_f64),
                    end_angle: m.get("endAngle").and_then(Value::as_f64),
                },
            ))
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

    /// VF-B5a: `SketchOpParams::host_face` is a CORE-ONLY dependency field. SCHEMA
    /// §7.3 states the sketch attachment never crosses the wire, and the worker
    /// stores raw sketch params verbatim, so the lowering must drop it — otherwise
    /// every plan frame for a face-hosted sketch would carry an undocumented key
    /// with no consumer, and every runtime-baseline fixture holding one would move.
    #[test]
    fn sketch_host_face_is_dropped_from_the_wire_params() {
        use onecad_core::document::record::{PlaneKind, SketchOpParams, SketchPlaneRef};
        use onecad_core::document::refs::{ElementKind, ElementRef, PrimaryRef};
        use onecad_core::ids::{ElementId, SketchId};
        use onecad_core::math::Vec3;

        let body = BodyId(Uuid::from_u128(0x77));
        let op = Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: SketchId(Uuid::from_u128(0x11)),
            plane: SketchPlaneRef {
                kind: PlaneKind::Xy,
                origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
                x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
                y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
                normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
                extra: Default::default(),
            },
            entities: vec![],
            constraints: vec![],
            host_face: Some(ElementRef {
                primary: Some(PrimaryRef {
                    body,
                    element: ElementId::new("el_top"),
                    kind: ElementKind::Face,
                    extra: Default::default(),
                }),
                intent: None,
                anchor: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        }));
        let lowered = preview_wire_op(&op, "op_1");
        assert_eq!(lowered["opType"], "Sketch");
        assert!(
            lowered["params"].get("hostFace").is_none(),
            "hostFace must not reach the worker, got {}",
            lowered["params"]
        );
        // …and the derived record inputs are likewise not echoed into the §7.3
        // `inputs[]` array for a Sketch op (the worker resolves nothing there).
        assert_eq!(lowered["inputs"], serde_json::json!([]));
    }

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
    fn body_topology_requires_both_bounded_counts() {
        let body = BodyId(Uuid::from_u128(7));
        assert_eq!(query_body_topology_args(body)["bodyId"], body_id_wire(body));
        let parsed =
            parse_body_topology("body_7".into(), &json!({ "solidCount": 1, "faceCount": 6 }))
                .expect("valid topology");
        assert_eq!((parsed.solid_count, parsed.face_count), (1, 6));
        assert!(parse_body_topology("body_7".into(), &json!({ "solidCount": 1 })).is_err());
        assert!(parse_body_topology(
            "body_7".into(),
            &json!({ "solidCount": 1, "faceCount": u64::from(u32::MAX) + 1 }),
        )
        .is_err());
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

    // ── SCHEMA §7.5 ResolveRefs echo ─────────────────────────────────────────

    fn resolve_req(snapshot: u64, ref_ids: &[&str]) -> ResolveRequest {
        use onecad_core::regen::ResolveRef;

        ResolveRequest {
            snapshot_id: SnapshotId(snapshot),
            refs: ref_ids
                .iter()
                .map(|id| ResolveRef {
                    ref_id: (*id).to_string(),
                    element: ElementRef {
                        primary: None,
                        intent: None,
                        anchor: None,
                        extra: Default::default(),
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn resolve_refs_echo_validates_snapshot_ref_id_and_arity() {
        let req = resolve_req(5012, &["op_5.input0", "op_5.input1"]);
        let good = json!({"resolutions": [
            {"refId": "op_5.input0", "snapshotId": 5012, "revision": 44,
             "bodyId": "body_3", "outcome": "unchanged", "elementId": "el_a"},
            {"refId": "op_5.input1", "snapshotId": 5012, "revision": 44,
             "outcome": "needsRepair", "needsRepair": {"refId": "op_5.input1",
             "reason": "no-candidates", "candidates": []}},
        ]});
        assert!(validate_resolve_refs_result(&req, &good).is_ok());

        // The load-bearing one: a resolution computed against ANOTHER snapshot must
        // not be filed under the requested one — that is the stale-candidate bind.
        let wrong_snapshot = json!({"resolutions": [
            {"refId": "op_5.input0", "snapshotId": 4999, "revision": 44, "outcome": "unchanged"},
            {"refId": "op_5.input1", "snapshotId": 5012, "revision": 44, "outcome": "unchanged"},
        ]});
        let err = validate_resolve_refs_result(&req, &wrong_snapshot).unwrap_err();
        assert!(err.contains("echoed snapshot 4999"), "{err}");

        // A worker that echoes nothing is refused too: silence is exactly the state
        // this echo exists to eliminate, so it cannot be the tolerated case.
        let no_echo = json!({"resolutions": [
            {"refId": "op_5.input0", "outcome": "unchanged"},
            {"refId": "op_5.input1", "outcome": "unchanged"},
        ]});
        assert!(validate_resolve_refs_result(&req, &no_echo)
            .unwrap_err()
            .contains("no snapshotId echo"));

        // Order is identity here: swapped resolutions would attach one ref's
        // evidence to the other's repair dialog.
        let swapped = json!({"resolutions": [
            {"refId": "op_5.input1", "snapshotId": 5012, "outcome": "unchanged"},
            {"refId": "op_5.input0", "snapshotId": 5012, "outcome": "unchanged"},
        ]});
        assert!(validate_resolve_refs_result(&req, &swapped)
            .unwrap_err()
            .contains("mismatched refId"));

        let short = json!({"resolutions": [
            {"refId": "op_5.input0", "snapshotId": 5012, "outcome": "unchanged"},
        ]});
        assert!(validate_resolve_refs_result(&req, &short)
            .unwrap_err()
            .contains("expected 2 resolutions"));

        assert!(validate_resolve_refs_result(&req, &json!({}))
            .unwrap_err()
            .contains("missing resolutions array"));
    }

    #[test]
    fn resolve_refs_rejects_wrong_or_unproven_body_provenance() {
        use onecad_core::document::refs::{ElementKind, PrimaryRef};
        let mut req = resolve_req(9, &["op.input0"]);
        req.refs[0].element.primary = Some(PrimaryRef {
            body: BodyId(Uuid::from_u128(3)),
            element: ElementId::new("el_a"),
            kind: ElementKind::Face,
            extra: Default::default(),
        });
        let primary_body = body_id_wire(BodyId(Uuid::from_u128(3)));
        let base = json!({"resolutions": [{
            "refId": "op.input0", "snapshotId": 9, "revision": 4,
            "bodyId": primary_body, "outcome": "unchanged", "elementId": "el_a"
        }]});
        assert!(validate_resolve_refs_result(&req, &base).is_ok());

        let mut wrong = base.clone();
        wrong["resolutions"][0]["bodyId"] = json!("body_4");
        assert!(validate_resolve_refs_result(&req, &wrong)
            .unwrap_err()
            .contains("!= primary"));

        let mut absent = base.clone();
        absent["resolutions"][0]
            .as_object_mut()
            .unwrap()
            .remove("bodyId");
        assert!(validate_resolve_refs_result(&req, &absent)
            .unwrap_err()
            .contains("omitted bodyId"));

        absent["resolutions"][0]["outcome"] = json!("needsRepair");
        absent["resolutions"][0]["needsRepair"] =
            json!({"reason": "no-candidates", "candidates": []});
        assert!(validate_resolve_refs_result(&req, &absent).is_ok());
    }

    #[test]
    fn parse_resolve_refs_carries_the_echo_onto_every_resolution() {
        let result = json!({"resolutions": [
            {"refId": "op_5.input0", "snapshotId": 5012, "revision": 44,
             "bodyId": "body_00000000-0000-0000-0000-000000000003",
             "outcome": "autoBind", "elementId": "el_a", "score": 0.94, "margin": 0.31,
             "topoKey": "f:1"},
        ]});
        let parsed = parse_resolve_refs(&result);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].snapshot_id, SnapshotId(5012));
        assert_eq!(parsed[0].revision, 44);
        assert_eq!(
            parsed[0].body_id,
            Some(BodyId(Uuid::from_u128(3))),
            "the echoed body is parsed back out of its wire form"
        );
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
            "bodyEvents": [ { "kind": "created", "bodyId": format!("body_{op}"), "health": "quarantined" } ],
            "elementMapDelta": {
                "added": [ { "elementId": "el_1", "topoKey": "f:2", "kind": "face", "bodyId": format!("body_{op}") } ],
                "removed": ["el_9"], "relabeled": []
            },
            "needsRepair": [],
            "signatures": { "geometry": "aa", "bodyLifecycle": "bb", "referencedBinding": "cc" },
            "diagnostics": [ { "severity": "warning", "code": "X", "message": "m" } ]
        });
        let step = parse_plan_step(&payload, 3).unwrap();
        assert_eq!(step.step_index, 3);
        assert!(
            matches!(step.body_events[0], BodyLifecycleEvent::Created { body } if body == BodyId(op))
        );
        assert_eq!(
            step.body_health.get(&BodyId(op)),
            Some(&BodyHealth::Quarantined)
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
    fn plan_step_requires_matching_integer_step_index() {
        for payload in [json!({}), json!({"stepIndex": "3"})] {
            assert!(parse_plan_step(&payload, 3).is_err());
        }
        assert!(parse_plan_step(&json!({"stepIndex": 3}), 2).is_err());
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
        let step = parse_plan_step(&payload, 5).unwrap();
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
    fn plan_prepared_requires_known_typed_terminal_fields() {
        let job = JobId(Uuid::from_u128(7));
        let valid = json!({
            "preparedSnapshotId": 1,
            "lastValidStep": 0,
            "stoppedReason": "completed",
            "perStepResults": [{"stepIndex": 0, "status": "ok"}],
            "historyPrefixHash": "hash"
        });
        let cases = [
            ("stoppedReason", Value::Null),
            ("stoppedReason", json!("unknown")),
            ("lastValidStep", json!("0")),
            ("perStepResults", Value::Null),
        ];
        for (field, replacement) in cases {
            let mut malformed = valid.clone();
            malformed[field] = replacement;
            assert!(parse_plan_prepared(job, &malformed).is_err(), "{field}");
        }
        let mut missing = valid;
        missing.as_object_mut().unwrap().remove("stoppedReason");
        assert!(parse_plan_prepared(job, &missing).is_err());
    }

    #[test]
    fn plan_prepared_requires_known_typed_per_step_fields() {
        let job = JobId(Uuid::from_u128(7));
        for row in [
            json!({"status": "ok"}),
            json!({"stepIndex": "0", "status": "ok"}),
            json!({"stepIndex": 0}),
            json!({"stepIndex": 0, "status": 7}),
            json!({"stepIndex": 0, "status": "unknown"}),
        ] {
            let result = json!({
                "preparedSnapshotId": 1,
                "lastValidStep": 0,
                "stoppedReason": "completed",
                "perStepResults": [row],
                "historyPrefixHash": "hash"
            });
            assert!(parse_plan_prepared(job, &result).is_err());
        }
    }

    #[test]
    fn failed_terminal_diagnostics_keep_optional_evidence_and_ignore_bad_detail() {
        let job = JobId(Uuid::from_u128(8));
        let result = json!({
            "preparedSnapshotId": 12,
            "lastValidStep": null,
            "stoppedReason": "opFailed",
            "perStepResults": [{
                "stepIndex": 0,
                "status": "opFailed",
                "message": "fillet failed",
                "diagnostics": [{
                    "severity": "error",
                    "code": "FILLET_WALKING_FAILED",
                    "message": "fillet failed",
                    "stage": "build",
                    "evidence": {"metrics": {"requestedRadius": 11.0}}
                }, {
                    "severity": "error",
                    "code": "BAD_OPTIONAL",
                    "message": "still usable",
                    "stage": 7,
                    "evidence": []
                }]
            }],
            "historyPrefixHash": "e3b0"
        });
        let prepared = parse_plan_prepared(job, &result).unwrap();
        let diagnostics = &prepared.per_step[0].diagnostics;
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].stage.as_deref(), Some("build"));
        assert!(diagnostics[0].evidence.is_some());
        assert!(diagnostics[1].stage.is_none());
        assert!(diagnostics[1].evidence.is_none());
    }

    #[test]
    fn diagnostic_bounds_drop_excess_and_malformed_optional_fields() {
        let valid = json!({"severity": "info", "code": "OK", "message": "usable"});
        let many = Value::Array(std::iter::repeat_n(valid.clone(), 65).collect());
        assert_eq!(parse_diagnostics(Some(&many)).len(), 64);

        let bounded = json!([
            {"severity": "error", "code": "x".repeat(129), "message": "ignored"},
            {
                "severity": "warning",
                "code": "KEPT",
                "message": "usable",
                "stage": "x".repeat(65),
                "evidence": {"value": "x".repeat(65_537)}
            }
        ]);
        let parsed = parse_diagnostics(Some(&bounded));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].code, "KEPT");
        assert!(parsed[0].stage.is_none());
        assert!(parsed[0].evidence.is_none());
    }

    #[test]
    fn plan_prepared_base_only_last_valid_null() {
        let job = JobId(Uuid::from_u128(1));
        let result = json!({
            "preparedSnapshotId": 1, "lastValidStep": null, "stoppedReason": "needsRepair",
            "perStepResults": [{"stepIndex": 0, "status": "needsRepair"}],
            "historyPrefixHash": "e3b0"
        });
        let p = parse_plan_prepared(job, &result).unwrap();
        assert_eq!(p.last_valid_step, None);
        assert_eq!(p.stopped_reason, StoppedReason::NeedsRepair);
    }

    // ── Inline tessellate artifacts (SCHEMA §7.2 `artifacts.tessellate`) ─────
    //
    // The contract under test is the degradation rule: these bytes are a cache
    // fill, so EVERY defect must cost at most that one body's fast path — never
    // the plan, never the other bodies.

    const SNAP: u64 = 5013;

    /// A minimal but valid MESH1 blob (64-byte header, `sectionCount = 0`), padded
    /// with `extra` trailing zero bytes so two bodies get distinguishable blobs.
    fn mesh1(extra: usize) -> Vec<u8> {
        let mut b = vec![0u8; 64 + extra];
        b[0x00..0x04].copy_from_slice(&0x4D45_5348u32.to_le_bytes()); // "MESH" (LE)
        b[0x04..0x06].copy_from_slice(&1u16.to_le_bytes()); // version
        b
    }

    /// A §7.6 mesh handle over `blob`, all integrity fields correct.
    fn handle(body: &str, section: &str, blob: &[u8]) -> Value {
        json!({
            "bodyId": body,
            "format": "MESH1",
            "bin": section,
            "lod": "coarse",
            "totalBytes": blob.len(),
            "triangleCount": 0,
            "sha256": crate::imports::sha256_hex(blob),
            "snapshotId": SNAP,
        })
    }

    /// A terminal `PlanPrepared` result carrying `handles`, plus the section table
    /// and tail that back them (blobs laid out back-to-back in handle order).
    fn artifact_fixture(blobs: &[(&str, Vec<u8>)]) -> (Value, Vec<BinSection>, Vec<u8>) {
        let mut sections = Vec::new();
        let mut tail = Vec::new();
        let mut handles = Vec::new();
        for (body, blob) in blobs {
            let name = format!("mesh:{body}");
            sections.push(BinSection {
                name: name.clone(),
                off: tail.len() as u32,
                len: blob.len() as u32,
            });
            handles.push(handle(body, &name, blob));
            tail.extend_from_slice(blob);
        }
        let result = json!({
            "planPrepared": true, "preparedSnapshotId": SNAP, "lastValidStep": 0,
            "stoppedReason": "completed", "perStepResults": [], "historyPrefixHash": "9c4d",
            "artifacts": { "tessellate": { "meshes": handles } },
        });
        (result, sections, tail)
    }

    #[test]
    fn artifact_meshes_parse_both_bodies_including_a_split_child() {
        let op = Uuid::from_u128(0x10);
        let plain = format!("body_{op}");
        let child = format!("body_{op}:0");
        let (result, sections, tail) = artifact_fixture(&[
            (plain.as_str(), mesh1(0)),
            (child.as_str(), mesh1(16)), // distinguishable length
        ]);

        let meshes = parse_plan_artifact_meshes(&result, Some(&sections), &tail);
        assert_eq!(meshes.len(), 2, "both handles ingested");
        assert_eq!(meshes[0].body, BodyId(op));
        assert_eq!(meshes[0].lod, Lod::Coarse);
        assert_eq!(meshes[0].bytes.len(), 64);
        assert_eq!(
            meshes[1].body,
            parse_body_id(&child).unwrap(),
            "split-child `body_<uuid>:<k>` maps to the derived uuid"
        );
        assert_eq!(meshes[1].bytes.len(), 80, "the right slice per section");

        // And the terminal parse carries them onto PlanPrepared.
        let p = parse_plan_prepared_with_artifacts(
            JobId(Uuid::from_u128(7)),
            &result,
            Some(&sections),
            &tail,
        )
        .unwrap();
        assert_eq!(p.artifact_meshes.len(), 2);
        assert_eq!(p.prepared_snapshot_id, SnapshotId(SNAP));
    }

    #[test]
    fn missing_artifacts_key_is_empty_not_an_error() {
        // The RETRANSMIT shape: the worker's idempotent cached re-return deliberately
        // carries NO artifacts (its bytes rode the original resp's tail only).
        let result = json!({
            "planPrepared": true, "preparedSnapshotId": SNAP, "lastValidStep": 0,
            "stoppedReason": "completed", "perStepResults": [], "historyPrefixHash": "9c4d",
        });
        assert!(parse_plan_artifact_meshes(&result, None, &[]).is_empty());
        assert!(parse_plan_artifact_meshes(&result, Some(&[]), &[]).is_empty());
        let p = parse_plan_prepared_with_artifacts(JobId(Uuid::from_u128(7)), &result, None, &[])
            .expect("a retransmit still parses as a normal prepare");
        assert!(p.artifact_meshes.is_empty());
        assert_eq!(p.prepared_snapshot_id, SnapshotId(SNAP));
    }

    #[test]
    fn sha_mismatch_skips_only_that_handle() {
        let a = format!("body_{}", Uuid::from_u128(0x10));
        let b = format!("body_{}", Uuid::from_u128(0x11));
        let (mut result, sections, tail) =
            artifact_fixture(&[(a.as_str(), mesh1(0)), (b.as_str(), mesh1(0))]);
        result["artifacts"]["tessellate"]["meshes"][0]["sha256"] =
            json!("00000000000000000000000000000000000000000000000000000000deadbeef");

        let meshes = parse_plan_artifact_meshes(&result, Some(&sections), &tail);
        assert_eq!(
            meshes.len(),
            1,
            "the corrupt handle is dropped, not the plan"
        );
        assert_eq!(meshes[0].body, BodyId(Uuid::from_u128(0x11)));
    }

    #[test]
    fn total_bytes_mismatch_skips_only_that_handle() {
        let a = format!("body_{}", Uuid::from_u128(0x10));
        let b = format!("body_{}", Uuid::from_u128(0x11));
        let (mut result, sections, tail) =
            artifact_fixture(&[(a.as_str(), mesh1(0)), (b.as_str(), mesh1(0))]);
        result["artifacts"]["tessellate"]["meshes"][0]["totalBytes"] = json!(999);

        let meshes = parse_plan_artifact_meshes(&result, Some(&sections), &tail);
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].body, BodyId(Uuid::from_u128(0x11)));
    }

    #[test]
    fn out_of_bounds_section_drops_the_artifacts_wholesale() {
        // An OOB section means the offsets cannot be trusted at all, so nothing is
        // ingested — but the PLAN still parses (the pull path serves every body).
        let a = format!("body_{}", Uuid::from_u128(0x10));
        let (result, mut sections, tail) = artifact_fixture(&[(a.as_str(), mesh1(0))]);
        sections[0].len = tail.len() as u32 + 1;

        assert!(parse_plan_artifact_meshes(&result, Some(&sections), &tail).is_empty());
        let p = parse_plan_prepared_with_artifacts(
            JobId(Uuid::from_u128(7)),
            &result,
            Some(&sections),
            &tail,
        )
        .expect("a bad section table never fails the plan");
        assert!(p.artifact_meshes.is_empty());
    }

    #[test]
    fn absent_bin_section_skips_only_that_handle() {
        let a = format!("body_{}", Uuid::from_u128(0x10));
        let b = format!("body_{}", Uuid::from_u128(0x11));
        let (mut result, sections, tail) =
            artifact_fixture(&[(a.as_str(), mesh1(0)), (b.as_str(), mesh1(0))]);
        result["artifacts"]["tessellate"]["meshes"][0]["bin"] = json!("mesh:nope");

        let meshes = parse_plan_artifact_meshes(&result, Some(&sections), &tail);
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].body, BodyId(Uuid::from_u128(0x11)));
    }

    #[test]
    fn bad_mesh1_magic_skips_only_that_handle() {
        let a = format!("body_{}", Uuid::from_u128(0x10));
        let b = format!("body_{}", Uuid::from_u128(0x11));
        let mut bad = mesh1(0);
        bad[0] = 0xFF; // not "MESH"
        let (result, sections, tail) =
            artifact_fixture(&[(a.as_str(), bad), (b.as_str(), mesh1(0))]);

        let meshes = parse_plan_artifact_meshes(&result, Some(&sections), &tail);
        assert_eq!(meshes.len(), 1, "a non-MESH1 blob never reaches the cache");
        assert_eq!(meshes[0].body, BodyId(Uuid::from_u128(0x11)));
    }

    #[test]
    fn wrong_snapshot_id_skips_that_handle() {
        // Geometry stamped with a DIFFERENT prepare would be cached under this
        // publish's generation — the one defect that would be a correctness bug.
        let a = format!("body_{}", Uuid::from_u128(0x10));
        let (mut result, sections, tail) = artifact_fixture(&[(a.as_str(), mesh1(0))]);
        result["artifacts"]["tessellate"]["meshes"][0]["snapshotId"] = json!(SNAP + 1);
        assert!(parse_plan_artifact_meshes(&result, Some(&sections), &tail).is_empty());
    }

    #[test]
    fn unparseable_body_id_skips_that_handle() {
        let a = format!("body_{}", Uuid::from_u128(0x10));
        let (mut result, sections, tail) = artifact_fixture(&[(a.as_str(), mesh1(0))]);
        result["artifacts"]["tessellate"]["meshes"][0]["bodyId"] = json!("not-a-body");
        assert!(parse_plan_artifact_meshes(&result, Some(&sections), &tail).is_empty());
    }

    #[test]
    fn ingest_cap_truncates_and_keeps_the_prefix() {
        // The production cap is 256 MiB; the rule is exercised at a scale a test can
        // allocate. Once the running total would pass it, ingestion stops and the
        // remaining bodies fall back to the pull.
        let a = format!("body_{}", Uuid::from_u128(0x10));
        let b = format!("body_{}", Uuid::from_u128(0x11));
        let c = format!("body_{}", Uuid::from_u128(0x12));
        let (result, sections, tail) = artifact_fixture(&[
            (a.as_str(), mesh1(0)),  // 64
            (b.as_str(), mesh1(0)),  // 64 ⇒ 128 total
            (c.as_str(), mesh1(16)), // 80 ⇒ would be 208 > 200
        ]);

        let meshes = parse_artifact_meshes_capped(&result, Some(&sections), &tail, 200);
        assert_eq!(meshes.len(), 2, "stopped at the cap");
        assert_eq!(meshes[1].body, BodyId(Uuid::from_u128(0x11)));
        // Uncapped, all three ride through.
        assert_eq!(
            parse_plan_artifact_meshes(&result, Some(&sections), &tail).len(),
            3
        );
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

        let e = map_error(&ErrorObject {
            code: ErrorCode::OpFailed,
            message: "top-level remains usable".into(),
            detail: Some(json!({"diagnostics": "malformed"})),
            retriable: false,
        });
        assert!(matches!(
            e,
            EngineError::OpFailed {
                diagnostics,
                ..
            } if diagnostics.is_empty()
        ));

        let e = map_error(&ErrorObject {
            code: ErrorCode::GeometryInvalid,
            message: "top-level remains usable".into(),
            detail: Some(json!({
                "diagnostics": [
                    {"severity": "future", "code": "BAD", "message": "ignored"},
                    {"severity": "error", "code": "GOOD", "message": "kept"}
                ]
            })),
            retriable: false,
        });
        assert!(matches!(
            e,
            EngineError::OpFailed {
                diagnostics,
                ..
            } if diagnostics.len() == 1 && diagnostics[0].code == "GOOD"
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
            "regionIdentityVersion": 2,
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
            entity1_position: CurvePosition::Arbitrary,
            entity2: p1,
            entity2_position: CurvePosition::Arbitrary,
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

    /// The pre-SP-2 `BeginGesture.args`, spelled out literally. If the Point branch
    /// ever drifts from this, every legacy client + the frozen `sketch_gesture`
    /// fixture drift with it.
    fn legacy_begin_gesture_json(drag_point: EntityId) -> Value {
        json!({
            "sketchId": "sk_1",
            "sketchRevision": 4,
            "gestureId": 51,
            "solverPolicyHash": "",
            "drag": { "pointId": drag_point.to_string() },
            "pointId": drag_point.to_string(),
        })
    }

    #[test]
    fn begin_gesture_point_kind_is_byte_identical_to_the_legacy_form() {
        let p = eid(0x10);
        let args = begin_gesture_args("sk_1", 4, 51, p, &GestureTarget::point(p), "");
        let legacy = legacy_begin_gesture_json(p);
        assert_eq!(args, legacy, "the point branch emits the legacy JSON");
        assert_eq!(
            serde_json::to_string(&args).unwrap(),
            serde_json::to_string(&legacy).unwrap(),
            "byte-identical serialization (no kind key, both legacy pointId slots)"
        );
        assert!(args["drag"].get("kind").is_none(), "no kind key on a point");
        assert!(args["drag"].get("entity").is_none());

        // `pointId` WINS on the point path (§7.4): a target naming a DIFFERENT
        // entity does not change the emitted request.
        let other = GestureTarget {
            entity: eid(0x99),
            role: Some("start"),
            grab: Some([1.0, 2.0]),
            ..GestureTarget::point(p)
        };
        assert_eq!(
            begin_gesture_args("sk_1", 4, 51, p, &other, ""),
            legacy,
            "a point-kind target never adds keys — pointId wins"
        );
    }

    #[test]
    fn begin_gesture_non_point_kinds_drop_the_legacy_keys() {
        let p = eid(0x10);
        let arc = eid(0x41);
        // arcEnd: kind + entity + role, no grab (§7.4 ignores it for this kind).
        let args = begin_gesture_args(
            "sk_1",
            4,
            51,
            p,
            &GestureTarget {
                kind: DragKind::ArcEnd,
                entity: arc,
                role: Some("end"),
                grab: None,
            },
            "",
        );
        assert_eq!(
            args["drag"],
            json!({ "kind": "arcEnd", "entity": arc.to_string(), "role": "end" })
        );
        assert!(
            args.get("pointId").is_none() && args["drag"].get("pointId").is_none(),
            "a non-point kind drops BOTH legacy pointId slots"
        );
        assert_eq!(args["sketchRevision"], json!(4));
        assert_eq!(args["gestureId"], json!(51));

        // radius: grab rides, role is absent.
        let args = begin_gesture_args(
            "sk_1",
            4,
            51,
            p,
            &GestureTarget {
                kind: DragKind::Radius,
                entity: arc,
                role: None,
                grab: Some([31.2, 4.0]),
            },
            "",
        );
        assert_eq!(
            args["drag"],
            json!({ "kind": "radius", "entity": arc.to_string(), "grab": [31.2, 4.0] })
        );

        // entityBody: grab RECOMMENDED, still optional.
        let args = begin_gesture_args(
            "sk_1",
            4,
            51,
            p,
            &GestureTarget {
                kind: DragKind::EntityBody,
                entity: arc,
                role: None,
                grab: None,
            },
            "",
        );
        assert_eq!(
            args["drag"],
            json!({ "kind": "entityBody", "entity": arc.to_string() })
        );
    }

    #[test]
    fn drag_kind_tokens_round_trip_and_reject_the_unknown() {
        for kind in [
            DragKind::Point,
            DragKind::ArcEnd,
            DragKind::Radius,
            DragKind::EntityBody,
        ] {
            assert_eq!(DragKind::parse(kind.wire()), Some(kind));
        }
        assert_eq!(DragKind::Point.wire(), "point");
        assert_eq!(DragKind::ArcEnd.wire(), "arcEnd");
        assert_eq!(DragKind::Radius.wire(), "radius");
        assert_eq!(DragKind::EntityBody.wire(), "entityBody");
        // STRICT: unknown + wrong case are rejected, never degraded to `point`.
        for bad in ["", "Point", "ARCEND", "arcend", "entitybody", "scale"] {
            assert_eq!(DragKind::parse(bad), None, "{bad:?} must not parse");
        }
    }

    #[test]
    fn parse_curves_reads_changed_members_and_tolerates_absence() {
        let result = json!({
            "gestureId": 51, "seq": 3, "status": "success", "dof": 1,
            "curves": {
                "e7": { "radius": 12.5 },
                "e9": { "startAngle": 0.35, "endAngle": 1.9634 },
                "e11": { "radius": 4.0, "startAngle": -0.5, "endAngle": 2.0 },
                "e13": {},
                "e15": 7,
            }
        });
        let d = parse_solve_drag(&result);
        assert_eq!(
            d.curves["e7"],
            CurveParamsDto {
                radius: Some(12.5),
                ..Default::default()
            },
            "an absent member stays None (UNCHANGED, not zero)"
        );
        assert_eq!(
            d.curves["e9"],
            CurveParamsDto {
                radius: None,
                start_angle: Some(0.35),
                end_angle: Some(1.9634),
            }
        );
        assert_eq!(
            d.curves["e11"],
            CurveParamsDto {
                radius: Some(4.0),
                start_angle: Some(-0.5),
                end_angle: Some(2.0),
            }
        );
        assert_eq!(d.curves["e13"], CurveParamsDto::default());
        assert!(
            !d.curves.contains_key("e15"),
            "a non-object entry is skipped"
        );

        // Absent `curves` ⇒ empty, on BOTH parsers (pre-SP-2 worker).
        let legacy = json!({ "gestureId": 51, "seq": 3, "status": "success", "dof": 1 });
        assert!(parse_solve_drag(&legacy).curves.is_empty());
        let end = json!({ "gestureId": 51, "status": "success", "dof": 0, "sketchRevision": 5 });
        assert!(parse_sketch_upsert("sk_1", &end).solved_curves.is_empty());
        // EndGesture carries it through the SketchUpsert parser.
        let end = json!({ "gestureId": 51, "status": "success", "dof": 0, "sketchRevision": 5,
            "positions": {}, "curves": { "e7": { "radius": 8.25 } } });
        assert_eq!(
            parse_sketch_upsert("sk_1", &end).solved_curves["e7"].radius,
            Some(8.25)
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
            "regionIdentityVersion": 2,
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
        assert_eq!(regions.region_identity_version, 2);
        let triangles = regions.regions[0].preview_triangles.as_ref().unwrap();
        assert_eq!(triangles.holes_subtracted, 1);
        assert_eq!(triangles.indices, vec![0, 1, 2]);

        let mut v3_result = result;
        v3_result["regionIdentityVersion"] = json!(3);
        let v3 = parse_sketch_regions("sk_1", &v3_result, &sections, &tail).unwrap();
        assert_eq!(v3.region_identity_version, 3);
    }

    #[test]
    fn sketch_regions_require_response_and_region_structure() {
        let (result, sections, tail) = valid_region_response();
        for field in [
            "sketchId",
            "sketchRevision",
            "regionIdentityVersion",
            "regions",
        ] {
            let mut malformed = result.clone();
            malformed.as_object_mut().unwrap().remove(field);
            assert_region_parse_error(&malformed, &sections, &tail);
        }
        let mut wrong_sketch = result.clone();
        wrong_sketch["sketchId"] = json!("sk_other");
        assert_region_parse_error(&wrong_sketch, &sections, &tail);
        let mut legacy_version = result.clone();
        legacy_version["regionIdentityVersion"] = json!(1);
        assert_region_parse_error(&legacy_version, &sections, &tail);

        for field in ["regionId", "outerLoop", "holes", "previewTriangles"] {
            let mut malformed = result.clone();
            malformed["regions"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert_region_parse_error(&malformed, &sections, &tail);
        }
        let empty = json!({
            "sketchId": "sk_1",
            "sketchRevision": 1,
            "regionIdentityVersion": 2,
            "regions": []
        });
        assert!(parse_sketch_regions("sk_1", &empty, &[], &[])
            .unwrap()
            .regions
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

    /// SCHEMA §7.7 wire pin: the four `RestoreCheckpoint` arg keys, and specifically
    /// that `workerEpoch` comes from the REQUEST (the plan's fenced token), not from a
    /// live engine read — two verbs of one plan reading two epochs is the VF-B4 desync.
    #[test]
    fn restore_checkpoint_args_pin_the_wire_keys() {
        let req = RestoreRequest {
            checkpoint: onecad_core::regen::CheckpointRef {
                step_index: 4,
                checkpoint_id: onecad_core::regen::CheckpointId::new("ckpt_9"),
            },
            expected_history_prefix_hash: onecad_core::regen::HistoryPrefixHash::new("9c4d"),
            worker_epoch: WorkerEpoch(7),
            artifacts: None,
        };
        let args = restore_checkpoint_args(&req);
        assert_eq!(args["checkpointId"], "ckpt_9");
        assert_eq!(args["stepIndex"], 4);
        assert_eq!(args["expectedHistoryPrefixHash"], "9c4d");
        assert_eq!(args["workerEpoch"], 7);
        assert_eq!(
            args.as_object().unwrap().len(),
            4,
            "exactly the four §7.7 keys"
        );
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
    fn bind_element_ids_args_and_exact_echo() {
        use onecad_core::regen::ElementBinding;

        let body = BodyId(Uuid::from_u128(3));
        let req = BindElementIdsRequest {
            snapshot_id: SnapshotId(5012),
            bindings: vec![ElementBinding {
                element_id: ElementId::new("el_face"),
                topo_key: TopoKey::new("f:2"),
                body,
                kind: ElementKind::Face,
                anchor: None,
            }],
        };
        let args = bind_element_ids_args(&req);
        assert_eq!(args["snapshotId"], 5012);
        assert_eq!(args["bindings"][0]["bodyId"], body_id_wire(body));
        assert_eq!(args["bindings"][0]["topoKey"], "f:2");
        assert_eq!(args["bindings"][0]["elementId"], "el_face");
        assert_eq!(args["bindings"][0]["kind"], "face");

        let ok = json!({ "bound": [{
            "bodyId": body_id_wire(body),
            "topoKey": "f:2",
            "elementId": "el_face",
            "kind": "face"
        }] });
        assert!(validate_bind_element_ids_result(&req, &ok).is_ok());
        let wrong = json!({ "bound": [{
            "bodyId": body_id_wire(body),
            "topoKey": "f:3",
            "elementId": "el_face",
            "kind": "face"
        }] });
        assert!(validate_bind_element_ids_result(&req, &wrong).is_err());
        assert!(validate_bind_element_ids_result(&req, &json!({ "bound": [] })).is_err());
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
            tangent_closure_version: None,
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

    /// SCHEMA §7.3 (2026-08-06) `op.offsetFace`: the params lower every scalar +
    /// the bare `faceIds`, `targetBodyId` crosses in the worker's `body_<uuid>`
    /// form, and `inputs[]` carries the typed refs in the NORMATIVE slot order
    /// (operative faces in stored order, `Total` opposite LAST).
    #[test]
    fn wire_op_offset_face_lowers_params_and_the_normative_slot_order() {
        use onecad_core::document::record::{OffsetDistanceType, OffsetFaceParams};

        let body = BodyId(Uuid::from_u128(0x68));
        let face = |el: &str| ElementRef {
            primary: Some(PrimaryRef {
                body,
                element: ElementId::new(el),
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            intent: None,
            anchor: Some(onecad_core::document::refs::AnchorIntent {
                world_point: Vec3::new_unchecked(1.0, 2.0, 3.0),
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        };
        let lower = |p: OffsetFaceParams| {
            let op = Operation::Known(KnownOperation::OffsetFace(p));
            let inputs = op.derive_inputs();
            wire_op(&planned(op, inputs))
        };

        let multi = lower(OffsetFaceParams {
            face_ids: vec![ElementId::new("el_f1"), ElementId::new("el_f2")],
            primary_face_ids: vec![ElementId::new("el_f1")],
            faces: vec![face("el_f1"), face("el_f2")],
            distance: Scalar::new(2.5),
            distance_type: OffsetDistanceType::Offset,
            chain_tangent_faces: true,
            opposite_face_id: None,
            opposite_face: None,
            target_body: body,
            result_policy_version: Some(2),
            extra: Default::default(),
        });
        assert_eq!(multi["opType"], json!("OffsetFace"));
        assert_eq!(multi["params"]["faceIds"], json!(["el_f1", "el_f2"]));
        assert_eq!(multi["params"]["primaryFaceIds"], json!(["el_f1"]));
        assert_eq!(multi["params"]["resultPolicyVersion"], json!(2));
        assert_eq!(multi["params"]["distance"], json!({ "value": 2.5 }));
        assert_eq!(multi["params"]["distanceType"], json!("Offset"));
        assert_eq!(multi["params"]["chainTangentFaces"], json!(true));
        assert_eq!(
            multi["params"]["targetBodyId"],
            json!(body_id_wire(body)),
            "the worker keys its BodyStore by body_<uuid>, not a bare uuid"
        );
        // Skip-none: an `Offset` push-pull emits no opposite-face keys at all.
        assert!(multi["params"].get("oppositeFaceId").is_none());
        assert!(multi["params"].get("oppositeFace").is_none());
        // Slot table: one typed ref per operative face, in stored order.
        let slots = multi["inputs"].as_array().expect("inputs[]");
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0]["primary"]["elementId"], json!("el_f1"));
        assert_eq!(slots[1]["primary"]["elementId"], json!("el_f2"));
        assert_eq!(
            slots[0]["primary"]["bodyId"],
            json!(body_id_wire(body)),
            "a nested primary.bodyId is rewritten to wire form too"
        );

        let total = lower(OffsetFaceParams {
            face_ids: vec![ElementId::new("el_top")],
            primary_face_ids: vec![ElementId::new("el_top")],
            faces: vec![face("el_top")],
            distance: Scalar::new(12.0),
            distance_type: OffsetDistanceType::Total,
            chain_tangent_faces: false,
            opposite_face_id: Some(ElementId::new("el_bottom")),
            opposite_face: Some(face("el_bottom")),
            target_body: body,
            result_policy_version: Some(2),
            extra: Default::default(),
        });
        assert_eq!(total["params"]["distanceType"], json!("Total"));
        assert_eq!(total["params"]["oppositeFaceId"], json!("el_bottom"));
        let slots = total["inputs"].as_array().expect("inputs[]");
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0]["primary"]["elementId"], json!("el_top"));
        assert_eq!(
            slots[1]["primary"]["elementId"],
            json!("el_bottom"),
            "the Total opposite face is the LAST slot"
        );
    }

    /// SCHEMA §7.3 (2026-08-03): a two-distance chamfer's second leg reaches the
    /// worker as `params.distance2`, and an equal-leg one emits NO such key — the
    /// wire form of every existing chamfer is byte-identical.
    #[test]
    fn wire_op_chamfer_carries_distance2_only_when_set() {
        use onecad_core::document::record::ChamferParams;

        let body = BodyId(Uuid::from_u128(0x67));
        let chamfer = |d2: Option<f64>| {
            let op = Operation::Known(KnownOperation::Chamfer(ChamferParams {
                radius: Scalar::new(1.0),
                distance2: d2.map(Scalar::new),
                edge_ids: vec![ElementId::new("e:14")],
                edges: vec![],
                chain_tangent_edges: true,
                tangent_closure_version: None,
                extra: Default::default(),
            }));
            let mut inputs = OperationInputs::default();
            inputs.bodies.push(body);
            inputs.elements.push(ElementId::new("e:14"));
            wire_op(&planned(op, inputs))
        };

        let equal = chamfer(None);
        assert!(
            equal["params"].get("distance2").is_none(),
            "equal-leg chamfer emits no distance2: {}",
            equal["params"]
        );

        let asym = chamfer(Some(2.5));
        assert_eq!(asym["params"]["distance2"], json!({ "value": 2.5 }));
        assert_eq!(asym["params"]["radius"], json!({ "value": 1.0 }));
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
            region_identity_version: Some(2),
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
        assert_eq!(w["params"]["regionIdentityVersion"], json!(2));
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
        assert_eq!(w["params"]["regionIdentityVersion"], json!(2));
        assert!(w["params"].get("profile").is_none());

        // An EMPTY regionId is NOT forwarded (keeps the worker's first-region fallback).
        let ex_empty = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch: sid,
                region: RegionId::new(""),
                region_identity_version: None,
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
            region_identity_version: Some(2),
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
        let face_ref = |body: BodyId, element: &str| ElementRef {
            primary: Some(PrimaryRef {
                body,
                element: ElementId::new(element),
                kind: ElementKind::Face,
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
                        edge_ref: None,
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
                    tangent_closure_version: None,
                    extra: Default::default(),
                })),
            ),
            (
                "Chamfer",
                Operation::Known(KnownOperation::Chamfer(ChamferParams {
                    radius: Scalar::new(1.0),
                    distance2: None,
                    edge_ids: vec![ElementId::new("e:16")],
                    edges: vec![edge_ref(edge_body, "e:16")],
                    chain_tangent_edges: true,
                    tangent_closure_version: None,
                    extra: Default::default(),
                })),
            ),
            (
                "Shell",
                Operation::Known(KnownOperation::Shell(ShellParams {
                    thickness: Scalar::new(1.5),
                    open_faces: vec![ElementId::new("el_f1")],
                    faces: vec![face_ref(target, "el_f1")],
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
        assert_eq!(previewed["params"]["regionIdentityVersion"], json!(2));
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
                edge_ref: None,
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

    #[test]
    fn wire_op_typed_revolve_axis_emits_one_repairable_input() {
        let body = BodyId(Uuid::from_u128(0x91));
        let typed = ElementRef {
            primary: Some(PrimaryRef {
                body,
                element: ElementId::new("el_axis"),
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: Some(onecad_core::document::refs::IntentQuery {
                version: 1,
                kind: ElementKind::Edge,
                descriptor: json!({ "curveType": 0 }),
                extra: Default::default(),
            }),
            anchor: None,
            extra: Default::default(),
        };
        let op = Operation::Known(KnownOperation::Revolve(RevolveParams {
            profile: None,
            angle_deg: Scalar::new(90.0),
            axis: Some(AxisRef::Element {
                body,
                edge: ElementId::new("e:stale-legacy"),
                edge_ref: Some(typed),
                extra: Default::default(),
            }),
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            extra: Default::default(),
        }));
        let wire = wire_op(&planned(op.clone(), op.derive_inputs()));
        assert_eq!(wire["params"]["axis"]["edgeId"], json!("e:stale-legacy"));
        assert_eq!(wire["inputs"].as_array().map(Vec::len), Some(1));
        assert_eq!(wire["inputs"][0]["primary"]["elementId"], json!("el_axis"));
        assert_eq!(wire["inputs"][0], wire["params"]["axis"]["edgeRef"]);
    }

    // ── HISTORY-HARDEN H5 — op-set agreement ────────────────────────────────

    /// `KnownOperation::element_refs_mut` (where the single writer stamps
    /// `intent.descriptor`) and [`wire_op_inputs`] (what the worker's ladder
    /// actually receives) must cover the SAME op set with the SAME slot count.
    ///
    /// A ref slot present in the wire but missing from `element_refs_mut` would ship
    /// `intent: null` forever — the H5 defect, reintroduced silently for one op. The
    /// reverse would stamp evidence into a params field the worker never reads,
    /// churning the golden history-prefix hash for nothing.
    ///
    /// Every op the enum knows is listed, each in its MOST ref-bearing form
    /// (`ToFace` in both directions, a typed edge ref beside the bare `edgeIds`
    /// fallback, a `hostFace`, a hole face). The fixtures are JSON so the required
    /// params keys are the wire's own; the discriminator for "typed semantic ref" on
    /// the wire side is the presence of `anchor` — only `element_ref_wire` emits it,
    /// while legacy element-only fallbacks and `body_input_ref` render `primary` alone.
    #[test]
    fn element_refs_mut_covers_exactly_the_wire_typed_ref_slots() {
        let b = "00000000-0000-0000-0000-0000000000b0";
        let sk = "00000000-0000-0000-0000-000000000011";
        let edge_ref = json!({
            "primary": { "bodyId": b, "elementId": "el_1", "kind": "edge" },
            "anchor": { "worldPoint": [1.0, 2.0, 3.0] }
        });
        let face_ref = json!({
            "primary": { "bodyId": b, "elementId": "el_2", "kind": "face" },
            "anchor": { "worldPoint": [1.0, 2.0, 3.0] }
        });

        // (opType, params, expected element_refs_mut slots, expected wire inputs[]
        // typed-ref slots). The two counts are the SAME for every op except
        // PlaceComponent (see its own case below) — kept as two columns rather
        // than one shared `expected` so that deliberate exception is a data
        // difference, not a special-cased assertion.
        let cases: Vec<(&str, Value, usize, usize)> = vec![
            // hostFace is core-only (`strip_sketch_host_face`) ⇒ no slot either side.
            (
                "Sketch",
                json!({ "sketchId": sk, "plane": { "kind": "XY", "origin": [0,0,0], "xAxis": [0,1,0], "yAxis": [-1,0,0], "normal": [0,0,1] }, "hostFace": face_ref }),
                0,
                0,
            ),
            (
                "Extrude",
                json!({ "distance": 5.0, "draftAngleDeg": 0.0, "distance2": 0.0, "extrudeMode": "ToFace", "targetFace": face_ref, "twoDirections": true, "extrudeMode2": "ToFace", "targetFace2": face_ref, "booleanMode": "NewBody" }),
                2,
                2,
            ),
            // A typed Revolve edge axis is a semantic `inputs[0]` companion.
            (
                "Revolve",
                json!({ "angleDeg": 90.0, "booleanMode": "NewBody", "axis": { "kind": "edge", "bodyId": b, "edgeId": "e:2", "edgeRef": edge_ref } }),
                1,
                1,
            ),
            (
                "Fillet",
                json!({ "radius": 2.0, "edgeIds": ["el_1"], "edges": [edge_ref] }),
                1,
                1,
            ),
            (
                "Chamfer",
                json!({ "radius": 1.0, "edgeIds": ["el_1"], "edges": [edge_ref] }),
                1,
                1,
            ),
            (
                "Shell",
                json!({ "thickness": 1.5, "openFaces": ["el_2"], "faces": [face_ref], "targetBodyId": b }),
                1,
                1,
            ),
            (
                "Boolean",
                json!({ "operation": "Cut", "targetBodyId": b, "toolBodyId": b }),
                0,
                0,
            ),
            (
                "LinearPattern",
                json!({ "direction": [1,0,0], "count": 2, "spacing": 5.0, "sourceBodyId": b }),
                0,
                0,
            ),
            (
                "CircularPattern",
                json!({ "axisOrigin": [0,0,0], "axisDirection": [0,0,1], "count": 3, "angleDeg": 120.0, "sourceBodyId": b }),
                0,
                0,
            ),
            (
                "Loft",
                json!({ "booleanMode": "NewBody", "profiles": [] }),
                0,
                0,
            ),
            ("Sweep", json!({ "booleanMode": "NewBody" }), 0, 0),
            (
                "MirrorBody",
                json!({ "planePoint": [0,0,0], "planeNormal": [0,0,1], "sourceBodyId": b }),
                0,
                0,
            ),
            (
                "ImportStep",
                json!({ "sourceSha256": "aa", "sourceCodec": "step", "sourceName": "x.step" }),
                0,
                0,
            ),
            (
                "TransformBody",
                json!({ "translate": [1,0,0], "targets": [b] }),
                0,
                0,
            ),
            (
                "Hole",
                json!({ "targetBodyId": b, "face": face_ref, "point": [0,0,0], "axis": [0,0,-1], "depth": null, "holeType": "simple", "diameter": 5.0 }),
                1,
                1,
            ),
            // OffsetFace: one stampable slot per operative face, plus the `Total`
            // opposite face — every ref is typed, so every slot takes evidence.
            (
                "OffsetFace",
                json!({ "targetBodyId": b, "faceIds": ["el_2"], "faces": [face_ref], "distance": 2.5, "distanceType": "Total", "chainTangentFaces": false, "oppositeFaceId": "el_2", "oppositeFace": face_ref }),
                2,
                2,
            ),
            // PlaceComponent: `element_refs_mut` still exposes the mate target (the
            // manual-repair-rebind UI surface, unaffected) but WIRE inputs[] is
            // ALWAYS empty (Component Library P3 WP-3.1) — the worker's generic
            // `resolve_input_refs` pre-flight treats an unresolved `inputs[]` entry
            // as blocking, which is wrong for a mate (spec §5.5: "never drop it,
            // never silently move it"). `ComponentOp.cpp::resolve_mate_reseat` now
            // owns mate resolution entirely, in-process, non-blocking. See
            // `wire_op_inputs`'s own `PlaceComponent` arm comment.
            (
                "PlaceComponent",
                json!({
                    "componentId": "onecad.std.iso4762",
                    "componentVersion": "1.0.0",
                    "componentRevision": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                    "source": { "kind": "generator", "generatorId": "iso4762", "generatorVersion": 1 },
                    "mate": { "selfAttachment": "shank_axis", "target": face_ref, "kind": "concentric", "flipped": false },
                    "placement": { "translate": [0.0, 0.0, 0.0] }
                }),
                1,
                0,
            ),
            // DetachComponent: no mate, no identity — no typed slots at all.
            (
                "DetachComponent",
                json!({
                    "source": { "kind": "generator", "generatorId": "iso4762", "generatorVersion": 1 },
                    "placement": { "translate": [0.0, 0.0, 0.0] }
                }),
                0,
                0,
            ),
        ];

        for (op_type, params, expected_element_refs, expected_wire) in &cases {
            let mut known: KnownOperation =
                serde_json::from_value(json!({ "opType": op_type, "params": params }))
                    .unwrap_or_else(|e| panic!("{op_type} fixture deserializes: {e}"));
            assert_eq!(
                known.element_refs_mut().len(),
                *expected_element_refs,
                "{op_type}: element_refs_mut slot count"
            );
            let operation = Operation::Known(known);
            let inputs = operation.derive_inputs();
            let wired = wire_op_inputs(&operation, &inputs);
            let typed = wired
                .as_array()
                .expect("inputs[] is an array")
                .iter()
                .filter(|r| r.get("anchor").is_some())
                .count();
            assert_eq!(
                typed, *expected_wire,
                "{op_type}: wire inputs[] typed-ref count — got {wired}"
            );
        }

        // The enum is covered exhaustively: a new variant fails to compile below
        // until it is given a decision in `cases` above.
        fn _covered(k: &KnownOperation) {
            match k {
                KnownOperation::Sketch(_)
                | KnownOperation::Extrude(_)
                | KnownOperation::Revolve(_)
                | KnownOperation::Fillet(_)
                | KnownOperation::Chamfer(_)
                | KnownOperation::Shell(_)
                | KnownOperation::Boolean(_)
                | KnownOperation::LinearPattern(_)
                | KnownOperation::CircularPattern(_)
                | KnownOperation::Loft(_)
                | KnownOperation::Sweep(_)
                | KnownOperation::MirrorBody(_)
                | KnownOperation::ImportStep(_)
                | KnownOperation::TransformBody(_)
                | KnownOperation::Hole(_)
                | KnownOperation::Gear(_)
                | KnownOperation::OffsetFace(_)
                | KnownOperation::PlaceComponent(_)
                | KnownOperation::DetachComponent(_) => {}
            }
        }
        assert_eq!(cases.len(), 18, "one fixture per KnownOperation variant");
    }

    // ── HISTORY-HARDEN H9 — the repair SLOT TABLE ───────────────────────────

    /// H9: an EMPTY `anchor` object from the worker is read as "no anchor", not
    /// as a parse failure.
    ///
    /// `PlanExecutor.cpp` renders `{"anchor": {}}` for a ref authored WITHOUT one
    /// (`ref.anchor_json.is_null() ? json::object() : …`), while
    /// `AnchorIntent::world_point` is required. The strict read turned "this
    /// reference broke and carries no evidence" — the most ordinary unrepairable
    /// case — into a fatal `Protocol` error that tore the worker down, so the item
    /// never reached the repair panel at all. Structural fields stay strict; only
    /// this optional evidence field is tolerant.
    #[test]
    fn an_empty_anchor_object_parses_as_no_anchor_instead_of_failing() {
        let items = parse_needs_repair(
            Some(&json!([{
                "refId": "op_5.input1",
                "ladderFailed": "descriptor",
                "reason": "no-candidates",
                "anchor": {}
            }])),
            4,
        )
        .expect("an anchorless needsRepair item must PARSE, not tear down the worker");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].step_index, 4, "the enclosing step is injected");
        assert_eq!(items[0].ref_id, "op_5.input1");
        assert!(items[0].anchor.is_none(), "an empty anchor is no anchor");

        // `null` is treated the same way…
        let items = parse_needs_repair(
            Some(&json!([{ "refId": "r", "ladderFailed": "descriptor", "reason": "ambiguous", "anchor": null }])),
            0,
        )
        .expect("a null anchor parses");
        assert!(items[0].anchor.is_none());

        // …and a REAL anchor still rides through untouched.
        let items = parse_needs_repair(
            Some(&json!([{
                "refId": "r",
                "ladderFailed": "descriptor",
                "reason": "ambiguous",
                "anchor": { "worldPoint": [1.0, 2.0, 3.0], "surfaceUv": [0.25, 0.75] }
            }])),
            0,
        )
        .expect("a populated anchor parses");
        let anchor = items[0].anchor.as_ref().expect("anchor survives");
        assert_eq!(anchor.world_point.x, 1.0);
        assert!(anchor.surface_uv.is_some());

        // A malformed STRUCTURAL field is still a hard error (no blanket leniency).
        assert!(parse_needs_repair(
            Some(&json!([{ "refId": "r", "ladderFailed": "not-a-level", "reason": "ambiguous" }])),
            0
        )
        .is_err());
    }

    /// **The slot table.** [`wire_op_inputs`] decides, per `opType`, what the
    /// `k`-th entry of `inputs[]` IS — and therefore what a repair `refId`
    /// (`"<recordId>.input<k>"`, SCHEMA §9) names. The frontend's
    /// `inputPathFor(opType, slot, params)` (`src/ipc/tauriCommandMap.ts`) must
    /// map the SAME `(opType, k)` to the `InputPath` that writes that slot, and
    /// its vitest twin (`tauriCommandMap.test.ts`,
    /// "inputPathFor mirrors the Rust wire_op_inputs slot table") asserts the
    /// mirror case by case against this list.
    ///
    /// **A divergence between the two is a SILENT MIS-REPAIR**: the panel would
    /// send a well-formed `EditOperationInput` that overwrites a DIFFERENT input
    /// than the one the user clicked — e.g. before H9 every op's rebind was sent
    /// as `FilletEdges{k}`, which the core rejected for Hole/Shell/Extrude and
    /// would have silently rewritten edge `k` of any fillet that happened to
    /// match. Change one side, change the other, in the same commit.
    ///
    /// Each slot is pinned as its `primary.kind`: `"body"` (a whole-body ref —
    /// NOT addressable from the repair panel, whose candidates are element
    /// `TopoKey`s) or `"face"`/`"edge"` (a topological element slot).
    #[test]
    fn wire_op_inputs_slot_order_is_the_repair_slot_table() {
        let b = "00000000-0000-0000-0000-0000000000b0";
        let b2 = "00000000-0000-0000-0000-0000000000b1";
        let face_ref = json!({
            "primary": { "bodyId": b, "elementId": "el_2", "kind": "face" },
            "anchor": { "worldPoint": [1.0, 2.0, 3.0] }
        });
        let edge_ref = |el: &str| {
            json!({
                "primary": { "bodyId": b, "elementId": el, "kind": "edge" },
                "anchor": { "worldPoint": [1.0, 2.0, 3.0] }
            })
        };

        // (case name, opType, params, ordered slot kinds).
        let cases: Vec<(&str, &str, Value, Vec<&str>)> = vec![
            (
                "fillet: one slot per edge, in `edges` order",
                "Fillet",
                json!({ "radius": 2.0, "edgeIds": ["el_1", "el_3"], "edges": [edge_ref("el_1"), edge_ref("el_3")] }),
                vec!["edge", "edge"],
            ),
            (
                "chamfer: same table as fillet",
                "Chamfer",
                json!({ "radius": 1.0, "edgeIds": ["el_1"], "edges": [edge_ref("el_1")] }),
                vec!["edge"],
            ),
            (
                "shell: one slot per open face, in `openFaces` order",
                "Shell",
                json!({ "thickness": 1.5, "openFaces": ["el_2", "el_4"], "targetBodyId": b }),
                vec!["face", "face"],
            ),
            (
                "hole: [host body, host face] — slot 0 is NOT element-addressable",
                "Hole",
                json!({ "targetBodyId": b, "face": face_ref, "point": [0,0,0], "depth": null, "holeType": "simple", "diameter": 5.0 }),
                vec!["body", "face"],
            ),
            (
                "boolean: [target, tool] — both whole bodies",
                "Boolean",
                json!({ "operation": "Cut", "targetBodyId": b, "toolBodyId": b2 }),
                vec!["body", "body"],
            ),
            (
                "extrude Blind: no ToFace target ⇒ NO slots at all",
                "Extrude",
                json!({ "distance": 5.0, "draftAngleDeg": 0.0, "distance2": 0.0, "extrudeMode": "Blind", "twoDirections": false, "extrudeMode2": "Blind", "booleanMode": "NewBody" }),
                vec![],
            ),
            (
                "extrude ToFace, one direction: slot 0 = targetFace",
                "Extrude",
                json!({ "distance": 5.0, "draftAngleDeg": 0.0, "distance2": 0.0, "extrudeMode": "ToFace", "targetFace": face_ref, "twoDirections": false, "extrudeMode2": "Blind", "booleanMode": "NewBody" }),
                vec!["face"],
            ),
            (
                "extrude ToFace both directions: slot 0 = targetFace, slot 1 = targetFace2",
                "Extrude",
                json!({ "distance": 5.0, "draftAngleDeg": 0.0, "distance2": 0.0, "extrudeMode": "ToFace", "targetFace": face_ref, "twoDirections": true, "extrudeMode2": "ToFace", "targetFace2": face_ref, "booleanMode": "NewBody" }),
                vec!["face", "face"],
            ),
            (
                // THE trap the frontend must mirror: direction 1 is Blind, so
                // `targetFace2` COLLAPSES to slot 0 — a mapper that assumed
                // "slot 0 ⇒ second:false" would rebind the wrong face.
                "extrude Blind + second direction ToFace: slot 0 = targetFace2",
                "Extrude",
                json!({ "distance": 5.0, "draftAngleDeg": 0.0, "distance2": 0.0, "extrudeMode": "Blind", "targetFace": face_ref, "twoDirections": true, "extrudeMode2": "ToFace", "targetFace2": face_ref, "booleanMode": "NewBody" }),
                vec!["face"],
            ),
            (
                "extrude ToFace + twoDirections but mode2 Blind: only slot 0",
                "Extrude",
                json!({ "distance": 5.0, "draftAngleDeg": 0.0, "distance2": 0.0, "extrudeMode": "ToFace", "targetFace": face_ref, "twoDirections": true, "extrudeMode2": "Blind", "targetFace2": face_ref, "booleanMode": "NewBody" }),
                vec!["face"],
            ),
            (
                "revolve typed edge axis: edgeRef is inputs[0]",
                "Revolve",
                json!({ "angleDeg": 90.0, "booleanMode": "NewBody", "axis": { "kind": "edge", "bodyId": b, "edgeId": "e:2", "edgeRef": edge_ref("el_axis") } }),
                vec!["edge"],
            ),
            (
                "linearPattern: the source body",
                "LinearPattern",
                json!({ "direction": [1,0,0], "count": 2, "spacing": 5.0, "sourceBodyId": b }),
                vec!["body"],
            ),
            (
                "circularPattern: the source body",
                "CircularPattern",
                json!({ "axisOrigin": [0,0,0], "axisDirection": [0,0,1], "count": 3, "angleDeg": 120.0, "sourceBodyId": b }),
                vec!["body"],
            ),
            (
                "mirrorBody: the source body",
                "MirrorBody",
                json!({ "planePoint": [0,0,0], "planeNormal": [0,0,1], "sourceBodyId": b }),
                vec!["body"],
            ),
            (
                "transformBody: one slot per target, in `targets` order",
                "TransformBody",
                json!({ "translate": [1,0,0], "targets": [b, b2] }),
                vec!["body", "body"],
            ),
            (
                "sketch: hostFace is core-only ⇒ no slots",
                "Sketch",
                json!({ "sketchId": "00000000-0000-0000-0000-000000000011", "plane": { "kind": "XY", "origin": [0,0,0], "xAxis": [0,1,0], "yAxis": [-1,0,0], "normal": [0,0,1] }, "hostFace": face_ref }),
                vec![],
            ),
            (
                "loft: profile sketches only",
                "Loft",
                json!({ "booleanMode": "NewBody", "profiles": [] }),
                vec![],
            ),
            (
                "sweep: profile + path sketches only",
                "Sweep",
                json!({ "booleanMode": "NewBody" }),
                vec![],
            ),
            (
                "importStep: no inputs at all",
                "ImportStep",
                json!({ "sourceSha256": "aa", "sourceCodec": "step", "sourceName": "x.step" }),
                vec![],
            ),
            (
                "offsetFace: one slot per operative face, in `faces` order",
                "OffsetFace",
                json!({ "targetBodyId": b, "faceIds": ["el_2", "el_2"], "faces": [face_ref, face_ref], "distance": 2.5, "chainTangentFaces": true }),
                vec!["face", "face"],
            ),
            (
                // The `Total` opposite face is the LAST slot — a mapper that
                // assumed "every slot is an operative face" would rebind the
                // thickness reference as if it were part of the moved set.
                "offsetFace Total: the opposite face is the LAST slot",
                "OffsetFace",
                json!({ "targetBodyId": b, "faceIds": ["el_2"], "faces": [face_ref], "distance": 12.0, "distanceType": "Total", "chainTangentFaces": false, "oppositeFaceId": "el_2", "oppositeFace": face_ref }),
                vec!["face", "face"],
            ),
            (
                "placeComponent: no mate ⇒ dropped in free space, no slots",
                "PlaceComponent",
                json!({
                    "componentId": "onecad.std.iso4762", "componentVersion": "1.0.0",
                    "componentRevision": format!("sha256:{}", "0".repeat(64)),
                    "source": { "kind": "generator", "generatorId": "iso4762", "generatorVersion": 1 },
                    "placement": { "translate": [0.0, 0.0, 0.0] }
                }),
                vec![],
            ),
            (
                // P3 WP-3.1: mate is NEVER a wire input, present or not — the
                // worker's generic pre-flight would otherwise block the op on an
                // unresolved mate, contradicting spec §5.5 ("never drop it, never
                // silently move it"). `ComponentOp.cpp::resolve_mate_reseat` owns
                // mate resolution entirely, in-process, from `params.mate` alone.
                "placeComponent: mate present ⇒ STILL no wire input slot",
                "PlaceComponent",
                json!({
                    "componentId": "onecad.std.iso4762", "componentVersion": "1.0.0",
                    "componentRevision": format!("sha256:{}", "0".repeat(64)),
                    "source": { "kind": "generator", "generatorId": "iso4762", "generatorVersion": 1 },
                    "mate": { "selfAttachment": "shank_axis", "target": face_ref, "kind": "concentric", "flipped": false },
                    "placement": { "translate": [0.0, 0.0, 0.0] }
                }),
                vec![],
            ),
            (
                "detachComponent: no mate, no identity ⇒ no slots at all",
                "DetachComponent",
                json!({
                    "source": { "kind": "generator", "generatorId": "iso4762", "generatorVersion": 1 },
                    "placement": { "translate": [0.0, 0.0, 0.0] }
                }),
                vec![],
            ),
        ];

        for (name, op_type, params, expected) in &cases {
            let known: KnownOperation =
                serde_json::from_value(json!({ "opType": op_type, "params": params }))
                    .unwrap_or_else(|e| panic!("{name}: fixture deserializes: {e}"));
            let operation = Operation::Known(known);
            let inputs = operation.derive_inputs();
            let wired = wire_op_inputs(&operation, &inputs);
            let kinds: Vec<String> = wired
                .as_array()
                .expect("inputs[] is an array")
                .iter()
                .map(|r| r["primary"]["kind"].as_str().unwrap_or("?").to_string())
                .collect();
            assert_eq!(
                kinds, *expected,
                "{name}: inputs[] slot table — got {wired}"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PrepareEdgeOp codec
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod prepare_edge_op_tests {
    use super::*;

    fn body() -> BodyId {
        BodyId(Uuid::from_u128(0x70))
    }

    #[test]
    fn args_render_mode_fence_chain_and_both_addressing_rungs() {
        let picks = [
            EdgeOpPick {
                body: Some(body()),
                address: FaceAddress::TopoKey("e:4"),
            },
            EdgeOpPick {
                body: None,
                address: FaceAddress::ElementId("el_9"),
            },
        ];
        let args = prepare_edge_op_args(SnapshotId(5012), EdgeOpMode::Chamfer, &picks, false);
        assert_eq!(args["snapshotId"], json!(5012));
        assert_eq!(args["mode"], json!("Chamfer"));
        assert_eq!(args["chainTangentEdges"], json!(false));
        assert_eq!(
            args["pickedEdges"][0]["bodyId"],
            json!(body_id_wire(body()))
        );
        assert_eq!(args["pickedEdges"][0]["topoKey"], json!("e:4"));
        assert!(args["pickedEdges"][0].get("elementId").is_none());
        assert_eq!(args["pickedEdges"][1]["elementId"], json!("el_9"));
        assert!(args["pickedEdges"][1].get("topoKey").is_none());
    }

    #[test]
    fn parses_accepted_closure_and_refusal() {
        let accepted = parse_prepare_edge_op(&json!({
            "snapshotId": 5012,
            "targetBodyId": "body_1",
            "edges": [
                { "topoKey": "e:4", "picked": true, "anchor": { "worldPoint": [1,2,3] } },
                { "topoKey": "e:5", "picked": false, "descriptor": { "curveType": 0 } }
            ],
            "refusal": null
        }))
        .expect("accepted closure");
        assert_eq!(accepted.edges.len(), 2);
        assert!(accepted.edges[0].picked);
        assert_eq!(
            accepted.edges[1].descriptor,
            Some(json!({ "curveType": 0 }))
        );

        let refused = parse_prepare_edge_op(&json!({
            "snapshotId": 5012,
            "targetBodyId": "",
            "edges": [],
            "refusal": { "code": "chainMismatch", "message": "expanded", "edges": ["e:5"] }
        }))
        .expect("refusal is an answer");
        assert_eq!(refused.refusal.expect("refusal").code, "chainMismatch");
    }

    #[test]
    fn rejects_malformed_accepted_results() {
        assert!(parse_prepare_edge_op(&json!({ "targetBodyId": "body_1" })).is_err());
        assert!(parse_prepare_edge_op(&json!({
            "snapshotId": 1, "targetBodyId": "", "edges": [{ "topoKey": "e:1" }]
        }))
        .is_err());
        assert!(parse_prepare_edge_op(&json!({
            "snapshotId": 1, "targetBodyId": "body_1", "edges": []
        }))
        .is_err());
        assert!(parse_prepare_edge_op(&json!({
            "snapshotId": 1, "targetBodyId": "body_1", "edges": [{ "picked": true }]
        }))
        .is_err());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AnalyzeEdgeOpRange codec (SCHEMA §7.6, 2026-08-20)
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod analyze_edge_op_range_tests {
    use super::*;

    fn body() -> BodyId {
        BodyId(Uuid::from_u128(0x51))
    }

    fn picks() -> Vec<EdgeOpPick<'static>> {
        vec![EdgeOpPick {
            body: Some(body()),
            address: FaceAddress::TopoKey("e:4"),
        }]
    }

    fn answer() -> Value {
        json!({
            "snapshotId": 5012,
            "mode": "Fillet",
            "targetBodyId": "body_1",
            "edges": ["e:4", "e:5"],
            "searchedRange": { "min": 0.001, "max": 17.32 },
            "lowerBound": 0.001,
            "bestKnownMax": 9.99925,
            "provenUpperBound": 10.0,
            "feasibleIntervals": [{ "lower": 0.001, "upper": 9.99925 }],
            "intervalsTruncated": false,
            "limitingEntities": [{ "topoKey": "e:4", "kind": "edge" }],
            "confidence": "bracketed",
            "monotonicObserved": true,
            "probesUsed": 71,
            "budgetExhausted": false,
            "stoppedReason": "converged",
            "refusal": null
        })
    }

    #[test]
    fn optional_request_fields_are_omitted_not_defaulted() {
        let bare = analyze_edge_op_range_args(
            SnapshotId(5012),
            EdgeOpMode::Fillet,
            &picks(),
            true,
            EdgeOpRangeRequest::default(),
        );
        assert_eq!(bare["snapshotId"], json!(5012));
        assert_eq!(bare["mode"], json!("Fillet"));
        assert_eq!(bare["pickedEdges"][0]["topoKey"], json!("e:4"));
        // An absent `range` means "the kernel's own body-derived bracket". A
        // `{min:0,max:0}` here would search nothing.
        assert!(bare.get("range").is_none());
        assert!(bare.get("probeBudget").is_none());

        let full = analyze_edge_op_range_args(
            SnapshotId(7),
            EdgeOpMode::Chamfer,
            &picks(),
            false,
            EdgeOpRangeRequest {
                min: Some(1.0),
                max: Some(6.0),
                probe_budget: Some(32),
            },
        );
        assert_eq!(full["mode"], json!("Chamfer"));
        assert_eq!(full["chainTangentEdges"], json!(false));
        assert_eq!(full["range"], json!({ "min": 1.0, "max": 6.0 }));
        assert_eq!(full["probeBudget"], json!(32));

        let floor_only = analyze_edge_op_range_args(
            SnapshotId(7),
            EdgeOpMode::Fillet,
            &picks(),
            true,
            EdgeOpRangeRequest {
                min: Some(1.0),
                ..EdgeOpRangeRequest::default()
            },
        );
        assert_eq!(floor_only["range"], json!({ "min": 1.0 }));
    }

    #[test]
    fn parses_a_measured_answer() {
        let dto = parse_analyze_edge_op_range(&answer()).expect("measured answer");
        assert_eq!(dto.snapshot_id, 5012);
        assert_eq!(dto.mode, "Fillet");
        assert_eq!(dto.edges, vec!["e:4".to_string(), "e:5".to_string()]);
        assert_eq!(dto.lower_bound, Some(0.001));
        assert_eq!(dto.best_known_max, Some(9.99925));
        assert_eq!(dto.proven_upper_bound, Some(10.0));
        assert_eq!(dto.feasible_intervals.len(), 1);
        assert_eq!(dto.limiting_entities[0].topo_key, "e:4");
        assert_eq!(dto.confidence, "bracketed");
        assert_eq!(dto.stopped_reason, "converged");
        assert_eq!(dto.probes_used, 71);
        assert!(dto.refusal.is_none());
    }

    #[test]
    fn unproven_bounds_stay_none_and_serialize_as_null() {
        let mut result = answer();
        result["lowerBound"] = Value::Null;
        result["bestKnownMax"] = Value::Null;
        result["provenUpperBound"] = Value::Null;
        result["feasibleIntervals"] = json!([]);
        result["confidence"] = json!("none");
        let dto = parse_analyze_edge_op_range(&result).expect("an unproven answer is an answer");
        assert_eq!(dto.lower_bound, None);
        assert_eq!(dto.best_known_max, None);
        assert_eq!(dto.proven_upper_bound, None);
        // The FRONTEND must see the key. An absent key and a `null` read the same
        // in TypeScript, but only one of them survives a `skip_serializing_if`,
        // and the DTO must not be allowed to grow one.
        let wire = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(wire["lowerBound"], Value::Null);
        assert_eq!(wire["bestKnownMax"], Value::Null);
        assert_eq!(wire["provenUpperBound"], Value::Null);
    }

    #[test]
    fn a_refusal_is_an_answer() {
        let mut result = answer();
        result["refusal"] = json!({
            "code": "crossBody", "message": "picks span bodies", "edges": ["e:4"]
        });
        let dto = parse_analyze_edge_op_range(&result).expect("refusal parses");
        assert_eq!(dto.refusal.expect("refusal").code, "crossBody");
    }

    #[test]
    fn rejects_an_unknown_vocabulary_rather_than_guessing() {
        // A confidence rung the frontend cannot interpret means it cannot know
        // what it is allowed to clamp. Failing loudly beats picking a clamp.
        let mut unknown_confidence = answer();
        unknown_confidence["confidence"] = json!("probably");
        assert!(parse_analyze_edge_op_range(&unknown_confidence).is_err());

        let mut unknown_stop = answer();
        unknown_stop["stoppedReason"] = json!("gaveUp");
        assert!(parse_analyze_edge_op_range(&unknown_stop).is_err());

        let mut unknown_mode = answer();
        unknown_mode["mode"] = json!("Draft");
        assert!(parse_analyze_edge_op_range(&unknown_mode).is_err());

        assert!(parse_analyze_edge_op_range(&json!({ "mode": "Fillet" })).is_err());
    }

    #[test]
    fn rejects_a_violated_ordering_invariant() {
        // SCHEMA §7.6: `lowerBound <= bestKnownMax < provenUpperBound`. Every
        // consumer clamps with these three, so a violation must not reach one.
        let mut inverted = answer();
        inverted["lowerBound"] = json!(20.0);
        assert!(parse_analyze_edge_op_range(&inverted).is_err());

        let mut ceiling_at_the_max = answer();
        ceiling_at_the_max["provenUpperBound"] = json!(9.99925);
        assert!(parse_analyze_edge_op_range(&ceiling_at_the_max).is_err());

        let mut inverted_interval = answer();
        inverted_interval["feasibleIntervals"] = json!([{ "lower": 5.0, "upper": 1.0 }]);
        assert!(parse_analyze_edge_op_range(&inverted_interval).is_err());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PrepareOffsetFace codec (SCHEMA §7.6, 2026-08-06)
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod prepare_offset_face_tests {
    use super::*;

    fn body() -> BodyId {
        BodyId(Uuid::from_u128(0x70))
    }

    #[test]
    fn args_render_both_addressing_rungs_and_the_wire_body_form() {
        let picks = [
            OffsetFacePick {
                body: Some(body()),
                address: FaceAddress::TopoKey("f:22"),
            },
            OffsetFacePick {
                body: None,
                address: FaceAddress::ElementId("el_9c"),
            },
        ];
        let args = prepare_offset_face_args(
            SnapshotId(5012),
            &picks,
            false,
            OffsetDistanceType::Diameter,
        );
        assert_eq!(args["snapshotId"], json!(5012));
        assert_eq!(args["chainTangentFaces"], json!(false));
        assert_eq!(args["distanceType"], json!("Diameter"));

        let picked = args["pickedFaces"].as_array().expect("pickedFaces[]");
        assert_eq!(picked[0]["bodyId"], json!(body_id_wire(body())));
        assert_eq!(picked[0]["topoKey"], json!("f:22"));
        assert!(
            picked[0].get("elementId").is_none(),
            "the two rungs are mutually exclusive — the worker branches on the \
             PRESENCE of elementId"
        );
        assert_eq!(picked[1]["elementId"], json!("el_9c"));
        assert!(picked[1].get("topoKey").is_none());
        assert!(picked[1].get("bodyId").is_none());
    }

    /// Every distance type spells identically on the verb and in the op's own
    /// `distanceType` param — they are the SAME enumeration.
    #[test]
    fn args_distance_type_matches_the_params_spelling() {
        for t in [
            OffsetDistanceType::Offset,
            OffsetDistanceType::Total,
            OffsetDistanceType::Radius,
            OffsetDistanceType::Diameter,
        ] {
            let picks = [OffsetFacePick {
                body: Some(body()),
                address: FaceAddress::TopoKey("f:1"),
            }];
            let args = prepare_offset_face_args(SnapshotId(1), &picks, true, t);
            assert_eq!(args["distanceType"], serde_json::to_value(t).unwrap());
        }
    }

    fn accepted() -> Value {
        json!({
            "snapshotId": 5012,
            "targetBodyId": "body_00000000-0000-0000-0000-000000000070",
            "faces": [
                { "topoKey": "f:22", "picked": true,
                  "anchor": { "worldPoint": [1.0, 2.0, 30.0] },
                  "descriptor": { "surfaceType": "plane" } },
                { "topoKey": "f:23", "picked": false,
                  "anchor": { "worldPoint": [4.0, 2.0, 30.0] },
                  "descriptor": { "surfaceType": "plane" } }
            ],
            "oppositeFace": { "topoKey": "f:04",
                              "anchor": { "worldPoint": [1.0, 2.0, 0.0] },
                              "descriptor": { "surfaceType": "plane" } },
            "currentDims": { "radius": 5.0, "thickness": 10.0 },
            "refusal": null
        })
    }

    #[test]
    fn parses_an_accepted_closure_verbatim() {
        let dto = parse_prepare_offset_face(&accepted()).expect("accepted result");
        assert_eq!(dto.snapshot_id, 5012);
        assert_eq!(
            dto.target_body_id,
            "body_00000000-0000-0000-0000-000000000070"
        );
        assert_eq!(dto.faces.len(), 2);
        assert_eq!(dto.faces[0].topo_key, "f:22");
        assert!(dto.faces[0].picked, "the user's pick is flagged");
        assert!(
            !dto.faces[1].picked,
            "a face the tangent closure added is not"
        );
        assert_eq!(
            dto.faces[0].descriptor,
            Some(json!({ "surfaceType": "plane" })),
            "worker-owned evidence rides through verbatim"
        );
        let opposite = dto.opposite_face.expect("Total opposite candidate");
        assert_eq!(opposite.topo_key, "f:04");
        assert!(
            !opposite.picked,
            "SCHEMA emits no `picked` on the opposite face — it is never operative"
        );
        assert_eq!(dto.current_dims.radius, Some(5.0));
        assert_eq!(dto.current_dims.thickness, Some(10.0));
        assert!(dto.refusal.is_none());
    }

    /// A refusal is an ANSWER (`ok:true`), so it parses cleanly even though it
    /// carries no closure — the tool has to explain WHY it cannot offset.
    #[test]
    fn parses_a_refusal_without_a_closure() {
        let dto = parse_prepare_offset_face(&json!({
            "snapshotId": 5012,
            "targetBodyId": "",
            "faces": [],
            "currentDims": {},
            "refusal": { "code": "chainMismatch",
                         "message": "tangent faces would move too",
                         "faces": ["f:31"] }
        }))
        .expect("a refusal is a successful answer");
        let refusal = dto.refusal.expect("refusal");
        assert_eq!(refusal.code, "chainMismatch");
        assert_eq!(refusal.faces, vec!["f:31".to_string()]);
        assert!(dto.faces.is_empty());
        assert_eq!(dto.current_dims.radius, None);
    }

    /// A structurally broken result is a PROTOCOL break, never a default: this
    /// response is what the record FREEZES.
    #[test]
    fn rejects_a_malformed_result() {
        // No snapshot echo ⇒ nothing to compare the freeze against.
        assert!(parse_prepare_offset_face(&json!({ "targetBodyId": "body_1" })).is_err());
        // Accepted but bodyless ⇒ the op would name no body to modify.
        assert!(parse_prepare_offset_face(&json!({
            "snapshotId": 1, "targetBodyId": "", "faces": [{ "topoKey": "f:1" }]
        }))
        .is_err());
        // Accepted but empty ⇒ an operative set with nothing in it.
        assert!(parse_prepare_offset_face(&json!({
            "snapshotId": 1, "targetBodyId": "body_1", "faces": []
        }))
        .is_err());
        // A face entry with no topoKey has no evidence to promote.
        assert!(parse_prepare_offset_face(&json!({
            "snapshotId": 1, "targetBodyId": "body_1", "faces": [{ "picked": true }]
        }))
        .is_err());
        // A refusal with no code cannot be explained to anyone.
        assert!(parse_prepare_offset_face(&json!({
            "snapshotId": 1, "targetBodyId": "", "faces": [], "refusal": { "message": "no" }
        }))
        .is_err());
    }

    /// A non-finite `currentDims` reading is DROPPED, not surfaced: an absent seed
    /// makes the tool ask for a value, a `NaN` seed would render as one.
    #[test]
    fn drops_a_non_finite_current_dim() {
        let mut r = accepted();
        r["currentDims"] = json!({ "radius": "oops" });
        let dto = parse_prepare_offset_face(&r).expect("accepted");
        assert_eq!(dto.current_dims.radius, None);
        assert_eq!(dto.current_dims.thickness, None);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Component-source blob path injection (Component Library WP-3.2)
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod component_blob_path_tests {
    use super::*;
    use onecad_core::document::record::{
        ComponentSourceRef, DetachComponentParams, FrozenPlacement, ImportSourceCodec,
        PlaceComponentParams,
    };
    use onecad_core::document::variables::Scalar;
    use onecad_core::ids::DocumentId;

    fn placement() -> FrozenPlacement {
        FrozenPlacement {
            translate: [Scalar::new(1.0), Scalar::new(2.0), Scalar::new(3.0)],
            rotate: Default::default(),
        }
    }

    fn embedded(sha: &str) -> ComponentSourceRef {
        ComponentSourceRef::Embedded {
            sha256: sha.to_string(),
            codec: ImportSourceCodec::Brep,
            brep_format: Some(4),
            extra: Default::default(),
        }
    }

    fn place(source: ComponentSourceRef) -> Operation {
        Operation::Known(KnownOperation::PlaceComponent(PlaceComponentParams {
            component_id: "acme.bracket".into(),
            component_version: "1.0.0".into(),
            component_revision: format!("sha256:{}", "0".repeat(64)),
            params: Default::default(),
            source,
            mate: None,
            placement: placement(),
            extra: Default::default(),
        }))
    }

    /// The bytes reach the worker as a FILE, exactly like `ImportStep`'s: the
    /// record carries only the content address, and Rust injects the path it
    /// materialized the blob to.
    #[test]
    fn a_materialized_component_blob_lowers_its_path_under_source() {
        let bytes = b"pretend brep bytes";
        let sha = crate::imports::sha256_hex(bytes);
        let mut ws = crate::imports::ImportWorkspace::new(DocumentId(Uuid::from_u128(0xB10B)));
        let written = ws
            .materialize(&sha, ImportSourceCodec::Brep, bytes)
            .expect("materialize");

        let op = place(embedded(&sha));
        let wire = preview_wire_op(&op, "op_1");
        assert_eq!(
            wire["params"]["source"]["path"],
            json!(written.to_string_lossy())
        );
        // …and the record's own fields are untouched: the path is wire-only, so
        // it must never displace the content address the planner hashes.
        assert_eq!(wire["params"]["source"]["sha256"], json!(sha));
        assert_eq!(wire["params"]["source"]["brepFormat"], json!(4));

        // The SAME rule for a detached component: it keeps replaying the blob.
        let detach = Operation::Known(KnownOperation::DetachComponent(DetachComponentParams {
            source: embedded(&sha),
            placement: placement(),
            extra: Default::default(),
        }));
        assert_eq!(
            preview_wire_op(&detach, "op_2")["params"]["source"]["path"],
            json!(written.to_string_lossy())
        );
    }

    /// An unmaterialized blob lowers an EMPTY path on purpose — the worker fails
    /// THAT step with a named `OP_FAILED` while every other feature in the
    /// document still regenerates (the `io::imports` blast-radius rule).
    #[test]
    fn an_unmaterialized_component_blob_lowers_an_empty_path() {
        let wire = preview_wire_op(&place(embedded(&"e".repeat(64))), "op_3");
        assert_eq!(wire["params"]["source"]["path"], json!(""));
    }

    /// A generator source depends on no bytes, so nothing is injected — a `path`
    /// key there would be meaningless noise the worker would have to ignore.
    #[test]
    fn a_generator_source_gets_no_path() {
        let wire = preview_wire_op(
            &place(ComponentSourceRef::Generator {
                generator_id: "iso4762".into(),
                generator_version: 1,
                params: Default::default(),
                extra: Default::default(),
            }),
            "op_4",
        );
        assert!(wire["params"]["source"].get("path").is_none());
    }
}
