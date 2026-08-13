/*
 * CadClient — the frontend's single seam to the backend.
 *
 * Only the surface the START SCREEN needs today is declared. Later WPs EXTEND
 * this interface (document mutations, mesh fetch, picking, solver gestures, …);
 * keep additions append-only so the mock + real clients evolve together.
 *
 * Two implementations satisfy CadClient:
 *   - mockClient  (this WP) — in-memory, drives the whole UI with no backend.
 *   - tauri client (F-WP8) — real IPC via tauri commands + the C++ worker.
 */
import type {
  ApplyOperationResult,
  BeginGestureResult,
  ClassifyResult,
  ComponentParamValue,
  ComponentUpgrade,
  NewComponentSpec,
  ProjectTemplate,
  ReplaceComponentReport,
  DocumentChange,
  DocumentModule,
  DocumentProjectionWire,
  DocumentSnapshot,
  DragSolveResult,
  ElementInfo,
  LibraryComponent,
  MassProperties,
  ModuleState,
  EnterSketchTarget,
  FeatureDependencies,
  FinishSketchResult,
  GestureTarget,
  Lod,
  NeedsRepairEvent,
  OperationOp,
  PlaceComponentSource,
  ReindexReport,
  PreviewDraft,
  PreviewParams,
  PreviewResult,
  PreviewSession,
  PrepareEdgeOpRequest,
  PrepareEdgeOpResult,
  PrepareOffsetFaceRequest,
  PrepareOffsetFaceResult,
  PromotedElement,
  SketchPlane,
  PromotePick,
  RecentProject,
  RecoveryInfo,
  SaveOutcome,
  ResolveRefRequest,
  ResolveRefResult,
  SketchAttachTarget,
  SketchConstraint,
  SketchConstraintType,
  SketchEntity,
  SketchSession,
  SketchUpsertResult,
  TransformRotationParams,
  Unsubscribe,
  WorkerStatus,
} from "./types";
import type { WireEditCommand } from "./tauriCommandMap";
import { mockClient } from "./mockClient";
import { createTauriClient } from "./tauriClient";

export interface CadClient {
  /** Recent projects for the start screen list. */
  listRecents(): Promise<RecentProject[]>;
  /**
   * Rename a recent project's `.onecad` file on disk (start-screen card menu).
   * Rust owns the fs rename + the `recents.json` update; rejects on an empty
   * name or a collision with an existing file. Caller refetches `listRecents`
   * afterward — this does not return the updated entry.
   */
  renameRecentProject(path: string, newName: string): Promise<void>;
  /**
   * Move a recent project's `.onecad` file to the OS trash (start-screen card
   * menu) — recoverable, not a permanent delete. Rust owns the trash move + the
   * `recents.json` removal.
   */
  deleteRecentProject(path: string): Promise<void>;
  /**
   * Reveal a recent project's `.onecad` file in the OS file manager (Finder /
   * Explorer / platform equivalent), selected where the platform supports it.
   */
  revealInFileManager(path: string): Promise<void>;
  /** Create a blank document and open it. */
  newDocument(): Promise<DocumentSnapshot>;
  /** Open an existing .onecad project at `path`. */
  openDocument(path: string): Promise<DocumentSnapshot>;
  /** Import a STEP file at `path` into a new document. */
  importStep(path: string): Promise<DocumentSnapshot>;
  /**
   * Import another `.onecad` project. If a document is open the source timeline
   * is appended to it; otherwise a fresh document is created. Resolves `null`
   * when the dialog is cancelled.
   */
  importProject(): Promise<DocumentSnapshot | null>;
  /** Pick a STEP or OneCAD file for the unified start-screen import button. */
  importFileDialog(): Promise<string | null>;
  /**
   * Read one module's slice of the open document. `null` when the module has
   * none. The payload is returned untouched — the caller is its only reader.
   */
  getModuleState(moduleId: string): Promise<ModuleState | null>;
  /**
   * Write (`state`) or clear (`null`) one module's slice. Goes through the same
   * transaction path as a user edit, so it is undoable.
   */
  setModuleState(moduleId: string, state: { schemaVersion: number; payload: unknown } | null): Promise<void>;
  /**
   * Every module the open document carries state for. Lets the app report a
   * missing addon instead of silently ignoring its data.
   */
  listDocumentModules(): Promise<DocumentModule[]>;
  /**
   * Close the open document, dropping the runtime + caches and returning to the
   * start screen. The backend emits an empty projection so the editor clears.
   * Safe to call when no document is open (no-op).
   */
  closeDocument(): Promise<void>;
  /**
   * Show a native file-open dialog and resolve to the chosen path (or null if
   * cancelled). Rust owns the real dialog (tauri-plugin-dialog Rust API — the
   * webview gets zero fs/dialog capability); the mock returns a fake path.
   */
  openFileDialog(): Promise<string | null>;

