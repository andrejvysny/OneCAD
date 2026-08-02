/*
 * IPC data types — a MIRROR of the future Rust DTOs.
 *
 * This file is intentionally TINY. The real, authoritative shapes are minted on
 * the Rust side (serde camelCase) and land with their own work packages:
 *   - DocumentSnapshot   → R-WP10 (projection DTOs / app shell)
 *   - RecentProject      → Rust settings/recents store (later WP)
 *
 * Until then these placeholders let the start screen (F-WP2) compile and run
 * against the mock client. Keep every field here 1:1 with the eventual Rust
 * struct so the swap to the real tauri client (F-WP8) is a no-op for the UI.
 */

/** One entry in the "Recent projects" list on the start screen. */
export interface RecentProject {
  id: string;
  name: string;
  /** Absolute project path (also used as the card's title tooltip). */
  path: string;
  /** ISO-8601 timestamp of the last modification. */
  modifiedAt: string;
  /** Optional data-URI / asset URL for the preview thumbnail. */
  thumbnail?: string;
}

/**
 * Crash-recovery offer surfaced on the start screen: a prior session left an
 * autosave behind. `check_recovery` returns one (or null when nothing to offer);
 * `recover_document` accepts (restore) or rejects (discard) it. Keep 1:1 with the
 * Rust `RecoveryInfoDto` (serde camelCase).
 */
export interface RecoveryInfo {
  /** Absolute path of the document the autosave belongs to (absent if never saved). */
  originalPath?: string;
  /** Absolute path of the autosave sidecar the crashed session left behind. */
  autosavePath: string;
  /** Epoch-millis mtime of the autosave (last time work was captured). */
  modifiedMs: number;
}

/**
 * Placeholder document handle returned by open/new/import.
 *
 * The real DocumentSnapshot (full projection: bodies, timeline, revision, …)
 * lands with R-WP10. Keep this minimal — the start screen only needs to know a
 * document exists so it can transition to the editor.
 */
export interface DocumentSnapshot {
  documentId: string;
  title: string;
}

/** Level-of-detail tier for a mesh fetch (deflection relative to bbox diagonal). */
export type Lod = "coarse" | "medium" | "fine";

/** Unsubscribe handle returned by event subscriptions. */
export type Unsubscribe = () => void;

/**
 * One changed body in a `document-changed` event (plan's PULL model). The
 * backend only announces WHICH bodies changed + an opaque cache key; the
 * frontend fetches the MESH1 bytes for the visible ones via `getBodyMesh`.
 * `meshKey` mirrors the Rust MeshCache key `(BodyId, Lod, generation)`.
 */
export interface BodyMeshRef {
  bodyId: string;
  meshKey: string;
}

/**
 * `document-changed` payload. Projection stores are written only by backend
 * events; this is the delta a later IPC WP delivers for real (mock emits it).
 */
export interface DocumentChange {
  revision: number;
  /**
   * The published snapshot id this geometry belongs to (SCHEMA §7.5). Forwarded
   * to `promoteSelection` so a picked TopoKey resolves against the exact snapshot
   * the mesh was tessellated at (Invariant 4). The Rust backend always emits it;
   * optional here so the mock lane / tests can omit it (then it stays `0`).
   */
  snapshotId?: number;
  changedBodies: BodyMeshRef[];
  removedBodies: string[];
}

// ── Sketch wire shapes (SCHEMA §7.3 Sketch op params + §7.4 solver lane) ─────
//
// These mirror the JSON the C++ worker's solver lane speaks. The mock client
// (this WP) implements the same shapes so the whole sketch UI runs with no
// backend; the real tauri client swaps in later with zero UI changes.

/** Named or custom sketch plane (SCHEMA §7.3 — the non-standard XY basis). */
export type SketchPlaneKind = "XY" | "XZ" | "YZ" | "custom";

export interface SketchPlane {
  kind: SketchPlaneKind;
  origin: [number, number, number];
  xAxis: [number, number, number];
  yAxis: [number, number, number];
  normal: [number, number, number];
}

/** Entity kinds the vertical-slice tools author (subset of §7.3's six). */
export type SketchEntityType = "Point" | "Line" | "Arc" | "Circle" | "Ellipse";

/**
 * One sketch entity in **plane (u,v) coordinates**. Only the fields relevant to
 * `type` are populated (Line → p0/p1; Circle → center/radius; Arc →
 * center/radius/start/end; Ellipse → center/majorR/minorR/rotation; Point → p0).
 */
