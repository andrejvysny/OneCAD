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
  EnterSketchTarget,
  FeatureRecord,
  Lod,
  NeedsRepairEvent,
  OperationOp,
  PromotedElement,
  SketchPlane,
  PromotePick,
  RecentProject,
  RecoveryInfo,
  ResolveCandidate,
  ResolveRefRequest,
  ResolveRefResult,
  SketchEntity,
  SketchSession,
  Unsubscribe,
  WorkerStatus,
} from "./types";
import type { WireEditCommand } from "./tauriCommandMap";
import { wireParamsOf } from "./tauriCommandMap";
import { concatMesh1, makeBoxMesh, makeCylinderMesh, makeExtrudeBodyMesh, makeRevolveBodyMesh } from "./mockMeshes";
import type { LatheAxis } from "@/tools/preview/lathePreview";
import { createLocalSolverLane } from "./localSolver";
import { detectRegions, planeFor, solveDof } from "./mockSketch";
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

// Varied names + dates (unsorted on purpose — the UI sorts).
const RECENTS: RecentProject[] = [
  {
    id: "p-bracket",
    name: "Bracket v2",
    path: "/Users/andrej/CAD/Projects/Bracket v2.onecad",
    modifiedAt: "2026-07-16T14:20:00Z",
  },
  {
    id: "p-enclosure",
    name: "Enclosure rev C",
    path: "/Users/andrej/CAD/Projects/Enclosure rev C.onecad",
    modifiedAt: "2026-07-14T09:05:00Z",
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
  { id: "f2", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "83.3 mm", status: "ok" },
  { id: "f3", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" },
  { id: "f4", kind: "sketch", opType: "Sketch", label: "Sketch 2", valueText: "", status: "ok" },
  { id: "f5", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "12.0 mm", status: "ok" },
];

const cloneFeature = (f: FeatureRecord): FeatureRecord => ({ ...f });

/** Synthetic body meshes by bodyId (seed body1 is a fallback box, not stored). */
const syntheticBodies = new Map<string, ArrayBuffer>();
let mockFeatures: FeatureRecord[] = MOCK_BASE_FEATURES.map(cloneFeature);
let mockRevision = 5; // matches the seed projection revision
let nextBodySeq = 2; // body1 is the seed body
let nextFeatureSeq = 100;

/** featureId → bodyId, so a parametric edit rebuilds the SAME body. */
const featureBodies = new Map<string, string>();

/** featureId → last committed wire params (the `get_operation_params` source). */
const featureParams = new Map<string, Record<string, unknown>>();

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
  documentStore.getState().applyChange({ datums: { ...s.datums } });
  mockRevision += 1;
  return diffBodies(before, syntheticBodies);
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
      mockFeatures = [...mockFeatures, { id: featureId, kind: "extrude", opType: "Extrude", label, valueText, status: "ok" }];
      return { changed: [target], removed: [], label, featureId };
    }
    const editing = op.featureId !== undefined && featureBodies.has(op.featureId);
    const featureId = op.featureId ?? nextFeatureId();
    const bodyId = editing ? featureBodies.get(featureId)! : nextBodyId();
    syntheticBodies.set(bodyId, makeExtrudeBodyMesh(profile, plane, distance));
    featureBodies.set(featureId, bodyId);
    const valueText = `${Math.abs(distance).toFixed(1)} mm`;
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText } : f));
    } else {
      mockFeatures = [...mockFeatures, { id: featureId, kind: "extrude", opType: "Extrude", label: "Extrude", valueText, status: "ok" }];
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
      mockFeatures = [...mockFeatures, { id: featureId, kind: "revolve", opType: "Revolve", label, valueText, status: "ok" }];
      return { changed: [target], removed: [], label, featureId };
    }
    const editing = op.featureId !== undefined && featureBodies.has(op.featureId);
    const featureId = op.featureId ?? nextFeatureId();
    const bodyId = editing ? featureBodies.get(featureId)! : nextBodyId();
    syntheticBodies.set(bodyId, makeRevolveBodyMesh(profile.ring, axis, plane, angle));
    featureBodies.set(featureId, bodyId);
    const valueText = `${Math.round(Math.abs(angle))}°`;
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText } : f));
    } else {
      mockFeatures = [...mockFeatures, { id: featureId, kind: "revolve", opType: "Revolve", label: "Revolve", valueText, status: "ok" }];
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
    const valueText = `${op.params.radius.toFixed(1)} mm`;
    const editing = op.featureId !== undefined && mockFeatures.some((f) => f.id === featureId);
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText } : f));
    } else {
      mockFeatures = [...mockFeatures, { id: featureId, kind: "fillet", opType: op.opType, label, valueText, status: "ok" }];
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
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText } : f));
    } else {
      // `kind` mirrors the REAL projection bucket (`dto.rs feature_kind` folds
      // Shell → fillet and the pattern/mirror ops → boolean); `opType` carries the
      // authored identity every re-edit routes on. Emitting the invented kinds the
      // mock used to emit made the mock lane green while the Tauri lane was dead.
      mockFeatures = [...mockFeatures, { id: featureId, kind: "fillet", opType: "Shell", label: "Shell", valueText, status: "ok" }];
    }
    return { changed: [bodyId], removed: [], label: "Shell", featureId };
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
  undoStack.push(snap(labelForOp(op)));
  redoStack.length = 0;
  const { changed, removed, label, featureId } = mutateOp(op);
  // Remember the committed wire params so `getOperationParams` can serve a re-edit
  // its non-scalar inputs (axis / openFaces / edges) verbatim.
  featureParams.set(featureId, wireParamsOf(op));
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
  return wait().then(() => {
    const res = commitOp(op);
    emitMockDocumentChanged({
      revision: res.revision,
      changedBodies: res.changedBodies,
      removedBodies: res.removedBodies,
    });
    return res;
  });
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