  /**
   * Save the open document. `path` `undefined` reuses the last save path; a
   * never-saved document then rejects (io error) and the caller falls back to
   * Save As. Rust owns the filesystem write.
   *
   * `previewPng` is an OPTIONAL viewport thumbnail (a `data:image/png;base64,…`
   * URL from `ViewportEngine.captureThumbnail()`; bare base64 is accepted too),
   * stored as the container's `preview.png` and served back by `listRecents`.
   * It is decoration, never a precondition: absent, malformed, or oversized, the
   * backend drops it with a warning and the SAVE STILL SUCCEEDS. Callers pass it
   * best-effort and must never fail a save because capture failed.
   */
  saveDocument(path?: string, previewPng?: string | null): Promise<SaveOutcome>;
  /**
   * Save As: show a native save dialog (`.onecad`), save to the chosen path, and
   * return the authoritative outcome — or null if the dialog was cancelled.
   *
   * Takes `previewPng` for the same reason `saveDocument` does, and it matters
   * MORE here: a never-saved document's first save is always a Save As, so
   * without it a brand-new project would land in recents with no thumbnail.
   */
  saveDocumentAs(previewPng?: string | null): Promise<SaveOutcome | null>;
  /**
   * Export every body at head to a STEP file. Rust owns the `.step` save dialog +
   * the worker ExportStep verb; resolves to the written path, or null on cancel.
   */
  exportStep(): Promise<string | null>;
  /**
   * Export every body at head to a binary STL file. Rust owns the `.stl` save
   * dialog + the worker ExportStl verb; resolves to the written path, or null on
   * cancel.
   */
  exportStl(): Promise<string | null>;
  /**
   * Export every body at head to an ASCII OBJ file. Rust owns the `.obj` save
   * dialog + the worker ExportObj verb; resolves to the written path, or null on
   * cancel.
   */
  exportObj(): Promise<string | null>;

  /**
   * Subscribe to worker-lifecycle `worker-status` events (starting / ready /
   * restarting / failed). The real event arrives from the C++ sidecar supervisor;
   * the mock never emits (returns a no-op unsubscribe).
   */
  onWorkerStatus(cb: (status: WorkerStatus) => void): Unsubscribe;

  /**
   * Fetch a body's MESH1 blob at `lod` (pull model). The real client returns a
   * single-body blob verbatim from the Rust MeshCache via `tauri::ipc::Response`
   * (zero-copy ArrayBuffer). The mock synthesizes the exact bytes locally.
   */
  getBodyMesh(bodyId: string, lod: Lod): Promise<ArrayBuffer>;

  /**
   * Subscribe to backend `document-changed` events. Fires with the changed +
   * removed bodies so the viewport can fetch/swap/drop meshes. Returns an
   * unsubscribe. The real event arrives from the worker (F-WP8); the mock is an
   * in-process emitter.
   */
  onDocumentChanged(cb: (change: DocumentChange) => void): Unsubscribe;

  // ── Sketch solver lane (SCHEMA §7.4) ──────────────────────────────────────
  // The real client routes these to the worker's PlaneGCS actor; the mock runs
  // an in-memory naive "solver" so the full sketch UI works with no backend.

  /**
   * Open a sketch for editing. `target` is an existing sketch id or a request
   * for a fresh sketch on a named plane. Returns the authoritative sketch
   * (plane basis + entities + constraints + dof/status).
   */
  enterSketch(target: EnterSketchTarget): Promise<SketchSession>;

  /**
   * Upsert the authoritative sketch (append/replace entities + constraints) and
   * re-solve. Returns the new dof/status and any CHANGED point coordinates.
   */
  sketchUpsert(
    sketchId: string,
    entities: SketchEntity[],
    constraints: SketchConstraint[],
  ): Promise<SketchUpsertResult>;

