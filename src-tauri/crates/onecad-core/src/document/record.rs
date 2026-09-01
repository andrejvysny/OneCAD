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

use std::collections::BTreeMap;

use serde::de::{self, DeserializeOwned};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::document::refs::{AxisRef, ElementKind, ElementRef, Extra, SketchRegionRef};
use crate::document::variables::Scalar;
use crate::expr::Dimension;
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
    "OffsetFace",
    "Gear",
    "PlaceComponent",
    "DetachComponent",
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
    OffsetFace(OffsetFaceParams),
    /// A generated gear body (Gear Generator G1, SCHEMA §7.3).
    /// Mints `body_<opId>`; no host body, no sketch.
    Gear(GearParams),
    /// Instantiate a library component (Component Library WP-0.2; spec §3.1).
    /// New v2 op, no OneCAD-CPP analogue.
    PlaceComponent(PlaceComponentParams),
    /// Drop a placed component's library identity, keeping its cached
    /// geometry as an ordinary body (Component Library WP-1.2; spec §3.4).
    /// New v2 op, no OneCAD-CPP analogue.
    DetachComponent(DetachComponentParams),
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
    #[must_use]
    pub fn element_refs_mut(&mut self) -> Vec<&mut ElementRef> {
        match self {
            KnownOperation::Fillet(p) => p.edges.iter_mut().collect(),
            KnownOperation::Chamfer(p) => p.edges.iter_mut().collect(),
            KnownOperation::Shell(p) => p.faces.iter_mut().collect(),
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
            // A gear's ONLY topological input is its placement face; a frame
            // placement contributes none.
            KnownOperation::Gear(p) => p.placement.face.iter_mut().collect(),
            // A typed body-edge Revolve axis is an `inputs[0]` semantic ref.
            // Legacy axes intentionally have no slot, preserving their payload.
            KnownOperation::Revolve(p) => match p.axis.as_mut() {
                Some(AxisRef::Element {
                    edge_ref: Some(edge_ref),
                    ..
                }) => vec![edge_ref],
                _ => Vec::new(),
            },
            // NORMATIVE slot order (SCHEMA §7.3 `op.offsetFace`): operative faces
            // in stored order, then the `Total` opposite face LAST when present.
            // `wire::wire_op_inputs`, `document_runtime::element_ref_input` and
            // [`crate::edit::command::InputPath`] all mirror this table; a
            // divergence is a silent mis-repair (H9).
            KnownOperation::OffsetFace(p) => {
                let mut refs: Vec<&mut ElementRef> = p.faces.iter_mut().collect();
                if let Some(o) = p.opposite_face.as_mut() {
                    refs.push(o);
                }
                refs
            }
            // A placed component's only topological input is its (optional) mate
            // target — the element it seats against. Absent `mate` ⇒ dropped in
            // free space, no ref at all.
            KnownOperation::PlaceComponent(p) => p
                .mate
                .as_mut()
                .map(|m| vec![&mut m.target])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Mutable access to every dimensional [`Scalar`] this op carries as a
    /// **parameter**, paired with its `opType.field` label for diagnostics and
    /// the [`Dimension`] that label is measured in.
    ///
    /// The variable-substitution pass
    /// ([`crate::regen::variables`]) walks exactly this set to replace an
    /// expr-bound scalar's cached `value` with the value its expression
    /// evaluates to, on an effective COPY of the record. Field order is fixed
    /// and deterministic — the derived write-back zips two records' lists by
    /// position, so an arm must never reorder its fields.
    ///
    /// **Adding a `Scalar` param means adding it here, WITH its dimension.** A
    /// field left out is silently un-drivable by a variable (it would keep its
    /// stale cached value forever), which is the exact defect this pass exists
    /// to remove. The dimension is the expression engine's call site: it is what
    /// makes `"45deg"` a loud refusal in a length field instead of a silent 45 mm,
    /// so a new scalar MUST declare one rather than defaulting to
    /// [`Dimension::Scalar`]. The same hand-maintained-table discipline (and
    /// hazard) as [`element_refs_mut`](Self::element_refs_mut).
    ///
    /// Canonical units per dimension are the expression engine's
    /// ([`crate::expr`]): millimetres for [`Dimension::Length`], **degrees** for
    /// [`Dimension::Angle`] (the op wire's angle convention — `Revolve.angleDeg`,
    /// `Chamfer.angleDeg`), unitless for [`Dimension::Scalar`].
    ///
    /// Ops with no dimensional parameter at all — `Sketch` (its entities and
    /// constraints are an opaque, already-solved wire snapshot; see
    /// `crate::sketch::constraint` and WP-VE.3), `Boolean`, `Loft`, `Sweep`,
    /// `MirrorBody`, `DetachComponent` — contribute nothing.
    #[must_use]
    pub fn scalars_mut(&mut self) -> Vec<(&'static str, Dimension, &mut Scalar)> {
        match self {
            KnownOperation::Extrude(p) => vec![
                ("Extrude.distance", Dimension::Length, &mut p.distance),
                (
                    "Extrude.draftAngleDeg",
                    Dimension::Angle,
                    &mut p.draft_angle_deg,
                ),
                ("Extrude.distance2", Dimension::Length, &mut p.distance2),
            ],
            KnownOperation::Revolve(p) => {
                vec![("Revolve.angleDeg", Dimension::Angle, &mut p.angle_deg)]
            }
            KnownOperation::Fillet(p) => {
                vec![("Fillet.radius", Dimension::Length, &mut p.radius)]
            }
            KnownOperation::Chamfer(p) => {
                let mut out = vec![("Chamfer.radius", Dimension::Length, &mut p.radius)];
                if let Some(d2) = p.distance2.as_mut() {
                    out.push(("Chamfer.distance2", Dimension::Length, d2));
                }
                if let Some(angle) = p.angle_deg.as_mut() {
                    out.push(("Chamfer.angleDeg", Dimension::Angle, angle));
                }
                out
            }
            KnownOperation::Shell(p) => {
                vec![("Shell.thickness", Dimension::Length, &mut p.thickness)]
            }
            KnownOperation::LinearPattern(p) => {
                vec![("LinearPattern.spacing", Dimension::Length, &mut p.spacing)]
            }
            KnownOperation::CircularPattern(p) => {
                vec![(
                    "CircularPattern.angleDeg",
                    Dimension::Angle,
                    &mut p.angle_deg,
                )]
            }
            // A unit scale is a pure ratio — the one genuinely dimensionless
            // registered scalar.
            KnownOperation::ImportStep(p) => {
                vec![("ImportStep.unitScale", Dimension::Scalar, &mut p.unit_scale)]
            }
            KnownOperation::TransformBody(p) => {
                let [tx, ty, tz] = &mut p.translate;
                vec![
                    ("TransformBody.translate[0]", Dimension::Length, tx),
                    ("TransformBody.translate[1]", Dimension::Length, ty),
                    ("TransformBody.translate[2]", Dimension::Length, tz),
                    (
                        "TransformBody.rotate.angleDeg",
                        Dimension::Angle,
                        &mut p.rotate.angle_deg,
                    ),
                ]
            }
            KnownOperation::Hole(p) => {
                let mut out = vec![("Hole.diameter", Dimension::Length, &mut p.diameter)];
                if let Some(s) = p.depth.as_mut() {
                    out.push(("Hole.depth", Dimension::Length, s));
                }
                if let Some(s) = p.cb_diameter.as_mut() {
                    out.push(("Hole.cbDiameter", Dimension::Length, s));
                }
                if let Some(s) = p.cb_depth.as_mut() {
                    out.push(("Hole.cbDepth", Dimension::Length, s));
                }
                if let Some(s) = p.cs_diameter.as_mut() {
                    out.push(("Hole.csDiameter", Dimension::Length, s));
                }
                if let Some(s) = p.cs_angle_deg.as_mut() {
                    out.push(("Hole.csAngleDeg", Dimension::Angle, s));
                }
                out
            }
            // Gear: the DIMENSIONS a variable may drive. `shift`, `clearance`,
            // `head` and `backlash` are deliberately absent — they are
            // dimensionless coefficients (multiples of module), not lengths,
            // so binding them to a length variable would be a category error.
            KnownOperation::Gear(p) => {
                let mut out = Vec::new();
                if let Some(inv) = p.involute_external.as_mut() {
                    out.push(("Gear.module", Dimension::Length, &mut inv.module));
                    out.push(("Gear.height", Dimension::Length, &mut inv.height));
                    out.push((
                        "Gear.pressureAngleDeg",
                        Dimension::Angle,
                        &mut inv.pressure_angle_deg,
                    ));
                    if let Some(s) = inv.axle_hole_diameter.as_mut() {
                        out.push(("Gear.axleHoleDiameter", Dimension::Length, s));
                    }
                    if let Some(s) = inv.offset_hole_diameter.as_mut() {
                        out.push(("Gear.offsetHoleDiameter", Dimension::Length, s));
                    }
                    if let Some(s) = inv.offset_hole_offset.as_mut() {
                        out.push(("Gear.offsetHoleOffset", Dimension::Length, s));
                    }
                }
                out
            }
            KnownOperation::OffsetFace(p) => {
                vec![("OffsetFace.distance", Dimension::Length, &mut p.distance)]
            }
            KnownOperation::PlaceComponent(p) => {
                let [tx, ty, tz] = &mut p.placement.translate;
                vec![
                    (
                        "PlaceComponent.placement.translate[0]",
                        Dimension::Length,
                        tx,
                    ),
                    (
                        "PlaceComponent.placement.translate[1]",
                        Dimension::Length,
                        ty,
                    ),
                    (
                        "PlaceComponent.placement.translate[2]",
                        Dimension::Length,
                        tz,
                    ),
                    (
                        "PlaceComponent.placement.rotate.angleDeg",
                        Dimension::Angle,
                        &mut p.placement.rotate.angle_deg,
                    ),
                ]
            }
            KnownOperation::Sketch(_)
            | KnownOperation::Boolean(_)
            | KnownOperation::Loft(_)
            | KnownOperation::Sweep(_)
            | KnownOperation::MirrorBody(_)
            | KnownOperation::DetachComponent(_) => Vec::new(),
        }
    }

    /// Drops the `expr` text of every **registered** [`Scalar`]
    /// ([`scalars_mut`](Self::scalars_mut)), leaving each `value` untouched.
    ///
    /// The canonical form both the planner hash
    /// ([`crate::regen::history_prefix_hash`]) and the OCW1 wire lowering are
    /// taken over: an expression is AUTHORING, not geometry. Two records that
    /// evaluate to the same number must hash the same and must send the worker
    /// the same params, whether the number was typed or computed — otherwise
    /// re-typing `20` where `w*2` stood would invalidate every checkpoint and
    /// rebuild identical geometry, and the worker would receive a string it has
    /// no way to evaluate.
    ///
    /// Always called on a CLONE: the stored record keeps its binding, which is
    /// the whole point of storing one.
    ///
    /// **Registered scalars only.** `KnownOperation::Sketch` carries its
    /// dimensional constraint values inside an opaque already-solved wire
    /// snapshot and exposes no scalars here; sketch-dimension expressions are
    /// WP-VE.3 and are neither substituted nor stripped by this pass.
    pub fn clear_scalar_exprs(&mut self) {
        for (_, _, scalar) in self.scalars_mut() {
            scalar.expr = None;
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
                KnownOperation::Gear(_) => "Gear",
                KnownOperation::OffsetFace(_) => "OffsetFace",
                KnownOperation::PlaceComponent(_) => "PlaceComponent",
                KnownOperation::DetachComponent(_) => "DetachComponent",
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
                    Some(AxisRef::Element {
                        edge_ref: Some(edge_ref),
                        ..
                    }) => {
                        // `edgeRef` wins for new records. A malformed typed ref
                        // contributes no legacy fallback: the worker refuses it.
                        if let Some(primary) = &edge_ref.primary {
                            inputs.push_body(primary.body);
                            inputs.push_element(primary.element.clone());
                        }
                    }
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

            // Gear: MINTS a body, so it has no host to depend on. Its only
            // topological input is the placement face, and only when the
            // placement is a face at all — a frame placement is frozen world
            // data and depends on nothing. An intent-only face ref contributes
            // no element dep and is bound by the ladder at regen time (the same
            // rule Hole and Fillet edge refs follow).
            KnownOperation::Gear(p) => {
                if let Some(face) = &p.placement.face {
                    if let Some(primary) = &face.primary {
                        inputs.push_body(primary.body);
                        inputs.push_element(primary.element.clone());
                    }
                }
            }

            // OffsetFace: the body it modifies IN PLACE, plus every operative face
            // (and the `Total` opposite face). No C++ analogue (new v2 op). The
            // body dep is UNCONDITIONAL — unlike Extrude/Shell, `target_body` is a
            // mandatory field, because an op that only ever modifies an existing
            // body has no "NewBody" reading in which the dependency is absent.
            KnownOperation::OffsetFace(p) => {
                inputs.push_body(p.target_body);
                for f in &p.face_ids {
                    inputs.push_element(f.clone());
                }
                for r in p.faces.iter().chain(p.opposite_face.iter()) {
                    if let Some(primary) = &r.primary {
                        inputs.push_body(primary.body);
                        inputs.push_element(primary.element.clone());
                    }
                }
                if let Some(id) = &p.opposite_face_id {
                    inputs.push_element(id.clone());
                }
            }

            // PlaceComponent: no host body (it mints a NewBody), so the only
            // dependency is a conditional mate target — mirrors Extrude's
            // ToFace-conditional handling of `target_face`. Absent `mate` ⇒ no
            // dependency at all (dropped in free space).
            KnownOperation::PlaceComponent(p) => {
                if let Some(m) = &p.mate {
                    if let Some(primary) = &m.target.primary {
                        inputs.push_body(primary.body);
                        inputs.push_element(primary.element.clone());
                    }
                }
            }

            // DetachComponent: no library identity, no mate — the record
            // re-describes geometry directly (same source+placement shape as
            // PlaceComponent, minus everything component-specific), so it has
            // no topological dependency at all. No C++ analogue (new v2 op).
            KnownOperation::DetachComponent(_) => {}
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

/// How [`OffsetFaceParams::distance`] is READ (SCHEMA §7.3 `distanceType` ∈
/// `Offset` | `Total` | `Radius` | `Diameter`). PascalCase wire values, like the
/// mode enums above.
///
/// The stored number is always the USER's value; the signed per-face kernel
/// offset is re-derived from current upstream geometry every regen, so "Ø8 stays
/// Ø8" across a parametric edit. Which types are legal for a given selection is
/// [`OffsetFaceParams::validate`]'s job — only [`Offset`](Self::Offset) admits a
/// multi-face set.
/// Smallest authored Offset delta the kernel contract can distinguish (mm).
pub const OFFSET_MIN_EFFECTIVE_CHANGE_MM: f64 = 1.0e-3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OffsetDistanceType {
    /// Signed delta along the topological outward normal (positive grows
    /// material). The only type valid for a multi-face selection.
    #[default]
    Offset,
    /// Absolute wall thickness measured against a persisted opposite face.
    Total,
    /// Absolute cylinder radius (cylindrical faces only).
    Radius,
    /// Absolute cylinder diameter (cylindrical faces only).
    Diameter,
}

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

fn de_optional_result_policy_version<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u8>, D::Error> {
    Option::<u8>::deserialize(deserializer)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tangent_closure_version: Option<u8>,
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
///
/// `angleDeg` is the CHAMFER-ONLY third mode (distance-angle), skip-none for the
/// same backward-compatibility reason. The mode is chosen by PRESENCE, never by
/// precedence: `angleDeg` present ⇒ distance-angle, else `distance2` present ⇒
/// two-distance, else equal-leg. Both present at once is refused by name in
/// [`ChamferParams::validate`] — the two describe the same second leg twice and
/// picking one would silently discard what the user typed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChamferParams {
    pub radius: Scalar,
    /// Distance on the NON-reference adjacent face. Absent ⇒ equal-leg (`radius`
    /// on both faces). See [`ChamferParams::validate`] and SCHEMA §7.3 for the
    /// deterministic reference-face rule the worker applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance2: Option<Scalar>,
    /// Angle in DEGREES between the chamfer face and the deterministic reference
    /// face (the adjacent face with the smaller resolved face ordinal — the same
    /// face `radius` is measured on, and the same rule `distance2` uses for its
    /// NON-reference partner), measured in the material. On a 90° dihedral,
    /// `angleDeg` 45 is the equal-leg chamfer. Absent ⇒ this is not a
    /// distance-angle chamfer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle_deg: Option<Scalar>,
    pub edge_ids: Vec<ElementId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<ElementRef>,
    #[serde(default = "default_true")]
    pub chain_tangent_edges: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tangent_closure_version: Option<u8>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl ChamferParams {
    /// SCHEMA §7.3 two-distance validity: when present, `distance2` must be a
    /// finite POSITIVE length. A zero/negative second leg is not a degenerate
    /// chamfer the kernel can round down — it is a param the worker would have to
    /// guess at, so it is refused here (the session is the single writer).
    ///
    /// SCHEMA §7.3 distance-angle validity: `distance2` and `angleDeg` may not both
    /// be present — they name the same second leg twice, and resolving that by
    /// precedence would silently discard one of the two numbers the user typed. The
    /// rejection NAMES both fields so the caller knows which to clear. When present
    /// alone, `angleDeg` must be finite and strictly between 0 and 180 degrees;
    /// either endpoint is a degenerate chamfer face (coplanar with the reference
    /// face, or folded back onto it) rather than a shape the kernel can build. The
    /// bound is deliberately LOOSE — whether a given angle actually fits the local
    /// dihedral is the worker's to decide, as a RECOVERABLE `OP_FAILED`.
    ///
    /// `radius` itself is deliberately NOT range-checked here: the "too small"
    /// floor is the worker's `kMinValue` (1e-3) and is a RECOVERABLE `OP_FAILED`,
    /// not a rejected edit — moving it would change behaviour this WP does not own.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(d2) = self.distance2.as_ref() {
            if self.angle_deg.is_some() {
                return Err(
                    "Chamfer may carry distance2 or angleDeg, not both (SCHEMA §7.3): \
                     clear one to pick the two-distance or the distance-angle mode"
                        .to_string(),
                );
            }
            if !d2.value.is_finite() || d2.value <= 0.0 {
                return Err(format!(
                    "Chamfer distance2 must be a positive finite length (got {})",
                    d2.value
                ));
            }
        }
        if let Some(angle) = self.angle_deg.as_ref() {
            if !angle.value.is_finite() || angle.value <= 0.0 || angle.value >= 180.0 {
                return Err(format!(
                    "Chamfer angleDeg must be a finite angle strictly between 0 and 180 degrees (got {})",
                    angle.value
                ));
            }
        }
        Ok(())
    }
}

/// Shell parameters (OneCAD-CPP `ShellParams` `OperationRecord.h:122-125`).
/// `target_body` is the shelled body (C++ supplies it via the `BodyRef` input).
///
/// `faces` is the typed home for each `open_faces` entry. New authored records
/// keep both lists in lockstep so descriptor/anchor evidence reaches the worker's
/// reference ladder. Empty remains valid for legacy bare-id records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellParams {
    pub thickness: Scalar,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_faces: Vec<ElementId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<ElementRef>,
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

impl ShellParams {
    /// Validates typed face evidence against its legacy id view and operated body.
    /// Empty `faces` remains the only legacy form.
    pub fn validate(&self) -> Result<(), String> {
        if self.faces.is_empty() {
            return Ok(());
        }
        if self.faces.len() != self.open_faces.len() {
            return Err(format!(
                "shell faces ({}) and openFaces ({}) length mismatch",
                self.faces.len(),
                self.open_faces.len()
            ));
        }
        let target = self
            .target_body
            .ok_or_else(|| "typed shell faces require targetBodyId".to_string())?;
        for (index, reference) in self.faces.iter().enumerate() {
            let primary = reference
                .primary
                .as_ref()
                .ok_or_else(|| format!("shell face {index} requires a FACE primary"))?;
            if primary.kind != ElementKind::Face {
                return Err(format!("shell face {index} requires a FACE primary"));
            }
            if primary.element != self.open_faces[index] {
                return Err(format!(
                    "shell face {index}: typed ref element {} != openFaces[{index}] {}",
                    primary.element, self.open_faces[index]
                ));
            }
            if primary.body != target {
                return Err(format!(
                    "shell face {index}: typed ref body {} != targetBodyId {target}",
                    primary.body
                ));
            }
        }
        Ok(())
    }
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
    /// Absent records retain v1 one-body output semantics. Version 2 is authored
    /// explicitly and makes child/body lineage part of the persisted contract.
    #[serde(
        default,
        deserialize_with = "de_optional_result_policy_version",
        skip_serializing_if = "Option::is_none"
    )]
    pub result_policy_version: Option<u8>,
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
    /// See [`LinearPatternParams::result_policy_version`].
    #[serde(
        default,
        deserialize_with = "de_optional_result_policy_version",
        skip_serializing_if = "Option::is_none"
    )]
    pub result_policy_version: Option<u8>,
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
pub const HOLE_RESULT_POLICY_VERSION: u8 = 2;

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
    /// Absent preserves the legacy split-host residual. Fresh authoring writes V2,
    /// which refuses any result that is not exactly one connected solid.
    #[serde(
        default,
        deserialize_with = "de_optional_result_policy_version",
        skip_serializing_if = "Option::is_none"
    )]
    pub result_policy_version: Option<u8>,
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
        let primary = self
            .face
            .primary
            .as_ref()
            .ok_or_else(|| "Hole face requires a FACE primary".to_string())?;
        if primary.kind != ElementKind::Face {
            return Err("Hole face requires a FACE primary".into());
        }
        if primary.body != self.target_body {
            return Err(format!(
                "Hole face body {} != targetBodyId {}",
                primary.body, self.target_body
            ));
        }
        if !self.point.is_finite() {
            return Err("Hole point has a non-finite component".into());
        }
        if let Some(version) = self.result_policy_version {
            if version != HOLE_RESULT_POLICY_VERSION {
                return Err(format!(
                    "Hole resultPolicyVersion {version} is unsupported (expected {HOLE_RESULT_POLICY_VERSION})"
                ));
            }
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

/// Which gear the [`GearParams`] recipe block describes (SCHEMA §7.3 `Gear`).
///
/// A typed enum, never a free-form string: ADR-0002 keeps the modeling kernel
/// closed, so a recipe is a variant the core team adds, not something an addon
/// registers. New recipes extend this enum and §7.3 together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GearRecipe {
    /// External involute gear — spur today, helical/herringbone once the
    /// sweep infrastructure lands (see [`InvoluteExternalParams::helix_angle_deg`]).
    InvoluteExternal,
}

