//! `OperationRecord` v2 — the on-disk file-format node (**high-stakes**: changing
//! its serialized shape is a schema change requiring sign-off).
//!
//! Serde contract (plan "Rust core specifics"):
//! * camelCase everywhere; **no `deny_unknown_fields` anywhere**.
//! * The operation is flattened onto the record as `{opType, params}`
//!   (adjacently tagged — SCHEMA §7.3, matching OneCAD-CPP `operationTypeName`
//!   PascalCase tag values).
//! * `extra` (`flatten`) maps at BOTH record and params levels carry unknown
//!   keys forward losslessly.
//! * An unknown `opType` deserializes to [`Operation::Opaque`] and round-trips
//!   byte-stably as a frozen node.
//! * `Scalar` (see [`crate::document::variables`]) carries dimension values
//!   (distance/radius/angle …) and accepts a bare wire number.
//!
//! Fillet and Chamfer are **split** ops (OneCAD-CPP shares `FilletChamferParams`;
//! here they are distinct variants keyed by `opType`).
//!
//! Record (de)serialization is **hand-written** (not derived) because the
//! required combination — flattening an adjacently-tagged-with-untagged-fallback
//! enum next to a second `flatten` extra map — is exactly the corner of serde's
//! `flatten` support that misbehaves. The manual impl gives byte-exact control
//! and is exercised by the snapshot + round-trip tests.
//!
//! **Reserved keys in `extra`.** The `extra` maps (record / params / nested-ref
//! level) are for *unknown* keys only. A caller MUST NOT stash a *reserved* key
//! (one a typed field owns — `opType`, `params`, `recordId`, `distance`, …) in an
//! `extra` map: on serialize the typed field is written first and the `extra`
//! entry is written second under the same name, producing a duplicate key. The
//! (de)serialize path never *reads back into* `extra` a key a typed field
//! claimed, so a well-formed load never populates `extra` with a reserved key;
//! the constraint is on hand-constructed values. The round-trip/proptest suites
//! only ever inject non-reserved (`alien*`) keys.
//!
//! **Duplicate JSON keys — file vs wire divergence.** On the FILE path the core
//! parses through `serde_json`, whose object model is **last-writer-wins** for a
//! duplicated key; a duplicate is therefore silently collapsed, not rejected.
//! This is accepted for the file format (files are Rust-authored; a duplicate can
//! only arise from external tampering, where last-wins is a safe, deterministic
//! resolution). The WIRE path is stricter: SCHEMA §4 mandates that a worker frame
//! with a duplicated object key is a `PROTOCOL_ERROR`. The divergence is
//! intentional — the wire is an adversarial trust boundary, the on-disk document
//! is not.

use serde::de::{self, DeserializeOwned};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::document::refs::{AxisRef, ElementRef, Extra, SketchRegionRef};
use crate::document::variables::Scalar;
use crate::ids::{BodyId, ElementId, RecordId, SketchId};
use crate::math::Vec3;

/// The record-schema version stamped on freshly authored records.
pub const RECORD_SCHEMA_VERSION: u32 = 1;

/// Known operation tag values (PascalCase; OneCAD-CPP `operationTypeName` +
/// the new `Sketch` op). An `opType` outside this set becomes
/// [`Operation::Opaque`].
const KNOWN_OP_TYPES: &[&str] = &[
    "Sketch",
    "Extrude",
    "Revolve",
    "Fillet",
    "Chamfer",
    "Shell",
    "Boolean",
    "LinearPattern",
    "CircularPattern",
    "Loft",
    "Sweep",
    "MirrorBody",
    "ImportStep",
    "TransformBody",
    "Hole",
];

// ─────────────────────────────────────────────────────────────────────────────
// OperationRecord
// ─────────────────────────────────────────────────────────────────────────────

/// A single node in the linear timeline; the unit of the persisted file format.
#[derive(Debug, Clone, PartialEq)]
pub struct OperationRecord {
    /// Stable record identity.
    pub record_id: RecordId,
    /// Record-schema version (currently [`RECORD_SCHEMA_VERSION`]).
    pub record_schema_version: u32,
    /// Position in the timeline. Serialized for human readability; the **array
    /// order is authoritative** on load.
    pub step_index: u32,
    /// Human-facing name / alias.
    pub name: String,
    /// The operation (`{opType, params}`, flattened onto the record).
    pub op: Operation,
    /// Derived uniform view of the op's typed inputs (bodies/sketches/elements).
    /// Serialized for tooling; rebuilt from `op` on demand.
    pub inputs: OperationInputs,
    /// Bodies produced/modified by this op (OneCAD-CPP `resultBodyIds`).
    pub outputs: Vec<BodyId>,
    /// Determinism policy captured for reproducible replay.
    pub determinism: DeterminismSettings,
    /// Whether the op is suppressed (skipped during regen).
    pub suppressed: bool,
    /// Unknown record-level keys, preserved verbatim.
    pub extra: Extra,
}

impl OperationRecord {
    /// Builds a v1 record with derived inputs and default determinism.
    #[must_use]
    pub fn new(
        record_id: RecordId,
        step_index: u32,
        name: impl Into<String>,
        op: Operation,
    ) -> Self {
        let inputs = op.derive_inputs();
        Self {
            record_id,
            record_schema_version: RECORD_SCHEMA_VERSION,
            step_index,
            name: name.into(),
            op,
            inputs,
            outputs: Vec::new(),
            determinism: DeterminismSettings::default(),
            suppressed: false,
            extra: Extra::new(),
        }
    }
}

impl Serialize for OperationRecord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // The op flattens onto the record as its own top-level keys.
        let op_value = serde_json::to_value(&self.op).map_err(serde::ser::Error::custom)?;
        let op_obj = op_value
            .as_object()
            .ok_or_else(|| serde::ser::Error::custom("operation did not serialize to an object"))?;

        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("recordId", &self.record_id)?;
        map.serialize_entry("recordSchemaVersion", &self.record_schema_version)?;
        map.serialize_entry("stepIndex", &self.step_index)?;
        map.serialize_entry("name", &self.name)?;
        for (k, v) in op_obj {
            map.serialize_entry(k, v)?;
        }
        map.serialize_entry("inputs", &self.inputs)?;
        map.serialize_entry("outputs", &self.outputs)?;
        map.serialize_entry("determinism", &self.determinism)?;
        map.serialize_entry("suppressed", &self.suppressed)?;
        for (k, v) in &self.extra {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for OperationRecord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut map = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;

        fn take<T: DeserializeOwned, E: de::Error>(
            map: &mut serde_json::Map<String, serde_json::Value>,
            key: &str,
        ) -> Result<Option<T>, E> {
            match map.remove(key) {
                None => Ok(None),
                Some(v) => serde_json::from_value(v)
                    .map(Some)
                    .map_err(|e| E::custom(format!("field `{key}`: {e}"))),
            }
        }

        let record_id: RecordId =
            take(&mut map, "recordId")?.ok_or_else(|| de::Error::missing_field("recordId"))?;
        let record_schema_version =
            take(&mut map, "recordSchemaVersion")?.unwrap_or(RECORD_SCHEMA_VERSION);
        let step_index = take(&mut map, "stepIndex")?.unwrap_or(0);
        let name = take(&mut map, "name")?.unwrap_or_default();
        // The stored `inputs` are DERIVED from `op` and treated as advisory: for a
        // Known op they are re-derived below (self-healing, M3); only an Opaque
        // frozen node keeps whatever was on disk.
        let stored_inputs: OperationInputs = take(&mut map, "inputs")?.unwrap_or_default();
        let outputs = take(&mut map, "outputs")?.unwrap_or_default();
        let determinism = take(&mut map, "determinism")?.unwrap_or_default();
        let suppressed = take(&mut map, "suppressed")?.unwrap_or(false);

        // Everything left is op-related (`opType`/`params`) plus record-level extra.
        let op_type = map
            .get("opType")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let (op, extra) = match op_type {
            Some(tag) if KNOWN_OP_TYPES.contains(&tag.as_str()) => {
                map.remove("opType");
                let params = map.remove("params");
                let mut op_obj = serde_json::Map::new();
                op_obj.insert("opType".into(), serde_json::Value::String(tag));
                if let Some(p) = params {
                    op_obj.insert("params".into(), p);
                }
                let known: KnownOperation =
                    serde_json::from_value(serde_json::Value::Object(op_obj))
                        .map_err(|e| de::Error::custom(format!("operation: {e}")))?;
                (Operation::Known(known), map)
            }
            _ => {
                // Unknown/missing opType → frozen node; everything left is its raw payload.
                (
                    Operation::Opaque(OpaqueOperation { raw: map }),
                    Extra::new(),
                )
            }
        };

        // M3 (Invariant: derived inputs are never trusted from disk): a Known op
        // RE-DERIVES its inputs from `op`, overwriting any tampered/stale stored
        // value; an Opaque frozen node (no typed deps) keeps the stored inputs.
        let inputs = match &op {
            Operation::Known(_) => op.derive_inputs(),
            Operation::Opaque(_) => stored_inputs,
        };

        Ok(OperationRecord {
            record_id,
            record_schema_version,
            step_index,
            name,
            op,
            inputs,
            outputs,
            determinism,
            suppressed,
            extra,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Operation
// ─────────────────────────────────────────────────────────────────────────────

/// The operation payload of a record: a known op or an opaque frozen node.
///
/// Serialize is untagged (a known op serializes as its `{opType, params}`; an
/// opaque op serializes as its flattened raw map). **Deserialize is
/// hand-written** and gates on [`KNOWN_OP_TYPES`] rather than falling through
/// untagged: an `opType` in the known set MUST deserialize as that typed op —
/// malformed `params` are a hard ERROR, never a silent demotion to `Opaque`
/// (M1). Only an unknown/absent `opType` becomes [`Operation::Opaque`]. This
/// makes the direct-`Operation` path agree with the hand-written
/// [`OperationRecord`] path (both error on a known op with bad params).
// Size disparity is inherent (Extrude carries rich typed face refs); records
// live behind a `Vec` and are not moved in hot loops, so the payload is left
// unboxed for a straightforward hand-written (de)serialize path.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Operation {
    /// A known, typed operation.
    Known(KnownOperation),
    /// An unknown-`opType` op captured verbatim (frozen node).
    Opaque(OpaqueOperation),
}

impl<'de> Deserialize<'de> for Operation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        let op_type = map
            .get("opType")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        match op_type {
            // Known opType: deserialize as the typed op; malformed params ERROR
            // (do NOT fall back to Opaque — that is the M1 fix).
            Some(tag) if KNOWN_OP_TYPES.contains(&tag.as_str()) => {
                let known: KnownOperation = serde_json::from_value(serde_json::Value::Object(map))
                    .map_err(|e| de::Error::custom(format!("operation `{tag}`: {e}")))?;
                Ok(Operation::Known(known))
            }
            // Unknown/absent opType: frozen node captured verbatim.
            _ => Ok(Operation::Opaque(OpaqueOperation { raw: map })),
        }
    }
}

