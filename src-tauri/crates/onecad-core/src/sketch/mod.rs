//! Sketch domain: the authoritative 2D parametric sketch (plane + entities +
//! constraints + a derived region cache).
//!
//! This is the document / file-format model (`sketches/*.json` in the v2
//! container), distinct from the op-record wire shape (`SketchOpParams` in
//! [`crate::document::record`], which carries the SCHEMA §7.3 worker-lane JSON
//! opaquely) and from the solver-lane `SketchUpsert` payload (SCHEMA §7.4). The
//! `onecad-protocol` adapter bridges this typed model to those wire shapes.
//!
//! **Serde discipline** (SCHEMA §5): camelCase, no `deny_unknown_fields`; the
//! top-level [`Sketch`] and each [`RegionInfo`] carry an `extra` flatten so
//! document-level unknown keys round-trip. Entity/constraint enums are
//! internally tagged and do NOT preserve alien variants — see the forward-compat
//! notes in [`entity`] / [`constraint`]. Sketch-schema evolution is therefore
//! gated by the `sketch_freeze` snapshots (like `schema_freeze`).

pub mod constraint;
pub mod entity;
pub mod plane;
pub mod projection;

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::document::refs::{ElementRef, Extra};
use crate::ids::{ConstraintId, DatumPlaneId, EntityId, RegionId, SketchId};
use crate::math::Vec2;

pub use constraint::{Constraint, CurvePosition};
pub use entity::SketchEntity;
pub use plane::{plane_from_point_normal, SketchPlane};
pub use projection::{
    projected_sketch_content, FaceFrame, ProjectedEntity, ProjectedGeometry, ProjectedSource,
    ProjectionPayload,
};

/// A named world reference plane (SCHEMA §7.3 `plane.kind` ∈ `XY`|`XZ`|`YZ`).
/// The concrete basis is [`SketchPlane::xy`]/[`xz`](SketchPlane::xz)/
/// [`yz`](SketchPlane::yz). Serialized as the bare `"XY"`/`"XZ"`/`"YZ"` token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub enum WorldPlane {
    /// The XY plane.
    XY,
    /// The XZ plane.
    XZ,
    /// The YZ plane.
    YZ,
}

impl WorldPlane {
    /// The concrete (non-standard) coordinate frame for this named plane.
    #[must_use]
    pub const fn plane(self) -> SketchPlane {
        match self {
            Self::XY => SketchPlane::xy(),
            Self::XZ => SketchPlane::xz(),
            Self::YZ => SketchPlane::yz(),
        }
    }
}

/// How a sketch is attached to the model (what its plane is derived from).
///
/// Internally tagged on `"kind"` ∈ `datum` | `world` | `hostFace`.
// Size disparity is inherent (HostFace carries a rich typed `ElementRef`);
// attachments live inside a `Sketch` behind a `Vec` of sketches and are not
// moved in hot loops, so the payload is left unboxed (matches `Operation`).
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SketchAttachment {
    /// Bound to a document datum plane.
    Datum {
        /// The datum plane feature.
        datum: DatumPlaneId,
    },
    /// Bound to a named world plane (no datum feature).
    World {
        /// The world plane.
        plane: WorldPlane,
    },
    /// Bound to a solid face (C++ `HostFaceAttachment`).
    ///
    /// `face` is a typed [`ElementRef`] (identity + evidence + anchor) — richer
    /// than C++'s flat `{bodyId, faceId}` so the host face survives edits via
    /// the resolution ladder (mirrors the `FaceRef` decision in
    /// [`crate::document::refs`]). `projected_boundary_version` bumps whenever
    /// the host face's projected boundary edges are re-projected (C++
    /// `HostFaceAttachment::projectedBoundaryVersion`).
    HostFace {
        /// Reference to the host face.
        face: ElementRef,
        /// Version of the projected boundary edges (0 = not yet projected).
        projected_boundary_version: u32,
    },
}

/// Winding of a region loop — part of the deterministic [`derive_region_id`]
/// input. Outer loops are conventionally CCW, holes CW.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winding {
    /// Counter-clockwise (typical outer loop).
    Ccw,
    /// Clockwise (typical hole).
    Cw,
}

impl Winding {
    /// The discriminant byte fed into the region-id hash (STABLE).
    const fn hash_byte(self) -> u8 {
        match self {
            Self::Ccw => 0,
            Self::Cw => 1,
        }
    }
}