/** The feature-chip value text for a re-edited op's wire params (mirrors mutateOp). */
function valueTextForFeature(
  kind: FeatureRecord["kind"],
  params: Record<string, unknown>,
): string | undefined {
  switch (kind) {
    case "extrude": {
      const d = scalarValue(params.distance);
      return d === undefined ? undefined : `${Math.abs(d).toFixed(1)} mm`;
    }
    case "revolve": {
      const a = scalarValue(params.angleDeg);
      return a === undefined ? undefined : `${Math.round(Math.abs(a))}°`;
    }
    case "fillet": {
      const r = scalarValue(params.radius);
      return r === undefined ? undefined : `${r.toFixed(1)} mm`;
    }
    case "shell": {
      const t = scalarValue(params.thickness);
      return t === undefined ? undefined : `${t.toFixed(1)} mm`;
    }
    case "linearPattern":
    case "circularPattern":
      return typeof params.count === "number" ? `×${params.count}` : undefined;
    default:
      return undefined;
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
      mockFeatures = mockFeatures.filter((f) => f.id !== command.record);
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
      if (feat) {
        undoStack.push(snap("Update"));
        redoStack.length = 0;
        featureParams.set(command.record, { ...(featureParams.get(command.record) ?? {}), ...params });
        const valueText = valueTextForFeature(feat.kind, params);
        // A Boolean re-edit swaps ONLY the operation, and a boolean carries no
        // dimension (`valueTextForFeature` returns undefined, matching dto.rs
        // `feature_value_text`) — but the mock LABELS a boolean row by its
        // operation (mutateOp), so the label is what has to follow the swap.
        const label =
          feat.opType === "Boolean" && typeof params.operation === "string" ? params.operation : undefined;
        if (valueText !== undefined || label !== undefined) {
          mockFeatures = mockFeatures.map((f) =>
            f.id === command.record
              ? { ...f, ...(valueText !== undefined ? { valueText } : {}), ...(label !== undefined ? { label } : {}) }
              : f,
          );
        }
      }
      mockRevision += 1;
      const bodyId = featureBodies.get(command.record);
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
    case "setRollback":
    default:
      // Rollback carries no distinct projection signal in the lean mock. Return a
      // valid no-op result.
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

/** Test seam: forget all sketch state so a fresh sketch starts empty. */
export function resetMockSketches(): void {
  lane.resetSketches();
}

/** Test seam: forget the whole mock document (bodies, features, undo, sessions). */
export function resetMockDocument(): void {
  syntheticBodies.clear();
  featureBodies.clear();
  featureParams.clear();
  lane.resetPreview();
  mockFeatures = MOCK_BASE_FEATURES.map(cloneFeature);
  mockRevision = 5;
  nextBodySeq = 2;
  nextFeatureSeq = 100;
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
  async importStep(path) {
    await wait();
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

  // Save/export are Rust-owned in the real app; the mock keeps them deterministic
  // (no filesystem): saveDocument is a no-op, Save As / Export return fake paths.
  async saveDocument(_path?: string) {
    await wait(40);
  },
  async saveDocumentAs() {
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
    // Synthesized bodies (extrude output) win; else the seed box/cylinder.
    return syntheticBodies.get(bodyId) ?? meshForBody(bodyId);
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
   * MOCK LIMIT: no kernel to query, so the frame is synthesized from the mock
   * body's world position rather than a real face descriptor — a +Z-facing plane
   * at the picked point. It applies the SAME in-plane axis rule as the backend
   * (`plane_from_point_normal`: a +Z normal falls back to the +X seed and lands
   * on the named XY basis), so the mock lane and the real lane agree on
   * orientation even though they disagree on which face was picked.
   */
  async faceSketchPlane(_bodyId: string, _elementId: string): Promise<SketchPlane> {
    await wait(MESH_LATENCY_MS);
    return { ...planeFor("XY"), kind: "custom" };
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
    return mockApplyEditCommand(command);
  },

  async getOperationParams(recordId: string): Promise<Record<string, unknown>> {
    await wait();
    const params = featureParams.get(recordId);
    if (!params) throw new Error(`get_operation_params: unknown record ${recordId}`);
    // Deep clone so a caller's deep-merge never mutates the stored params.
    return JSON.parse(JSON.stringify(params)) as Record<string, unknown>;
  },

  // ── Model operations (SCHEMA §7.3) — the mock's local document model ───────

  applyOperation(op: OperationOp): Promise<ApplyOperationResult> {
    return commitAndEmit(op);
  },

  async undo(): Promise<ApplyOperationResult> {
    await wait();
    if (undoStack.length === 0) return noopResult();
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
    return res;
  },

  async redo(): Promise<ApplyOperationResult> {
    await wait();
    if (redoStack.length === 0) return noopResult();
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
    return res;
  },

  // ── Sketch solver lane + two-level preview (shared local lane) ─────────────

  enterSketch: enterSketchWithHydration,
  sketchUpsert: lane.sketchUpsert,
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
  beginGesture: lane.beginGesture,
  solveDrag: lane.solveDrag,
  endGesture: lane.endGesture,
  beginPreview: lane.beginPreview,
  updatePreview: lane.updatePreview,
  endPreview: lane.endPreview,
  onPreviewResult: lane.onPreviewResult,
};

/** Small deterministic hash for mock ElementIds (FNV-1a-32 hex). */
function mockElementHash(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(16).padStart(8, "0");
}

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