/// A known operation, adjacently tagged `{opType, params}` (SCHEMA §7.3).
// See [`Operation`] on the size disparity / unboxed rationale.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "opType", content = "params")]
pub enum KnownOperation {
    Sketch(SketchOpParams),
    Extrude(ExtrudeParams),
    Revolve(RevolveParams),
    Fillet(FilletParams),
    Chamfer(ChamferParams),
    Shell(ShellParams),
    Boolean(BooleanParams),
    LinearPattern(LinearPatternParams),
    CircularPattern(CircularPatternParams),
    Loft(LoftParams),
    Sweep(SweepParams),
    MirrorBody(MirrorBodyParams),
    ImportStep(ImportStepParams),
    TransformBody(TransformBodyParams),
    Hole(HoleParams),
}

impl KnownOperation {
    /// Mutable access to every [`ElementRef`] this op carries as a topological
    /// **input**, over the SAME op set and under the SAME conditions as the wire's
    /// SCHEMA §7.3 `inputs[]` semantic-ref array (app crate
    /// `worker::wire::wire_op_inputs`). The two must agree: an op that lowers a
    /// semantic ref but is missing here would ship `intent: null` forever.
    ///
    /// The single writer uses this to stamp `intent.descriptor` evidence onto a
    /// freshly authored or re-edited ref (HISTORY-HARDEN H5), so the worker's
    /// resolution ladder scores real evidence instead of a bare anchor.
    ///
    /// **No `Sketch` arm, deliberately.** `SketchOpParams::host_face` is a
    /// CORE-ONLY dependency field — `wire::strip_sketch_host_face` drops `hostFace`
    /// from the lowered params (VF-B5a), so evidence stamped there could never
    /// reach the ladder. It would only churn the golden-pinned history-prefix hash
    /// for zero effect (the WP-FIX W4 no-backfill discipline).
    ///
    /// **No `Shell` arm.** `ShellParams::open_faces` are bare [`ElementId`]s with
    /// no ref slot to carry evidence — `wire::face_input_refs` renders them
    /// element-only. Giving Shell descriptor evidence needs a params-shape change,
    /// not a hydration pass. Recorded gap.
    #[must_use]
    pub fn element_refs_mut(&mut self) -> Vec<&mut ElementRef> {
        match self {
            KnownOperation::Fillet(p) => p.edges.iter_mut().collect(),
            KnownOperation::Chamfer(p) => p.edges.iter_mut().collect(),
            // The two ToFace gates mirror `wire_op_inputs` exactly: a `target_face`
            // left over from a mode the op no longer runs in is NOT lowered, so
            // stamping evidence on it would move the params hash for a ref the
            // worker never reads.
            KnownOperation::Extrude(p) => {
                let to_face = p.mode == ExtrudeMode::ToFace;
                let to_face2 = p.two_directions && p.mode2 == ExtrudeMode::ToFace;
                let mut refs: Vec<&mut ElementRef> = Vec::new();
                if to_face {
                    if let Some(f) = p.target_face.as_mut() {
                        refs.push(f);
                    }
                }
                if to_face2 {
                    if let Some(f) = p.target_face2.as_mut() {
                        refs.push(f);
                    }
                }
                refs
            }
            KnownOperation::Hole(p) => vec![&mut p.face],
            _ => Vec::new(),
        }
    }
}

/// Unknown-`opType` payload, captured as a raw map (frozen node).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpaqueOperation {
    /// The entire op object (`opType`, `params`, and any other keys) verbatim.
    #[serde(flatten)]
    pub raw: Extra,
}

impl Operation {
    /// The `opType` tag this operation serializes under (SCHEMA §7.3). For an
    /// [`Operation::Opaque`] node it is whatever tag the frozen JSON carried, or
    /// `"Opaque"` when it had none.
    #[must_use]
    pub fn op_type(&self) -> &str {
        match self {
            Operation::Known(k) => match k {
                KnownOperation::Sketch(_) => "Sketch",
                KnownOperation::Extrude(_) => "Extrude",
                KnownOperation::Revolve(_) => "Revolve",
                KnownOperation::Fillet(_) => "Fillet",
                KnownOperation::Chamfer(_) => "Chamfer",
                KnownOperation::Shell(_) => "Shell",
                KnownOperation::Boolean(_) => "Boolean",
                KnownOperation::LinearPattern(_) => "LinearPattern",
                KnownOperation::CircularPattern(_) => "CircularPattern",
                KnownOperation::Loft(_) => "Loft",
                KnownOperation::Sweep(_) => "Sweep",
                KnownOperation::MirrorBody(_) => "MirrorBody",
                KnownOperation::ImportStep(_) => "ImportStep",
                KnownOperation::TransformBody(_) => "TransformBody",
                KnownOperation::Hole(_) => "Hole",
            },
            // The frozen node keeps its original tag inside `raw`; report it so a
            // future opType is not mislabelled as one of the known ops.
            Operation::Opaque(o) => o
                .raw
                .get("opType")
                .and_then(serde_json::Value::as_str)
                .filter(|t| !t.is_empty())
                .unwrap_or("Opaque"),
        }
    }

    /// Derives the uniform input view, mirroring OneCAD-CPP
    /// `DependencyGraph::extractDependencies` (`DependencyGraph.cpp:252-332`)
    /// including param-embedded dependencies. See per-arm comments for the
    /// C++↔Rust parity mapping (SCHEMA/`OperationRecord.h` line citations).
    #[must_use]
    pub fn derive_inputs(&self) -> OperationInputs {
        let mut inputs = OperationInputs::default();
        let known = match self {
            Operation::Known(k) => k,
            // A frozen node exposes no typed deps (it never regenerates).
            Operation::Opaque(_) => return inputs,
        };
        match known {
            // No C++ analogue (Sketch is a new v2 op). A world-/datum-attached
            // sketch has no upstream feature dependency. A HOST-FACE sketch does:
            // its frozen frame is glued to a face of a model body, so the body (and
            // the face element) are real inputs — VF-B5a. Mirrors the Hole arm's
            // shape (host body + host face element from the ref's `primary`); an
            // intent-only ref contributes nothing and is bound by the ladder.
            // Absent on legacy records (never backfilled — see `SketchOpParams`).
            KnownOperation::Sketch(p) => {
                if let Some(primary) = p.host_face.as_ref().and_then(|f| f.primary.as_ref()) {
                    inputs.push_body(primary.body);
                    inputs.push_element(primary.element.clone());
                }
            }

            // Extrude: profile sketch + (target body iff boolean != NewBody).
            // Parity: DependencyGraph.cpp:254-256 (SketchRegionRef→sketch),
            // :270-278 (targetBody only when booleanMode != NewBody).
            // Note: C++ does NOT track ToFace target faces as deps — mirrored here.
            KnownOperation::Extrude(p) => {
                if let Some(profile) = &p.profile {
                    inputs.push_sketch(profile.sketch);
                }
                if p.boolean_mode != BooleanMode::NewBody {
                    if let Some(b) = p.target_body {
                        inputs.push_body(b);
                    }
                }
            }

            // Revolve: profile sketch + axis + (target body iff boolean != NewBody).
            // Parity: DependencyGraph.cpp:279-295.
            KnownOperation::Revolve(p) => {
                if let Some(profile) = &p.profile {
                    inputs.push_sketch(profile.sketch);
                }
                match &p.axis {
                    Some(AxisRef::SketchLine { sketch, .. }) => inputs.push_sketch(*sketch),
                    Some(AxisRef::Element { body, edge, .. }) => {
                        inputs.push_body(*body);
                        inputs.push_element(edge.clone());
                    }
                    None => {}
                }
                if p.boolean_mode != BooleanMode::NewBody {
                    if let Some(b) = p.target_body {
                        inputs.push_body(b);
                    }
                }
            }

            // Fillet/Chamfer: referenced edges (elements) + the operated body.
            // Parity: DependencyGraph.cpp:296-300 (edgeIds→inputEdgeIds) for the
            // edges, PLUS :264-267 (the op's input BodyRef → the operated body) —
            // recovered here from each typed edge ref's `primary.body` (M5). An
            // edge ref without a `primary` (intent-only) contributes no body; the
            // operated body is then bound at regen time. Bare `edge_ids` (no typed
            // ref) contribute only the element id.
            KnownOperation::Fillet(p) => {
                derive_fillet_chamfer_inputs(&mut inputs, &p.edge_ids, &p.edges)
            }
            KnownOperation::Chamfer(p) => {
                derive_fillet_chamfer_inputs(&mut inputs, &p.edge_ids, &p.edges)
            }

            // Shell: open faces (elements) + shelled body. Parity:
            // DependencyGraph.cpp:301-305 (openFaceIds→inputFaceIds); the body
            // comes from the C++ BodyRef input (:264-267), modeled here as
            // `target_body`.
            KnownOperation::Shell(p) => {
                if let Some(b) = p.target_body {
                    inputs.push_body(b);
                }
                for f in &p.open_faces {
                    inputs.push_element(f.clone());
                }
            }

            // Boolean: target + tool bodies. Parity: DependencyGraph.cpp:306-309.
            KnownOperation::Boolean(p) => {
                inputs.push_body(p.target_body);
                inputs.push_body(p.tool_body);
            }

            // Linear/Circular pattern: source body. Parity:
            // DependencyGraph.cpp:310-319.
            KnownOperation::LinearPattern(p) => {
                if let Some(b) = p.source_body {
                    inputs.push_body(b);
                }
            }
            KnownOperation::CircularPattern(p) => {
                if let Some(b) = p.source_body {
                    inputs.push_body(b);
                }
            }

            // Loft: profile sketches. NOTE: C++ extractDependencies OMITS Loft
            // (a gap — LoftParams is absent from the if-chain, cpp:252-332); the
            // Rust port tracks profile sketches so a loft regenerates when a
            // profile sketch changes.
            KnownOperation::Loft(p) => {
                for profile in &p.profiles {
                    inputs.push_sketch(profile.sketch);
                }
            }

            // Sweep: profile sketch + path sketch + path edge. Parity:
            // DependencyGraph.cpp:320-331.
            KnownOperation::Sweep(p) => {
                if let Some(profile) = &p.profile {
                    inputs.push_sketch(profile.sketch);
                }
                if let Some(s) = p.path_sketch {
                    inputs.push_sketch(s);
                }
                if let Some(e) = &p.path_edge {
                    inputs.push_element(e.clone());
                }
            }

            // MirrorBody: source body. NOTE: C++ extractDependencies OMITS
            // MirrorBody (cpp:252-332); the task mandates tracking mirror sources,
            // so the Rust port adds it.
            KnownOperation::MirrorBody(p) => {
                if let Some(b) = p.source_body {
                    inputs.push_body(b);
                }
            }

            // ImportStep: NO inputs. An import depends on nothing in the document
            // — its whole upstream is the content-addressed source blob, which is
            // a container section (`imports/<sha256>.<codec>`), not a timeline
            // node. Giving it a dependency would make the dependency graph claim
            // an edge that regen cannot honour. No C++ analogue (new v2 op).
            KnownOperation::ImportStep(_) => {}

            // TransformBody: every target body (SCHEMA §7.3 — `inputs[]` mirrors
            // `params.targets`). No C++ analogue (new v2 op). `copy: true` still
            // depends on its sources: the copies are rebuilt from them each regen.
            KnownOperation::TransformBody(p) => {
                for b in &p.targets {
                    inputs.push_body(*b);
                }
            }

            // Hole: the host body it is machined into, plus the host FACE the
            // axis is derived from (SCHEMA §7.3 — `inputs: [semanticRef(host
            // body), semanticRef(host face)]`). No C++ analogue (new v2 op).
            // The face element id comes from the ref's `primary` when it has one;
            // an intent-only face ref contributes no element dep and is bound by
            // the ladder at regen time (mirrors the Fillet edge-ref rule).
            KnownOperation::Hole(p) => {
                inputs.push_body(p.target_body);
                if let Some(primary) = &p.face.primary {
                    inputs.push_body(primary.body);
                    inputs.push_element(primary.element.clone());
                }
            }
        }
        inputs
    }
}