/// Derives a **deterministic, stable** [`RegionId`] from a loop's member entity
/// ids + winding.
///
/// Per the plan ("RegionId derivation ... coordinate w/ sidecar"): the id must
/// be reproducible from loop membership alone, so the Rust core and the C++
/// worker sidecar agree on region identity without shared mutable state.
///
/// **Algorithm (STABLE — changing it remaps every region id; fixture-gated):**
/// 1. Take each member's 16-byte UUID; sort the byte arrays ascending so the id
///    is independent of loop-member ordering.
/// 2. **FNV-1a 64-bit** over: every 16-byte UUID in sorted order, then one
///    winding byte (`0`=Ccw, `1`=Cw).
/// 3. Format as `"r_"` + 16 lowercase hex digits.
///
/// FNV-1a (not SHA-256) is chosen deliberately: it matches the OneCAD-CPP
/// ElementMap hashing family, needs no new dependency, and is fully
/// deterministic. 64 bits is ample here — a collision only causes a
/// recomputed-cache miss (regions are a cache, not authoritative identity), not
/// a correctness bug.
#[must_use]
pub fn derive_region_id(members: &[EntityId], winding: Winding) -> RegionId {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut uuids: Vec<[u8; 16]> = members.iter().map(|e| *e.as_uuid().as_bytes()).collect();
    uuids.sort_unstable();

    let mut hash = FNV_OFFSET;
    let mut mix = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    };
    for uuid in &uuids {
        for &byte in uuid {
            mix(byte);
        }
    }
    mix(winding.hash_byte());

    RegionId::new(format!("r_{hash:016x}"))
}

/// Cached closed-profile region of a sketch (outer loop + hole loops).
///
/// **CACHE, NOT AUTHORITATIVE.** Regions are derived by the worker's
/// `SketchRegions` (SCHEMA §7.4) from the entities/constraints; this is a
/// rebuildable projection, never a source of truth.
///
/// **NOT POPULATED IN V2.** Nothing in the shipped pipeline writes this — the
/// only caller of [`Sketch::set_regions`] is the serde freeze test. Every region
/// consumer asks the worker live instead: `finish_sketch` issues `SketchRegions`
/// per call and hands the result straight to the DTO, and the modelling tools
/// re-fetch on every arm (`ModelToolController`), so a persisted copy would only
/// be a second, staler source of truth for something already content-addressed
/// by `regionId`. The field and setter are retained because the document serde
/// schema is FROZEN (`tests/sketch_freeze.rs`) and v1 corpus samples carry the
/// key — removing it would be a schema change with no functional gain. Populate
/// it only alongside a decision about who invalidates it on edit.
///
/// **DISCREPANCY** (report): SCHEMA §7.4 `SketchRegions` names the wire fields
/// `regionId` / `outerLoop` / `holes`; this cache uses `id` / `outer` / `holes`
/// per the WP spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionInfo {
    /// Deterministic region identity (see [`derive_region_id`]).
    pub id: RegionId,
    /// Outer boundary loop, as an ordered list of member entity ids.
    pub outer: Vec<EntityId>,
    /// Hole loops (each an ordered list of member entity ids).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<EntityId>>,
    /// Unknown keys, preserved verbatim (SCHEMA §5).
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// Entities + constraints that reference some entity (returned by
/// [`Sketch::remove_entity`] / [`Sketch::dependents_of`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dependents {
    /// Entity ids whose geometry references the subject (they would dangle).
    pub entities: Vec<EntityId>,
    /// Constraint ids that reference the subject.
    pub constraints: Vec<ConstraintId>,
}

impl Dependents {
    /// True iff nothing references the subject.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.constraints.is_empty()
    }
}

/// The CHANGED parameters of one solver-registered curve (SCHEMA §7.4 `curves`),
/// applied by [`Sketch::apply_solved_curves`].
///
/// Every member is optional under the same incremental discipline as the
/// `positions` channel: the solver reports only what MOVED, so `None` means
/// "unchanged", never "zero". Angles are radians CCW from +X (§7.6), matching
/// [`SketchEntity::Arc`]'s own domain — nothing converts here.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CurveParams {
    /// New radius (mm) of a `Circle`/`Arc`.
    pub radius: Option<f64>,
    /// New start angle (radians) of an `Arc`.
    pub start_angle: Option<f64>,
    /// New end angle (radians) of an `Arc`.
    pub end_angle: Option<f64>,
}

/// A sketch mutation rejected by validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SketchError {
    /// An entity with this id already exists.
    #[error("duplicate entity id {0}")]
    DuplicateEntity(EntityId),
    /// A constraint with this id already exists.
    #[error("duplicate constraint id {0}")]
    DuplicateConstraint(ConstraintId),
    /// An entity references a point entity that is not in the sketch.
    #[error("entity {entity} references missing entity {missing}")]
    DanglingEntityRef {
        /// The offending entity.
        entity: EntityId,
        /// The missing referenced entity.
        missing: EntityId,
    },
    /// A constraint references an entity that is not in the sketch.
    #[error("constraint {constraint} references missing entity {missing}")]
    DanglingConstraintRef {
        /// The offending constraint.
        constraint: ConstraintId,
        /// The missing referenced entity.
        missing: EntityId,
    },
    /// A geometry-mutating edit named locked host-face reference geometry.
    ///
    /// Loud by design: the projected boundary must stay congruent with the model
    /// face it came from, so a partial edit is worse than a rejected one. Adding
    /// a CONSTRAINT against locked geometry is allowed (that is how a profile
    /// snaps to the face boundary) — only geometry mutation is refused.
    #[error("entity {0} is reference-locked (host-face geometry) and cannot be edited")]
    ReferenceLocked(EntityId),
}

