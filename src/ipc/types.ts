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
  /**
   * An unresolved crash-recovery offer names this path — the file on disk is OLDER
   * than work a previous session left behind. Opening it is how that work gets
   * destroyed, so the card has to say so.
   */
  hasRecovery?: boolean;
}

/**
 * Crash-recovery offer surfaced on the start screen: a prior session left an
 * autosave behind. `check_recovery` returns every outstanding offer, newest first;
 * `recover_document` accepts (restore) or rejects (discard) one by `documentId`.
 * Keep 1:1 with the Rust `RecoveryInfoDto` (serde camelCase).
 */
export interface RecoveryInfo {
  /** The document this offer restores — the key `recoverDocument` acts on. */
  documentId: string;
  /**
   * The document's title at the last autosave. Absent for an ORPHAN: a container
   * whose session marker did not survive, where nothing records the name.
   */
  title?: string;
  /** Absolute path of the document the autosave belongs to (absent if never saved). */
  originalPath?: string;
  /** Absolute path of the autosave sidecar the crashed session left behind. */
  autosavePath: string;
  /**
   * Epoch-millis mtime of the autosave (last time work was captured), or `0` when
   * the filesystem could not answer — render no timestamp rather than the epoch.
   */
  modifiedMs: number;
}

/**
 * What to do about a pending crash-recovery offer that names the path being opened.
 * Required in that case: without it `openDocument` rejects with `recoveryPending`
 * rather than opening the older file and destroying the autosave.
 */
export type OnRecovery = "openSaved";

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

/**
 * Authoritative state after a completed container write.  A save can succeed
 * while an edit races the write, so callers must use `clean` rather than
 * treating command resolution itself as permission to replace/close a document.
 */
export interface SaveOutcome {
  documentId: string;
  savedRevision: number;
  currentRevision: number;
  clean: boolean;
  path: string;
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
  | "Symmetric"
  /**
   * Two points share a Y / an X — the POINT-PAIR axis alignments (SNAP P3).
   *
   * What an H/V alignment guide actually asserts, and expressible by neither
   * existing kind: `Horizontal`/`Vertical` are LINE-only (they constrain one
   * entity), and a zero-valued `HorizontalDistance`/`VerticalDistance` is a
   * user-visible DRIVING dimension. SCHEMA §7.3 carries both.
   */
  | "HorizontalPoints"
  | "VerticalPoints";

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

/**
 * PER-ENTITY constrained state (SCHEMA §7.4 `entityStates`). `SketchSolveStatus`
 * answers for the whole sketch; this answers for one entity, so a pinned-down
 * line can read differently from the free circle beside it in the SAME
 * under-constrained sketch.
 *
 * Exactly three tokens, and there is deliberately no `partiallyConstrained` and
 * no per-entity DOF count: PlaneGCS reports null-space MEMBERSHIP per parameter
 * ("this parameter is still free"), never how much freedom an entity has, so a
 * count would be fabricated. `conflicting` outranks the other two.
 */
export type EntityConstrainedState = "underConstrained" | "fullyConstrained" | "conflicting";

/**
 * Per-entity constrained states keyed by FRONTEND entity id (SCHEMA §7.4
 * `entityStates`).
 *
 * An entity ABSENT from the map is **unknown** — a reader MUST NOT default it to
 * `underConstrained`; it falls back to the whole-sketch tint. An EMPTY map is
 * therefore "nothing to say about any entity", which is what an ellipse-bearing
 * sketch (never diagnosed by the solver) and every hand-written client double
 * report.
 */
export type SketchEntityStates = Record<string, EntityConstrainedState>;

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
  /** Per-entity constrained state of the entering solve (SCHEMA §7.4). Seeds the
   *  sketch store's `entityStates`. Optional for the same reason `solvedCurves`
   *  is — hand-written `CadClient` doubles predate it; absent ⇒ `{}` (unknown
   *  for every entity), never "everything is under-constrained". */
  entityStates?: SketchEntityStates;
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
   * two-rung ladder as `face_sketch_plane`. A successful promotion now binds the
   * `elementId` into the same unchanged worker head; `topoKey` remains useful as
   * snapshot-scoped pre-promotion evidence and defense against stale UI state.
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
  /**
   * CHANGED curve parameters after the solve (SCHEMA §7.4 `curves`), keyed by
   * FRONTEND entity id — the channel `solvedPositions` cannot carry (a radius
   * moves no point; an arc's radius/angles are not points at all). Both clients
   * always populate it; optional for the same reason `solvedPositions` is —
   * hand-written `CadClient` mocks predate it, so consumers read it as
   * `solvedCurves ?? {}`.
   */
  solvedCurves?: Record<string, CurveParams>;
  /** Per-entity constrained state after this solve (SCHEMA §7.4). Every solve
   *  write-back REPLACES the store's `entityStates` from it. Absent ⇒ `{}`. */
  entityStates?: SketchEntityStates;
}

/** One closed profile region (SCHEMA §7.4 SketchRegions). */
export interface SketchRegion {
  regionId: string;
  /** Identity format returned with this region; undefined is mock/legacy-only. */
  regionIdentityVersion?: number;
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
  /** Canonical region-id format; production P2 worker responses use 2. */
  regionIdentityVersion?: number;
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
  /**
   * Per-entity constrained state of the COMMITTED sketch this gesture opened
   * against (SCHEMA §7.4). GESTURE-FIXED: `solveDrag` never carries it and
   * `endGesture` echoes this exact map, so a consumer HOLDS this one for the
   * whole gesture. Absent ⇒ `{}`.
   */
  entityStates?: SketchEntityStates;
}

/**
 * WHAT the pointer grabbed (SCHEMA §7.4 `BeginGesture.drag`) — the gesture's
 * TARGET KIND. Optional on `beginGesture`: an absent target means `point`, the
 * pre-SP-2 gesture, whose request stays byte-identical to the one before the
 * kinds existed.
 *
 * `entityId` is a FRONTEND id; the client translates it to the backend uuid (the
 * point ref for `point`, the entity uuid for every other kind). An unknown kind
 * or role is refused at the command boundary rather than degraded to a point
 * drag — degrading would move a handle the user never grabbed (§7.4).
 */
export interface GestureTarget {
  /** `point` (default) | `arcEnd` | `radius` | `entityBody`. */
  kind: "point" | "arcEnd" | "radius" | "entityBody";
  /** The grabbed entity — a point REF (`"e1.Start"`) for `point`, else an entity id. */
  entityId: string;
  /** Which handle of `entityId`. REQUIRED for `arcEnd`; ignored by `radius`/`entityBody`. */
  role?: "Start" | "End" | "Center";
  /**
   * Pointer-down position in sketch-plane (u,v). The backend derives the per-kind
   * offset from it ONCE, at `beginGesture`, so a gesture cannot drift as it is
   * dragged: `entityBody` moves by `target − grab`, `radius` keeps the radial
   * offset `|grab − center| − radius`. IGNORED by `point`/`arcEnd` (the handle
   * teleports to the cursor).
   */
  grab?: [number, number];
}

