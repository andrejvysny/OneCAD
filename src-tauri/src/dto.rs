//! Frontend-facing projection DTOs (camelCase serde).
//!
//! These MIRROR the zustand store shapes the frontend already renders from
//! (`src/stores/documentStore.ts` — `DocumentProjection`/`BodyMeta`/`SketchMeta`/
//! `FeatureMeta` — and `src/ipc/types.ts`). Projection stores are written **only
//! by backend events** (plan "Frontend owns projection stores"); the app crate
//! mints these DTOs from the authoritative [`onecad_core`] document + the latest
//! regen [`ModelSnapshot`], and hands them to the webview via commands + events.
//!
//! The DTO layer lives in the app crate so `onecad-core` stays tauri-free and its
//! frozen file-format serde is never coupled to a UI wire shape.

use serde::{Deserialize, Serialize};

use onecad_core::document::record::{KnownOperation, OffsetDistanceType, Operation};
use onecad_core::history::StepState;

/// Whether a document is open (`src/stores/documentStore.ts` `DocStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DocStatus {
    /// No document open.
    Empty,
    /// A document is loading.
    Loading,
    /// A document is open and ready.
    Ready,
}

/// One body in the tree (`documentStore.ts` `BodyMeta`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyDto {
    pub id: String,
    pub name: String,
    pub visible: bool,
    /// User-authored body color as sRGB+A (`[r,g,b,a]`). `None` means "use the
    /// theme's neutral body fill".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 4]>,
    /// User-authored per-face colors keyed by persistent `ElementId`. Omitted when
    /// empty so the wire stays lean for bodies without face colors.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub face_colors: std::collections::BTreeMap<String, [u8; 4]>,
}

/// Sketch solve status (`documentStore.ts` `SketchStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SketchStatus {
    Ok,
    Under,
    Over,
    Error,
}

/// The model face a sketch is hosted on (`types.ts` `SketchHostFace`).
///
/// Projected from [`SketchAttachment::HostFace`](onecad_core::sketch::SketchAttachment)
/// so the frontend can answer "is there already a sketch on this face?" without a
/// round-trip — the double-click-a-face re-entry (SKETCH-ON-FACE W3). Identity
/// ONLY: the frozen basis, the anchor and the worker-owned `intent` descriptor are
/// deliberately NOT projected (the sketch's own `plane` is the basis authority,
/// and the descriptor is evidence the core never interprets).
///
/// `body_id`/`element_id` render exactly as [`BodyDto::id`] and a promoted
/// `elementId` do, so a face pick's promoted id compares `==` against this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchHostFaceDto {
    pub body_id: String,
    pub element_id: String,
}

/// One sketch in the tree (`documentStore.ts` `SketchMeta`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchDto {
    pub id: String,
    pub name: String,
    pub visible: bool,
    pub dof: u32,
    pub status: SketchStatus,
    /// Deterministic hash of plane, attachment, entities, and constraints. The
    /// frontend uses it to invalidate static profile fills without rebuilding
    /// every sketch for unrelated document revisions.
    pub geometry_token: String,
    /// The host face, for a `HostFace`-attached sketch that carries a `primary`
    /// binding. Omitted entirely for world- and datum-hosted sketches, so their
    /// projection rows stay byte-identical to before this field existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_face: Option<SketchHostFaceDto>,
}

/// Feature-timeline entry kind (`documentStore.ts` `FeatureKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FeatureKind {
    Sketch,
    Extrude,
    Revolve,
    Fillet,
    Boolean,
}

/// Feature regen status (`documentStore.ts` `FeatureStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FeatureStatus {
    Ok,
    Dirty,
    Error,
    NeedsRepair,
}

/// One feature-timeline entry (`documentStore.ts` `FeatureMeta`; identical shape
/// to `types.ts` `FeatureRecord` so a controller maps it 1:1).
// Not `Eq`: `primary_value` is an `f64` (same reason as `DatumDto`/`DocumentProjection`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureDto {
    pub id: String,
    pub kind: FeatureKind,
    /// The exact `opType` this feature was authored as (`"Extrude"`, `"Chamfer"`,
    /// …). [`FeatureKind`] is a coarse icon bucket that cannot distinguish
    /// Fillet from Chamfer or a Linear Pattern from a Boolean, so a re-edit
    /// cannot route on it. Additive; consumers that only draw an icon may ignore
    /// it. `"Opaque"` for a frozen unknown node.
    pub op_type: String,
    pub label: String,
    /// Mono value shown on the right of the history chip (e.g. `"25.0 mm"`).
    ///
    /// MILLIMETRE-FIXED and always produced, whatever the frontend's display-unit
    /// preference: three frontend consumers parse it back as mm to seed a re-edit
    /// (`filletRadius.radiusFromValueText`, `shellThickness.thicknessFromValueText`,
    /// `patternPreview.countFromValueText`). [`FeatureDto::primary_value`] is the
    /// NUMBER a unit-aware editor renders; this stays the document-domain text.
    pub value_text: String,
    /// The ONE dimension the history row edits in place (H3), in the **document
    /// domain**: millimetres for a length/diameter, degrees for an angle.
    ///
    /// Additive and OPTIONAL: `None` for every feature with no single primary
    /// dimension (Sketch / Boolean / Move / patterns / mirror / import / opaque),
    /// which is exactly the set whose row stays read-only. Minted in the same
    /// match as [`value_text`](FeatureDto::value_text) — see [`feature_value`] —
    /// so the two can never drift.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_value: Option<f64>,
    /// Domain tag for [`FeatureDto::primary_value`]: `"length"` | `"angle"` |
    /// `"diameter"`. Drives the inline editor's suffix (unit-aware vs `°`) and the
    /// `Ø` prefix a hole's row carries. Absent whenever `primary_value` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_value_kind: Option<String>,
    pub status: FeatureStatus,
    /// The worker failure reason for an errored step (`StepState::Error{reason}`),
    /// surfaced to the HistoryList row tooltip (Codex MAJOR-4). Absent for any
    /// non-error status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    /// Whether the op is suppressed — an axis ORTHOGONAL to [`FeatureStatus`],
    /// sourced from the RECORD flag (`OperationRecord::suppressed`), never from a
    /// [`StepState`]. Always serialized so the frontend can drop its optimistic
    /// suppression overlay and read the backend truth.
    pub suppressed: bool,
}

/// One datum plane in the tree (`documentStore.ts` `DatumMeta`).
///
/// The definition fields (`kind`/`basePlaneId`/`offset`) are what the user
/// authored; `plane` is the **core-resolved** frame and is authoritative — a
/// sketch attached to this datum is stamped with exactly this basis
/// (`DocumentSession::stamp_datum_plane`), so the frontend must render/preview
/// from `plane` and never re-derive it from the definition.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatumDto {
    pub id: String,
    pub name: String,
    /// PascalCase `DatumKind` token (`"OffsetFromPlane"` …) — the same spelling
    /// the core serde emits (`DatumKind::name`, lock-tested there).
    pub kind: String,
    /// `"XY"`/`"XZ"`/`"YZ"` or another datum's id.
    pub base_plane_id: String,
    pub offset: f64,
    pub plane: SketchPlaneDto,
    /// `false` ⇒ the definition could not be resolved; the datum exists in the
    /// tree but cannot host a sketch (`AddSketch` rejects it).
    pub resolved_valid: bool,
}

/// The full document projection (`documentStore.ts` `DocumentProjection`).
///
/// `bodies`/`sketches`/`datums` serialize as JSON objects keyed by id (the
/// store's `Record<string, …>`); a `BTreeMap` keeps the key order deterministic.
// Not `Eq`: `DatumDto` carries `f64` (offset + the resolved basis).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentProjection {
    pub status: DocStatus,
    /// The document this projection describes. Revisions restart at 1 for every
    /// newly opened runtime, so the frontend's stale-projection guard MUST scope
    /// its revision compare to one document — comparing revisions across
    /// documents drops every projection of a freshly opened one. Empty for the
    /// "no document" projection.
    pub document_id: String,
    pub revision: u64,
    pub title: String,
    pub dirty: bool,
    pub bodies: std::collections::BTreeMap<String, BodyDto>,
    pub sketches: std::collections::BTreeMap<String, SketchDto>,
    /// Datum planes, keyed by id. Core-owned state — datums never cross the OCW1
    /// wire (SCHEMA §7.3: only the RESOLVED basis reaches the worker, as a
    /// `custom` sketch plane), so this projection is their only route to the UI.
    pub datums: std::collections::BTreeMap<String, DatumDto>,
    pub features: Vec<FeatureDto>,
    /// Applied op count (timeline cursor): `features[0, appliedOps)` are applied,
    /// `[appliedOps, totalOps)` are drafts beyond the rollback bar. Drives the
    /// legacy-draft recovery hint (`appliedOps < totalOps` ⇒ "N ops not applied").
    pub applied_ops: usize,
    /// Total op count (timeline length).
    pub total_ops: usize,
    /// Where the geometry the viewport is currently showing came from — the honest
    /// answer to "is this the real model?".
    ///
    /// * `"live"` — a regen has published; the meshes are this document's geometry.
    /// * `"cached"` — no publish yet, but the container's LAST-SAVED meshes are
    ///   resident and being served. Correct as of the last save, and possibly
    ///   *older* than the timeline the tree is showing (the document may have been
    ///   saved, edited, saved again with an evicted cache, …). The UI must say so
    ///   rather than let the user measure or machine off it.
    /// * `"none"` — nothing to paint yet.
    pub geometry_source: String,
}

impl DocumentProjection {
    /// The projection for "no document open".
    #[must_use]
    pub fn empty() -> Self {
        Self {
            status: DocStatus::Empty,
            document_id: String::new(),
            revision: 0,
            title: String::new(),
            dirty: false,
            bodies: std::collections::BTreeMap::new(),
            sketches: std::collections::BTreeMap::new(),
            datums: std::collections::BTreeMap::new(),
            features: Vec::new(),
            applied_ops: 0,
            total_ops: 0,
            geometry_source: GEOMETRY_SOURCE_NONE.to_string(),
        }
    }
}