  /**
   * Set ONE dimensional constraint's value on a sketch, WITHOUT opening an edit
   * session — the history panel's sketch-dimension quick path (H3b).
   *
   * Separate from `sketchUpsert` because that verb diffs a whole entity+constraint
   * array against the id-map and requires the sketch to have been ENTERED; this one
   * addresses a single constraint on a sketch the user has only SELECTED, which is
   * the only thing the inspector can offer for a past sketch feature.
   *
   * `type` is required (not derivable from the id) because it decides the wire
   * domain: an `Angle` is DEGREES here and RADIANS on the wire, and that conversion
   * belongs to `ipc/angleUnits` — the one seam all three marshalling sites share.
   * Every other dimensional kind is millimetres in both domains.
   *
   * Downstream regen is the BACKEND's job: changing a sketch dirties the consuming
   * features, so a caller neither replays the timeline nor hydrates from a result.
   */
  setSketchDimension(
    sketchId: string,
    constraintId: string,
    type: SketchConstraintType,
    value: number,
  ): Promise<SketchUpsertResult>;

  /** Compute the closed profile regions for a sketch (extrude/revolve input). */
  finishSketch(sketchId: string): Promise<FinishSketchResult>;

  /**
   * Recompute closed regions without ending/squashing an edit session. This is
   * the model-mode/static-layer read and works for persisted unopened sketches.
   */
  getSketchRegions(sketchId: string): Promise<FinishSketchResult>;

  /**
   * Read a sketch's authoritative geometry WITHOUT opening an edit session — a
   * pure read for the always-visible sketch layer (no session opened, no cleanup).
   * Same wire shape as `enterSketch` (plane + entities + constraints + dof/status).
   * An unknown sketch id rejects.
   */
  getSketch(sketchId: string): Promise<SketchSession>;

  /** Discard the in-flight sketch edit session (no geometry change). */
  cancelSketch(sketchId: string): Promise<void>;

  /**
   * Remove a sketch feature from the document (a `DeleteSketch` EditCommand).
   * This is a COMPENSATION / cleanup verb, not part of normal editing: it undoes a
   * just-committed AddSketch whose follow-up `enter_sketch` rejected (an orphaned
   * empty sketch), or the fresh sketch a plane-pick created when the user cancelled
   * during the deferred enter. The real client routes to `apply_edit_command`; the
   * mock drops the store row + the local solver-lane session.
   */
  deleteSketch(sketchId: string): Promise<void>;

  /**
   * REATTACH (H9): move a sketch onto a different host plane
   * (`UpdateSketchAttachment`). The sketch's 2D entity coordinates are kept and
   * REINTERPRETED in the new basis — the standard CAD reattach semantics — and
   * everything downstream regenerates (`RegenHint::ToEnd`).
   *
   * V1 targets are named WORLD planes and DATUM planes only. A host-FACE target
   * is deliberately absent: the frame of a face-hosted sketch is derived from the
   * face by the worker and its projected boundary is expressed in that frame, so
   * an attachment assembled here could only ever produce an unprojected
   * (`projectedBoundaryVersion: 0`) host-face sketch — the seam SKETCH-ON-FACE W2
   * closed. Reattaching a face-hosted sketch AWAY from its face is likewise not
   * offered in V1: its projected boundary entities would be silently
   * reinterpreted under the new basis with nothing to re-project them.
   *
   * The basis is BACKEND-authoritative for a datum (the core re-stamps the
   * datum's resolved frame over whatever is sent).
   */
  reattachSketch(sketchId: string, target: SketchAttachTarget): Promise<ApplyOperationResult>;

  // ── Sketch drag gesture (SCHEMA §7.4) ──────────────────────────────────────
  // A point drag: beginGesture → many solveDrag (latest-wins) → endGesture (ONE
  // undo step). The real client routes to the worker's gesture verbs; the mock
  // runs a local identity solve. No frontend caller yet (SketchController point
  // drag lands with M2/M4); wired + tested here so the seam is real.

  /**
   * Open a drag gesture on a sketch point (`dragPointId` = a point entity id).
   *
   * `target` is OPTIONAL and additive (SCHEMA §7.4): absent ⇒ the plain POINT
   * drag on `dragPointId`, byte-identical to the request before the gesture kinds
   * existed. A non-`point` target (`arcEnd`/`radius`/`entityBody`) names what the
   * pointer really grabbed, and `dragPointId` is then only the fallback handle.
   */
  beginGesture(
    sketchId: string,
    dragPointId: string,
    target?: GestureTarget,
  ): Promise<BeginGestureResult>;

