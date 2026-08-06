/*
 * In-memory CadClient — drives the full start screen + editor UI with no backend.
 *
 * Seeded with a spread of names + dates so name-sort (A→Z), date-sort (newest
 * first) and substring search are all visibly exercised. Doc operations resolve
 * after a short simulated latency so the store's loading states are real.
 *
 * The sketch SOLVER lane + the drag-time PREVIEW lane live in the shared
 * `localSolver` module (F-WP8 seam) so the real `tauriClient` reuses them
 * verbatim. This file owns the mock's DOCUMENT model (synthetic bodies + feature
 * timeline + undo/redo); the tauri client replaces that half with real commands.
 */
import type { CadClient } from "./client";
import type {
  ApplyOperationResult,
  BodyMeshRef,
  DocumentChange,
  DocumentProjectionWire,
  DocumentSnapshot,
  ElementInfo,
  MassProperties,
  EnterSketchTarget,
  FeatureDependencies,
  FeatureRecord,
  Lod,
  NeedsRepairEvent,
  OperationOp,
  PrepareOffsetFaceRequest,
  PrepareOffsetFaceResult,
  PromotedElement,
  SketchAttachTarget,
  SketchPlane,
  PromotePick,
  RecentProject,
  RecoveryInfo,
  ResolveCandidate,
  ResolveRefRequest,
  ResolveRefResult,
  SketchConstraintType,
  SketchEntity,
  SketchSession,
  SketchUpsertResult,
  TransformBodyParams,
  Unsubscribe,
  WorkerStatus,
} from "./types";
import { IMPORT_STEP_OP_TYPE } from "./types";
import type { WireEditCommand } from "./tauriCommandMap";
import { bareBodyId, wireParamsOf } from "./tauriCommandMap";
import { holeValueText } from "@/tools/modelTools/holeMachine";
import { offsetFaceValue } from "@/tools/preview/faceOffset";
import type { FaceColor } from "./mockMeshes";
import {
  concatMesh1,
  makeBoxMesh,
  makeCylinderMesh,
  makeExtrudeBodyMesh,
  makeRevolveBodyMesh,
  transformMesh1,
} from "./mockMeshes";
import { placementMatrix } from "@/tools/preview/patternPreview";
import type { LatheAxis } from "@/tools/preview/lathePreview";
import { createLocalSolverLane } from "./localSolver";
import { lookupMockFace, mockElementHash } from "./mockFaceGeometry";
import {
  edgeMetricsFromMesh,
  faceMetricsFromMesh,
  massPropertiesFromMesh,
} from "./mockMeshMetrics";
import { planeFor, solveDof } from "./mockSketch";
import { detectRegions } from "./mockRegions";
import type { DatumMeta } from "@/stores/documentStore";
import { documentStore, emptyDocument } from "@/stores/documentStore";

const LATENCY_MS = 120;
const MESH_LATENCY_MS = 30;

/**
 * Backend latency the mock simulates for document-changed + preview results.
 * Mutable so the 60fps gate can crank it to 300ms and prove the L1 preview holds
 * refresh rate while L2 lags. `wait()` for doc ops reads this live.
 */
let mockLatency = LATENCY_MS;
export function setMockLatency(ms: number): void {
  mockLatency = Math.max(0, ms);
}
export function getMockLatency(): number {
  return mockLatency;
}

const wait = (ms = mockLatency) => new Promise((r) => setTimeout(r, ms));

let nextDocId = 1;
const snapshot = (title: string): DocumentSnapshot => ({
  documentId: `doc-${nextDocId++}`,
  title,
});

/*
 * Stand-ins for the data-URI thumbnails `list_recents` returns for a container
 * that has a `preview.png`. Two 1×1 PNGs — the smallest payload that is a REAL
 * decodable image, so ProjectCard's `<img>` branch (rather than its hatched
 * empty well) is what the mock lane and the e2e/vitest suites actually exercise.
 *
 * Only the first two entries carry one, deliberately: the mixed grid is the
 * honest shape of a recents list, where an older project saved before this
 * feature — or one whose capture was refused — has no preview at all.
 */
const MOCK_THUMB_A =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR42mPo6JkCAANIAamDtMgRAAAAAElFTkSuQmCC";
const MOCK_THUMB_B =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR42mOIyesCAAJ+AVW9raoPAAAAAElFTkSuQmCC";

// Varied names + dates (unsorted on purpose — the UI sorts).
const RECENTS: RecentProject[] = [
  {
    id: "p-bracket",
    name: "Bracket v2",
    path: "/Users/andrej/CAD/Projects/Bracket v2.onecad",
    modifiedAt: "2026-07-16T14:20:00Z",
    thumbnail: MOCK_THUMB_A,
  },
  {
    id: "p-enclosure",
    name: "Enclosure rev C",
    path: "/Users/andrej/CAD/Projects/Enclosure rev C.onecad",
    modifiedAt: "2026-07-14T09:05:00Z",
    thumbnail: MOCK_THUMB_B,
  },
  {
    id: "p-gearbox",
    name: "Gearbox mount",
    path: "/Users/andrej/Client/Gearbox/Gearbox mount.onecad",
    modifiedAt: "2026-07-09T18:42:00Z",
  },
  {
    id: "p-camera",
    name: "Camera rig plate",
    path: "/Users/andrej/CAD/Rigs/Camera rig plate.onecad",
    modifiedAt: "2026-06-30T11:15:00Z",
  },
  {
    id: "p-heatsink",
    name: "Heatsink shroud",
    path: "/Users/andrej/CAD/Projects/Heatsink shroud.onecad",
    modifiedAt: "2026-06-21T16:00:00Z",
  },
  {
    id: "p-adapter",
    name: "Adapter flange",
    path: "/Users/andrej/CAD/Projects/Adapter flange.onecad",
    modifiedAt: "2026-06-10T08:30:00Z",
  },
  {
    id: "p-untitled",
    name: "Untitled",
    path: "/Users/andrej/CAD/Projects/Untitled.onecad",
    modifiedAt: "2026-06-02T13:00:00Z",
  },
];

// ── Crash recovery (start screen) — test-seeded seam ────────────────────────
//
// DEFAULT NONE so the start screen shows no recovery banner unless a test opts in
// (existing StartScreen/App tests stay green). `setMockRecovery` seeds/clears it;
// `checkRecovery` reports it; `recoverDocument` accepts (restore) or discards it.
let mockRecovery: RecoveryInfo | null = null;

/** Test seam: seed (or clear) the crash-recovery offer the start screen checks. */
export function setMockRecovery(r: RecoveryInfo | null): void {
  mockRecovery = r ? { ...r } : null;
}

// ── Mesh + document-changed emitter (mock backend surface) ──────────────────

/** Which synthesized body geometry a given bodyId serves. */
function meshForBody(bodyId: string): ArrayBuffer {
  return bodyId === "body2" ? makeCylinderMesh() : makeBoxMesh();
}

/** MeshCache-style key mirroring Rust's `(BodyId, Lod, generation)`. */
export function mockMeshKey(bodyId: string, lod: Lod, generation = 1): string {
  return `${bodyId}:${lod}:${generation}`;
}

const docChangeListeners = new Set<(c: DocumentChange) => void>();

/**
 * Simulate a worker `document-changed` event (the demo / seed fires this so the
 * viewport ingests through the SAME onDocumentChanged path the real worker uses).
 */
export function emitMockDocumentChanged(change: DocumentChange): void {
  for (const cb of [...docChangeListeners]) cb(change);
}

// ── Topology repair (M4b) — needs-repair emitter + canned resolveRefs ──────────

const needsRepairListeners = new Set<(e: NeedsRepairEvent) => void>();

/** Test seam: push a `needs-repair` event through the mock (drives the banner). */
export function emitMockNeedsRepair(event: NeedsRepairEvent): void {
  for (const cb of [...needsRepairListeners]) cb(event);
}

// ── Mock document model: synthetic bodies + feature timeline + undo/redo ───────
//
// applyOperation / endPreview(commit) append feature entries and synthesize
// bodies; undo/redo restore whole-document snapshots (simple + always correct for
// a mock). Body meshes live here keyed by bodyId (getBodyMesh reads them, falling
// back to the seed box/cylinder). All shapes mirror SCHEMA §7.3 so the F-WP8 swap
// is a no-op for the tool layer.