/// [`DocumentProjection::geometry_source`] — nothing to paint.
pub const GEOMETRY_SOURCE_NONE: &str = "none";
/// [`DocumentProjection::geometry_source`] — serving the container's last-saved
/// meshes while the open-regen runs.
pub const GEOMETRY_SOURCE_CACHED: &str = "cached";
/// [`DocumentProjection::geometry_source`] — a regen has published.
pub const GEOMETRY_SOURCE_LIVE: &str = "live";

/// A handle returned by open/new/close (`src/ipc/types.ts` `DocumentSnapshot`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSnapshotDto {
    pub document_id: String,
    pub title: String,
}

/// Authoritative state after a completed save (`src/ipc/types.ts` `SaveOutcome`).
///
/// A container write can race a later edit because it deliberately runs outside
/// the runtime lock. `clean` is therefore a result field, not an implication of
/// command success: close/replacement flows may proceed only when it is true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveOutcomeDto {
    pub document_id: String,
    pub saved_revision: u64,
    pub current_revision: u64,
    pub clean: bool,
    pub path: String,
    pub title: String,
}

/// One module's slice of the document (`types.ts` `ModuleState`).
///
/// The payload is opaque to everything but the owning module — it crosses this
/// boundary as arbitrary JSON and is never inspected on the way (ADR-0005).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleStateDto {
    /// Owning module/addon id (reverse-DNS).
    pub module_id: String,
    /// The schema version the OWNING module wrote this payload at.
    pub schema_version: u32,
    /// Module-owned data.
    pub payload: serde_json::Value,
}

/// What a document records about a module it carries state for (`types.ts`
/// `DocumentModule`). Enough to report a missing addon without decoding its data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentModuleDto {
    pub module_id: String,
    pub schema_version: u32,
}

/// One recent-project entry for the start screen (`types.ts` `RecentProject`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProjectDto {
    pub id: String,
    pub name: String,
    pub path: String,
    /// ISO-8601 last-modified timestamp.
    pub modified_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
}

/// A crash-recovery offer surfaced at startup (`check_recovery`; `types.ts`
/// `RecoveryInfo`). A previous session left an autosave whose owning process is
/// gone — the start screen offers to Restore or Discard it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryInfoDto {
    /// The document's real on-disk path, if it had ever been saved (absent for a
    /// never-saved autosave-only document).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_path: Option<String>,
    /// The autosave container to recover from (absolute path).
    pub autosave_path: String,
    /// The autosave's last-modified time, in Unix-epoch milliseconds.
    pub modified_ms: u64,
}

/// The `worker-status` event payload (`types.ts` `WorkerStatus`) — the sidecar
/// lifecycle the status bar surfaces. `state` is one of
/// `starting`|`ready`|`restarting`|`failed`; `epoch` is the worker epoch that
/// transition belongs to (`0` when unknown, e.g. terminal failure).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerStatusDto {
    pub state: String,
    pub epoch: u64,
}

/// One changed body in a `document-changed` event (`types.ts` `BodyMeshRef`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyMeshRef {
    pub body_id: String,
    /// Mirrors the Rust `MeshCache` key `(BodyId, Lod, generation)`, rendered
    /// `"<bodyId>:<lod>:<generation>"` (matches the mock's `mockMeshKey`).
    pub mesh_key: String,
}

/// The `document-changed` event payload (`types.ts` `DocumentChange`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentChange {
    pub revision: u64,
    /// The published [`ModelSnapshot`](onecad_core::regen::ModelSnapshot) id this
    /// geometry belongs to (SCHEMA §7.5). The frontend forwards it to
    /// `promoteSelection` so a picked TopoKey resolves against the exact snapshot
    /// the mesh was tessellated at (Invariant 4 — bodies/maps/meshes share one id).
    pub snapshot_id: u64,
    pub changed_bodies: Vec<BodyMeshRef>,
    pub removed_bodies: Vec<String>,
}

// ── Solver-lane DTOs (SCHEMA §7.4) — mirror `src/ipc/types.ts` sketch shapes ──
//
// These MIRROR the frontend `localSolver`/`types.ts` sketch shapes so the F-WP9
// swap (mock lane → real tauri commands) is a drop-in: `SketchSolveStatus` matches
// `types.ts SketchSolveStatus` (the four PascalCase tokens the worker's SketchUpsert
// `state` returns verbatim), `SketchSessionDto` == `SketchSession`, `SketchUpsertDto`
// == `SketchUpsertResult`, `SketchRegionDto` == `SketchRegion`.

/// Solver state (SCHEMA §7.4 `SketchUpsert.state`; `types.ts SketchSolveStatus`).
/// Serializes as the bare PascalCase token the worker emits (variant name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SketchSolveStatus {
    UnderConstrained,
    FullyConstrained,
    OverConstrained,
    Conflicting,
}

impl SketchSolveStatus {
    /// Parses the worker's `state` string; unknown ⇒ `UnderConstrained` (the safe
    /// "not solved" default).
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "FullyConstrained" => Self::FullyConstrained,
            "OverConstrained" => Self::OverConstrained,
            "Conflicting" => Self::Conflicting,
            _ => Self::UnderConstrained,
        }
    }

    /// The tree-projection [`SketchStatus`] for this solve state.
    #[must_use]
    pub fn tree_status(self) -> SketchStatus {
        match self {
            Self::FullyConstrained => SketchStatus::Ok,
            Self::UnderConstrained => SketchStatus::Under,
            Self::OverConstrained => SketchStatus::Over,
            Self::Conflicting => SketchStatus::Error,
        }
    }
}

/// A live sketch session (`enterSketch` result; `types.ts SketchSession`). The
/// `plane`/`entities`/`constraints` carry the SCHEMA §7.3/§7.4 wire JSON verbatim
/// (identical to the frontend wire form) so the seam is a drop-in.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchSessionDto {
    pub sketch_id: String,
    pub plane: serde_json::Value,
    pub entities: serde_json::Value,
    pub constraints: serde_json::Value,
    pub dof: u32,
    pub status: SketchSolveStatus,
    /// Constraint ids in conflict (SCHEMA §7.4; empty when the sketch is solvable).
    /// Threaded from the entering `SketchUpsert` solve so re-entering a conflicting
    /// sketch surfaces the offending constraints. Always serialized (no skip), so
    /// the frontend sees an explicit `conflicting: []` on a clean sketch.
    pub conflicting: Vec<String>,
}

/// A re-solve result (`sketchUpsert`/`endGesture`; `types.ts SketchUpsertResult`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchUpsertDto {
    pub sketch_id: String,
    pub sketch_revision: u64,
    pub dof: u32,
    pub status: SketchSolveStatus,
    /// Constraint ids in conflict (SCHEMA §7.4; empty when the sketch is solvable).
    /// The worker `SketchUpsert`/`EndGesture` result carries these; absent ⇒ empty
    /// at the wire parse (`str_array`, strict back/forward compat). Always
    /// serialized (no skip) so the frontend sees an explicit `conflicting: []`.
    pub conflicting: Vec<String>,
    /// CHANGED point coordinates after the solve, keyed by the point entity id.
    pub solved_positions: std::collections::BTreeMap<String, [f64; 2]>,
    /// CHANGED curve parameters after the solve (SCHEMA §7.4 `curves`), keyed by
    /// the curve entity id. Always serialized (no skip), same `conflicting: []`
    /// precedent — the frontend always sees the key, empty when nothing moved.
    pub solved_curves: std::collections::BTreeMap<String, CurveParamsDto>,
}

/// The CHANGED members of one curve in a solve result (SCHEMA §7.4 `curves`).
///
/// `positions` is point-only, which is not the whole result of a drag: a `radius`
/// gesture moves no point, an `arcEnd` gesture reshapes the arc's radius/angles,
/// and a `Tangent` propagates even a plain point drag into a neighbouring curve's
/// radius. Members are per-member optional under the same incremental discipline
/// as `positions` — an absent member is UNCHANGED, never zero — so each one skips
/// serialization when `None` (mirrors `types.ts CurveParams`). Angles are radians
/// CCW from +X, the wire domain (`angleUnits.ts` converts for display).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurveParamsDto {
    /// New radius (mm) of a `Circle`/`Arc`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    /// New start angle (radians) of an `Arc`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_angle: Option<f64>,
    /// New end angle (radians) of an `Arc`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_angle: Option<f64>,
}

/// A `BeginGesture` acknowledgement (SCHEMA §7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginGestureDto {
    pub gesture_id: u64,
    pub ready: bool,
}

/// One incremental drag solve (SCHEMA §7.4 `SolveDrag` result). `status` is the
/// worker token (`success`|`partial`|`conflicting`|`redundant`), plus the
/// client-side `superseded` terminal a stale `seq` may receive (latest-wins).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DragSolveDto {
    pub gesture_id: u64,
    pub seq: u64,
    pub status: String,
    pub dof: u32,
    pub conflicting: Vec<String>,
    /// CHANGED point coordinates, keyed by point entity id.
    pub positions: std::collections::BTreeMap<String, [f64; 2]>,
    /// CHANGED curve parameters (SCHEMA §7.4 `curves`), keyed by curve entity id.
    /// Always serialized (no skip), like `conflicting`.
    pub curves: std::collections::BTreeMap<String, CurveParamsDto>,
    pub solve_micros: u64,
    /// True when this `seq` was superseded by a newer drag (positions empty).
    pub superseded: bool,
}

/// One closed profile region (`finishSketch`; `types.ts SketchRegion`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchRegionDto {
    pub region_id: String,
    pub outer_loop: Vec<String>,
    pub holes: Vec<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_triangles: Option<PreviewTrianglesDto>,
}

/// A region's triangulated fill in plane (u,v) coordinates (`types.ts
/// SketchRegion.previewTriangles`): flat `positions` (u,v pairs) + `indices`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTrianglesDto {
    pub positions: Vec<f64>,
    pub indices: Vec<u32>,
    /// Number of required region holes actually removed from this fill. The
    /// frontend rejects a fill when this differs from `holes.len()`.
    pub holes_subtracted: u32,
}