/**
 * The CHANGED members of one curve after a solve (SCHEMA §7.4 `curves`).
 *
 * Per-member optional under the same incremental discipline as `positions`: an
 * ABSENT member is UNCHANGED, never zero. Angles are RADIANS CCW from +X — the
 * wire domain, not the UI degree domain (`angleUnits.ts`).
 */
export interface CurveParams {
  /** New radius (mm) of a Circle/Arc. */
  radius?: number;
  /** New start angle (radians) of an Arc. */
  startAngle?: number;
  /** New end angle (radians) of an Arc. */
  endAngle?: number;
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
  /**
   * CHANGED curve parameters (SCHEMA §7.4 `curves`), keyed by FRONTEND entity id.
   * Emitted for EVERY drag kind, not only `radius`: a `Tangent` propagates even a
   * plain point drag into a neighbouring curve's radius.
   *
   * OPTIONAL for the same reason `solvedCurves` is: both real clients always
   * populate it, but a pre-SP-2 test double has no way to know about a channel
   * that did not exist when it was written. Consumers read it as `?? {}`, the
   * idiom `solvedPositions` already established.
   */
  curves?: Record<string, CurveParams>;
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

/**
 * One body's exact kernel mass properties (`massProperties` → Rust
 * `MassPropertiesDto` → the worker's `QueryMassProperties`; SCHEMA §7.5, WP-C1).
 *
 * Every number is OCCT `GProp` over the real BRep, not re-derived from the mesh
 * the viewport is drawing.
 *
 * **Density-free.** The app has no material system, so `principalMoments` are
 * pure VOLUME integrals (mm⁵) about the centroid, never kg·mm². A UI must not
 * label them "mass moments".
 */
export interface MassProperties {
  /** The body this reading describes — the id the caller asked about. */
  bodyId: string;
  /** Enclosed volume, mm³. 0 for a shape with no solid — an honest reading. */
  volume: number;
  /** Total area of every face, mm². */
  surfaceArea: number;
  /**
   * The true VOLUME centre of mass, mm. Distinct from `ElementInfo.center`,
   * which is a bounding-box centre.
   */
  centroid: [number, number, number];
  /** Principal moments about the centroid (mm⁵), paired IN ORDER with `principalAxes`. */
  principalMoments: [number, number, number];
  /** The principal frame: three unit, right-handed rows. */
  principalAxes: [
    [number, number, number],
    [number, number, number],
    [number, number, number],
  ];
}

/**
 * A picked face/edge's surface/curve classification + a seatable frame
 * (`classifyElement` → Rust `ClassifyElementDto` → the worker's
 * `ClassifyElement`; SCHEMA §7.5, Component Library WP-0.1).
 *
 * The placement solver's hover query, distinct from `ElementInfo`: that one's
 * `normal` is a surface normal (meaningless as an axis for a cylinder) and
 * carries no radius. This exists specifically for concentric/flush mate
 * seating.
 */
export interface ClassifyResult {
  /** `face` | `edge` | `other`. */
  kind: string;
  /** `plane` | `cylinder` | `cone` | `sphere` | `torus` | `other`. Empty unless `kind === "face"`. */
  surfaceType: string;
  /** `line` | `circle` | `ellipse` | `other`. Empty unless `kind === "edge"`. */
  curveType: string;
  /**
   * A seatable frame — present only for plane/cylinder faces and line/circle
   * edges, the kinds a mate solver can snap against.
   */
  frame: ClassifyFrame | null;
}

export interface ClassifyFrame {
  /** Plane origin, or cylinder/circle axis location. */
  origin: [number, number, number];
  /** Plane normal. `null` for cylinder/circle/line frames (those carry `axis`). */
  normal: [number, number, number] | null;
  /** Cylinder/circle axis direction, or a line's own direction. `null` for a plane frame. */
  axis: [number, number, number] | null;
  /** Cylinder or circle radius. `null` for plane/line frames. */
  radius: number | null;
}

/**
 * One project template (spec §8; Rust `ProjectTemplateDto`). `previewDataUrl`
 * is inlined because the webview has no filesystem capability — a path would
 * be unreadable to it.
 */
export interface ProjectTemplate {
  id: string;
  name: string;
  description?: string;
  previewDataUrl?: string;
}

/**
 * "Save as Component" input (spec §7; Rust `NewComponentSpec`) — identity and
 * metadata only.
 *
 * Everything about GEOMETRY is the backend's: which body is baked, in which
 * codec, and where the blob lands. The frontend names the component; it never
 * describes how the solid is stored.
 */
export interface NewComponentSpec {
  /** Namespaced (`<ns>.<name>`), validated backend-side. */
  id: string;
  version: string;
  name: string;
  category: string[];
  tags: string[];
  designation?: string;
  /** `[attachments]` — named mate points, `{ on, accepts }` per key. */
  attachments: Record<string, LibraryAttachment>;
  /**
   * `[parameters]` — the `role: "free"` parameters this component exposes,
   * each bound to a document VARIABLE (WP-F1.3). Editing one on a placed
   * instance re-bakes the frozen source document with the new value. Absent or
   * empty ⇒ a component with no editable size.
   */
  parameters?: Record<string, NewComponentParameter>;
}

/**
 * One declared free parameter of an authored component (Rust
 * `FreeParameterInput`). Numeric only — a document variable is a number, so
 * there is nothing else a re-bake could set.
 */
export interface NewComponentParameter {
  /** The source document's variable name this parameter drives. */
  key: string;
  /** The variable's value at authoring time — the package's declared default. */
  value: number;
}

/**
 * An opt-in upgrade offer for one placed instance (Rust `ComponentUpgradeDto`;
 * spec §3.3). Reporting only — applying it is an explicit `replaceComponent`.
 */
export interface ComponentUpgrade {
  componentId: string;
  currentVersion: string;
  latestVersion: string;
  latestRevision: string;
}

/**
 * The outcome of a `replaceComponent` (Rust `ReplaceComponentReportDto`).
 *
 * `droppedMateAttachment` names the attachment the old instance was mated
 * through when the new component does not declare it: the mate is dropped and
 * the instance holds its frozen placement rather than being re-bound to a
 * different attachment.
 */
export interface ReplaceComponentReport {
  droppedMateAttachment?: string;
}

/**
 * What `replaceComponent` resolves with (Rust `ReplaceComponentResultDto`).
 *
 * A replace is an `UpdateOperationParams` that re-bakes geometry and schedules a
 * regen, so resolving is NOT succeeding: read the `terminal` (via
 * `classifyRegen`), exactly as `setComponentParams`/`detachComponent` require.
 * The {@link ReplaceComponentReport} keeps its own shape and stays the only
 * place a dropped mate attachment is named (W5 result truth).
 */
export interface ReplaceComponentResult extends ApplyOperationResult {
  report: ReplaceComponentReport;
}

/**
 * One indexed library component (Component Library WP-1.3; mirrors Rust
 * `LibraryComponentDto` — `onecad-library`'s `IndexEntry` plus the identity
 * fields the index keys on).
 */
export interface LibraryComponent {
  id: string;
  version: string;
  name: string;
  category: string[];
  tags: string[];
  sourceKind: string;
  revision: string;
  /** `sourceKind === "generator"` only (WP-1.5 ghost-preview draft identity). */
  generatorId?: string;
  generatorVersion?: number;
  /** `[attachments]` table verbatim — keyed by attachment name (WP-1.5 snap solver input). */
  attachments: Record<string, LibraryAttachment>;
  /**
   * `[parameters]` table verbatim (WP-2.4: the configurator's role/domain/
   * snap source — role must be checked before `setComponentParams` accepts
   * a key, same enforcement the backend applies).
   */
  parameters: Record<string, ComponentParameterSpec>;
  /**
   * `metadata.designation` (spec §2.1 BOM string template, e.g. `"ISO 4762
   * M{thread}x{length}"`) — substitute each `{key}` with the CURRENT param
   * value to get the live designation string. Absent when the package
   * declares none.
   */
  designation?: string;
}

/** One `[attachments].<key>` entry (mirrors Rust `AttachmentSpec`, spec §2.1). */
export interface LibraryAttachment {
  /** Local geometry the attachment names (`"face:head_underside"`, `"cylinder:shank"`). */
  on: string;
  /** Geometry kinds this attachment mates with (`"plane"`/`"cylinder"`/`"hole"`/`"circularEdge"`). */
  accepts: string[];
  /**
   * The component-local basis this attachment seats FROM (WP-F1.1; mirrors
   * Rust `AttachmentFrame`). ABSENT ⇒ the identity frame, i.e. the component
   * seats at its own model origin.
   *
   * Already orthonormal and right-handed (`y = z × x`, derived): the backend
   * normalizes every frame at manifest parse, so the solver may assume it.
   */
  frame?: LibraryAttachmentFrame;
}

/** A `[attachments].<key>.frame` (mirrors Rust `AttachmentFrame`, spec §2.1). */
export interface LibraryAttachmentFrame {
  /** Component-local point that lands on the target's seat point. */
  origin: [number, number, number];
  /** Direction aligned to the target's axis / outward normal. */
  z: [number, number, number];
  /** Roll reference about `z`. */
  x: [number, number, number];
}

/** `component.toml` `[parameters].<key>`'s role (spec §2.1). */
export type ComponentParameterRole = "free" | "table" | "computed";

/**
 * One `[parameters].<key>` entry (mirrors Rust `ParameterSpec`). Fields are
 * role-dependent — `role: "free"` is the only role the configurator renders
 * an editable control for; `"table"`/`"computed"` are display-only, derived
 * values `setComponentParams` refuses (spec §3.2, enforced server-side).
 */
export interface ComponentParameterSpec {
  role: ComponentParameterRole;
  /** `role: "free"` only: the current/default value for a string-domain param (e.g. a thread designation). */
  key?: string;
  /** `role: "free"` only: the current/default value for a numeric param. */
  value?: ComponentParamValue;
  /** `role: "free"` only, when the value must come from a fixed set (e.g. a thread designation). */
  domain?: ComponentParamValue[];
  snap?: string;
  min?: number;
  /** `role: "table"` only: the dimension-table column this value derives from. */
  from?: string;
}

/**
 * A free-parameter override value (mirrors Rust `ComponentParamValue`,
 * untagged on the wire — a bare JSON scalar, never a wrapper object).
 */
export type ComponentParamValue = number | string | boolean;

/**
 * The placement gesture's recorded snap, sent with a `placeComponent` commit
 * (spec §5.4 step 5; mirrors Rust `PlaceComponentMateInput`). Raw pick
 * EVIDENCE: the backend promotes `targetTopoKey` to a Rust-minted `ElementId`
 * at the head when `targetElementId` is absent — the frontend never
 * fabricates identity. A mate that cannot be promoted refuses the whole
 * placement (fail closed), never a silent free-space record.
 */
export interface PlaceComponentMate {
  /** Key into the component's `[attachments]` table (the Tab-cycled match). */
  selfAttachment: string;
  targetBodyId: string;
  targetTopoKey: string;
  targetElementId?: string;
  targetKind: "face" | "edge";
  /** Snap classification — matches `placementSolver.ts` `MateSnapKind`. */
  kind: "concentric" | "coincident" | "concentricAndCoincident";
  flipped: boolean;
  /** The pick's world position — anchor evidence for the resolution ladder. */
  anchorWorldPoint: [number, number, number];
}

/** A `reindexLibrary()` outcome (mirrors Rust `ReindexReportDto`). */
export interface ReindexReport {
  total: number;
  indexed: number;
  /** `"<packageDir>: <reason>"` per skipped (malformed) package. */
  skipped: string[];
}

// ── Topology repair (SCHEMA §9; M4b) — the `needs-repair` event + `resolveRefs` ─
//
// These MIRROR the Rust DTOs in `src-tauri/src/dto.rs` (camelCase serde):
//   NeedsRepairItem  == NeedsRepairItemDto  (lean banner/badge summary)
//   NeedsRepairEvent == NeedsRepairEvent    (`{revision, snapshotId, items}`; empty ⇒ cleared)
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
  /**
   * The body the failing step OPERATES ON, bare uuid (H9;
   * `NeedsRepairItemDto.bodyId`). A rebind must promote its chosen `TopoKey`
   * against some body — without this the panel could only guess, and refused
   * outright in a multi-body document. Absent when the backend could not derive
   * one (or on an older backend), in which case the caller falls back to its
   * single-body / selection derivation.
   */
  bodyId?: string;
}