/** Base timeline — MUST mirror documentStore.seedMockDocument().features. */
const MOCK_BASE_FEATURES: FeatureRecord[] = [
  { id: "f1", kind: "sketch", opType: "Sketch", label: "Sketch 1", valueText: "", status: "ok" },
  { id: "f2", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "83.3 mm", primaryValue: 83.3, primaryValueKind: "length", status: "ok" },
  { id: "f3", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", primaryValue: 2, primaryValueKind: "length", status: "ok" },
  { id: "f4", kind: "sketch", opType: "Sketch", label: "Sketch 2", valueText: "", status: "ok" },
  { id: "f5", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "12.0 mm", primaryValue: 12, primaryValueKind: "length", status: "ok" },
];

/**
 * Stored params for the SEEDED Sketch features, so `getOperationParams` answers
 * for them the way the real backend does (`SketchOpParams` serializes its sketch
 * as `sketchId`).
 *
 * The seeded timeline is fabricated, so its rows had no params at all and every
 * caller that resolves a feature→sketch link (H9 reattach from a history row)
 * hit a hard "unknown record" on the mock while working fine against the real
 * backend — a lane divergence, not a limit. The mapping pairs each seeded Sketch
 * ROW with a seeded document SKETCH in tree order (see `seedMockDocument`).
 */
const MOCK_SEEDED_SKETCH_PARAMS: Record<string, Record<string, unknown>> = {
  f1: { sketchId: "sketch2" },
  f4: { sketchId: "sketch4" },
};

/** The `{primaryValue, primaryValueKind}` pair for a feature row — mirrors the
 *  Rust `dto.rs feature_value` arms, which are what the real lane projects. */
function primary(
  value: number,
  kind: "length" | "angle" | "diameter" = "length",
): Pick<FeatureRecord, "primaryValue" | "primaryValueKind"> {
  return { primaryValue: value, primaryValueKind: kind };
}

const cloneFeature = (f: FeatureRecord): FeatureRecord => ({ ...f });

/** Synthetic body meshes by bodyId (seed body1 is a fallback box, not stored). */
const syntheticBodies = new Map<string, ArrayBuffer>();

/**
 * The MESH1 bytes this mock serves for `bodyId` — synthesized geometry (extrude
 * output) first, else the seed box/cylinder.
 *
 * Factored out because `getBodyMesh` is no longer the only reader: WP-C1's
 * `massProperties` and the mesh-derived `elementInfo` both MEASURE these bytes,
 * and all three must see the same geometry or the mock would report numbers for
 * a body other than the one on screen.
 */
function mockBodyMesh(bodyId: string): ArrayBuffer {
  return syntheticBodies.get(bodyId) ?? meshForBody(bodyId);
}
let mockFeatures: FeatureRecord[] = MOCK_BASE_FEATURES.map(cloneFeature);
/**
 * The mock ROLLBACK CURSOR — the applied op count, mirroring core
 * `Timeline::cursor` (M8). `mockFeatures[0, mockAppliedOps)` are applied;
 * everything past it is a draft whose bodies are masked out of the scene
 * ({@link maskedBodies}). Seeded fully-applied, like `seedMockDocument`.
 */
let mockAppliedOps = MOCK_BASE_FEATURES.length;
/** Bodies hidden by the cursor: `bodyId → mesh`, restored on a roll forward. */
const maskedBodies = new Map<string, ArrayBuffer>();
let mockRevision = 5; // matches the seed projection revision
let nextBodySeq = 2; // body1 is the seed body
let nextFeatureSeq = 100;

/** featureId → bodyId, so a parametric edit rebuilds the SAME body. */
const featureBodies = new Map<string, string>();

/** featureId → last committed wire params (the `get_operation_params` source).
 *  Pre-seeded for the fabricated Sketch rows — see {@link MOCK_SEEDED_SKETCH_PARAMS}. */
const featureParams = new Map<string, Record<string, unknown>>(
  Object.entries(MOCK_SEEDED_SKETCH_PARAMS).map(([id, p]) => [id, { ...p }]),
);

// ── Body / sketch METADATA (name + visible) — the mock's stand-in for the Rust
//    `document.bodies` / `document.sketches` overlay (TRUST wave).
//
// The real backend owns these facts and re-asserts them onto the frontend through
// every `projection-updated`, which is exactly why a LOCAL-only tree flip silently
// reverted. The mock has no event stream, so without an equivalent authority a
// local flip would survive here and any e2e "still hidden after a commit" check
// would pass vacuously. `reassertMockMetadata()` (run on every committed op) plays
// the backend's part: it forces each id it OWNS back to the mock's recorded value.
//
// Ownership is seeded from the mock document (so a never-touched row keeps its
// seeded value — `Sketch 5` stays hidden) and extended by the metadata commands.
// Bodies/sketches created later are NOT owned and are left alone.
interface MockMeta {
  name?: string;
  visible: boolean;
}
const mockBodyMeta = new Map<string, MockMeta>();
const mockSketchMeta = new Map<string, MockMeta>();

function seedMockMetadata(): void {
  mockBodyMeta.clear();
  mockSketchMeta.clear();
  const s = documentStore.getState();
  for (const b of Object.values(s.bodies)) mockBodyMeta.set(b.id, { visible: b.visible });
  for (const k of Object.values(s.sketches)) mockSketchMeta.set(k.id, { visible: k.visible });
}
seedMockMetadata();

/** Re-assert the mock-owned metadata onto the projection store (the backend's role). */
function reassertMockMetadata(): void {
  const s = documentStore.getState();
  const bodies = { ...s.bodies };
  let bodiesChanged = false;
  for (const [id, meta] of mockBodyMeta) {
    const row = bodies[id];
    if (!row) continue;
    const name = meta.name ?? row.name;
    if (row.visible === meta.visible && row.name === name) continue;
    bodies[id] = { ...row, name, visible: meta.visible };
    bodiesChanged = true;
  }
  const sketches = { ...s.sketches };
  let sketchesChanged = false;
  for (const [id, meta] of mockSketchMeta) {
    const row = sketches[id];
    if (!row) continue;
    const name = meta.name ?? row.name;
    if (row.visible === meta.visible && row.name === name) continue;
    sketches[id] = { ...row, name, visible: meta.visible };
    sketchesChanged = true;
  }
  if (bodiesChanged || sketchesChanged) s.applyChange({ bodies, sketches });
}

/** Record + apply one metadata mutation (the `setVisibility`/`rename*` arms). */
function writeMockMeta(kind: "body" | "sketch", id: string, patch: Partial<MockMeta>): void {
  const registry = kind === "body" ? mockBodyMeta : mockSketchMeta;
  const s = documentStore.getState();
  const row = kind === "body" ? s.bodies[id] : s.sketches[id];
  // Adopt the row's CURRENT state on first touch, so renaming a body never also
  // resurrects it (the patch is the only thing this command means to change).
  const current = registry.get(id) ?? { visible: row?.visible ?? true, name: row?.name };
  registry.set(id, { ...current, ...patch });
  reassertMockMetadata();
}

// ── Datum planes (DATUM W1) ──────────────────────────────────────────────────
//
// The mock owns datums in the projection store, like the backend does. Two mock
// LIMITS worth naming:
//
// * **Sketch→datum attachments are not modelled.** `SketchMeta` carries no
//   attachment (the real `SketchDto` doesn't either — attachments are core-owned
//   and never projected), and the mock lane has no sketch-creation command at
//   all, so nothing populates `mockSketchDatum` today. It exists so the
//   referenced-guard below is real code with a real registry rather than a
//   permanently-true branch, and so the datum TOOL half can register an
//   attachment through `mockAttachSketchToDatum` without reworking this.
// * Resolution is a hand-mirror of the core rule, not the core itself.

/** sketch id → the datum it is attached to (see the limits note above). */
const mockSketchDatum = new Map<string, string>();

/**
 * Test/tool seam: record that `sketchId` is hosted on `datumId` so `deleteDatum`
 * guards on it. Pass `null` to clear. No-op on the real client.
 */
export function mockAttachSketchToDatum(sketchId: string, datumId: string | null): void {
  if (datumId === null) mockSketchDatum.delete(sketchId);
  else mockSketchDatum.set(sketchId, datumId);
}

/**
 * MOCK MIRROR of `DocumentSession::resolve_datum_frame`: slide the base frame
 * along its own NORMAL by `offset`, carrying the base axes verbatim. The base is
 * a named world plane or another RESOLVED datum; anything else is unresolvable
 * and comes back `resolvedValid: false` (lenient, exactly like the core).
 *
 * The named bases are the non-standard `Sketch.h` ones (`mockSketch.PLANES`), so
 * "XZ offset 10" moves along world **+X** here too.
 */
function mockResolveDatum(basePlaneId: string, offset: number): { plane: SketchPlane; resolvedValid: boolean } {
  const named = basePlaneId === "XY" || basePlaneId === "XZ" || basePlaneId === "YZ";
  const chained = named ? undefined : documentStore.getState().datums[basePlaneId];
  const base = named ? planeFor(basePlaneId) : chained?.resolvedValid ? chained.plane : undefined;
  if (!base) return { plane: { ...planeFor("XY"), kind: "custom" }, resolvedValid: false };
  const n = base.normal;
  return {
    plane: {
      kind: "custom",
      origin: [base.origin[0] + n[0] * offset, base.origin[1] + n[1] * offset, base.origin[2] + n[2] * offset],
      xAxis: [...base.xAxis],
      yAxis: [...base.yAxis],
      normal: [...base.normal],
    },
    resolvedValid: true,
  };
}

interface DocSnap {
  label: string;
  features: FeatureRecord[];
  bodies: Map<string, ArrayBuffer>;
  /** Datums are projection-store state, so undo has to carry them too — a snap
   *  that forgot them would silently resurrect a deleted datum on redo. */
  datums: Record<string, DatumMeta>;
  /** The rollback cursor + the bodies it was masking, so an undo that crosses a
   *  cursor move restores the same document the user was looking at (M8). */
  cursor: number;
  masked: Map<string, ArrayBuffer>;
}
const undoStack: DocSnap[] = [];
const redoStack: DocSnap[] = [];

const bodyRef = (bodyId: string): BodyMeshRef => ({
  bodyId,
  meshKey: mockMeshKey(bodyId, "coarse", mockRevision),
});

function snap(label: string): DocSnap {
  return {
    label,
    features: mockFeatures.map(cloneFeature),
    bodies: new Map(syntheticBodies),
    datums: { ...documentStore.getState().datums },
    cursor: mockAppliedOps,
    masked: new Map(maskedBodies),
  };
}

/** Compute changed (new/replaced) + removed bodies between two body maps. */
function diffBodies(
  from: Map<string, ArrayBuffer>,
  to: Map<string, ArrayBuffer>,
): { changed: string[]; removed: string[] } {
  const changed: string[] = [];
  const removed: string[] = [];
  for (const [id, mesh] of to) if (from.get(id) !== mesh) changed.push(id);
  for (const id of from.keys()) if (!to.has(id)) removed.push(id);
  return { changed, removed };
}

/** Restore a snapshot; bumps the revision + returns the resulting body diff. */
function restoreSnap(s: DocSnap): { changed: string[]; removed: string[] } {
  const before = new Map(syntheticBodies);
  mockFeatures = s.features.map(cloneFeature);
  syntheticBodies.clear();
  for (const [k, v] of s.bodies) syntheticBodies.set(k, v);
  maskedBodies.clear();
  for (const [k, v] of s.masked) maskedBodies.set(k, v);
  mockAppliedOps = s.cursor;
  documentStore.getState().applyChange({ datums: { ...s.datums } });
  mockRevision += 1;
  return diffBodies(before, syntheticBodies);
}

// ── Rollback cursor (M8) ──────────────────────────────────────────────────────
//
// The mock's mirror of core `Timeline` (`history/timeline.rs`). Two rules, both
// taken verbatim from there rather than invented here:
//
// * `set_cursor(k)` (timeline.rs:198-210) — clamp k to [0, len]; steps that leave
//   the applied prefix stop contributing geometry. The mock has no regen, so
//   "stops contributing" is modelled by MASKING the bodies those records touched
//   (`featureTouched`, the same lineage `mockCanFoldTransform` walks).
// * `insert_at_cursor` (timeline.rs:174-185) — a fresh record goes in AT the
//   cursor, the cursor advances past it (`cursor = index + 1`), and that record
//   plus every step after it become Dirty.
//
// MOCK LIMIT: masking is set arithmetic over `featureTouched`, not a replay. A
// body an applied record also touched stays visible even if a later, rolled-back
// record touched it too — the mock has no way to rebuild the earlier state of
// that body, and showing the LAST applied state is the honest approximation.

/**
 * Re-derive which bodies the cursor hides, then move it to `k`. Returns the body
 * diff (masked ⇒ removed, unmasked ⇒ changed) for the caller's result payload.
 */
function setMockCursor(k: number): { changed: string[]; removed: string[] } {
  mockAppliedOps = Math.max(0, Math.min(Math.floor(k), mockFeatures.length));
  const hidden = new Set<string>();
  const kept = new Set<string>();
  mockFeatures.forEach((f, i) => {
    for (const b of featureTouched.get(f.id) ?? []) (i < mockAppliedOps ? kept : hidden).add(b);
  });
  const changed: string[] = [];
  const removed: string[] = [];
  // Unmask first: a body an applied record touched must come back even if a
  // rolled-back record touched it too (see the MOCK LIMIT above).
  for (const [id, mesh] of [...maskedBodies]) {
    if (hidden.has(id) && !kept.has(id)) continue;
    maskedBodies.delete(id);
    syntheticBodies.set(id, mesh);
    changed.push(id);
  }
  for (const id of hidden) {
    if (kept.has(id) || !syntheticBodies.has(id)) continue;
    maskedBodies.set(id, syntheticBodies.get(id)!);
    syntheticBodies.delete(id);
    removed.push(id);
  }
  return { changed, removed };
}

/**
 * Move a just-APPENDED record to the cursor, mirroring `insert_at_cursor`.
 *
 * `mutateOp` appends (it is the frontier case and by far the common one), so this
 * is the one place the rolled-back case is corrected: splice the new row from the
 * tail to `cursor`, advance the cursor past it, and mark the tail dirty. When the
 * document is already at the frontier this is exactly "append + cursor += 1".
 */
function insertAtMockCursor(featureId: string): void {
  const last = mockFeatures.length - 1;
  if (last < 0 || mockFeatures[last].id !== featureId) return; // an EDIT, not an append
  const index = Math.min(mockAppliedOps, last);
  if (index < last) {
    const [row] = mockFeatures.splice(last, 1);
    mockFeatures.splice(index, 0, row);
  }
  mockAppliedOps = index + 1;
  // Inserted node + tail are pending regen (timeline.rs `mark_dirty_from`),
  // skipping suppressed steps exactly as `set_dirty_preserving_suppressed` does.
  // A row that already carries a FAILURE status keeps it: dirtying it would erase
  // the halt point the failure-visibility layer reads, and the mock has no regen
  // that could clear it again.
  mockFeatures = mockFeatures.map((f, i) =>
    i > index && !f.suppressed && f.status === "ok" ? { ...f, status: "dirty" } : f,
  );
}

/** Stamp the timeline cursor onto a result (H7b: the cursor rides the edit result). */
function withCursor(res: ApplyOperationResult): ApplyOperationResult {
  return { ...res, appliedOps: mockAppliedOps, totalOps: mockFeatures.length };
}

function nextBodyId(): string {
  return `body${nextBodySeq++}`;
}
function nextFeatureId(): string {
  return `mf${nextFeatureSeq++}`;
}

/** A deterministic axis just left of a profile (so a re-edit with no axis still forms a body). */
function fallbackRevolveAxis(ring: [number, number][]): LatheAxis {
  let minU = Infinity;
  let minV = Infinity;
  let maxV = -Infinity;
  for (const [u, v] of ring) {
    if (u < minU) minU = u;
    if (v < minV) minV = v;
    if (v > maxV) maxV = v;
  }
  const x = minU - 1;
  return { a: [x, minV], b: [x, maxV] };
}

/**
 * Boolean-mode suffix for a feature-mode Add/Cut/Intersect extrude/revolve label
 * ("Extrude (Cut)" etc). NewBody → no suffix.
 *
 * MOCK BOOLEAN LIMIT (documented honestly): the mock has no CSG. "Add" is a NAIVE
 * VISUAL CONCAT — the new prism/lathe mesh is appended to the target body's mesh
 * (concatMesh1), so the target visibly grows but no shared boundary is removed.
 * "Cut"/"Intersect" are geometry NO-OPS — the target mesh is left unchanged and
 * only a feature row is added. Every boolean mode reports `changedBodies:[target]`
 * and creates NO new body. The e2e specs assert history rows + body-diff, never
 * the resulting geometry, so this stand-in is sufficient (real CSG is OCCT's job).
 */
function booleanSuffix(mode: string | undefined): string {
  return mode === "Add" || mode === "Cut" || mode === "Intersect" ? ` (${mode})` : "";
}

/**
 * Whether a boolean target id names a body the mock knows — either a mock-synthesized
 * body (created via a prior op) OR a projection body (e.g. the seeded Body 1). A
 * synthetic mesh is required only for an Add concat; a Cut/Intersect no-op just needs
 * a valid id. An unknown id falls back to a fresh NewBody op.
 */
function booleanTargetKnown(target: string | undefined): target is string {
  return !!target && (syntheticBodies.has(target) || documentStore.getState().bodies[target] !== undefined);
}

// ── TransformBody (WP-B W1) ──────────────────────────────────────────────────
//
// The one mock body op that is EXACT rather than a stand-in: a rigid transform of
// a tessellation is what the kernel produces too (`transformMesh1`). Two pieces of
// bookkeeping make it parametric rather than incremental:
//
// * `transformBases` freezes each target's PRE-placement mesh at the record's
//   first commit, so a re-edit from 30mm to 50mm lands at 50 — not at 80. Without
//   it the mock would compose every edit onto the already-moved mesh and the
//   "one cumulative record" semantics would be a lie on this lane.
// * `transformCopies` pins a `copy: true` record's minted body ids, so a re-edit
//   rewrites the SAME copies instead of littering new ones.

/** featureId → {bodyId → the mesh that record started from}. */
const transformBases = new Map<string, Map<string, ArrayBuffer>>();
/** featureId → the body ids a `copy: true` record minted, in `targets` order. */
const transformCopies = new Map<string, string[]>();
/** featureId → the bodies that record touched (the mock's stand-in for record lineage). */
const featureTouched = new Map<string, string[]>();

/**
 * Ensure every body an op will rewrite IN PLACE is in `syntheticBodies` BEFORE the
 * undo snapshot is taken. The seed bodies (`body1`/`body2`) live only as a
 * `getBodyMesh` fallback, so a placement that added them afterwards would make
 * `diffBodies` read the undo as a body REMOVAL and delete it from the scene.
 */
function materializeOpTargets(op: OperationOp): void {
  if (op.opType !== "TransformBody") return;
  for (const id of op.params.targets) {
    if (!syntheticBodies.has(id)) syntheticBodies.set(id, meshForBody(id));
  }
}

/** The mock's `dto.rs transform_value_text` mirror: Δ, else angle, else identity. */
function transformValueText(p: TransformBodyParams): string {
  const [dx, dy, dz] = p.translate;
  if (dx !== 0 || dy !== 0 || dz !== 0) {
    return `Δ(${dx.toFixed(1)}, ${dy.toFixed(1)}, ${dz.toFixed(1)})`;
  }
  if (p.rotate.angleDeg !== 0) return `${p.rotate.angleDeg.toFixed(1)}°`;
  return "0.0 mm";
}

function mutateTransform(
  p: TransformBodyParams,
  editId: string | undefined,
): { changed: string[]; removed: string[]; label: string; featureId: string } {
  const featureId = editId ?? nextFeatureId();
  const editing = editId !== undefined && transformBases.has(editId);
  const bases = editing
    ? transformBases.get(featureId)!
    : new Map(p.targets.map((id) => [id, syntheticBodies.get(id) ?? meshForBody(id)]));
  if (!editing) transformBases.set(featureId, bases);

  const m = placementMatrix(p.translate, p.rotate.center, p.rotate.axis, p.rotate.angleDeg);
  const changed: string[] = [];
  if (p.copy) {
    // `copy: true` preserves the sources and mints one body per target, id-stable
    // across re-edits (the real op mints `body_<opId>[:k]` — same guarantee).
    const ids = transformCopies.get(featureId) ?? p.targets.map(() => nextBodyId());
    transformCopies.set(featureId, ids);
    p.targets.forEach((src, k) => {
      syntheticBodies.set(ids[k], transformMesh1(bases.get(src)!, m));
      changed.push(ids[k]);
    });
  } else {
    for (const id of p.targets) {
      syntheticBodies.set(id, transformMesh1(bases.get(id)!, m));
      changed.push(id);
    }
  }

  const valueText = transformValueText(p);
  if (editing) {
    mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText } : f));
  } else {
    mockFeatures = [
      ...mockFeatures,
      { id: featureId, kind: "boolean", opType: "TransformBody", label: "Move", valueText, status: "ok" },
    ];
  }
  return { changed, removed: [], label: "Move", featureId };
}