/// One element's geometric evidence from `QueryElement` (SCHEMA §7.5).
///
/// `surface_type` is OCCT's `GeomAbs_SurfaceType` ordinal, where **0 == plane**
/// (`GeomAbs_Plane` is the first enumerator). Consumers that need a planar face —
/// sketch-on-face, the Extrude `ToFace` target — MUST check it rather than
/// assuming: a cylinder's `normal` is meaningless as a plane normal.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementInfoDto {
    pub element_id: String,
    pub topo_key: String,
    pub body_id: String,
    /// `"face"` | `"edge"` | `"vertex"` | `"body"`.
    pub kind: String,
    pub surface_type: i64,
    /// OCCT's `GeomAbs_CurveType` ordinal for an EDGE, where **0 == line**
    /// (`GeomAbs_Line` is the first enumerator). `-1` means the descriptor
    /// carried none — a missing value must never read as "this is a line".
    pub curve_type: i64,
    /// Bounding-box centre. For a PLANAR face this lies ON the plane (every point
    /// of the face is coplanar, so the box centre is too), which is what makes it
    /// a usable plane origin.
    ///
    /// It is a BOX centre, **not** a centroid — the kernel takes it from
    /// `Bnd_Box` (`ElementMap::computeDescriptor`). UI that reports a distance
    /// between two of these must say so ("center ↔ center"), never "centroid".
    pub center: [f64; 3],
    pub normal: [f64; 3],
    /// Whether the descriptor actually carries a normal (`hasNormal`).
    pub has_normal: bool,
    /// Bounding-box DIAGONAL length (`sqrt(dx²+dy²+dz²)`), the kernel's coarse
    /// size proxy. Not a measurement — `magnitude` is the exact quantity.
    pub size: f64,
    /// The element's EXACT measured quantity, straight from OCCT `GProp`:
    /// a face's **area** (`BRepGProp::SurfaceProperties`), an edge's **arc
    /// length** (`LinearProperties`), a solid's **volume** (`VolumeProperties`).
    /// `ElementMap::computeDescriptor` is the authority; this is not an estimate
    /// derived from the tessellation.
    pub magnitude: f64,
}

/// One body's exact kernel mass properties from `QueryMassProperties`
/// (SCHEMA §7.5; WP-C1).
///
/// **Density-free.** The app has no material system, so `principal_moments` are
/// pure VOLUME integrals (mm⁵) about the centroid — multiply by a density to get
/// a mass moment. Reporting kg·mm² here would require inventing a material.
///
/// Every number is `GProp` over the real BRep, never re-derived from the
/// tessellation the viewport happens to be drawing.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MassPropertiesDto {
    /// The body this reading describes (frontend `body<n>` form, as every other DTO).
    pub body_id: String,
    /// Enclosed volume, mm³ (`BRepGProp::VolumeProperties`). 0 for a shape with
    /// no solid — an honest reading, not a failure.
    pub volume: f64,
    /// Total area of every face, mm² (`BRepGProp::SurfaceProperties`).
    pub surface_area: f64,
    /// The VOLUME centre of mass, mm. Distinct from `ElementInfoDto::center`,
    /// which is a bounding-box centre — this one is a true centroid.
    pub centroid: [f64; 3],
    /// Principal moments about the centroid (mm⁵), paired IN ORDER with
    /// `principal_axes`.
    pub principal_moments: [f64; 3],
    /// The principal frame: three unit, right-handed, sign-canonical rows.
    pub principal_axes: [[f64; 3]; 3],
}

/// One face's snapshot-scoped EVIDENCE from `PrepareOffsetFace` (SCHEMA §7.6).
///
/// **Never an identity.** The verb returns `topoKey` + descriptor/anchor evidence
/// and nothing else, because minting `ElementId`s is Rust's alone (Invariant 2).
/// The caller promotes this through the `AcquireElementIds` path and only THEN
/// authors the typed refs a record can hold.
///
/// `anchor` and `descriptor` are opaque passthroughs — worker-owned payloads the
/// core never interprets, forwarded verbatim so nothing is lost in translation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OffsetFaceEvidenceDto {
    /// Snapshot-scoped ordinal key (`"f:22"`), valid only against
    /// [`PrepareOffsetFaceDto::snapshot_id`].
    pub topo_key: String,
    /// `true` for a face the USER picked, `false` for one the G1 tangent closure
    /// added. Always `false` on the `Total` opposite face — SCHEMA emits no
    /// `picked` key there, and the opposite face is never part of the operative
    /// set.
    pub picked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<serde_json::Value>,
}

/// The measurable dimensions that SEED an absolute distance type (SCHEMA §7.6
/// `currentDims`). A key is absent when the closure has no such reading — a
/// planar face has no `radius`, and a face with no unique opposite has no
/// `thickness`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OffsetCurrentDimsDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thickness: Option<f64>,
}

/// A `PrepareOffsetFace` REFUSAL (SCHEMA §7.6) — an answer with `ok:true`, not an
/// error. `code` ∈ `crossBody` | `unsupportedSurface` | `chainMismatch` |
/// `noUniqueOpposite` | `notPlanar` | `chainOnTotal` | `notCylindrical` |
/// `mixedAxis` | `nonManifold`; `faces` names the offending TopoKeys when the code
/// has any to name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OffsetFaceRefusalDto {
    pub code: String,
    pub message: String,
    pub faces: Vec<String>,
}

/// `prepare_offset_face` result — the read-only first half of the OffsetFace
/// authoring transaction (SCHEMA §7.6 `PrepareOffsetFace`).
///
/// Carries the FULL operative closure (picks ∪ G1 tangent chain) the record will
/// freeze, the `Total` opposite candidate, and the seeds for the absolute
/// distance types. Nothing here is fenced, prepared, accepted or minted by the
/// worker; `snapshot_id` is the head it ANSWERED against, echoed so the caller can
/// see it never moved.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareOffsetFaceDto {
    /// Echo of the head the worker answered against.
    pub snapshot_id: u64,
    /// The single body every picked face belongs to (`body_<uuid>` wire form, as
    /// the worker sent it). Empty on a refusal.
    pub target_body_id: String,
    /// The full operative closure in stable face-ordinal order. Empty on a
    /// refusal.
    pub faces: Vec<OffsetFaceEvidenceDto>,
    /// The `Total` opposite-face candidate; `None` for every other distance type
    /// and whenever no unique candidate passed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opposite_face: Option<OffsetFaceEvidenceDto>,
    pub current_dims: OffsetCurrentDimsDto,
    /// `Some` ⇒ the handshake REFUSED and `faces` is not a usable closure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<OffsetFaceRefusalDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeOpEvidenceDto {
    pub topo_key: String,
    pub picked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeOpRefusalDto {
    pub code: String,
    pub message: String,
    pub edges: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareEdgeOpDto {
    pub snapshot_id: u64,
    pub target_body_id: String,
    pub edges: Vec<EdgeOpEvidenceDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<EdgeOpRefusalDto>,
}

/// One previewed body's mesh (`PreviewOp` result → `types.ts PreviewResult`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewBodyDto {
    pub body_id: String,
    /// The assembled MESH1 blob for this body.
    pub mesh: Vec<u8>,
}

/// One lifecycle event produced by a throwaway preview candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewBodyEventDto {
    /// `"created"` | `"modified"` | `"deleted"`.
    pub kind: String,
    pub body_id: String,
}

/// `preview_op` result — the bodies the candidate op created or changed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResultDto {
    /// The HEAD snapshot the preview ran against. A preview creates no snapshot
    /// of its own, so this is the current head's id, not a new one.
    pub snapshot_id: u64,
    pub bodies: Vec<PreviewBodyDto>,
    pub body_events: Vec<PreviewBodyEventDto>,
    pub changed_bodies: Vec<String>,
    pub deleted_bodies: Vec<String>,
    /// Successful NeedsRepair state (never a protocol error).
    pub needs_repair: Vec<serde_json::Value>,
}

/// A resolved sketch plane basis (`types.ts SketchPlane`) — what the frontend
/// needs to place a sketch on a picked face or a datum.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchPlaneDto {
    pub origin: [f64; 3],
    pub x_axis: [f64; 3],
    pub y_axis: [f64; 3],
    pub normal: [f64; 3],
}

impl From<onecad_core::sketch::SketchPlane> for SketchPlaneDto {
    /// Carries the basis VERBATIM — never re-derived, never re-ordered (the
    /// non-standard `Sketch.h` bases are load-bearing; see `sketch/plane.rs`).
    fn from(p: onecad_core::sketch::SketchPlane) -> Self {
        Self {
            origin: [p.origin.x, p.origin.y, p.origin.z],
            x_axis: [p.x_axis.x, p.x_axis.y, p.x_axis.z],
            y_axis: [p.y_axis.x, p.y_axis.y, p.y_axis.z],
            normal: [p.normal.x, p.normal.y, p.normal.z],
        }
    }
}

/// `addSketchOnFace` result — the projected host-face sketch the backend just
/// committed (SCHEMA §7.6 `ProjectFaceBoundary` → `EditCommand::AddSketch`).
///
/// Deliberately lean: the sketch's full geometry arrives through the normal
/// `enterSketch` / `getSketch` path, so this carries only what the caller cannot
/// derive — the frozen frame plus enough counts to render a confirmation and to
/// assert the projection was not empty.
///
/// `sketch_id` echoes the id the CALLER minted (the id-adoption rule: the backend
/// `SketchId`, the frontend id and the projection-store key are one string).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchOnFaceDto {
    pub sketch_id: String,
    /// The frozen sketch frame, derived from the kernel-exact face plane.
    pub plane: SketchPlaneDto,
    /// Locked entities minted from the projection (points + curves).
    pub entity_count: u32,
    /// `Fixed` constraints minted — one per projected point.
    pub constraint_count: u32,
    /// Faces the projection covered: the seed face plus its coplanar companions
    /// (SCHEMA §7.6 `faceCount`).
    pub face_count: u32,
    /// Whether the projected geometry closes into at least one boundary. `false`
    /// is NOT an error — the sketch still exists, it just forms no region yet.
    pub has_closed_boundary: bool,
    /// The attachment's `projectedBoundaryVersion` after this projection.
    pub projected_boundary_version: u32,
}

/// `finishSketch` result (`types.ts FinishSketchResult`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishSketchDto {
    pub regions: Vec<SketchRegionDto>,
}