  /**
   * One incremental drag solve to `target` (fire-and-forget, latest-wins). Fire
   * without awaiting serially; each resolves with the fresh preview positions, or
   * `null` when the response was stale/superseded (dropped client-side by `seq`).
   */
  solveDrag(target: [number, number]): Promise<DragSolveResult | null>;

  /** Pointer-up: final exact solve committed as ONE undo command. */
  endGesture(finalTarget?: [number, number]): Promise<SketchUpsertResult>;

  // ── Element identity (SCHEMA §7.5) — pick → promote ────────────────────────

  /**
   * Promote snapshot-scoped TopoKey picks on a body to persistent, Rust-minted
   * `ElementId`s. The real client routes to `AcquireElementIds`; the mock mints
   * deterministic ids. Promoted ids flow back into the selection refs.
   */
  promoteSelection(
    bodyId: string,
    picks: PromotePick[],
    /** Explicit candidate provenance; omitted only for a fresh live pick. */
    snapshotId?: number,
  ): Promise<PromotedElement[]>;

  /**
   * Resolve a picked planar FACE to the sketch plane a sketch on it would freeze
   * (MODEL-OPS W2). REJECTS a non-planar face rather than approximating one —
   * silently sketching on a made-up plane is the "silent wrong bind" class this
   * codebase refuses. The mock derives an equivalent frame locally.
   *
   * `topoKey` is forwarded alongside `elementId` when the caller has it: the
   * backend walks the same two-rung ladder `elementInfo` does (topoKey first,
   * then elementId) because a just-promoted, not-yet-consumed ElementId is not
   * yet in the worker's on-demand element-map partition — see `api::element_info`.
   */
  faceSketchPlane(bodyId: string, elementId: string, topoKey?: string): Promise<SketchPlane>;

  /**
   * Read one element's geometric evidence — the MEASURE tool's only backend
   * call (W2-B). A pure read: nothing is minted, nothing enters history, so
   * measuring can never appear in the undo stack.
   *
   * Resolves `null` when the element is not present in the current snapshot (a
   * stale pick after an edit); the caller drops the pick rather than showing a
   * fabricated zero. Pass BOTH handles when the caller has them: the backend
   * walks a two-rung ladder (topoKey, then elementId) because neither alone
   * answers every case — see `api::element_info`.
   */
  elementInfo(
    bodyId: string,
    elementId: string,
    topoKey?: string,
  ): Promise<ElementInfo | null>;

  /**
   * One body's exact mass properties — volume, surface area, centroid and the
   * principal inertia frame (WP-C1; SCHEMA §7.5 `QueryMassProperties`).
   *
   * A pure read, like `elementInfo`: nothing is minted, nothing enters history.
   *
   * No snapshot argument, unlike `elementInfo`: a BodyId is durable across
   * snapshots while a TopoKey is not, so the answer is always "this body as the
   * head has it now" — which is what a live measurement panel wants.
   *
   * REJECTS an unknown body rather than resolving `null`. A stale face pick is
   * an ordinary outcome worth absorbing; a missing body has no partial reading
   * to fall back to, and an empty answer would be indistinguishable from a body
   * that genuinely encloses no volume.
   */
  massProperties(bodyId: string): Promise<MassProperties>;

  /**
   * Surface/curve classification of a picked face or edge, plus a seatable
   * frame — the Component Library placement solver's hover query (SCHEMA §7.5
   * `ClassifyElement`, WP-0.1).
   *
   * A pure read like `elementInfo`, and always the current head (no snapshot
   * argument) since this is re-issued on every hover frame during a live
   * drag, not tied to a frozen pick. Resolves `null` when the target isn't
   * classifiable geometry — an ordinary outcome for a continuously-polled
   * query, never an error. Same two-rung addressing as `elementInfo`
   * (topoKey tried first, elementId is the fallback).
   */
  classifyElement(
    bodyId: string,
    elementId: string,
    topoKey?: string,
  ): Promise<ClassifyResult | null>;

  // ── Component Library (WP-1.3) ──────────────────────────────────────────

  /**
   * Lists the CURRENTLY PERSISTED library index — does not reindex (that's
   * `reindexLibrary`, a separate explicit action). Empty for a library that
   * has never been indexed, never an error.
   */
  listLibraryComponents(): Promise<LibraryComponent[]>;