/**
 * The mock's mirror of core `document::transform::can_fold_transform`.
 *
 * Walk the record list from the END for the first record that TOUCHED `bodyId`;
 * fold iff that record is a `copy: false` TransformBody listing it as a target.
 * `featureTouched` stands in for the core's `inputs.bodies ∪ outputs` — it is the
 * body diff each record actually produced, which is the same question.
 *
 * Anything else — a fillet, a boolean, a `copy: true` placement, an untouched
 * body — is `null` and appends a fresh record. Folding into a transform some
 * LATER op already consumed would retroactively move the geometry that op was
 * built on, which is the H5-B failure mode one layer up.
 */
function mockCanFoldTransform(bodyId: string): string | null {
  for (let i = mockFeatures.length - 1; i >= 0; i--) {
    const f = mockFeatures[i];
    if (!featureTouched.get(f.id)?.includes(bodyId)) continue;
    if (f.opType !== "TransformBody") return null;
    const params = featureParams.get(f.id);
    if (!params || params.copy === true) return null;
    const targets = params.targets;
    return Array.isArray(targets) && targets.includes(bareBodyId(bodyId)) ? f.id : null;
  }
  return null;
}

/**
 * The mock's mirror of core `DependencyGraph::upstream`/`downstream` (H10
 * dependency view). `featureTouched` again stands in for
 * `inputs.bodies ∪ outputs` (see {@link mockCanFoldTransform}): walking the
 * feature list in creation order, the most recent EARLIER feature to touch a
 * given body is that body's "producer" at this point — an edge producer→consumer
 * forms whenever a later feature touches a body an earlier one did, the same
 * body-producer-chain the core `DependencyGraph` builds from
 * `OperationRecord::outputs`. Upstream/downstream are the transitive closures
 * over those edges, same algorithm shape as `graph.rs` `collect`.
 *
 * HONEST LIMIT: `MOCK_BASE_FEATURES` (the seeded document) bypasses `mutateOp`,
 * so those rows carry no `featureTouched` entry — querying one answers with two
 * empty arrays rather than a fabricated chain. A feature created through a real
 * mock op (extrude/fillet/boolean/…) gets its real producer-chain edges.
 */
function mockFeatureDependencies(featureId: string): FeatureDependencies {
  const forward = new Map<string, Set<string>>(); // producer id → consumer ids
  const backward = new Map<string, Set<string>>(); // consumer id → producer ids
  const lastToucher = new Map<string, string>(); // bodyId → most recent feature id
  for (const f of mockFeatures) {
    for (const bodyId of featureTouched.get(f.id) ?? []) {
      const producer = lastToucher.get(bodyId);
      if (producer && producer !== f.id) {
        if (!forward.has(producer)) forward.set(producer, new Set());
        forward.get(producer)!.add(f.id);
        if (!backward.has(f.id)) backward.set(f.id, new Set());
        backward.get(f.id)!.add(producer);
      }
      lastToucher.set(bodyId, f.id);
    }
  }
  const closure = (start: string, edges: Map<string, Set<string>>): string[] => {
    const visited = new Set<string>();
    const stack = [...(edges.get(start) ?? [])];
    while (stack.length > 0) {
      const next = stack.pop()!;
      if (visited.has(next)) continue;
      visited.add(next);
      for (const n of edges.get(next) ?? []) stack.push(n);
    }
    return [...visited];
  };
  return { upstream: closure(featureId, backward), downstream: closure(featureId, forward) };
}