/// One promoted element (`promoteSelection`; SCHEMA §7.5 `AcquireElementIds`
/// result — Rust-minted `elementId`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotedElementDto {
    pub topo_key: String,
    pub element_id: String,
    pub kind: String,
    pub body_id: String,
}

/// One repair candidate surfaced to the M4b repair UI (`resolveRefs` →
/// `needsRepair`; SCHEMA §9 `candidates[]`). Carries the evidence handle
/// (`topoKey`), the normalized score + margin, a geometric hint (`worldPos` centre),
/// and a human-usable `summary` (e.g. `"planar face, area≈120mm²"`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveCandidateDto {
    pub topo_key: String,
    pub score: f64,
    pub margin: f64,
    /// Candidate centre in world coords — a geometric hint for highlighting.
    pub world_pos: [f64; 3],
    pub summary: String,
    /// Per-feature score contributions (SCHEMA §9 `featureContributions`), when the
    /// worker carried them (rides in the candidate's `extra`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_contributions: Option<serde_json::Value>,
}

/// One dry-run resolution (`resolveRefs`; SCHEMA §7.5 `ResolveRefs` result) — the
/// **full** ladder result the M4b repair UI consumes (un-lossy: the older DTO
/// dropped candidates/reason/anchor).
///
/// Per ref: `outcome` (status), `elementId` (the bound id — `autoBind`/`unchanged`,
/// or the last-known id on `needsRepair`), and — on `needsRepair` — the ranked
/// `candidates[]` plus `ladderFailed`/`reason`/`scoringVersion`/`uiLabel`/`anchor`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveRefDto {
    /// Exact published snapshot that enumerated this candidate set.
    pub snapshot_id: u64,
    /// Document revision current while the candidate set was enumerated.
    pub revision: u64,
    pub ref_id: String,
    /// Body used to enumerate the candidate TopoKeys, when the stored ref names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_id: Option<String>,
    /// `autoBind` | `needsRepair` | `unchanged`.
    pub outcome: String,
    /// The bound `ElementId` (empty ⇒ omitted). For `autoBind`/`unchanged` this is
    /// the resolved id; for `needsRepair` it is the ref's last-known id, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    /// The bound element's `TopoKey` evidence (SCHEMA §9) — present on `autoBind`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topo_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<f64>,
    /// `history` | `descriptor` — the ladder level that could not decide (needsRepair).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ladder_failed: Option<String>,
    /// `ambiguous` | `no-candidates` | `low-confidence` (needsRepair).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The `resolverVersion` the scores were computed under (SCHEMA §9).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoring_version: Option<u32>,
    /// UI-friendly label (SCHEMA §9 `uiLabel`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_label: Option<String>,
    /// The selection intent captured when the ref was authored (SCHEMA §9 `anchor`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<serde_json::Value>,
    /// Ranked candidates (needsRepair), sorted by score descending.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<ResolveCandidateDto>,
}

impl ResolveRefDto {
    /// Maps a core [`RefResolution`](onecad_core::regen::RefResolution) to the full
    /// repair-UI DTO (SCHEMA §7.5/§9). Un-lossy: the `needsRepair` path carries the
    /// ranked candidates + reason + anchor the old DTO dropped.
    #[must_use]
    pub fn from_resolution(
        r: onecad_core::regen::RefResolution,
        snapshot_id: u64,
        revision: u64,
        body_id: Option<String>,
    ) -> Self {
        use onecad_core::regen::ResolveOutcome;
        let non_empty = |id: onecad_core::ids::ElementId| {
            let s = id.as_str().to_string();
            (!s.is_empty()).then_some(s)
        };
        let base = |ref_id: String, outcome: &str| ResolveRefDto {
            snapshot_id,
            revision,
            ref_id,
            body_id: body_id.clone(),
            outcome: outcome.to_string(),
            element_id: None,
            topo_key: None,
            score: None,
            margin: None,
            ladder_failed: None,
            reason: None,
            scoring_version: None,
            ui_label: None,
            anchor: None,
            candidates: Vec::new(),
        };
        match r.outcome {
            ResolveOutcome::AutoBind {
                element_id,
                score,
                margin,
                topo_key,
            } => ResolveRefDto {
                element_id: non_empty(element_id),
                topo_key: topo_key.map(|k| k.as_str().to_string()),
                score: Some(score),
                margin: Some(margin),
                ..base(r.ref_id, "autoBind")
            },
            ResolveOutcome::Unchanged { element_id } => ResolveRefDto {
                element_id: element_id.and_then(non_empty),
                ..base(r.ref_id, "unchanged")
            },
            ResolveOutcome::NeedsRepair(item) => ResolveRefDto {
                element_id: item.element_id.and_then(non_empty),
                ladder_failed: Some(ladder_level_str(item.ladder_failed).to_string()),
                reason: Some(repair_reason_str(item.reason).to_string()),
                scoring_version: item.scoring_version,
                ui_label: (!item.ui_label.is_empty()).then_some(item.ui_label),
                anchor: item
                    .anchor
                    .as_ref()
                    .and_then(|a| serde_json::to_value(a).ok()),
                candidates: item.candidates.into_iter().map(candidate_dto).collect(),
                ..base(r.ref_id, "needsRepair")
            },
        }
    }
}

fn ladder_level_str(l: onecad_core::document::repair::LadderLevel) -> &'static str {
    use onecad_core::document::repair::LadderLevel;
    match l {
        LadderLevel::History => "history",
        LadderLevel::Descriptor => "descriptor",
    }
}

fn repair_reason_str(r: onecad_core::document::repair::RepairReason) -> &'static str {
    use onecad_core::document::repair::RepairReason;
    match r {
        RepairReason::Ambiguous => "ambiguous",
        RepairReason::NoCandidates => "no-candidates",
        RepairReason::LowConfidence => "low-confidence",
        // VF-B6 (Rust-seeded) and the forward-compat fallback. The repair UI renders
        // `reason` as an opaque string, so both flow through without a frontend change.
        RepairReason::OrdinalPermutation => "ordinal-permutation",
        RepairReason::Unknown => "unknown",
    }
}

fn candidate_dto(c: onecad_core::document::repair::RepairCandidate) -> ResolveCandidateDto {
    ResolveCandidateDto {
        topo_key: c.topo_key.as_str().to_string(),
        score: c.score,
        margin: c.margin,
        world_pos: [c.world_pos.x, c.world_pos.y, c.world_pos.z],
        summary: c.summary,
        feature_contributions: c.extra.get("featureContributions").cloned(),
    }
}

/// One timeline record left in `StepState::Error` by a published regen — the
/// `failedSteps` correlation entry (MODEL-HARDEN finding 1). Its `recordId` lets the
/// frontend awaiter detect that ITS committed op failed even though the surrounding
/// from-0 regen published OTHER bodies (which would otherwise read as a blanket
/// commit success). `message` is the worker's failure reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedStep {
    pub record_id: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<onecad_core::regen::Diagnostic>,
}

/// The `regen-finished` event payload so the frontend correlation resolves
/// promptly without the 8 s fallback (F-WP8 flag 3). `sourceRevision` is the
/// fencing revision the regen was PREPARED for (Codex MAJOR-3 commit provenance):
/// the awaiter for a commit at revision R resolves a `failed`/`noop` completion
/// only when `sourceRevision >= R`, and IGNORES a `superseded` one, so a stale
/// report from an earlier commit can never resolve a later one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegenFinished {
    pub revision: u64,
    /// The revision this regen was fenced against at `begin_regen`.
    pub source_revision: u64,
    /// `published` | `superseded` | `failed` | `cancelled` | `noop`.
    pub outcome: String,
    /// The failure reason for a `failed` regen (SCHEMA §8), so a correlated apply
    /// surfaces WHY. Absent for any other outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Structured kernel evidence for a hard failure or published failed step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<onecad_core::regen::Diagnostic>,
    /// Per-record failure detail for a PUBLISHED regen that nonetheless left one or
    /// more timeline steps in `Error` (MODEL-HARDEN finding 1). Empty (omitted) on a
    /// clean publish and on every non-published outcome. The frontend awaiter checks
    /// its commit's `recordId` against this so a failed op is never mistaken for a
    /// commit success just because sibling bodies republished.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_steps: Vec<FailedStep>,
    /// Per-record-id, the body ids an op CREATED or MODIFIED in THIS published regen
    /// (created incl. split children mapped to their derived uuids, plus modified
    /// Add/Cut targets), rendered in the same wire form as `document-changed`'s
    /// `changedBodies`. Lets a correlated apply resolve its own body precisely rather
    /// than off the whole republished set. Empty (omitted) on failed/superseded/noop.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub affected_bodies: std::collections::BTreeMap<String, Vec<String>>,
}

/// One entry in the `needs-repair` event — a **lean** summary of a step left in
/// NeedsRepair (SCHEMA §9). The repair panel fetches the full candidate evidence via
/// `resolveRefs` on demand, so this carries only what the banner/badge needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsRepairItemDto {
    /// The op record id (`RecordId`) of the step needing repair.
    pub op_id: String,
    /// The op-input ref identity (SCHEMA §9 `refId`, e.g. `"op_5.input0"`).
    pub ref_id: String,
    /// `ambiguous` | `no-candidates` | `low-confidence` (SCHEMA §9 `reason`).
    pub reason: String,
    /// The `resolverVersion` the candidate scores were computed under (SCHEMA §9).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoring_version: Option<u32>,
    /// How many candidates the ladder surfaced (0 ⇒ `no-candidates`).
    pub candidate_count: usize,
    /// The body the failing step OPERATES ON, in the bare-uuid form the frontend
    /// holds (H9). A rebind must promote its chosen `TopoKey` against SOME body,
    /// and without this the frontend could only guess — it refused outright in a
    /// multi-body document, which made repair unusable exactly where topological
    /// naming is hardest. Derived from the record's own inputs (see
    /// `DocumentRuntime::needs_repair_items`), so it is never a guess.
    ///
    /// `None` only when the step has no derivable body at all (an op whose inputs
    /// are sketches only, or an unresolvable step index); the frontend then falls
    /// back to its single-body/selection derivation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_id: Option<String>,
}