/**
 * The `needs-repair` event payload (`{revision, items}`). Emitted after EVERY
 * published regen; an EMPTY `items` means repairs cleared (drop the banner).
 */
export interface NeedsRepairEvent {
  revision: number;
  /** Snapshot that produced the repairs; candidate TopoKeys are valid only here. */
  snapshotId: number;
  items: NeedsRepairItem[];
}

/**
 * A feature's upstream/downstream transitive closures from the core dependency
 * graph (`FeatureDependenciesDto`; H10 dependency view). Read-only lineage: never
 * contains the queried feature's own id. Consumed by delete/suppress confirmations
 * (dependent counts) and the inspector's "Depends on / Used by" section.
 */
export interface FeatureDependencies {
  /** Record ids this feature depends on (transitive). */
  upstream: string[];
  /** Record ids that depend on this feature (transitive). */
  downstream: string[];
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
  /**
   * The authoritative body this candidate belongs to. NOT a wire field on
   * `ResolveCandidateDto` — the backend only carries it once, on the parent
   * `ResolveRefResult.bodyId` (SCHEMA §9), so the FE denormalizes it onto each
   * candidate when building this list (`RepairPanel`), letting `rebindCandidate`
   * promote against the body the resolve ladder actually enumerated instead of
   * guessing from the (possibly stale) repair item.
   */
  bodyId?: string;
}

/**
 * One dry-run ref resolution (`ResolveRefDto`) — the FULL ladder result the
 * repair panel consumes. On `needsRepair` it carries the ranked `candidates[]`
 * plus `reason`/`ladderFailed`/`anchor`; on `autoBind`/`unchanged` the bound id.
 */