  /** Rebuilds the library index from packages on disk (spec §4 `reindex`). */
  reindexLibrary(): Promise<ReindexReport>;

  /**
   * Resolves a component's geometry `source` for the placement GHOST, staging
   * its baked bytes into the open document on the way (Component Library
   * WP-3.2). Authors nothing.
   *
   * Call this when ARMING a placement, and put the result in the draft params:
   * the ghost previews through a candidate `PlaceComponent` op that lowers its
   * `source` exactly as a committed record does, so a blob-backed component
   * (`embedded` / `document`) needs its bytes materialized for the worker
   * BEFORE the first preview, and the preview must carry the real source rather
   * than a hardcoded generator one.
   *
   * Idempotent. A `generator` component stages nothing but still answers, so
   * there is exactly one arming path.
   */
  resolveComponentSource(
    componentId: string,
    componentVersion: string,
  ): Promise<PlaceComponentSource>;

  /**
   * Instantiates a library component as a placed instance (spec §3.1
   * `PlaceComponent`). `translate` + `rotate` are the placement solver's
   * (`placementSolver.ts`) computed candidate transform, already resolved
   * client-side from a classify+attachment match.
   *
   * `params` carries the free-parameter overrides the gesture chose — today
   * auto-size on a hole rim (WP-A3), which must reach the COMMIT and not only
   * the ghost. Keys are validated against the component's own signature
   * backend-side (`role = "free"` only); a non-free key is refused, never
   * silently dropped.
   *
   * No `mate` is sent from here: the gesture's own transform is frozen into
   * `placement`, and regen-time re-seating of a recorded mate is the worker's
   * job (WP-3.1). Geometry arrives via the usual
   * `onProjectionUpdated`/`onDocumentChanged` event stream, same as every
   * other mutating command — this resolves once the edit is APPLIED, not
   * once it has regenerated.
   */
  placeComponent(
    componentId: string,
    componentVersion: string,
    translate: [number, number, number],
    rotate?: TransformRotationParams,
    params?: Record<string, ComponentParamValue>,
  ): Promise<void>;

  /**
   * Every project template in the library (spec §8). An unreadable library is
   * an empty list, never an error — the start screen still has to render.
   */
  listTemplates(): Promise<ProjectTemplate[]>;

  /**
   * Freezes the OPEN document as a starter template (spec §8). Nothing about
   * the open document changes — it keeps its path, title and dirty flag,
   * because saving a template is not saving your work.
   */
  saveAsTemplate(
    id: string,
    name: string,
    description?: string,
    previewPng?: string | null,
  ): Promise<ProjectTemplate>;

  /**
   * Starts a NEW untitled document from a template (spec §8). The copy is
   * detached from the template file, so the first Save prompts for a location
   * and the template itself is never the save target — that detach IS its
   * immutability.
   */
  newFromTemplate(id: string): Promise<DocumentSnapshot>;

  /**
   * Captures one body at head as a reusable `document`-kind component (spec §7
   * "Save as Component").
   *
   * The backend freezes the document, bakes the body's solid through the
   * worker, stores it content-addressed, writes the package and re-indexes.
   * Refused when the body bakes to anything other than exactly one solid (spec
   * §9) — checked at SAVE time, where the author can still fix it, rather than
   * at placement on someone else's machine.
   *
   * **The component seats at its MODEL ORIGIN, +Z up.** The placement solver
   * aligns a component's local origin/+Z to the target, and an authored body's
   * origin is wherever it was modelled — so "put the origin on the face that
   * seats" is the authoring rule, the same convention every built-in generator
   * follows. Per-attachment frames (which would lift that restriction) are not
   * in this build.
   *
   * `previewPng` is an optional viewport capture (data URL or bare base64),
   * stored as the package thumbnail; a malformed one is dropped, never fatal.
   */
  saveAsComponent(
    bodyId: string,
    spec: NewComponentSpec,
    previewPng?: string | null,
  ): Promise<LibraryComponent>;

  /**
   * Swaps a placed instance to a different component, IN PLACE at the same
   * record (spec §3.3 `ReplaceComponent`) — identity, timeline position and
   * every downstream reference survive.
   *
   * A recorded mate rides across only when the new component declares an
   * attachment of the same name; otherwise it is dropped, reported in the
   * returned {@link ReplaceComponentReport}, and the instance holds its frozen
   * placement. Never silently re-bound to a different attachment.
   */
  replaceComponent(
    recordId: string,
    componentId: string,
    componentVersion: string,
    params?: Record<string, ComponentParamValue>,
  ): Promise<ReplaceComponentReport>;