/** Apply one op forward (mutates features + bodies); returns the body diff. */
function mutateOp(op: OperationOp): {
  changed: string[];
  removed: string[];
  label: string;
  featureId: string;
} {
  if (op.opType === "Extrude") {
    const { plane, profile } = lane.resolveExtrudeInput(op.sketchId, op.regionId);
    const distance = op.params.distance ?? 10;
    const booleanMode = op.params.booleanMode ?? "NewBody";
    const target = op.params.targetBodyId;
    // Feature-mode boolean (Add/Cut/Intersect against an existing body): changes the
    // TARGET, creates no new body (mock CSG limit — see booleanSuffix).
    if (booleanMode !== "NewBody" && booleanTargetKnown(target)) {
      // Add grows the target mesh (naive concat) IF the target has a synthetic mesh;
      // Cut/Intersect leave geometry untouched.
      if (booleanMode === "Add" && syntheticBodies.has(target)) {
        syntheticBodies.set(target, concatMesh1(syntheticBodies.get(target)!, makeExtrudeBodyMesh(profile, plane, distance)));
      }
      const featureId = op.featureId ?? nextFeatureId();
      const label = `Extrude${booleanSuffix(booleanMode)}`;
      const valueText = `${Math.abs(distance).toFixed(1)} mm`;
      mockFeatures = [...mockFeatures, { id: featureId, kind: "extrude", opType: "Extrude", label, valueText, ...primary(Math.abs(distance)), status: "ok" }];
      return { changed: [target], removed: [], label, featureId };
    }
    const editing = op.featureId !== undefined && featureBodies.has(op.featureId);
    const featureId = op.featureId ?? nextFeatureId();
    const bodyId = editing ? featureBodies.get(featureId)! : nextBodyId();
    syntheticBodies.set(bodyId, makeExtrudeBodyMesh(profile, plane, distance));
    featureBodies.set(featureId, bodyId);
    const valueText = `${Math.abs(distance).toFixed(1)} mm`;
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText, ...primary(Math.abs(distance)) } : f));
    } else {
      mockFeatures = [...mockFeatures, { id: featureId, kind: "extrude", opType: "Extrude", label: "Extrude", valueText, ...primary(Math.abs(distance)), status: "ok" }];
    }
    return { changed: [bodyId], removed: [], label: "Extrude", featureId };
  }
  if (op.opType === "Revolve") {
    const { plane, profile } = lane.resolveExtrudeInput(op.sketchId, op.regionId);
    const angle = op.params.angleDeg ?? 360;
    const axisLine =
      op.params.axis?.kind === "sketchLine"
        ? lane.resolveSketchLine(op.sketchId, op.params.axis.lineId)
        : null;
    // Fall back to a vertical axis just left of the profile so a body still forms
    // (re-edit carries no axis; the mock only needs a deterministic revolve).
    const axis = axisLine ?? fallbackRevolveAxis(profile.ring);
    const booleanMode = op.params.booleanMode ?? "NewBody";
    const target = op.params.targetBodyId;
    // Feature-mode boolean revolve (see the Extrude branch + booleanSuffix).
    if (booleanMode !== "NewBody" && booleanTargetKnown(target)) {
      if (booleanMode === "Add" && syntheticBodies.has(target)) {
        syntheticBodies.set(target, concatMesh1(syntheticBodies.get(target)!, makeRevolveBodyMesh(profile.ring, axis, plane, angle)));
      }
      const featureId = op.featureId ?? nextFeatureId();
      const label = `Revolve${booleanSuffix(booleanMode)}`;
      const valueText = `${Math.round(Math.abs(angle))}°`;
      mockFeatures = [...mockFeatures, { id: featureId, kind: "revolve", opType: "Revolve", label, valueText, ...primary(angle, "angle"), status: "ok" }];
      return { changed: [target], removed: [], label, featureId };
    }
    const editing = op.featureId !== undefined && featureBodies.has(op.featureId);
    const featureId = op.featureId ?? nextFeatureId();
    const bodyId = editing ? featureBodies.get(featureId)! : nextBodyId();
    syntheticBodies.set(bodyId, makeRevolveBodyMesh(profile.ring, axis, plane, angle));
    featureBodies.set(featureId, bodyId);
    const valueText = `${Math.round(Math.abs(angle))}°`;
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText, ...primary(angle, "angle") } : f));
    } else {
      mockFeatures = [...mockFeatures, { id: featureId, kind: "revolve", opType: "Revolve", label: "Revolve", valueText, ...primary(angle, "angle"), status: "ok" }];
    }
    return { changed: [bodyId], removed: [], label: "Revolve", featureId };
  }
  if (op.opType === "Fillet" || op.opType === "Chamfer") {
    // MOCK LIMIT: no real rounding/bevelling — re-emit the target body + add a
    // feature. Chamfer shares FilletChamferParams, so the two differ only in the
    // label; a re-edit (featureId of an existing one) updates the value text.
    const label = op.opType;
    const bodyId = op.inputs?.[0]?.primary.bodyId ?? "body1";
    const featureId = op.featureId ?? nextFeatureId();
    const valueText = edgeOpValueText(op.params.radius, op.params.distance2);
    const editing = op.featureId !== undefined && mockFeatures.some((f) => f.id === featureId);
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText, ...primary(op.params.radius) } : f));
    } else {
      mockFeatures = [...mockFeatures, { id: featureId, kind: "fillet", opType: op.opType, label, valueText, ...primary(op.params.radius), status: "ok" }];
    }
    return { changed: [bodyId], removed: [], label, featureId };
  }
  if (op.opType === "Shell") {
    // MOCK LIMIT: no real hollowing — re-emit the shelled body + a feature. A
    // re-edit (featureId of an existing shell) just updates the thickness text.
    const bodyId = op.params.targetBodyId ?? op.inputs?.[0]?.primary.bodyId ?? "body1";
    const featureId = op.featureId ?? nextFeatureId();
    const valueText = `${op.params.thickness.toFixed(1)} mm`;
    const editing = op.featureId !== undefined && mockFeatures.some((f) => f.id === featureId);
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText, ...primary(op.params.thickness) } : f));
    } else {
      // `kind` mirrors the REAL projection bucket (`dto.rs feature_kind` folds
      // Shell → fillet and the pattern/mirror ops → boolean); `opType` carries the
      // authored identity every re-edit routes on. Emitting the invented kinds the
      // mock used to emit made the mock lane green while the Tauri lane was dead.
      mockFeatures = [...mockFeatures, { id: featureId, kind: "fillet", opType: "Shell", label: "Shell", valueText, ...primary(op.params.thickness), status: "ok" }];
    }
    return { changed: [bodyId], removed: [], label: "Shell", featureId };
  }
  if (op.opType === "OffsetFace") {
    // MOCK LIMIT: no real face offset. Moving a face and re-closing the solid is
    // OCCT's `BRepOffset_MakeOffset`; reproducing it here would be a second,
    // worse kernel. So the mock does what it does for every other body-MODIFYING
    // op (Shell, Fillet/Chamfer, Hole): re-emit the target untouched and record
    // the feature. The GEOMETRY is pinned against the real kernel by
    // `worker/tests/test_offsetface.cpp` + `src-tauri/tests/offset_face.rs`; what
    // the mock lane owns is the UI chain — prepare → arm → drag → chip → commit.
    const bodyId = op.params.targetBodyId || op.inputs?.[0]?.primary.bodyId || "body1";
    const featureId = op.featureId ?? nextFeatureId();
    const { valueText, primaryValue, primaryValueKind } = offsetFaceValue(
      op.params.distance,
      op.params.distanceType,
    );
    const editing = op.featureId !== undefined && mockFeatures.some((f) => f.id === featureId);
    if (editing) {
      mockFeatures = mockFeatures.map((f) =>
        f.id === featureId ? { ...f, valueText, primaryValue, primaryValueKind } : f,
      );
    } else {
      // `kind: "fillet"` mirrors the REAL projection bucket — `dto.rs feature_kind`
      // folds OffsetFace into the Fillet/Chamfer/Shell dress-up family. `opType`
      // carries the authored identity every re-edit routes on.
      mockFeatures = [
        ...mockFeatures,
        {
          id: featureId,
          kind: "fillet",
          opType: "OffsetFace",
          // Matches `dto.rs default_label`.
          label: "Offset face",
          valueText,
          primaryValue,
          primaryValueKind,
          status: "ok",
        },
      ];
    }
    return { changed: [bodyId], removed: [], label: "Offset face", featureId };
  }
  if (op.opType === "Hole") {
    // MOCK LIMIT: no real drilling. Subtracting a faceted cylinder from the mock
    // box mesh would be a second, worse CSG implementation living in the mock —
    // so the mock does what it does for every body-MODIFYING op (Shell,
    // Fillet/Chamfer): re-emit the host untouched and record the feature. The
    // GEOMETRY is pinned against the real kernel by `worker/tests/test_hole_op.cpp`
    // and `src-tauri/tests/hole_ops.rs`; what the mock lane owns is the UI chain —
    // face pick → chips → committed record → re-edit seed.
    const bodyId = op.params.targetBodyId || op.inputs?.[0]?.primary.bodyId || "body1";
    const featureId = op.featureId ?? nextFeatureId();
    // ONE formatter, shared with the tool layer, mirroring `dto.rs
    // feature_value_text` for Hole exactly (`Ø` + one decimal).
    const valueText = holeValueText(op.params.diameter);
    const editing = op.featureId !== undefined && mockFeatures.some((f) => f.id === featureId);
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText, ...primary(op.params.diameter, "diameter") } : f));
    } else {
      // `kind: "boolean"` mirrors the REAL projection bucket — `dto.rs feature_kind`
      // buckets Hole with the body-modifier family, not with Fillet's dress-up
      // bucket. `opType` carries the authored identity the re-edit routes on.
      mockFeatures = [...mockFeatures, { id: featureId, kind: "boolean", opType: "Hole", label: "Hole", valueText, ...primary(op.params.diameter, "diameter"), status: "ok" }];
    }
    return { changed: [bodyId], removed: [], label: "Hole", featureId };
  }
  if (op.opType === "LinearPattern") {
    // MOCK LIMIT: no real instancing — re-emit the source body + a feature.
    const bodyId = op.params.sourceBodyId ?? op.inputs?.[0]?.primary.bodyId ?? "body1";
    const featureId = op.featureId ?? nextFeatureId();
    const valueText = `×${op.params.count}`;
    const editing = op.featureId !== undefined && mockFeatures.some((f) => f.id === featureId);
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText } : f));
    } else {
      mockFeatures = [
        ...mockFeatures,
        { id: featureId, kind: "boolean", opType: "LinearPattern", label: "Linear Pattern", valueText, status: "ok" },
      ];
    }
    return { changed: [bodyId], removed: [], label: "Linear Pattern", featureId };
  }
  if (op.opType === "CircularPattern") {
    const bodyId = op.params.sourceBodyId ?? op.inputs?.[0]?.primary.bodyId ?? "body1";
    const featureId = op.featureId ?? nextFeatureId();
    const valueText = `×${op.params.count}`;
    const editing = op.featureId !== undefined && mockFeatures.some((f) => f.id === featureId);
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText } : f));
    } else {
      mockFeatures = [
        ...mockFeatures,
        { id: featureId, kind: "boolean", opType: "CircularPattern", label: "Circular Pattern", valueText, status: "ok" },
      ];
    }
    return { changed: [bodyId], removed: [], label: "Circular Pattern", featureId };
  }
  if (op.opType === "MirrorBody") {
    const bodyId = op.params.sourceBodyId ?? op.inputs?.[0]?.primary.bodyId ?? "body1";
    const featureId = op.featureId ?? nextFeatureId();
    const valueText = planeLabelForNormal(op.params.planeNormal);
    const editing = op.featureId !== undefined && mockFeatures.some((f) => f.id === featureId);
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText } : f));
    } else {
      mockFeatures = [...mockFeatures, { id: featureId, kind: "boolean", opType: "MirrorBody", label: "Mirror", valueText, status: "ok" }];
    }
    return { changed: [bodyId], removed: [], label: "Mirror", featureId };
  }
  if (op.opType === "TransformBody") return mutateTransform(op.params, op.featureId);
  // Boolean: MOCK removes the tool body, keeps the target (no real fusion).
  const { targetBodyId, toolBodyId, operation } = op.params;
  syntheticBodies.delete(toolBodyId);
  const featureId = op.featureId ?? nextFeatureId();
  mockFeatures = [
    ...mockFeatures,
    { id: featureId, kind: "boolean", opType: "Boolean", label: operation, valueText: "", status: "ok" },
  ];
  return { changed: [targetBodyId], removed: [toolBodyId], label: operation, featureId };
}

/** Commit an op: push undo, mutate, bump revision, build the result. */
function commitOp(op: OperationOp): ApplyOperationResult {
  // BEFORE the snapshot: an op that rewrites a seed body in place needs that
  // body's pre-op mesh captured, or undo reads as a deletion (see the helper).
  materializeOpTargets(op);
  undoStack.push(snap(labelForOp(op)));
  redoStack.length = 0;
  const { changed, removed, label, featureId } = mutateOp(op);
  // Remember the committed wire params so `getOperationParams` can serve a re-edit
  // its non-scalar inputs (axis / openFaces / edges) verbatim.
  featureParams.set(featureId, wireParamsOf(op));
  // …and which bodies it touched, the lineage `canFoldTransform` walks.
  featureTouched.set(featureId, [...changed, ...removed]);
  // A FRESH record joins the timeline at the rollback cursor, not at the tail
  // (core `Timeline::insert_at_cursor`); a re-edit of an existing one is a no-op here.
  insertAtMockCursor(featureId);
  mockRevision += 1;
  // A committed op is the mock's "projection-updated": whatever the backend holds
  // wins over whatever the tree flipped locally (see the mockBodyMeta header).
  reassertMockMetadata();
  return {
    revision: mockRevision,
    changedBodies: changed.map(bodyRef),
    removedBodies: removed,
    features: mockFeatures.map(cloneFeature),
    opLabel: label,
  };
}

function labelForOp(op: OperationOp): string {
  if (op.opType === "Boolean") return op.params.operation;
  return op.opType;
}

/**
 * The ONE `opType` change `UpdateOperationParams` may make — mirrors core
 * `session::op_type_edit_allowed`. Fillet and Chamfer have field-identical
 * params and derive the same inputs, so the unified edge tool's re-edit may flip
 * one into the other; every other pair is a remove + re-add, not a params update.
 */
function isSanctionedOpTypeSwap(prior: string, next: string): boolean {
  return (
    (prior === "Fillet" && next === "Chamfer") || (prior === "Chamfer" && next === "Fillet")
  );
}

/** Mirrors core `session::OP_TYPE_EDIT_REASON`. */
const OP_TYPE_EDIT_REASON = "UpdateOperationParams may not change opType";

/**
 * Mirrors core `session::CHAMFER_D2_FLIP_REASON` VERBATIM (SCHEMA §7.3,
 * 2026-08-03). A Chamfer carrying `distance2` breaks the swap's field-identity
 * precondition — a Fillet has no home for the second leg — so the flip is refused
 * until that field is cleared by an ordinary params edit.
 */
const CHAMFER_D2_FLIP_REASON =
  "UpdateOperationParams may not change opType: a Chamfer carrying distance2 is not flippable to Fillet (clear distance2 first)";

/**
 * The reason a `updateOperationParams` opType change is refused, or `null` when
 * it is allowed. The precondition is read off the PRIOR (stored) params, exactly
 * as core reads it off the prior RECORD: what the caller sends cannot buy its way
 * past a second leg the document still holds.
 */
function opTypeSwapRejection(
  prior: string,
  next: string,
  priorParams: Record<string, unknown> | undefined,
): string | null {
  if (prior === next) return null;
  if (!isSanctionedOpTypeSwap(prior, next)) return OP_TYPE_EDIT_REASON;
  if (prior === "Chamfer" && priorParams?.distance2 !== undefined) return CHAMFER_D2_FLIP_REASON;
  return null;
}