export interface SketchEntity {
  id: string;
  type: SketchEntityType;
  /** Construction geometry (dashed, not part of profiles). */
  construction?: boolean;
  /** Host-face reference geometry (SCHEMA §7.3 `referenceLocked`): projected from
   *  the face this sketch sits on. Selectable and snappable, and — unlike
   *  `construction` — it DOES bound regions; what it forbids is EDITING. Nothing
   *  in the frontend authors it yet; it only ever arrives on the hydration wire. */
  referenceLocked?: boolean;
  p0?: [number, number];
  p1?: [number, number];
  center?: [number, number];
  radius?: number;
  start?: [number, number];
  end?: [number, number];
  /** Ellipse semi-major axis (always ≥ `minorR` — normalized by the tool). */
  majorR?: number;
  /** Ellipse semi-minor axis. */
  minorR?: number;
  /** Ellipse major-axis rotation, RADIANS CCW from plane +X, in [0, 2π).
   *  (Unlike sketch dimension VALUES, this is never a UI-domain degree — it is
   *  geometry, carried in the wire domain end to end; see `angleUnits.ts`.) */
  rotation?: number;
}

/** The 18 constraint kinds (SCHEMA §7.3, verbatim from SketchTypes.h). */
export type SketchConstraintType =
  | "Coincident"
  | "Horizontal"
  | "Vertical"
  | "Fixed"
  | "Midpoint"
  | "OnCurve"
  | "Parallel"
  | "Perpendicular"
  | "Tangent"
  | "Concentric"
  | "Equal"
  | "Distance"
  | "HorizontalDistance"
  | "VerticalDistance"
  | "Angle"
  | "Radius"
  | "Diameter"
  | "Symmetric";

/** Which point of an entity a positional constraint references. */
export type ConstraintPosition = "Start" | "End" | "Center" | "Midpoint";

export interface SketchConstraint {
  id: string;
  type: SketchConstraintType;
  /** Referenced entity ids (1 for H/V/Radius, 2 for Coincident/Equal, …). */
  entities: string[];
  /** Per-entity point selector for positional constraints (Coincident, …). */
  positions?: ConstraintPosition[];
  /** Value for dimensional constraints (Distance/Radius/Angle/…). */
  value?: number;
}

/** Solver state (SCHEMA §7.4 SketchUpsert `state`). */
export type SketchSolveStatus =
  | "UnderConstrained"
  | "FullyConstrained"
  | "OverConstrained"
  | "Conflicting";

/** Full authoritative sketch, returned by `enterSketch`. */
export interface SketchSession {
  sketchId: string;
  plane: SketchPlane;
  entities: SketchEntity[];
  constraints: SketchConstraint[];
  dof: number;
  status: SketchSolveStatus;
  /** Constraint ids in conflict (SCHEMA §7.4; FRONTEND ids, unknown dropped by the
   *  client). Seeds the sketch store's `conflictingIds` on session enter. Absent ⇒ []. */
  conflicting?: string[];
}

/** `enterSketch` target: an existing sketch id, or a fresh sketch on a plane. */
export type EnterSketchTarget =
  | string
  | { newOnPlane: SketchPlaneKind; sketchId?: string }
  /**
   * A new sketch placed on a model FACE (MODEL-OPS W2). The basis comes from the
   * backend (`face_sketch_plane` → the kernel's own face descriptor + the
   * lock-tested in-plane axis rule), never from a tessellated triangle normal.
   * V1 policy: the frame is FROZEN at creation.
   *
   * `topoKey` rides along when the caller has it (SKETCH-ON-FACE W2): the real
   * lane creates the sketch through `add_sketch_on_face`, which walks the SAME
   * two-rung ladder as `face_sketch_plane` — topoKey FIRST, because a
   * just-promoted, never-consumed `elementId` is genuinely absent from the
   * worker's on-demand element-map partition. Dropping it here would make the
   * pick-a-face-then-sketch flow fail to resolve at all.
   */
  | {
      newOnFace: {
        bodyId: string;
        elementId: string;
        topoKey?: string;
        worldPoint?: [number, number, number];
      };
      plane: SketchPlane;
      sketchId?: string;
    }
  /**
   * A new sketch placed on a DATUM plane (DATUM W1). `plane` is the datum's
   * BACKEND-RESOLVED frame, read straight off `documentStore.datums[id].plane` —
   * never re-derived from `basePlaneId + offset`, since the core stamps the
   * sketch with exactly that basis (SCHEMA §7.3: only the resolved basis ever
   * reaches the worker, as a `custom` sketch plane). Same V1 freeze-at-creation
   * policy as sketch-on-face.
   */
  | {
      newOnDatum: { datumId: string };
      plane: SketchPlane;
      sketchId?: string;
    };