  /**
   * Whether a newer version of the component behind `recordId` is indexed
   * (spec §3.3's opt-in upgrade). Read-only: a document must never open
   * differently because a shared library moved underneath it. `null` when the
   * record is not a placed component, or is already on the newest version.
   */
  componentUpgradeAvailable(recordId: string): Promise<ComponentUpgrade | null>;

  /**
   * Drops a placed component's library identity, keeping its cached
   * geometry as an ordinary body (spec §3.4) — the sanctioned in-place
   * `PlaceComponent` → `DetachComponent` op-type swap at the same record.
   */
  detachComponent(recordId: string): Promise<void>;

  /**
   * Applies free-parameter overrides to an already-placed component instance
   * (spec §3.1 `params`; Component Library WP-2.3). Every requested key must
   * be declared `role: "free"` on the component's `[parameters]` signature
   * (`LibraryComponent.parameters`) — an unknown or non-free key is refused
   * server-side, never silently dropped or silently applied. Resolves once
   * the edit is applied, same as `placeComponent`/`detachComponent`; the
   * regenerated geometry arrives via the usual projection event stream.
   */
  setComponentParams(
    recordId: string,
    params: Record<string, ComponentParamValue>,
  ): Promise<void>;

  /**
   * The read-only first half of the OffsetFace authoring transaction
   * (SCHEMA §7.6 `PrepareOffsetFace`): the G1 tangent closure over the picked
   * faces, the `Total` opposite candidate, and the `currentDims` that seed an
   * absolute distance type.
   *
   * MINTS NOTHING. The response carries snapshot-scoped TopoKeys plus
   * descriptor/anchor evidence only; the caller promotes that evidence through
   * `promoteSelection` (Invariant 2) and only THEN authors the typed params.
   *
   * A REFUSAL (`crossBody`, `chainMismatch`, `unsupportedSurface`, …) resolves
   * SUCCESSFULLY carrying `refusal` — the tool has to explain why it cannot
   * offset, and a rejection would erase the reason.
   *
   * MOCK LIMIT: the mock's closure is the picks themselves (no tangent
   * expansion, no opposite-face search) with fixed `currentDims`.
   */
  prepareOffsetFace(req: PrepareOffsetFaceRequest): Promise<PrepareOffsetFaceResult>;

  /** Read-only, snapshot-fenced Fillet/Chamfer tangent-closure preparation. */
  prepareEdgeOp(req: PrepareEdgeOpRequest): Promise<PrepareEdgeOpResult>;

  // ── Topology repair (SCHEMA §9; M4b) ──────────────────────────────────────

  /**
   * Subscribe to `needs-repair` events (emitted after every published regen —
   * empty `items` means repairs cleared). The real event arrives from Rust; the
   * mock exposes a test seam (`emitMockNeedsRepair`). Returns an unsubscribe.
   */
  onNeedsRepair(cb: (event: NeedsRepairEvent) => void): Unsubscribe;

  /**
   * Dry-run the resolution ladder for repair refs (`resolve_refs`; binds
   * nothing). Returns the full un-lossy resolution per ref (candidates + reason +
   * anchor on `needsRepair`). The mock returns canned candidates.
   */
  resolveRefs(refs: ResolveRefRequest[]): Promise<ResolveRefResult[]>;

  /**
   * Apply one RAW `EditCommand` (repair rebind + history affordances — suppress /
   * rollback / delete). Returns the correlated regen result (same shape as
   * `applyOperation`). The real client routes to `apply_edit_command`; the mock
   * mutates its local document model.
   */
  applyEditCommand(command: WireEditCommand): Promise<ApplyOperationResult>;

  /**
   * Clear the worker crash-circuit breaker and resolve to how many poison keys
   * were forgotten (H2 `clear_worker_circuit`).
   *
   * The ordinary fix for a poisoned op is to EDIT it — a poison key is
   * `(historyPrefixHash, opRecordId, occtFingerprint)`, so changing the params
   * mints a new key by itself. This exists for what the key cannot see: an input
   * file repaired outside the document, or a worker rebuilt in place. No document
   * needs to be open. The mock has no circuit and resolves 0.
   */
  clearWorkerCircuit(): Promise<number>;