/// Where a generated gear sits. **Exactly one of `face` / `frame` is set** —
/// the two placement modes are mutually exclusive and neither is optional, so
/// this is validated both ways rather than defaulted.
///
/// This mirrors [`HoleParams`]'s placement contract deliberately: a frozen
/// world point re-projected onto the resolved face every regen, fenced at
/// 1e-3 mm, with the axis taken as the face's INWARD normal. What a gear adds
/// is the second mode — a fully explicit frozen frame for datum/world
/// placement, which a hole never needs because a hole always has a host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GearPlacement {
    /// The planar face the gear is seated on. Resolved through the ladder
    /// (§10) like a Hole face. `None` ⇒ this is a frame placement.
    #[serde(default)]
    pub face: Option<ElementRef>,
    /// Explicit frozen frame for datum/world placement. `None` ⇒ face
    /// placement.
    #[serde(default)]
    pub frame: Option<GearFrame>,
    /// World-space gear centre, frozen at authoring. With a face this is
    /// re-projected onto the resolved plane each regen; with a frame it must
    /// equal [`GearFrame::origin`].
    pub point: Vec3,
}

/// An explicit, frozen placement frame. `x_dir` fixes the gear's angular
/// PHASE — carried rather than derived because tooth phasing is what makes a
/// pair of gears mesh, and a derived x-axis would silently rotate the teeth
/// when the axis changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GearFrame {
    pub origin: Vec3,
    /// The gear's rotation axis; the body grows along it.
    pub axis: Vec3,
    /// Reference direction in the gear's plane, fixing the angular phase.
    pub x_dir: Vec3,
}

/// External involute gear parameters (SCHEMA §7.3 `Gear.involuteExternal`).
///
/// `clearance`, `head` and `shift` are dimensionless COEFFICIENTS (multiples of
/// module) per gear-design convention, not lengths — hence plain `f64` rather
/// than [`Scalar`]: they are not dimensions a variable could drive, and giving
/// them expression slots would invite `clearance = someLength` which is a
/// category error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoluteExternalParams {
    /// Tooth count, >= 3.
    pub teeth: u32,
    /// Module in mm (the normal module when `properties_from_tool`).
    pub module: Scalar,
    /// Extrusion height in mm.
    pub height: Scalar,
    /// Pressure angle in DEGREES (the `Deg` suffix matches `Hole::cs_angle_deg`).
    pub pressure_angle_deg: Scalar,
    /// Profile shift coefficient x.
    #[serde(default)]
    pub shift: f64,
    /// Helix angle in degrees. **Must be 0 in this version** — helical gears
    /// need the Frenet sweep infrastructure; the field exists so the payload
    /// will not change shape when that lands (SCHEMA §7.3).
    #[serde(default)]
    pub helix_angle_deg: f64,
    /// Herringbone. **Must be false in this version**, same reason.
    #[serde(default)]
    pub double_helix: bool,
    /// Recompute transverse pressure angle / module from the normal (tool)
    /// values using the helix angle.
    #[serde(default)]
    pub properties_from_tool: bool,
    /// Trochoid root curve. Forces `root_fillet` to 0 where fillets exist.
    #[serde(default)]
    pub undercut: bool,
    /// Arc-length tooth thinning at the pitch circle, in mm.
    #[serde(default)]
    pub backlash: f64,
    /// Dedendum clearance coefficient (×module).
    pub clearance: f64,
    /// Extra addendum coefficient (×module).
    #[serde(default)]
    pub head: f64,
    /// Spline samples per curve segment — an ACCURACY knob that changes the
    /// body's topology, so it participates in the plan hash like any parameter.
    pub sample_count: u32,
    #[serde(default)]
    pub axle_hole: bool,
    #[serde(default)]
    pub axle_hole_diameter: Option<Scalar>,
    #[serde(default)]
    pub offset_hole: bool,
    #[serde(default)]
    pub offset_hole_diameter: Option<Scalar>,
    #[serde(default)]
    pub offset_hole_offset: Option<Scalar>,
}

/// A generated gear body (SCHEMA §7.3 `Gear`, Gear Generator G1).
///
/// **Lineage is a `NewBody` mint** — `body_<opId>` (D1), one `created` event,
/// an empty element-map delta. There is no target body: unlike every other
/// feature op, a gear has no host to modify, and its only topological input is
/// its placement.
///
/// The recipe blocks follow [`HoleParams`]'s conditional-block contract:
/// `recipe` selects which block is non-null, every inactive block is spelled
/// `null` rather than omitted, and [`Self::validate`] checks it **both ways**
/// so a stale block left behind by a recipe switch cannot sit in the record
/// invisibly and reappear later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GearParams {
    pub recipe: GearRecipe,
    pub placement: GearPlacement,
    /// Non-null iff `recipe == InvoluteExternal`.
    #[serde(default)]
    pub involute_external: Option<InvoluteExternalParams>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl GearParams {
    /// The active recipe block's parameters, when the record is well formed.
    #[must_use]
    pub fn involute(&self) -> Option<&InvoluteExternalParams> {
        match self.recipe {
            GearRecipe::InvoluteExternal => self.involute_external.as_ref(),
        }
    }

    /// Validates the SCHEMA §7.3 `Gear` invariants, returning a human-facing
    /// reason on failure. Checked at every authoring entry point (see
    /// [`crate::edit::session`]), NOT at deserialize time — a document written
    /// by another build must still open and round-trip, the same single-writer
    /// rule [`HoleParams::validate`] follows.
    ///
    /// # Errors
    /// Returns a human-facing reason when a recipe block is missing or stale,
    /// when the placement is not exactly one of face/frame, or when a
    /// dimension is outside its domain.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_placement()?;
        match self.recipe {
            GearRecipe::InvoluteExternal => {
                let p = self.involute_external.as_ref().ok_or_else(|| {
                    "Gear involuteExternal params are required for an involuteExternal recipe"
                        .to_string()
                })?;
                p.validate()
            }
        }
    }

    fn validate_placement(&self) -> Result<(), String> {
        let pl = &self.placement;
        match (&pl.face, &pl.frame) {
            (Some(_), Some(_)) => {
                return Err(
                    "Gear placement carries BOTH a face and a frame; exactly one is allowed".into(),
                )
            }
            (None, None) => {
                return Err(
                    "Gear placement carries neither a face nor a frame; exactly one is required"
                        .into(),
                )
            }
            _ => {}
        }
        if !pl.point.is_finite() {
            return Err("Gear placement point has a non-finite component".into());
        }
        if let Some(face) = &pl.face {
            if let Some(primary) = &face.primary {
                if primary.kind != ElementKind::Face {
                    return Err("Gear placement face requires a FACE primary".into());
                }
            }
        }
        if let Some(frame) = &pl.frame {
            if !frame.origin.is_finite() || !frame.axis.is_finite() || !frame.x_dir.is_finite() {
                return Err("Gear placement frame has a non-finite component".into());
            }
            if !is_direction(&frame.axis) {
                return Err("Gear placement frame axis is degenerate".into());
            }
            if !is_direction(&frame.x_dir) {
                return Err("Gear placement frame xDir is degenerate".into());
            }
            // The point IS the origin in frame mode; two sources of truth for
            // the same position would drift apart under a later edit.
            if !frame.origin.approx_eq(&pl.point, 1e-9) {
                return Err("Gear placement point must equal the frame origin".into());
            }
        }
        Ok(())
    }
}

/// True when `v` can serve as a direction — finite and not the zero vector.
/// A zero axis would make the gear's frame unbuildable in the worker, so it is
/// refused at authoring rather than discovered at regen.
fn is_direction(v: &Vec3) -> bool {
    v.is_finite() && (v.x != 0.0 || v.y != 0.0 || v.z != 0.0)
}