/** `sketchUpsert` result (SCHEMA §7.4 SketchUpsert result + solved coords). */
export interface SketchUpsertResult {
  sketchId: string;
  sketchRevision: number;
  dof: number;
  status: SketchSolveStatus;
  /** Constraint ids in conflict (SCHEMA §7.4; FRONTEND ids, unknown dropped by the
   *  client). Every solve write-back REPLACES the store's `conflictingIds` from this.
   *  Absent ⇒ []. */
  conflicting?: string[];
  /** CHANGED point coordinates after the solve, keyed `entityId.point`. */
  solvedPositions?: Record<string, [number, number]>;
}

/** One closed profile region (SCHEMA §7.4 SketchRegions). */
export interface SketchRegion {
  regionId: string;
  outerLoop: string[];
  holes: string[][];
  /**
   * Optional triangulated fill in **plane (u,v) coordinates**: flat `positions`
   * (u,v pairs) + `indices` (triangle triples). Consumers apply the plane basis.
   */
  previewTriangles?: {
    positions: number[];
    indices: number[];
    /** Number of declared holes actually removed from this fill. */
    holesSubtracted?: number;
  };
}

/** `finishSketch` result — the profiles an extrude/revolve can consume. */
export interface FinishSketchResult {
  regions: SketchRegion[];
}

// ── Sketch drag gesture (SCHEMA §7.4 BeginGesture / SolveDrag / EndGesture) ───
//
// The real client routes these to the worker's PlaneGCS gesture verbs; the mock
// runs a local identity solve. A drag is: beginGesture(point) → many solveDrag
// (latest-wins, fire-and-reconcile) → endGesture (commits ONE undo step).

/** `beginGesture` acknowledgement (`BeginGestureDto`). */
export interface BeginGestureResult {
  gestureId: number;
  ready: boolean;
}

/**
 * One incremental drag solve (`SolveDrag`; `DragSolveDto`). Carries the backend
 * `seq` (assigned per drag) so the client drops stale/superseded responses
 * latest-wins. `positions` are a PREVIEW (uncommitted), keyed by point entity id.
 */
export interface DragSolveResult {
  gestureId: number;
  seq: number;
  /** `success` | `partial` | `conflicting` | `redundant` | `superseded`. */
  status: string;
  dof: number;
  conflicting: string[];
  positions: Record<string, [number, number]>;
  solveMicros: number;
  /** True when this `seq` was superseded by a newer drag (positions empty). */
  superseded: boolean;
}

// ── Element identity (SCHEMA §7.5 AcquireElementIds) — pick → promote ─────────

/** One pick to promote (`{topoKey, anchor?}`). */
export interface PromotePick {
  topoKey: string;
  anchor?: { worldPoint?: [number, number, number]; surfaceUv?: [number, number] };
}

/** One promoted element (Rust-minted `elementId`; `PromotedElementDto`). */
export interface PromotedElement {
  topoKey: string;
  elementId: string;
  /** `face` | `edge` | `vertex`. */
  kind: string;
  bodyId: string;
}

/**
 * One element's geometric evidence (`elementInfo` → Rust `ElementInfoDto` →
 * the worker's `QueryElement` descriptor). MEASURE V1a's whole input.
 *
 * Every number here is the KERNEL's, computed with OCCT `GProp` on the real
 * BRep — not re-derived from the tessellation the viewport is drawing.
 */
export interface ElementInfo {
  elementId: string;
  topoKey: string;
  bodyId: string;
  /** `face` | `edge` | `vertex` | `body`. */
  kind: string;
  /** OCCT `GeomAbs_SurfaceType` ordinal; **0 == plane**, `-1` == absent. */
  surfaceType: number;
  /** OCCT `GeomAbs_CurveType` ordinal; **0 == line**, `-1` == absent. */
  curveType: number;
  /**
   * BOUNDING-BOX CENTRE — emphatically **not** a centroid (the kernel takes it
   * from `Bnd_Box`). Any UI reporting a distance between two of these must say
   * "center ↔ center".
   */
  center: [number, number, number];
  normal: [number, number, number];
  hasNormal: boolean;
  /** Bounding-box diagonal length — a coarse size proxy, not a measurement. */
  size: number;
  /**
   * The EXACT measured quantity: a face's **area** (mm²), an edge's **arc
   * length** (mm), a solid's **volume** (mm³).
   */
  magnitude: number;
}