/// Serde mirror of [`Sketch`] (the persisted fields only). Deserializing into
/// this and converting rebuilds the (non-serialized) lookup indices; see the
/// `#[serde(from/into)]` on [`Sketch`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SketchData {
    id: SketchId,
    name: String,
    plane: SketchPlane,
    attachment: SketchAttachment,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    entities: Vec<SketchEntity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    constraints: Vec<Constraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    regions: Vec<RegionInfo>,
    /// Projection provenance (WP-P). Additive and OMITTED when empty, so every
    /// pre-WP-P sketch serializes byte-identically and no schema version moves.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    projections: BTreeMap<EntityId, ProjectedSource>,
    #[serde(flatten, default, skip_serializing_if = "Extra::is_empty")]
    extra: Extra,
}

/// A 2D parametric sketch: plane + entities + constraints + a region cache.
///
/// `entities` / `constraints` are kept private and mutated only through the
/// validating API ([`Sketch::add_entity`] / [`add_constraint`](Sketch::add_constraint)
/// / [`remove_entity`](Sketch::remove_entity) / …) so a live `Sketch` never has
/// a duplicate id or (via `add_*`) a dangling reference. Lookup goes through
/// **rebuilt id→index maps that are NOT serialized** — they are reconstructed on
/// deserialize and maintained on every mutation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "SketchData", into = "SketchData")]
pub struct Sketch {
    /// Sketch identity.
    pub id: SketchId,
    /// Human-readable name.
    pub name: String,
    /// The coordinate frame (basis carried verbatim; see [`SketchPlane`]).
    pub plane: SketchPlane,
    /// What the sketch is attached to.
    pub attachment: SketchAttachment,
    /// Region cache (not authoritative — see [`RegionInfo`]).
    pub regions: Vec<RegionInfo>,
    /// Projected-entity provenance: `entity → the source it was projected from`
    /// plus the hash of the geometry it was projected TO (WP-P; SCHEMA §7.6
    /// `ProjectToSketchPlane`).
    ///
    /// **Only projected entities appear here**, and only the CURVES — a projected
    /// point is shared by the curves meeting at it, so it has no single source.
    /// An entity in this map is `reference_locked`; the converse does not hold
    /// (sketch-on-face boundary geometry is locked and unprojected).
    ///
    /// **Every key names a live entity.** Deserialization drops orphans (see
    /// [`From<SketchData>`]) and the edit layer re-filters after every batch, so a
    /// removed entity can never leave a row behind.
    pub projections: BTreeMap<EntityId, ProjectedSource>,
    /// Document-level unknown keys, preserved verbatim.
    pub extra: Extra,

    entities: Vec<SketchEntity>,
    constraints: Vec<Constraint>,
    // Rebuilt, never serialized (see SketchData / from/into).
    entity_index: HashMap<EntityId, usize>,
    constraint_index: HashMap<ConstraintId, usize>,
}

impl From<SketchData> for Sketch {
    fn from(d: SketchData) -> Self {
        let mut s = Self {
            id: d.id,
            name: d.name,
            plane: d.plane,
            attachment: d.attachment,
            regions: d.regions,
            projections: d.projections,
            extra: d.extra,
            entities: d.entities,
            constraints: d.constraints,
            entity_index: HashMap::new(),
            constraint_index: HashMap::new(),
        };
        // Deserialize trusts the stored file (a valid file has no dup/dangling
        // ids); validation is enforced on the mutation API, not on load.
        s.rebuild_indexes();
        // ORPHAN RECONCILE (WP-P). A row whose entity is gone is unusable — it can
        // be neither updated nor detached — and would make `update_projection`
        // re-project geometry into nothing. Dropping it here makes "every key names
        // a live entity" a type-level invariant for every deserialize, whatever the
        // source. The user-visible load diagnostic is minted separately, on the raw
        // value, by `io::migrate` (this conversion cannot report).
        s.projections
            .retain(|id, _| s.entity_index.contains_key(id));
        s
    }
}

impl From<Sketch> for SketchData {
    fn from(s: Sketch) -> Self {
        Self {
            id: s.id,
            name: s.name,
            plane: s.plane,
            attachment: s.attachment,
            entities: s.entities,
            constraints: s.constraints,
            regions: s.regions,
            projections: s.projections,
            extra: s.extra,
        }
    }
}