impl InvoluteExternalParams {
    /// # Errors
    /// Returns a human-facing reason when any dimension is outside its domain.
    pub fn validate(&self) -> Result<(), String> {
        if self.teeth < 3 {
            return Err(format!(
                "Gear teeth must be at least 3 (got {})",
                self.teeth
            ));
        }
        positive("Gear module", self.module.value)?;
        positive("Gear height", self.height.value)?;
        let alpha = self.pressure_angle_deg.value;
        if !(alpha > 0.0 && alpha < 90.0) {
            return Err(format!(
                "Gear pressureAngleDeg must be in (0, 90) (got {alpha})"
            ));
        }
        if self.sample_count < 2 {
            return Err(format!(
                "Gear sampleCount must be at least 2 (got {})",
                self.sample_count
            ));
        }
        if !self.shift.is_finite() || !self.backlash.is_finite() {
            return Err("Gear shift/backlash must be finite".into());
        }
        if self.backlash < 0.0 {
            return Err(format!(
                "Gear backlash must not be negative (got {})",
                self.backlash
            ));
        }
        if !self.clearance.is_finite() || self.clearance < 0.0 {
            return Err("Gear clearance must be finite and non-negative".into());
        }
        if !self.head.is_finite() {
            return Err("Gear head must be finite".into());
        }
        // Helical is UNSUPPORTED in this version (SCHEMA §7.3). Refused by
        // name rather than silently flattened to a spur gear, which would
        // hand back a body that is not what was asked for.
        if self.helix_angle_deg != 0.0 {
            return Err(
                "Gear helixAngleDeg must be 0 in this version (helical gears are not yet supported)"
                    .into(),
            );
        }
        if self.double_helix {
            return Err(
                "Gear doubleHelix must be false in this version (herringbone gears are not yet supported)"
                    .into(),
            );
        }
        // Bore blocks, checked BOTH ways so a stale dimension left by toggling
        // a bore off cannot reappear when it is toggled back on.
        Self::bore(
            self.axle_hole,
            self.axle_hole_diameter.as_ref(),
            "axleHole",
            "axleHoleDiameter",
        )?;
        Self::bore(
            self.offset_hole,
            self.offset_hole_diameter.as_ref(),
            "offsetHole",
            "offsetHoleDiameter",
        )?;
        if self.offset_hole {
            let off = self.offset_hole_offset.as_ref().ok_or_else(|| {
                "Gear offsetHoleOffset is required when offsetHole is on".to_string()
            })?;
            positive("Gear offsetHoleOffset", off.value)?;
        } else if self.offset_hole_offset.is_some() {
            return Err("Gear offsetHoleOffset is offsetHole-only (got offsetHole = false)".into());
        }
        Ok(())
    }

    fn bore(on: bool, dia: Option<&Scalar>, flag: &str, field: &str) -> Result<(), String> {
        if on {
            let d = dia.ok_or_else(|| format!("Gear {field} is required when {flag} is on"))?;
            positive(&format!("Gear {field}"), d.value)?;
        } else if dia.is_some() {
            return Err(format!("Gear {field} is {flag}-only (got {flag} = false)"));
        }
        Ok(())
    }
}

/// Direct-modeling face offset (SCHEMA §7.3 `op.offsetFace`, added 2026-08-06;
/// new v2 op, no OneCAD-CPP analogue — legacy deliberately removed face
/// push-pull). Selected faces move along their surface normals and the adjacent
/// faces extend/trim to re-close the solid.
///
/// **Lineage is `modified` on [`target_body`](Self::target_body)** — an offset
/// mints nothing and never fans a body out. That is why `target_body` is a
/// MANDATORY [`BodyId`] rather than [`ExtrudeParams`]'s optional form: there is
/// no NewBody reading of this op in which the field could legitimately be absent.
///
/// `face_ids` + `faces` are the Fillet dual (see [`FilletParams::edges`]): bare
/// ids matching SCHEMA's `faceIds`, plus one TYPED [`ElementRef`] per entry
/// carrying descriptor + anchor evidence so each face is repairable through the
/// ladder (§10) instead of guessed at. The two lists are kept in LOCKSTEP by
/// [`validate`](Self::validate) and by the repair write path
/// ([`InputPath::OffsetFaceFace`](crate::edit::command::InputPath::OffsetFaceFace)).
///
/// The operative set is the FULL FROZEN closure — the user's picks PLUS the G1
/// tangent chain, expanded ONCE at authoring by the `PrepareOffsetFace` handshake
/// (SCHEMA §7.6) and persisted. The worker never re-expands at regen, so an
/// upstream edit cannot silently widen or narrow what the op operates on;
/// `chain_tangent_faces` survives only as authoring metadata for re-edit UX.
///
/// Fresh authoring emits [`OFFSET_FACE_RESULT_POLICY_VERSION`] (3, the
/// blend-aware reinterpretation of the SAME stored pair). V2 stays executable
/// verbatim forever — it is a different geometric reading of an identical
/// payload, not a superseded encoding — so both values are accepted here under
/// one identical structural rule.
pub const OFFSET_FACE_RESULT_POLICY_VERSION: u8 = 3;

/// The V2 reading, still accepted: a stored record executes at the version it
/// was authored under and is NEVER auto-migrated (an in-place bump would change
/// the geometry an existing document rebuilds to).
pub const OFFSET_FACE_RESULT_POLICY_VERSION_V2: u8 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OffsetFaceParams {
    /// The operative faces as bare ids (SCHEMA `faceIds`), in slot order.
    pub face_ids: Vec<ElementId>,
    /// V2 user-picked design faces. Must be a non-empty subset of `face_ids`;
    /// dependent blend/support faces remain only in the frozen full closure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub primary_face_ids: Vec<ElementId>,
    /// The typed per-face semantic refs, one per [`face_ids`](Self::face_ids)
    /// entry in the SAME order. `default` so a SCHEMA §7.3 wire payload (which
    /// carries the refs in `inputs[]`, not in params) still parses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<ElementRef>,
    /// The USER's value, read per [`distance_type`](Self::distance_type).
    pub distance: Scalar,
    #[serde(default)]
    pub distance_type: OffsetDistanceType,
    /// Authoring metadata (see the type docs) — `true` by default.
    #[serde(default = "default_true")]
    pub chain_tangent_faces: bool,
    /// The `Total` opposite face as a bare id, mirroring
    /// [`opposite_face`](Self::opposite_face).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opposite_face_id: Option<ElementId>,
    /// The `Total` opposite face's typed ref — persisted at authoring and
    /// re-resolved VERBATIM each regen (never re-discovered, so an inserted wall
    /// cannot silently retarget the thickness).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opposite_face: Option<ElementRef>,
    /// The body this op modifies in place (SCHEMA `targetBodyId`).
    #[serde(rename = "targetBodyId")]
    pub target_body: BodyId,
    #[serde(
        default,
        deserialize_with = "de_optional_result_policy_version",
        skip_serializing_if = "Option::is_none"
    )]
    pub result_policy_version: Option<u8>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl OffsetFaceParams {
    /// Validates the SCHEMA §7.3 `op.offsetFace` invariants, returning a
    /// human-facing reason on failure. Checked at every authoring entry point
    /// (see [`crate::edit::session`]), NOT at deserialize time — same
    /// single-writer reason as [`HoleParams::validate`].
    ///
    /// Nothing here is ever CLAMPED. A clamped value desynchronizes the stored
    /// param, the preview the user approved and the geometry the next regen
    /// builds; an out-of-domain value is refused at the boundary instead.
    ///
    /// The conditional block is checked BOTH ways (the [`HoleParams`] doctrine):
    /// an opposite face is required for `Total` **and** rejected for every other
    /// type, so a stale opposite left behind by a distance-type switch cannot ride
    /// the record invisibly and resurrect when the type switches back.
    ///
    /// # Errors
    /// A message naming the violated invariant.
    pub fn validate(&self) -> Result<(), String> {
        if self.faces.is_empty() {
            return Err("OffsetFace requires at least one operative face".into());
        }
        if self.faces.len() != self.face_ids.len() {
            return Err(format!(
                "OffsetFace faces ({}) and faceIds ({}) length mismatch",
                self.faces.len(),
                self.face_ids.len()
            ));
        }
        for (i, f) in self.faces.iter().enumerate() {
            let primary = f
                .primary
                .as_ref()
                .ok_or_else(|| format!("OffsetFace face {i} requires a FACE primary"))?;
            if primary.kind != ElementKind::Face {
                return Err(format!("OffsetFace face {i} requires a FACE primary"));
            }
            if primary.element != self.face_ids[i] {
                return Err(format!(
                    "OffsetFace face {i}: typed ref element {} != faceIds[{i}] {}",
                    primary.element, self.face_ids[i]
                ));
            }
            if primary.body != self.target_body {
                return Err(format!(
                    "OffsetFace face {i}: typed ref body {} != targetBodyId {}",
                    primary.body, self.target_body
                ));
            }
        }
        match self.result_policy_version {
            None if !self.primary_face_ids.is_empty() => {
                return Err("OffsetFace primaryFaceIds requires resultPolicyVersion 2 or 3".into())
            }
            // ONE arm, deliberately: V3 reinterprets the SAME payload (see the
            // constant's docs), so a structural rule that held for V2 must hold
            // identically for V3 or the two readings would not be interchangeable
            // inputs to the same record.
            Some(OFFSET_FACE_RESULT_POLICY_VERSION_V2 | OFFSET_FACE_RESULT_POLICY_VERSION) => {
                if self.primary_face_ids.is_empty() {
                    return Err("OffsetFace V2 requires at least one primary face".into());
                }
                for (index, id) in self.primary_face_ids.iter().enumerate() {
                    if !self.face_ids.contains(id) {
                        return Err(format!(
                            "OffsetFace primaryFaceIds[{index}] {id} is not in the frozen faceIds closure"
                        ));
                    }
                    if self.primary_face_ids[..index].contains(id) {
                        return Err(format!(
                            "OffsetFace primaryFaceIds contains duplicate {id}"
                        ));
                    }
                }
            }
            Some(version) => {
                return Err(format!(
                    "OffsetFace resultPolicyVersion {version} is unsupported (expected {OFFSET_FACE_RESULT_POLICY_VERSION_V2} or {OFFSET_FACE_RESULT_POLICY_VERSION})"
                ))
            }
            None => {}
        }
        if !self.distance.value.is_finite() {
            return Err(format!(
                "OffsetFace distance must be finite (got {})",
                self.distance.value
            ));
        }
        if self.distance_type != OffsetDistanceType::Offset && self.faces.len() != 1 {
            return Err(format!(
                "OffsetFace distanceType {:?} operates on exactly one face (got {})",
                self.distance_type,
                self.faces.len()
            ));
        }
        self.validate_opposite()?;
        match self.distance_type {
            OffsetDistanceType::Total => {
                if self.chain_tangent_faces {
                    return Err(
                        "OffsetFace distanceType Total requires chainTangentFaces = false".into(),
                    );
                }
            }
            OffsetDistanceType::Radius | OffsetDistanceType::Diameter => {
                if self.distance.value <= 0.0 {
                    return Err(format!(
                        "OffsetFace distanceType {:?} needs a positive distance (got {})",
                        self.distance_type, self.distance.value
                    ));
                }
            }
            OffsetDistanceType::Offset => {
                if self.distance.value.abs() < OFFSET_MIN_EFFECTIVE_CHANGE_MM {
                    return Err(format!(
                        "OffsetFace Offset distance magnitude must be at least {} mm (got {})",
                        OFFSET_MIN_EFFECTIVE_CHANGE_MM, self.distance.value
                    ));
                }
            }
        }
        Ok(())
    }

    /// The opposite face is present **iff** `distanceType == Total`, and its bare
    /// id mirrors its typed ref (the same lockstep `faceIds`/`faces` hold).
    fn validate_opposite(&self) -> Result<(), String> {
        let total = self.distance_type == OffsetDistanceType::Total;
        match (&self.opposite_face, total) {
            (None, true) => {
                return Err("OffsetFace distanceType Total requires an oppositeFace".into())
            }
            (Some(_), false) => {
                return Err(format!(
                    "OffsetFace oppositeFace is Total-only (got a {:?} offset)",
                    self.distance_type
                ))
            }
            _ => {}
        }
        match (&self.opposite_face, &self.opposite_face_id) {
            (Some(_), None) | (None, Some(_)) => {
                Err("OffsetFace oppositeFace and oppositeFaceId must be set together".into())
            }
            (Some(r), Some(id)) => {
                let primary = r
                    .primary
                    .as_ref()
                    .ok_or_else(|| "OffsetFace oppositeFace requires a FACE primary".to_string())?;
                if primary.kind != ElementKind::Face {
                    return Err("OffsetFace oppositeFace requires a FACE primary".into());
                }
                if &primary.element != id {
                    return Err(format!(
                        "OffsetFace oppositeFace element {} != oppositeFaceId {id}",
                        primary.element
                    ));
                }
                if primary.body != self.target_body {
                    return Err(format!(
                        "OffsetFace oppositeFace body {} != targetBodyId {}",
                        primary.body, self.target_body
                    ));
                }
                Ok(())
            }
            (None, None) => Ok(()),
        }
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

// ─────────────────────────────────────────────────────────────────────────────
// PlaceComponent (Component Library WP-0.2; spec §3.1)
// ─────────────────────────────────────────────────────────────────────────────

/// A free-parameter override value for a placed component instance (spec §3.1
/// `params`, mirroring `component.toml`'s `[parameters]` scalar domain: a
/// thread designation is text, a length is a number, a thread-detail level is
/// text). Untagged: the wire form is a bare JSON scalar, matching how
/// `component.toml` authors these values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ComponentParamValue {
    Number(f64),
    Text(String),
    Bool(bool),
}

/// Where a placed component's geometry comes from (spec §3.1 `source`).
///
/// The three spec §2.1 kinds as of WP-3.2, plus WP-C's [`Self::Profile`].
///
/// `Embedded` and `Document` both resolve to **the same thing at regen time** — a baked solid, content
/// addressed, cached in this document's own
/// [`imports`](crate::io::imports) section — and differ only in what the
/// record remembers about where that solid came from. That is deliberate: a
/// document must render with the library folder deleted (spec §4/§12), so
/// nothing on the regen path may depend on re-deriving geometry from a
/// package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ComponentSourceRef {
    /// Resolves via a built-in, versioned generator running through the
    /// worker op path (spec §6) — no blob, no library-folder dependency once
    /// authored. This is what makes P0's spike self-contained: unlike
    /// `Embedded`, there is no document-side blob copy-in to plumb.
    Generator {
        #[serde(rename = "generatorId")]
        generator_id: String,
        #[serde(rename = "generatorVersion")]
        generator_version: u32,
        /// The RESOLVED parameter set the generator ran with (free values
        /// merged with table/computed derivations) — frozen at authoring so
        /// regen never re-touches the library for this instance.
        #[serde(default)]
        params: BTreeMap<String, ComponentParamValue>,
        #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
        extra: Extra,
    },
    /// A content-addressed blob payload (spec §2.3 `<library-root>/blobs/<sha>`
    /// at authoring; copied into the DOCUMENT's own `io::imports::ImportBlobs`
    /// section at place time, WP-1.3) — reuses [`ImportSourceCodec`] verbatim,
    /// the same "params carry only a pointer, never bytes/paths" discipline
    /// [`ImportStepParams`] follows.
    Embedded {
        sha256: String,
        codec: ImportSourceCodec,
        /// The binary format version `sha256`'s bytes were written in, for the
        /// CONVERTED codecs (`brep` / `xbf`) — the same pin
        /// [`ImportStepParams::brep_format`] carries, validated by the worker
        /// against the version it writes today. `None` for `step`, which is a
        /// text format with no such pin.
        ///
        /// `#[serde(default)]`: records authored before WP-3.2 carry no
        /// `embedded` source at all (the kind never reached the worker), so this
        /// is additive with nothing to migrate.
        #[serde(
            rename = "brepFormat",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        brep_format: Option<u32>,
        #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
        extra: Extra,
    },
    /// A user-authored component: a frozen `.onecad` document in the package
    /// (spec §2.1 `[source] kind = "document"`), placed as the **baked solid**
    /// that document produces.
    ///
    /// `document_sha256` is identity/provenance — the frozen document the
    /// package carried at place time. It is deliberately NOT what regen reads:
    /// replaying a nested document would need a second worker session (the
    /// worker is one-session-per-process) and the library folder to still
    /// exist, and both of those are exactly what spec §4's "geometry is always
    /// cached locally" rules out. Regen reads `sha256` — the baked blob in this
    /// document's own `imports/` section — and nothing else.
    ///
    /// It is recorded because it is the key the RE-BAKE needs: re-resolving
    /// free-parameter overrides against the authoring document (spec §3.1's
    /// "replays the document with overrides") requires knowing WHICH document
    /// the baked bytes came from. `setComponentParams` (WP-F1.3) replays that
    /// document on its own ephemeral worker, bakes the solid the new values
    /// produce, and swaps `sha256` for it — so an override always moves
    /// geometry, never just a designation string.
    Document {
        /// sha256 of the package's frozen `source.onecad` at place time.
        #[serde(rename = "documentSha256")]
        document_sha256: String,
        /// The baked geometry blob, in this document's `imports/` section.
        sha256: String,
        codec: ImportSourceCodec,
        /// Format pin for `sha256`'s bytes — see [`Self::Embedded::brep_format`].
        #[serde(
            rename = "brepFormat",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        brep_format: Option<u32>,
        /// Free-parameter overrides recorded for this instance — the variable
        /// values `sha256`'s bytes were re-baked at (WP-F1.3). Provenance and
        /// UI state, not a regen input: the geometry they produced is already
        /// in `sha256`, so the worker never reads them.
        #[serde(default)]
        params: BTreeMap<String, ComponentParamValue>,
        #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
        extra: Extra,
    },
    /// A **length-parametric extrusion of an embedded planar profile**
    /// (Component Library WP-C; SCHEMA §7.3).
    ///
    /// It exists because vendor stock arrives as ONE fixed length — an
    /// aluminium extrusion STEP is a 500 mm stick — while the component has to
    /// be placeable at any length, and none of the other three kinds could
    /// express that: `embedded`/`document` are baked SOLIDS by definition, and
    /// `generator` has no blob at all. So the blob here is a single canonical
    /// planar FACE and the worker builds the solid by prism
    /// (`BRepPrimAPI_MakePrism(face, gp_Vec(0, 0, length))`).
    ///
    /// **`params.length` is a REGEN INPUT** — the one place this kind diverges
    /// from [`Self::Document`], whose `params` the worker still ignores. It is
    /// covered by the planner hash, so editing it MOVES geometry rather than
    /// relabelling it, and it needs no re-bake (there is no authoring document
    /// to replay).
    Profile {
        /// Content address of the canonical face blob, in this document's own
        /// `imports/` section — the same pointer discipline
        /// [`Self::Embedded`] follows.
        sha256: String,
        /// MUST be [`ImportSourceCodec::Brep`]: the `step` and `xbf` readers on
        /// this lane return SOLIDS, so accepting them would answer a face
        /// question with a solid reader and fail obscurely.
        codec: ImportSourceCodec,
        /// Format pin for `sha256`'s bytes — see [`Self::Embedded::brep_format`].
        #[serde(
            rename = "brepFormat",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        brep_format: Option<u32>,
        /// The extrusion parameters. `length` (millimetres) is REQUIRED,
        /// finite, and `0 < length ≤ 1e5` — see
        /// [`validate_component_source`].
        #[serde(default)]
        params: BTreeMap<String, ComponentParamValue>,
        #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
        extra: Extra,
    },
}