// ── Topology repair (SCHEMA §9; M4b) — the `needs-repair` event + `resolveRefs` ─
//
// These MIRROR the Rust DTOs in `src-tauri/src/dto.rs` (camelCase serde):
//   NeedsRepairItem  == NeedsRepairItemDto  (lean banner/badge summary)
//   NeedsRepairEvent == NeedsRepairEvent    (`{revision, items}`; empty ⇒ cleared)
//   ResolveCandidate == ResolveCandidateDto (one ranked candidate)
//   ResolveRefResult == ResolveRefDto       (the un-lossy dry-run resolution)

/** One entry in the `needs-repair` event — a step left in NeedsRepair. */
export interface NeedsRepairItem {
  /** The op record id (`RecordId`) of the step needing repair. */
  opId: string;
  /** The op-input ref identity (SCHEMA §9 `refId`, e.g. `"op_5.input0"`). */
  refId: string;
  /** `ambiguous` | `no-candidates` | `low-confidence`. */
  reason: string;
  /** The `resolverVersion` the candidate scores were computed under. */
  scoringVersion?: number;
  /** How many candidates the ladder surfaced (0 ⇒ `no-candidates`). */
  candidateCount: number;
}

/**
 * The `needs-repair` event payload (`{revision, items}`). Emitted after EVERY
 * published regen; an EMPTY `items` means repairs cleared (drop the banner).
 */
export interface NeedsRepairEvent {
  revision: number;
  items: NeedsRepairItem[];
}

/** One ranked repair candidate (`ResolveCandidateDto`). */
export interface ResolveCandidate {
  /** The evidence handle (snapshot-scoped TopoKey) to promote on rebind. */
  topoKey: string;
  score: number;
  margin: number;
  /** Candidate centre in world coords — a geometric hint for highlighting. */
  worldPos: [number, number, number];
  summary: string;
  /** Per-feature score contributions (opaque; SCHEMA §9). */
  featureContributions?: unknown;
}

/**
 * One dry-run ref resolution (`ResolveRefDto`) — the FULL ladder result the
 * repair panel consumes. On `needsRepair` it carries the ranked `candidates[]`
 * plus `reason`/`ladderFailed`/`anchor`; on `autoBind`/`unchanged` the bound id.
 */
export interface ResolveRefResult {
  refId: string;
  /** `autoBind` | `needsRepair` | `unchanged`. */
  outcome: string;
  elementId?: string;
  topoKey?: string;
  score?: number;
  margin?: number;
  /** `history` | `descriptor` (needsRepair). */
  ladderFailed?: string;
  /** `ambiguous` | `no-candidates` | `low-confidence` (needsRepair). */
  reason?: string;
  scoringVersion?: number;
  uiLabel?: string;
  /** The selection intent captured when the ref was authored (opaque). */
  anchor?: unknown;
  /** Ranked candidates (needsRepair), sorted by score descending. */
  candidates: ResolveCandidate[];
}

/**
 * One ref to dry-run-resolve (`ResolveRefInput` — `{refId, …ElementRef}`). The
 * lean `needs-repair` event carries no ElementRef, so the panel passes `refId`
 * only and relies on the backend resolving the STORED ref by id; a `primary`/
 * `anchor` may be supplied when the caller has them.
 */
export interface ResolveRefRequest {
  refId: string;
  primary?: { bodyId: string; elementId?: string; kind: "body" | "face" | "edge" | "vertex" };
  anchor?: { worldPoint?: [number, number, number]; surfaceUv?: [number, number] };
}

// ── Projection hydration (SCHEMA §7.2 projection-updated) ─────────────────────
//
// The authoritative document projection the backend publishes on open/new/close/
// edit/regen. Field-identical to `documentStore.DocumentProjection` so the
// hydration bridge writes the store 1:1 (F-WP8 flag 2).

/** One body in the projection (mirrors `documentStore.BodyMeta`). */
export interface BodyProjection {
  id: string;
  name: string;
  visible: boolean;
}