export interface ResolveRefResult {
  /** Exact snapshot that enumerated this candidate set. */
  snapshotId: number;
  /** Document revision current during candidate enumeration. */
  revision: number;
  refId: string;
  /** Body used for candidate enumeration, when the stored ref names one. */
  bodyId?: string;
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
  /** Candidate provenance supplied by the repair event. */
  snapshotId?: number;
  revision?: number;
  primary?: { bodyId: string; elementId?: string; kind: "body" | "face" | "edge" | "vertex" };
  anchor?: { worldPoint?: [number, number, number]; surfaceUv?: [number, number] };
}

// ── Projection hydration (SCHEMA §7.2 projection-updated) ─────────────────────
//
// The authoritative document projection the backend publishes on open/new/close/
// edit/regen. Field-identical to `documentStore.DocumentProjection` so the
// hydration bridge writes the store 1:1 (F-WP8 flag 2).

/**
 * sRGB+A color as `[r, g, b, a]` with each channel in `0..255`.
 *
 * READONLY on purpose: a color is a value, never mutated in place. Keeping it
 * readonly is what lets the mesh fixtures/`mockMeshes.FaceColor` (also readonly)
 * flow into every consumer without a cast.
 */
export type Rgba = readonly [number, number, number, number];

/** One body in the projection (mirrors `documentStore.BodyMeta`). */
export interface BodyProjection {
  id: string;
  name: string;
  visible: boolean;
  /** User-authored body color. Absent = theme neutral body fill. */
  color?: Rgba;
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

/**
 * Where a REATTACH (H9) sends a sketch — the V1 target set of
 * `CadClient.reattachSketch`.
 *
 * World and datum only. A host-FACE target is deliberately absent (see the
 * `reattachSketch` docs): the frame of a face-hosted sketch is derived from the
 * face by the worker, so the frontend can neither author one nor safely take one
 * away.
 */
export type SketchAttachTarget =
  | { kind: "world"; plane: "XY" | "XZ" | "YZ" }
  | { kind: "datum"; datumId: string };

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
   * The document variable table, in declaration order (W5).
   *
   * Optional because only the real backend sends it — the mock lane keeps its
   * table beside the projection, as `geometrySource` is likewise omitted there.
   * It rides here so `upsertVariable`/`removeVariable` can return the same
   * projection every other mutating command does and be regen-correlated; the
   * documentStore does NOT hydrate from it (the Variables section owns the
   * table, and a projection is about the timeline).
   */
  variables?: DocumentVariable[];
  /**
   * Applied op count (timeline cursor): `features[0, appliedOps)` are applied,
   * `[appliedOps, totalOps)` are drafts beyond the rollback bar. Optional — the
   * backend always sends both; no frontend consumer wired yet (MODEL-HARDEN W0
   * seam for the legacy-draft recovery hint).
   */
  appliedOps?: number;
  /** Total op count (timeline length). See {@link appliedOps}. */
  totalOps?: number;
  /**
   * Where the geometry the viewport is showing came from (backend-authoritative).
   *
   * - `"live"` — a regen has published; this IS the document's geometry.
   * - `"cached"` — no publish yet, but the container's LAST-SAVED meshes are being
   *   served so a reopened document paints immediately. Correct as of the last
   *   save, and possibly older than the timeline the tree shows — the UI must
   *   label it rather than let the user measure off it.
   * - `"none"` — nothing to paint yet.
   *
   * Optional: the mock lane omits it. The chip that renders `"cached"` lands in a
   * follow-up wave; the field exists so both sides of the seam agree today.
   */
  geometrySource?: "none" | "cached" | "live";
}

/** Every terminal a correlated regeneration can report. */
export type RegenTerminal = "published" | "noop" | "needsRepair" | "failed" | "timeout";

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
  /** Bounded structured kernel evidence; absent for legacy/clean terminals. */
  diagnostics?: OperationDiagnostic[];
  /**
   * Records whose POST-regen step state is Error, even when the regen itself
   * published (other steps' geometry). A commit correlated to this completion whose
   * own `recordId` appears here is a FAILURE (empty bodies + the step message) — it
   * must NOT correlate as success off the sibling `document-changed` (MODEL-HARDEN).
   * Present only when non-empty; the mock lane omits it.
   */
  failedSteps?: Array<{ recordId: string; message: string; diagnostics?: OperationDiagnostic[] }>;
  /**
   * Record ids this published regen left in `NeedsRepair`. Same per-record shape
   * as `failedSteps`, and present for the same reason — the sibling `needs-repair`
   * event carries the same facts but is emitted AFTER this one, so an awaiter that
   * settles here would always miss it. NeedsRepair is document state, never a
   * failure: this exists so a commit stops reporting itself as published, not so
   * it can be raised into an error. Present only when non-empty; the mock lane
   * omits it unless a scenario asks for it.
   */
  repairSteps?: string[];
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
  | "MirrorBody"
  /** Rigid placement of a body selection (SCHEMA §7.3; WP-B W0/W1). */
  | "TransformBody"
  /** Machined hole on a planar face (SCHEMA §7.3; WP-C T3). */
  | "Hole"
  /** Direct-modelling face offset (SCHEMA §7.3 `op.offsetFace`, 2026-08-06). */
  | "OffsetFace"
  /** Generated parametric gear body (SCHEMA §7.3 `Gear`; Gear Generator G1). */
  | "Gear"
  /**
   * Place a library component instance (spec §3.1; Component Library WP-1.5).
   * PREVIEW-ONLY here: the ghost session this op-type drives is always
   * cancelled, never committed through `endPreview` — the real commit goes
   * through the dedicated `CadClient.placeComponent` (library.rs re-verifies
   * the package revision there, which a generic preview commit would skip).
   */
  | "PlaceComponent";

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
  /**
   * CHAMFER-ONLY second leg (SCHEMA §7.3, 2026-08-03). Absent ⇒ equal-leg, and
   * the wire carries no such key. `radius` lands on the adjacent face with the
   * smaller resolved face ordinal (the worker's deterministic reference face),
   * `distance2` on the other. A `mode: "Fillet"` params object carrying it is
   * dropped at the marshalling seam and rejected by core.
   */
  distance2?: number;
  /**
   * CHAMFER-ONLY chamfer angle in DEGREES (SCHEMA §7.3). Presence picks the mode:
   * `angleDeg` ⇒ distance-angle, else `distance2` ⇒ two-distance, else equal-leg.
   * MUTUALLY EXCLUSIVE with `distance2` — core rejects a Chamfer carrying both by
   * name, so no marshalling seam may emit both. Strictly between 0 and 180.
   *
   * DEGREES end to end on this lane: the op-param convention (`RevolveParams.angleDeg`,
   * `Hole.csAngleDeg`) is degrees on the wire too, so nothing converts. `src/ipc/angleUnits.ts`
   * is the SKETCH seam (UI degrees ↔ wire radians) and does not apply here.
   */
  angleDeg?: number;
  /** TopoKeys (snapshot-scoped) or ElementIds; resolved through the ladder. */
  edgeIds: string[];
  chainTangentEdges?: boolean;
  /** Present only on freshly prepared records; absence preserves legacy seed-only execution. */
  tangentClosureVersion?: 1;
}