/// The `params` key a [`ComponentSourceRef::Profile`] extrudes along, in
/// millimetres (SCHEMA §7.3). Named so [`validate_component_source`] and the
/// tests that pin it cannot drift apart, and public because it is part of the
/// kind's authored contract — a package declaring some other key for its length
/// produces a record this validator refuses.
pub const PROFILE_LENGTH_PARAM: &str = "length";

/// Upper bound on a profile extrusion's length, in millimetres.
const PROFILE_MAX_LENGTH_MM: f64 = 1e5;

/// The baked-blob pointer an `embedded` / `document` / `profile` component
/// source carries: everything the regen path needs, with the provenance
/// differences stripped.
///
/// Exists so the three blob-backed kinds are handled ONCE — wire lowering and
/// the save-time blob refcount both walk this, not a per-variant match, which is
/// what keeps a future fifth kind from silently missing one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentBlobRef<'a> {
    /// The blob's content address, keying this document's `imports/` section.
    pub sha256: &'a str,
    /// The byte form those bytes are in.
    pub codec: ImportSourceCodec,
    /// Binary format pin for the converted codecs.
    pub brep_format: Option<u32>,
}

impl ComponentSourceRef {
    /// The baked blob this source resolves through, or `None` for a `generator`
    /// source (which re-runs from params and depends on no bytes at all).
    #[must_use]
    pub fn blob_ref(&self) -> Option<ComponentBlobRef<'_>> {
        match self {
            ComponentSourceRef::Generator { .. } => None,
            ComponentSourceRef::Embedded {
                sha256,
                codec,
                brep_format,
                ..
            }
            | ComponentSourceRef::Document {
                sha256,
                codec,
                brep_format,
                ..
            }
            | ComponentSourceRef::Profile {
                sha256,
                codec,
                brep_format,
                ..
            } => Some(ComponentBlobRef {
                sha256,
                codec: *codec,
                brep_format: *brep_format,
            }),
        }
    }
}

/// The snap classification a placement mate resolved to (spec §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MateKind {
    /// Axis aligned to a cylinder/hole axis, seated at the near end.
    Concentric,
    /// Mating face coplanar, normal-aligned, at the pick point.
    Coincident,
    /// Both in one gesture — the preferred fastener-on-a-hole-rim snap.
    ConcentricAndCoincident,
}

/// The component-local basis a mate seats FROM, FROZEN into the record at
/// authoring out of the package's `[attachments].<key>.frame` (spec §2.1;
/// Component Library WP-F1.1).
///
/// Frozen, not looked up on regen, for the same reason `source` is: a
/// placement must re-seat identically with the library folder deleted (spec
/// §4). A later package revision that moves its attachment does NOT silently
/// move already-placed instances — that is an explicit `replaceComponent`.
///
/// Right-handed and orthonormal by construction: `y` is derived (`z × x`) and
/// never stored, and `onecad-library`'s manifest parse normalizes both axes
/// before this is minted. `origin` is the component-local point that lands on
/// the target's seat point; `z` is the direction aligned to the target
/// axis/normal; `x` fixes only the roll about `z`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MateFrame {
    pub origin: Vec3,
    pub z: Vec3,
    pub x: Vec3,
}

/// A placed component's recorded attachment to the document (spec §3.1
/// `mate`). Absent on the owning [`PlaceComponentParams`] ⇒ dropped in free
/// space, positioned by `placement` alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentMate {
    /// Names a key in the component's `[attachments]` table — the
    /// component's own local frame the mate seats from.
    pub self_attachment: String,
    /// The target element in the document. A full [`ElementRef`] (not a bare
    /// id) so the resolution ladder can re-resolve it after upstream edits —
    /// this is what makes the mate PERSISTENT (spec §5.5, landing P3).
    pub target: ElementRef,
    pub kind: MateKind,
    /// Orientation flip at insert (the `A`-key gesture), applied on top of
    /// the solved frame.
    #[serde(default)]
    pub flipped: bool,
    /// The component-local frame [`self_attachment`](Self::self_attachment)
    /// declared, frozen at authoring (WP-F1.1). ABSENT ⇒ the identity frame,
    /// i.e. the component seats at its own model origin — every document
    /// written before WP-F1.1 loads and re-seats byte-identically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_frame: Option<MateFrame>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// A placed component's frozen world placement (spec §3.1 `placement`).
///
/// Reuses [`TransformRotation`] verbatim — same frozen-pivot semantics
/// (`TransformBodyParams` docs): re-edits recompose against the stored pivot
/// so repeated edits are exact. When `mate` is present this is the RESOLVED
/// transform the mate produced, kept as the fallback if the mate later fails
/// to resolve (never dropped, never silently substituted).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenPlacement {
    pub translate: [Scalar; 3],
    #[serde(default)]
    pub rotate: TransformRotation,
}

impl FrozenPlacement {
    /// Drops every `expr` binding on the four placement scalars, keeping their
    /// numbers.
    ///
    /// **The detach seam calls this; nothing else should.** A detached
    /// component is INERT PROVENANCE (spec §3.4): it records where the instance
    /// came from and where it sat, and deliberately stops tracking anything.
    /// `DetachComponentParams` exposes no drivable scalars
    /// ([`KnownOperation::scalars_mut`]), so a binding carried across a detach
    /// would be worse than useless — it would never be substituted again (the
    /// number silently frozen at whatever it was), it would survive a
    /// `rename_variable` as a dangling reference, and because the strip that
    /// keeps an expression out of the planner hash and off the OCW1 wire is
    /// registry-driven, it would reach BOTH. Clearing it here is what makes
    /// "inert" true rather than merely intended.
    pub fn clear_bindings(&mut self) {
        for scalar in &mut self.translate {
            scalar.expr = None;
        }
        self.rotate.angle_deg.expr = None;
    }
}

/// Instantiate a library component (spec §3.1 `PlaceComponent`; Component
/// Library WP-0.2; new v2 op, no OneCAD-CPP analogue).
///
/// **Lineage: mints a NewBody** (`body_<opId>`), `modified` on nothing — a
/// placed component is a first-class instance, never a copied-in body (spec
/// §3, the decision that keeps the library aligned with the founding
/// `NeedsRepair`-over-silent-substitution invariant).
///
/// A component resolves to exactly ONE solid in v1 (spec §9,
/// `single_solid_policy` — the existing single-solid publication policy every
/// other publishing op already satisfies).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceComponentParams {
    /// Library identity the instance was placed from (namespaced, e.g.
    /// `"onecad.std.iso4762"`).
    pub component_id: String,
    /// Semver at place time.
    pub component_version: String,
    /// `"sha256:…"` content hash of the package at place time — the revision
    /// re-verified on regen (spec §4); a mismatch surfaces `NeedsRepair`,
    /// never a silent substitution (the SolidWorks Toolbox failure mode).
    pub component_revision: String,
    /// Free-parameter overrides for THIS instance. Keys must exist in the
    /// component signature and be `role: free`; enforced at authoring, not
    /// deserialize (see `crate::edit::session`) — structurally here, and
    /// against the resolved component signature at the app-crate entry point
    /// (`onecad-core` cannot depend on the library crate).
    #[serde(default)]
    pub params: BTreeMap<String, ComponentParamValue>,
    /// Source resolution, so regen can re-derive geometry without the
    /// library (P0/WP-0.2: `Generator` only).
    pub source: ComponentSourceRef,
    /// Optional placement mate recorded at insert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate: Option<ComponentMate>,
    /// Frozen world placement, frozen at authoring.
    pub placement: FrozenPlacement,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl PlaceComponentParams {
    /// Validates the params' self-contained invariants (spec §3.1), returning
    /// a human-facing reason on failure. Checked at every authoring entry
    /// point (see [`crate::edit::session`]), NOT at deserialize time — same
    /// single-writer reason as [`ImportStepParams::validate`].
    ///
    /// # Errors
    /// A message naming the violated invariant.
    pub fn validate(&self) -> Result<(), String> {
        if self.component_id.trim().is_empty() {
            return Err("PlaceComponent componentId must not be empty".into());
        }
        if !self.component_id.contains('.') {
            return Err(format!(
                "PlaceComponent componentId `{}` must be namespaced (`<ns>.<...>`)",
                self.component_id
            ));
        }
        // The id/version pair is also a PATH component at the library layer
        // (`<root>/<id>@<version>`) — keep this charset in lockstep with
        // `onecad-library::package::validate_identity` (no shared dep by design).
        if !self.component_id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        }) || self.component_id.split('.').any(str::is_empty)
        {
            return Err(format!(
                "PlaceComponent componentId `{}` may only contain [a-z0-9._-] with \
                 non-empty dot segments (it names the package directory)",
                self.component_id
            ));
        }
        if self.component_version.trim().is_empty() {
            return Err("PlaceComponent componentVersion must not be empty".into());
        }
        if !self
            .component_version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
        {
            return Err(format!(
                "PlaceComponent componentVersion `{}` may only contain [0-9A-Za-z.+-] \
                 (it names the package directory)",
                self.component_version
            ));
        }
        match self.component_revision.strip_prefix("sha256:") {
            Some(hex) if is_sha256_hex(hex) => {}
            _ => {
                return Err(format!(
                "PlaceComponent componentRevision `{}` must be `sha256:` + 64 lowercase-hex chars",
                self.component_revision
            ))
            }
        }
        validate_component_source(&self.source, "PlaceComponent")?;
        if let Some(mate) = &self.mate {
            if mate.self_attachment.trim().is_empty() {
                return Err("PlaceComponent mate.selfAttachment must not be empty".into());
            }
            if let Some(frame) = &mate.self_frame {
                for (field, v) in [("origin", frame.origin), ("z", frame.z), ("x", frame.x)] {
                    if !v.is_finite() {
                        return Err(format!(
                            "PlaceComponent mate.selfFrame.{field} must be finite"
                        ));
                    }
                }
            }
        }
        for (axis, s) in ["x", "y", "z"].iter().zip(self.placement.translate.iter()) {
            if !s.value.is_finite() {
                return Err(format!(
                    "PlaceComponent placement.translate.{axis} must be finite (got {})",
                    s.value
                ));
            }
        }
        if !self.placement.rotate.angle_deg.value.is_finite() {
            return Err(format!(
                "PlaceComponent placement.rotate.angleDeg must be finite (got {})",
                self.placement.rotate.angle_deg.value
            ));
        }
        if !self.placement.rotate.center.is_finite() {
            return Err("PlaceComponent placement.rotate.center has a non-finite component".into());
        }
        if !self.placement.rotate.axis.is_finite() {
            return Err("PlaceComponent placement.rotate.axis has a non-finite component".into());
        }
        Ok(())
    }
}