  /**
   * Fetch a stored operation's params (the EditCommand `op.params` serde shape),
   * keyed by its record id. A parametric re-edit that changes ONE scalar (revolve
   * angle / shell thickness / fillet radius) fetches these on arm and deep-merges
   * the scalar on commit, so it preserves the non-scalar inputs the projection does
   * not expose (axis / openFaces / edges). The real client routes to
   * `get_operation_params`; the mock returns the op's stored params.
   */
  getOperationParams(recordId: string): Promise<Record<string, unknown>>;

  /**
   * A feature's upstream/downstream transitive closures from the core dependency
   * graph (H10 dependency view). Read-only — never mutates the timeline. Consumed
   * by delete/suppress confirmations (dependent counts) and the inspector's
   * "Depends on / Used by" section. The real client routes to
   * `feature_dependencies`; the mock derives the same producer-chain shape from
   * its own record lineage (`featureTouched`), honestly returning empty arrays for
   * a feature whose lineage the mock never tracked (e.g. the seeded fixture rows).
   */
  featureDependencies(featureId: string): Promise<FeatureDependencies>;

  /**
   * `recordId` iff the next placement gesture on `bodyId` may FOLD into that
   * existing `TransformBody` record instead of appending a new one; `null`
   * otherwise (SCHEMA §7.3 cumulative-placement rule — a placement is ONE
   * re-edited record per intent, never a stack of nudges).
   *
   * A read-only lineage query, deliberately answered by the backend: the frontend
   * sees a picked body and a FLAT history list, never "which record last touched
   * this body". The mock mirrors the same rule against its own record list.
   */
  canFoldTransform(bodyId: string): Promise<string | null>;

  // ── Model operations + two-level preview (SCHEMA §7.3 / NEW_SPEC §15) ──────
  // The real client routes these to the worker's ExecutePlan (op) + solver-style
  // preview lane; the mock synthesizes bodies + a feature timeline locally.

  /**
   * Apply one operation (Extrude / Fillet / Boolean) and commit it, returning the
   * new revision, the changed/removed bodies (pull-model mesh refs) and the full
   * feature timeline. A `featureId` on the op re-targets an existing feature
   * (parametric edit). Also emits a `document-changed` event.
   */
  applyOperation(op: OperationOp): Promise<ApplyOperationResult>;

  /**
   * Open a Level-2 preview session for a drafted op (NEW_SPEC §15). Returns the
   * session id + the body id the exact preview mesh is published under. The
   * frontend runs the Level-1 preview locally while streaming param updates here.
   */
  beginPreview(draft: PreviewDraft): Promise<PreviewSession>;

  /**
   * Push the latest drag params for a preview session (fire-and-forget,
   * latest-wins). `epoch` stamps the request so the frontend can discard stale
   * exact results. Exact meshes arrive asynchronously via `onPreviewResult`.
   */
  updatePreview(sessionId: string, params: PreviewParams, epoch: number): void;

  /**
   * End a preview session. `commit` materializes the op (same result shape as
   * `applyOperation`, plus a committed exact mesh on `onPreviewResult`); a cancel
   * discards the scratch state and resolves null.
   */
  endPreview(sessionId: string, commit: boolean): Promise<ApplyOperationResult | null>;

  /** Subscribe to exact Level-2 preview results (carry their epoch for reconcile). */
  onPreviewResult(cb: (r: PreviewResult) => void): Unsubscribe;

  /**
   * Subscribe to authoritative `projection-updated` events (SCHEMA §7.2). The
   * frontend owns the projection stores and re-hydrates them from one payload on
   * open/new/close/edit/regen (F-WP8 flag 2). The real event arrives from Rust;
   * the mock is a no-op (it seeds + mutates its projection stores directly).
   */
  onProjectionUpdated(cb: (projection: DocumentProjectionWire) => void): Unsubscribe;

  /**
   * Pull the current authoritative projection directly (SCHEMA §7.2), instead of
   * waiting for the next `projection-updated` event. Used once on viewport mount to
   * recover bodies/sketches/features that were published BEFORE the listeners
   * attached (open/new/recover populate the document before the viewport wires up).
   * The real client hydrates `documentStore` as a side effect (revision-reconciled,
   * same as the event path); the mock derives the same shape from its already-
   * hydrated stores (a no-op read).
   */
  getProjection(): Promise<DocumentProjectionWire>;