/** Standalone body-body boolean op params (SCHEMA §7.3 BooleanParams). */
export interface BooleanParams {
  operation: BooleanOperation;
  targetBodyId: string;
  toolBodyId: string;
}

/**
 * Shell op params (Rust `ShellParams`). `openFaces` are the removed faces
 * (ElementIds or snapshot TopoKeys, resolved through the ladder); new authoring
 * supplies the matching semantic refs through `OperationOp.inputs`, which the
 * command mapper persists as typed `faces`. `thickness` is the wall thickness.
 * `targetBodyId` is the shelled body.
 */
export interface ShellParams {
  thickness: number;
  openFaces: string[];
  targetBodyId?: string;
}

/**
 * Machined-hole profile (SCHEMA §7.3 `Hole.holeType`). LOWERCASE on the wire —
 * unlike the PascalCase mode enums, these are new-in-v2 values.
 */
export type HoleType = "simple" | "counterbore" | "countersink";

/**
 * Hole op params (SCHEMA §7.3 `Hole`, Rust `HoleParams`; WP-C T3).
 *
 * ALWAYS RAW MILLIMETRES. The standard-size tables (`tools/modelTools/holeStandards.ts`)
 * are a frontend convenience that FILL these numbers — they never travel: a
 * document must mean the same thing to a reader who has never heard of DIN 74.
 *
 * `point` is the world-space hole centre frozen at authoring; `face` is the typed
 * host-face ref the worker resolves through the ladder (and re-projects `point`
 * onto, with a 1e-3 mm fence). `depth: null` = through-all.
 *
 * The conditional blocks are REQUIRED by, and exclusive to, their own `holeType`:
 * a `simple` hole carrying `cbDiameter` is rejected by the Rust session, so the
 * chip cluster must CLEAR the other profile's fields when the type flips.
 */
export interface HoleParams {
  targetBodyId: string;
  face: SemanticRef;
  point: [number, number, number];
  holeType: HoleType;
  diameter: number;
  /** Blind depth in mm, or `null` = through-all. */
  depth: number | null;
  cbDiameter?: number | null;
  cbDepth?: number | null;
  csDiameter?: number | null;
  csAngleDeg?: number | null;
  /** Absent = legacy split-host replay; 2 = strict one-connected-solid result. */
  resultPolicyVersion?: 2;
}

/**
 * Which gear a {@link GearParams} block describes (SCHEMA §7.3 `Gear.recipe`).
 *
 * A closed union, never a free-form string: ADR-0002 keeps the modeling kernel
 * closed, so a recipe is a variant the core team adds alongside its §7.3 entry.
 */
export type GearRecipe = "involuteExternal";

/**
 * An explicit frozen placement frame for a gear on a datum / in world space.
 *
 * `xDir` fixes the gear's angular PHASE and is carried rather than derived:
 * phasing is what makes a pair of gears mesh, and a derived x-axis would
 * silently rotate the teeth whenever the axis changed.
 */
export interface GearFrame {
  origin: [number, number, number];
  axis: [number, number, number];
  xDir: [number, number, number];
}

/**
 * Where a generated gear sits (SCHEMA §7.3 `Gear.placement`).
 *
 * EXACTLY ONE of `face` / `frame` is non-null — both set, or neither, is
 * rejected by the Rust session AND independently by the worker. With a face,
 * `point` is the frozen world centre the worker re-projects onto the resolved
 * plane each regen (fenced at 1e-3 mm, Hole's semantics) and the gear's axis is
 * that face's INWARD normal; with a frame, `point` must equal `frame.origin`.
 */
export interface GearPlacement {
  face: SemanticRef | null;
  frame: GearFrame | null;
  point: [number, number, number];
}

/**
 * External involute gear parameters (SCHEMA §7.3 `Gear.involuteExternal`).
 *
 * ALWAYS RAW MILLIMETRES AND DEGREES. `clearance`, `head` and `shift` are
 * dimensionless COEFFICIENTS (multiples of module) per gear-design convention,
 * not lengths — a UI that appends "mm" to them is lying.
 *
 * `helixAngleDeg` and `doubleHelix` are present so the payload will not change
 * shape when helical gears land, but a non-zero / true value is refused as
 * `UNSUPPORTED` in this version. The chip must not offer them as live controls.
 */
export interface InvoluteExternalParams {
  teeth: number;
  module: number;
  height: number;
  pressureAngleDeg: number;
  shift: number;
  helixAngleDeg: number;
  doubleHelix: boolean;
  propertiesFromTool: boolean;
  undercut: boolean;
  backlash: number;
  clearance: number;
  head: number;
  /** Spline samples per curve segment — an accuracy knob that changes topology. */
  sampleCount: number;
  axleHole: boolean;
  axleHoleDiameter?: number | null;
  offsetHole: boolean;
  offsetHoleDiameter?: number | null;
  offsetHoleOffset?: number | null;
}

/**
 * Gear op params (SCHEMA §7.3 `Gear`, Rust `GearParams`; Gear Generator G1).
 *
 * The recipe blocks follow Hole's conditional-block contract: `recipe` selects
 * which block is non-null and **every inactive recipe key is spelled `null`**,
 * never omitted. The wire mapper enforces that on the way out, so the chip
 * cluster must CLEAR a block when the recipe flips rather than leaving a stale
 * one behind — a stale block is rejected by the Rust session.
 *
 * Lineage is a NewBody MINT (`body_<opId>`, decision D1): a gear has no target
 * body and modifies nothing.
 */
export interface GearParams {
  recipe: GearRecipe;
  placement: GearPlacement;
  involuteExternal?: InvoluteExternalParams | null;
}

/**
 * How an {@link OffsetFaceParams.distance} is READ (SCHEMA §7.3 `distanceType`).
 * PascalCase on the wire, like the mode enums above.
 *
 * `Offset` is the only type valid for a multi-face selection and the only one the
 * frontend lets a DRAG author — the three absolute forms are numeric input seeded
 * from `PrepareOffsetFace.currentDims` (a drag has no meaningful zero for them).
 */
export type OffsetDistanceType = "Offset" | "Total" | "Radius" | "Diameter";

/**
 * Face-offset op params (SCHEMA §7.3 `op.offsetFace`, Rust `OffsetFaceParams`).
 *
 * `faces` is the FULL FROZEN operative set — the user's picks PLUS the G1 tangent
 * closure `PrepareOffsetFace` computed at authoring time. The worker NEVER
 * re-expands it at regen, so the array a commit sends is the array every future
 * rebuild operates on. Typed refs (the Fillet `edges` / new Shell `faces` model):
 * each carries the anchor evidence the resolution ladder rebinds with.
 *
 * `oppositeFace` is `Total`-only and REQUIRED there — core validates the
 * conditional BOTH ways, so a stale opposite left behind by a distance-type
 * switch must be cleared, never carried.
 *
 * `targetBodyId` is MANDATORY: an in-place op always depends on its body.
 */