/**
 * REATTACH (H9), honestly modelled on this lane.
 *
 * The mock's sketch frame lives in the local solver lane (`cacheSketchPlane`),
 * and `resolveExtrudeInput` reads it, so moving the plane there and
 * re-synthesizing every extrude whose profile is this sketch reproduces the real
 * behaviour: the sketch's 2D coordinates are kept and reinterpreted in the new
 * basis, and everything downstream follows.
 *
 * MOCK LIMITS, stated rather than faked:
 *  • Only EXTRUDE geometry re-synthesizes — the same documented limit the
 *    `updateOperationParams` arm carries (Fillet/Shell/Hole have no mock CSG).
 *  • A sketch with an OPEN lane session keeps that session's plane until it is
 *    closed (`resolveExtrudeInput` prefers a live session), so reattach is a
 *    model-mode verb here exactly as it is in the UI.
 *  • Undoable, like every other mock EditCommand.
 */
async function mockReattachSketch(
  sketchId: string,
  target: SketchAttachTarget,
): Promise<ApplyOperationResult> {
  await wait();
  if (!documentStore.getState().sketches[sketchId]) {
    throw new Error(`reattachSketch: unknown sketch ${sketchId}`);
  }
  let plane: SketchPlane;
  if (target.kind === "world") {
    plane = planeFor(target.plane);
  } else {
    const datum = documentStore.getState().datums[target.datumId];
    if (!datum) throw new Error(`reattachSketch: unknown datum ${target.datumId}`);
    if (!datum.resolvedValid) {
      throw new Error(`reattachSketch: datum '${datum.name}' has an unresolved frame`);
    }
    plane = { ...datum.plane, kind: "custom" };
  }
  undoStack.push(snap("Reattach sketch"));
  redoStack.length = 0;
  lane.cacheSketchPlane(sketchId, plane);
  mockAttachSketchToDatum(sketchId, target.kind === "datum" ? target.datumId : null);

  // Rebuild every extrude standing on this sketch — this IS the "downstream
  // regen" a reattach is for; without it the mock would report success and show
  // nothing moving.
  const changed: string[] = [];
  for (const f of mockFeatures) {
    const params = featureParams.get(f.id);
    const profile = params?.profile as { sketchId?: string; regionId?: string } | undefined;
    const bodyId = featureBodies.get(f.id);
    if (f.opType !== "Extrude" || !bodyId || profile?.sketchId !== sketchId) continue;
    const distance = scalarValue(params?.distance as unknown);
    if (distance === undefined) continue;
    const resolved = lane.resolveExtrudeInput(profile.sketchId, profile.regionId);
    syntheticBodies.set(bodyId, makeExtrudeBodyMesh(resolved.profile, resolved.plane, distance));
    changed.push(bodyId);
  }
  mockRevision += 1;
  const res: ApplyOperationResult = {
    revision: mockRevision,
    changedBodies: changed.map(bodyRef),
    removedBodies: [],
    features: mockFeatures.map(cloneFeature),
    opLabel: "Reattach Sketch",
  };
  emitMockDocumentChanged({
    revision: res.revision,
    changedBodies: res.changedBodies,
    removedBodies: [],
  });
  return withCursor(res);
}

function noopResult(): ApplyOperationResult {
  return {
    revision: mockRevision,
    changedBodies: [],
    removedBodies: [],
    features: mockFeatures.map(cloneFeature),
  };
}

/** Commit one op through the local model + emit its document-changed (the lane's
 *  `commit` seam + the client's own `applyOperation` share this path). */
function commitAndEmit(op: OperationOp): Promise<ApplyOperationResult> {
  // The mock has no regen job, but the busy indicator is part of the UI under
  // test: bracket the (latency-simulating) apply with the same store transitions
  // the real `regen-started`/`regen-finished` pair drives.
  documentStore.getState().regenStarted();
  return wait()
    .then(() => {
      const res = commitOp(op);
      emitMockDocumentChanged({
        revision: res.revision,
        changedBodies: res.changedBodies,
        removedBodies: res.removedBodies,
      });
      return withCursor(res);
    })
    .finally(() => documentStore.getState().regenSettled());
}

// ── STEP import (STEP-IMPORT WP-A) ────────────────────────────────────────────
//
// MOCK LIMIT: there is no STEP reader in this lane. The fabrication below stands
// in for one — ONE box body offset clear of the seed box (so the import is
// visibly distinct rather than buried inside it), plus the `ImportStep` history
// row and the body NAME the real projection would publish.
//
// It writes the projection store itself because the mock has no event stream:
// the same "play the backend's part" role `reassertMockMetadata` fills. The name
// is registered as mock-OWNED so a later committed op's re-assert cannot rewrite
// it to "Body N".

let nextImportSeq = 1;

/** Where the Nth fabricated import body sits — a row marching clear along +X. */
const importOrigin = (n: number): [number, number, number] => [110 + (n - 1) * 60, 0, 0];

/**
 * The authored appearance the fabricated import carries (MESH1 FACE_COLORS) —
 * the mock's stand-in for STEP/XCAF colors, so the frontend's whole color path
 * (flag → section → de-index → vertex attribute → `shadedVertex` material) runs
 * with no backend. Exactly TWO of the six `BOX_FACES` are colored, so the same
 * body also proves the unset ⇒ body-token fallback.
 *
 * These are sRGB 0–255 DATA, not design tokens: an imported file's colors can
 * never be theme-derived.
 */
const IMPORT_FACE_COLORS: ReadonlyArray<FaceColor | null> = [
  [214, 74, 62, 255], // f:0  +X
  null, // f:1
  null, // f:2
  null, // f:3
  [58, 122, 196, 255], // f:4  +Z
  null, // f:5
];

/** Fabricate one import: body + `ImportStep` row + projection write. */
function commitImportStep(): ApplyOperationResult {
  undoStack.push(snap("Import"));
  redoStack.length = 0;
  const seq = nextImportSeq++;
  const bodyId = nextBodyId();
  const name = `Imported ${seq}`;
  syntheticBodies.set(
    bodyId,
    makeBoxMesh(40, 40, 40, 0, importOrigin(seq), IMPORT_FACE_COLORS),
  );
  const featureId = nextFeatureId();
  mockFeatures = [
    ...mockFeatures,
    // `kind` mirrors the REAL projection bucket (interim "boolean" — the backend
    // has no import bucket yet), so the row's icon MUST come from `opType`.
    { id: featureId, kind: "boolean", opType: IMPORT_STEP_OP_TYPE, label: "Import", valueText: "", status: "ok" },
  ];
  featureTouched.set(featureId, [bodyId]);
  insertAtMockCursor(featureId); // same insert-at-cursor rule as any other op
  mockRevision += 1;
  const doc = documentStore.getState();
  doc.applyChange({
    revision: mockRevision,
    features: mockFeatures.map(cloneFeature),
    bodies: { ...doc.bodies, [bodyId]: { id: bodyId, name, visible: true } },
    dirty: true,
    appliedOps: mockAppliedOps,
  });
  writeMockMeta("body", bodyId, { name }); // also re-asserts the owned metadata
  return {
    revision: mockRevision,
    changedBodies: [bodyRef(bodyId)],
    removedBodies: [],
    features: mockFeatures.map(cloneFeature),
    opLabel: "Import",
  };
}

/** Fabricate an import + fire the document-changed the viewport ingests through. */
function importStepAndEmit(): ApplyOperationResult {
  const res = commitImportStep();
  emitMockDocumentChanged({
    revision: res.revision,
    changedBodies: res.changedBodies,
    removedBodies: res.removedBodies,
  });
  return res;
}

/** Canned repair candidates for a ref (deterministic; descending score). */
function mockResolveRefs(refs: ResolveRefRequest[]): ResolveRefResult[] {
  return refs.map((r) => {
    const h = mockElementHash(r.refId);
    const candidates: ResolveCandidate[] = [
      {
        topoKey: `e:${(parseInt(h.slice(0, 2), 16) % 40) + 1}`,
        score: 0.91,
        margin: 0.02,
        worldPos: [12, 3.5, 0],
        summary: "linear edge, len≈40mm",
      },
      {
        topoKey: `e:${(parseInt(h.slice(2, 4), 16) % 40) + 1}`,
        score: 0.89,
        margin: 0.02,
        worldPos: [12, -3.5, 0],
        summary: "linear edge, len≈40mm",
      },
    ];
    return {
      refId: r.refId,
      outcome: "needsRepair",
      ladderFailed: "descriptor",
      reason: "ambiguous",
      scoringVersion: 1,
      uiLabel: "Fillet edge",
      candidates,
    };
  });
}

/** The numeric value of a wire `Scalar` (`{value}`), or `undefined` if not one. */
function scalarValue(v: unknown): number | undefined {
  if (v && typeof v === "object" && "value" in (v as Record<string, unknown>)) {
    const n = (v as { value: unknown }).value;
    if (typeof n === "number") return n;
  }
  return undefined;
}

/**
 * The history-row value text of an edge op — MIRRORS Rust `dto.rs
 * feature_value_text` (pinned there by
 * `chamfer_value_text_shows_the_second_distance_only_when_set`). A two-distance
 * chamfer reads `d1×d2`; anything else keeps the single-number form.
 * `radiusFromValueText` parses the LEADING number back, so d1 stays first.
 */
function edgeOpValueText(radius: number, distance2?: number): string {
  return distance2 === undefined
    ? `${radius.toFixed(1)} mm`
    : `${radius.toFixed(1)}×${distance2.toFixed(1)} mm`;
}

/**
 * The feature-chip value (text + inline-editable primary dimension) for a re-edited
 * op's wire params — the mock's mirror of Rust `dto.rs feature_value`, which likewise
 * decides both in ONE place so a row cannot show one number and edit another.
 *
 * Routed on the AUTHORED `opType`, not the coarse `kind` bucket: `kind: "boolean"`
 * holds Hole alongside Boolean/patterns/Move, and `kind: "fillet"` holds Shell —
 * neither can tell which params key carries the dimension.
 *
 * `undefined` fields mean "this op has nothing of that sort to update".
 */
function featureValueForParams(
  opType: string | undefined,
  kind: FeatureRecord["kind"],
  params: Record<string, unknown>,
): Partial<Pick<FeatureRecord, "valueText" | "primaryValue" | "primaryValueKind">> {
  const none = { valueText: undefined, primaryValue: undefined, primaryValueKind: undefined };
  const dimensioned = (
    valueText: string,
    value: number,
    primaryValueKind: FeatureRecord["primaryValueKind"] = "length",
  ) => ({ valueText, primaryValue: value, primaryValueKind });
  switch (opType ?? kind) {
    case "Extrude":
    case "extrude": {
      const d = scalarValue(params.distance);
      return d === undefined ? none : dimensioned(`${Math.abs(d).toFixed(1)} mm`, Math.abs(d));
    }
    case "Revolve":
    case "revolve": {
      const a = scalarValue(params.angleDeg);
      return a === undefined ? none : dimensioned(`${Math.round(Math.abs(a))}°`, a, "angle");
    }
    case "Fillet":
    case "Chamfer":
    case "fillet": {
      const r = scalarValue(params.radius);
      // `distance2` is Chamfer-only and skip-none on both sides, so its mere
      // presence is what makes the row asymmetric. The inline editor still targets
      // d1 (`radius`) alone, exactly as `dto.rs` does.
      return r === undefined ? none : dimensioned(edgeOpValueText(r, scalarValue(params.distance2)), r);
    }
    case "Shell":
    case "shell": {
      const t = scalarValue(params.thickness);
      return t === undefined ? none : dimensioned(`${t.toFixed(1)} mm`, t);
    }
    case "Hole": {
      const d = scalarValue(params.diameter);
      return d === undefined ? none : dimensioned(holeValueText(d), d, "diameter");
    }
    case "OffsetFace": {
      const d = scalarValue(params.distance);
      if (d === undefined) return none;
      // WHAT the number means is `distanceType`'s to say (dto.rs prefixes the row
      // `Ø`/`R` for the absolute cylindrical forms), so the text cannot be derived
      // from the value alone. An absent/unknown type is `Offset` — the wire default.
      const t = params.distanceType;
      const v = offsetFaceValue(
        d,
        t === "Total" || t === "Radius" || t === "Diameter" ? t : "Offset",
      );
      return dimensioned(v.valueText, v.primaryValue, v.primaryValueKind);
    }
    case "LinearPattern":
    case "CircularPattern":
    case "linearPattern":
    case "circularPattern":
      // A count is not a dimension — the row shows it and stays read-only.
      return typeof params.count === "number"
        ? { ...none, valueText: `×${params.count}` }
        : none;
    default:
      return none;
  }
}