/// The `needs-repair` event payload (`{revision, items}`). Emitted after **every**
/// published regen; an EMPTY `items` means repairs cleared, so the frontend can drop
/// the banner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsRepairEvent {
    pub revision: u64,
    /// Snapshot that produced `items`; candidate TopoKeys are valid only here.
    pub snapshot_id: u64,
    pub items: Vec<NeedsRepairItemDto>,
}

/// A feature's upstream/downstream transitive closures from the session's derived
/// dependency graph (H10 dependency view; `get_feature_dependencies`) — read-only,
/// same class of lineage query as [`can_fold_transform`]. Consumed by delete/
/// suppress confirmations (dependent counts) and the inspector's "Depends on /
/// Used by" section. Never contains the queried record itself; order is the
/// graph's own (creation-index-stable, not wire-significant).
///
/// [`can_fold_transform`]: onecad_core::document::transform::can_fold_transform
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureDependenciesDto {
    /// Record ids this feature depends on (transitive).
    pub upstream: Vec<String>,
    /// Record ids that depend on this feature (transitive).
    pub downstream: Vec<String>,
}

/// Builds the lean [`NeedsRepairItemDto`] for one repair item + its op record id.
///
/// `body_id` is the operated body the caller derived from the record (bare uuid);
/// see the field docs for why the frontend cannot derive it itself.
#[must_use]
pub fn needs_repair_item_dto(
    op_id: String,
    body_id: Option<String>,
    item: &onecad_core::document::repair::RepairItem,
) -> NeedsRepairItemDto {
    NeedsRepairItemDto {
        op_id,
        ref_id: item.ref_id.clone(),
        reason: repair_reason_str(item.reason).to_string(),
        scoring_version: item.scoring_version,
        candidate_count: item.candidates.len(),
        body_id,
    }
}

// ── Mappers (op → feature kind / value; regen state → feature status) ─────────

/// Maps a timeline [`Operation`] to its frontend feature kind.
///
/// Ops outside the vertical slice (Shell/patterns/Loft/Sweep/Mirror) and opaque
/// frozen nodes fall back to the nearest slice kind — they never appear in a V1
/// document but keep the projection total.
#[must_use]
pub fn feature_kind(op: &Operation) -> FeatureKind {
    match op {
        Operation::Known(k) => match k {
            KnownOperation::Sketch(_) => FeatureKind::Sketch,
            KnownOperation::Extrude(_) | KnownOperation::Loft(_) | KnownOperation::Sweep(_) => {
                FeatureKind::Extrude
            }
            KnownOperation::Revolve(_) => FeatureKind::Revolve,
            // OffsetFace joins the Fillet/Chamfer/Shell dress-up bucket: it is a
            // face-level modifier of an existing body, not a body producer.
            // `FeatureKind` is a coarse icon grouping (see `default_label` for why
            // the label is keyed off the OPERATION instead).
            KnownOperation::Fillet(_)
            | KnownOperation::Chamfer(_)
            | KnownOperation::Shell(_)
            | KnownOperation::OffsetFace(_) => FeatureKind::Fillet,
            KnownOperation::Boolean(_)
            | KnownOperation::LinearPattern(_)
            | KnownOperation::CircularPattern(_)
            | KnownOperation::MirrorBody(_)
            | KnownOperation::ImportStep(_)
            // Interim bucket: `FeatureKind` is a coarse icon grouping and has no
            // placement/move member yet (adding one is a frontend-owned change).
            | KnownOperation::TransformBody(_)
            // A Hole is a body MODIFIER (it cuts its host), so it groups with the
            // boolean family rather than the Fillet/Chamfer/Shell dress-up bucket.
            | KnownOperation::Hole(_) => FeatureKind::Boolean,
        },
        Operation::Opaque(_) => FeatureKind::Extrude,
    }
}

/// A feature chip's value: the mono `valueText` **and** the primary dimension
/// behind it ([`FeatureDto::primary_value`]).
///
/// ONE type for both because they are minted by ONE match ([`feature_value`]): the
/// history row renders the number through a unit-aware formatter while three
/// frontend parsers still read the millimetre-fixed text, and a drift between them
/// would show one value and edit another.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureValue {
    /// Mono value text (e.g. `"25.0 mm"` / `"360.0°"`). Empty ⇒ dimensionless.
    pub text: String,
    /// The primary editable dimension in the document domain (mm / degrees).
    pub primary: Option<f64>,
    /// `"length"` | `"angle"` | `"diameter"`; `Some` exactly when `primary` is.
    pub primary_kind: Option<&'static str>,
}

impl FeatureValue {
    /// A row that reads as text only — nothing on it is inline-editable.
    fn text_only(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            primary: None,
            primary_kind: None,
        }
    }

    /// A row whose text is backed by one editable dimension of `kind`.
    fn dimensioned(text: String, primary: f64, kind: &'static str) -> Self {
        Self {
            text,
            primary: Some(primary),
            primary_kind: Some(kind),
        }
    }
}

/// The mono value text for a feature chip (e.g. `"25.0 mm"` / `"360.0°"`). Empty
/// for dimensionless features (sketch/boolean).
///
/// Thin projection of [`feature_value`] — kept as its own entry point because the
/// text is a stable, millimetre-fixed contract with the frontend re-edit parsers.
#[must_use]
pub fn feature_value_text(op: &Operation) -> String {
    feature_value(op).text
}

/// The value text + primary editable dimension for a feature chip.
///
/// Every arm decides BOTH at once, so a row can never display one number and edit
/// another. `primary` is `None` for a feature with no single editable dimension
/// (Boolean's operation, a placement's Δ, a pattern count, an import) — those rows
/// stay read-only in the history panel.
#[must_use]
pub fn feature_value(op: &Operation) -> FeatureValue {
    let Operation::Known(k) = op else {
        return FeatureValue::text_only("");
    };
    match k {
        KnownOperation::Extrude(p) => FeatureValue::dimensioned(
            format!("{:.1} mm", p.distance.value.abs()),
            p.distance.value.abs(),
            "length",
        ),
        KnownOperation::Revolve(p) => FeatureValue::dimensioned(
            format!("{:.1}°", p.angle_deg.value),
            p.angle_deg.value,
            "angle",
        ),
        KnownOperation::Fillet(p) => FeatureValue::dimensioned(
            format!("{:.1} mm", p.radius.value),
            p.radius.value,
            "length",
        ),
        // A two-distance chamfer (SCHEMA §7.3, 2026-08-03) reads as `d1×d2`: the
        // row must not claim to be an equal-leg chamfer of `radius` when the
        // second leg is what makes the feature what it is. Absent ⇒ the plain
        // single-number form every existing chamfer row already shows.
        // `src/tools/preview/filletRadius.ts radiusFromValueText` parses the
        // LEADING number back for a re-edit seed, so d1 stays first (pinned both
        // sides); the second leg is seeded from the stored params, not from here.
        // The inline editor targets d1 (`radius`) only — a second leg is a
        // two-field edit that belongs in the Chamfer tool, not a one-number row.
        KnownOperation::Chamfer(p) => FeatureValue::dimensioned(
            match &p.distance2 {
                Some(d2) => format!("{:.1}×{:.1} mm", p.radius.value, d2.value),
                None => format!("{:.1} mm", p.radius.value),
            },
            p.radius.value,
            "length",
        ),
        KnownOperation::Shell(p) => FeatureValue::dimensioned(
            format!("{:.1} mm", p.thickness.value),
            p.thickness.value,
            "length",
        ),
        // A hole's one identifying number is its DRILL diameter — the counterbore
        // /countersink dimensions dress it but never change what fastener it takes.
        // Diameter-prefixed (`Ø`) rather than " mm"-suffixed so the row cannot be
        // misread as a length; the unit is mm document-wide.
        KnownOperation::Hole(p) => FeatureValue::dimensioned(
            format!("Ø{:.1}", p.diameter.value),
            p.diameter.value,
            "diameter",
        ),
        // An offset's one identifying number is its `distance`, but WHAT that
        // number means is `distanceType`'s to say — so the row is prefixed the way
        // the user typed it (`Ø` / `R` for the absolute cylindrical forms, the Hole
        // precedent) rather than always reading as a length. `primary` stays the
        // raw stored value in every case; only the text differs.
        KnownOperation::OffsetFace(p) => {
            let d = p.distance.value;
            let (text, kind) = match p.distance_type {
                OffsetDistanceType::Diameter => (format!("Ø{d:.1}"), "diameter"),
                OffsetDistanceType::Radius => (format!("R{d:.1}"), "length"),
                OffsetDistanceType::Offset | OffsetDistanceType::Total => {
                    (format!("{d:.1} mm"), "length")
                }
            };
            FeatureValue::dimensioned(text, d, kind)
        }
        // The operation IS a boolean row's one editable parameter — without it a
        // re-edit op swap has no visible projection signal at all. NOT a number,
        // so the row carries no `primary` and its value stays read-only.
        KnownOperation::Boolean(p) => FeatureValue::text_only(format!("{:?}", p.operation)),
        // A placement's one-line summary: the translation when there is one, else
        // the rotation angle, else "0.0 mm" for an identity (a legal no-op that
        // must still read as an editable value, not as a blank row).
        //
        // No `primary`: a placement is 3 translations + an axis-angle, and picking
        // any ONE of them as "the" dimension would let a row edit dx while showing
        // Δ(dx, dy, dz). The Move tool owns that edit.
        KnownOperation::TransformBody(p) => {
            let [dx, dy, dz] = [
                p.translate[0].value,
                p.translate[1].value,
                p.translate[2].value,
            ];
            FeatureValue::text_only(if dx != 0.0 || dy != 0.0 || dz != 0.0 {
                format!("Δ({dx:.1}, {dy:.1}, {dz:.1})")
            } else if p.angle_deg() != 0.0 {
                format!("{:.1}°", p.angle_deg())
            } else {
                "0.0 mm".to_string()
            })
        }
        _ => FeatureValue::text_only(""),
    }
}

/// The default label for a feature kind (used when a record carries no name).
#[must_use]
pub fn op_type_name(op: &Operation) -> String {
    op.op_type().to_string()
}