export interface OffsetFaceParams {
  faces: SemanticRef[];
  /** V2 user-picked design-face ids; a non-empty subset of the full closure. */
  primaryFaceIds?: string[];
  /**
   * `2` is the plain sweep, `3` the blend-aware reading of the SAME pair (a
   * fillet inside the closure is suppressed and rebuilt rather than swept).
   * Fresh authoring emits `3`; a stored `2` executes verbatim forever.
   */
  resultPolicyVersion?: 2 | 3;
  /** The USER's value, read per {@link OffsetFaceParams.distanceType}. Never clamped. */
  distance: number;
  distanceType: OffsetDistanceType;
  /** Authoring metadata after the closure is frozen (re-edit UX only). */
  chainTangentFaces: boolean;
  /** `Total` only — persisted at authoring and re-resolved verbatim each regen. */
  oppositeFace?: SemanticRef;
  targetBodyId: string;
}

// ── PrepareOffsetFace (SCHEMA §7.6) — the read-only authoring handshake ───────
//
// The first half of the OffsetFace freeze: the worker computes the G1 tangent
// closure, the Total opposite candidate and the seeds for the absolute distance
// types. It MINTS NOTHING — the response carries snapshot-scoped TopoKeys plus
// descriptor/anchor evidence only (Invariant 2: ElementIds are Rust's alone), so
// the caller promotes the evidence before authoring any params.

/** One picked face addressed for {@link CadClient.prepareOffsetFace}. */
export interface OffsetFacePick {
  bodyId?: string;
  /** Snapshot-scoped ordinal key. MUTUALLY EXCLUSIVE with `elementId`. */
  topoKey?: string;
  elementId?: string;
}

/** One face's snapshot-scoped evidence from `PrepareOffsetFace`. Never an identity. */
export interface OffsetFaceEvidence {
  topoKey: string;
  /** `true` for a face the USER picked, `false` for one the tangent closure added. */
  picked: boolean;
  anchor?: { worldPoint?: [number, number, number]; surfaceUv?: [number, number] };
  /** Opaque worker-owned descriptor, forwarded verbatim (never interpreted here). */
  descriptor?: unknown;
}

/** Measurable dimensions that SEED an absolute distance type. Keys are absent when
 *  the closure has no such reading (a planar face has no radius). */
export interface OffsetCurrentDims {
  radius?: number;
  thickness?: number;
}

/**
 * A `PrepareOffsetFace` REFUSAL — an ANSWER (`ok:true`), not an error. The tool
 * needs to explain *why* it cannot offset, and an error would erase the reason.
 */
export interface OffsetFaceRefusal {
  /** `crossBody` | `unsupportedSurface` | `chainMismatch` | `noUniqueOpposite` |
   *  `notPlanar` | `chainOnTotal` | `notCylindrical` | `mixedAxis` | `nonManifold`. */
  code: string;
  message: string;
  /** The offending TopoKeys, when the code has any to name. */
  faces: string[];
}

/** `prepareOffsetFace` request (SCHEMA §7.6 `req.args`). */
export interface PrepareOffsetFaceRequest {
  /**
   * The head the closure is computed against. OPTIONAL because only the transport
   * knows it: `tauriClient` owns `currentSnapshotId` (the same value it fences
   * `promoteSelection` with) and fills it in, and the mock has no snapshots at
   * all. A caller that passes one pins the fence explicitly.
   */
  snapshotId?: number;
  pickedFaces: OffsetFacePick[];
  chainTangentFaces: boolean;
  distanceType: OffsetDistanceType;
}

/** `prepareOffsetFace` result (SCHEMA §7.6 `result`; Rust `PrepareOffsetFaceDto`). */
export interface PrepareOffsetFaceResult {
  /** Echo of the head the worker answered against. */
  snapshotId: number;
  /** The single body every picked face belongs to. Empty on a refusal. */
  targetBodyId: string;
  /** The full operative closure (picks ∪ tangent chain). Empty on a refusal. */
  faces: OffsetFaceEvidence[];
  /** The `Total` opposite candidate; absent for every other type. */
  oppositeFace?: OffsetFaceEvidence;
  currentDims: OffsetCurrentDims;
  /** Present ⇒ the handshake REFUSED and `faces` is not a usable closure. */
  refusal?: OffsetFaceRefusal | null;
}

// ── PrepareEdgeOp — shared Fillet/Chamfer authoring handshake ────────────────

export interface EdgeOpPick {
  bodyId?: string;
  /** Snapshot-scoped ordinal key. MUTUALLY EXCLUSIVE with `elementId`. */
  topoKey?: string;
  elementId?: string;
}

export interface EdgeOpEvidence {
  topoKey: string;
  picked: boolean;
  /** Rust-minted persistent identity; present on every accepted edge. */
  elementId: string;
  bodyId: string;
  kind: "edge";
  anchor?: { worldPoint?: [number, number, number]; surfaceUv?: [number, number] };
  /** Opaque worker-owned descriptor. */
  descriptor?: unknown;
}

export interface EdgeOpRefusal {
  code: string;
  message: string;
  edges: string[];
}

export interface PrepareEdgeOpRequest {
  snapshotId?: number;
  mode: "Fillet" | "Chamfer";
  pickedEdges: EdgeOpPick[];
  chainTangentEdges: boolean;
}

export interface PrepareEdgeOpResult {
  snapshotId: number;
  targetBodyId: string;
  edges: EdgeOpEvidence[];
  refusal?: EdgeOpRefusal | null;
}

// ── AnalyzeEdgeOpRange — what radius will this edge ACTUALLY take? ───────────

/**
 * What a consumer may enforce, per SCHEMA §7.6. A TOTAL decision tree, not
 * advice: every answer lands on exactly one rung, and the rung says what the
 * clamp is allowed to do.
 *
 * - `none` — nothing was proven. Do not clamp.
 * - `nonMonotonic` — an ISLAND was observed. Honour `feasibleIntervals`; a
 *   single ceiling would licence a value measured to fail.
 * - `lowerOnly` — a floor, no proven ceiling. Enforce the floor only.
 * - `bracketed` — a complete monotonic bracket. A hard ceiling is safe.
 * - `coarse` — the search stopped early. Cap at `bestKnownMax`, NEVER at
 *   `provenUpperBound`, which may sit far above the real frontier.
 */
export type EdgeOpRangeConfidence =
  | "none"
  | "nonMonotonic"
  | "lowerOnly"
  | "bracketed"
  | "coarse";

/** Why the search ended. All three are normal; only a cancel is an error. */
export type EdgeOpRangeStop = "converged" | "budgetExhausted" | "deadline";

/** A run of adjacent feasible probes. BOTH endpoints were actually built. */
export interface EdgeOpRangeInterval {
  lower: number;
  upper: number;
}

/** What the bounding refusal named. EMPTY when the kernel did not attribute it. */
export interface EdgeOpLimitingEntity {
  topoKey: string;
  kind: string;
}