/** Whether a wire-params subtree names `bodyId` (bare or in the `body_` wire form). */
function mentionsBody(value: unknown, bodyId: string): boolean {
  if (typeof value === "string") return value === bodyId || value === `body_${bodyId}`;
  if (Array.isArray(value)) return value.some((v) => mentionsBody(v, bodyId));
  if (value && typeof value === "object") return Object.values(value).some((v) => mentionsBody(v, bodyId));
  return false;
}

/**
 * The features the mock can HONESTLY prove depend on `recordId` — the downstream
 * half of the backend's suppression cascade (core `set_suppression` walks the
 * record dependency graph, whose body edges come from each record's regen
 * `outputs`).
 *
 * The mock has no dependency graph, so it derives the one edge it really tracks:
 * the body `recordId` produced (`featureBodies`) named by a LATER feature's
 * committed wire params (`featureParams` — `targetBodyId` / `toolBodyId` /
 * `sourceBodyId`, or a body id nested inside a typed ElementRef). Transitive via
 * the worklist below, since each newly-caught feature has its own produced body.
 *
 * MOCK LIMIT, deliberately UNDER-approximating: a feature with no recorded body
 * (the seeded f1–f5 timeline) or a consumer with no recorded params yields no
 * cascade. Inventing "everything after it is downstream" would be a WRONG graph,
 * and a wrong cascade is worse than a missing one.
 */
function dependentFeatureIds(recordId: string): string[] {
  const found = new Set<string>();
  const queue: string[] = [recordId];
  while (queue.length > 0) {
    const id = queue.shift()!;
    const body = featureBodies.get(id);
    if (!body) continue;
    const from = mockFeatures.findIndex((f) => f.id === id);
    if (from < 0) continue;
    for (const f of mockFeatures.slice(from + 1)) {
      if (f.id === recordId || found.has(f.id)) continue;
      const params = featureParams.get(f.id);
      if (params && mentionsBody(params, body)) {
        found.add(f.id);
        queue.push(f.id);
      }
    }
  }
  return [...found];
}

/** Apply one raw EditCommand against the mock document model (M4b). */
async function mockApplyEditCommand(command: WireEditCommand): Promise<ApplyOperationResult> {
  await wait();
  switch (command.cmd) {
    case "removeOperation": {
      undoStack.push(snap("Delete feature"));
      redoStack.length = 0;
      // A record removed from INSIDE the applied prefix shortens it — the cursor
      // counts applied ops, so leaving it alone would make it point past the end.
      const removedIndex = mockFeatures.findIndex((f) => f.id === command.record);
      mockFeatures = mockFeatures.filter((f) => f.id !== command.record);
      if (removedIndex >= 0 && removedIndex < mockAppliedOps) mockAppliedOps -= 1;
      mockAppliedOps = Math.min(mockAppliedOps, mockFeatures.length);
      mockRevision += 1;
      const res: ApplyOperationResult = {
        revision: mockRevision,
        changedBodies: [],
        removedBodies: [],
        features: mockFeatures.map(cloneFeature),
        opLabel: "Delete",
      };
      emitMockDocumentChanged({ revision: res.revision, changedBodies: [], removedBodies: [] });
      return res;
    }
    case "updateOperationParams": {
      // A parametric re-edit deep-merged the scalar into the stored params at the
      // mapper (preserving axis / openFaces / edges). Reflect it: store the merged
      // params, refresh the feature's value text from the changed scalar, and re-emit
      // its body (the lean mock keeps the same mesh — a documented geometry limit).
      const feat = mockFeatures.find((f) => f.id === command.record);
      const params = command.op.params as unknown as Record<string, unknown>;
      const nextOpType = command.op.opType;
      // MIRROR the core allow-list (`session::op_type_edit_allowed`): `opType` is
      // structural, so a params update may not change it — with the ONE sanctioned
      // Fillet⇄Chamfer pair. Without this the mock lane stays green on an edit the
      // real backend rejects (e.g. Fillet→Extrude), which is exactly the class of
      // divergence the mock exists to avoid.
      const priorOpType = feat?.opType;
      if (priorOpType !== undefined) {
        const rejection = opTypeSwapRejection(
          priorOpType,
          nextOpType,
          featureParams.get(command.record),
        );
        if (rejection !== null) throw new Error(rejection);
      }
      const typeChanged = priorOpType !== undefined && priorOpType !== nextOpType;
      if (feat) {
        undoStack.push(snap("Update"));
        redoStack.length = 0;
        // `op.params` is the COMPLETE params object — every producer merges a
        // re-edit patch into the stored params BEFORE sending
        // (`updateScalarParamsCommand` / `rewriteFilletEdgeParams` /
        // `operationToEditCommand`), and the real backend replaces the whole op.
        // Spreading the previous params over it would therefore make a key the
        // caller deliberately OMITTED un-removable — a cleared `distance2` would
        // come straight back, and the mock would report an asymmetric chamfer the
        // backend no longer holds.
        featureParams.set(command.record, { ...params });
        const { valueText, primaryValue, primaryValueKind } = featureValueForParams(
          nextOpType,
          feat.kind,
          params,
        );
        // A Boolean re-edit swaps ONLY the operation, and a boolean carries no
        // dimension (`featureValueForParams` returns nothing, matching dto.rs
        // `feature_value`) — but the mock LABELS a boolean row by its
        // operation (mutateOp), so the label is what has to follow the swap.
        // A sanctioned Fillet⇄Chamfer swap labels by the new `opType`, matching how
        // `mutateOp` stamps a freshly authored edge op.
        const label =
          feat.opType === "Boolean" && typeof params.operation === "string"
            ? params.operation
            : typeChanged
              ? nextOpType
              : undefined;
        const opType = typeChanged ? nextOpType : undefined;
        if (valueText !== undefined || label !== undefined || opType !== undefined) {
          mockFeatures = mockFeatures.map((f) =>
            f.id === command.record
              ? {
                  ...f,
                  ...(valueText !== undefined ? { valueText, primaryValue, primaryValueKind } : {}),
                  ...(label !== undefined ? { label } : {}),
                  ...(opType !== undefined ? { opType } : {}),
                }
              : f,
          );
        }
      }
      mockRevision += 1;
      const bodyId = featureBodies.get(command.record);
      // Re-synthesize the geometry an EXTRUDE's params describe, so a params-only
      // re-edit (the history row's inline depth edit) actually changes the mesh
      // instead of re-emitting the old one. Deliberately extrude-only: it is the
      // one op whose mock mesh is a pure function of its params + profile — Fillet
      // /Shell/Hole have no mock geometry to recompute (documented MOCK LIMIT), and
      // faking one would be a second, worse CSG living in the mock.
      const profile = params.profile as { sketchId?: string; regionId?: string } | undefined;
      const distance = scalarValue(params.distance);
      if (bodyId && feat?.opType === "Extrude" && profile?.sketchId && distance !== undefined) {
        const resolved = lane.resolveExtrudeInput(profile.sketchId, profile.regionId);
        syntheticBodies.set(bodyId, makeExtrudeBodyMesh(resolved.profile, resolved.plane, distance));
      }
      const changedBodies = bodyId ? [bodyRef(bodyId)] : [];
      const res: ApplyOperationResult = {
        revision: mockRevision,
        changedBodies,
        removedBodies: [],
        features: mockFeatures.map(cloneFeature),
        opLabel: "Update",
      };
      emitMockDocumentChanged({ revision: res.revision, changedBodies, removedBodies: [] });
      return res;
    }
    case "editOperationInput": {
      // Rebind: no structural change in the lean mock, but bump the revision + emit
      // document-changed so the regen correlation resolves.
      mockRevision += 1;
      const res = noopResult();
      emitMockDocumentChanged({ revision: res.revision, changedBodies: [], removedBodies: [] });
      return res;
    }
    case "setOperationSuppression": {
      // Flip the tracked `suppressed` flag — the projection IS the dim state now
      // (no frontend overlay), so a suppress spec asserts on the returned array.
      // Undoable, like every other EditCommand (core `session.apply` mints an
      // inverse for SetOperationSuppression too), so ⌘Z round-trips a toggle.
      undoStack.push(snap(command.suppressed ? "Suppress" : "Unsuppress"));
      redoStack.length = 0;
      const cascaded = command.cascade && command.suppressed ? dependentFeatureIds(command.record) : [];
      const flip = new Set([command.record, ...cascaded]);
      mockFeatures = mockFeatures.map((f) =>
        flip.has(f.id) ? { ...f, suppressed: command.suppressed } : f,
      );
      mockRevision += 1;
      const res: ApplyOperationResult = {
        revision: mockRevision,
        changedBodies: [],
        removedBodies: [],
        features: mockFeatures.map(cloneFeature),
        opLabel: command.suppressed ? "Suppress" : "Unsuppress",
      };
      return res;
    }
    // ── Body / sketch metadata (TRUST wave) ────────────────────────────────────
    // These are `RegenHint::None` on the real backend: they publish a projection
    // and fire NO regen, so the mock likewise emits no document-changed. The mock
    // ids stay in the MOCK domain (`body1` / `sketch2`) — they never pass through
    // `bareBodyId`'s uuid expectations, and the two lanes' id domains stay apart.
    case "setVisibility": {
      const target = command.target;
      if ("body" in target) writeMockMeta("body", target.body, { visible: command.visible });
      else writeMockMeta("sketch", target.sketch, { visible: command.visible });
      mockRevision += 1;
      return { ...noopResult(), opLabel: command.visible ? "Show" : "Hide" };
    }
    case "renameBody": {
      writeMockMeta("body", command.body, { name: command.name });
      mockRevision += 1;
      return { ...noopResult(), opLabel: "Rename" };
    }
    case "renameSketch": {
      writeMockMeta("sketch", command.sketch, { name: command.name });
      mockRevision += 1;
      return { ...noopResult(), opLabel: "Rename" };
    }
    // ── Datum planes (DATUM W1) — also RegenHint::None, no document-changed ──
    case "addDatumPlane": {
      const d = command.datum;
      undoStack.push(snap("Create datum plane"));
      redoStack.length = 0;
      // The client-sent `resolvedPlane` is DISCARDED, exactly as the core
      // discards it — the mock re-derives the frame from the definition so both
      // lanes agree on where a datum actually is.
      const { plane, resolvedValid } = mockResolveDatum(d.basePlaneId, d.offset);
      documentStore.getState().addDatum({
        id: d.id,
        name: d.name,
        basePlaneId: d.basePlaneId,
        offset: d.offset,
        plane,
        resolvedValid,
      });
      mockRevision += 1;
      return { ...noopResult(), opLabel: "Create datum plane" };
    }
    case "deleteDatum": {
      const id = command.datum;
      if (!documentStore.getState().datums[id]) throw new Error(`deleteDatum: unknown datum ${id}`);
      // Referenced-guard, mirroring the core: a datum a sketch is hosted on must
      // not vanish under it, and the message NAMES the blockers.
      const blockers = [...mockSketchDatum.entries()].filter(([, d]) => d === id).map(([s]) => s);
      if (blockers.length > 0) {
        throw new Error(
          `deleteDatum: datum ${id} is referenced by ${blockers.length} sketch(es): ${blockers.join(", ")}`,
        );
      }
      undoStack.push(snap("Delete datum plane"));
      redoStack.length = 0;
      documentStore.getState().removeDatum(id);
      mockRevision += 1;
      return { ...noopResult(), opLabel: "Delete datum plane" };
    }
    case "setRollback": {
      // Move the cursor + mask/unmask the bodies the rolled-back records made
      // (see `setMockCursor`). Undoable like every other EditCommand, and the
      // masked meshes ride the snapshot so ⌘Z cannot resurrect a half-state.
      undoStack.push(snap("Rollback"));
      redoStack.length = 0;
      const { changed, removed } = setMockCursor(command.cursor);
      mockRevision += 1;
      const res: ApplyOperationResult = {
        revision: mockRevision,
        changedBodies: changed.map(bodyRef),
        removedBodies: removed,
        features: mockFeatures.map(cloneFeature),
        opLabel: "Rollback",
      };
      emitMockDocumentChanged({
        revision: res.revision,
        changedBodies: res.changedBodies,
        removedBodies: res.removedBodies,
      });
      return res;
    }
    default:
      mockRevision += 1;
      return { ...noopResult(), opLabel: "Edit" };
  }
}