/**
 * The model face a sketch is hosted on (mirrors Rust `SketchHostFaceDto`, from
 * `SketchAttachment::HostFace`'s `primary` binding).
 *
 * Identity only — the sketch's frozen basis is its own `plane`, never re-derived
 * from the host. `elementId` is the persistent Rust-minted id, so a face pick
 * that has been promoted compares `===` against it (SKETCH-ON-FACE W3: the
 * double-click-a-face re-entry). Absent for world- and datum-hosted sketches.
 */
export interface SketchHostFace {
  bodyId: string;
  elementId: string;
}

/** One sketch in the projection (mirrors `documentStore.SketchMeta`). */
export interface SketchProjection {
  id: string;
  name: string;
  visible: boolean;
  dof: number;
  /** `ok` | `under` | `over` | `error`. */
  status: string;
  /** Deterministic identity of authoritative plane/entities/constraints. */
  geometryToken: string;
  /** The host face, for a face-hosted sketch. Omitted for world/datum sketches. */
  hostFace?: SketchHostFace;
}

/**
 * One datum plane in the projection (mirrors Rust `DatumDto`).
 *
 * `kind`/`basePlaneId`/`offset` are the DEFINITION the user authored; `plane` is
 * the **backend-resolved** frame and is authoritative — a sketch attached to this
 * datum is stamped with exactly this basis by the core, so the frontend renders
 * and previews from `plane` and never re-derives it from the definition.
 *
 * Datums never cross the OCW1 wire (SCHEMA §7.3: only the resolved basis reaches
 * the worker, as a `custom` sketch plane), so this projection is their one route
 * from the core to the UI.
 */
export interface DatumProjection {
  id: string;
  name: string;
  /** PascalCase `DatumKind` token — V1 authors only `"OffsetFromPlane"`. */
  kind: string;
  /** `"XY"` | `"XZ"` | `"YZ"`, or another datum's id (chained offsets). */
  basePlaneId: string;
  offset: number;
  plane: {
    origin: [number, number, number];
    xAxis: [number, number, number];
    yAxis: [number, number, number];
    normal: [number, number, number];
  };
  /** `false` ⇒ the definition did not resolve; the datum cannot host a sketch. */
  resolvedValid: boolean;
}

/** The `projection-updated` payload (mirrors `documentStore.DocumentProjection`). */
export interface DocumentProjectionWire {
  status: "empty" | "loading" | "ready";
  /**
   * The document this projection describes. Revisions restart at 1 for every
   * newly opened runtime, so the stale-projection guard scopes its revision
   * compare to one document. Optional: the mock lane omits it (same-document
   * semantics — the mock never replaces the document out from under the store).
   */
  documentId?: string;
  revision: number;
  title: string;
  dirty: boolean;
  bodies: Record<string, BodyProjection>;
  sketches: Record<string, SketchProjection>;
  /** Datum planes keyed by id. Always sent by the backend (possibly empty). */
  datums: Record<string, DatumProjection>;
  features: FeatureRecord[];
  /**
   * Applied op count (timeline cursor): `features[0, appliedOps)` are applied,
   * `[appliedOps, totalOps)` are drafts beyond the rollback bar. Optional — the
   * backend always sends both; no frontend consumer wired yet (MODEL-HARDEN W0
   * seam for the legacy-draft recovery hint).
   */
  appliedOps?: number;
  /** Total op count (timeline length). See {@link appliedOps}. */
  totalOps?: number;
}

/** The `regen-finished` payload (F-WP8 flag 3). `sourceRevision` is the revision
 *  the regen was fenced against at `begin_regen` (MODEL-HARDEN W0.5 commit
 *  provenance); the mock lane may omit it (then it falls back to `revision`). */
export interface RegenFinished {
  revision: number;
  /** The revision this regen was PREPARED for (rapid-commit correlation). */
  sourceRevision?: number;
  /** `published` | `superseded` | `failed` | `cancelled` | `noop`. */
  outcome: string;
  /** Failure reason for `outcome === "failed"` (SCHEMA §8). Absent otherwise. */
  message?: string;
  /**
   * Records whose POST-regen step state is Error, even when the regen itself
   * published (other steps' geometry). A commit correlated to this completion whose
   * own `recordId` appears here is a FAILURE (empty bodies + the step message) — it
   * must NOT correlate as success off the sibling `document-changed` (MODEL-HARDEN).
   * Present only when non-empty; the mock lane omits it.
   */
  failedSteps?: Array<{ recordId: string; message: string }>;
  /**
   * Per record-id, the body ids that op created (incl. split children) or modified
   * in THIS published regen. Lets a correlated commit scope its result bodies to
   * ONLY its own op's bodies (MODEL-HARDEN); absent ⇒ fall back to the full
   * `document-changed` list (mock lane).
   */
  affectedBodies?: Record<string, string[]>;
}