impl Sketch {
    /// An empty sketch on `plane` with the given `attachment`.
    #[must_use]
    pub fn new(id: SketchId, name: impl Into<String>, attachment: SketchAttachment) -> Self {
        let plane = match &attachment {
            SketchAttachment::World { plane } => plane.plane(),
            // Datum / host-face frames are derived later from the model; default
            // to XY until resolved.
            _ => SketchPlane::xy(),
        };
        Self {
            id,
            name: name.into(),
            plane,
            attachment,
            regions: Vec::new(),
            projections: BTreeMap::new(),
            extra: Extra::new(),
            entities: Vec::new(),
            constraints: Vec::new(),
            entity_index: HashMap::new(),
            constraint_index: HashMap::new(),
        }
    }

    /// An empty sketch on a named world plane (convenience for tests / defaults).
    #[must_use]
    pub fn on_world_plane(id: SketchId, name: impl Into<String>, plane: WorldPlane) -> Self {
        Self::new(id, name, SketchAttachment::World { plane })
    }

    /// The host FACE this sketch is glued to, or `None` for a world/datum
    /// attachment.
    ///
    /// The single reader of [`SketchAttachment::HostFace`]'s `face` used to stamp
    /// `SketchOpParams::host_face` — the timeline record's copy of the host-face
    /// dependency (VF-B5a). Kept here so the record-mint path (app crate) and the
    /// attachment-retarget path (edit session) can never disagree about what the
    /// host ref is.
    #[must_use]
    pub fn host_face(&self) -> Option<&ElementRef> {
        match &self.attachment {
            SketchAttachment::HostFace { face, .. } => Some(face),
            SketchAttachment::World { .. } | SketchAttachment::Datum { .. } => None,
        }
    }

    /// The entities, in authoritative order.
    #[must_use]
    pub fn entities(&self) -> &[SketchEntity] {
        &self.entities
    }

    /// The constraints, in authoritative order.
    #[must_use]
    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    /// Looks up an entity by id (via the rebuilt index).
    #[must_use]
    pub fn get_entity(&self, id: EntityId) -> Option<&SketchEntity> {
        self.entity_index.get(&id).map(|&i| &self.entities[i])
    }

    /// Looks up a constraint by id (via the rebuilt index).
    #[must_use]
    pub fn get_constraint(&self, id: ConstraintId) -> Option<&Constraint> {
        self.constraint_index
            .get(&id)
            .map(|&i| &self.constraints[i])
    }

    /// True iff an entity with `id` exists.
    #[must_use]
    pub fn contains_entity(&self, id: EntityId) -> bool {
        self.entity_index.contains_key(&id)
    }

    /// True iff a constraint with `id` exists.
    #[must_use]
    pub fn contains_constraint(&self, id: ConstraintId) -> bool {
        self.constraint_index.contains_key(&id)
    }

    /// Appends an entity, rejecting a duplicate id or a reference to a
    /// point-entity that is not already in the sketch (dangling ref).
    ///
    /// # Errors
    /// [`SketchError::DuplicateEntity`] or [`SketchError::DanglingEntityRef`].
    pub fn add_entity(&mut self, entity: SketchEntity) -> Result<(), SketchError> {
        let id = entity.id();
        if self.entity_index.contains_key(&id) {
            return Err(SketchError::DuplicateEntity(id));
        }
        for referenced in entity.referenced_entities() {
            if !self.entity_index.contains_key(&referenced) {
                return Err(SketchError::DanglingEntityRef {
                    entity: id,
                    missing: referenced,
                });
            }
        }
        let index = self.entities.len();
        self.entities.push(entity);
        self.entity_index.insert(id, index);
        Ok(())
    }

    /// Appends a constraint, rejecting a duplicate id or a reference to an
    /// entity that is not in the sketch (dangling ref).
    ///
    /// # Errors
    /// [`SketchError::DuplicateConstraint`] or
    /// [`SketchError::DanglingConstraintRef`].
    pub fn add_constraint(&mut self, constraint: Constraint) -> Result<(), SketchError> {
        let id = constraint.id();
        if self.constraint_index.contains_key(&id) {
            return Err(SketchError::DuplicateConstraint(id));
        }
        for referenced in constraint.entities() {
            if !self.entity_index.contains_key(&referenced) {
                return Err(SketchError::DanglingConstraintRef {
                    constraint: id,
                    missing: referenced,
                });
            }
        }
        let index = self.constraints.len();
        self.constraints.push(constraint);
        self.constraint_index.insert(id, index);
        Ok(())
    }

    /// Removes an entity and returns what referenced it, or `None` if absent.
    ///
    /// Removal does **not** auto-cascade — the returned [`Dependents`] lets the
    /// edit layer decide whether to cascade (mirrors C++ `Sketch::removeEntity`
    /// reporting; keeps edit *policy* out of the domain model). Because a
    /// removal can leave dangling references, callers should resolve the
    /// dependents.
    pub fn remove_entity(&mut self, id: EntityId) -> Option<Dependents> {
        let index = *self.entity_index.get(&id)?;
        let dependents = self.dependents_of(id);
        self.entities.remove(index);
        self.rebuild_entity_index();
        Some(dependents)
    }