// ── Shared sketch-solver + preview lane (F-WP8 seam; same module the tauri
//    client uses). Commit routes into the local document model above. ──────────
const lane = createLocalSolverLane({ commit: commitAndEmit, latencyMs: () => mockLatency });

/** A deterministic 60×40 rectangle used to hydrate a re-entered document sketch,
 *  so a reopened persisted sketch shows real geometry (parity with the real client
 *  reading the backend's stored entities) instead of a silent empty session. */
function seededSketchRectangle(): SketchEntity[] {
  return [
    { id: "e1", type: "Line", p0: [-30, -20], p1: [30, -20] },
    { id: "e2", type: "Line", p0: [30, -20], p1: [30, 20] },
    { id: "e3", type: "Line", p0: [30, 20], p1: [-30, 20] },
    { id: "e4", type: "Line", p0: [-30, 20], p1: [-30, -20] },
  ];
}

/**
 * Enter a sketch, hydrating a persisted document sketch on first entry (re-entry
 * parity with the real client). A string target that names a sketch in the
 * projection store but has no live lane session gets a deterministic rectangle
 * seeded before delegating. An unknown id (not in the document) is left untouched —
 * it opens an empty session, exactly as before.
 */
async function enterSketchWithHydration(target: EnterSketchTarget): Promise<SketchSession> {
  if (typeof target === "string" && documentStore.getState().sketches[target] && !lane.hasSession(target)) {
    await lane.enterSketch(target);
    await lane.sketchUpsert(target, seededSketchRectangle(), []);
  }
  const session = await lane.enterSketch(target);
  // DATUM W1: the shared lane models the sketch's PLANE but not its attachment
  // (see the `mockSketchDatum` note above), so the mock client is where a
  // datum-hosted sketch gets registered against its host. Without this the
  // `deleteDatum` referenced-guard would be permanently vacuous in the mock lane.
  if (typeof target !== "string" && "newOnDatum" in target) {
    mockAttachSketchToDatum(session.sketchId, target.newOnDatum.datumId);
  }
  return session;
}

/**
 * H3b — set ONE dimensional constraint's value without an edit session.
 *
 * Routed through the SHARED lane's `sketchUpsert` (the only writer of a lane
 * session) rather than mutating the session behind its back, so the re-solve, the
 * dof/status and the sketch revision all come from the same place the drawing tools
 * get them. A sketch the user has only selected has no lane session yet, so it is
 * hydrated first exactly as `enterSketchWithHydration` does.
 *
 * `type` is unused here: the mock IS the document, so it stores the UI-domain value
 * verbatim. The deg→rad conversion is a WIRE concern and lives in the tauri client.
 */
async function mockSetSketchDimension(
  sketchId: string,
  constraintId: string,
  _type: SketchConstraintType,
  value: number,
): Promise<SketchUpsertResult> {
  const session = lane.peekSession(sketchId) ?? (await enterSketchWithHydration(sketchId));
  const constraints = session.constraints.map((c) =>
    c.id === constraintId ? { ...c, value } : c,
  );
  return lane.sketchUpsert(sketchId, session.entities, constraints);
}

/** Test seam: forget all sketch state so a fresh sketch starts empty. */
export function resetMockSketches(): void {
  lane.resetSketches();
}

/** Test seam: forget the whole mock document (bodies, features, undo, sessions). */
export function resetMockDocument(): void {
  syntheticBodies.clear();
  maskedBodies.clear();
  featureBodies.clear();
  featureParams.clear();
  for (const [id, params] of Object.entries(MOCK_SEEDED_SKETCH_PARAMS)) {
    featureParams.set(id, { ...params });
  }
  featureTouched.clear();
  lane.resetPreview();
  mockFeatures = MOCK_BASE_FEATURES.map(cloneFeature);
  mockAppliedOps = MOCK_BASE_FEATURES.length;
  mockRevision = 5;
  nextBodySeq = 2;
  nextFeatureSeq = 100;
  nextImportSeq = 1;
  undoStack.length = 0;
  redoStack.length = 0;
  mockRecovery = null;
  mockSketchDatum.clear();
  documentStore.getState().applyChange({ datums: {} });
  // Re-adopt the (already reset) projection store as the mock's metadata authority.
  seedMockMetadata();
}