/**
 * The `worker-status` event payload (mirrors Rust `WorkerStatusDto`) — the C++
 * sidecar lifecycle the status bar surfaces. The mock never emits it.
 */
export interface WorkerStatus {
  /** `starting` | `ready` | `restarting` | `failed`. */
  state: "starting" | "ready" | "restarting" | "failed";
  /** The worker epoch this transition belongs to (`0` when unknown). */
  epoch: number;
}

// ── Model operations (SCHEMA §7.3 op payloads) ───────────────────────────────
//
// These mirror the JSON the C++ worker consumes inside `ExecutePlan.ops`. The
// mock accepts the SAME shapes so the later real-backend swap (F-WP8) is a no-op
// for the tool layer. Values keep OneCAD-CPP `operationTypeName` spelling
// (PascalCase). The vertical slice authors Extrude | Fillet | Boolean.

export type OpType =
  | "Extrude"
  | "Revolve"
  | "Fillet"
  // Chamfer shares FilletChamferParams with Fillet (SCHEMA §7.3) but is its OWN
  // `opType` on the wire, so a preview draft must be able to name it.
  | "Chamfer"
  | "Boolean"
  | "Shell"
  | "LinearPattern"
  | "CircularPattern"
  | "MirrorBody";

/** Extrude end condition (SCHEMA §7.3 ExtrudeParams). */
export type ExtrudeMode = "Blind" | "ThroughAll" | "Symmetric" | "ToNext" | "ToFace";
/** Boolean fused into a feature op (SCHEMA §7.3 `booleanMode`). */
export type FeatureBooleanMode = "NewBody" | "Add" | "Cut" | "Intersect";
/** Standalone body-body boolean (SCHEMA §7.3 BooleanParams `operation`). */
export type BooleanOperation = "Union" | "Cut" | "Intersect";

/**
 * A semantic reference (SCHEMA §7.3 `inputs[]` element) — the topological input
 * to an op carried as evidence + identity so the resolution ladder can rebind
 * after edits. The mock only reads `primary`/`anchor`; `intent.descriptor` is
 * captured by Rust in F-WP8. Kept minimal here on purpose.
 */
export interface SemanticRef {
  primary: {
    bodyId: string;
    elementId?: string;
    kind: "body" | "face" | "edge" | "vertex";
  };
  anchor?: {
    worldPoint?: [number, number, number];
    surfaceUv?: [number, number];
  };
}

/** Extrude op params (SCHEMA §7.3 ExtrudeParams — vertical-slice subset). */
export interface ExtrudeParams {
  distance: number;
  draftAngleDeg?: number;
  extrudeMode?: ExtrudeMode;
  booleanMode?: FeatureBooleanMode;
  targetBodyId?: string;
  twoDirections?: boolean;
  extrudeMode2?: ExtrudeMode;
  distance2?: number;
  /**
   * `ToFace` end-condition targets (SCHEMA §7.3 typed semantic refs — a bare face
   * id would carry no anchor/intent and could not be rebound by the resolution
   * ladder after a parametric edit). Marshalled ONLY when the matching
   * `extrudeMode`/`extrudeMode2` is `ToFace`.
   */
  targetFace?: SemanticRef;
  targetFace2?: SemanticRef;
}

/**
 * A revolve/pattern axis (SCHEMA §7.3 `axis`, Rust `AxisRef`, serde tag `kind`).
 * The vertical-slice revolve tool only authors the `sketchLine` variant (a line
 * entity in the profile's sketch); `edge` mirrors the Rust variant for parity.
 */
export type AxisRef =
  | { kind: "sketchLine"; sketchId: string; lineId: string }
  | { kind: "edge"; bodyId: string; edgeId: string };

/**
 * Revolve op params (SCHEMA §7.3 / Rust `RevolveParams`). `angleDeg` is the sweep
 * in DEGREES — the same unit Rust's `angleDeg` Scalar carries, so the command
 * mapper passes it through with NO conversion. `axis` is the sketch line to
 * revolve around; `booleanMode`/`targetBodyId` mirror ExtrudeParams.
 */