/// Shared `source` self-consistency check for [`PlaceComponentParams`] and
/// [`DetachComponentParams`] — both carry the same [`ComponentSourceRef`]
/// shape. `op_name` names the caller in the error message.
fn validate_component_source(source: &ComponentSourceRef, op_name: &str) -> Result<(), String> {
    if let ComponentSourceRef::Generator { generator_id, .. } = source {
        return if generator_id.trim().is_empty() {
            Err(format!("{op_name} source.generatorId must not be empty"))
        } else {
            Ok(())
        };
    }
    if let ComponentSourceRef::Document {
        document_sha256, ..
    } = source
    {
        if !is_sha256_hex(document_sha256) {
            return Err(format!(
                "{op_name} source.documentSha256 `{document_sha256}` is not a 64-character \
                 lowercase-hex sha256"
            ));
        }
    }
    // Both blob-backed kinds share the pointer rules: the blob names a document
    // `imports/` entry, and a CONVERTED codec must pin the binary format version
    // its bytes were written in (the worker refuses a record pinned to a version
    // it does not write — see SCHEMA §7.3 `ImportStep.brepFormat`). A `step`
    // source is text and carries no such pin.
    let Some(blob) = source.blob_ref() else {
        return Ok(());
    };
    if !is_sha256_hex(blob.sha256) {
        return Err(format!(
            "{op_name} source.sha256 `{}` is not a 64-character lowercase-hex sha256",
            blob.sha256
        ));
    }
    match blob.codec {
        ImportSourceCodec::Step => {
            // A `profile` blob is a FACE, and only the `brep` reader returns one
            // (below). Every other kind accepts `step`.
            if matches!(source, ComponentSourceRef::Profile { .. }) {
                return Err(profile_codec_error(op_name, blob.codec));
            }
            Ok(())
        }
        ImportSourceCodec::Brep | ImportSourceCodec::Xbf if blob.brep_format.is_none() => {
            Err(format!(
                "{op_name} source.brepFormat is required for sourceCodec `{}`",
                blob.codec.extension()
            ))
        }
        ImportSourceCodec::Xbf if matches!(source, ComponentSourceRef::Profile { .. }) => {
            Err(profile_codec_error(op_name, blob.codec))
        }
        ImportSourceCodec::Brep | ImportSourceCodec::Xbf => Ok(()),
    }?;
    // `profile` alone reads `params` as a REGEN INPUT, so its one required key
    // is checked here rather than left to the worker: an out-of-range length
    // authored into a record would otherwise only fail at the next regen, on
    // someone else's machine.
    if let ComponentSourceRef::Profile { params, .. } = source {
        let length = match params.get(PROFILE_LENGTH_PARAM) {
            Some(ComponentParamValue::Number(l)) => *l,
            Some(_) => {
                return Err(format!(
                    "{op_name} source.params.{PROFILE_LENGTH_PARAM} must be a number"
                ))
            }
            None => {
                return Err(format!(
                    "{op_name} source.params.{PROFILE_LENGTH_PARAM} is required for a `profile` \
                     source (it is the extrusion distance, in millimetres)"
                ))
            }
        };
        if !length.is_finite() || length <= 0.0 || length > PROFILE_MAX_LENGTH_MM {
            return Err(format!(
                "{op_name} source.params.{PROFILE_LENGTH_PARAM} must be finite and in \
                 (0, {PROFILE_MAX_LENGTH_MM}] mm (got {length})"
            ));
        }
    }
    Ok(())
}

fn profile_codec_error(op_name: &str, codec: ImportSourceCodec) -> String {
    format!(
        "{op_name} source.codec must be `brep` for a `profile` source (got `{}`) — the \
         other readers return solids, and this blob is a single planar face",
        codec.extension()
    )
}