export interface AnalyzeEdgeOpRangeRequest {
  snapshotId?: number;
  mode: "Fillet" | "Chamfer";
  pickedEdges: EdgeOpPick[];
  chainTangentEdges: boolean;
  /** Optional mm window and build budget. Every field is a hint the worker clamps. */
  range?: { min?: number; max?: number; probeBudget?: number };
}

export interface AnalyzeEdgeOpRangeResult {
  snapshotId: number;
  mode: "Fillet" | "Chamfer";
  targetBodyId: string;
  /** Ordinal-ordered TopoKeys of the analysed closure. */
  edges: string[];
  /** The effective mm window this answer covers. */
  searchedRange: { min: number; max: number };
  /**
   * `null` means UNPROVEN, and is never a number — 0 is a radius a user could
   * act on, and none of these were measured unless they are non-null.
   */
  lowerBound: number | null;
  bestKnownMax: number | null;
  /** Smallest infeasible probe ABOVE `bestKnownMax` — not the smallest refusal. */
  provenUpperBound: number | null;
  feasibleIntervals: EdgeOpRangeInterval[];
  intervalsTruncated: boolean;
  limitingEntities: EdgeOpLimitingEntity[];
  confidence: EdgeOpRangeConfidence;
  monotonicObserved: boolean;
  /** DIAGNOSTIC, non-contractual. */
  probesUsed: number;
  budgetExhausted: boolean;
  stoppedReason: EdgeOpRangeStop;
  /** Present ⇒ the closure refused; zero probes ran and every bound is null. */
  refusal?: EdgeOpRefusal | null;
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
  /** Absent preserves legacy one-body publication; 2 enables versioned lineage. */
  resultPolicyVersion?: 2;
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
  /** Absent preserves legacy one-body publication; 2 enables versioned lineage. */
  resultPolicyVersion?: 2;
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
 * The rotation half of a `TransformBody` (Rust `TransformRotation`). `center` is
 * the FROZEN pivot — the targets' combined bbox centre at first authoring, never
 * re-derived, so repeated re-edits recompose against the same point and cannot
 * drift. `axis` need not be unit length but MUST be non-zero when `angleDeg ≠ 0`.
 */
export interface TransformRotationParams {
  center: [number, number, number];
  axis: [number, number, number];
  angleDeg: number;
}

/**
 * Rigid-placement op params (SCHEMA §7.3 `TransformBody`, Rust
 * `TransformBodyParams`). NORMATIVE evaluation order is
 * `X' = T ∘ R(center, axis, angleDeg) · X` — rotate about the frozen pivot
 * first, THEN translate.
 *
 * `targets` mirrors `inputs[]` in order (the ordinal is load-bearing for
 * `copy: true`, which mints `body_<opId>:<k>` at the target's index). A zero
 * motion is legal and a geometric no-op.
 */
export interface TransformBodyParams {
  targets: string[];
  translate: [number, number, number];
  rotate: TransformRotationParams;
  copy: boolean;
}

/**
 * `PlaceComponent` GHOST-preview params (Rust `PlaceComponentParams` — spec
 * §3.1; Component Library WP-1.5). Narrower than the Rust struct: no `mate`
 * (the gesture records its own on commit; regen-time re-seating is the
 * worker's job since WP-3.1). Free-parameter overrides ride inside
 * `source.params` — where the worker reads them — rather than as a second
 * top-level copy the ghost would have to keep in sync.
 */
export interface PlaceComponentParams {
  componentId: string;
  componentVersion: string;
  componentRevision: string;
  /**
   * Where the ghost's geometry comes from — the SAME discriminant the committed
   * record carries (spec §2.1). Before WP-3.2 the preview hardcoded a generator
   * source, which meant arming a non-generator component previewed a
   * DIFFERENT body than committing it would produce (the generator stub's
   * ISO 4762 screw). A preview that can disagree with its own commit is worse
   * than no preview.
   */
  source: PlaceComponentSource;
  translate: [number, number, number];
  rotate: TransformRotationParams;
}

/**
 * Ghost-preview geometry source, mirroring Rust `ComponentSourceRef`.
 *
 * `params` on the generator variant carries the free-parameter overrides the
 * gesture chose (auto-size on a hole rim, WP-A3). It is what the worker's own
 * table lookup reads, so a ghost that omits it previews the generator's
 * DEFAULT size while the commit places the chosen one — the preview/commit
 * disagreement WP-3.2 already had to fix once for `source` itself.
 */
export type PlaceComponentSource =
  | {
      kind: "generator";
      generatorId: string;
      generatorVersion: number;
      params?: Record<string, ComponentParamValue>;
    }
  | ({ kind: "embedded" } & ComponentGeometry)
  | ({ kind: "document"; documentSha256: string } & ComponentGeometry);

/**
 * The staged geometry pointer `stageComponentGeometry` returns (Rust
 * `ComponentGeometryDto`): a blob-backed component's baked solid, already cached
 * in the open document and materialized for the worker.
 */
export interface ComponentGeometry {
  sha256: string;
  /** `step` | `brep` | `xbf`. */
  codec: string;
  /** Binary format pin; absent for `step`. */
  brepFormat?: number;
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
      /** Absent retains the persisted v1 profile lookup semantics. */
      regionIdentityVersion?: number;
      inputs?: SemanticRef[];
      params: ExtrudeParams;
    }
  | {
      opType: "Revolve";
      opId?: string;
      featureId?: string;
      sketchId: string;
      regionId: string;
      /** Absent retains the persisted v1 profile lookup semantics. */
      regionIdentityVersion?: number;
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
    }
  | {
      opType: "TransformBody";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: TransformBodyParams;
    }
  | {
      opType: "Hole";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: HoleParams;
    }
  | {
      opType: "OffsetFace";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: OffsetFaceParams;
    }
  | {
      opType: "Gear";
      opId?: string;
      featureId?: string;
      inputs?: SemanticRef[];
      params: GearParams;
    }
  /**
   * Ghost-preview only (see {@link PlaceComponentParams}). `featureId`/`inputs`
   * are structurally present (every `OperationOp` arm carries them, and code
   * widens across the union) but never populated — this session is never
   * re-edited or committed through the preview lane.
   */
  | {
      opType: "PlaceComponent";
      opId?: string;
      featureId?: undefined;
      inputs?: undefined;
      params: PlaceComponentParams;
    };

/**
 * `opType` of a STEP-import history row (STEP-IMPORT WP-A).
 *
 * DELIBERATELY NOT in `OpType`: that union is the set of ops the FRONTEND authors
 * and previews (it feeds `OperationOp` / `tauriCommandMap` / the preview builders),
 * and an import has no frontend-authored params at all — Rust owns the file dialog
 * and the record. It only ever arrives on `FeatureRecord.opType` (a plain string),
 * so it is exported as a shared literal instead: the icon map, the re-edit router
 * and the mock fabrication all key off THIS rather than three loose spellings.
 *
 * The projection buckets it under `kind: "boolean"` (interim — the backend has no
 * dedicated import bucket yet), which is exactly why the icon must be opType-keyed.
 */
export const IMPORT_STEP_OP_TYPE = "ImportStep";

/**
 * Domain of a feature's primary editable dimension ({@link FeatureRecord.primaryValue}).
 * `"length"`/`"diameter"` are millimetres, `"angle"` is DEGREES — the document
 * domain in both cases, never a display unit.
 */
export type FeaturePrimaryKind = "length" | "angle" | "diameter";

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
  /**
   * The row's mono value text, MILLIMETRE-FIXED and always produced whatever the
   * display-unit preference — three re-edit seeds parse it back as mm
   * (`radiusFromValueText` / `thicknessFromValueText` / `countFromValueText`).
   * {@link FeatureRecord.primaryValue} is what a UNIT-AWARE editor renders.
   */
  valueText: string;
  /**
   * The ONE dimension the history row edits in place (H3), in the DOCUMENT domain
   * (mm for a length/diameter, degrees for an angle). Absent for a feature with no
   * single primary dimension (Sketch/Boolean/Move/patterns/mirror/import), whose
   * row stays read-only. Optional so an older backend payload still parses.
   */
  primaryValue?: number;
  /** Domain of {@link FeatureRecord.primaryValue}; absent whenever it is. */
  primaryValueKind?: FeaturePrimaryKind;
  /**
   * The document VARIABLE driving {@link FeatureRecord.primaryValue} (WP-VE.2),
   * when the stored `Scalar` carries an `expr`. Absent ⇒ the number is a literal.
   *
   * BACKEND-AUTHORITATIVE and the only thing the row may render `=name` from —
   * the backend mints it from the same `Scalar` the number came from, so the UI
   * can never show a binding the document does not hold.
   */
  primaryExpr?: string;
  status: "ok" | "dirty" | "error" | "needsRepair";
  /** Worker failure reason for an errored step (`status === "error"`), surfaced as
   *  the HistoryList row tooltip (MODEL-HARDEN W0.5). Absent for any other status. */
  statusMessage?: string;
  /** Bounded structured diagnostics for this feature's latest terminal. */
  diagnostics?: OperationDiagnostic[];
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
  /** Structured evidence for `errorMessage`; separate from NeedsRepair evidence. */
  diagnostics?: OperationDiagnostic[];
  /**
   * The timeline CURSOR after the edit (applied op count), H7b.
   *
   * An edit result used to carry no cursor at all, so a rollback only became
   * visible when the next `projection-updated` landed — and never at all on the
   * mock lane. Both clients now read it off the same place the timeline came
   * from: the real one off the command's own projection (`applied_ops`), the mock
   * off its cursor. Absent ⇒ the caller falls back to `nextAppliedOps`.
   */
  appliedOps?: number;
  /** Timeline LENGTH after the edit — always `features.length`; sent for symmetry
   *  with {@link ApplyOperationResult.appliedOps} and cross-checked in tests. */
  totalOps?: number;
  /**
   * Classified transport terminal, populated by both clients. Optional only for
   * compatibility with legacy test fixtures while callers migrate from body-count
   * inference; no production result may omit it.
   */
  terminal?: RegenTerminal;
}