    /// Removes a constraint. Returns `true` if one was removed.
    pub fn remove_constraint(&mut self, id: ConstraintId) -> bool {
        if let Some(&index) = self.constraint_index.get(&id) {
            self.constraints.remove(index);
            self.rebuild_constraint_index();
            true
        } else {
            false
        }
    }

    /// Entities + constraints that reference `id` (would dangle on its removal).
    #[must_use]
    pub fn dependents_of(&self, id: EntityId) -> Dependents {
        let entities = self
            .entities
            .iter()
            .filter(|e| e.referenced_entities().contains(&id))
            .map(SketchEntity::id)
            .collect();
        let constraints = self
            .constraints
            .iter()
            .filter(|c| c.entities().contains(&id))
            .map(Constraint::id)
            .collect();
        Dependents {
            entities,
            constraints,
        }
    }

    /// Replaces the region cache (recomputed by the worker's `SketchRegions`).
    /// Regions are a cache, never authoritative — see [`RegionInfo`], which also
    /// records why nothing in the shipped pipeline calls this.
    pub fn set_regions(&mut self, regions: Vec<RegionInfo>) {
        self.regions = regions;
    }

    /// Applies solver-returned point positions (the solver-lane drag/solve
    /// write-back — SCHEMA §7.4 `SolveDrag`/`EndGesture` `positions`). Only
    /// [`Point`](SketchEntity::Point) entities carry coordinates, so only points
    /// are moved; a non-point or unknown id is ignored (lines/arcs/circles follow
    /// their referenced points). Returns the number of points actually moved.
    ///
    /// This is the single-writer application step: the app clones the sketch
    /// before a gesture, drives the solver lane, then applies the final exact
    /// positions here to build the `after` sketch it commits as **one** undo
    /// command (plan "Pick tokens" / "Solver lane in V1").
    pub fn apply_solved_positions(&mut self, positions: &[(EntityId, Vec2)]) -> usize {
        let mut moved = 0;
        for &(id, pos) in positions {
            if let Some(&index) = self.entity_index.get(&id) {
                if let SketchEntity::Point { at, .. } = &mut self.entities[index] {
                    *at = pos;
                    moved += 1;
                }
            }
        }
        moved
    }

    /// Applies solver-returned CURVE parameters (SCHEMA §7.4 `SolveDrag`/
    /// `EndGesture` `curves`) — the companion channel to
    /// [`apply_solved_positions`](Self::apply_solved_positions).
    ///
    /// `positions` is point-only, which is not the whole result of a drag: a
    /// `radius` gesture moves no point at all, an `arcEnd` gesture reshapes the
    /// arc's radius + angles, and a `Tangent` propagates even a plain point drag
    /// into a neighbouring curve's radius. Only [`Circle`](SketchEntity::Circle)
    /// (radius) and [`Arc`](SketchEntity::Arc) (radius + both angles) carry such
    /// parameters; an [`Ellipse`](SketchEntity::Ellipse) is never solver-registered
    /// and is ignored here too, as are non-curve and unknown ids. A non-finite
    /// member is DROPPED (never written — [`SketchEntity::arc`]/[`circle`] reject
    /// non-finite at construction, so the invariant must hold here as well), and a
    /// radius is clamped at ≥ 0 (mirrors C++ `SketchArc::setRadius`).
    ///
    /// Returns the number of ENTITIES that changed (not members).
    ///
    /// [`circle`]: SketchEntity::circle
    pub fn apply_solved_curves(&mut self, curves: &[(EntityId, CurveParams)]) -> usize {
        fn finite(v: Option<f64>) -> Option<f64> {
            v.filter(|x| x.is_finite())
        }
        let mut changed = 0;
        for (id, params) in curves {
            let Some(&index) = self.entity_index.get(id) else {
                continue;
            };
            let mut touched = false;
            match &mut self.entities[index] {
                SketchEntity::Circle { radius, .. } => {
                    if let Some(r) = finite(params.radius) {
                        *radius = r.max(0.0);
                        touched = true;
                    }
                }
                SketchEntity::Arc {
                    radius,
                    start_angle,
                    end_angle,
                    ..
                } => {
                    if let Some(r) = finite(params.radius) {
                        *radius = r.max(0.0);
                        touched = true;
                    }
                    if let Some(a) = finite(params.start_angle) {
                        *start_angle = a;
                        touched = true;
                    }
                    if let Some(a) = finite(params.end_angle) {
                        *end_angle = a;
                        touched = true;
                    }
                }
                _ => {}
            }
            if touched {
                changed += 1;
            }
        }
        changed
    }

    fn rebuild_indexes(&mut self) {
        self.rebuild_entity_index();
        self.rebuild_constraint_index();
    }