export interface RevolveParams {
  angleDeg: number;
  axis?: AxisRef;
  booleanMode?: FeatureBooleanMode;
  targetBodyId?: string;
}

/** Fillet/Chamfer op params (SCHEMA §7.3 FilletChamferParams; `mode` distinguishes). */
export interface FilletParams {
  mode: "Fillet" | "Chamfer";
  radius: number;
  /** TopoKeys (snapshot-scoped) or ElementIds; resolved through the ladder. */
  edgeIds: string[];
  chainTangentEdges?: boolean;
}

/** Standalone body-body boolean op params (SCHEMA §7.3 BooleanParams). */
export interface BooleanParams {
  operation: BooleanOperation;
  targetBodyId: string;
  toolBodyId: string;
}

/**
 * Shell op params (Rust `ShellParams`). `openFaces` are the removed faces
 * (ElementIds or snapshot TopoKeys, resolved through the ladder); `thickness` is
 * the wall thickness. `targetBodyId` is the shelled body.
 */
export interface ShellParams {
  thickness: number;
  openFaces: string[];
  targetBodyId?: string;
}

/**
 * Linear-pattern op params (Rust `LinearPatternParams`). `direction` is a WORLD
 * unit vector (the Rust port uses a single `direction: Vec3`, NOT an axis enum —
 * the UI's axis chip maps to one of the world axes). `spacing` is the per-step
 * distance, `count` the total instances (source + clones).
 */
export interface LinearPatternParams {
  sourceBodyId?: string;
  direction: [number, number, number];
  spacing: number;
  count: number;
  fuseResult?: boolean;
}

/**
 * Circular-pattern op params (Rust `CircularPatternParams`). The axis is a world
 * ray (`axisOrigin` + `axisDirection` Vec3s); `angleDeg` is the TOTAL sweep.
 */
export interface CircularPatternParams {
  sourceBodyId?: string;
  axisOrigin: [number, number, number];
  axisDirection: [number, number, number];
  angleDeg: number;
  count: number;
  fuseResult?: boolean;
}

/**
 * Mirror-body op params (Rust `MirrorBodyParams`). The mirror plane is a world
 * point + normal (`planePoint` + `planeNormal` Vec3s).
 */
export interface MirrorBodyParams {
  sourceBodyId?: string;
  planePoint: [number, number, number];
  planeNormal: [number, number, number];
  fuseWithOriginal?: boolean;
}

/**
 * One op in an `ExecutePlan` (SCHEMA §7.3), discriminated by `opType`. An
 * optional `featureId` re-targets an EXISTING feature (parametric edit —
 * double-click a history entry → re-drag). `sketchId`/`regionId` on Extrude tell
 * the mock which finished region to synthesize a body from (the worker resolves
 * the region from the semantic ref in F-WP8).
 */
export type OperationOp =
  | {
      opType: "Extrude";
      opId?: string;
      featureId?: string;
      sketchId: string;
      regionId: string;
      inputs?: SemanticRef[];
      params: ExtrudeParams;
    }
  | {
      opType: "Revolve";
      opId?: string;
      featureId?: string;
      sketchId: string;
      regionId: string;
      inputs?: SemanticRef[];
      params: RevolveParams;
    }
  | {
      opType: "Fillet";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: FilletParams;
    }
  // Chamfer shares FilletChamferParams with Fillet (C++ + SCHEMA §7.3); `mode`
  // on the params distinguishes them.
  | {
      opType: "Chamfer";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: FilletParams;
    }
  | {
      opType: "Boolean";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: BooleanParams;
    }
  | {
      opType: "Shell";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: ShellParams;
    }
  | {
      opType: "LinearPattern";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: LinearPatternParams;
    }
  | {
      opType: "CircularPattern";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: CircularPatternParams;
    }
  | {
      opType: "MirrorBody";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: MirrorBodyParams;
    };

/**
 * One feature-timeline entry (mirrors the Rust projection DTO; identical shape to
 * the store's FeatureMeta so the controller maps it 1:1). The mock now emits
 * these with real values (e.g. "25.0 mm").
 */