/// The default history-tree label for an operation.
#[must_use]
pub fn default_label(op: &Operation) -> &'static str {
    // Keyed off the OPERATION, not [`feature_kind`]. `FeatureKind` is a coarse
    // icon/grouping bucket that folds Chamfer+Shell into `Fillet` and the
    // pattern/mirror ops into `Boolean`; labelling from it made a Chamfer read
    // "Fillet" and a Linear Pattern read "Boolean" in the history tree.
    match op {
        Operation::Known(k) => match k {
            KnownOperation::Sketch(_) => "Sketch",
            KnownOperation::Extrude(_) => "Extrude",
            KnownOperation::Revolve(_) => "Revolve",
            KnownOperation::Fillet(_) => "Fillet",
            KnownOperation::Chamfer(_) => "Chamfer",
            KnownOperation::Shell(_) => "Shell",
            KnownOperation::Boolean(_) => "Boolean",
            KnownOperation::LinearPattern(_) => "Linear Pattern",
            KnownOperation::CircularPattern(_) => "Circular Pattern",
            KnownOperation::MirrorBody(_) => "Mirror",
            KnownOperation::Loft(_) => "Loft",
            KnownOperation::Sweep(_) => "Sweep",
            KnownOperation::ImportStep(_) => "Import",
            KnownOperation::TransformBody(_) => "Move",
            KnownOperation::Hole(_) => "Hole",
            KnownOperation::OffsetFace(_) => "Offset face",
        },
        // A frozen unknown node keeps its opType as the label rather than
        // masquerading as an Extrude.
        Operation::Opaque(_) => "Feature",
    }
}

/// Maps a regen [`StepState`] to a frontend feature status.
///
/// Suppression is an **orthogonal axis** and rides [`FeatureDto::suppressed`]
/// (sourced from the record flag), not this status: `StepState::Suppressed` keeps
/// folding into `Dirty` here — a skipped step has no regen result, which is exactly
/// what "awaiting regen" renders as — while the dedicated flag tells the frontend
/// *why*.
#[must_use]
pub fn feature_status(state: &StepState) -> FeatureStatus {
    match state {
        StepState::Valid => FeatureStatus::Ok,
        StepState::Error { .. } => FeatureStatus::Error,
        StepState::NeedsRepair => FeatureStatus::NeedsRepair,
        // Dirty (awaiting regen) and Suppressed (skipped) both read as inactive.
        StepState::Dirty | StepState::Suppressed => FeatureStatus::Dirty,
    }
}