// ── Two-level preview (NEW_SPEC §15) ─────────────────────────────────────────

/**
 * Params a preview update carries (opType-specific; loosely typed for the wire).
 * One union per previewable opType — the `ipc/previewOps.ts` builders narrow it
 * back down and reject anything that does not belong to the op being previewed.
 *
 * This is an INTERSECTION, so a key two ops both declare gets the INTERSECTION of
 * their types, not the union: `resultPolicyVersion` is `2` on Hole and `2 | 3` on
 * OffsetFace, which would collapse to `2` and make a legal V3 offset draft
 * unassignable. Hole's declaration is omitted so the widest reading survives; both
 * builders re-narrow the value themselves, which is where the real check belongs.
 */
export type PreviewParams = Partial<ExtrudeParams> &
  Partial<RevolveParams> &
  Partial<FilletParams> &
  Partial<ShellParams> &
  Partial<BooleanParams> &
  Partial<Omit<HoleParams, "resultPolicyVersion">> &
  Partial<OffsetFaceParams> &
  Partial<PlaceComponentParams> & { [k: string]: unknown };

/** `beginPreview` draft — the base op the drag will refine. */
export interface PreviewDraft {
  /** Stable RecordId reused by exact preview and commit. Minted when absent. */
  opId?: string;
  opType: OpType;
  sketchId?: string;
  regionId?: string;
  /** Forwarded into a new `SketchRegionRef`; absent preserves v1 storage. */
  regionIdentityVersion?: number;
  /**
   * Typed op inputs, carried verbatim into the session. Load-bearing for
   * Fillet/Chamfer and Shell — `wireOperation` drops an op's top-level inputs,
   * then the param mappers synthesize typed `params.edges` / `params.faces` from
   * these in lockstep with their bare-id arrays. Boolean bodies ride
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
  /** Kernel failure evidence; never overloaded with NeedsRepair evidence. */
  diagnostics?: OperationDiagnostic[];
}

/** Additive worker operation diagnostic (SCHEMA §7.2/§8). */
export interface OperationDiagnostic {
  severity: "info" | "warning" | "error";
  code: string;
  message: string;
  stage?: string;
  /**
   * Fine-grained machine-readable publication-refusal reason (SCHEMA §7.2
   * `diagnostics[].reasonCode`, SCREAMING_SNAKE). A sibling of `code`, which
   * stays the §8 taxonomy value — route on this, never on `message`.
   */
  reasonCode?: string;
  evidence?: Record<string, unknown>;
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

/**
 * One module's slice of the document (ADR-0004).
 *
 * `payload` is opaque to everything but the owning module: it crosses the wire
 * as arbitrary JSON and is never inspected on the way (ADR-0005).
 */
export interface ModuleState {
  moduleId: string;
  /** The schema version the OWNING module wrote this payload at. */
  schemaVersion: number;
  payload: unknown;
}

/** What a document records about a module it carries state for. */
export interface DocumentModule {
  moduleId: string;
  schemaVersion: number;
}

/**
 * One document variable (WP-VE.2 — the authoring surface over the table WP-VE.1
 * made drive regen).
 *
 * `value` is the LAST EVALUATED number: for a plain variable that is simply what
 * the user typed; for one that is itself `expr`-driven it is the cached result.
 * V1 refuses to resolve a chained expression at regen, so `expr` is surfaced
 * rather than hidden — a broken binding must be visible where it was authored.
 */
export interface DocumentVariable {
  id: string;
  name: string;
  value: number;
  /** Present only for a variable whose own value is expression-driven. */
  expr?: string;
}

/** The V1 variable-name grammar, shared with `regen::variables::is_bare_name`. */
export const VARIABLE_NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

/**
 * What `upsertVariable`/`removeVariable` resolve with (W5 result truth).
 *
 * BOTH truths, deliberately: `variables` is the table the document actually
 * holds after the write — the write is never rolled back — and the inherited
 * `terminal` is the verdict of the regen that write scheduled. A variable edit
 * dirties `[0, len)`, so a failing step anywhere IS this edit's failure; the
 * caller reports it and leaves the saved value on screen.
 */
export interface VariableEditResult extends ApplyOperationResult {
  variables: DocumentVariable[];
}