  /** Undo the last committed op → new revision + changed/removed bodies + timeline. */
  undo(): Promise<ApplyOperationResult>;
  /** Redo the last undone op. */
  redo(): Promise<ApplyOperationResult>;

  // ── Crash recovery (start screen) ─────────────────────────────────────────
  // A crashed session may leave an autosave behind; the start screen offers to
  // restore it. Rust owns the autosave sidecar + the crash detection; the mock
  // exposes a test-seeded seam (`setMockRecovery`).

  /**
   * Check whether a crashed session left an autosave to offer. Resolves the
   * recovery info, or null when there is nothing to recover.
   */
  checkRecovery(): Promise<RecoveryInfo | null>;

  /**
   * Resolve a pending recovery. `accept:true` restores the autosave and opens the
   * recovered document (returns a snapshot); `accept:false` discards it (returns null).
   */
  recoverDocument(accept: boolean): Promise<DocumentSnapshot | null>;

  // ── Close/quit confirmation (unsaved-changes guard) ────────────────────────
  // Rust intercepts BOTH the native window-close button AND an app-level exit
  // request (⌘Q), preventing either server-side so the frontend always gets a
  // chance to prompt for unsaved changes first. Both funnel into ONE event.

  /**
   * Subscribe to backend `close-requested` events — the native window-close
   * button or an app-level exit request (e.g. ⌘Q), either already prevented
   * server-side pending this decision (`appStore.requestClose("quit")`). The
   * mock never emits (there is no native window/process to close).
   */
  onCloseRequested(cb: () => void): Unsubscribe;

  /**
   * Resolve a pending close/quit prompt by proceeding: clears the backend's
   * re-entrancy guard and exits the app. A no-op in the mock (Playwright-over-vite
   * has no app process to exit).
   */
  confirmExit(): Promise<void>;

  /**
   * Resolve a pending close/quit prompt by cancelling: clears the backend's
   * re-entrancy guard WITHOUT exiting, so a later attempt prompts again. A no-op
   * in the mock.
   */
  cancelExit(): Promise<void>;

  // ── STEP import into the OPEN document (STEP-IMPORT WP-A) ──────────────────

  /**
   * Append a STEP import to the OPEN document — an `ImportStep` history record,
   * NOT a document swap (that is `importStep(path)`, the start-screen lane).
   *
   * Takes no arguments: Rust owns the `.step` file dialog exactly as it owns the
   * open dialog, so the path never crosses the webview boundary. Resolves `null`
   * when the dialog was cancelled (the house shape — see `exportStep`), otherwise
   * the correlated regen result whose `changedBodies` are the imported bodies.
   *
   * The AUTHORITATIVE document update rides the normal projection/regen events on
   * the real lane (the mock writes its projection stores directly, as it does for
   * every other committed change), so a caller only awaits this for the outcome —
   * it must not hydrate the store off the return value.
   */
  insertStep(): Promise<ApplyOperationResult | null>;

  /**
   * Show a native file-open dialog filtered to STEP files (`.step` / `.stp`) and
   * resolve to the chosen path, or null when cancelled.
   *
   * SEPARATE from `openFileDialog` on purpose: that one filters `.onecad`, so a
   * user driving the start screen's "Import STEP…" through it could not select a
   * STEP file at all. Rust owns both dialogs (the webview has zero fs/dialog
   * capability); the mock returns a fake path.
   *
   * Only the START-SCREEN lane needs this — `insertStep` opens its own dialog
   * inside Rust and takes no path.
   */
  stepFileDialog(): Promise<string | null>;
}

/**
 * Pick the client for the current runtime — the single construction point.
 *
 * Inside a Tauri webview `window.__TAURI_INTERNALS__` is injected (the property
 * `invoke` bridge lives on it); we build the real `tauriClient`. In a plain
 * browser, vitest jsdom, or Playwright-over-vite there is no bridge, so the mock
 * drives the whole UI unchanged. `mockIPC` (test) also sets `__TAURI_INTERNALS__`,
 * so a test that mocks the bridge exercises the real client — as intended.
 *
 * The tauri client is memoized: it owns persistent event listeners + one solver
 * lane, so all call sites (appStore, ViewportRoot, badge layer) share one.
 */
let tauriClientSingleton: CadClient | null = null;

export function createClient(): CadClient {
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    return (tauriClientSingleton ??= createTauriClient());
  }
  return mockClient;
}