export const mockClient: CadClient = {
  async listRecents() {
    await wait();
    return RECENTS.map((p) => ({ ...p }));
  },
  async newDocument() {
    await wait();
    return snapshot("Untitled");
  },
  async openDocument(path) {
    await wait();
    const known = RECENTS.find((p) => p.path === path);
    return snapshot(known?.name ?? basename(path));
  },
  // START-SCREEN lane: a new document FROM a STEP file. The mock has no document
  // model to swap (newDocument/openDocument likewise just hand back a snapshot and
  // leave the seeded projection in place), so this runs the same fabrication the
  // in-editor lane does — the editor then opens with the imported body + its row.
  async importStep(path) {
    await wait();
    importStepAndEmit();
    return snapshot(basename(path));
  },
  async closeDocument() {
    await wait();
    documentStore.getState().applySnapshot(emptyDocument());
  },
  async checkRecovery() {
    await wait();
    return mockRecovery ? { ...mockRecovery } : null;
  },
  async recoverDocument(accept: boolean) {
    await wait();
    if (!accept) {
      mockRecovery = null;
      return null;
    }
    const snap = snapshot(mockRecovery?.originalPath ? basename(mockRecovery.originalPath) : "Recovered");
    mockRecovery = null;
    return snap;
  },
  async openFileDialog() {
    await wait(40);
    // Rust returns the real chosen path in F-WP8; here we fake a pick.
    return "/Users/andrej/CAD/Projects/Imported.onecad";
  },
  // The STEP-filtered sibling (start-screen import lane). Same fake-pick shape,
  // with the extension the real dialog would actually be able to return.
  async stepFileDialog() {
    await wait(40);
    return "/Users/andrej/CAD/Projects/Imported.step";
  },

  // Save/export are Rust-owned in the real app; the mock keeps them deterministic
  // (no filesystem): saveDocument is a no-op, Save As / Export return fake paths.
  // `_previewPng` is accepted and DISCARDED: there is no container to write a
  // preview.png into. Taking the parameter keeps the two clients' signatures in
  // step so a caller compiles identically against either.
  async saveDocument(_path?: string, _previewPng?: string | null) {
    await wait(40);
  },
  async saveDocumentAs(_previewPng?: string | null) {
    await wait(40);
    return "/Users/andrej/CAD/Projects/Untitled.onecad";
  },
  async exportStep() {
    await wait(40);
    return "/Users/andrej/CAD/Projects/Untitled.step";
  },
  async exportStl() {
    await wait(40);
    return "/Users/andrej/CAD/Projects/Untitled.stl";
  },
  async exportObj() {
    await wait(40);
    return "/Users/andrej/CAD/Projects/Untitled.obj";
  },

  // The mock has no worker, so it never emits worker-status (no-op unsubscribe).
  onWorkerStatus(_cb: (status: WorkerStatus) => void): Unsubscribe {
    return () => {};
  },

  async getBodyMesh(bodyId, _lod) {
    await wait(MESH_LATENCY_MS);
    return mockBodyMesh(bodyId);
  },

  onDocumentChanged(cb): () => void {
    docChangeListeners.add(cb);
    return () => docChangeListeners.delete(cb);
  },

  // The mock writes its projection stores directly (no backend event stream), so
  // the projection-updated subscription is a no-op that never fires.
  onProjectionUpdated(_cb: (p: DocumentProjectionWire) => void): Unsubscribe {
    return () => {};
  },

  // The mock's documentStore is already hydrated synchronously (no backend round
  // trip), so a one-shot pull is just a snapshot of current state — no store writes.
  async getProjection(): Promise<DocumentProjectionWire> {
    const s = documentStore.getState();
    return {
      status: s.status,
      revision: s.revision,
      title: s.title,
      dirty: s.dirty,
      bodies: { ...s.bodies },
      sketches: { ...s.sketches },
      // Back onto the WIRE shape: the store's `plane` carries a `kind` the
      // backend never sends (`DatumDto.plane` is a bare basis), so drop it here
      // rather than round-tripping a field the real projection lacks.
      datums: Object.fromEntries(
        Object.entries(s.datums).map(([id, d]) => [
          id,
          {
            id: d.id,
            name: d.name,
            kind: "OffsetFromPlane",
            basePlaneId: d.basePlaneId,
            offset: d.offset,
            plane: {
              origin: [...d.plane.origin] as [number, number, number],
              xAxis: [...d.plane.xAxis] as [number, number, number],
              yAxis: [...d.plane.yAxis] as [number, number, number],
              normal: [...d.plane.normal] as [number, number, number],
            },
            resolvedValid: d.resolvedValid,
          },
        ]),
      ),
      features: s.features.map((f) => ({ ...f })),
    };
  },

  /**
   * The picked face's frame, answered ANALYTICALLY per face (SKETCH-ON-FACE W2).
   *
   * There is still no kernel here, but the mock's two synthetic shapes are exact
   * solids whose faces are known in closed form (`mockFaceGeometry`), so the
   * frame comes from the real face's own centroid + outward normal run through
   * the SAME `plane_from_point_normal` rule the backend applies. Picking the top
   * face and picking a side face now genuinely differ — before W2 this returned
   * XY-at-the-body for every face, which made the mock lane unable to show the
   * defect class sketch-on-face is about.
   *
   * The cylinder's curved side face REFUSES, exactly as the backend does for a
   * non-planar face, so that path is reachable without a worker.
   *
   * MOCK LIMIT: a synthesized (extruded/boolean) body has no analytic entry, and
   * falls back to the old +Z-at-the-origin frame rather than refusing.
   */
  async faceSketchPlane(bodyId: string, elementId: string, topoKey?: string): Promise<SketchPlane> {
    await wait(MESH_LATENCY_MS);
    const found = lookupMockFace(bodyId, elementId, topoKey);
    if (found.kind === "nonPlanar") {
      throw new Error("only a planar face can host a sketch");
    }
    if (found.kind === "planar") return found.geometry.plane;
    return { ...planeFor("XY"), kind: "custom" };
  },

  /**
   * The mock's element descriptor, MEASURED off the body's own MESH1 bytes
   * (WP-C1) with a synthesized fallback.
   *
   * It used to be hash-synthesized end to end. That was defensible while measure
   * only reported a magnitude — but it reported the normal `(0,0,1)` for EVERY
   * face, so a two-face ANGLE reads 0° for every pair in this lane and any e2e
   * coverage of it would pass vacuously. The mock draws real MESH1 triangles, so
   * `mockMeshMetrics` derives area / length / bbox-centre / normal from the very
   * geometry on screen instead.
   *
   * MOCK LIMIT, unchanged in spirit: these are MESH values, not kernel values.
   * They are exact for the box (planar faces, straight edges) and faceted for the
   * cylinder — its side "face" is 24 quads, so its area under-reports 2πrh and it
   * correctly reports as NON-planar. `surfaceType` is therefore 0 (plane) only
   * when every triangle of the face shares a normal, else 1 (a stand-in for
   * `GeomAbs_Cylinder`); `curveType` is 0 (line) for a 2-point edge, else 1.
   * Numeric truth for real curved geometry stays pinned against the OCCT worker
   * in `src-tauri/tests/wire_contract.rs`.
   *
   * The hash fallback survives for a pick this mesh cannot resolve — an
   * `elementId`-only lookup (nothing maps a minted id back to a face here) or an
   * id from a body with no synthesized mesh.
   */
  async elementInfo(
    bodyId: string,
    elementId: string,
    topoKey?: string,
  ): Promise<ElementInfo | null> {
    await wait(MESH_LATENCY_MS);
    const key = topoKey ?? elementId;
    if (!key) return null;
    const h = mockElementHash(`${bodyId}#${key}`);
    const at = (i: number) => parseInt(h.slice(i, i + 2), 16); // 0..255
    const isEdge = key.startsWith("e:");
    const base = {
      elementId: elementId || `el_${h}`,
      topoKey: key,
      bodyId,
      kind: isEdge ? "edge" : "face",
    };

    const blob = mockBodyMesh(bodyId);
    if (isEdge) {
      const edge = edgeMetricsFromMesh(blob, key);
      if (edge) {
        return {
          ...base,
          surfaceType: -1,
          curveType: edge.straight ? 0 : 1,
          center: edge.center,
          normal: [0, 0, 1],
          hasNormal: false,
          size: edge.size,
          magnitude: edge.length,
        };
      }
    } else {
      const face = faceMetricsFromMesh(blob, key);
      if (face) {
        return {
          ...base,
          // 0 == GeomAbs_Plane. A curved face must NOT claim to be one: the
          // measure tool's angle reading is gated on this, and a lying flag would
          // let it take the "angle between planes" of a cylinder wall.
          surfaceType: face.planar ? 0 : 1,
          curveType: -1,
          center: face.center,
          normal: face.normal,
          hasNormal: true,
          size: face.size,
          magnitude: face.area,
        };
      }
    }

    // Fallback: no mesh entry for this key (an elementId-only lookup, or a body
    // with no synthesized geometry). Deterministic + dimensionally plausible.
    return {
      ...base,
      surfaceType: isEdge ? -1 : 0,
      curveType: isEdge ? 0 : -1,
      center: [at(0) / 8 - 16, at(2) / 8 - 16, at(4) / 8],
      normal: [0, 0, 1],
      hasNormal: !isEdge,
      size: 10 + at(6) / 8,
      // An edge reads as a LENGTH (tens of mm), a face as an AREA (hundreds of
      // mm²) — so the two label forms are visibly distinguishable in the UI.
      magnitude: isEdge ? 10 + at(4) / 4 : 200 + at(2) * 4,
    };
  },

  /**
   * The mock's body mass properties, computed EXACTLY from the body's MESH1
   * bytes (WP-C1) — see `mockMeshMetrics.massPropertiesFromMesh`.
   *
   * Not synthesized: for a closed triangle mesh the divergence theorem gives the
   * enclosed volume, centroid and inertia exactly, so the mock lane's reading is
   * a true measurement of the box it is drawing (144000 mm³ / 18000 mm² for the
   * 80×60×30 seed). What it is NOT is a kernel reading of a curved solid — the
   * mock's "cylinder" is a 24-gon prism and answers as one.
   *
   * REJECTS an unknown body, mirroring `api::query_mass_properties`: the real
   * lane answers `REF_UNRESOLVED` there, and a mock that returned zeros would
   * make a UI bug (rendering "0 mm³") pass e2e.
   */
  async massProperties(bodyId: string): Promise<MassProperties> {
    await wait(MESH_LATENCY_MS);
    const known =
      syntheticBodies.has(bodyId) || documentStore.getState().bodies[bodyId] !== undefined;
    if (!known) throw new Error(`massProperties: unknown body ${bodyId}`);
    return massPropertiesFromMesh(bodyId, mockBodyMesh(bodyId));
  },

  // Deterministic mock promotion (Invariant 1: same pick → same id).
  async promoteSelection(bodyId: string, picks: PromotePick[]): Promise<PromotedElement[]> {
    await wait(MESH_LATENCY_MS);
    return picks.map((p) => ({
      topoKey: p.topoKey,
      elementId: `el_${mockElementHash(`${bodyId}#${p.topoKey}`)}`,
      kind: p.topoKey.startsWith("e:") ? "edge" : "face",
      bodyId,
    }));
  },

  /**
   * `PrepareOffsetFace` (SCHEMA §7.6), mock lane.
   *
   * MOCK LIMIT — deliberately the IDENTITY closure. The real verb runs
   * `BRepLib::ContinuityOfFaces` BFS over the head shape, hunts the `Total`
   * opposite by footprint coverage + material-column validation, and measures the
   * cylinder it is asked about. None of that exists without OCCT, and a
   * hand-rolled approximation would be a SECOND, wrong closure that the mock lane
   * would then certify as green. So: the operative set is exactly the picks
   * (`picked: true`, no chain), `currentDims` is a fixed pair, and there is no
   * opposite-face candidate at all.
   *
   * What IS real here is the REFUSAL that the tool's arm gate depends on:
   * `crossBody` when the picks span bodies. That is the one rule the frontend can
   * evaluate honestly, and it is the one the arm path fails closed on.
   *
   * Geometry is pinned against the kernel by `worker/tests/test_offsetface.cpp`
   * and `src-tauri/tests/offset_face.rs`; what the mock owns is the UI chain.
   */
  async prepareOffsetFace(req: PrepareOffsetFaceRequest): Promise<PrepareOffsetFaceResult> {
    await wait(MESH_LATENCY_MS);
    const snapshotId = req.snapshotId ?? mockRevision;
    const bodies = [...new Set(req.pickedFaces.map((p) => p.bodyId ?? ""))];
    if (bodies.length > 1) {
      return {
        snapshotId,
        targetBodyId: "",
        faces: [],
        currentDims: {},
        refusal: {
          code: "crossBody",
          message: "Offset face: every selected face must belong to the same body",
          faces: req.pickedFaces.map((p) => p.topoKey ?? p.elementId ?? ""),
        },
      };
    }
    return {
      snapshotId,
      targetBodyId: bodies[0] ?? "",
      faces: req.pickedFaces.map((p) => ({
        topoKey: p.topoKey ?? p.elementId ?? "",
        picked: true,
      })),
      // MOCK LIMIT: fixed seeds, not a measurement. Enough for the chip to open at
      // a number for the absolute distance types; never claimed to be the body's.
      currentDims: { radius: 10, thickness: 10 },
      refusal: null,
    };
  },

  // ── Topology repair (M4b) ──────────────────────────────────────────────────
  onNeedsRepair(cb: (e: NeedsRepairEvent) => void): Unsubscribe {
    needsRepairListeners.add(cb);
    return () => needsRepairListeners.delete(cb);
  },

  // ── Close/quit confirmation ─────────────────────────────────────────────────
  // The mock has no native window/process — nothing ever asks to close except
  // the in-app "close" intent, which never routes through the backend at all
  // (see appStore.requestClose). These exist to satisfy the CadClient contract.
  onCloseRequested(_cb: () => void): Unsubscribe {
    return () => {};
  },
  async confirmExit(): Promise<void> {},
  async cancelExit(): Promise<void> {},
  async resolveRefs(refs: ResolveRefRequest[]): Promise<ResolveRefResult[]> {
    await wait(MESH_LATENCY_MS);
    return mockResolveRefs(refs);
  },
  applyEditCommand(command: WireEditCommand): Promise<ApplyOperationResult> {
    documentStore.getState().regenStarted();
    return mockApplyEditCommand(command)
      .then(withCursor)
      .finally(() => documentStore.getState().regenSettled());
  },

  // No worker, so no crash circuit and nothing to forget — but the affordance
  // must still resolve so the mock-lane UI can exercise the button.
  async clearWorkerCircuit(): Promise<number> {
    await wait();
    return 0;
  },

  async getOperationParams(recordId: string): Promise<Record<string, unknown>> {
    await wait();
    const params = featureParams.get(recordId);
    if (!params) throw new Error(`get_operation_params: unknown record ${recordId}`);
    // Deep clone so a caller's deep-merge never mutates the stored params.
    return JSON.parse(JSON.stringify(params)) as Record<string, unknown>;
  },

  async featureDependencies(featureId: string): Promise<FeatureDependencies> {
    await wait();
    if (!mockFeatures.some((f) => f.id === featureId)) {
      throw new Error(`feature_dependencies: unknown record ${featureId}`);
    }
    return mockFeatureDependencies(featureId);
  },

  // ── Model operations (SCHEMA §7.3) — the mock's local document model ───────

  applyOperation(op: OperationOp): Promise<ApplyOperationResult> {
    return commitAndEmit(op);
  },

  async canFoldTransform(bodyId: string): Promise<string | null> {
    await wait();
    return mockCanFoldTransform(bodyId);
  },

  // IN-EDITOR lane: append an import to the OPEN document. Rust owns the dialog on
  // the real client, so there is nothing to cancel here — the mock always "picks"
  // a file and therefore never resolves null.
  async insertStep(): Promise<ApplyOperationResult | null> {
    await wait();
    return withCursor(importStepAndEmit());
  },

  async undo(): Promise<ApplyOperationResult> {
    await wait();
    if (undoStack.length === 0) return withCursor(noopResult());
    const preOp = undoStack.pop()!;
    redoStack.push(snap(preOp.label));
    const { changed, removed } = restoreSnap(preOp);
    const res: ApplyOperationResult = {
      revision: mockRevision,
      changedBodies: changed.map(bodyRef),
      removedBodies: removed,
      features: mockFeatures.map(cloneFeature),
      opLabel: preOp.label,
    };
    emitMockDocumentChanged({ revision: res.revision, changedBodies: res.changedBodies, removedBodies: res.removedBodies });
    return withCursor(res);
  },

  async redo(): Promise<ApplyOperationResult> {
    await wait();
    if (redoStack.length === 0) return withCursor(noopResult());
    const postOp = redoStack.pop()!;
    undoStack.push(snap(postOp.label));
    const { changed, removed } = restoreSnap(postOp);
    const res: ApplyOperationResult = {
      revision: mockRevision,
      changedBodies: changed.map(bodyRef),
      removedBodies: removed,
      features: mockFeatures.map(cloneFeature),
      opLabel: postOp.label,
    };
    emitMockDocumentChanged({ revision: res.revision, changedBodies: res.changedBodies, removedBodies: res.removedBodies });
    return withCursor(res);
  },

  // ── Sketch solver lane + two-level preview (shared local lane) ─────────────

  enterSketch: enterSketchWithHydration,
  sketchUpsert: lane.sketchUpsert,
  setSketchDimension: mockSetSketchDimension,
  finishSketch: lane.finishSketch,
  // Pure read for the always-visible sketch layer (no session opened). A live lane
  // session wins; else a persisted document sketch returns the deterministic seeded
  // rectangle (parity with the real client reading stored entities); unknown → reject.
  async getSketch(sketchId: string): Promise<SketchSession> {
    await wait(MESH_LATENCY_MS);
    const peeked = lane.peekSession(sketchId);
    if (peeked) return peeked;
    if (documentStore.getState().sketches[sketchId]) {
      const entities = seededSketchRectangle();
      const { dof, status } = solveDof(entities, []);
      return { sketchId, plane: planeFor("XY"), entities, constraints: [], dof, status, conflicting: [] };
    }
    throw new Error(`getSketch: unknown sketch ${sketchId}`);
  },
  async getSketchRegions(sketchId: string) {
    await wait(MESH_LATENCY_MS);
    const session = lane.peekSession(sketchId);
    if (session) {
      const regions = detectRegions(session.entities);
      lane.cacheSketchPlane(sketchId, session.plane);
      lane.cacheFinishedRegions(sketchId, regions);
      return { regions };
    }
    if (documentStore.getState().sketches[sketchId]) {
      const plane = planeFor("XY");
      const regions = detectRegions(seededSketchRectangle());
      lane.cacheSketchPlane(sketchId, plane);
      lane.cacheFinishedRegions(sketchId, regions);
      return { regions };
    }
    throw new Error(`getSketchRegions: unknown sketch ${sketchId}`);
  },
  cancelSketch: lane.cancelSketch,
  // Compensation: drop the document row + the local lane session so a re-enter of
  // the same id opens a fresh empty session (no longer a document sketch, so the
  // hydration seed no longer fires).
  async deleteSketch(id: string): Promise<void> {
    documentStore.getState().removeSketch(id);
    lane.dropSession(id);
    // A deleted sketch no longer references its host datum — otherwise the
    // orphan-cleanup path (SketchController leaving mid-enter) would leave a
    // phantom blocker that makes the datum permanently undeletable.
    mockAttachSketchToDatum(id, null);
  },
  reattachSketch: mockReattachSketch,
  beginGesture: lane.beginGesture,
  solveDrag: lane.solveDrag,
  endGesture: lane.endGesture,
  beginPreview: lane.beginPreview,
  updatePreview: lane.updatePreview,
  endPreview: lane.endPreview,
  onPreviewResult: lane.onPreviewResult,
};

function basename(path: string): string {
  const file = path.split(/[\\/]/).pop() ?? path;
  return file.replace(/\.[^.]+$/, "");
}

/** Short mirror-plane label from a world plane normal (mock feature valueText). */
function planeLabelForNormal(n: [number, number, number]): string {
  const [x, y, z] = n.map((c) => Math.abs(c));
  if (z > x && z > y) return "XY";
  if (y > x && y > z) return "XZ";
  if (x > y && x > z) return "YZ";
  return "";
}