    fn rebuild_entity_index(&mut self) {
        self.entity_index = self
            .entities
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id(), i))
            .collect();
    }

    fn rebuild_constraint_index(&mut self) {
        self.constraint_index = self
            .constraints
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id(), i))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec2;
    use uuid::Uuid;

    fn sid(n: u128) -> SketchId {
        SketchId(Uuid::from_u128(n))
    }
    fn eid(n: u128) -> EntityId {
        EntityId(Uuid::from_u128(n))
    }
    fn cid(n: u128) -> ConstraintId {
        ConstraintId(Uuid::from_u128(n))
    }

    fn sketch_with_two_points() -> (Sketch, EntityId, EntityId) {
        let mut s = Sketch::on_world_plane(sid(1), "Sketch 1", WorldPlane::XY);
        let (p0, p1) = (eid(0x10), eid(0x11));
        s.add_entity(SketchEntity::point(
            p0,
            Vec2::new_unchecked(0.0, 0.0),
            false,
            false,
        ))
        .unwrap();
        s.add_entity(SketchEntity::point(
            p1,
            Vec2::new_unchecked(40.0, 0.0),
            false,
            false,
        ))
        .unwrap();
        (s, p0, p1)
    }

    #[test]
    fn add_entity_rejects_duplicate_id() {
        let (mut s, p0, _) = sketch_with_two_points();
        let dup = SketchEntity::point(p0, Vec2::new_unchecked(1.0, 1.0), false, false);
        assert_eq!(s.add_entity(dup), Err(SketchError::DuplicateEntity(p0)));
    }

    #[test]
    fn add_entity_rejects_dangling_point_ref() {
        let (mut s, p0, _) = sketch_with_two_points();
        let missing = eid(0xDEAD);
        let line = SketchEntity::line(eid(0x20), p0, missing, false);
        assert_eq!(
            s.add_entity(line),
            Err(SketchError::DanglingEntityRef {
                entity: eid(0x20),
                missing,
            })
        );
    }

    #[test]
    fn add_constraint_rejects_dangling_ref_and_duplicate() {
        let (mut s, p0, p1) = sketch_with_two_points();
        // dangling: references a non-existent entity.
        let missing = eid(0xBEEF);
        let bad = Constraint::Coincident {
            id: cid(1),
            point1: p0,
            point2: missing,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        };
        assert_eq!(
            s.add_constraint(bad),
            Err(SketchError::DanglingConstraintRef {
                constraint: cid(1),
                missing,
            })
        );
        // valid.
        let good = Constraint::Coincident {
            id: cid(2),
            point1: p0,
            point2: p1,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        };
        s.add_constraint(good.clone()).unwrap();
        // duplicate id.
        assert_eq!(
            s.add_constraint(good),
            Err(SketchError::DuplicateConstraint(cid(2)))
        );
    }

    #[test]
    fn remove_entity_reports_dependents() {
        let (mut s, p0, p1) = sketch_with_two_points();
        let line = eid(0x20);
        s.add_entity(SketchEntity::line(line, p0, p1, false))
            .unwrap();
        s.add_constraint(Constraint::Horizontal { id: cid(1), line })
            .unwrap();
        // Removing p0: the line references it, the constraint references the line
        // (not p0 directly) — so p0's direct dependents are just the line.
        let deps = s.remove_entity(p0).unwrap();
        assert_eq!(deps.entities, vec![line]);
        assert!(deps.constraints.is_empty());
        assert!(!s.contains_entity(p0));
        // Index stayed consistent after the shift-remove.
        assert!(s.get_entity(p1).is_some());
        assert!(s.get_entity(line).is_some());
    }

    #[test]
    fn lookup_index_survives_serde_round_trip() {
        let (mut s, p0, p1) = sketch_with_two_points();
        s.add_entity(SketchEntity::line(eid(0x20), p0, p1, false))
            .unwrap();
        let json = serde_json::to_string(&s).unwrap();
        // Index is NOT in the JSON.
        assert!(!json.contains("entity_index"));
        assert!(!json.contains("entityIndex"));
        let back: Sketch = serde_json::from_str(&json).unwrap();
        // Rebuilt index works.
        assert!(back.get_entity(p0).is_some());
        assert!(back.get_entity(eid(0x20)).is_some());
        assert_eq!(s, back);
    }

    #[test]
    fn derive_region_id_is_order_independent_and_winding_sensitive() {
        let a = eid(0xA1);
        let b = eid(0xB2);
        let c = eid(0xC3);
        let id1 = derive_region_id(&[a, b, c], Winding::Ccw);
        let id2 = derive_region_id(&[c, a, b], Winding::Ccw);
        assert_eq!(id1, id2, "region id must be independent of member order");
        let id3 = derive_region_id(&[a, b, c], Winding::Cw);
        assert_ne!(id1, id3, "winding must change the region id");
        assert!(id1.as_str().starts_with("r_"), "wire form is r_<hex>");
        assert_eq!(id1.as_str().len(), 2 + 16, "r_ + 16 hex digits");
        // Stability lock: exact value must not drift silently.
        let stable = derive_region_id(&[eid(1), eid(2)], Winding::Ccw);
        assert_eq!(stable.as_str(), "r_fbf1e34acfb51ba4");
    }

    #[test]
    fn apply_solved_positions_moves_points_only() {
        let (mut s, p0, p1) = sketch_with_two_points();
        let line = eid(0x20);
        s.add_entity(SketchEntity::line(line, p0, p1, false))
            .unwrap();
        // Move p0; a line id (non-point) and an unknown id are ignored.
        let moved = s.apply_solved_positions(&[
            (p0, Vec2::new_unchecked(7.0, 8.0)),
            (line, Vec2::new_unchecked(99.0, 99.0)),
            (eid(0xDEAD), Vec2::new_unchecked(1.0, 1.0)),
        ]);
        assert_eq!(moved, 1, "only the point moved");
        match s.get_entity(p0).unwrap() {
            SketchEntity::Point { at, .. } => assert_eq!([at.x, at.y], [7.0, 8.0]),
            _ => panic!("p0 is a point"),
        }
        // p1 untouched.
        match s.get_entity(p1).unwrap() {
            SketchEntity::Point { at, .. } => assert_eq!([at.x, at.y], [40.0, 0.0]),
            _ => panic!("p1 is a point"),
        }
    }

    /// A sketch carrying one circle + one arc (each with its own center point),
    /// returning `(sketch, circle, arc)`.
    fn sketch_with_curves() -> (Sketch, EntityId, EntityId) {
        let (mut s, _, _) = sketch_with_two_points();
        let (cc, ac) = (eid(0x30), eid(0x31));
        let (circle, arc) = (eid(0x40), eid(0x41));
        s.add_entity(SketchEntity::point(
            cc,
            Vec2::new_unchecked(10.0, 10.0),
            false,
            false,
        ))
        .unwrap();
        s.add_entity(SketchEntity::point(
            ac,
            Vec2::new_unchecked(50.0, 0.0),
            false,
            false,
        ))
        .unwrap();
        s.add_entity(SketchEntity::circle(circle, cc, 5.0, false).unwrap())
            .unwrap();
        s.add_entity(
            SketchEntity::arc(arc, ac, 4.0, 0.0, std::f64::consts::FRAC_PI_2, false).unwrap(),
        )
        .unwrap();
        (s, circle, arc)
    }

    fn circle_radius(s: &Sketch, id: EntityId) -> f64 {
        match s.get_entity(id).unwrap() {
            SketchEntity::Circle { radius, .. } => *radius,
            other => panic!("{other:?} is not a circle"),
        }
    }

    fn arc_params(s: &Sketch, id: EntityId) -> (f64, f64, f64) {
        match s.get_entity(id).unwrap() {
            SketchEntity::Arc {
                radius,
                start_angle,
                end_angle,
                ..
            } => (*radius, *start_angle, *end_angle),
            other => panic!("{other:?} is not an arc"),
        }
    }

    #[test]
    fn apply_solved_curves_writes_circle_radius_and_arc_members() {
        let (mut s, circle, arc) = sketch_with_curves();
        let changed = s.apply_solved_curves(&[
            (
                circle,
                CurveParams {
                    radius: Some(12.5),
                    ..Default::default()
                },
            ),
            (
                arc,
                CurveParams {
                    radius: Some(9.0),
                    start_angle: Some(0.25),
                    end_angle: Some(1.75),
                },
            ),
        ]);
        assert_eq!(changed, 2, "both curves changed");
        assert_eq!(circle_radius(&s, circle), 12.5);
        assert_eq!(arc_params(&s, arc), (9.0, 0.25, 1.75));
    }

    #[test]
    fn apply_solved_curves_applies_only_the_members_present() {
        let (mut s, _, arc) = sketch_with_curves();
        // Only startAngle changed (the incremental discipline: None ≠ 0).
        let changed = s.apply_solved_curves(&[(
            arc,
            CurveParams {
                start_angle: Some(-0.5),
                ..Default::default()
            },
        )]);
        assert_eq!(changed, 1);
        assert_eq!(
            arc_params(&s, arc),
            (4.0, -0.5, std::f64::consts::FRAC_PI_2),
            "radius + endAngle keep their authored values"
        );
        // An entry with nothing set touches nothing and is not counted.
        assert_eq!(s.apply_solved_curves(&[(arc, CurveParams::default())]), 0);
    }

    #[test]
    fn apply_solved_curves_clamps_radius_and_drops_non_finite() {
        let (mut s, circle, arc) = sketch_with_curves();
        // A negative radius clamps at 0 rather than storing a negative curve.
        assert_eq!(
            s.apply_solved_curves(&[(
                circle,
                CurveParams {
                    radius: Some(-3.0),
                    ..Default::default()
                }
            )]),
            1
        );
        assert_eq!(circle_radius(&s, circle), 0.0);
        // Non-finite members are dropped — the entity keeps its finite values and
        // is not counted as changed.
        let changed = s.apply_solved_curves(&[(
            arc,
            CurveParams {
                radius: Some(f64::NAN),
                start_angle: Some(f64::INFINITY),
                end_angle: Some(f64::NEG_INFINITY),
            },
        )]);
        assert_eq!(changed, 0, "an all-non-finite entry changes nothing");
        assert_eq!(arc_params(&s, arc), (4.0, 0.0, std::f64::consts::FRAC_PI_2));
    }

    #[test]
    fn apply_solved_curves_ignores_non_curves_and_unknown_ids() {
        let (mut s, p0, p1) = sketch_with_two_points();
        let line = eid(0x20);
        s.add_entity(SketchEntity::line(line, p0, p1, false))
            .unwrap();
        let ell = eid(0x50);
        s.add_entity(SketchEntity::ellipse(ell, p0, 6.0, 3.0, 0.25, false).unwrap())
            .unwrap();
        let radius = CurveParams {
            radius: Some(99.0),
            ..Default::default()
        };
        let changed = s.apply_solved_curves(&[
            (line, radius),
            (p0, radius),
            (ell, radius),
            (eid(0xDEAD), radius),
        ]);
        assert_eq!(changed, 0, "line/point/ellipse/unknown are all ignored");
        // The ellipse is never solver-registered — its radii must stay verbatim.
        match s.get_entity(ell).unwrap() {
            SketchEntity::Ellipse {
                major_r, minor_r, ..
            } => assert_eq!([*major_r, *minor_r], [6.0, 3.0]),
            other => panic!("{other:?} is not an ellipse"),
        }
    }

    #[test]
    fn attachment_serde_shapes() {
        let host = SketchAttachment::HostFace {
            face: ElementRef {
                primary: None,
                intent: None,
                anchor: None,
                extra: Default::default(),
            },
            projected_boundary_version: 3,
        };
        let v = serde_json::to_value(&host).unwrap();
        assert_eq!(v["kind"], serde_json::json!("hostFace"));
        assert_eq!(v["projectedBoundaryVersion"], serde_json::json!(3));

        let world: SketchAttachment =
            serde_json::from_value(serde_json::json!({ "kind": "world", "plane": "XZ" })).unwrap();
        assert_eq!(
            world,
            SketchAttachment::World {
                plane: WorldPlane::XZ
            }
        );
    }

    /// WP-P: an empty `projections` map is OMITTED, so every pre-WP-P sketch
    /// serializes byte-identically and no schema version has to move.
    #[test]
    fn an_empty_projections_map_is_omitted_from_the_json() {
        let (s, _, _) = sketch_with_two_points();
        let v = serde_json::to_value(&s).unwrap();
        assert!(
            v.get("projections").is_none(),
            "empty projections must not appear on the wire: {v}"
        );
    }

    /// WP-P: a populated map round-trips, keyed by entity id.
    #[test]
    fn a_populated_projections_map_round_trips() {
        let (mut s, p0, _) = sketch_with_two_points();
        s.projections.insert(
            p0,
            ProjectedSource {
                source_body: crate::ids::BodyId(Uuid::from_u128(0xB0D1)),
                source_element_id: crate::ids::ElementId::new("el_edge_7"),
                source_kind: crate::document::refs::ElementKind::Edge,
                source_ordinal: 0,
                projected_hash: "9f2c4d1e77a0b355".into(),
            },
        );
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            v["projections"][p0.to_string()]["sourceElementId"],
            serde_json::json!("el_edge_7")
        );
        let back: Sketch = serde_json::from_value(v).unwrap();
        assert_eq!(back.projections, s.projections);
    }

    /// WP-P orphan reconcile: a row naming an entity the sketch no longer has is
    /// DROPPED on deserialize. An orphan can be neither updated nor detached, and
    /// it would make the staleness pass re-project into nothing.
    #[test]
    fn a_projection_row_for_a_missing_entity_is_dropped_on_load() {
        let (mut s, p0, _) = sketch_with_two_points();
        let source = ProjectedSource {
            source_body: crate::ids::BodyId(Uuid::from_u128(0xB0D1)),
            source_element_id: crate::ids::ElementId::new("el_edge_7"),
            source_kind: crate::document::refs::ElementKind::Edge,
            source_ordinal: 0,
            projected_hash: "9f2c4d1e77a0b355".into(),
        };
        s.projections.insert(p0, source.clone());
        let mut v = serde_json::to_value(&s).unwrap();
        // Plant a row for an entity that is not in `entities`.
        v["projections"][eid(0xDEAD).to_string()] = serde_json::to_value(&source).unwrap();

        let back: Sketch = serde_json::from_value(v).unwrap();
        assert_eq!(back.projections.len(), 1, "the orphan row is dropped");
        assert!(back.projections.contains_key(&p0));
        assert!(!back.projections.contains_key(&eid(0xDEAD)));
    }
}