/// Shared Fillet/Chamfer input derivation (M5): bare `edge_ids` supply element
/// deps; typed `edges` additionally supply the operated body (`primary.body`,
/// deduped) and — when present — the primary element id. Intent-only edge refs
/// (no `primary`) contribute nothing here; regen binds the body later.
fn derive_fillet_chamfer_inputs(
    inputs: &mut OperationInputs,
    edge_ids: &[ElementId],
    edges: &[ElementRef],
) {
    for e in edge_ids {
        inputs.push_element(e.clone());
    }
    for r in edges {
        if let Some(primary) = &r.primary {
            inputs.push_body(primary.body);
            inputs.push_element(primary.element.clone());
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// OperationInputs (derived uniform view)
// ─────────────────────────────────────────────────────────────────────────────

/// Derived, order-preserving, de-duplicated view of a record's typed inputs,
/// for dependency-graph construction. Faces and edges are unified into
/// `elements` (plan "derived uniform view").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationInputs {
    pub bodies: Vec<BodyId>,
    pub sketches: Vec<SketchId>,
    pub elements: Vec<ElementId>,
}

impl OperationInputs {
    fn push_body(&mut self, id: BodyId) {
        if !self.bodies.contains(&id) {
            self.bodies.push(id);
        }
    }
    fn push_sketch(&mut self, id: SketchId) {
        if !self.sketches.contains(&id) {
            self.sketches.push(id);
        }
    }
    fn push_element(&mut self, id: ElementId) {
        if !self.elements.contains(&id) {
            self.elements.push(id);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DeterminismSettings
// ─────────────────────────────────────────────────────────────────────────────

/// Determinism policy captured on a record for reproducible replay
/// (OneCAD-CPP `OperationMetadata.h DeterminismSettings` + SCHEMA §7.3
/// `determinism`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeterminismSettings {
    /// `false` = single-threaded OCCT (reproducible); default.
    #[serde(default)]
    pub parallel: bool,
    /// OCCT algorithm knobs (SCHEMA `occtOptions`, e.g. `fuzzyValue`, `useOBB`).
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub occt_options: Extra,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub occt_options_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tolerance_policy_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub solver_policy_hash: String,
    /// Unknown determinism-level keys, preserved verbatim (M2). Distinct from
    /// `occt_options`, which is the typed OCCT-knob map.
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

// ─────────────────────────────────────────────────────────────────────────────
// Enums shared by params (PascalCase wire values, matching SCHEMA §7.3)
// ─────────────────────────────────────────────────────────────────────────────

/// Extrude end condition (OneCAD-CPP `ExtrudeMode`; SCHEMA
/// `Blind`/`ThroughAll`/`Symmetric`/`ToNext`/`ToFace`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExtrudeMode {
    #[default]
    Blind,
    ThroughAll,
    Symmetric,
    ToNext,
    ToFace,
}

/// Feature-fused boolean mode (OneCAD-CPP `BooleanMode`; SCHEMA
/// `NewBody`/`Add`/`Cut`/`Intersect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BooleanMode {
    #[default]
    NewBody,
    Add,
    Cut,
    Intersect,
}

/// Standalone boolean operation (OneCAD-CPP `BooleanParams::Op`; SCHEMA
/// `Union`/`Cut`/`Intersect`). Distinct from the feature-fused [`BooleanMode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BooleanOp {
    #[default]
    Union,
    Cut,
    Intersect,
}

/// Machined-hole profile (SCHEMA §7.3 `Hole.holeType` ∈ `simple` /
/// `counterbore` / `countersink`). Lowercase wire values — unlike the PascalCase
/// mode enums above, these are new-in-v2 values SCHEMA spells lowercase.
///
/// The variant selects which conditional param block is REQUIRED (see
/// [`HoleParams::validate`]): `Counterbore` needs `cb*`, `Countersink` needs
/// `cs*`, `Simple` needs neither and permits neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HoleType {
    /// A plain drilled cylinder.
    #[default]
    Simple,
    /// A drilled cylinder with a larger coaxial flat-bottomed recess seated at
    /// the face (socket-head cap-screw clearance).
    Counterbore,
    /// A drilled cylinder with a conical recess seated at the face (flat-head
    /// screw clearance).
    Countersink,
}

/// The countersink included angles SCHEMA §7.3 admits, in degrees.
///
/// Not a free scalar: a countersink angle must match the screw head it clears,
/// and the four values here are the entire population of standard included
/// angles (DIN 74 / ISO 7046 90°, ANSI 82°, DIN 7721 / metric-coarse 100°,
/// rivet/aerospace 120°). An arbitrary angle would silently produce a cone no
/// fastener seats in, so it is refused at the authoring boundary.
pub const HOLE_CS_ANGLES_DEG: [f64; 4] = [82.0, 90.0, 100.0, 120.0];

/// Named sketch plane (SCHEMA §7.3 `plane.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PlaneKind {
    #[serde(rename = "XY")]
    #[default]
    Xy,
    #[serde(rename = "XZ")]
    Xz,
    #[serde(rename = "YZ")]
    Yz,
    #[serde(rename = "custom")]
    Custom,
}

/// Codec of an [`ImportStepParams`] source blob. Lowercase wire values, matching
/// the `imports/<sha256>.<ext>` container-section extension exactly (the enum IS
/// the extension registry — see [`ImportSourceCodec::extension`]).
///
/// `Step` is the raw user-supplied STEP text; `Brep` and `Xbf` are kernel-native
/// dumps produced ONCE at import and pinned by [`ImportStepParams::brep_format`] —
/// a from-0 replay never consults checkpoints, so re-parsing STEP on every open is
/// the alternative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportSourceCodec {
    /// ISO 10303-21 STEP text (`imports/<sha256>.step`).
    #[default]
    Step,
    /// OCCT BinTools BREP dump (`imports/<sha256>.brep`). Topology only — it
    /// carries NO XCAF attributes, so a document authored against it loses the
    /// imported product names and face colors. Superseded by [`Self::Xbf`] as the
    /// conversion lane's output (SCHEMA §14, 2026-08-02) and kept only so already
    /// authored documents keep replaying.
    Brep,
    /// OCCT BinXCAF document (`imports/<sha256>.xbf`) — shapes **plus** the XCAF
    /// product names and per-face colors. What the §7.8 conversion lane emits
    /// today, so an import survives save→reopen with its appearance intact.
    Xbf,
}

impl ImportSourceCodec {
    /// The container-section file extension for this codec (no leading dot).
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            ImportSourceCodec::Step => "step",
            ImportSourceCodec::Brep => "brep",
            ImportSourceCodec::Xbf => "xbf",
        }
    }

    /// The codec for a container-section extension (no leading dot), if known.
    /// An unrecognised extension is `None` — a foreign `imports/` entry is
    /// ignored, never guessed at.
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "step" => Some(ImportSourceCodec::Step),
            "brep" => Some(ImportSourceCodec::Brep),
            "xbf" => Some(ImportSourceCodec::Xbf),
            _ => None,
        }
    }

    /// True for a CONVERTED replay form — one whose bytes are a kernel dump whose
    /// binary format version must be pinned in [`ImportStepParams::brep_format`].
    /// False for [`Self::Step`], where a format pin would be meaningless.
    #[must_use]
    pub fn is_converted(self) -> bool {
        matches!(self, ImportSourceCodec::Brep | ImportSourceCodec::Xbf)
    }
}

/// The only heal policy this build authors / accepts (SCHEMA §7.3 `healPolicy`).
/// A future policy is a new *value*, never a reinterpretation of `"v1"`: the
/// string is part of the geometry-relevant params, so a document pins the exact
/// healing behaviour it was imported under.
pub const IMPORT_HEAL_POLICY_V1: &str = "v1";

fn default_heal_policy() -> String {
    IMPORT_HEAL_POLICY_V1.to_string()
}

fn default_unit_scale() -> Scalar {
    Scalar::new(1.0)
}

/// True iff `s` is a 64-character lowercase-hex SHA-256 digest — the ONLY shape
/// accepted for a content-addressed import blob key.
///
/// Strictness is load-bearing, not cosmetic: the key is interpolated into a ZIP
/// entry name (`imports/<sha256>.step`), so a lax check would let a hostile /
/// buggy `sourceSha256` inject a path separator. `[0-9a-f]{64}` cannot.
/// Uppercase hex is rejected too — two spellings of one digest would break
/// content-addressing (two entries, one blob).
#[must_use]
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn default_true() -> bool {
    true
}