export interface FeatureRecord {
  id: string;
  kind:
    | "sketch"
    | "extrude"
    | "revolve"
    | "fillet"
    | "boolean"
    | "shell"
    | "linearPattern"
    | "circularPattern"
    | "mirror";
  /**
   * The exact `opType` the feature was authored as (`"Extrude"`, `"Chamfer"`, …).
   * `kind` is a coarse icon bucket that the backend folds Chamfer+Shell into
   * `fillet` and the pattern/mirror ops into `boolean`, so a re-edit must route
   * on THIS. Optional for backwards compatibility with a projection emitted
   * before the field existed.
   */
  opType?: string;
  label: string;
  valueText: string;
  status: "ok" | "dirty" | "error" | "needsRepair";
  /** Worker failure reason for an errored step (`status === "error"`), surfaced as
   *  the HistoryList row tooltip (MODEL-HARDEN W0.5). Absent for any other status. */
  statusMessage?: string;
  /** Whether the step is suppressed (backend-authoritative; a concurrent Rust change
   *  adds this to `FeatureDto`). Optional so an older backend payload still parses. */
  suppressed?: boolean;
}

/** `applyOperation` / `endPreview(commit)` / `undo` / `redo` result. */
export interface ApplyOperationResult {
  revision: number;
  changedBodies: BodyMeshRef[];
  removedBodies: string[];
  /** Full feature timeline after the change (authoritative). */
  features: FeatureRecord[];
  /** Human label of the op just applied/undone, for a status hint ("Extrude"). */
  opLabel?: string;
  /** Set ONLY when the correlated regen FAILED (`regen-finished{outcome:"failed"}`)
   *  — the worker's reason, so a caller can surface WHY without inspecting geometry
   *  (MODEL-HARDEN W0.5). Empty changedBodies + this ⇒ a hard failure, not a no-op.
   *  The mock lane never sets it. */
  errorMessage?: string;
}

// ── Two-level preview (NEW_SPEC §15) ─────────────────────────────────────────

/**
 * Params a preview update carries (opType-specific; loosely typed for the wire).
 * One union per previewable opType — the `ipc/previewOps.ts` builders narrow it
 * back down and reject anything that does not belong to the op being previewed.
 */
export type PreviewParams = Partial<ExtrudeParams> &
  Partial<RevolveParams> &
  Partial<FilletParams> &
  Partial<ShellParams> &
  Partial<BooleanParams> & { [k: string]: unknown };

/** `beginPreview` draft — the base op the drag will refine. */
export interface PreviewDraft {
  /** Stable RecordId reused by exact preview and commit. Minted when absent. */
  opId?: string;
  opType: OpType;
  sketchId?: string;
  regionId?: string;
  /**
   * Typed op inputs, carried verbatim into the session. Load-bearing for
   * Fillet/Chamfer ONLY — `wireOperation` drops an op's top-level inputs
   * everywhere else, and `filletParams` synthesizes the typed `params.edges` from
   * these (in lockstep with `params.edgeIds`). Shell/Boolean bodies ride
   * `params.targetBodyId`/`toolBodyId`. See `ipc/previewOps.ts`.
   */
  inputs?: SemanticRef[];
  params: PreviewParams;
}

/** `beginPreview` result — the session + the body the L2 mesh is published under. */
export interface PreviewSession {
  sessionId: string;
  previewBodyId: string;
}

/** Backend preview failure. Stale snapshots are transient; repair failures are structural. */
export interface PreviewFailure {
  kind:
    | "opFailed"
    | "invalidCommand"
    | "worker"
    | "noDocument"
    | "needsRepair"
    | "stalePreview"
    | "unknown";
  message: string;
  structural: boolean;
  /** Backend repair evidence retained for the future repair UI. */
  evidence?: unknown[];
}

/**
 * An exact L2 preview result (NEW_SPEC §15 "Replace preview with exact result").
 * Carries its `epoch` so the frontend can reconcile against the latest params it
 * sent and discard stale responses (Invariant: L1 removed only after the matching
 * epoch arrives). A success carries a full MESH1 blob; a failure carries the
 * backend's typed reason and leaves the last valid mesh visible.
 */
export interface PreviewResult {
  sessionId: string;
  epoch: number;
  bodyId: string;
  mesh?: ArrayBuffer;
  /** All exact candidate bodies. Cut/split operations may return more than one. */
  bodies?: { bodyId: string; mesh: ArrayBuffer }[];
  /** Committed head bodies hidden while their candidate replacements are shown. */
  replacedBodyIds?: string[];
  error?: PreviewFailure;
  /** True for the final exact mesh delivered on commit. */
  committed?: boolean;
}