/// Drop a placed component's library identity, keeping its cached geometry as
/// an ordinary body (spec §3.4 `DetachComponent`; Component Library WP-1.2;
/// new v2 op, no OneCAD-CPP analogue).
///
/// **The "honest break link."** Same `source`+`placement` shape as
/// [`PlaceComponentParams`] (so regen still has enough to rebuild the exact
/// geometry — a `generator` source re-runs deterministically; the result is
/// indistinguishable from a static copy), but carries NO `component_id`/
/// `component_version`/`component_revision`/`mate` — spec §3.4: "after
/// detach, no `component_*` fields remain; the op becomes inert provenance."
/// This is an in-place edit at the SAME `RecordId` (same trick `Hole`'s
/// profile-mode switch uses to keep identity), not a new record — applied via
/// `crate::edit::session`'s existing `update_operation_params`, which already
/// supports an op-TYPE change at one `RecordId` (the Fillet⇄Chamfer
/// precedent). No `KnownOperation::ReplaceComponent`/`SetComponentParams`
/// variants exist for the same reason: both are ALSO in-place edits of
/// `PlaceComponentParams`'s own fields (identity/params) at the same
/// `RecordId`, not distinct persisted op shapes — only `DetachComponent`'s
/// shape is genuinely different (it drops fields `PlaceComponentParams`
/// requires), which is what earns it a real variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachComponentParams {
    pub source: ComponentSourceRef,
    pub placement: FrozenPlacement,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl DetachComponentParams {
    /// Validates the params' self-contained invariants, returning a
    /// human-facing reason on failure. Checked at every authoring entry
    /// point (see [`crate::edit::session`]), NOT at deserialize time — same
    /// single-writer reason as [`PlaceComponentParams::validate`].
    ///
    /// # Errors
    /// A message naming the violated invariant.
    pub fn validate(&self) -> Result<(), String> {
        validate_component_source(&self.source, "DetachComponent")?;
        for (axis, s) in ["x", "y", "z"].iter().zip(self.placement.translate.iter()) {
            if !s.value.is_finite() {
                return Err(format!(
                    "DetachComponent placement.translate.{axis} must be finite (got {})",
                    s.value
                ));
            }
        }
        if !self.placement.rotate.angle_deg.value.is_finite() {
            return Err(format!(
                "DetachComponent placement.rotate.angleDeg must be finite (got {})",
                self.placement.rotate.angle_deg.value
            ));
        }
        if !self.placement.rotate.center.is_finite() {
            return Err(
                "DetachComponent placement.rotate.center has a non-finite component".into(),
            );
        }
        if !self.placement.rotate.axis.is_finite() {
            return Err("DetachComponent placement.rotate.axis has a non-finite component".into());
        }
        Ok(())
    }
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
            result_policy_version: Some(HOLE_RESULT_POLICY_VERSION),
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
        assert_eq!(p["resultPolicyVersion"], serde_json::json!(2));
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

        let p = HoleParams {
            face: ElementRef {
                primary: None,
                ..hole_face_ref()
            },
            ..hole_params()
        };
        assert!(p.validate().unwrap_err().contains("FACE primary"));
        let mut foreign_face = hole_face_ref();
        foreign_face.primary.as_mut().unwrap().body = body(2);
        let p = HoleParams {
            face: foreign_face,
            ..hole_params()
        };
        assert!(p.validate().unwrap_err().contains("!= targetBodyId"));

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

    #[test]
    fn hole_result_policy_absence_and_future_values_round_trip() {
        let mut legacy = hole_params();
        legacy.result_policy_version = None;
        assert!(legacy.validate().is_ok());
        let legacy_json =
            serde_json::to_value(Operation::Known(KnownOperation::Hole(legacy))).unwrap();
        assert!(legacy_json["params"].get("resultPolicyVersion").is_none());

        let mut future =
            serde_json::to_value(Operation::Known(KnownOperation::Hole(hole_params()))).unwrap();
        future["params"]["resultPolicyVersion"] = serde_json::json!(3);
        let parsed: Operation = serde_json::from_value(future.clone()).unwrap();
        let Operation::Known(KnownOperation::Hole(params)) = &parsed else {
            panic!("expected Hole");
        };
        assert!(params.validate().unwrap_err().contains("unsupported"));
        assert_eq!(
            serde_json::to_value(parsed).unwrap()["params"]["resultPolicyVersion"],
            future["params"]["resultPolicyVersion"]
        );
    }

    #[test]
    fn shell_typed_faces_serialize_and_legacy_empty_round_trips() {
        let legacy = serde_json::json!({
            "opType": "Shell",
            "params": {
                "thickness": 2.0,
                "openFaces": ["el_f1"],
                "targetBodyId": body(1).to_string()
            }
        });
        let legacy_op: Operation = serde_json::from_value(legacy).unwrap();
        let legacy_json = serde_json::to_value(&legacy_op).unwrap();
        assert!(legacy_json["params"].get("faces").is_none());

        let mut typed = KnownOperation::Shell(ShellParams {
            thickness: Scalar::new(2.0),
            open_faces: vec![ElementId::new("el_f1")],
            faces: vec![offset_ref("el_f1")],
            target_body: Some(body(1)),
            extra: Extra::new(),
        });
        assert_eq!(typed.element_refs_mut().len(), 1);
        let json = serde_json::to_value(Operation::Known(typed)).unwrap();
        assert!(json["params"]["faces"].is_array());
    }

    // ── OffsetFace (SCHEMA §7.3 `op.offsetFace`, 2026-08-06) ────────────────

    fn offset_ref(el: &str) -> ElementRef {
        ElementRef {
            primary: Some(crate::document::refs::PrimaryRef {
                body: body(1),
                element: ElementId::new(el),
                kind: crate::document::refs::ElementKind::Face,
                extra: Extra::new(),
            }),
            intent: None,
            anchor: None,
            extra: Extra::new(),
        }
    }

    /// A canonical single-face `Offset` push-pull.
    fn offset_params() -> OffsetFaceParams {
        OffsetFaceParams {
            face_ids: vec![ElementId::new("el_f1")],
            primary_face_ids: vec![ElementId::new("el_f1")],
            faces: vec![offset_ref("el_f1")],
            distance: Scalar::new(2.5),
            distance_type: OffsetDistanceType::Offset,
            chain_tangent_faces: true,
            opposite_face_id: None,
            opposite_face: None,
            target_body: body(1),
            result_policy_version: Some(OFFSET_FACE_RESULT_POLICY_VERSION),
            extra: Extra::new(),
        }
    }

    fn offset_total_params() -> OffsetFaceParams {
        OffsetFaceParams {
            distance_type: OffsetDistanceType::Total,
            chain_tangent_faces: false,
            opposite_face_id: Some(ElementId::new("el_f9")),
            opposite_face: Some(offset_ref("el_f9")),
            ..offset_params()
        }
    }

    #[test]
    fn offset_face_is_a_known_op_type() {
        assert!(KNOWN_OP_TYPES.contains(&"OffsetFace"));
        let op = Operation::Known(KnownOperation::OffsetFace(offset_params()));
        assert_eq!(op.op_type(), "OffsetFace");
        let json = serde_json::to_value(&op).unwrap();
        assert_eq!(json["opType"], serde_json::json!("OffsetFace"));
        // Round-trips through the KNOWN gate (never demoted to Opaque).
        let back: Operation = serde_json::from_value(json).unwrap();
        assert!(matches!(
            back,
            Operation::Known(KnownOperation::OffsetFace(_))
        ));
    }

    /// The SCHEMA §7.3 wire spelling of every field, plus the two defaults.
    #[test]
    fn offset_face_params_round_trip_with_schema_field_names() {
        let op = Operation::Known(KnownOperation::OffsetFace(offset_total_params()));
        let json = serde_json::to_value(&op).unwrap();
        let params = &json["params"];
        assert_eq!(params["faceIds"], serde_json::json!(["el_f1"]));
        assert_eq!(params["distance"], serde_json::json!({ "value": 2.5 }));
        assert_eq!(params["distanceType"], serde_json::json!("Total"));
        assert_eq!(params["chainTangentFaces"], serde_json::json!(false));
        assert_eq!(params["oppositeFaceId"], serde_json::json!("el_f9"));
        assert_eq!(
            params["targetBodyId"],
            serde_json::json!(body(1).to_string())
        );
        assert_eq!(
            serde_json::from_value::<Operation>(json).unwrap(),
            op,
            "byte-stable round-trip"
        );

        // Skip-none: an `Offset` push-pull grows NO opposite-face keys.
        let plain = serde_json::to_value(Operation::Known(KnownOperation::OffsetFace(
            offset_params(),
        )))
        .unwrap();
        assert!(plain["params"].get("oppositeFaceId").is_none());
        assert!(plain["params"].get("oppositeFace").is_none());
    }

    /// Unknown params keys ride through `extra` verbatim (no `deny_unknown_fields`),
    /// and the two defaulted fields fill in when the payload omits them.
    #[test]
    fn offset_face_preserves_unknown_params_keys_and_defaults() {
        let raw = serde_json::json!({
            "opType": "OffsetFace",
            "params": {
                "faceIds": ["el_f1"],
                "distance": 2.5,
                "targetBodyId": body(1).to_string(),
                "alienKey": { "future": true }
            }
        });
        let op: Operation = serde_json::from_value(raw.clone()).unwrap();
        let Operation::Known(KnownOperation::OffsetFace(p)) = &op else {
            panic!("expected OffsetFace");
        };
        assert_eq!(p.distance_type, OffsetDistanceType::Offset);
        assert!(p.chain_tangent_faces, "chainTangentFaces defaults to TRUE");
        assert!(p.faces.is_empty());
        let back = serde_json::to_value(&op).unwrap();
        assert_eq!(back["params"]["alienKey"], raw["params"]["alienKey"]);
    }

    /// The full SCHEMA §7.3 rejection matrix. Nothing is ever clamped — an
    /// out-of-domain value is refused so the stored param, the approved preview
    /// and the next regen can never disagree.
    #[test]
    fn offset_face_validate_matrix() {
        assert!(offset_params().validate().is_ok());
        assert!(offset_total_params().validate().is_ok());

        // ── the operative set ──
        let p = OffsetFaceParams {
            face_ids: vec![],
            faces: vec![],
            ..offset_params()
        };
        assert!(p.validate().unwrap_err().contains("at least one"));
        let p = OffsetFaceParams {
            face_ids: vec![ElementId::new("el_f1"), ElementId::new("el_f2")],
            ..offset_params()
        };
        assert!(p.validate().unwrap_err().contains("length mismatch"));
        let p = OffsetFaceParams {
            face_ids: vec![ElementId::new("el_other")],
            ..offset_params()
        };
        assert!(p.validate().unwrap_err().contains("!= faceIds[0]"));
        let p = OffsetFaceParams {
            faces: vec![ElementRef {
                primary: None,
                ..offset_ref("el_f1")
            }],
            ..offset_params()
        };
        assert!(p
            .validate()
            .unwrap_err()
            .contains("requires a FACE primary"));
        let mut foreign_face = offset_ref("el_f1");
        foreign_face.primary.as_mut().unwrap().body = body(2);
        let p = OffsetFaceParams {
            faces: vec![foreign_face],
            ..offset_params()
        };
        assert!(p.validate().unwrap_err().contains("!= targetBodyId"));

        // ── the distance ──
        // `Scalar::new` panics on a non-finite value and its Deserialize rejects
        // one, so this belt is only reachable through a hand-built struct — which
        // is exactly the caller this validation exists to stop.
        for bad in [f64::NAN, f64::INFINITY] {
            let p = OffsetFaceParams {
                distance: Scalar {
                    value: bad,
                    expr: None,
                },
                ..offset_params()
            };
            assert!(p.validate().unwrap_err().contains("must be finite"));
        }

        // ── multi-face is Offset-only ──
        let multi = OffsetFaceParams {
            face_ids: vec![ElementId::new("el_f1"), ElementId::new("el_f2")],
            faces: vec![offset_ref("el_f1"), offset_ref("el_f2")],
            ..offset_params()
        };
        assert!(multi.validate().is_ok(), "Offset admits a multi-face set");
        for t in [
            OffsetDistanceType::Radius,
            OffsetDistanceType::Diameter,
            OffsetDistanceType::Total,
        ] {
            let p = OffsetFaceParams {
                distance_type: t,
                ..multi.clone()
            };
            assert!(p.validate().unwrap_err().contains("exactly one face"));
        }

        // ── Total: opposite face REQUIRED, chaining OFF ──
        let p = OffsetFaceParams {
            distance_type: OffsetDistanceType::Total,
            chain_tangent_faces: false,
            ..offset_params()
        };
        assert!(p
            .validate()
            .unwrap_err()
            .contains("requires an oppositeFace"));
        let p = OffsetFaceParams {
            chain_tangent_faces: true,
            ..offset_total_params()
        };
        assert!(p
            .validate()
            .unwrap_err()
            .contains("chainTangentFaces = false"));

        // ── the opposite face is Total-ONLY, and mirrors its bare id ──
        for t in [
            OffsetDistanceType::Offset,
            OffsetDistanceType::Radius,
            OffsetDistanceType::Diameter,
        ] {
            let p = OffsetFaceParams {
                distance_type: t,
                distance: Scalar::new(2.5),
                chain_tangent_faces: false,
                ..offset_total_params()
            };
            assert!(p.validate().unwrap_err().contains("Total-only"));
        }
        let p = OffsetFaceParams {
            opposite_face_id: None,
            ..offset_total_params()
        };
        assert!(p.validate().unwrap_err().contains("set together"));
        let p = OffsetFaceParams {
            opposite_face_id: Some(ElementId::new("el_stale")),
            ..offset_total_params()
        };
        assert!(p.validate().unwrap_err().contains("!= oppositeFaceId"));

        // ── Radius / Diameter are ABSOLUTE ⇒ strictly positive ──
        for t in [OffsetDistanceType::Radius, OffsetDistanceType::Diameter] {
            for bad in [0.0, -1.0] {
                let p = OffsetFaceParams {
                    distance_type: t,
                    distance: Scalar::new(bad),
                    ..offset_params()
                };
                assert!(p.validate().unwrap_err().contains("positive distance"));
            }
            let p = OffsetFaceParams {
                distance_type: t,
                distance: Scalar::new(6.0),
                ..offset_params()
            };
            assert!(p.validate().is_ok());
        }
        // An `Offset` delta is SIGNED, but identity/sub-resolution edits are not
        // authorable features: the worker refuses them rather than republishing
        // unchanged geometry as a successful modification.
        for bad in [0.0, 5.0e-4, -5.0e-4] {
            let p = OffsetFaceParams {
                distance: Scalar::new(bad),
                ..offset_params()
            };
            assert!(p.validate().unwrap_err().contains("at least"));
        }
        let p = OffsetFaceParams {
            distance: Scalar::new(-2.5),
            ..offset_params()
        };
        assert!(p.validate().is_ok());
    }

    /// V2 and V3 are two READINGS of one payload (SCHEMA §7.3, WP3-C5), so the
    /// record-level rules must be indistinguishable between them: whatever a V2
    /// record may say, a V3 record may say, and vice versa. Anything else would
    /// make a re-authoring bump a structural migration instead of the pure
    /// reinterpretation it is.
    #[test]
    fn offset_face_accepts_both_result_policy_versions_under_identical_rules() {
        for version in [
            OFFSET_FACE_RESULT_POLICY_VERSION_V2,
            OFFSET_FACE_RESULT_POLICY_VERSION,
        ] {
            let base = OffsetFaceParams {
                result_policy_version: Some(version),
                ..offset_params()
            };
            assert!(base.validate().is_ok(), "v{version} canonical record");

            let empty = OffsetFaceParams {
                primary_face_ids: vec![],
                ..base.clone()
            };
            assert!(empty
                .validate()
                .unwrap_err()
                .contains("at least one primary face"));

            let foreign = OffsetFaceParams {
                primary_face_ids: vec![ElementId::new("el_not_in_closure")],
                ..base.clone()
            };
            assert!(foreign
                .validate()
                .unwrap_err()
                .contains("is not in the frozen faceIds closure"));

            let duplicated = OffsetFaceParams {
                face_ids: vec![ElementId::new("el_f1"), ElementId::new("el_f2")],
                faces: vec![offset_ref("el_f1"), offset_ref("el_f2")],
                primary_face_ids: vec![ElementId::new("el_f1"), ElementId::new("el_f1")],
                ..base.clone()
            };
            assert!(duplicated
                .validate()
                .unwrap_err()
                .contains("contains duplicate"));
        }

        // Absent version + present primaries names BOTH accepted values, so the
        // message tells an author what it would take to make the record legal.
        let orphan = OffsetFaceParams {
            result_policy_version: None,
            ..offset_params()
        };
        assert_eq!(
            orphan.validate().unwrap_err(),
            "OffsetFace primaryFaceIds requires resultPolicyVersion 2 or 3"
        );

        // 4 is neither reading. It is refused by name, never executed as the
        // nearest known version.
        let future = OffsetFaceParams {
            result_policy_version: Some(4),
            ..offset_params()
        };
        assert_eq!(
            future.validate().unwrap_err(),
            "OffsetFace resultPolicyVersion 4 is unsupported (expected 2 or 3)"
        );
    }

    /// The NORMATIVE slot order (SCHEMA §7.3): operative faces in stored order,
    /// then the `Total` opposite face LAST. `wire_op_inputs`, `element_ref_input`
    /// and `InputPath` all mirror this table.
    #[test]
    fn offset_face_element_refs_are_faces_then_opposite() {
        let mut op = KnownOperation::OffsetFace(OffsetFaceParams {
            face_ids: vec![ElementId::new("el_f1"), ElementId::new("el_f2")],
            faces: vec![offset_ref("el_f1"), offset_ref("el_f2")],
            ..offset_params()
        });
        let ids: Vec<String> = op
            .element_refs_mut()
            .iter()
            .map(|r| r.primary.as_ref().unwrap().element.to_string())
            .collect();
        assert_eq!(ids, vec!["el_f1", "el_f2"]);

        let mut total = KnownOperation::OffsetFace(offset_total_params());
        let ids: Vec<String> = total
            .element_refs_mut()
            .iter()
            .map(|r| r.primary.as_ref().unwrap().element.to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["el_f1", "el_f9"],
            "opposite face is the LAST slot"
        );
    }

    /// The body dependency is UNCONDITIONAL (the field is mandatory) — unlike
    /// Extrude/Shell, an offset has no NewBody reading in which it is absent.
    #[test]
    fn offset_face_derive_inputs_always_depends_on_its_body() {
        let inputs =
            Operation::Known(KnownOperation::OffsetFace(offset_total_params())).derive_inputs();
        assert_eq!(inputs.bodies, vec![body(1)]);
        assert_eq!(
            inputs.elements,
            vec![ElementId::new("el_f1"), ElementId::new("el_f9")]
        );
        assert!(inputs.sketches.is_empty());
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

    #[test]
    fn pattern_result_policy_version_preserves_future_numeric_values() {
        let legacy = serde_json::json!({
            "opType": "LinearPattern",
            "params": {"direction": [1.0, 0.0, 0.0], "spacing": 1.0, "count": 2}
        });
        assert!(serde_json::from_value::<Operation>(legacy).is_ok());
        let future = serde_json::json!({
            "opType": "LinearPattern",
            "params": {"direction": [1.0, 0.0, 0.0], "spacing": 1.0, "count": 2,
                       "resultPolicyVersion": 3}
        });
        let parsed: Operation = serde_json::from_value(future.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap()["params"]["resultPolicyVersion"],
            future["params"]["resultPolicyVersion"]
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // PlaceComponent (Component Library WP-0.2)
    // ─────────────────────────────────────────────────────────────────────

    fn place_component_params() -> PlaceComponentParams {
        PlaceComponentParams {
            component_id: "onecad.std.iso4762".to_string(),
            component_version: "1.0.0".to_string(),
            component_revision: format!("sha256:{}", "0".repeat(64)),
            params: BTreeMap::new(),
            source: ComponentSourceRef::Generator {
                generator_id: "iso4762".to_string(),
                generator_version: 1,
                params: BTreeMap::new(),
                extra: Extra::new(),
            },
            mate: None,
            placement: FrozenPlacement {
                translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
                rotate: TransformRotation::default(),
            },
            extra: Extra::new(),
        }
    }

    #[test]
    fn place_component_is_a_known_op_type() {
        assert!(KNOWN_OP_TYPES.contains(&"PlaceComponent"));
        let op = Operation::Known(KnownOperation::PlaceComponent(place_component_params()));
        assert_eq!(op.op_type(), "PlaceComponent");
        let json = serde_json::to_value(&op).unwrap();
        assert_eq!(json["opType"], serde_json::json!("PlaceComponent"));
        // Round-trips through the KNOWN gate (never demoted to Opaque).
        let back: Operation = serde_json::from_value(json).unwrap();
        assert_eq!(back, op);
    }

    /// The serialized shape is camelCase throughout, INCLUDING the internally-tagged
    /// `source` enum's struct-variant fields — `rename_all` on the enum renames the
    /// VARIANT name ("generator") but does NOT cascade into struct-variant field
    /// names (unlike a plain struct), so `generatorId`/`generatorVersion` need their
    /// own `#[serde(rename = …)]`. Regression pin: this was silently wrong once
    /// (the worker read an empty `generatorId` and failed every placement).
    #[test]
    fn place_component_source_fields_are_camel_case() {
        let json =
            serde_json::to_value(KnownOperation::PlaceComponent(place_component_params())).unwrap();
        let source = &json["params"]["source"];
        assert_eq!(source["kind"], serde_json::json!("generator"));
        assert_eq!(source["generatorId"], serde_json::json!("iso4762"));
        assert_eq!(source["generatorVersion"], serde_json::json!(1));
        assert!(source.get("generator_id").is_none());
        assert!(source.get("generator_version").is_none());
    }

    /// Same camelCase trap as the generator arm above, for the WP-3.2 kinds —
    /// and the `document` variant has TWO digests, so a rename slip there would
    /// silently swap provenance for geometry.
    #[test]
    fn document_component_source_fields_are_camel_case() {
        let mut params = place_component_params();
        params.source = ComponentSourceRef::Document {
            document_sha256: "a".repeat(64),
            sha256: "b".repeat(64),
            codec: ImportSourceCodec::Xbf,
            brep_format: Some(12),
            params: BTreeMap::new(),
            extra: Extra::new(),
        };
        let json = serde_json::to_value(KnownOperation::PlaceComponent(params)).unwrap();
        let source = &json["params"]["source"];
        assert_eq!(source["kind"], serde_json::json!("document"));
        assert_eq!(source["documentSha256"], serde_json::json!("a".repeat(64)));
        assert_eq!(source["sha256"], serde_json::json!("b".repeat(64)));
        assert_eq!(source["codec"], serde_json::json!("xbf"));
        assert_eq!(source["brepFormat"], serde_json::json!(12));
        assert!(source.get("document_sha256").is_none());
        assert!(source.get("brep_format").is_none());
    }

    /// A CONVERTED codec (`brep`/`xbf`) must pin the binary format version its
    /// bytes were written in, exactly as `ImportStep` does — without it the
    /// worker cannot tell "written by this build" from "written by a build whose
    /// format it cannot read", and a misparse would be silent.
    #[test]
    fn a_blob_component_source_requires_a_format_pin_for_converted_codecs() {
        let mut params = place_component_params();
        params.source = ComponentSourceRef::Embedded {
            sha256: "b".repeat(64),
            codec: ImportSourceCodec::Brep,
            brep_format: None,
            extra: Extra::new(),
        };
        let err = params.validate().unwrap_err();
        assert!(err.contains("brepFormat"), "{err}");

        // …and a `step` payload (spec §2.1 allows a plain STEP in an `embedded`
        // package) has no such pin to require.
        let mut step = place_component_params();
        step.source = ComponentSourceRef::Embedded {
            sha256: "b".repeat(64),
            codec: ImportSourceCodec::Step,
            brep_format: None,
            extra: Extra::new(),
        };
        assert!(step.validate().is_ok());
    }

    #[test]
    fn a_blob_component_source_rejects_a_malformed_digest() {
        let mut geometry = place_component_params();
        geometry.source = ComponentSourceRef::Embedded {
            sha256: "not-a-digest".to_string(),
            codec: ImportSourceCodec::Step,
            brep_format: None,
            extra: Extra::new(),
        };
        assert!(geometry.validate().unwrap_err().contains("sha256"));

        let mut provenance = place_component_params();
        provenance.source = ComponentSourceRef::Document {
            document_sha256: "nope".to_string(),
            sha256: "b".repeat(64),
            codec: ImportSourceCodec::Brep,
            brep_format: Some(4),
            params: BTreeMap::new(),
            extra: Extra::new(),
        };
        assert!(provenance
            .validate()
            .unwrap_err()
            .contains("documentSha256"));
    }

    /// The one accessor wire lowering and the save-time blob refcount both walk —
    /// a fourth source kind that forgets it would silently lose its blob.
    #[test]
    fn blob_ref_covers_exactly_the_blob_backed_kinds() {
        assert!(ComponentSourceRef::Generator {
            generator_id: "iso4762".to_string(),
            generator_version: 1,
            params: BTreeMap::new(),
            extra: Extra::new(),
        }
        .blob_ref()
        .is_none());

        let embedded = ComponentSourceRef::Embedded {
            sha256: "b".repeat(64),
            codec: ImportSourceCodec::Brep,
            brep_format: Some(4),
            extra: Extra::new(),
        };
        let blob = embedded.blob_ref().expect("embedded carries a blob");
        assert_eq!(blob.sha256, "b".repeat(64));
        assert_eq!(blob.codec, ImportSourceCodec::Brep);
        assert_eq!(blob.brep_format, Some(4));

        let document = ComponentSourceRef::Document {
            document_sha256: "a".repeat(64),
            sha256: "c".repeat(64),
            codec: ImportSourceCodec::Xbf,
            brep_format: Some(12),
            params: BTreeMap::new(),
            extra: Extra::new(),
        };
        // The GEOMETRY digest, never the provenance one — swapping them would
        // pin the wrong blob at save and materialize the wrong path at regen.
        assert_eq!(document.blob_ref().unwrap().sha256, "c".repeat(64));

        let profile = ComponentSourceRef::Profile {
            sha256: "d".repeat(64),
            codec: ImportSourceCodec::Brep,
            brep_format: Some(4),
            params: BTreeMap::new(),
            extra: Extra::new(),
        };
        let blob = profile.blob_ref().expect("profile carries a blob");
        assert_eq!(blob.sha256, "d".repeat(64));
        assert_eq!(blob.codec, ImportSourceCodec::Brep);
        assert_eq!(blob.brep_format, Some(4));
    }

    /// WP-C: `profile` is the one kind whose `params` are a REGEN INPUT, so its
    /// `length` is validated at authoring rather than left to the worker. And
    /// the codec is pinned to `brep` BY NAME — the `step` and `xbf` readers
    /// return solids, so accepting them would answer a face question with a
    /// solid reader.
    #[test]
    fn profile_component_source_validation_matrix() {
        let with_source = |source| {
            let mut p = place_component_params();
            p.source = source;
            p
        };
        let profile = |codec, brep_format, params: BTreeMap<String, ComponentParamValue>| {
            ComponentSourceRef::Profile {
                sha256: "d".repeat(64),
                codec,
                brep_format,
                params,
                extra: Extra::new(),
            }
        };
        let length = |mm| {
            let mut m = BTreeMap::new();
            m.insert(
                PROFILE_LENGTH_PARAM.to_string(),
                ComponentParamValue::Number(mm),
            );
            m
        };

        assert!(
            with_source(profile(ImportSourceCodec::Brep, Some(4), length(500.0)))
                .validate()
                .is_ok()
        );

        for bad_codec in [ImportSourceCodec::Step, ImportSourceCodec::Xbf] {
            assert!(
                with_source(profile(bad_codec, Some(4), length(500.0)))
                    .validate()
                    .is_err(),
                "codec `{}` must be refused for a profile source",
                bad_codec.extension()
            );
        }
        assert!(
            with_source(profile(ImportSourceCodec::Brep, None, length(500.0)))
                .validate()
                .is_err(),
            "brepFormat pins the BinTools version the face was written in"
        );
        assert!(
            with_source(profile(ImportSourceCodec::Brep, Some(4), BTreeMap::new()))
                .validate()
                .is_err(),
            "length is the whole point of the kind — its absence is not a default"
        );
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e5 + 1.0] {
            assert!(
                with_source(profile(ImportSourceCodec::Brep, Some(4), length(bad)))
                    .validate()
                    .is_err(),
                "length {bad} must be refused"
            );
        }
        let mut text_length = BTreeMap::new();
        text_length.insert(
            PROFILE_LENGTH_PARAM.to_string(),
            ComponentParamValue::Text("500".into()),
        );
        assert!(
            with_source(profile(ImportSourceCodec::Brep, Some(4), text_length))
                .validate()
                .is_err(),
            "a stringly-typed length must not be coerced"
        );
    }

    /// Same camelCase trap the other arms carry, plus the shape the worker
    /// branches on: `kind: "profile"` with `params` alongside the blob pointer.
    #[test]
    fn profile_component_source_wire_shape() {
        let mut params = place_component_params();
        let mut source_params = BTreeMap::new();
        source_params.insert(
            PROFILE_LENGTH_PARAM.to_string(),
            ComponentParamValue::Number(500.0),
        );
        params.source = ComponentSourceRef::Profile {
            sha256: "d".repeat(64),
            codec: ImportSourceCodec::Brep,
            brep_format: Some(4),
            params: source_params,
            extra: Extra::new(),
        };
        let op = Operation::Known(KnownOperation::PlaceComponent(params));
        let json = serde_json::to_value(&op).unwrap();
        let source = &json["params"]["source"];
        assert_eq!(source["kind"], serde_json::json!("profile"));
        assert_eq!(source["codec"], serde_json::json!("brep"));
        assert_eq!(source["brepFormat"], serde_json::json!(4));
        assert_eq!(source["params"]["length"], serde_json::json!(500.0));
        assert!(source.get("brep_format").is_none());

        let back: Operation = serde_json::from_value(json).unwrap();
        assert_eq!(back, op, "the profile kind round-trips through KNOWN");
    }

    /// `inputs[]` = the mate target ONLY when a mate is present (spec §3.1); a
    /// component dropped in free space has no topological dependency at all.
    #[test]
    fn place_component_derives_conditional_mate_input() {
        let free_space = Operation::Known(KnownOperation::PlaceComponent(place_component_params()));
        assert!(free_space.derive_inputs().bodies.is_empty());

        let mut mated = place_component_params();
        let target_body = body(1);
        mated.mate = Some(ComponentMate {
            self_attachment: "shank_axis".to_string(),
            target: ElementRef {
                primary: Some(crate::document::refs::PrimaryRef {
                    body: target_body,
                    element: ElementId::new("el_1"),
                    kind: ElementKind::Face,
                    extra: Extra::new(),
                }),
                intent: None,
                anchor: None,
                extra: Extra::new(),
            },
            kind: MateKind::Concentric,
            flipped: false,
            self_frame: None,
            extra: Extra::new(),
        });
        let op = Operation::Known(KnownOperation::PlaceComponent(mated));
        let inputs = op.derive_inputs();
        assert_eq!(inputs.bodies, vec![target_body]);
        assert_eq!(inputs.elements.len(), 1);
    }

    #[test]
    fn place_component_validation_matrix() {
        let ok = place_component_params();
        assert!(ok.validate().is_ok());

        let mut bad_id = ok.clone();
        bad_id.component_id = "unnamespaced".to_string();
        assert!(bad_id.validate().is_err());

        let mut bad_version = ok.clone();
        bad_version.component_version = "  ".to_string();
        assert!(bad_version.validate().is_err());

        let mut bad_revision = ok.clone();
        bad_revision.component_revision = "not-a-hash".to_string();
        assert!(bad_revision.validate().is_err());

        // The id/version pair names the package directory — a path-escaping
        // value must die at authoring, before any `Path::join` sees it.
        for evil in [
            "onecad.std/../evil",
            "..",
            "a..b",
            r"onecad\evil.x",
            "Onecad.Std.X",
        ] {
            let mut traversal_id = ok.clone();
            traversal_id.component_id = evil.to_string();
            assert!(
                traversal_id.validate().is_err(),
                "id `{evil}` must be refused"
            );
        }
        let mut traversal_version = ok.clone();
        traversal_version.component_version = "1.0.0/../..".to_string();
        assert!(traversal_version.validate().is_err());

        let mut bad_generator = ok.clone();
        bad_generator.source = ComponentSourceRef::Generator {
            generator_id: String::new(),
            generator_version: 1,
            params: BTreeMap::new(),
            extra: Extra::new(),
        };
        assert!(bad_generator.validate().is_err());

        let mut bad_mate = ok.clone();
        bad_mate.mate = Some(ComponentMate {
            self_attachment: String::new(),
            target: ElementRef {
                primary: None,
                intent: None,
                anchor: None,
                extra: Extra::new(),
            },
            kind: MateKind::Coincident,
            flipped: false,
            self_frame: None,
            extra: Extra::new(),
        });
        assert!(bad_mate.validate().is_err());
    }

    /// WP-F1.1: `mate.selfFrame` is additive and OPTIONAL. What this pins is
    /// the compatibility half — a mate with no frame must serialize with NO
    /// `selfFrame` key at all, so every document written before WP-F1.1 stays
    /// byte-identical on rewrite and no reader has to special-case a null.
    #[test]
    fn mate_self_frame_round_trips_and_is_absent_when_unset() {
        let mate = |self_frame| ComponentMate {
            self_attachment: "shank_axis".to_string(),
            target: ElementRef {
                primary: Some(crate::document::refs::PrimaryRef {
                    body: body(1),
                    element: ElementId::new("el_1"),
                    kind: ElementKind::Face,
                    extra: Extra::new(),
                }),
                intent: None,
                anchor: None,
                extra: Extra::new(),
            },
            kind: MateKind::Concentric,
            flipped: false,
            self_frame,
            extra: Extra::new(),
        };

        let bare = serde_json::to_value(mate(None)).unwrap();
        assert!(
            bare.get("selfFrame").is_none(),
            "an unset frame must not even emit the key: {bare}"
        );
        assert_eq!(
            serde_json::from_value::<ComponentMate>(bare).unwrap(),
            mate(None)
        );

        let framed = mate(Some(MateFrame {
            origin: Vec3::new_unchecked(0.0, 0.0, 10.0),
            z: Vec3::new_unchecked(0.0, 0.0, 1.0),
            x: Vec3::new_unchecked(1.0, 0.0, 0.0),
        }));
        let json = serde_json::to_value(framed.clone()).unwrap();
        // Vec3's wire form is a bare `[x,y,z]` array (SCHEMA §4) — the same
        // spelling every other coordinate on this op uses.
        assert_eq!(
            json["selfFrame"]["origin"],
            serde_json::json!([0.0, 0.0, 10.0])
        );
        assert_eq!(json["selfFrame"]["z"], serde_json::json!([0.0, 0.0, 1.0]));
        assert_eq!(json["selfFrame"]["x"], serde_json::json!([1.0, 0.0, 0.0]));
        assert_eq!(
            serde_json::from_value::<ComponentMate>(json).unwrap(),
            framed
        );
    }

    /// A non-finite frame component is refused at authoring, the same way
    /// every other coordinate on this op is (`Vec3`'s own deserialize already
    /// blocks the wire path; this covers the programmatic one).
    #[test]
    fn place_component_refuses_a_non_finite_self_frame() {
        let mut params = place_component_params();
        params.mate = Some(ComponentMate {
            self_attachment: "shank_axis".to_string(),
            target: ElementRef {
                primary: None,
                intent: None,
                anchor: None,
                extra: Extra::new(),
            },
            kind: MateKind::Concentric,
            flipped: false,
            self_frame: Some(MateFrame {
                origin: Vec3::new_unchecked(0.0, 0.0, f64::NAN),
                z: Vec3::new_unchecked(0.0, 0.0, 1.0),
                x: Vec3::new_unchecked(1.0, 0.0, 0.0),
            }),
            extra: Extra::new(),
        });
        assert!(params.validate().is_err());
    }

    // ---------------------------------------------------------------
    // Gear (SCHEMA §7.3, Gear Generator G1)
    // ---------------------------------------------------------------

    fn involute_block() -> InvoluteExternalParams {
        InvoluteExternalParams {
            teeth: 20,
            module: Scalar::new(2.0),
            height: Scalar::new(5.0),
            pressure_angle_deg: Scalar::new(20.0),
            shift: 0.0,
            helix_angle_deg: 0.0,
            double_helix: false,
            properties_from_tool: false,
            undercut: false,
            backlash: 0.0,
            clearance: 0.25,
            head: 0.0,
            sample_count: 20,
            axle_hole: false,
            axle_hole_diameter: None,
            offset_hole: false,
            offset_hole_diameter: None,
            offset_hole_offset: None,
        }
    }

    /// A gear placed on a picked face (the common authoring path).
    fn gear_params() -> GearParams {
        GearParams {
            recipe: GearRecipe::InvoluteExternal,
            placement: GearPlacement {
                face: Some(hole_face_ref()),
                frame: None,
                point: Vec3::new_unchecked(25.0, 10.0, 30.0),
            },
            involute_external: Some(involute_block()),
            extra: Extra::new(),
        }
    }

    /// A gear placed on an explicit frozen frame (datum/world placement).
    fn gear_params_frame() -> GearParams {
        GearParams {
            recipe: GearRecipe::InvoluteExternal,
            placement: GearPlacement {
                face: None,
                frame: Some(GearFrame {
                    origin: Vec3::new_unchecked(1.0, 2.0, 3.0),
                    axis: Vec3::new_unchecked(0.0, 0.0, 1.0),
                    x_dir: Vec3::new_unchecked(1.0, 0.0, 0.0),
                }),
                point: Vec3::new_unchecked(1.0, 2.0, 3.0),
            },
            involute_external: Some(involute_block()),
            extra: Extra::new(),
        }
    }

    #[test]
    fn gear_round_trips_through_the_wire_shape() {
        let op = Operation::Known(KnownOperation::Gear(gear_params()));
        let json = serde_json::to_value(&op).unwrap();
        assert_eq!(json["opType"], "Gear");
        // SCHEMA §7.3 key spelling, checked key-for-key rather than by
        // round-trip alone: a rename would round-trip happily and still break
        // the worker.
        let p = &json["params"];
        assert!(p["placement"]["face"].is_object());
        assert!(p["placement"]["frame"].is_null());
        assert_eq!(p["recipe"], "involuteExternal");
        let inv = &p["involuteExternal"];
        assert_eq!(inv["teeth"], 20);
        assert_eq!(inv["module"]["value"], 2.0);
        assert_eq!(inv["pressureAngleDeg"]["value"], 20.0);
        assert_eq!(inv["sampleCount"], 20);
        assert_eq!(inv["clearance"], 0.25);

        let back: Operation = serde_json::from_value(json).unwrap();
        assert_eq!(back, op);
    }

    #[test]
    fn gear_frame_placement_round_trips() {
        let op = Operation::Known(KnownOperation::Gear(gear_params_frame()));
        let json = serde_json::to_value(&op).unwrap();
        assert!(json["params"]["placement"]["face"].is_null());
        assert_eq!(
            json["params"]["placement"]["frame"]["axis"],
            serde_json::json!([0.0, 0.0, 1.0])
        );
        let back: Operation = serde_json::from_value(json).unwrap();
        assert_eq!(back, op);
    }

    #[test]
    fn gear_is_a_known_op_type() {
        assert!(KNOWN_OP_TYPES.contains(&"Gear"));
        assert_eq!(
            Operation::Known(KnownOperation::Gear(gear_params())).op_type(),
            "Gear"
        );
    }

    #[test]
    fn gear_derives_only_its_placement_face_as_input() {
        // A gear MINTS its body, so it must never claim a host-body dependency.
        let inputs = Operation::Known(KnownOperation::Gear(gear_params())).derive_inputs();
        assert_eq!(inputs.bodies.len(), 1, "only the placement face's body");
        assert_eq!(inputs.elements.len(), 1, "only the placement face element");

        // A frame placement depends on nothing at all.
        let none = Operation::Known(KnownOperation::Gear(gear_params_frame())).derive_inputs();
        assert!(none.bodies.is_empty());
        assert!(none.elements.is_empty());
    }

    #[test]
    fn gear_exposes_its_placement_face_as_a_mutable_ref() {
        let mut op = KnownOperation::Gear(gear_params());
        assert_eq!(op.element_refs_mut().len(), 1);
        let mut framed = KnownOperation::Gear(gear_params_frame());
        assert!(framed.element_refs_mut().is_empty());
    }

    #[test]
    fn gear_exposes_only_dimensional_scalars() {
        let mut op = KnownOperation::Gear(gear_params());
        let names: Vec<&str> = op.scalars_mut().into_iter().map(|(n, _, _)| n).collect();
        assert!(names.contains(&"Gear.module"));
        assert!(names.contains(&"Gear.height"));
        assert!(names.contains(&"Gear.pressureAngleDeg"));
        // Coefficients are NOT drivable dimensions — binding `clearance` to a
        // length variable would be a category error, so it must not appear.
        assert!(!names.iter().any(|n| n.contains("clearance")));
        assert!(!names.iter().any(|n| n.contains("shift")));
    }

    #[test]
    fn gear_accepts_its_canonical_forms() {
        assert!(gear_params().validate().is_ok());
        assert!(gear_params_frame().validate().is_ok());
    }

    #[test]
    fn gear_placement_must_be_exactly_one_of_face_or_frame() {
        let mut both = gear_params();
        both.placement.frame = Some(GearFrame {
            origin: Vec3::new_unchecked(25.0, 10.0, 30.0),
            axis: Vec3::new_unchecked(0.0, 0.0, 1.0),
            x_dir: Vec3::new_unchecked(1.0, 0.0, 0.0),
        });
        assert!(both.validate().is_err(), "face AND frame must be refused");

        let mut neither = gear_params();
        neither.placement.face = None;
        assert!(neither.validate().is_err(), "neither must be refused");
    }

    #[test]
    fn gear_frame_origin_must_equal_the_frozen_point() {
        // Two sources of truth for one position would drift apart on the next
        // edit; the record is refused rather than silently preferring one.
        let mut p = gear_params_frame();
        p.placement.point = Vec3::new_unchecked(9.0, 9.0, 9.0);
        assert!(p.validate().is_err());
    }

    #[test]
    fn gear_frame_directions_must_be_real_directions() {
        let mut p = gear_params_frame();
        p.placement.frame.as_mut().unwrap().axis = Vec3::new_unchecked(0.0, 0.0, 0.0);
        assert!(p.validate().is_err(), "a zero axis is not a direction");

        let mut q = gear_params_frame();
        q.placement.frame.as_mut().unwrap().x_dir = Vec3::new_unchecked(0.0, 0.0, 0.0);
        assert!(q.validate().is_err(), "a zero xDir is not a direction");
    }

    #[test]
    fn gear_requires_the_recipe_block_its_recipe_names() {
        let mut p = gear_params();
        p.involute_external = None;
        assert!(p.validate().is_err());
    }

    #[test]
    fn gear_refuses_dimensions_outside_their_domain() {
        for mutate in [
            (|i: &mut InvoluteExternalParams| i.teeth = 2) as fn(&mut InvoluteExternalParams),
            |i| i.module = Scalar::new(0.0),
            |i| i.height = Scalar::new(0.0),
            |i| i.pressure_angle_deg = Scalar::new(0.0),
            |i| i.pressure_angle_deg = Scalar::new(90.0),
            |i| i.sample_count = 1,
            |i| i.backlash = -1.0,
            |i| i.clearance = -0.1,
        ] {
            let mut p = gear_params();
            mutate(p.involute_external.as_mut().unwrap());
            assert!(
                p.validate().is_err(),
                "expected a refusal for an out-of-domain dimension"
            );
        }
    }

    #[test]
    fn gear_refuses_helical_until_the_sweep_infrastructure_lands() {
        // Refused BY NAME rather than silently flattened to a spur gear —
        // returning a body that is not what was asked for is the worse failure.
        let mut helical = gear_params();
        helical.involute_external.as_mut().unwrap().helix_angle_deg = 15.0;
        assert!(helical.validate().is_err());

        let mut herringbone = gear_params();
        herringbone.involute_external.as_mut().unwrap().double_helix = true;
        assert!(herringbone.validate().is_err());
    }

    #[test]
    fn gear_bore_blocks_are_checked_both_ways() {
        // On without its dimension.
        let mut on_no_dim = gear_params();
        on_no_dim.involute_external.as_mut().unwrap().axle_hole = true;
        assert!(on_no_dim.validate().is_err());

        // A stale dimension left behind after toggling the bore OFF must be
        // rejected, or it silently reappears the next time it is toggled on.
        let mut off_with_dim = gear_params();
        off_with_dim
            .involute_external
            .as_mut()
            .unwrap()
            .axle_hole_diameter = Some(Scalar::new(10.0));
        assert!(off_with_dim.validate().is_err());

        // The valid on-form is accepted.
        let mut ok = gear_params();
        {
            let inv = ok.involute_external.as_mut().unwrap();
            inv.axle_hole = true;
            inv.axle_hole_diameter = Some(Scalar::new(10.0));
        }
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn gear_offset_bore_requires_its_offset() {
        let mut p = gear_params();
        {
            let inv = p.involute_external.as_mut().unwrap();
            inv.offset_hole = true;
            inv.offset_hole_diameter = Some(Scalar::new(6.0));
        }
        assert!(
            p.validate().is_err(),
            "offset is required when the bore is on"
        );

        p.involute_external.as_mut().unwrap().offset_hole_offset = Some(Scalar::new(12.0));
        assert!(p.validate().is_ok());

        // ...and a stale offset with the bore off is refused.
        let mut stale = gear_params();
        stale.involute_external.as_mut().unwrap().offset_hole_offset = Some(Scalar::new(12.0));
        assert!(stale.validate().is_err());
    }

    // ── Chamfer distance-angle mode: `angleDeg` (SCHEMA §7.3) ────────────────

    fn chamfer_params(distance2: Option<f64>, angle_deg: Option<f64>) -> ChamferParams {
        ChamferParams {
            radius: Scalar::new(1.0),
            distance2: distance2.map(Scalar::new),
            angle_deg: angle_deg.map(Scalar::new),
            edge_ids: vec![ElementId::new("e:14")],
            edges: vec![],
            chain_tangent_edges: true,
            tangent_closure_version: None,
            extra: Extra::new(),
        }
    }

    /// `angleDeg` is skip-none, so a chamfer that does not use the distance-angle
    /// mode serializes to the EXACT bytes it did before the field existed. The
    /// expected string is a literal pin, not a re-derivation: a regression that
    /// grew a key here would silently rewrite every stored document.
    #[test]
    fn chamfer_without_angle_deg_serializes_byte_identically() {
        let equal = Operation::Known(KnownOperation::Chamfer(chamfer_params(None, None)));
        assert_eq!(
            serde_json::to_string(&equal).unwrap(),
            r#"{"opType":"Chamfer","params":{"radius":{"value":1.0},"edgeIds":["e:14"],"chainTangentEdges":true}}"#
        );

        let two_distance =
            Operation::Known(KnownOperation::Chamfer(chamfer_params(Some(2.5), None)));
        assert_eq!(
            serde_json::to_string(&two_distance).unwrap(),
            r#"{"opType":"Chamfer","params":{"radius":{"value":1.0},"distance2":{"value":2.5},"edgeIds":["e:14"],"chainTangentEdges":true}}"#
        );
    }

    /// The distance-angle form emits `angleDeg` in DEGREES right after the two
    /// distance keys, and round-trips — including the hand-authored bare-number
    /// SCHEMA form.
    #[test]
    fn chamfer_angle_deg_round_trips_in_degrees() {
        let angled = Operation::Known(KnownOperation::Chamfer(chamfer_params(None, Some(30.0))));
        let json = serde_json::to_string(&angled).unwrap();
        assert_eq!(
            json,
            r#"{"opType":"Chamfer","params":{"radius":{"value":1.0},"angleDeg":{"value":30.0},"edgeIds":["e:14"],"chainTangentEdges":true}}"#
        );
        assert_eq!(serde_json::from_str::<Operation>(&json).unwrap(), angled);

        let wire = r#"{ "opType": "Chamfer", "params": {
            "mode": "Chamfer", "radius": 1.0, "angleDeg": 30.0,
            "edgeIds": ["e:14"], "chainTangentEdges": true } }"#;
        match serde_json::from_str::<Operation>(wire).unwrap() {
            Operation::Known(KnownOperation::Chamfer(p)) => {
                assert_eq!(p.angle_deg.expect("angleDeg parsed").value, 30.0);
                assert!(p.distance2.is_none());
            }
            other => panic!("expected Chamfer, got {other:?}"),
        }
    }

    /// The two second-leg spellings are mutually exclusive and the refusal NAMES
    /// both — resolving them by precedence would drop one of the two numbers the
    /// user typed.
    #[test]
    fn chamfer_refuses_distance2_and_angle_deg_together_by_name() {
        let err = chamfer_params(Some(2.5), Some(30.0))
            .validate()
            .expect_err("both modes at once");
        assert!(
            err.contains("distance2") && err.contains("angleDeg"),
            "the refusal names both fields: {err}"
        );
    }

    /// 0 and 180 are EXCLUSIVE bounds (a chamfer face coplanar with, or folded back
    /// onto, the reference face is not a shape). The bound is otherwise loose on
    /// purpose: whether an angle fits the local dihedral is the worker's call, as a
    /// recoverable `OP_FAILED`.
    #[test]
    fn chamfer_angle_deg_must_be_strictly_between_0_and_180() {
        for bad in [0.0, -1.0, 180.0, 270.0] {
            let err = chamfer_params(None, Some(bad))
                .validate()
                .expect_err("out-of-range angleDeg is refused");
            assert!(err.contains("angleDeg"), "{bad}: {err}");
        }
        // `Scalar::new` panics on a non-finite value and deserialize rejects one, so
        // the finite guard is reachable only through the struct literal — checked
        // here for the same reason `distance2` checks it.
        for bad in [f64::NAN, f64::INFINITY] {
            let mut p = chamfer_params(None, Some(1.0));
            p.angle_deg = Some(Scalar {
                value: bad,
                expr: None,
            });
            let err = p.validate().expect_err("non-finite angleDeg is refused");
            assert!(err.contains("angleDeg"), "{bad}: {err}");
        }
        for ok in [0.001, 45.0, 90.0, 179.999] {
            assert!(
                chamfer_params(None, Some(ok)).validate().is_ok(),
                "{ok} is in range"
            );
        }
    }

    // ── The drivable-scalar registry freeze ─────────────────────────────────

    fn labels_of(mut op: KnownOperation) -> Vec<(&'static str, Dimension)> {
        op.scalars_mut()
            .into_iter()
            .map(|(name, dim, _)| (name, dim))
            .collect()
    }

    fn extrude_scalar_params() -> ExtrudeParams {
        ExtrudeParams {
            profile: None,
            distance: Scalar::new(10.0),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Extra::new(),
        }
    }

    fn hole_scalar_params() -> HoleParams {
        HoleParams {
            target_body: body(1),
            face: hole_face_ref(),
            point: Vec3::new_unchecked(0.0, 0.0, 0.0),
            hole_type: HoleType::Counterbore,
            diameter: Scalar::new(5.0),
            depth: Some(Scalar::new(10.0)),
            cb_diameter: Some(Scalar::new(9.0)),
            cb_depth: Some(Scalar::new(3.0)),
            cs_diameter: Some(Scalar::new(9.0)),
            cs_angle_deg: Some(Scalar::new(90.0)),
            result_policy_version: None,
            extra: Extra::new(),
        }
    }

    /// **The registry freeze.** Every drivable `Scalar` in every scalar-bearing
    /// `KnownOperation`, in order, with the [`Dimension`] it is measured in.
    ///
    /// Two contracts in one list:
    ///
    /// * **Order is normative.** `write_back_resolved_values` zips two records'
    ///   lists BY POSITION, so reordering an arm silently writes one field's
    ///   resolved number onto another.
    /// * **Every entry declares a dimension.** The dimension is the expression
    ///   engine's call site — it is what makes `"45deg"` a loud refusal in a
    ///   length field instead of a silent 45 mm — so a scalar added without one
    ///   (or with the wrong one) is a correctness bug, not a cosmetic omission.
    ///
    /// The `Vec::new()` arm of `scalars_mut` is an EXHAUSTIVE match, so a new
    /// `KnownOperation` variant cannot skip this table without a compile error;
    /// this test covers every variant that currently contributes an entry.
    #[test]
    fn the_drivable_scalar_registry_is_frozen_in_order_and_dimension() {
        use Dimension::{Angle, Length, Scalar as Unitless};

        assert_eq!(
            labels_of(KnownOperation::Extrude(extrude_scalar_params())),
            vec![
                ("Extrude.distance", Length),
                ("Extrude.draftAngleDeg", Angle),
                ("Extrude.distance2", Length),
            ]
        );
        assert_eq!(
            labels_of(KnownOperation::Revolve(RevolveParams {
                profile: None,
                angle_deg: Scalar::new(360.0),
                axis: None,
                boolean_mode: BooleanMode::NewBody,
                target_body: None,
                extra: Extra::new(),
            })),
            vec![("Revolve.angleDeg", Angle)]
        );
        assert_eq!(
            labels_of(KnownOperation::Fillet(FilletParams {
                radius: Scalar::new(2.0),
                edge_ids: Vec::new(),
                edges: Vec::new(),
                chain_tangent_edges: false,
                tangent_closure_version: None,
                extra: Extra::new(),
            })),
            vec![("Fillet.radius", Length)]
        );
        assert_eq!(
            labels_of(KnownOperation::Chamfer(chamfer_params(Some(2.5), None))),
            vec![("Chamfer.radius", Length), ("Chamfer.distance2", Length)]
        );
        assert_eq!(
            labels_of(KnownOperation::Chamfer(chamfer_params(None, Some(30.0)))),
            vec![("Chamfer.radius", Length), ("Chamfer.angleDeg", Angle)]
        );
        assert_eq!(
            labels_of(KnownOperation::Shell(ShellParams {
                thickness: Scalar::new(1.5),
                open_faces: Vec::new(),
                faces: Vec::new(),
                target_body: None,
                extra: Extra::new(),
            })),
            vec![("Shell.thickness", Length)]
        );
        assert_eq!(
            labels_of(KnownOperation::LinearPattern(LinearPatternParams {
                source_body: None,
                direction: Vec3::new_unchecked(1.0, 0.0, 0.0),
                spacing: Scalar::new(10.0),
                count: 3,
                fuse_result: false,
                result_policy_version: None,
                extra: Extra::new(),
            })),
            vec![("LinearPattern.spacing", Length)]
        );
        assert_eq!(
            labels_of(KnownOperation::CircularPattern(CircularPatternParams {
                source_body: None,
                axis_origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
                axis_direction: Vec3::new_unchecked(0.0, 0.0, 1.0),
                angle_deg: Scalar::new(360.0),
                count: 4,
                fuse_result: false,
                result_policy_version: None,
                extra: Extra::new(),
            })),
            vec![("CircularPattern.angleDeg", Angle)]
        );
        // A unit scale is a ratio, not a length: binding it to a `10mm` variable
        // must be a dimension mismatch, not a silent 10×.
        assert_eq!(
            labels_of(KnownOperation::ImportStep(step_params())),
            vec![("ImportStep.unitScale", Unitless)]
        );
        assert_eq!(
            labels_of(KnownOperation::TransformBody(TransformBodyParams {
                targets: Vec::new(),
                translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
                rotate: TransformRotation::default(),
                copy: false,
                extra: Extra::new(),
            })),
            vec![
                ("TransformBody.translate[0]", Length),
                ("TransformBody.translate[1]", Length),
                ("TransformBody.translate[2]", Length),
                ("TransformBody.rotate.angleDeg", Angle),
            ]
        );
        assert_eq!(
            labels_of(KnownOperation::Hole(hole_scalar_params())),
            vec![
                ("Hole.diameter", Length),
                ("Hole.depth", Length),
                ("Hole.cbDiameter", Length),
                ("Hole.cbDepth", Length),
                ("Hole.csDiameter", Length),
                ("Hole.csAngleDeg", Angle),
            ]
        );
        // With every optional hole present, so the full arm is frozen.
        let mut geared = gear_params();
        if let Some(inv) = geared.involute_external.as_mut() {
            inv.axle_hole = true;
            inv.axle_hole_diameter = Some(Scalar::new(4.0));
            inv.offset_hole = true;
            inv.offset_hole_diameter = Some(Scalar::new(3.0));
            inv.offset_hole_offset = Some(Scalar::new(8.0));
        }
        assert_eq!(
            labels_of(KnownOperation::Gear(geared)),
            vec![
                ("Gear.module", Length),
                ("Gear.height", Length),
                ("Gear.pressureAngleDeg", Angle),
                ("Gear.axleHoleDiameter", Length),
                ("Gear.offsetHoleDiameter", Length),
                ("Gear.offsetHoleOffset", Length),
            ]
        );
        assert_eq!(
            labels_of(KnownOperation::OffsetFace(OffsetFaceParams {
                face_ids: Vec::new(),
                primary_face_ids: Vec::new(),
                faces: Vec::new(),
                distance: Scalar::new(1.0),
                distance_type: OffsetDistanceType::Offset,
                chain_tangent_faces: false,
                opposite_face_id: None,
                opposite_face: None,
                target_body: body(1),
                result_policy_version: None,
                extra: Extra::new(),
            })),
            vec![("OffsetFace.distance", Length)]
        );
        assert_eq!(
            labels_of(KnownOperation::PlaceComponent(place_component_params())),
            vec![
                ("PlaceComponent.placement.translate[0]", Length),
                ("PlaceComponent.placement.translate[1]", Length),
                ("PlaceComponent.placement.translate[2]", Length),
                ("PlaceComponent.placement.rotate.angleDeg", Angle),
            ]
        );
    }

    /// The canonical-form / wire strip: every REGISTERED expression goes, every
    /// `value` stays.
    #[test]
    fn clear_scalar_exprs_drops_bindings_and_keeps_numbers() {
        let mut params = extrude_scalar_params();
        params.distance = Scalar::with_expr(20.0, "w * 2");
        params.draft_angle_deg = Scalar::with_expr(3.0, "draft");
        let mut op = KnownOperation::Extrude(params);
        op.clear_scalar_exprs();
        let KnownOperation::Extrude(p) = &op else {
            panic!("expected an Extrude")
        };
        assert_eq!(p.distance, Scalar::new(20.0));
        assert_eq!(p.draft_angle_deg, Scalar::new(3.0));
    }

    /// A `Scalar` param left out of `scalars_mut` is silently un-drivable by a
    /// document variable, keeping its stale cached value forever.
    #[test]
    fn chamfer_exposes_angle_deg_as_a_drivable_scalar() {
        let labels = |mut op: KnownOperation| -> Vec<&'static str> {
            op.scalars_mut().into_iter().map(|(n, _, _)| n).collect()
        };
        assert_eq!(
            labels(KnownOperation::Chamfer(chamfer_params(None, None))),
            vec!["Chamfer.radius"]
        );
        assert_eq!(
            labels(KnownOperation::Chamfer(chamfer_params(None, Some(30.0)))),
            vec!["Chamfer.radius", "Chamfer.angleDeg"]
        );
        assert_eq!(
            labels(KnownOperation::Chamfer(chamfer_params(Some(2.5), None))),
            vec!["Chamfer.radius", "Chamfer.distance2"]
        );
    }
}