/// Deserializes an optional `BodyId` where an empty string means "no body"
/// (SCHEMA §7.3 sends `"targetBodyId": ""` for the `NewBody` case).
fn de_opt_body_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<BodyId>, D::Error> {
    let opt = Option::<String>::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => Uuid::parse_str(&s)
            .map(|u| Some(BodyId(u)))
            .map_err(de::Error::custom),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Param structs (ported 1:1 from OperationRecord.h; SCHEMA §7.3 field names)
// ─────────────────────────────────────────────────────────────────────────────

/// Sketch feature (new v2 op; no OneCAD-CPP `OperationRecord.h` analogue —
/// sketches are inputs there, not ops). Entities/constraints are carried as
/// opaque JSON pending the typed sketch model (separate sketch WP); they
/// round-trip verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchOpParams {
    #[serde(rename = "sketchId")]
    pub sketch: SketchId,
    pub plane: SketchPlaneRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<serde_json::Value>,
    /// The host FACE a `HostFace`-attached sketch stands on (VF-B5a).
    ///
    /// **Why the record carries it at all.** The authoritative attachment lives on
    /// `Document.sketches[id].attachment` and is core-owned; but
    /// [`Operation::derive_inputs`] only sees the *record*, so without this field a
    /// face-hosted sketch declares NO dependency on the body it is glued to. The
    /// dependency graph then cannot see the edge, and — the actual defect — the
    /// SCHEMA §7.3 `TransformBody` edit-safety gate cannot gate it: moving the host
    /// body silently re-projects every downstream cut against stale geometry.
    ///
    /// **Never backfilled.** `inputs` is inside the golden prefix hash
    /// (`regen::planner`) and is re-derived on every deserialize, so populating this
    /// from the attachment at load time would move the hash and the bytes of every
    /// legacy document. It is stamped ONLY when the record is minted / refreshed
    /// from a live sketch, or when the attachment is re-picked. Legacy records keep
    /// `None` and are gated through the edit layer's attachment bridge instead.
    ///
    /// **Wire-omitted**: SCHEMA §7.3 states the attachment never crosses the wire,
    /// so the wire lowering strips this field (it is a `document.json` + planner-hash
    /// field only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_face: Option<ElementRef>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// A sketch plane (kind + basis) as carried in `SketchOpParams` (SCHEMA §7.3
/// `plane`). The named-plane bases are the NON-STANDARD OneCAD-CPP mapping
/// (see [`crate::sketch::plane`]).
///
/// Not `Copy`: carries an `extra` map so unknown keys injected at the `plane`
/// level round-trip losslessly (M2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchPlaneRef {
    pub kind: PlaneKind,
    pub origin: Vec3,
    pub x_axis: Vec3,
    pub y_axis: Vec3,
    pub normal: Vec3,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Extrude parameters. Ported from OneCAD-CPP `ExtrudeParams`
/// (`OperationRecord.h:91-104`); scalar/enum field names align with SCHEMA §7.3.
///
/// Discrepancies (SCHEMA wins on names; plan's typed-refs otherwise):
/// * `profile` (typed `SketchRegionRef`) is a Rust-core field only — SCHEMA/C++
///   carry the region in the separate `inputs[]`/`input`, not in params.
///   Optional/defaulted so SCHEMA §7.3 payloads (no `profile`) still parse.
/// * `target_face`/`target_face2` are typed `ElementRef`s (serialized
///   `targetFace`/`targetFace2`) for the `ToFace` end condition. SCHEMA §7.3 now
///   carries the SAME typed semantic-ref shape (amended 2026-07-16 — see the
///   SCHEMA Changelog): the previous bare-string `targetFaceId`/`targetFaceId2`
///   could not carry anchor/intent, leaving a ToFace target un-repairable across
///   parametric edits (Invariant 2/3). Absent for non-`ToFace` extrudes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtrudeParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<SketchRegionRef>,
    pub distance: Scalar,
    pub draft_angle_deg: Scalar,
    #[serde(rename = "extrudeMode")]
    pub mode: ExtrudeMode,
    pub boolean_mode: BooleanMode,
    #[serde(
        rename = "targetBodyId",
        default,
        deserialize_with = "de_opt_body_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_body: Option<BodyId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_face: Option<ElementRef>,
    pub two_directions: bool,
    #[serde(rename = "extrudeMode2")]
    pub mode2: ExtrudeMode,
    pub distance2: Scalar,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_face2: Option<ElementRef>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Revolve parameters. Ported from OneCAD-CPP `RevolveParams`
/// (`OperationRecord.h:106-112`); SCHEMA §7.3 field names.
///
/// `profile` is a Rust-core typed input (as for Extrude). `axis` is
/// `Option<AxisRef>`; SCHEMA's `kind:"none"` maps to `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevolveParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<SketchRegionRef>,
    pub angle_deg: Scalar,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<AxisRef>,
    pub boolean_mode: BooleanMode,
    #[serde(
        rename = "targetBodyId",
        default,
        deserialize_with = "de_opt_body_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_body: Option<BodyId>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Fillet parameters (SPLIT from OneCAD-CPP `FilletChamferParams`
/// `OperationRecord.h:114-120`). SCHEMA §7.3 field names.
///
/// The redundant C++/SCHEMA `mode` field (`"Fillet"`) is dropped in favor of the
/// authoritative `opType` tag; if present on input it round-trips via `extra`.
/// `edge_ids` are TopoKeys/ElementIds (bare strings), matching SCHEMA `edgeIds`.
///
/// `edges` is the typed home for SCHEMA §7.3 fillet's per-edge `inputs[]`
/// semantic refs (one `ElementRef` per `edge_ids` entry): each carries the
/// operated body (`primary.body`) plus descriptor/anchor evidence so the edge is
/// repairable across parametric edits via the ladder. Empty for legacy/bare-id
/// fillets (then the operated body is bound at regen time). SCHEMA's `edgeIds`
/// stays a bare-string list — the semantic refs live in `inputs[]`, so no SCHEMA
/// amendment is required (unlike Extrude ToFace, which had no such home; M4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilletParams {
    pub radius: Scalar,
    pub edge_ids: Vec<ElementId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<ElementRef>,
    #[serde(default = "default_true")]
    pub chain_tangent_edges: bool,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Chamfer parameters (SPLIT from OneCAD-CPP `FilletChamferParams`). `radius`
/// doubles as the chamfer distance (OneCAD-CPP comment `OperationRecord.h:117`).
/// `edges` mirrors [`FilletParams::edges`] (typed per-edge semantic refs).
///
/// `distance2` (SCHEMA §7.3, amended 2026-08-03 — WP-C T2a) is the CHAMFER-ONLY
/// second leg of an asymmetric two-distance chamfer. It is `skip_serializing_if =
/// "Option::is_none"` on purpose: every document authored before this field
/// existed still serializes byte-identically, and an equal-leg chamfer never
/// grows a key. [`FilletParams`] has no such field — a Fillet carrying one is
/// rejected by the session ([`crate::edit`]), not silently round-tripped through
/// `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChamferParams {
    pub radius: Scalar,
    /// Distance on the NON-reference adjacent face. Absent ⇒ equal-leg (`radius`
    /// on both faces). See [`ChamferParams::validate`] and SCHEMA §7.3 for the
    /// deterministic reference-face rule the worker applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance2: Option<Scalar>,
    pub edge_ids: Vec<ElementId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<ElementRef>,
    #[serde(default = "default_true")]
    pub chain_tangent_edges: bool,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl ChamferParams {
    /// SCHEMA §7.3 two-distance validity: when present, `distance2` must be a
    /// finite POSITIVE length. A zero/negative second leg is not a degenerate
    /// chamfer the kernel can round down — it is a param the worker would have to
    /// guess at, so it is refused here (the session is the single writer).
    ///
    /// `radius` itself is deliberately NOT range-checked here: the "too small"
    /// floor is the worker's `kMinValue` (1e-3) and is a RECOVERABLE `OP_FAILED`,
    /// not a rejected edit — moving it would change behaviour this WP does not own.
    pub fn validate(&self) -> Result<(), String> {
        let Some(d2) = self.distance2.as_ref() else {
            return Ok(());
        };
        if !d2.value.is_finite() || d2.value <= 0.0 {
            return Err(format!(
                "Chamfer distance2 must be a positive finite length (got {})",
                d2.value
            ));
        }
        Ok(())
    }
}

/// Shell parameters (OneCAD-CPP `ShellParams` `OperationRecord.h:122-125`).
/// `target_body` is the shelled body (C++ supplies it via the `BodyRef` input).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellParams {
    pub thickness: Scalar,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_faces: Vec<ElementId>,
    #[serde(
        rename = "targetBodyId",
        default,
        deserialize_with = "de_opt_body_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_body: Option<BodyId>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Standalone boolean parameters (OneCAD-CPP `BooleanParams`
/// `OperationRecord.h:127-132`; SCHEMA §7.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BooleanParams {
    pub operation: BooleanOp,
    #[serde(rename = "targetBodyId")]
    pub target_body: BodyId,
    #[serde(rename = "toolBodyId")]
    pub tool_body: BodyId,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Linear pattern parameters (OneCAD-CPP `LinearPatternParams`
/// `OperationRecord.h:134-142`).
///
/// Discrepancy: C++ stores the direction as flat `dirX/dirY/dirZ`; per the task
/// ("Vec3 for triples") the Rust port uses a single `direction: Vec3`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearPatternParams {
    #[serde(
        rename = "sourceBodyId",
        default,
        deserialize_with = "de_opt_body_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_body: Option<BodyId>,
    pub direction: Vec3,
    pub spacing: Scalar,
    pub count: u32,
    #[serde(default = "default_true")]
    pub fuse_result: bool,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Circular pattern parameters (OneCAD-CPP `CircularPatternParams`
/// `OperationRecord.h:171-182`).
///
/// Discrepancy: C++ stores flat `axisX/Y/Z` (point) and `axisDirX/Y/Z`; the Rust
/// port uses `axis_origin: Vec3` and `axis_direction: Vec3`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CircularPatternParams {
    #[serde(
        rename = "sourceBodyId",
        default,
        deserialize_with = "de_opt_body_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_body: Option<BodyId>,
    pub axis_origin: Vec3,
    pub axis_direction: Vec3,
    pub angle_deg: Scalar,
    pub count: u32,
    #[serde(default = "default_true")]
    pub fuse_result: bool,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Loft parameters (OneCAD-CPP `LoftParams` `OperationRecord.h:144-150`).
///
/// Discrepancy: C++ keeps parallel arrays `profileSketchIds` + `profileRegionIds`;
/// the Rust port pairs them into `profiles: Vec<SketchRegionRef>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoftParams {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<SketchRegionRef>,
    #[serde(default = "default_true")]
    pub is_solid: bool,
    #[serde(default)]
    pub is_ruled: bool,
    pub boolean_mode: BooleanMode,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Sweep parameters (OneCAD-CPP `SweepParams` `OperationRecord.h:152-158`).
/// `profile` pairs the C++ `profileSketchId`+`profileRegionId`; the path is a
/// sketch wire (`path_sketch`) or a body edge (`path_edge`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<SketchRegionRef>,
    #[serde(
        rename = "pathSketchId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub path_sketch: Option<SketchId>,
    #[serde(
        rename = "pathEdgeId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub path_edge: Option<ElementId>,
    pub boolean_mode: BooleanMode,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Mirror-body parameters (OneCAD-CPP `MirrorBodyParams`
/// `OperationRecord.h:160-169`).
///
/// Discrepancy: C++ stores flat `planePointX/Y/Z` + `planeNormalX/Y/Z`; the Rust
/// port uses `plane_point: Vec3` + `plane_normal: Vec3`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorBodyParams {
    #[serde(
        rename = "sourceBodyId",
        default,
        deserialize_with = "de_opt_body_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_body: Option<BodyId>,
    pub plane_point: Vec3,
    pub plane_normal: Vec3,
    #[serde(default)]
    pub fuse_with_original: bool,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// STEP/BREP import parameters (new v2 op; no OneCAD-CPP analogue).
///
/// **The params are a POINTER, never a payload.** They carry the content hash of
/// the source blob, not the blob: the bytes live in the container's authoritative
/// `imports/<sourceSha256>.<codec>` section (see
/// [`crate::io::imports`]). Three reasons this is not negotiable:
///
/// * `params` is hashed by
///   [`history_prefix_hash`](crate::regen::planner::history_prefix_hash) on every
///   plan compile — embedding megabytes of STEP would make each hash O(file).
/// * `params` round-trips through `document.json`; a 200 MB base64 blob inside
///   the authoritative JSON section would blow its 64 MB cap.
/// * A content hash is stable across machines; an absolute path is not. **No
///   filesystem path is stored** — `source_name` is a display-only basename and
///   is deliberately NOT used to locate anything.
///
/// Every field is geometry-relevant (all of them feed the hash), so re-importing
/// the same bytes under the same policy is a no-op edit, while a different file,
/// scale, or heal policy is a genuinely different operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStepParams {
    /// SHA-256 of the source blob, lowercase hex, 64 chars — the container key
    /// (`imports/<sourceSha256>.<codec>`) and the sole link to the bytes.
    pub source_sha256: String,
    /// Which codec the blob is stored in (picks the section extension).
    pub source_codec: ImportSourceCodec,
    /// Display-only source basename (tree label / diagnostics). **Never a path**
    /// and never resolved — see the type docs.
    pub source_name: String,
    /// The healing policy the import was performed under
    /// ([`IMPORT_HEAL_POLICY_V1`]).
    #[serde(default = "default_heal_policy")]
    pub heal_policy: String,
    /// Uniform scale applied to the imported geometry (1.0 = source units are
    /// already document units).
    #[serde(default = "default_unit_scale")]
    pub unit_scale: Scalar,
    /// Binary format version the converted blob was written with — the BinTools
    /// version for `Brep`, the OCAF storage version for `Xbf`. Present iff
    /// [`ImportSourceCodec::is_converted`], absent (skipped) otherwise, so a STEP
    /// import serializes without the key at all. Pinned because a kernel dump is
    /// only readable by a kernel that understands its format version. (The field
    /// name predates the `xbf` codec and is frozen — renaming it would move a
    /// `document.json` key for zero behavioural gain.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brep_format: Option<u32>,
    /// SHA-256 of the ORIGINAL user-supplied bytes when `source_sha256` points at
    /// a converted replay form (brep-primary policy: the record replays the brep,
    /// but the user's STEP file is kept co-stored as provenance — re-export,
    /// re-heal under a future policy, audit). Referenced here so the save-time
    /// refcount pins the provenance blob too; absent when the replayed blob IS
    /// the original.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_sha256: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl ImportStepParams {
    /// Validates the params' self-contained invariants, returning a human-facing
    /// reason on failure. Checked at every authoring entry point (see
    /// `crate::edit::session`), NOT at deserialize time: a document written by a
    /// future build must still load (and round-trip) rather than become
    /// unopenable.
    ///
    /// # Errors
    /// A message naming the violated invariant.
    pub fn validate(&self) -> Result<(), String> {
        if !is_sha256_hex(&self.source_sha256) {
            return Err(format!(
                "import sourceSha256 `{}` is not a 64-character lowercase-hex sha256",
                self.source_sha256
            ));
        }
        if !self.unit_scale.value.is_finite() || self.unit_scale.value <= 0.0 {
            return Err(format!(
                "import unitScale must be finite and > 0 (got {})",
                self.unit_scale.value
            ));
        }
        if self.heal_policy != IMPORT_HEAL_POLICY_V1 {
            return Err(format!(
                "unsupported import healPolicy `{}` (this build authors `{IMPORT_HEAL_POLICY_V1}`)",
                self.heal_policy
            ));
        }
        if let Some(p) = &self.provenance_sha256 {
            if !is_sha256_hex(p) {
                return Err(format!(
                    "import provenanceSha256 `{p}` is not a 64-character lowercase-hex sha256"
                ));
            }
            if *p == self.source_sha256 {
                return Err(
                    "import provenanceSha256 must differ from sourceSha256 (drop it when the \
                     replayed blob IS the original)"
                        .into(),
                );
            }
        }
        match (self.source_codec.is_converted(), self.brep_format) {
            (true, None) => Err(format!(
                "import sourceCodec `{}` requires a brepFormat (binary format version pin)",
                self.source_codec.extension()
            )),
            (false, Some(v)) => Err(format!(
                "import sourceCodec `step` must not carry a brepFormat (got {v})"
            )),
            _ => Ok(()),
        }
    }
}

/// The rotation half of a [`TransformBodyParams`] (SCHEMA §7.3 `rotate`).
///
/// `center` is the **frozen pivot**: it is captured once when the placement is
/// first authored (the targets' combined bbox centre at that moment) and is never
/// re-derived. Re-edits recompose against the stored pivot, so repeated edits are
/// exact (no drift) and the stored form stays canonical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformRotation {
    /// Frozen pivot point in world coordinates.
    pub center: Vec3,
    /// Rotation axis direction (need not be unit length; MUST be non-zero when
    /// `angle_deg != 0`).
    pub axis: Vec3,
    /// Rotation angle in **degrees** (the file/wire domain for this field —
    /// SCHEMA §7.3 `angleDeg`, matching Revolve/CircularPattern).
    pub angle_deg: Scalar,
}

impl Default for TransformRotation {
    /// The identity rotation about the world origin, +Z axis.
    fn default() -> Self {
        Self {
            center: Vec3::new_unchecked(0.0, 0.0, 0.0),
            axis: Vec3::new_unchecked(0.0, 0.0, 1.0),
            angle_deg: Scalar::new(0.0),
        }
    }
}

/// Rigid placement of one or more bodies (SCHEMA §7.3 `TransformBody`; new v2 op,
/// no OneCAD-CPP analogue). The light multi-part "position parts for fit-check"
/// primitive: parametric, ONE cumulative record per placement intent, re-edited in
/// place — never a stack of nudges.
///
/// **Evaluation order is normative**: `X' = T ∘ R(center, axis, angleDeg) · X` —
/// rotate about the frozen pivot FIRST, then translate. `R`-then-`T` and
/// `T`-then-`R` differ for any non-central pivot, so the order is pinned by
/// `transform_body.rs` / `test_transform_body`.
///
/// Lineage: `copy: false` ⇒ every target emits `modified` (BodyId preserved);
/// `copy: true` ⇒ the sources are untouched and the copies mint under the §2
/// N-body rule (one target ⇒ `body_<opId>`, N > 1 ⇒ `body_<opId>:<k>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformBodyParams {
    /// The bodies to place. Non-empty, no duplicates; mirrored by `inputs[]`.
    #[serde(default)]
    pub targets: Vec<BodyId>,
    /// World-space translation `[dx, dy, dz]`, applied AFTER the rotation.
    pub translate: [Scalar; 3],
    /// The rotation about the frozen pivot, applied FIRST.
    #[serde(default)]
    pub rotate: TransformRotation,
    /// `true` ⇒ the targets are preserved and the transformed shapes become NEW
    /// bodies; `false` ⇒ the targets are modified in place.
    #[serde(default)]
    pub copy: bool,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl TransformBodyParams {
    /// The rotation angle in degrees.
    #[must_use]
    pub fn angle_deg(&self) -> f64 {
        self.rotate.angle_deg.value
    }

    /// True iff this placement is the identity (zero translation, zero rotation).
    /// An identity transform is LEGAL and a geometric no-op (SCHEMA §7.3).
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.translate.iter().all(|s| s.value == 0.0) && self.angle_deg() == 0.0
    }

    /// Validates the params' self-contained invariants (SCHEMA §7.3), returning a
    /// human-facing reason on failure. Checked at every authoring entry point (see
    /// [`crate::edit::session`]), NOT at deserialize time: a document written by a
    /// future build must still load rather than become unopenable.
    ///
    /// # Errors
    /// A message naming the violated invariant.
    pub fn validate(&self) -> Result<(), String> {
        if self.targets.is_empty() {
            return Err("TransformBody requires at least one target body".into());
        }
        for (i, b) in self.targets.iter().enumerate() {
            if self.targets[..i].contains(b) {
                return Err(format!(
                    "TransformBody targets contain a duplicate body {b}"
                ));
            }
        }
        for (axis, s) in ["x", "y", "z"].iter().zip(self.translate.iter()) {
            if !s.value.is_finite() {
                return Err(format!(
                    "TransformBody translate.{axis} must be finite (got {})",
                    s.value
                ));
            }
        }
        let angle = self.angle_deg();
        if !angle.is_finite() {
            return Err(format!(
                "TransformBody rotate.angleDeg must be finite (got {angle})"
            ));
        }
        if !self.rotate.center.is_finite() {
            return Err("TransformBody rotate.center has a non-finite component".into());
        }
        if !self.rotate.axis.is_finite() {
            return Err("TransformBody rotate.axis has a non-finite component".into());
        }
        // A zero axis is only meaningful when there is no rotation to perform.
        let axis_len2 = self.rotate.axis.x * self.rotate.axis.x
            + self.rotate.axis.y * self.rotate.axis.y
            + self.rotate.axis.z * self.rotate.axis.z;
        if angle != 0.0 && axis_len2 <= 0.0 {
            return Err("TransformBody rotate.axis must be non-zero when angleDeg != 0".into());
        }
        Ok(())
    }
}

/// Machined hole on a planar face (SCHEMA §7.3 `Hole`, added 2026-08-03 —
/// WP-C T3; new v2 op, no OneCAD-CPP analogue). Simple / counterbore /
/// countersink are ONE parametric feature, not three ops: the profile is a
/// param, so switching a counterbore to a countersink is a param edit that keeps
/// the record's identity (and every downstream ref bound to it).
///
/// **Lineage is `modified` on [`target_body`](Self::target_body)** — a hole
/// mints nothing. The tool solid (drill cylinder + the conditional cb cylinder /
/// cs cone) is fused and cut from the host in one boolean.
///
/// `point` is the world-space hole centre, **frozen at authoring**. The worker
/// re-projects it onto the resolved face's plane every regen and fails loudly
/// (recoverable `OP_FAILED`) past 1e-3 mm — so a face that moved *within its own
/// plane* keeps the hole put, while a face that moved *out from under* the point
/// is a named failure rather than a hole drilled through empty space. The axis is
/// the face's INWARD normal (−outward) at `point`; it is never stored, because a
/// stored axis and a re-resolved face can disagree.
///
/// Standard-size tables (M-series clearance, SHCS counterbores, DIN 74
/// countersinks) are a **frontend** concern: these params always carry raw mm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoleParams {
    /// The host body the hole is machined into (SCHEMA `targetBodyId`).
    #[serde(rename = "targetBodyId")]
    pub target_body: BodyId,
    /// The planar host face the hole enters through — identity + descriptor +
    /// anchor evidence, resolved through the ladder (§10) like a Fillet edge.
    pub face: ElementRef,
    /// World-space hole centre, frozen at authoring (see the type docs).
    pub point: Vec3,
    pub hole_type: HoleType,
    /// Drill diameter in mm.
    pub diameter: Scalar,
    /// Blind depth in mm, or `None`/`null` = **through-all** (the worker extends
    /// the drill past the host's extent along the axis). Serialized explicitly as
    /// `null` rather than skipped: "through-all" is a deliberate authored choice,
    /// and an absent key would read as an omission.
    #[serde(default)]
    pub depth: Option<Scalar>,
    /// Counterbore diameter in mm. REQUIRED iff `hole_type == Counterbore`.
    #[serde(default)]
    pub cb_diameter: Option<Scalar>,
    /// Counterbore depth in mm (measured from the face inward). REQUIRED iff
    /// `hole_type == Counterbore`.
    #[serde(default)]
    pub cb_depth: Option<Scalar>,
    /// Countersink major (face-level) diameter in mm. REQUIRED iff
    /// `hole_type == Countersink`.
    #[serde(default)]
    pub cs_diameter: Option<Scalar>,
    /// Countersink INCLUDED angle in degrees, one of [`HOLE_CS_ANGLES_DEG`].
    /// REQUIRED iff `hole_type == Countersink`.
    #[serde(default)]
    pub cs_angle_deg: Option<Scalar>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl HoleParams {
    /// Validates the SCHEMA §7.3 `Hole` invariants, returning a human-facing
    /// reason on failure. Checked at every authoring entry point (see
    /// [`crate::edit::session`]), NOT at deserialize time, for the same
    /// single-writer reason as [`TransformBodyParams::validate`]: a document
    /// written by another build must still open and round-trip.
    ///
    /// The conditional blocks are checked **both ways** — a counterbore without
    /// `cb*` is rejected, and so is a *simple* hole carrying `cb*`. A stale
    /// conditional left behind by a profile switch would otherwise sit in the
    /// record invisibly and reappear the moment the profile switched back.
    ///
    /// # Errors
    /// A message naming the violated invariant.
    pub fn validate(&self) -> Result<(), String> {
        if !self.point.is_finite() {
            return Err("Hole point has a non-finite component".into());
        }
        positive("Hole diameter", self.diameter.value)?;
        if let Some(d) = &self.depth {
            positive("Hole depth", d.value)?;
        }
        match self.hole_type {
            HoleType::Simple => {
                self.reject_counterbore("simple")?;
                self.reject_countersink("simple")
            }
            HoleType::Counterbore => {
                self.reject_countersink("counterbore")?;
                let cb_d = require(self.cb_diameter.as_ref(), "cbDiameter", "counterbore")?;
                let cb_t = require(self.cb_depth.as_ref(), "cbDepth", "counterbore")?;
                positive("Hole cbDiameter", cb_d)?;
                positive("Hole cbDepth", cb_t)?;
                if cb_d <= self.diameter.value {
                    return Err(format!(
                        "Hole cbDiameter must exceed diameter (got {cb_d} <= {})",
                        self.diameter.value
                    ));
                }
                Ok(())
            }
            HoleType::Countersink => {
                self.reject_counterbore("countersink")?;
                let cs_d = require(self.cs_diameter.as_ref(), "csDiameter", "countersink")?;
                let cs_a = require(self.cs_angle_deg.as_ref(), "csAngleDeg", "countersink")?;
                positive("Hole csDiameter", cs_d)?;
                if cs_d <= self.diameter.value {
                    return Err(format!(
                        "Hole csDiameter must exceed diameter (got {cs_d} <= {})",
                        self.diameter.value
                    ));
                }
                if !HOLE_CS_ANGLES_DEG.contains(&cs_a) {
                    return Err(format!(
                        "Hole csAngleDeg must be one of {HOLE_CS_ANGLES_DEG:?} (got {cs_a})"
                    ));
                }
                Ok(())
            }
        }
    }

    fn reject_counterbore(&self, kind: &str) -> Result<(), String> {
        if self.cb_diameter.is_some() || self.cb_depth.is_some() {
            return Err(format!(
                "Hole cb* params are counterbore-only (got a {kind} hole)"
            ));
        }
        Ok(())
    }

    fn reject_countersink(&self, kind: &str) -> Result<(), String> {
        if self.cs_diameter.is_some() || self.cs_angle_deg.is_some() {
            return Err(format!(
                "Hole cs* params are countersink-only (got a {kind} hole)"
            ));
        }
        Ok(())
    }
}

/// `Some(value)` or a "`<field>` is required for a `<kind>` hole" message.
fn require(s: Option<&Scalar>, field: &str, kind: &str) -> Result<f64, String> {
    s.map(|s| s.value)
        .ok_or_else(|| format!("Hole {field} is required for a {kind} hole"))
}

/// Rejects a non-finite or non-positive dimension, naming the field.
fn positive(field: &str, v: f64) -> Result<(), String> {
    if !v.is_finite() || v <= 0.0 {
        return Err(format!(
            "{field} must be a positive finite length (got {v})"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canonical STEP-codec import (the shape the importer authors).
    fn step_params() -> ImportStepParams {
        ImportStepParams {
            source_sha256: "a".repeat(64),
            source_codec: ImportSourceCodec::Step,
            source_name: "bracket.step".into(),
            heal_policy: IMPORT_HEAL_POLICY_V1.into(),
            unit_scale: Scalar::new(1.0),
            brep_format: None,
            provenance_sha256: None,
            extra: Extra::new(),
        }
    }

    fn brep_params() -> ImportStepParams {
        ImportStepParams {
            source_codec: ImportSourceCodec::Brep,
            brep_format: Some(4),
            provenance_sha256: None,
            ..step_params()
        }
    }

    #[test]
    fn import_step_is_a_known_op_type() {
        assert!(KNOWN_OP_TYPES.contains(&"ImportStep"));
        let op = Operation::Known(KnownOperation::ImportStep(step_params()));
        assert_eq!(op.op_type(), "ImportStep");
        // The tag must round-trip through the KNOWN gate, not fall through to
        // Opaque (which would freeze imports out of regen forever).
        let json = serde_json::to_value(&op).unwrap();
        assert_eq!(json["opType"], serde_json::json!("ImportStep"));
        assert!(matches!(
            serde_json::from_value::<Operation>(json).unwrap(),
            Operation::Known(KnownOperation::ImportStep(_))
        ));
    }

    #[test]
    fn import_step_params_serialize_camel_case_and_round_trip() {
        let op = Operation::Known(KnownOperation::ImportStep(brep_params()));
        let json = serde_json::to_value(&op).unwrap();
        let params = &json["params"];
        assert_eq!(params["sourceSha256"], serde_json::json!("a".repeat(64)));
        assert_eq!(params["sourceCodec"], serde_json::json!("brep"));
        assert_eq!(params["sourceName"], serde_json::json!("bracket.step"));
        assert_eq!(params["healPolicy"], serde_json::json!("v1"));
        // `Scalar` serializes as `{value, expr?}` (bare numbers are accepted on
        // the way IN only) — same as every other dimensional param.
        assert_eq!(params["unitScale"], serde_json::json!({ "value": 1.0 }));
        assert_eq!(params["brepFormat"], serde_json::json!(4));

        let back: Operation = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), json);
        match back {
            Operation::Known(KnownOperation::ImportStep(p)) => assert_eq!(p, brep_params()),
            other => panic!("expected ImportStep, got {other:?}"),
        }
    }

    #[test]
    fn step_codec_omits_brep_format_entirely() {
        // skip-none: a STEP import must not grow a `brepFormat: null` key.
        let op = Operation::Known(KnownOperation::ImportStep(step_params()));
        let json = serde_json::to_value(&op).unwrap();
        assert!(json["params"].get("brepFormat").is_none());
    }

    #[test]
    fn import_params_defaults_fill_in_for_a_minimal_payload() {
        // A payload carrying only the three required keys still parses, with
        // healPolicy/unitScale defaulted (a future writer may omit them).
        let json = serde_json::json!({
            "opType": "ImportStep",
            "params": {
                "sourceSha256": "b".repeat(64),
                "sourceCodec": "step",
                "sourceName": "part.stp"
            }
        });
        match serde_json::from_value::<Operation>(json).unwrap() {
            Operation::Known(KnownOperation::ImportStep(p)) => {
                assert_eq!(p.heal_policy, IMPORT_HEAL_POLICY_V1);
                assert_eq!(p.unit_scale.value, 1.0);
                assert_eq!(p.brep_format, None);
                assert!(p.validate().is_ok());
            }
            other => panic!("expected ImportStep, got {other:?}"),
        }
    }

    #[test]
    fn import_params_are_hash_stable_across_identical_records() {
        // The planner hashes `params` verbatim (planner::history_prefix_hash), so
        // two independently-built identical records MUST serialize identically —
        // otherwise every plan compile would invalidate its own checkpoints.
        let a = serde_json::to_string(&KnownOperation::ImportStep(brep_params())).unwrap();
        let b = serde_json::to_string(&KnownOperation::ImportStep(brep_params())).unwrap();
        assert_eq!(a, b);

        let rec_a = OperationRecord::new(
            RecordId(Uuid::from_u128(0x1)),
            0,
            "Import 1",
            Operation::Known(KnownOperation::ImportStep(brep_params())),
        );
        let rec_b = OperationRecord::new(
            RecordId(Uuid::from_u128(0x1)),
            0,
            "Import 1",
            Operation::Known(KnownOperation::ImportStep(brep_params())),
        );
        assert_eq!(
            serde_json::to_string(&rec_a).unwrap(),
            serde_json::to_string(&rec_b).unwrap()
        );
    }

    #[test]
    fn import_params_carry_no_bytes_and_no_path() {
        // The params are a POINTER. Anything that could smuggle a payload or an
        // absolute path into `document.json` is a schema defect.
        let json = serde_json::to_value(KnownOperation::ImportStep(brep_params())).unwrap();
        let text = serde_json::to_string(&json).unwrap();
        assert!(!text.contains('/'), "no path component may appear: {text}");
        // Every params key is accounted for (no surprise payload field).
        let keys: Vec<&str> = json["params"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        // `serde_json::Map` is a BTreeMap here, so the listing is alphabetical.
        assert_eq!(
            keys,
            vec![
                "brepFormat",
                "healPolicy",
                "sourceCodec",
                "sourceName",
                "sourceSha256",
                "unitScale"
            ]
        );
    }

    #[test]
    fn import_step_derives_no_inputs() {
        let op = Operation::Known(KnownOperation::ImportStep(step_params()));
        let inputs = op.derive_inputs();
        assert!(inputs.bodies.is_empty());
        assert!(inputs.sketches.is_empty());
        assert!(inputs.elements.is_empty());
        assert_eq!(inputs, OperationInputs::default());
    }

    #[test]
    fn import_step_alien_params_keys_round_trip() {
        let mut json = serde_json::to_value(KnownOperation::ImportStep(step_params())).unwrap();
        json["params"]
            .as_object_mut()
            .unwrap()
            .insert("alienImportKey".into(), serde_json::json!({ "keep": 1 }));
        let back: Operation = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), json);
    }

    #[test]
    fn validate_rejects_malformed_sha256() {
        for bad in [
            "",
            "abc",
            &"A".repeat(64),    // uppercase hex is a second spelling
            &"g".repeat(64),    // non-hex
            "../../etc/passwd", // traversal attempt
            &format!("{}x", "a".repeat(63)),
        ] {
            let p = ImportStepParams {
                source_sha256: (*bad).to_string(),
                ..step_params()
            };
            assert!(p.validate().is_err(), "must reject sourceSha256 `{bad}`");
        }
        assert!(step_params().validate().is_ok());
    }

    #[test]
    fn validate_rejects_non_positive_or_non_finite_unit_scale() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let p = ImportStepParams {
                unit_scale: Scalar {
                    value: bad,
                    expr: None,
                },
                ..step_params()
            };
            assert!(p.validate().is_err(), "must reject unitScale {bad}");
        }
        let ok = ImportStepParams {
            unit_scale: Scalar::new(25.4),
            ..step_params()
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn validate_rejects_unknown_heal_policy_and_codec_format_disagreement() {
        let p = ImportStepParams {
            heal_policy: "v2".into(),
            ..step_params()
        };
        assert!(p.validate().is_err());

        // brep without a BinTools version pin is unreadable later.
        let p = ImportStepParams {
            brep_format: None,
            provenance_sha256: None,
            ..brep_params()
        };
        assert!(p.validate().is_err());

        // step with a version pin is a contradiction.
        let p = ImportStepParams {
            brep_format: Some(4),
            provenance_sha256: None,
            ..step_params()
        };
        assert!(p.validate().is_err());

        // `xbf` is a CONVERTED form too, so the same pin rule applies to it —
        // an OCAF document is only readable by a build that knows its storage
        // version, exactly like a BinTools dump.
        let p = ImportStepParams {
            source_codec: ImportSourceCodec::Xbf,
            brep_format: None,
            provenance_sha256: None,
            ..brep_params()
        };
        assert!(
            p.validate().is_err(),
            "an xbf record without a format pin must be rejected"
        );
        let p = ImportStepParams {
            source_codec: ImportSourceCodec::Xbf,
            brep_format: Some(12),
            provenance_sha256: None,
            ..brep_params()
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn xbf_codec_serializes_as_its_extension() {
        // The enum IS the `imports/<sha>.<ext>` registry, so the wire value, the
        // extension and the SCHEMA §7.8 `geometryCodec` string are one string.
        let params = ImportStepParams {
            source_codec: ImportSourceCodec::Xbf,
            brep_format: Some(12),
            provenance_sha256: None,
            ..brep_params()
        };
        let json = serde_json::to_value(KnownOperation::ImportStep(params.clone())).unwrap();
        assert_eq!(json["params"]["sourceCodec"], serde_json::json!("xbf"));
        assert_eq!(json["params"]["brepFormat"], serde_json::json!(12));
        assert_eq!(ImportSourceCodec::Xbf.extension(), "xbf");
        assert!(ImportSourceCodec::Xbf.is_converted());
        assert!(!ImportSourceCodec::Step.is_converted());

        let back: KnownOperation = serde_json::from_value(json).unwrap();
        match back {
            KnownOperation::ImportStep(p) => assert_eq!(p, params),
            other => panic!("expected ImportStep, got {other:?}"),
        }
    }

    #[test]
    fn codec_extension_registry_is_bijective() {
        for codec in [
            ImportSourceCodec::Step,
            ImportSourceCodec::Brep,
            ImportSourceCodec::Xbf,
        ] {
            assert_eq!(
                ImportSourceCodec::from_extension(codec.extension()),
                Some(codec)
            );
        }
        assert_eq!(ImportSourceCodec::from_extension("iges"), None);
        assert_eq!(ImportSourceCodec::from_extension("STEP"), None);
    }

    #[test]
    fn sha256_hex_shape_guard() {
        assert!(is_sha256_hex(&"0123456789abcdef".repeat(4)));
        assert!(!is_sha256_hex(&"0123456789ABCDEF".repeat(4)));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(&"a".repeat(65)));
        assert!(!is_sha256_hex("imports/../x"));
    }

    // ── TransformBody (WP-B W0; SCHEMA §7.3) ─────────────────────────────────

    fn body(n: u128) -> BodyId {
        BodyId(Uuid::from_u128(n))
    }

    fn transform_params() -> TransformBodyParams {
        TransformBodyParams {
            targets: vec![body(1), body(2)],
            translate: [Scalar::new(10.0), Scalar::new(0.0), Scalar::new(5.0)],
            rotate: TransformRotation {
                center: Vec3::new_unchecked(0.0, 0.0, 0.0),
                axis: Vec3::new_unchecked(0.0, 0.0, 1.0),
                angle_deg: Scalar::new(90.0),
            },
            copy: false,
            extra: Extra::new(),
        }
    }

    #[test]
    fn transform_body_is_a_known_op_type() {
        assert!(KNOWN_OP_TYPES.contains(&"TransformBody"));
        let op = Operation::Known(KnownOperation::TransformBody(transform_params()));
        assert_eq!(op.op_type(), "TransformBody");
        let json = serde_json::to_value(&op).unwrap();
        assert_eq!(json["opType"], serde_json::json!("TransformBody"));
        // Round-trips through the KNOWN gate (never demoted to Opaque).
        let back: Operation = serde_json::from_value(json).unwrap();
        assert_eq!(back, op);
    }

    /// The serialized shape must match the SCHEMA §7.3 example key-for-key
    /// (camelCase, `targets` as bare id strings, `rotate.angleDeg`, `copy`).
    #[test]
    fn transform_body_serializes_to_the_schema_shape() {
        let json = serde_json::to_value(KnownOperation::TransformBody(transform_params())).unwrap();
        let p = &json["params"];
        assert!(p["targets"].is_array() && p["targets"].as_array().unwrap().len() == 2);
        assert_eq!(p["targets"][0], serde_json::json!(body(1).to_string()));
        assert_eq!(p["translate"].as_array().unwrap().len(), 3);
        // Scalars normalize to the object form on write (SCHEMA §7.3).
        assert_eq!(p["translate"][0]["value"], serde_json::json!(10.0));
        assert_eq!(p["rotate"]["center"], serde_json::json!([0.0, 0.0, 0.0]));
        assert_eq!(p["rotate"]["axis"], serde_json::json!([0.0, 0.0, 1.0]));
        assert_eq!(p["rotate"]["angleDeg"]["value"], serde_json::json!(90.0));
        assert_eq!(p["copy"], serde_json::json!(false));
    }

    /// A hand-authored payload spelling every scalar as a BARE number must load
    /// (SCHEMA §7.3 "readers MUST accept both forms").
    #[test]
    fn transform_body_accepts_bare_number_scalars() {
        let raw = serde_json::json!({
            "opType": "TransformBody",
            "params": {
                "targets": [body(1).to_string()],
                "translate": [10.0, 0, 5.5],
                "rotate": { "center": [1.0, 2.0, 3.0], "axis": [0, 0, 1], "angleDeg": 45 },
                "copy": true
            }
        });
        let op: Operation = serde_json::from_value(raw).unwrap();
        let Operation::Known(KnownOperation::TransformBody(p)) = op else {
            panic!("expected TransformBody");
        };
        assert_eq!(p.translate[0].value, 10.0);
        assert_eq!(p.translate[2].value, 5.5);
        assert_eq!(p.angle_deg(), 45.0);
        assert!(p.copy);
        assert_eq!(p.rotate.center, Vec3::new_unchecked(1.0, 2.0, 3.0));
    }

    /// `inputs[]` MIRRORS `params.targets` (SCHEMA §7.3), in order and deduped.
    #[test]
    fn transform_body_derives_one_input_per_target() {
        let op = Operation::Known(KnownOperation::TransformBody(transform_params()));
        let inputs = op.derive_inputs();
        assert_eq!(inputs.bodies, vec![body(1), body(2)]);
        assert!(inputs.sketches.is_empty() && inputs.elements.is_empty());
        // `copy: true` still depends on its sources (the copies are rebuilt from them).
        let copy = Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
            copy: true,
            ..transform_params()
        }));
        assert_eq!(copy.derive_inputs().bodies, vec![body(1), body(2)]);
    }

    #[test]
    fn transform_body_validation() {
        // The canonical form is valid.
        assert!(transform_params().validate().is_ok());

        // An IDENTITY placement is LEGAL (SCHEMA §7.3) and reports itself as one.
        let identity = TransformBodyParams {
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: TransformRotation::default(),
            ..transform_params()
        };
        assert!(identity.validate().is_ok(), "identity is legal");
        assert!(identity.is_identity());
        assert!(!transform_params().is_identity());

        // Empty targets.
        let p = TransformBodyParams {
            targets: vec![],
            ..transform_params()
        };
        assert!(p.validate().unwrap_err().contains("at least one target"));

        // Duplicate target.
        let p = TransformBodyParams {
            targets: vec![body(1), body(1)],
            ..transform_params()
        };
        assert!(p.validate().unwrap_err().contains("duplicate"));

        // Zero axis with a NON-zero angle is rejected …
        let p = TransformBodyParams {
            rotate: TransformRotation {
                axis: Vec3::new_unchecked(0.0, 0.0, 0.0),
                ..transform_params().rotate
            },
            ..transform_params()
        };
        assert!(p.validate().unwrap_err().contains("non-zero when angleDeg"));

        // … but a zero axis with a ZERO angle is fine (nothing to rotate about).
        let p = TransformBodyParams {
            rotate: TransformRotation {
                axis: Vec3::new_unchecked(0.0, 0.0, 0.0),
                angle_deg: Scalar::new(0.0),
                ..transform_params().rotate
            },
            ..transform_params()
        };
        assert!(p.validate().is_ok());

        // Non-finite components.
        let p = TransformBodyParams {
            translate: [
                Scalar {
                    value: f64::NAN,
                    expr: None,
                },
                Scalar::new(0.0),
                Scalar::new(0.0),
            ],
            ..transform_params()
        };
        assert!(p.validate().unwrap_err().contains("finite"));
    }

    /// Unknown params keys ride through `extra` verbatim (no `deny_unknown_fields`).
    #[test]
    fn transform_body_preserves_unknown_params_keys() {
        let raw = serde_json::json!({
            "opType": "TransformBody",
            "params": {
                "targets": [body(1).to_string()],
                "translate": [1.0, 2.0, 3.0],
                "alienKey": { "future": true }
            }
        });
        let op: Operation = serde_json::from_value(raw.clone()).unwrap();
        let back = serde_json::to_value(&op).unwrap();
        assert_eq!(back["params"]["alienKey"], raw["params"]["alienKey"]);
    }

    // ── Hole (WP-C T3; SCHEMA §7.3, 2026-08-03) ──────────────────────────────

    fn hole_face_ref() -> ElementRef {
        ElementRef {
            primary: Some(crate::document::refs::PrimaryRef {
                body: body(1),
                element: ElementId::new("el_face_top"),
                kind: crate::document::refs::ElementKind::Face,
                extra: Extra::new(),
            }),
            intent: None,
            anchor: None,
            extra: Extra::new(),
        }
    }

    /// A canonical SIMPLE blind hole (the shape the frontend authors).
    fn hole_params() -> HoleParams {
        HoleParams {
            target_body: body(1),
            face: hole_face_ref(),
            point: Vec3::new_unchecked(25.0, 10.0, 30.0),
            hole_type: HoleType::Simple,
            diameter: Scalar::new(5.5),
            depth: Some(Scalar::new(20.0)),
            cb_diameter: None,
            cb_depth: None,
            cs_diameter: None,
            cs_angle_deg: None,
            extra: Extra::new(),
        }
    }

    fn counterbore_params() -> HoleParams {
        HoleParams {
            hole_type: HoleType::Counterbore,
            cb_diameter: Some(Scalar::new(9.5)),
            cb_depth: Some(Scalar::new(5.4)),
            ..hole_params()
        }
    }

    fn countersink_params() -> HoleParams {
        HoleParams {
            hole_type: HoleType::Countersink,
            cs_diameter: Some(Scalar::new(11.0)),
            cs_angle_deg: Some(Scalar::new(90.0)),
            ..hole_params()
        }
    }

    #[test]
    fn hole_is_a_known_op_type() {
        assert!(KNOWN_OP_TYPES.contains(&"Hole"));
        let op = Operation::Known(KnownOperation::Hole(hole_params()));
        assert_eq!(op.op_type(), "Hole");
        let json = serde_json::to_value(&op).unwrap();
        assert_eq!(json["opType"], serde_json::json!("Hole"));
        // Round-trips through the KNOWN gate (never demoted to Opaque).
        let back: Operation = serde_json::from_value(json).unwrap();
        assert_eq!(back, op);
    }

    /// The serialized shape must match the SCHEMA §7.3 `Hole` example key-for-key.
    #[test]
    fn hole_serializes_to_the_schema_shape() {
        let json = serde_json::to_value(KnownOperation::Hole(counterbore_params())).unwrap();
        let p = &json["params"];
        assert_eq!(p["targetBodyId"], serde_json::json!(body(1).to_string()));
        assert_eq!(p["point"], serde_json::json!([25.0, 10.0, 30.0]));
        assert_eq!(p["holeType"], serde_json::json!("counterbore"));
        // Scalars normalize to the object form on write (SCHEMA §7.3).
        assert_eq!(p["diameter"]["value"], serde_json::json!(5.5));
        assert_eq!(p["depth"]["value"], serde_json::json!(20.0));
        assert_eq!(p["cbDiameter"]["value"], serde_json::json!(9.5));
        assert_eq!(p["cbDepth"]["value"], serde_json::json!(5.4));
        // The inapplicable block renders as explicit `null`, matching the SCHEMA
        // example — "not a countersink" is authored, not omitted.
        assert!(p["csDiameter"].is_null() && p["csAngleDeg"].is_null());
        assert_eq!(p["face"]["primary"]["kind"], serde_json::json!("face"));
    }

    /// `depth: null` IS through-all, and survives a round-trip as `None`.
    #[test]
    fn hole_through_all_is_a_null_depth() {
        let raw = serde_json::json!({
            "opType": "Hole",
            "params": {
                "targetBodyId": body(1).to_string(),
                "face": { "primary": { "bodyId": body(1).to_string(),
                                       "elementId": "el_face_top", "kind": "face" } },
                "point": [1.0, 2.0, 3.0],
                "holeType": "simple",
                "diameter": 6.0,
                "depth": null
            }
        });
        let op: Operation = serde_json::from_value(raw).unwrap();
        let Operation::Known(KnownOperation::Hole(p)) = &op else {
            panic!("expected Hole");
        };
        assert!(p.depth.is_none(), "null depth = through-all");
        // A hand-authored BARE-number scalar loads (SCHEMA §7.3 both-forms rule).
        assert_eq!(p.diameter.value, 6.0);
        let back = serde_json::to_value(&op).unwrap();
        assert!(back["params"]["depth"].is_null());
        // An ABSENT depth key means the same thing (through-all), never an error.
        let mut raw2 = back.clone();
        raw2["params"].as_object_mut().unwrap().remove("depth");
        let op2: Operation = serde_json::from_value(raw2).unwrap();
        assert_eq!(op2, op);
    }

    /// `inputs[]` = host body + host face (SCHEMA §7.3).
    #[test]
    fn hole_derives_host_body_and_face_inputs() {
        let inputs = Operation::Known(KnownOperation::Hole(hole_params())).derive_inputs();
        assert_eq!(inputs.bodies, vec![body(1)]);
        assert_eq!(inputs.elements, vec![ElementId::new("el_face_top")]);
        assert!(inputs.sketches.is_empty());

        // An intent-only face ref contributes no element dep (the ladder binds it
        // at regen time) but the host body is still a dependency.
        let intent_only = HoleParams {
            face: ElementRef {
                primary: None,
                ..hole_face_ref()
            },
            ..hole_params()
        };
        let inputs = Operation::Known(KnownOperation::Hole(intent_only)).derive_inputs();
        assert_eq!(inputs.bodies, vec![body(1)]);
        assert!(inputs.elements.is_empty());
    }

    #[test]
    fn hole_validation_matrix() {
        // Every canonical profile is valid.
        assert!(hole_params().validate().is_ok());
        assert!(counterbore_params().validate().is_ok());
        assert!(countersink_params().validate().is_ok());
        // Through-all (no depth) is valid.
        assert!(HoleParams {
            depth: None,
            ..hole_params()
        }
        .validate()
        .is_ok());

        // ── dimensions ──
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let p = HoleParams {
                diameter: Scalar {
                    value: bad,
                    expr: None,
                },
                ..hole_params()
            };
            assert!(
                p.validate().unwrap_err().contains("Hole diameter"),
                "diameter {bad} must be rejected"
            );
            let p = HoleParams {
                depth: Some(Scalar {
                    value: bad,
                    expr: None,
                }),
                ..hole_params()
            };
            assert!(p.validate().unwrap_err().contains("Hole depth"));
        }
        let p = HoleParams {
            point: Vec3 {
                x: f64::NAN,
                y: 0.0,
                z: 0.0,
            },
            ..hole_params()
        };
        assert!(p.validate().unwrap_err().contains("non-finite"));

        // ── counterbore conditionals ──
        for missing in [
            HoleParams {
                cb_diameter: None,
                ..counterbore_params()
            },
            HoleParams {
                cb_depth: None,
                ..counterbore_params()
            },
        ] {
            assert!(missing
                .validate()
                .unwrap_err()
                .contains("required for a counterbore hole"));
        }
        // cbDiameter must EXCEED the drill diameter (equal is not a counterbore).
        for d in [5.5, 4.0] {
            let p = HoleParams {
                cb_diameter: Some(Scalar::new(d)),
                ..counterbore_params()
            };
            assert!(p.validate().unwrap_err().contains("cbDiameter must exceed"));
        }
        let p = HoleParams {
            cb_depth: Some(Scalar::new(0.0)),
            ..counterbore_params()
        };
        assert!(p.validate().unwrap_err().contains("Hole cbDepth"));

        // ── countersink conditionals ──
        for missing in [
            HoleParams {
                cs_diameter: None,
                ..countersink_params()
            },
            HoleParams {
                cs_angle_deg: None,
                ..countersink_params()
            },
        ] {
            assert!(missing
                .validate()
                .unwrap_err()
                .contains("required for a countersink hole"));
        }
        let p = HoleParams {
            cs_diameter: Some(Scalar::new(5.5)),
            ..countersink_params()
        };
        assert!(p.validate().unwrap_err().contains("csDiameter must exceed"));
        // Only the four standard included angles are admitted.
        for ok in HOLE_CS_ANGLES_DEG {
            let p = HoleParams {
                cs_angle_deg: Some(Scalar::new(ok)),
                ..countersink_params()
            };
            assert!(p.validate().is_ok(), "{ok}° is a standard angle");
        }
        for bad in [0.0, 60.0, 89.0, 91.0, 180.0] {
            let p = HoleParams {
                cs_angle_deg: Some(Scalar::new(bad)),
                ..countersink_params()
            };
            assert!(p
                .validate()
                .unwrap_err()
                .contains("csAngleDeg must be one of"));
        }

        // ── cross-profile leakage (both directions) ──
        let p = HoleParams {
            cb_diameter: Some(Scalar::new(9.5)),
            cb_depth: Some(Scalar::new(5.4)),
            ..hole_params()
        };
        assert!(p.validate().unwrap_err().contains("counterbore-only"));
        let p = HoleParams {
            cs_diameter: Some(Scalar::new(11.0)),
            ..hole_params()
        };
        assert!(p.validate().unwrap_err().contains("countersink-only"));
        // A counterbore must not carry cs* either (stale profile-switch residue).
        let p = HoleParams {
            cs_angle_deg: Some(Scalar::new(90.0)),
            ..counterbore_params()
        };
        assert!(p.validate().unwrap_err().contains("countersink-only"));
        let p = HoleParams {
            cb_depth: Some(Scalar::new(5.4)),
            ..countersink_params()
        };
        assert!(p.validate().unwrap_err().contains("counterbore-only"));
    }

    /// Unknown params keys ride through `extra` verbatim (no `deny_unknown_fields`).
    #[test]
    fn hole_preserves_unknown_params_keys() {
        let raw = serde_json::json!({
            "opType": "Hole",
            "params": {
                "targetBodyId": body(1).to_string(),
                "face": {},
                "point": [0.0, 0.0, 0.0],
                "holeType": "simple",
                "diameter": 3.0,
                "alienKey": { "future": true }
            }
        });
        let op: Operation = serde_json::from_value(raw.clone()).unwrap();
        let back = serde_json::to_value(&op).unwrap();
        assert_eq!(back["params"]["alienKey"], raw["params"]["alienKey"]);
    }
}