/// The worker failure reason for an errored step (`StepState::Error{reason}`),
/// or `None` for any other state — the [`FeatureDto::status_message`] source
/// (Codex MAJOR-4 error surfacing).
#[must_use]
pub fn feature_status_message(state: &StepState) -> Option<String> {
    match state {
        StepState::Error { reason } => Some(reason.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::document::record::{
        BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation,
    };
    use onecad_core::document::variables::Scalar;

    fn extrude(dist: f64) -> Operation {
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: None,
            distance: Scalar::new(dist),
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
        }))
    }

    #[test]
    fn projection_serializes_camelcase_mirroring_the_store() {
        let mut bodies = std::collections::BTreeMap::new();
        bodies.insert(
            "b1".to_string(),
            BodyDto {
                id: "b1".into(),
                name: "Body 1".into(),
                visible: true,
                color: None,
                face_colors: std::collections::BTreeMap::new(),
            },
        );
        let mut datums = std::collections::BTreeMap::new();
        datums.insert(
            "d1".to_string(),
            DatumDto {
                id: "d1".into(),
                name: "Datum 1".into(),
                kind: "OffsetFromPlane".into(),
                base_plane_id: "XY".into(),
                offset: 10.0,
                plane: onecad_core::sketch::SketchPlane::xy().into(),
                resolved_valid: true,
            },
        );
        let proj = DocumentProjection {
            status: DocStatus::Ready,
            document_id: "doc-1".into(),
            revision: 5,
            title: "Bracket".into(),
            dirty: false,
            bodies,
            sketches: std::collections::BTreeMap::new(),
            datums,
            features: vec![FeatureDto {
                op_type: "Extrude".into(),
                id: "f1".into(),
                kind: FeatureKind::Extrude,
                label: "Extrude".into(),
                value_text: "25.0 mm".into(),
                primary_value: Some(25.0),
                primary_value_kind: Some("length".into()),
                status: FeatureStatus::Ok,
                status_message: None,
                suppressed: false,
            }],
            applied_ops: 1,
            total_ops: 1,
            geometry_source: GEOMETRY_SOURCE_LIVE.to_string(),
        };
        let v = serde_json::to_value(&proj).unwrap();
        assert_eq!(v["status"], "ready");
        assert_eq!(v["revision"], 5);
        assert_eq!(v["bodies"]["b1"]["visible"], true);
        assert_eq!(v["features"][0]["kind"], "extrude");
        assert_eq!(v["features"][0]["valueText"], "25.0 mm");
        assert_eq!(v["features"][0]["status"], "ok");
        assert_eq!(v["appliedOps"], 1);
        assert_eq!(v["totalOps"], 1);
        assert_eq!(v["geometrySource"], "live");
        // Datums project camelCase, keyed by id, carrying the resolved basis
        // VERBATIM (the non-standard XY basis — see sketch/plane.rs).
        assert_eq!(v["datums"]["d1"]["kind"], "OffsetFromPlane");
        assert_eq!(v["datums"]["d1"]["basePlaneId"], "XY");
        assert_eq!(v["datums"]["d1"]["offset"], 10.0);
        assert_eq!(v["datums"]["d1"]["resolvedValid"], true);
        assert_eq!(
            v["datums"]["d1"]["plane"]["xAxis"],
            serde_json::json!([0.0, 1.0, 0.0])
        );
        assert_eq!(
            v["datums"]["d1"]["plane"]["yAxis"],
            serde_json::json!([-1.0, 0.0, 0.0])
        );
    }

    #[test]
    fn extrude_value_text_and_kind() {
        let op = extrude(25.0);
        assert_eq!(feature_kind(&op), FeatureKind::Extrude);
        assert_eq!(feature_value_text(&op), "25.0 mm");
    }

    /// H3 — every dimensioned op exposes the ONE number its history row edits, in
    /// the DOCUMENT domain (mm for a length/diameter, DEGREES for an angle), and
    /// its `value_text` is byte-identical to what it was before the field existed.
    #[test]
    fn primary_value_is_the_row_editable_dimension_per_op_type() {
        use onecad_core::document::record::{
            ChamferParams, FilletParams, RevolveParams, ShellParams,
        };
        use onecad_core::ids::ElementId;
        use onecad_core::math::Vec3;

        let check = |op: &Operation, text: &str, primary: Option<f64>, kind: Option<&str>| {
            let v = feature_value(op);
            assert_eq!(v.text, text, "value_text must not move");
            assert_eq!(v.primary, primary);
            assert_eq!(v.primary_kind, kind);
            // `feature_value_text` stays a pure projection of the same match.
            assert_eq!(feature_value_text(op), text);
        };

        // A NEGATIVE (reversed) extrude edits its MAGNITUDE — the row shows 25.0,
        // and committing 25.0 back must not silently flip the direction.
        check(&extrude(-25.0), "25.0 mm", Some(25.0), Some("length"));

        let revolve = Operation::Known(KnownOperation::Revolve(RevolveParams {
            profile: None,
            axis: None,
            angle_deg: Scalar::new(90.0),
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            extra: Default::default(),
        }));
        check(&revolve, "90.0°", Some(90.0), Some("angle"));

        let fillet = Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(2.0),
            edge_ids: vec![ElementId::new("e:14")],
            edges: vec![],
            chain_tangent_edges: true,
            tangent_closure_version: None,
            extra: Default::default(),
        }));
        check(&fillet, "2.0 mm", Some(2.0), Some("length"));

        // An asymmetric chamfer still edits d1 — the text keeps BOTH legs.
        let chamfer = Operation::Known(KnownOperation::Chamfer(ChamferParams {
            radius: Scalar::new(1.0),
            distance2: Some(Scalar::new(2.5)),
            edge_ids: vec![ElementId::new("e:14")],
            edges: vec![],
            chain_tangent_edges: true,
            tangent_closure_version: None,
            extra: Default::default(),
        }));
        check(&chamfer, "1.0×2.5 mm", Some(1.0), Some("length"));

        let shell = Operation::Known(KnownOperation::Shell(ShellParams {
            thickness: Scalar::new(1.5),
            open_faces: Vec::new(),
            faces: Vec::new(),
            target_body: None,
            extra: Default::default(),
        }));
        check(&shell, "1.5 mm", Some(1.5), Some("length"));

        // Hole/Boolean/Sketch carry required identity refs, so they are built the
        // way a document loads them — through serde, from the SCHEMA §7.3 shape.
        let from_json = |v: serde_json::Value| -> Operation { serde_json::from_value(v).unwrap() };
        let hole = from_json(serde_json::json!({
            "opType": "Hole",
            "params": {
                "targetBodyId": "00000000-0000-0000-0000-000000000001",
                "face": { "primary": { "bodyId": "00000000-0000-0000-0000-000000000001",
                                       "elementId": "el_face_top", "kind": "face" } },
                "point": [0.0, 0.0, 0.0],
                "holeType": "simple",
                "diameter": 6.0,
                "depth": null
            }
        }));
        check(&hole, "Ø6.0", Some(6.0), Some("diameter"));

        // ── No single primary dimension ⇒ the row stays read-only ──────────────
        let boolean = from_json(serde_json::json!({
            "opType": "Boolean",
            "params": {
                "operation": "Cut",
                "targetBodyId": "00000000-0000-0000-0000-000000000001",
                "toolBodyId": "00000000-0000-0000-0000-000000000002"
            }
        }));
        assert_eq!(feature_value(&boolean).text, "Cut");
        assert_eq!(feature_value(&boolean).primary, None);
        assert_eq!(feature_value(&boolean).primary_kind, None);

        let sketch = from_json(serde_json::json!({
            "opType": "Sketch",
            "params": {
                "sketchId": "00000000-0000-0000-0000-000000000003",
                "plane": { "kind": "XY", "origin": [0.0, 0.0, 0.0], "xAxis": [1.0, 0.0, 0.0],
                           "yAxis": [0.0, 1.0, 0.0], "normal": [0.0, 0.0, 1.0] }
            }
        }));
        check(&sketch, "", None, None);

        let pattern = Operation::Known(KnownOperation::LinearPattern(
            onecad_core::document::record::LinearPatternParams {
                source_body: None,
                direction: Vec3::new(1.0, 0.0, 0.0).unwrap(),
                spacing: Scalar::new(20.0),
                count: 4,
                fuse_result: true,
                extra: Default::default(),
            },
        ));
        check(&pattern, "", None, None);

        // An opaque frozen node has no params the projection can read at all.
        check(
            &from_json(serde_json::json!({ "opType": "Wobble" })),
            "",
            None,
            None,
        );
    }

    /// The two new fields are ADDITIVE: omitted entirely when absent, so a payload
    /// for a dimensionless feature is byte-identical to a pre-H3 one.
    #[test]
    fn primary_value_is_omitted_when_absent() {
        let dimensionless = FeatureDto {
            id: "f9".into(),
            kind: FeatureKind::Boolean,
            op_type: "Boolean".into(),
            label: "Boolean".into(),
            value_text: "Cut".into(),
            primary_value: None,
            primary_value_kind: None,
            status: FeatureStatus::Ok,
            status_message: None,
            suppressed: false,
        };
        let v = serde_json::to_value(&dimensionless).unwrap();
        assert!(v.get("primaryValue").is_none());
        assert!(v.get("primaryValueKind").is_none());

        let dimensioned = FeatureDto {
            primary_value: Some(6.0),
            primary_value_kind: Some("diameter".into()),
            ..dimensionless
        };
        let v = serde_json::to_value(&dimensioned).unwrap();
        assert_eq!(v["primaryValue"], 6.0);
        assert_eq!(v["primaryValueKind"], "diameter");
    }

    /// A two-distance chamfer's row shows BOTH legs (SCHEMA §7.3, 2026-08-03);
    /// an equal-leg one is byte-identical to what it always showed.
    #[test]
    fn chamfer_value_text_shows_the_second_distance_only_when_set() {
        use onecad_core::document::record::ChamferParams;
        use onecad_core::ids::ElementId;
        let chamfer = |d2: Option<f64>| {
            Operation::Known(KnownOperation::Chamfer(ChamferParams {
                radius: Scalar::new(1.0),
                distance2: d2.map(Scalar::new),
                edge_ids: vec![ElementId::new("e:14")],
                edges: vec![],
                chain_tangent_edges: true,
                tangent_closure_version: None,
                extra: Default::default(),
            }))
        };
        assert_eq!(feature_value_text(&chamfer(None)), "1.0 mm");
        assert_eq!(feature_value_text(&chamfer(Some(2.5))), "1.0×2.5 mm");
        // The re-edit seed parses the LEADING number (`radiusFromValueText`).
        assert!(feature_value_text(&chamfer(Some(2.5))).starts_with("1.0"));
    }

    /// PIN (frontend contract): the projection folds the pattern/mirror ops into
    /// `kind: "boolean"` and Shell into `kind: "fillet"`, so `kind` can NEVER be
    /// the routing key for a parametric re-edit — only `opType` can. The frontend
    /// re-edit guards (`ModelToolController.editLinearPatternFeature` et al.) gate
    /// on exactly these `opType` strings; changing either side breaks re-edit
    /// silently (the guard just returns), which is why both are pinned together.
    #[test]
    fn folded_kinds_keep_their_authored_op_type() {
        use onecad_core::document::record::{LinearPatternParams, ShellParams};
        use onecad_core::math::Vec3;

        let lp = Operation::Known(KnownOperation::LinearPattern(LinearPatternParams {
            source_body: None,
            direction: Vec3::new(1.0, 0.0, 0.0).unwrap(),
            spacing: Scalar::new(20.0),
            count: 4,
            fuse_result: true,
            extra: Default::default(),
        }));
        // A LinearPattern record's FeatureDto: kind Boolean, opType "LinearPattern".
        let dto = FeatureDto {
            id: "f7".into(),
            kind: feature_kind(&lp),
            op_type: op_type_name(&lp),
            label: "Linear Pattern".into(),
            value_text: feature_value_text(&lp),
            primary_value: None,
            primary_value_kind: None,
            status: FeatureStatus::Ok,
            status_message: None,
            suppressed: false,
        };
        assert_eq!(dto.kind, FeatureKind::Boolean);
        assert_eq!(dto.op_type, "LinearPattern");
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["kind"], "boolean");
        assert_eq!(v["opType"], "LinearPattern");

        let shell = Operation::Known(KnownOperation::Shell(ShellParams {
            thickness: Scalar::new(2.0),
            open_faces: Vec::new(),
            faces: Vec::new(),
            target_body: None,
            extra: Default::default(),
        }));
        assert_eq!(feature_kind(&shell), FeatureKind::Fillet);
        assert_eq!(op_type_name(&shell), "Shell");
    }

    #[test]
    fn sketch_upsert_dto_serializes_conflicting_ids() {
        let dto = SketchUpsertDto {
            sketch_id: "sk_1".into(),
            sketch_revision: 3,
            dof: 0,
            status: SketchSolveStatus::Conflicting,
            conflicting: vec!["c-a".into(), "c-b".into()],
            solved_positions: std::collections::BTreeMap::new(),
            solved_curves: std::collections::BTreeMap::new(),
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["conflicting"], serde_json::json!(["c-a", "c-b"]));

        // Empty is still emitted as `[]` (frontend types it optional; absent ⇒ []).
        let clean = SketchUpsertDto {
            conflicting: vec![],
            ..dto
        };
        let v = serde_json::to_value(&clean).unwrap();
        assert_eq!(v["conflicting"], serde_json::json!([]));

        // SketchSessionDto surfaces it too (session-enter surface).
        let session = SketchSessionDto {
            sketch_id: "sk_1".into(),
            plane: serde_json::json!({}),
            entities: serde_json::json!([]),
            constraints: serde_json::json!([]),
            dof: 0,
            status: SketchSolveStatus::Conflicting,
            conflicting: vec!["c-a".into()],
        };
        assert_eq!(
            serde_json::to_value(&session).unwrap()["conflicting"],
            serde_json::json!(["c-a"])
        );
    }

    #[test]
    fn curve_params_dto_skips_absent_members_but_the_map_is_always_emitted() {
        // Per-MEMBER skip: an absent member means UNCHANGED, so it must not
        // serialize as `null` (the frontend would write it as a value).
        let only_radius = CurveParamsDto {
            radius: Some(12.5),
            ..Default::default()
        };
        let v = serde_json::to_value(only_radius).unwrap();
        assert_eq!(v, serde_json::json!({ "radius": 12.5 }));
        let arc = CurveParamsDto {
            radius: Some(9.0),
            start_angle: Some(0.25),
            end_angle: Some(1.75),
        };
        assert_eq!(
            serde_json::to_value(arc).unwrap(),
            serde_json::json!({ "radius": 9.0, "startAngle": 0.25, "endAngle": 1.75 }),
            "camelCase members, radians verbatim"
        );
        assert_eq!(
            serde_json::to_value(CurveParamsDto::default()).unwrap(),
            serde_json::json!({}),
        );

        // Container-level: NEVER skipped (the `conflicting: []` precedent) — the
        // frontend always sees the key, empty when nothing moved.
        let dto = SketchUpsertDto {
            sketch_id: "sk_1".into(),
            sketch_revision: 3,
            dof: 0,
            status: SketchSolveStatus::FullyConstrained,
            conflicting: vec![],
            solved_positions: std::collections::BTreeMap::new(),
            solved_curves: std::collections::BTreeMap::new(),
        };
        assert_eq!(
            serde_json::to_value(&dto).unwrap()["solvedCurves"],
            serde_json::json!({})
        );
        let drag = DragSolveDto {
            gesture_id: 51,
            seq: 2,
            status: "success".into(),
            dof: 1,
            conflicting: vec![],
            positions: std::collections::BTreeMap::new(),
            curves: std::collections::BTreeMap::from([("e7".into(), only_radius)]),
            solve_micros: 0,
            superseded: false,
        };
        assert_eq!(
            serde_json::to_value(&drag).unwrap()["curves"],
            serde_json::json!({ "e7": { "radius": 12.5 } })
        );
    }

    #[test]
    fn step_state_maps_to_feature_status() {
        assert_eq!(feature_status(&StepState::Valid), FeatureStatus::Ok);
        assert_eq!(feature_status(&StepState::Dirty), FeatureStatus::Dirty);
        assert_eq!(
            feature_status(&StepState::NeedsRepair),
            FeatureStatus::NeedsRepair
        );
        assert_eq!(
            feature_status(&StepState::Error {
                reason: "boom".into()
            }),
            FeatureStatus::Error
        );
    }

    #[test]
    fn step_error_maps_to_status_message() {
        // Only an errored step carries a reason; every other state ⇒ None (skipped).
        assert_eq!(
            feature_status_message(&StepState::Error {
                reason: "revolve axis lineId not found".into()
            }),
            Some("revolve axis lineId not found".to_string())
        );
        assert_eq!(feature_status_message(&StepState::Valid), None);
        assert_eq!(feature_status_message(&StepState::Dirty), None);
        assert_eq!(feature_status_message(&StepState::NeedsRepair), None);

        // The DTO omits `statusMessage` entirely when absent, and carries it verbatim
        // (camelCase) when present.
        let ok = FeatureDto {
            op_type: "Extrude".into(),
            id: "f1".into(),
            kind: FeatureKind::Extrude,
            label: "Extrude".into(),
            value_text: "25.0 mm".into(),
            primary_value: Some(25.0),
            primary_value_kind: Some("length".into()),
            status: FeatureStatus::Ok,
            status_message: None,
            suppressed: false,
        };
        let v = serde_json::to_value(&ok).unwrap();
        assert!(v.get("statusMessage").is_none(), "absent ⇒ omitted");
        assert_eq!(
            v["suppressed"], false,
            "`suppressed` is a plain bool, ALWAYS on the wire"
        );

        let errored = FeatureDto {
            status: FeatureStatus::Error,
            status_message: Some("boom".into()),
            ..ok
        };
        let v = serde_json::to_value(&errored).unwrap();
        assert_eq!(v["status"], "error");
        assert_eq!(v["statusMessage"], "boom");
    }

    #[test]
    fn regen_finished_failed_steps_and_affected_bodies_serialize_additively() {
        // A clean publish carries neither field on the wire (old payloads parse).
        let clean = RegenFinished {
            revision: 7,
            source_revision: 7,
            outcome: "published".into(),
            message: None,
            diagnostics: Vec::new(),
            failed_steps: Vec::new(),
            affected_bodies: std::collections::BTreeMap::new(),
        };
        let v = serde_json::to_value(&clean).unwrap();
        assert_eq!(v["outcome"], "published");
        assert!(v.get("failedSteps").is_none(), "empty ⇒ omitted");
        assert!(v.get("affectedBodies").is_none(), "empty ⇒ omitted");
        assert!(v.get("message").is_none());

        // A published-with-failure carries both, camelCase, keyed by recordId.
        let mut affected = std::collections::BTreeMap::new();
        affected.insert("rec-extrude".to_string(), vec!["body-1".to_string()]);
        let with = RegenFinished {
            outcome: "published".into(),
            diagnostics: vec![onecad_core::regen::Diagnostic {
                severity: onecad_core::regen::Severity::Error,
                code: "FILLET_WALKING_FAILED".into(),
                message: "fillet failed".into(),
                stage: Some("build".into()),
                evidence: Some(serde_json::json!({"contour": {"index": 1}})),
            }],
            failed_steps: vec![FailedStep {
                record_id: "rec-revolve".into(),
                message: "axis not found".into(),
                diagnostics: vec![onecad_core::regen::Diagnostic {
                    severity: onecad_core::regen::Severity::Error,
                    code: "AXIS_FAILED".into(),
                    message: "axis not found".into(),
                    stage: None,
                    evidence: None,
                }],
            }],
            affected_bodies: affected,
            ..clean
        };
        let v = serde_json::to_value(&with).unwrap();
        assert_eq!(v["failedSteps"][0]["recordId"], "rec-revolve");
        assert_eq!(v["failedSteps"][0]["message"], "axis not found");
        assert_eq!(v["failedSteps"][0]["diagnostics"][0]["code"], "AXIS_FAILED");
        assert_eq!(v["diagnostics"][0]["code"], "FILLET_WALKING_FAILED");
        assert_eq!(v["diagnostics"][0]["stage"], "build");
        assert_eq!(
            v["affectedBodies"]["rec-extrude"],
            serde_json::json!(["body-1"])
        );
    }

    // ── M4a deliverable 3: the un-lossy resolve_refs DTO mapping ──────────────

    #[test]
    fn resolve_ref_dto_autobind_carries_id_and_topokey_evidence() {
        use onecad_core::ids::{ElementId, TopoKey};
        use onecad_core::regen::{RefResolution, ResolveOutcome};
        let dto = ResolveRefDto::from_resolution(
            RefResolution {
                ref_id: "op_5.input0".into(),
                outcome: ResolveOutcome::AutoBind {
                    element_id: ElementId::new("el_top"),
                    score: 0.94,
                    margin: 0.31,
                    topo_key: Some(TopoKey::new("f:1")),
                },
            },
            5012,
            44,
            Some("body_3".into()),
        );
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["snapshotId"], 5012);
        assert_eq!(v["revision"], 44);
        assert_eq!(v["bodyId"], "body_3");
        assert_eq!(v["outcome"], "autoBind");
        assert_eq!(v["elementId"], "el_top");
        assert_eq!(v["topoKey"], "f:1");
        assert_eq!(v["score"], 0.94);
        assert_eq!(v["margin"], 0.31);
        assert!(v.get("candidates").is_none(), "no candidates on autoBind");

        // Unminted dry-run autoBind: empty id ⇒ elementId omitted, topoKey kept.
        let dto = ResolveRefDto::from_resolution(
            RefResolution {
                ref_id: "op_5.input1".into(),
                outcome: ResolveOutcome::AutoBind {
                    element_id: ElementId::new(""),
                    score: 0.9,
                    margin: 0.2,
                    topo_key: Some(TopoKey::new("f:3")),
                },
            },
            5012,
            44,
            Some("body_3".into()),
        );
        let v = serde_json::to_value(&dto).unwrap();
        assert!(v.get("elementId").is_none(), "empty id ⇒ omitted");
        assert_eq!(v["topoKey"], "f:3");
    }

    #[test]
    fn resolve_ref_dto_needs_repair_carries_full_candidate_evidence() {
        use onecad_core::document::repair::{
            LadderLevel, RepairCandidate, RepairItem, RepairReason,
        };
        use onecad_core::ids::TopoKey;
        use onecad_core::math::Vec3;
        use onecad_core::regen::{RefResolution, ResolveOutcome};

        let mut extra = onecad_core::document::refs::Extra::new();
        extra.insert(
            "featureContributions".into(),
            serde_json::json!({ "area": 0.25 }),
        );
        let item = RepairItem {
            step_index: 6,
            ref_id: "op_6.input0".into(),
            element_id: Some(onecad_core::ids::ElementId::new("el_last")),
            ladder_failed: LadderLevel::Descriptor,
            reason: RepairReason::Ambiguous,
            candidates: vec![
                RepairCandidate {
                    topo_key: TopoKey::new("f:31"),
                    score: 0.91,
                    margin: 0.0,
                    world_pos: Vec3::new_unchecked(12.0, 3.5, 0.0),
                    summary: "planar face".into(),
                    extra,
                },
                RepairCandidate {
                    topo_key: TopoKey::new("f:44"),
                    score: 0.91,
                    margin: 0.0,
                    world_pos: Vec3::new_unchecked(12.0, -3.5, 0.0),
                    summary: "planar face".into(),
                    extra: Default::default(),
                },
            ],
            scoring_version: Some(1),
            anchor: None,
            ui_label: "Fillet edge".into(),
            seeded: false,
            ordinal_anchor: None,
        };
        let dto = ResolveRefDto::from_resolution(
            RefResolution {
                ref_id: "op_6.input0".into(),
                outcome: ResolveOutcome::NeedsRepair(item),
            },
            5012,
            44,
            Some("body_3".into()),
        );
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["outcome"], "needsRepair");
        assert_eq!(v["elementId"], "el_last");
        assert_eq!(v["ladderFailed"], "descriptor");
        assert_eq!(v["reason"], "ambiguous");
        assert_eq!(v["scoringVersion"], 1);
        assert_eq!(v["uiLabel"], "Fillet edge");
        // The candidate evidence the OLD DTO dropped is now carried in full.
        assert_eq!(v["candidates"].as_array().unwrap().len(), 2);
        assert_eq!(v["candidates"][0]["topoKey"], "f:31");
        assert_eq!(v["candidates"][0]["score"], 0.91);
        assert_eq!(
            v["candidates"][0]["worldPos"],
            serde_json::json!([12.0, 3.5, 0.0])
        );
        assert_eq!(
            v["candidates"][0]["featureContributions"],
            serde_json::json!({ "area": 0.25 })
        );
    }

    #[test]
    fn needs_repair_item_dto_shape() {
        use onecad_core::document::repair::{LadderLevel, RepairItem, RepairReason};
        let item = RepairItem {
            step_index: 2,
            ref_id: "op_2.input0".into(),
            element_id: None,
            ladder_failed: LadderLevel::Descriptor,
            reason: RepairReason::NoCandidates,
            candidates: Vec::new(),
            scoring_version: Some(1),
            anchor: None,
            ui_label: String::new(),
            seeded: false,
            ordinal_anchor: None,
        };
        let dto = needs_repair_item_dto("rec-1".into(), Some("body-7".into()), &item);
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["opId"], "rec-1");
        assert_eq!(v["refId"], "op_2.input0");
        assert_eq!(v["reason"], "no-candidates");
        assert_eq!(v["scoringVersion"], 1);
        assert_eq!(v["candidateCount"], 0);
        assert_eq!(v["bodyId"], "body-7");

        // H9: `bodyId` is ADDITIVE — a step with no derivable body omits the key
        // entirely rather than sending null, so an older frontend reads the event
        // exactly as it did before.
        let lean = needs_repair_item_dto("rec-1".into(), None, &item);
        let v = serde_json::to_value(&lean).unwrap();
        assert!(
            v.get("bodyId").is_none(),
            "bodyId must be skipped when absent: {v}"
        );
    }

    /// H10: the wire shape a delete/suppress confirm and the inspector's
    /// "Depends on / Used by" section both read.
    #[test]
    fn feature_dependencies_dto_shape() {
        let dto = FeatureDependenciesDto {
            upstream: vec!["op_1".into()],
            downstream: vec!["op_3".into(), "op_4".into()],
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["upstream"], serde_json::json!(["op_1"]));
        assert_eq!(v["downstream"], serde_json::json!(["op_3", "op_4"]));
    }

    /// MEASURE V1a: the three fields the frontend reads off `element_info`
    /// (`magnitude`, `size`, `curveType`) must reach it in camelCase alongside
    /// the pre-existing shape — the DTO is Serialize-only, so this IS the wire.
    #[test]
    fn element_info_dto_shape_is_camel_case() {
        let dto = ElementInfoDto {
            element_id: "el_7".into(),
            topo_key: "f:22".into(),
            body_id: "body_1".into(),
            kind: "face".into(),
            surface_type: 0,
            curve_type: -1,
            center: [1.0, 2.0, 3.0],
            normal: [0.0, 0.0, 1.0],
            has_normal: true,
            size: 44.72,
            magnitude: 800.0,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["elementId"], "el_7");
        assert_eq!(v["topoKey"], "f:22");
        assert_eq!(v["bodyId"], "body_1");
        assert_eq!(v["kind"], "face");
        assert_eq!(v["surfaceType"], 0);
        assert_eq!(v["curveType"], -1);
        assert_eq!(v["hasNormal"], true);
        assert_eq!(v["center"], serde_json::json!([1.0, 2.0, 3.0]));
        assert_eq!(v["magnitude"], 800.0);
        assert_eq!(v["size"], 44.72);
        // No snake_case leakage.
        assert!(v.get("element_id").is_none());
        assert!(v.get("curve_type").is_none());
        assert!(v.get("has_normal").is_none());
    }
}
