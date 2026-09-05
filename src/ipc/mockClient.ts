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
  ClassifyResult,
  ComponentParamValue,
  ComponentUpgrade,
  NewComponentSpec,
  ProjectTemplate,
  ReplaceComponentResult,
  VariableEditResult,
  DocumentChange,
  DocumentProjectionWire,
  DocumentVariable,
  DocumentSnapshot,
  ElementInfo,
  IngestComponentsRequest,
  IngestComponentsReport,
  LibraryComponent,
  PlaceComponentSource,
  MassProperties,
  EnterSketchTarget,
  FeatureDependencies,
  FeatureRecord,
  Lod,
  NeedsRepairEvent,
  OperationOp,
  PrepareOffsetFaceRequest,
  PrepareOffsetFaceResult,
  PrepareEdgeOpRequest,
  PrepareEdgeOpResult,
  AnalyzeEdgeOpRangeRequest,
  AnalyzeEdgeOpRangeResult,
  ProjectToSketchRequest,
  ProjectToSketchResult,
  ProjectedEntity,
  ProjectedSource,
  ProjectionRefusal,
  DetachProjectionResult,
  PromotedElement,
  SketchAttachTarget,
  SketchPlane,
  PromotePick,
  RecentProject,
  RecoveryInfo,
  PlaceComponentMate,
  RegenTerminal,
  ReindexReport,
  SaveOutcome,
  ResolveCandidate,
  ResolveRefRequest,
  ResolveRefResult,
  Rgba,
  SketchConstraint,
  SketchConstraintType,
  SketchEntity,
  SketchSession,
  SketchUpsertResult,
  TransformBodyParams,
  TransformRotationParams,
  Unsubscribe,
  WorkerStatus,
} from "./types";
import type { EvaluatedExpression, ExpressionDimension } from "./types";
import { IMPORT_STEP_OP_TYPE, VARIABLE_NAME_RE } from "./types";
import type { ResolvedTable } from "./expr";
import { evaluateAtSite, renameReference, resolveVariableTable } from "./expr";
import { BackendError } from "./apiError";
import type { WireEditCommand } from "./tauriCommandMap";
import { bareBodyId, parseRefId, wireParamsOf } from "./tauriCommandMap";
import { MATE_AXIS_FOLLOW_LABEL, MATE_AXIS_KEEP_LABEL } from "./operationDiagnostics";
import { holeValueText } from "@/tools/modelTools/holeMachine";
import { offsetFaceValue } from "@/tools/preview/faceOffset";
import type { FaceColor } from "./mockMeshes";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { TopoIndex } from "@/viewport/mesh/faceRangeIndex";
import {
  BOX_EDGE_PAIRS,
  BOX_SIZE,
  boxCorners,
  concatMesh1,
  makeBoredCylinderMesh,
  makeBoxMesh,
  makeCylinderMesh,
  makeExtrudeBodyMesh,
  makeRevolveBodyMesh,
  placeComponentGhostMesh,
  transformMesh1,
  type BoxCornerKey,
} from "./mockMeshes";
import { placementMatrix } from "@/tools/preview/patternPreview";
import type { LatheAxis } from "@/tools/preview/lathePreview";
import { createLocalSolverLane } from "./localSolver";
import {
  lookupMockFace,
  mockAdjacentFaces,
  mockElementHash,
  mockProjectedContent,
  worldToPlaneUv,
  type MockFaceGeometry,
} from "./mockFaceGeometry";
import { frontendConstraintsFromDto, frontendEntitiesFromDto } from "./sketchWireMap";
import {
  cylinderMetricsFromMesh,
  edgeMetricsFromMesh,
  faceMetricsFromMesh,
  massPropertiesFromMesh,
} from "./mockMeshMetrics";
import { planeFor, solveDof } from "./mockSketch";
import { detectRegions } from "./mockRegions";
import type { BodyMeta, DatumMeta } from "@/stores/documentStore";
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
const MOCK_BASE_RECENTS: RecentProject[] = [
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

/** Mutable working copy `rename`/`delete` operate on — see `resetMockRecents`. */
let RECENTS: RecentProject[] = MOCK_BASE_RECENTS.map((p) => ({ ...p }));

/** Test seam: restore the recents list to its seeded state (undoes a mock
 *  rename/delete from a prior test). */
export function resetMockRecents(): void {
  RECENTS = MOCK_BASE_RECENTS.map((p) => ({ ...p }));
}

// ── Crash recovery (start screen) — test-seeded seam ────────────────────────
//
// DEFAULT EMPTY so the start screen shows no recovery banner unless a test opts in
// (existing StartScreen/App tests stay green). `setMockRecovery` seeds/clears them;
// `checkRecovery` reports them newest-first, mirroring Rust; `recoverDocument`
// accepts (restore) or discards ONE by documentId.
//
// `?mockrecovery=1` opts a fixture in from a plain browser lane — the same dev-only
// URL-flag pattern `?mockimport=step` and `?mocklibrary=1` use. A browser has no
// crashed session to discover, so without it the entire recovery flow is
// unreachable from e2e. The fixture's `originalPath` is deliberately the FIRST
// seeded recent, so the "open a file an autosave shadows" conflict is reachable
// too.
let mockRecovery: RecoveryInfo[] = [];

/** The `?mockrecovery=1` fixture: one offer shadowing the first seeded recent. */
function mockRecoveryFixture(): RecoveryInfo[] {
  const shadowed = MOCK_BASE_RECENTS[0];
  return [
    {
      documentId: "9f3b7c1e-0000-4000-8000-000000000001",
      title: shadowed?.name ?? "Untitled document",
      originalPath: shadowed?.path,
      autosavePath: "/mock/autosave/9f3b7c1e-0000-4000-8000-000000000001.onecad",
      modifiedMs: Date.now() - 90_000,
    },
  ];
}

/** In-memory module slices, keyed by module id (the mock's `document.modules`). */
const mockModuleState = new Map<string, { schemaVersion: number; payload: unknown }>();

/*
 * The mock's document variable table. ORDERED (declaration order is
 * authoritative, mirroring Rust's `VariableTable`) rather than a Map keyed by
 * name, so a re-value keeps its position exactly as the real backend does.
 *
 * The validation below MIRRORS `api::upsert_variable_command` /
 * `api::upsert_variable_expr_command` / `session::rename_variable`, deliberately
 * including the messages' shape: the mock lane is where the Variables section's
 * error paths are proved, and a mock that accepted what the backend refuses
 * would make those tests vacuous. Every expression is resolved by the SHARED TS
 * port of the engine (`ipc/expr`), never by a second local approximation.
 */

/** The mock's STORED row, before resolution — `{value, expr?}` is the wire
 *  `Scalar` a variable holds. `listVariables` projects it plus the resolved
 *  half, exactly as `dto.rs variable_dtos` does. */
interface MockVariable {
  id: string;
  name: string;
  value: number;
  expr?: string;
}

let mockVariables: MockVariable[] = [];
let nextVariableSeq = 1;

/** `DocumentVariable` rows as the backend projects them: the stored numbers,
 *  plus what the WHOLE table resolves to right now. MIRRORS `variable_dtos`. */
function mockVariableRows(rows: readonly MockVariable[] = mockVariables): DocumentVariable[] {
  const table = resolveVariableTable(rows);
  return rows.map((v) => {
    const resolved = table.values.get(v.name);
    const failure = table.errors.find((e) => e.name === v.name);
    return {
      id: v.id,
      name: v.name,
      value: v.value,
      expr: v.expr,
      // A broken variable keeps the last number anybody could justify; `error`
      // is what tells the UI not to trust it.
      resolvedValue: resolved?.number ?? v.value,
      dimension: resolved?.dim ?? "scalar",
      error: failure?.message,
    };
  });
}

/** The name grammar, refused before anything is stored. MIRRORS
 *  `regen::variables::is_bare_name`. */
function mockCheckVariableName(name: string): string {
  const trimmed = name.trim();
  if (!VARIABLE_NAME_RE.test(trimmed)) {
    throw new Error(
      `invalid variable name "${trimmed}": a name must match [A-Za-z_][A-Za-z0-9_]*`,
    );
  }
  return trimmed;
}

/** Sets a LITERAL value. An existing expression on that variable is REPLACED
 *  (Rust's `Scalar::try_new` carries no `expr`), which is how a user goes from
 *  `=w*2` back to a plain number. */
function mockUpsertVariable(name: string, value: number): DocumentVariable[] {
  const trimmed = mockCheckVariableName(name);
  if (!Number.isFinite(value)) throw new Error("variable value must be finite");
  const existing = mockVariables.find((v) => v.name === trimmed);
  if (existing) {
    existing.value = value;
    existing.expr = undefined;
  } else {
    mockVariables.push({ id: `var${nextVariableSeq++}`, name: trimmed, value });
  }
  return mockVariableRows();
}

/**
 * Binds a variable to an EXPRESSION. MIRRORS
 * `api::upsert_variable_expr_command`: the stored `Scalar` carries BOTH the
 * text and the number it currently evaluates to, and the evaluation runs
 * against the table this edit WOULD produce — so a self-reference (`w = w + 1`)
 * and a cycle the edit closes are refused HERE rather than written and then
 * discovered by every bound parameter at once.
 */
function mockUpsertVariableExpr(name: string, text: string): DocumentVariable[] {
  const trimmed = mockCheckVariableName(name);
  const expr = text.trim();
  if (expr === "") throw new Error(`variable "${trimmed}": an expression must not be empty`);
  const existing = mockVariables.find((v) => v.name === trimmed);
  const candidate: MockVariable[] = existing
    ? mockVariables.map((v) => (v.name === trimmed ? { ...v, value: 0, expr } : v))
    : [...mockVariables, { id: "", name: trimmed, value: 0, expr }];
  const table = resolveVariableTable(candidate);
  const failure = table.errors.find((e) => e.name === trimmed);
  if (failure) throw new Error(`variable "${trimmed}": ${failure.message}`);
  const resolved = table.values.get(trimmed);
  if (resolved === undefined) throw new Error(`variable "${trimmed}" resolved to nothing`);
  if (existing) {
    existing.value = resolved.number;
    existing.expr = expr;
  } else {
    mockVariables.push({ id: `var${nextVariableSeq++}`, name: trimmed, value: resolved.number, expr });
  }
  return mockVariableRows();
}

/**
 * Renames a variable AND every expression that references it — the table's own
 * rows and every timeline scalar the mock holds. MIRRORS
 * `session::rename_variable`.
 *
 * Refused as a WHOLE if any expression cannot be rewritten: a half-renamed
 * document is a silently wrong document. Every refusal happens BEFORE the first
 * write, so a rejection leaves nothing behind.
 */
function mockRenameVariable(name: string, newName: string): DocumentVariable[] {
  const target = mockVariables.find((v) => v.name === name);
  if (!target) throw new Error(`unknown variable "${name}"`);
  const next = newName.trim();
  if (next === name) return mockVariableRows();
  mockCheckVariableName(next);
  if (mockVariables.some((v) => v.name === next)) {
    throw new Error(`duplicate variable name ${next}`);
  }

  // Rewrite into staged copies first; nothing is committed until every site
  // parsed. `site` names WHERE an unparseable expression lives, so the refusal
  // points at it.
  const rewrite = (text: string, site: string): string | null => {
    try {
      return renameReference(text, name, next);
    } catch (e) {
      throw new Error(
        `cannot rename \`${name}\`: ${site} carries an expression that does not parse ` +
          `(${e instanceof Error ? e.message : String(e)})`,
      );
    }
  };
  const nextVars = mockVariables.map((v) => {
    const renamed = v.name === name ? next : v.name;
    const expr = v.expr === undefined ? undefined : (rewrite(v.expr, `variable \`${v.name}\``) ?? v.expr);
    return { ...v, name: renamed, expr };
  });
  const paramPatches: [string, Record<string, unknown>][] = [];
  for (const [recordId, params] of featureParams) {
    let changed = false;
    const patched: Record<string, unknown> = { ...params };
    for (const [key, raw] of Object.entries(params)) {
      const expr = scalarExpr(raw);
      if (expr === undefined) continue;
      const rewritten = rewrite(expr, `${recordId}.${key}`);
      if (rewritten === null) continue;
      patched[key] = { ...(raw as Record<string, unknown>), expr: rewritten };
      changed = true;
    }
    if (changed) paramPatches.push([recordId, patched]);
  }

  mockVariables = nextVars;
  for (const [recordId, patched] of paramPatches) featureParams.set(recordId, patched);
  // Restamp the affected rows off their new params. The real backend rewrites
  // the RECORD and the projection derives `primaryExpr` from it, so a bound row
  // reads as the new name the moment the rename lands; a mock that patched only
  // the params would leave the row claiming a binding the table no longer has.
  const patchedIds = new Set(paramPatches.map(([recordId]) => recordId));
  if (patchedIds.size > 0) {
    mockFeatures = mockFeatures.map((f) => {
      const params = patchedIds.has(f.id) ? featureParams.get(f.id) : undefined;
      if (params === undefined) return f;
      const value = featureValueForParams(f.opType, f.kind, params);
      return value.valueText === undefined ? f : { ...f, ...value };
    });
  }
  return mockVariableRows();
}

function mockRemoveVariable(name: string): DocumentVariable[] {
  const at = mockVariables.findIndex((v) => v.name === name);
  if (at < 0) throw new Error(`unknown variable "${name}"`);
  mockVariables.splice(at, 1);
  return mockVariableRows();
}

/**
 * Evaluates one expression at its call site against the current table —
 * MIRRORS `regen::variables::evaluate_expression`, which is what the real
 * `evaluate_expression` command calls.
 *
 * Pure, and a failure is a populated `error` rather than a rejection: an
 * in-progress expression is not an API misuse.
 */
function mockEvaluateExpression(expr: string, site: ExpressionDimension): EvaluatedExpression {
  const result = evaluateAtSite(expr.trim(), site, resolveVariableTable(mockVariables));
  return result.ok
    ? { value: result.value.number, dimension: result.value.dim }
    : { value: 0, dimension: site, error: result.failure.message };
}

/**
 * Compose a variable write's result the way the tauri client composes its own
 * (W5): the SAVED table, plus the terminal of the regen the write scheduled.
 *
 * Both truths, always. A downstream step that can no longer resolve — or an
 * extrude the new value drives below the kernel's floor — reports
 * `terminal: "failed"` while `variables` still holds what the document holds. The
 * write is never rolled back and never turned into a rejection: a rejection would
 * say the variable was not saved, and it was.
 */
function variableEditResult(
  variables: DocumentVariable[],
  opLabel: string,
): VariableEditResult {
  const failed = mockVariableRegenFailures()[0];
  const res: ApplyOperationResult = {
    revision: mockRevision,
    changedBodies: [],
    removedBodies: [],
    features: mockFeatures.map(cloneFeature),
    opLabel,
    // No mock regen ⇒ nothing was rebuilt. `noop` is the honest success terminal
    // here (and `keepsRecord` treats it as one); a failure overrides it below.
    terminal: failed ? "failed" : "noop",
  };
  if (failed) res.errorMessage = failed.message;
  return { ...withCursor(res), variables };
}

/**
 * Test seam: seed (or clear) the crash-recovery offers the start screen checks.
 * Accepts a single offer for convenience; `null` clears.
 */
export function setMockRecovery(r: RecoveryInfo | RecoveryInfo[] | null): void {
  const list = r === null ? [] : Array.isArray(r) ? r : [r];
  // Newest first, as `scan_recoverable` orders them.
  mockRecovery = list.map((o) => ({ ...o })).sort((a, b) => b.modifiedMs - a.modifiedMs);
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

/**
 * Forced regen terminal for every subsequent mock apply, with the message a real
 * failure would carry. The mock has no regen, so it can never PRODUCE a
 * `needsRepair`/`failed`/`timeout` outcome on its own — but every consumer family
 * has to be provable against all five terminals, and the mock lane is where that
 * table-driven test runs. `null` restores the natural `published`/`noop`.
 */
let mockForcedTerminal: { terminal: RegenTerminal; errorMessage?: string } | null = null;

export function setMockRegenTerminal(
  forced: { terminal: RegenTerminal; errorMessage?: string } | null,
): void {
  mockForcedTerminal = forced;
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

// ── `?vpdemo=cyl` demo body: a bushing with a real bore ─────────────────────
//
// The seed box has six planar faces and nothing else, so the mock lane could
// only ever exercise the COINCIDENT half of the placement gesture; auto-size
// (spec §5.3's hole row) runs on a hole's RADIUS and had no hole to read. This
// body supplies one. It is opt-in for the same reason `?mocklibrary=1` is: an
// always-on second body would change what every existing spec's body list and
// camera fit see.
//
// The bore is Ø8.5 — the standard M8 clearance hole — so auto-size resolves to
// a size the armed default (M6) is not, which is what makes the e2e assertion
// mean something.

/** The `?vpdemo=cyl` body id (not `body<N>`: it must not collide with `nextBodyId`). */
export const MOCK_DEMO_BORE_BODY_ID = "body_demo_bore";
const DEMO_BORE_ORIGIN: readonly [number, number, number] = [80, 0, 0];

/**
 * Publish the demo bushing into the mock document (bodies registry + synthetic
 * mesh) and hand back its mesh ref. The caller fires the `document-changed`, so
 * ingestion still goes through the one path the real worker uses.
 */
export function seedMockDemoCylinder(): BodyMeshRef {
  const bodyId = MOCK_DEMO_BORE_BODY_ID;
  syntheticBodies.set(bodyId, makeBoredCylinderMesh(20, 4.25, 16, DEMO_BORE_ORIGIN));
  mockRevision += 1;
  const doc = documentStore.getState();
  doc.applyChange({
    revision: mockRevision,
    bodies: { ...doc.bodies, [bodyId]: { id: bodyId, name: "Bushing", visible: true } },
  });
  // The mock owns this body's metadata now, like any other body it minted —
  // otherwise the next `reassertMockMetadata` has nothing to re-assert from.
  writeMockMeta("body", bodyId, { name: "Bushing", visible: true });
  return bodyRef(bodyId);
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
  color?: Rgba;
  faceColors?: Record<string, Rgba>;
}
const mockBodyMeta = new Map<string, MockMeta>();
const mockSketchMeta = new Map<string, MockMeta>();
/** Persistent elementId → (bodyId, topoKey) for mock `elementInfo` resolution. */
const mockElementIdToTopoKey = new Map<string, { bodyId: string; topoKey: string }>();

function seedMockMetadata(): void {
  mockBodyMeta.clear();
  mockSketchMeta.clear();
  const s = documentStore.getState();
  for (const b of Object.values(s.bodies))
    mockBodyMeta.set(b.id, { visible: b.visible, color: b.color, faceColors: b.faceColors });
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
    const color = meta.color ?? row.color;
    const faceColors = meta.faceColors ?? row.faceColors;
    if (
      row.visible === meta.visible &&
      row.name === name &&
      colorsEqual(row.color, color) &&
      faceColorsEqual(row.faceColors, faceColors)
    )
      continue;
    bodies[id] = { ...row, name, visible: meta.visible };
    if (color !== undefined) bodies[id].color = color;
    else delete bodies[id].color;
    if (faceColors !== undefined && Object.keys(faceColors).length > 0) bodies[id].faceColors = faceColors;
    else delete bodies[id].faceColors;
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

function colorsEqual(
  a: Rgba | undefined,
  b: Rgba | undefined,
): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a[0] === b[0] && a[1] === b[1] && a[2] === b[2] && a[3] === b[3];
}

function faceColorsEqual(
  a: Record<string, Rgba> | undefined,
  b: Record<string, Rgba> | undefined,
): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  const ak = Object.keys(a);
  if (ak.length !== Object.keys(b).length) return false;
  for (const k of ak) if (!colorsEqual(a[k], b[k])) return false;
  return true;
}

/** Record + apply one metadata mutation (the `setVisibility`/`rename*` arms). */
function writeMockMeta(kind: "body" | "sketch", id: string, patch: Partial<MockMeta>): void {
  const registry = kind === "body" ? mockBodyMeta : mockSketchMeta;
  const s = documentStore.getState();
  const row = kind === "body" ? s.bodies[id] : s.sketches[id];
  // Adopt the row's CURRENT state on first touch, so renaming a body never also
  // resurrects it (the patch is the only thing this command means to change).
  const current =
    registry.get(id) ??
    (kind === "body"
      ? {
          visible: row?.visible ?? true,
          name: row?.name,
          color: (row as BodyMeta).color,
          faceColors: (row as BodyMeta).faceColors,
        }
      : { visible: row?.visible ?? true, name: row?.name });
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
  const forced = mockForcedTerminal;
  return {
    ...res,
    appliedOps: mockAppliedOps,
    totalOps: mockFeatures.length,
    terminal: forced?.terminal ?? res.terminal ?? "published",
    // A forced non-published terminal carries its reason exactly as the real
    // client would; `published`/`noop`/`needsRepair` carry none.
    errorMessage: forced?.errorMessage ?? res.errorMessage,
  };
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
    const valueText = edgeOpValueText(op.params.radius, op.params.distance2, op.params.angleDeg);
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
    // feature_value_text` for Hole exactly (`Ø` + one decimal, plus the WP-T1
    // designation when threaded).
    const valueText = holeValueText(op.params.diameter, op.params.thread?.designation);
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
  if (op.opType === "PlaceComponent") {
    // Ghost-preview only (see `OperationOp`'s doc comment) — its session is
    // always cancelled, never committed, so this must never actually run.
    throw new Error("PlaceComponent must not reach commitOp — preview-only op type");
  }
  if (op.opType === "Gear") {
    // MOCK LIMIT: no real involute geometry — generating it here would be a
    // second, worse gear implementation living in the mock. The GEOMETRY is
    // pinned against the real kernel by `worker/tests/test_gear_tool.cpp`,
    // `test_gear_op.cpp` and the G0 sampler oracles; what the mock lane owns is
    // the UI chain — placement pick -> chips -> committed record -> re-edit seed.
    //
    // Unlike every other op here a Gear MINTS a body (D1 `body_<opId>`) and has
    // no target, so the mock reports the id the real worker would mint rather
    // than re-emitting a host.
    const featureId = op.featureId ?? nextFeatureId();
    const bodyId = op.opId ? `body_${op.opId}` : `body_${featureId}`;
    const teeth = op.params.involuteExternal?.teeth ?? 0;
    const modulus = op.params.involuteExternal?.module ?? 0;
    // Mirrors the real projection's display naming for a gear.
    const valueText = `${teeth}T m${modulus}`;
    const editing = op.featureId !== undefined && mockFeatures.some((f) => f.id === featureId);
    if (editing) {
      mockFeatures = mockFeatures.map((f) => (f.id === featureId ? { ...f, valueText } : f));
    } else {
      mockFeatures = [...mockFeatures, { id: featureId, kind: "extrude", opType: "Gear", label: "Gear", valueText, status: "ok" }];
    }
    return { changed: [bodyId], removed: [], label: "Gear", featureId };
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
  enforceChamferReferenceFaces(op);
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
 * Mirrors core `session::CHAMFER_ANGLE_FLIP_REASON` VERBATIM (SCHEMA §7.3). Same
 * field-identity precondition as {@link CHAMFER_D2_FLIP_REASON}: a distance-angle
 * chamfer carries a field `FilletParams` has no home for, so the flip would drop
 * the user's angle silently.
 */
const CHAMFER_ANGLE_FLIP_REASON =
  "UpdateOperationParams may not change opType: a Chamfer carrying angleDeg is not flippable to Fillet (clear angleDeg first)";

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
  // Same precedence as core (`op_type_edit_allowed`): distance2 is read first,
  // then angleDeg. The two are mutually exclusive, so at most one can fire.
  if (prior === "Chamfer" && priorParams?.distance2 !== undefined) return CHAMFER_D2_FLIP_REASON;
  if (prior === "Chamfer" && priorParams?.angleDeg !== undefined) return CHAMFER_ANGLE_FLIP_REASON;
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
    terminal: "noop",
  };
}

/**
 * Mirror core's SCHEMA §7.3 refusal (WP-F): an ASYMMETRIC chamfer must name the
 * face `radius` is measured on, and an equal-leg one may not name any.
 *
 * The mock lane is not merely a stand-in here — a mock that accepted a pair-less
 * asymmetric chamfer would let the frontend store a LEGACY-shaped record that the
 * real backend rejects outright, and every spec built on it would certify a
 * commit path the Tauri lane cannot run.
 */
function enforceChamferReferenceFaces(op: OperationOp): void {
  if (op.opType !== "Chamfer") return;
  const p = op.params;
  const asymmetric = p.distance2 !== undefined || p.angleDeg !== undefined;
  const pairs = p.referenceFaces ?? [];
  if (asymmetric && pairs.length === 0) {
    throw new Error("referenceFaces required for an asymmetric chamfer (SCHEMA §7.3)");
  }
  if (!asymmetric && pairs.length > 0) {
    throw new Error("an equal-leg Chamfer has no reference face and may not carry referenceFaces");
  }
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

// ── Component Library (WP-1.5) ────────────────────────────────────────────────
//
// `listLibraryComponents()` stays an HONEST EMPTY LIST by default — WP-1.4's own
// rule (a dead catalog teaches a UI bug to pass): no components indexed, no
// placement to arm, nothing to manually verify. `?mocklibrary=1` opts a fixture
// IN, the same dev-only URL-flag pattern `?mockimport=step` uses for STEP import
// (both routers are otherwise unreachable from a plain browser lane with no
// backend to actually read a package tree). `placeComponent()` fabricates the
// SAME M6 SHCS mesh the ghost preview uses (`placeComponentGhostMesh`), so a
// committed body and its own preceding ghost agree exactly.

const MOCK_LIBRARY_FIXTURE: LibraryComponent = {
  id: "onecad.std.iso4762",
  version: "1.0.0",
  name: "Socket Head Cap Screw M6×20",
  category: ["fasteners", "screws"],
  tags: ["metric", "shcs", "iso4762"],
  sourceKind: "generator",
  revision: `sha256:${"0".repeat(64)}`,
  generatorId: "iso4762",
  generatorVersion: 1,
  attachments: {
    headSeat: { on: "face:head_underside", accepts: ["plane"] },
    shankAxis: { on: "cylinder:shank", accepts: ["cylinder", "hole", "circularEdge"] },
  },
  // WP-2.4: enough of a `[parameters]` table for the configurator to render
  // real controls in the mock/e2e lane — mirrors the real ISO 4762 package's
  // shape (`src-tauri/src/library.rs`'s test fixture), not a distinct mock-only
  // schema.
  parameters: {
    thread: { role: "free", key: "M6", domain: ["M3", "M4", "M5", "M6", "M8"] },
    length: { role: "free", value: 20, snap: "preferred", min: 4 },
    head_d: { role: "table", from: "iso4762.dk" },
    // WP-2.5: enum-domain free param, exercises `ComponentParametersSection`'s
    // dropdown rendering in the mock/e2e lane without a real component.toml.
    thread_detail: { role: "free", key: "cosmetic", domain: ["cosmetic", "simplified", "modeled"] },
  },
  // No literal "M" before `{thread}`: the stored thread VALUE is already the
  // full designation ("M6") in this codebase's convention (WP-2.1/2.2's
  // BOLTS-keyed tables), unlike the spec doc's own example.
  designation: "ISO 4762 {thread}x{length}",
};

/**
 * The whole opt-in catalog. ONE entry, because the shipped seed catalog is one
 * package (`src-tauri/src/library_seed.rs::SEED_PACKAGES`) — the nine other
 * families this list used to mirror went with their worker generators.
 *
 * MIRRORED, not invented: a mock catalog carrying a parameter shape the app
 * does not ship would make the configurator and the placement gesture provable
 * only against fiction. What is NOT mirrored is geometry — the mock has no
 * kernel (see `resolveComponentSource`'s note).
 */
const MOCK_LIBRARY_FIXTURES: LibraryComponent[] = [MOCK_LIBRARY_FIXTURE];

/**
 * The built-in starter templates, mirroring `src-tauri/src/
 * library_seed_templates.rs::SEED_TEMPLATES` (id / name / description).
 *
 * Gated behind the SAME `?mocklibrary=1` flag the component fixtures are,
 * because they are the same fact: these exist only where a seeded library ROOT
 * exists, and the mock lane has none. Without the flag `listTemplates` still
 * answers with this session's saved templates alone — which is what keeps "the
 * Templates row says how to make one when the library is empty" a real test.
 *
 * The DOCUMENTS are not mirrored (the mock has no frozen container to
 * instantiate) — `newFromTemplate` hands back a synthetic document for a
 * starter exactly as it does for a session-saved one.
 */
const MOCK_SEED_TEMPLATES: ProjectTemplate[] = [
  { id: "onecad.std.template.blank", name: "Blank", description: "An empty document in millimetres." },
  {
    id: "onecad.std.template.printed-part",
    name: "3D-Printed Part",
    description: "Millimetres, with a Build plate datum on XY to sketch the footprint on.",
  },
];

function mockLibraryEnabled(): boolean {
  return (
    typeof window !== "undefined" &&
    new URLSearchParams(window.location.search).get("mocklibrary") === "1"
  );
}

let nextComponentSeq = 1;

/**
 * Components authored through `saveAsComponent` this session (WP-B2). Kept
 * in memory only — the mock lane has no library root to write to, and this
 * exists so the authoring FLOW (menu → dialog → the panel listing the result)
 * is exercisable without a backend.
 */
let mockAuthored: LibraryComponent[] = [];

/**
 * Templates saved through `saveAsTemplate` this session (WP-B3). In memory
 * only, for the same reason `mockAuthored` is: there is no library root on this
 * lane. `newFromTemplate` therefore hands back the same synthetic document
 * `newDocument()` does — the FLOW is real, the starting geometry is not.
 */
let mockTemplates: ProjectTemplate[] = [];

/** The built-in starters, when the flag says a seeded library root exists. */
function seededTemplates(): ProjectTemplate[] {
  return mockLibraryEnabled() ? MOCK_SEED_TEMPLATES.map((t) => ({ ...t })) : [];
}

/**
 * The seed catalog or a session-authored component matching `id@version`, else
 * `null`. Authored components place with the fixture's geometry (nothing is
 * baked on this lane) but keep their own identity + attachments, which is
 * what the author→place e2e needs to exist at all.
 *
 * Deliberately NOT gated on `?mocklibrary=1`: the flag governs what the
 * catalog LISTS, and a placement already in flight must not stop resolving
 * because of a URL.
 */
function mockComponentByIdentity(id: string, version: string): LibraryComponent | null {
  return (
    [...MOCK_LIBRARY_FIXTURES, ...mockAuthored].find(
      (c) => c.id === id && c.version === version,
    ) ?? null
  );
}

/**
 * Rejects any key that is not `role = "free"` on `component` — the mock's
 * mirror of the backend's own signature check (`library.rs::check_free_params`).
 * `what` names the caller so the two commands' messages stay distinguishable.
 */
function assertFreeParams(
  component: LibraryComponent,
  params: Record<string, ComponentParamValue> | undefined,
  what = "placeComponent",
): void {
  for (const key of Object.keys(params ?? {})) {
    const spec = component.parameters[key];
    if (!spec) throw new Error(`${what}: unknown parameter \`${key}\` on ${component.id}`);
    if (spec.role !== "free") {
      throw new Error(`${what}: \`${key}\` is not a free parameter on ${component.id}`);
    }
  }
}

/** Every `role: "free"` key's default value — `key` (string domain) or `value` (numeric). */
function defaultFreeParams(component: LibraryComponent): Record<string, ComponentParamValue> {
  const out: Record<string, ComponentParamValue> = {};
  for (const [name, spec] of Object.entries(component.parameters)) {
    if (spec.role !== "free") continue;
    if (spec.key !== undefined) out[name] = spec.key;
    else if (spec.value !== undefined) out[name] = spec.value;
  }
  return out;
}

function commitPlaceComponent(
  component: LibraryComponent,
  translate: [number, number, number],
  rotate: TransformRotationParams | undefined,
  overrides: Record<string, ComponentParamValue> | undefined,
  mate?: PlaceComponentMate,
): ApplyOperationResult {
  const seq = nextComponentSeq++;
  const bodyId = nextBodyId();
  const rot = rotate ?? { center: [0, 0, 0] as [number, number, number], axis: [0, 0, 1] as [number, number, number], angleDeg: 0 };
  syntheticBodies.set(bodyId, placeComponentGhostMesh(translate, rot));
  const name = seq === 1 ? component.name : `${component.name} (${seq})`;
  const featureId = nextFeatureId();
  mockFeatures = [
    ...mockFeatures,
    { id: featureId, kind: "boolean", opType: "PlaceComponent", label: "Place Component", valueText: "", status: "ok" },
  ];
  featureTouched.set(featureId, [bodyId]);
  // Stored so `getOperationParams`/`setComponentParams` (WP-2.4) have a real
  // record to read/merge against — mirrors the real backend's
  // `PlaceComponentParams` shape (camelCase). `commitPlaceComponent` bypasses
  // `commitOp`'s generic `wireParamsOf` (PlaceComponent throws there — it
  // never reaches the generic op-preview lane), so this is the one place
  // that stores it for the library op.
  // The gesture's own free params (auto-size, WP-A3) layered over the
  // component's declared defaults — the same merge the real backend performs,
  // so the stored record and the configurator agree on which size was placed.
  const resolvedParams = { ...defaultFreeParams(component), ...(overrides ?? {}) };
  featureParams.set(featureId, {
    componentId: component.id,
    componentVersion: component.version,
    componentRevision: component.revision,
    params: resolvedParams,
    source: {
      kind: "generator",
      generatorId: component.generatorId,
      generatorVersion: component.generatorVersion,
      params: resolvedParams,
    },
    placement: {
      translate,
      rotate: rot,
    },
    // The gesture's recorded snap (WP-H2). The mock has no ladder, so this is
    // stored verbatim: tests assert the commit CARRIED it, the worker-backed
    // lane (component_ops.rs) is where it actually re-seats.
    ...(mate ? { mate } : {}),
  });
  insertAtMockCursor(featureId);
  mockRevision += 1;
  const doc = documentStore.getState();
  doc.applyChange({
    revision: mockRevision,
    features: mockFeatures.map(cloneFeature),
    bodies: { ...doc.bodies, [bodyId]: { id: bodyId, name, visible: true } },
    dirty: true,
    appliedOps: mockAppliedOps,
  });
  writeMockMeta("body", bodyId, { name });
  return {
    revision: mockRevision,
    changedBodies: [bodyRef(bodyId)],
    removedBodies: [],
    features: mockFeatures.map(cloneFeature),
    opLabel: "Place Component",
  };
}

/**
 * WP-I `mateAxisReversed`/`mateSeatOffFace` repair items, by `PlaceComponent`
 * recordId (the mock has no kernel and can never DETECT a mate reversal on its
 * own, so a test seeds the item the real backend would have raised — this is
 * what lets `resolveRefs("<opId>.input0")` answer the SAME DTO shape Rust
 * derives locally from the stored item, instead of a test hand-rolling one).
 */
interface MockMateAxisItem {
  /** Defaults to `"mateAxisReversed"`. */
  reason?: "mateAxisReversed" | "mateSeatOffFace";
  /** Required for `mateAxisReversed`; ignored (may be omitted) for `mateSeatOffFace`. */
  resolvedAxis?: [number, number, number];
  frozenAxis?: [number, number, number];
  resolvedSidedness?: "pin" | "hole";
  uiLabel?: string;
}
const mockMateAxisItems = new Map<string, MockMateAxisItem>();

/** Test seam: seed (or clear, with `null`) a mate repair item for `opId`. */
export function setMockMateAxisItem(opId: string, item: MockMateAxisItem | null): void {
  if (item) mockMateAxisItems.set(opId, item);
  else mockMateAxisItems.delete(opId);
}

/** Canned repair candidates for a ref (deterministic; descending score). */
function mockResolveRefs(refs: ResolveRefRequest[]): ResolveRefResult[] {
  return refs.map((r) => {
    const parsed = parseRefId(r.refId);
    const mateItem = parsed ? mockMateAxisItems.get(parsed.opId) : undefined;
    if (mateItem) {
      const reason = mateItem.reason ?? "mateAxisReversed";
      const candidates: ResolveCandidate[] =
        reason === "mateAxisReversed"
          ? // Both rows share a TopoKey and score (SCHEMA §9) — `label` is the
            // only thing telling them apart, exactly as the real ladder answers
            // this item.
            [
              {
                topoKey: "f:mate",
                score: 0.5,
                margin: 0,
                worldPos: [0, 0, 0],
                summary: "target face",
                label: MATE_AXIS_KEEP_LABEL,
              },
              {
                topoKey: "f:mate",
                score: 0.5,
                margin: 0,
                worldPos: [0, 0, 0],
                summary: "target face",
                label: MATE_AXIS_FOLLOW_LABEL,
              },
            ]
          : [
              {
                topoKey: "f:mate",
                score: 0.5,
                margin: 0,
                worldPos: [0, 0, 0],
                summary: "target face",
              },
            ];
      return {
        snapshotId: r.snapshotId ?? 0,
        revision: r.revision ?? 0,
        refId: r.refId,
        bodyId: r.primary?.bodyId,
        outcome: "needsRepair",
        reason,
        scoringVersion: 1,
        uiLabel:
          mateItem.uiLabel ??
          (reason === "mateAxisReversed"
            ? "This mate's axis may have reversed"
            : "This mate's seat moved off the target face"),
        resolvedAxis: mateItem.resolvedAxis,
        frozenAxis: mateItem.frozenAxis,
        resolvedSidedness: mateItem.resolvedSidedness,
        candidates,
      };
    }
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
      snapshotId: r.snapshotId ?? 0,
      revision: r.revision ?? 0,
      refId: r.refId,
      bodyId: r.primary?.bodyId,
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

/** The variable name a wire `Scalar` is bound to (WP-VE.2), or undefined. */
function scalarExpr(v: unknown): string | undefined {
  if (v && typeof v === "object" && "expr" in (v as Record<string, unknown>)) {
    const e = (v as { expr: unknown }).expr;
    if (typeof e === "string" && e !== "") return e;
  }
  return undefined;
}

/**
 * The call-site [`ExpressionDimension`] of a wire params key — the mock's stand
 * -in for the `(field, Dimension)` registry
 * `KnownOperation::scalars_mut` declares (`document/record.rs`).
 *
 * Derived from the NAME rather than tabulated per op because the wire spelling
 * already encodes it: every angle scalar in that registry ends in `angleDeg`
 * (`draftAngleDeg`, `csAngleDeg`, `pressureAngleDeg`, …) and `unitScale` is the
 * one genuinely dimensionless entry. A table keyed by op+field would be a
 * second copy of the registry, free to drift; this reads the same fact off the
 * key the mock already holds.
 */
function mockScalarSite(key: string): ExpressionDimension {
  if (key === "unitScale") return "scalar";
  return key.toLowerCase().endsWith("angledeg") ? "angle" : "length";
}

/**
 * Resolves one bound scalar against the mock's variable table, at its call
 * site. MIRRORS `regen::variables::substitute_variables`' per-scalar step
 * (`evaluate_at_site`), through the SHARED evaluator — never a silent fall back
 * to the scalar's cached number, which is the whole point of the pass.
 */
function mockResolveExpr(
  expr: string,
  site: ExpressionDimension,
  table: ResolvedTable,
): { value: number } | { reason: string } {
  const result = evaluateAtSite(expr, site, table);
  return result.ok ? { value: result.value.number } : { reason: result.failure.message };
}

/** Blind/symmetric extrude distance floor. MIRRORS `worker/src/ops/ExtrudeOp.cpp`
 *  `kMinValue` (1e-3, itself the legacy `RegenerationEngine.cpp:61` guard): the
 *  kernel REFUSES a distance-driven extrude below it rather than building a
 *  degenerate solid. It is the one kernel rule a variable can trip by VALUE. */
const MOCK_EXTRUDE_MIN_DISTANCE = 1e-3;

/**
 * The steps a variable edit's regen would leave in Error, as `(recordId, message)`
 * — the mock's stand-in for `failedSteps` on the `regen-finished` the real backend
 * emits (W5 result truth).
 *
 * Recomputed from (features, variables) on every variable write, because that is
 * exactly what makes it knowable with no kernel: `substitute_variables` is a pure
 * function of those two, and `resolve_expr` decides the first rule below without a
 * worker round-trip at all. The second rule is the ONE kernel refusal a variable
 * can trigger by value, mirrored from its source with a citation rather than
 * invented — a second, drifting CSG validator in the mock is the divergence the
 * mock exists to avoid.
 *
 * `distance2` is deliberately not checked: a two-direction second distance is not
 * bindable from the UI, so mirroring its guard would be speculative.
 */
function mockVariableRegenFailures(): { recordId: string; message: string }[] {
  const out: { recordId: string; message: string }[] = [];
  const table = resolveVariableTable(mockVariables);
  for (const f of mockFeatures) {
    const params = featureParams.get(f.id);
    if (!params || f.suppressed) continue;
    let failure: string | undefined;
    for (const [key, raw] of Object.entries(params)) {
      if (failure !== undefined) break; // one reason per step, as Rust reports
      const expr = scalarExpr(raw);
      if (expr === undefined) continue;
      const resolved = mockResolveExpr(expr, mockScalarSite(key), table);
      if ("reason" in resolved) {
        // `${opType}.${field}: ${reason}` — the shape `UnresolvedVariable.message`
        // carries (`regen::variables::substitute_variables`).
        failure = `${f.opType ?? f.kind}.${key}: ${resolved.reason}`;
        continue;
      }
      if (f.opType !== "Extrude" || key !== "distance") continue;
      const twoDirs = params.twoDirections === true;
      const mode = typeof params.mode === "string" ? params.mode : "Blind";
      const driven = twoDirs ? mode === "Blind" : mode === "Blind" || mode === "Symmetric";
      if (driven && Math.abs(resolved.value) < MOCK_EXTRUDE_MIN_DISTANCE) {
        failure = twoDirs ? "Extrude first distance too small" : "Extrude distance too small";
      }
    }
    if (failure !== undefined) out.push({ recordId: f.id, message: failure });
  }
  return out;
}

/**
 * The history-row value text of an edge op — MIRRORS Rust `dto.rs
 * feature_value_text` (pinned there by
 * `chamfer_value_text_shows_the_second_distance_only_when_set`). A two-distance
 * chamfer reads `d1×d2`, a distance-angle one `d1 mm ∠a°`; anything else keeps
 * the single-number form. Mode is chosen by PRESENCE in the same order Rust reads
 * it (angle first), and `radiusFromValueText` parses the LEADING number back, so
 * d1 stays first in every mode.
 */
function edgeOpValueText(radius: number, distance2?: number, angleDeg?: number): string {
  if (angleDeg !== undefined) return `${radius.toFixed(1)} mm ∠${angleDeg.toFixed(1)}°`;
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
): Partial<
  Pick<FeatureRecord, "valueText" | "primaryValue" | "primaryValueKind" | "primaryExpr">
> {
  const none = {
    valueText: undefined,
    primaryValue: undefined,
    primaryValueKind: undefined,
    primaryExpr: undefined,
  };
  /*
   * `source` is the wire `Scalar` the number was read from, and taking it FIRST
   * is deliberate: it makes it impossible to mint a row value without naming the
   * scalar the binding must come from — the same "one match decides all three"
   * discipline `dto.rs feature_value` keeps on the Rust side (WP-VE.2).
   */
  const dimensioned = (
    source: unknown,
    valueText: string,
    value: number,
    primaryValueKind: FeatureRecord["primaryValueKind"] = "length",
  ) => ({ valueText, primaryValue: value, primaryValueKind, primaryExpr: scalarExpr(source) });
  switch (opType ?? kind) {
    case "Extrude":
    case "extrude": {
      const d = scalarValue(params.distance);
      return d === undefined ? none : dimensioned(params.distance, `${Math.abs(d).toFixed(1)} mm`, Math.abs(d));
    }
    case "Revolve":
    case "revolve": {
      const a = scalarValue(params.angleDeg);
      return a === undefined ? none : dimensioned(params.angleDeg, `${Math.round(Math.abs(a))}°`, a, "angle");
    }
    case "Fillet":
    case "Chamfer":
    case "fillet": {
      const r = scalarValue(params.radius);
      // `distance2` / `angleDeg` are Chamfer-only and skip-none on both sides, so
      // their mere presence is what picks the row's mode. The inline editor still
      // targets d1 (`radius`) alone, exactly as `dto.rs` does.
      return r === undefined
        ? none
        : dimensioned(
            params.radius,
            edgeOpValueText(r, scalarValue(params.distance2), scalarValue(params.angleDeg)),
            r,
          );
    }
    case "Shell":
    case "shell": {
      const t = scalarValue(params.thickness);
      return t === undefined ? none : dimensioned(params.thickness, `${t.toFixed(1)} mm`, t);
    }
    case "Hole": {
      const d = scalarValue(params.diameter);
      if (d === undefined) return none;
      // WP-T1: `thread.designation`, read defensively off the raw wire params
      // (untyped here, unlike `HoleParams["thread"]` in the typed op builders).
      const thread = params.thread;
      const designation =
        thread && typeof thread === "object" && typeof (thread as { designation?: unknown }).designation === "string"
          ? (thread as { designation: string }).designation
          : undefined;
      return dimensioned(params.diameter, holeValueText(d, designation), d, "diameter");
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
      return dimensioned(params.distance, v.valueText, v.primaryValue, v.primaryValueKind);
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
        const { valueText, primaryValue, primaryValueKind, primaryExpr } = featureValueForParams(
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
                  ...(valueText !== undefined
                    ? { valueText, primaryValue, primaryValueKind, primaryExpr }
                    : {}),
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
    case "setBodyColor": {
      writeMockMeta("body", command.body, { color: command.color ?? undefined });
      mockRevision += 1;
      return { ...noopResult(), opLabel: command.color ? "Set body color" : "Reset body color" };
    }
    case "setFaceColor": {
      const prev = mockBodyMeta.get(command.body);
      const next: Record<string, Rgba> = { ...(prev?.faceColors ?? {}) };
      if (command.color === null) delete next[command.elementId];
      else next[command.elementId] = command.color;
      writeMockMeta("body", command.body, {
        faceColors: Object.keys(next).length > 0 ? next : undefined,
      });
      mockRevision += 1;
      return { ...noopResult(), opLabel: command.color ? "Set face color" : "Reset face color" };
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

// ── Body-edge projection into a sketch (WP-P), mock lane ────────────────────
//
// `lane`'s sketch sessions carry no `projections` field of their own (F-WP8
// owns entities/constraints only), so this module keeps the provenance rows
// beside it, keyed by frontend sketchId — merged into `getSketch`/`enterSketch`
// on read, exactly like the real client re-keys `dto.projections` on every read.
//
// MOCK LIMIT (documented, not silently guessed): there is no kernel here, so
// only the seed BOX (`body1`) resolves — its six analytic faces via
// `lookupMockFace` (face outline) and its twelve edges via `BOX_EDGE_PAIRS`
// (single-edge). Everything else (the cylinder's curved side, any synthesized
// body, an unresolvable pick) refuses `unsupportedCurve` rather than
// fabricating geometry.
const sketchProjections = new Map<string, Record<string, ProjectedSource>>();
let nextProjectionSeq = 1;

/** World point → this plane's origin + u·xAxis + v·yAxis — the exact inverse of
 *  `worldToPlaneUv` (both bases are orthonormal, so a round trip is exact). */
function planeUvToWorld(plane: SketchPlane, uv: [number, number]): [number, number, number] {
  return [
    plane.origin[0] + uv[0] * plane.xAxis[0] + uv[1] * plane.yAxis[0],
    plane.origin[1] + uv[0] * plane.xAxis[1] + uv[1] * plane.yAxis[1],
    plane.origin[2] + uv[0] * plane.xAxis[2] + uv[1] * plane.yAxis[2],
  ];
}

/** Re-express a `lookupMockFace` boundary (authored in the FACE's own plane) in
 *  the TARGET sketch's plane — round-tripping through world space. A
 *  `projectToSketch` target sketch is an arbitrary existing sketch, not
 *  necessarily the face's own frame (unlike sketch-on-face, whose sketch plane
 *  IS the face plane and needs no reprojection). */
function reprojectFaceBoundary(geometry: MockFaceGeometry, targetPlane: SketchPlane): MockFaceGeometry["boundary"] {
  const toTarget = (uv: [number, number]): [number, number] =>
    worldToPlaneUv(targetPlane, planeUvToWorld(geometry.plane, uv));
  if (geometry.boundary.kind === "polygon") {
    return { kind: "polygon", corners: geometry.boundary.corners.map(toTarget) };
  }
  return { kind: "circle", center: toTarget(geometry.boundary.center), radius: geometry.boundary.radius };
}

/** One edge of the mock box (`body1`), world-space endpoints — `BOX_EDGE_PAIRS`
 *  is the same table `makeBoxMesh` renders `e:0..e:11` from, so a picked edge id
 *  and its rendered wire can never drift apart. */
function boxEdgeWorldPoints(topoKey: string): [[number, number, number], [number, number, number]] | null {
  const m = /^e:(\d+)$/.exec(topoKey);
  if (!m) return null;
  const pair = BOX_EDGE_PAIRS[Number(m[1])];
  if (!pair) return null;
  const corners = boxCorners(BOX_SIZE);
  return [corners[pair[0] as BoxCornerKey], corners[pair[1] as BoxCornerKey]];
}

/** Build ONE locked reference Line (+ its two Fixed-pinned endpoints) from two
 *  already target-plane-projected (u,v) points — the open-edge sibling of
 *  `mockProjectedContent`'s closed-ring polygon case. */
function mockProjectedEdgeContent(
  a: [number, number],
  b: [number, number],
  idPrefix: string,
): { entities: SketchEntity[]; constraints: SketchConstraint[] } {
  const wireEntities: Record<string, unknown>[] = [
    { id: `${idPrefix}_p0`, type: "Point", at: a, referenceLocked: true },
    { id: `${idPrefix}_p1`, type: "Point", at: b, referenceLocked: true },
    { id: `${idPrefix}_e0`, type: "Line", p0Ref: `${idPrefix}_p0`, p1Ref: `${idPrefix}_p1`, referenceLocked: true },
  ];
  const wireConstraints: Record<string, unknown>[] = [
    { id: `${idPrefix}_c0`, type: "Fixed", entities: [`${idPrefix}_p0`] },
    { id: `${idPrefix}_c1`, type: "Fixed", entities: [`${idPrefix}_p1`] },
  ];
  return {
    entities: frontendEntitiesFromDto(wireEntities),
    constraints: frontendConstraintsFromDto(wireConstraints, wireEntities),
  };
}

/** Deterministic 16-lowercase-hex mock `projectedHash`, over the entity's own
 *  projected (u,v) geometry — two FNV-1a-32 passes (`mockElementHash`) with
 *  different seeds, concatenated, mirroring how `mockElementHash` itself mints
 *  mock ElementIds. */
function mockProjectionHash(entity: SketchEntity): string {
  const key =
    entity.type === "Circle"
      ? `C:${entity.center};${entity.radius}`
      : `L:${entity.p0};${entity.p1}`;
  return `${mockElementHash(key)}${mockElementHash(`${key}#b`)}`;
}

/** One source's resolution: either projected content ready to merge in, or a
 *  refusal to report. Never both. */
type MockProjectionOutcome =
  | { ok: true; entities: SketchEntity[]; constraints: SketchConstraint[]; dtoEntities: ProjectedEntity[] }
  | { ok: false; refusal: ProjectionRefusal };

/** Resolve ONE projection source against the mock's analytic geometry. */
function resolveMockProjectionSource(
  source: { bodyId: string; topoKey: string },
  mode: "edges" | "faceOutline",
  targetPlane: SketchPlane,
  idPrefix: string,
): MockProjectionOutcome {
  const elementId = `el_${mockElementHash(`${source.bodyId}#${source.topoKey}`)}`;
  const refuse = (message: string): MockProjectionOutcome => ({
    ok: false,
    refusal: {
      bodyId: source.bodyId,
      elementId,
      topoKey: source.topoKey,
      code: "unsupportedCurve",
      message,
    },
  });

  if (mode === "faceOutline") {
    const found = lookupMockFace(source.bodyId, undefined, source.topoKey);
    if (found.kind !== "planar") {
      return refuse("MOCK LIMIT: only box faces/edges project in the mock (needs the OCCT worker)");
    }
    const boundary = reprojectFaceBoundary(found.geometry, targetPlane);
    const { entities, constraints } = mockProjectedContent({ plane: targetPlane, boundary }, idPrefix);
    const curves = entities.filter((e) => e.type === "Line" || e.type === "Circle");
    const dtoEntities: ProjectedEntity[] = curves.map((e) => ({
      entityId: e.id,
      type: e.type,
      sourceBodyId: source.bodyId,
      sourceElementId: elementId,
      projectedHash: mockProjectionHash(e),
    }));
    return { ok: true, entities, constraints, dtoEntities };
  }

  // mode === "edges"
  const pts = source.bodyId === "body1" ? boxEdgeWorldPoints(source.topoKey) : null;
  if (!pts) {
    return refuse("MOCK LIMIT: only box faces/edges project in the mock (needs the OCCT worker)");
  }
  const a = worldToPlaneUv(targetPlane, pts[0]);
  const b = worldToPlaneUv(targetPlane, pts[1]);
  const { entities, constraints } = mockProjectedEdgeContent(a, b, idPrefix);
  const line = entities.find((e) => e.type === "Line");
  if (!line) return refuse("MOCK LIMIT: degenerate edge");
  return {
    ok: true,
    entities,
    constraints,
    dtoEntities: [
      {
        entityId: line.id,
        type: "Line",
        sourceBodyId: source.bodyId,
        sourceElementId: elementId,
        projectedHash: mockProjectionHash(line),
      },
    ],
  };
}

/** `projectToSketch` (SCHEMA §7.6 `ProjectToSketchPlane`), mock lane. Merges
 *  every resolvable source's locked reference geometry into the existing
 *  session via the shared lane's `sketchUpsert` (the only writer of a lane
 *  session), then records the provenance rows this module keeps. */
async function mockProjectToSketch(req: ProjectToSketchRequest): Promise<ProjectToSketchResult> {
  await wait(MESH_LATENCY_MS);
  const session = lane.peekSession(req.sketchId) ?? (await enterSketchWithHydration(req.sketchId));
  const rows = { ...(sketchProjections.get(req.sketchId) ?? {}) };
  const addedEntities: SketchEntity[] = [];
  const addedConstraints: SketchConstraint[] = [];
  const dtoEntities: ProjectedEntity[] = [];
  const refusals: ProjectionRefusal[] = [];
  for (const source of req.sources) {
    const outcome = resolveMockProjectionSource(
      source,
      req.mode,
      session.plane,
      `proj_${nextProjectionSeq++}`,
    );
    if (!outcome.ok) {
      refusals.push(outcome.refusal);
      continue;
    }
    addedEntities.push(...outcome.entities);
    addedConstraints.push(...outcome.constraints);
    dtoEntities.push(...outcome.dtoEntities);
    outcome.dtoEntities.forEach((e, i) => {
      rows[e.entityId] = {
        sourceBodyId: e.sourceBodyId,
        sourceElementId: e.sourceElementId,
        sourceKind: req.mode === "edges" ? "edge" : "face",
        sourceOrdinal: i,
        projectedHash: e.projectedHash,
      };
    });
  }
  if (addedEntities.length > 0) {
    await lane.sketchUpsert(
      req.sketchId,
      [...session.entities, ...addedEntities],
      [...session.constraints, ...addedConstraints],
    );
    sketchProjections.set(req.sketchId, rows);
  }
  return {
    sketchId: req.sketchId,
    snapshotId: req.snapshotId ?? mockRevision,
    entities: dtoEntities,
    pointCount: 0,
    refusals,
  };
}

/** `updateProjection` (WP-P), mock lane. Mock bodies never move, so this
 *  re-reports the CURRENT recorded rows verbatim — same ids, same hashes — the
 *  honest answer to "did anything change?" when nothing in the mock document
 *  can move a projected source. */
async function mockUpdateProjection(sketchId: string): Promise<ProjectToSketchResult> {
  await wait(MESH_LATENCY_MS);
  const session = lane.peekSession(sketchId) ?? (await enterSketchWithHydration(sketchId));
  const rows = sketchProjections.get(sketchId) ?? {};
  const entities: ProjectedEntity[] = [];
  for (const [entityId, row] of Object.entries(rows)) {
    const entity = session.entities.find((e) => e.id === entityId);
    entities.push({
      entityId,
      type: entity?.type ?? "Line",
      sourceBodyId: row.sourceBodyId,
      sourceElementId: row.sourceElementId,
      projectedHash: row.projectedHash,
    });
  }
  return { sketchId, snapshotId: mockRevision, entities, pointCount: 0, refusals: [] };
}

/** `detachProjection` (WP-P), mock lane. Unlocks the targeted entities and
 *  drops the `Fixed` pins on their endpoints, in one `sketchUpsert`.
 *
 *  MOCK LIMIT: a projected point shared by two still-locked curves (a box
 *  corner) is unpinned the moment EITHER curve detaches — the real backend only
 *  frees a point once every curve holding it has let go (see `detach_projection`
 *  doc comment). Reproducing that needs the point-ownership graph the real
 *  `sketch::projection` module tracks; the mock has no such graph. */
async function mockDetachProjection(
  sketchId: string,
  entityIds?: string[],
): Promise<DetachProjectionResult> {
  await wait(MESH_LATENCY_MS);
  const session = lane.peekSession(sketchId) ?? (await enterSketchWithHydration(sketchId));
  const rows = sketchProjections.get(sketchId) ?? {};
  const targets = (entityIds && entityIds.length > 0 ? entityIds : Object.keys(rows)).filter(
    (id) => id in rows,
  );
  const remaining = Object.keys(rows).length - targets.length;
  if (targets.length === 0) {
    return { sketchId, entityIds: [], releasedConstraints: 0, remaining: Object.keys(rows).length };
  }
  const targetSet = new Set(targets);
  const nextEntities = session.entities.map((e) =>
    targetSet.has(e.id) ? { ...e, referenceLocked: false } : e,
  );
  let releasedConstraints = 0;
  const nextConstraints = session.constraints.filter((c) => {
    if (c.type !== "Fixed" || !c.entities.some((id) => targetSet.has(id))) return true;
    releasedConstraints += 1;
    return false;
  });
  for (const id of targets) delete rows[id];
  sketchProjections.set(sketchId, rows);
  await lane.sketchUpsert(sketchId, nextEntities, nextConstraints);
  return { sketchId, entityIds: targets, releasedConstraints, remaining };
}

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
  const projections = sketchProjections.get(session.sketchId);
  return projections ? { ...session, projections } : session;
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
  sketchProjections.clear();
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
  mockMateAxisItems.clear();
  lane.resetPreview();
  mockFeatures = MOCK_BASE_FEATURES.map(cloneFeature);
  mockAppliedOps = MOCK_BASE_FEATURES.length;
  mockRevision = 5;
  nextBodySeq = 2;
  nextFeatureSeq = 100;
  nextImportSeq = 1;
  undoStack.length = 0;
  redoStack.length = 0;
  mockRecovery = [];
  mockAuthored = [];
  mockTemplates = [];
  mockSketchDatum.clear();
  mockVariables = [];
  nextVariableSeq = 1;
  documentStore.getState().applyChange({ datums: {} });
  // Re-adopt the (already reset) projection store as the mock's metadata authority.
  seedMockMetadata();
}

export const mockClient: CadClient = {
  async listRecents() {
    await wait();
    // Mirrors `api::list_recents`: a card whose file is older than an unresolved
    // autosave has to advertise it, because clicking it is what destroys the offer.
    return RECENTS.map((p) => ({
      ...p,
      hasRecovery: mockRecovery.some((o) => o.originalPath === p.path),
    }));
  },
  async renameRecentProject(path, newName) {
    await wait();
    const trimmed = newName.trim();
    if (!trimmed) throw new Error("project name cannot be empty");
    if (trimmed.includes("/") || trimmed.includes("\\")) {
      throw new Error("project name cannot contain a path separator");
    }
    const entry = RECENTS.find((p) => p.path === path);
    if (!entry) return; // already gone — nothing to rename
    const dir = path.slice(0, path.lastIndexOf("/") + 1);
    const newPath = `${dir}${trimmed}.onecad`;
    if (newPath !== path && RECENTS.some((p) => p.path === newPath)) {
      throw new Error(`a project named "${trimmed}" already exists here`);
    }
    entry.path = newPath;
    entry.name = trimmed;
  },
  async deleteRecentProject(path) {
    await wait();
    const i = RECENTS.findIndex((p) => p.path === path);
    if (i !== -1) RECENTS.splice(i, 1);
  },
  async revealInFileManager() {
    await wait();
  },
  async newDocument() {
    await wait();
    return snapshot("Untitled");
  },
  async openDocument(path, onRecovery) {
    await wait();
    // Mirrors the Rust guard: a path an unresolved offer names cannot be opened
    // blind, because doing so is what destroys that offer's autosave.
    const pending = mockRecovery.find((o) => o.originalPath === path);
    if (pending) {
      if (onRecovery === "openSaved") {
        mockRecovery = mockRecovery.filter((o) => o !== pending);
      } else {
        throw new BackendError(
          "recoveryPending",
          `${path} has unsaved changes from a previous session — restore them or open the saved version`,
        );
      }
    }
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
  async importProject() {
    await wait();
    importStepAndEmit();
    return snapshot("ImportedProject");
  },
  // Module-owned document state (ADR-0004). The mock keeps it in memory so the
  // whole lane — including a module writing and reading its own slice — is
  // exercisable with no backend.
  async getModuleState(moduleId: string) {
    await wait();
    const state = mockModuleState.get(moduleId);
    return state ? { moduleId, ...structuredClone(state) } : null;
  },
  async setModuleState(
    moduleId: string,
    state: { schemaVersion: number; payload: unknown } | null,
  ) {
    await wait();
    if (state === null) mockModuleState.delete(moduleId);
    else mockModuleState.set(moduleId, structuredClone(state));
  },
  async listDocumentModules() {
    await wait();
    return [...mockModuleState.entries()]
      .map(([moduleId, s]) => ({ moduleId, schemaVersion: s.schemaVersion }))
      .sort((a, b) => a.moduleId.localeCompare(b.moduleId));
  },
  // Document variables (WP-VE.2). A real in-memory table, not a stub: the whole
  // section — CRUD, validation messages, and the `=name` binding a bound row
  // renders — has to be exercisable with no backend.
  async listVariables() {
    await wait();
    return mockVariableRows();
  },
  async upsertVariable(name: string, value: number) {
    await wait();
    const variables = mockUpsertVariable(name, value);
    // A variable edit dirties the timeline in the real backend, so it emits the
    // same document-changed the UI refreshes off. The mock has no regen, so no
    // body moves — but the EVENT must fire, or a consumer that repaints on it
    // would look correct here and stale in the app.
    mockRevision += 1;
    emitMockDocumentChanged({ revision: mockRevision, changedBodies: [], removedBodies: [] });
    return variableEditResult(variables, "SetVariable");
  },
  async upsertVariableExpr(name: string, text: string) {
    await wait();
    const variables = mockUpsertVariableExpr(name, text);
    mockRevision += 1;
    emitMockDocumentChanged({ revision: mockRevision, changedBodies: [], removedBodies: [] });
    return variableEditResult(variables, "SetVariable");
  },
  async renameVariable(name: string, newName: string) {
    await wait();
    const variables = mockRenameVariable(name, newName);
    mockRevision += 1;
    emitMockDocumentChanged({ revision: mockRevision, changedBodies: [], removedBodies: [] });
    return variableEditResult(variables, "RenameVariable");
  },
  /* Pure: no revision bump, no event, no regen — the preview must be safe to
     call on a debounce while the user types. */
  async evaluateExpression(expr: string, site: ExpressionDimension) {
    await wait();
    return mockEvaluateExpression(expr, site);
  },
  async removeVariable(name: string) {
    await wait();
    const variables = mockRemoveVariable(name);
    mockRevision += 1;
    emitMockDocumentChanged({ revision: mockRevision, changedBodies: [], removedBodies: [] });
    return variableEditResult(variables, "RemoveVariable");
  },
  async closeDocument() {
    await wait();
    mockModuleState.clear();
    documentStore.getState().applySnapshot(emptyDocument());
  },
  async checkRecovery() {
    await wait();
    if (
      mockRecovery.length === 0 &&
      typeof window !== "undefined" &&
      new URLSearchParams(window.location.search).get("mockrecovery") === "1"
    ) {
      mockRecovery = mockRecoveryFixture();
    }
    return mockRecovery.map((o) => ({ ...o }));
  },
  async recoverDocument(documentId: string, accept: boolean) {
    await wait();
    const offer = mockRecovery.find((o) => o.documentId === documentId);
    if (!offer) return null; // nothing pending under that id
    mockRecovery = mockRecovery.filter((o) => o.documentId !== documentId);
    if (!accept) return null;
    // Title first, then the saved file's stem — the same ladder `mark_recovered`
    // walks, so the mock lane shows the name the real one would.
    return snapshot(offer.title ?? (offer.originalPath ? basename(offer.originalPath) : "Recovered"));
  },
  // The unified start-screen import picker. `appStore.importFromDialog` routes on
  // the EXTENSION, so the mock has to be able to hand back either kind or the
  // STEP half of that router is unreachable from a browser lane: `?mockimport=step`
  // picks it. Same dev-only URL-flag pattern as `?vpdemo` (ViewportRoot.tsx:145).
  async importFileDialog() {
    await wait(40);
    const kind = new URLSearchParams(window.location.search).get("mockimport");
    return kind === "step"
      ? "/Users/andrej/CAD/Projects/Imported.step"
      : "/Users/andrej/CAD/Projects/Imported.onecad";
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
  // (no filesystem): save adopts a plausible path and reports the same outcome
  // shape as Rust, so replacement guards exercise the clean-result contract.
  // `_previewPng` is accepted and DISCARDED: there is no container to write a
  // preview.png into. Taking the parameter keeps the two clients' signatures in
  // step so a caller compiles identically against either.
  async saveDocument(path?: string, _previewPng?: string | null): Promise<SaveOutcome> {
    await wait(40);
    const document = documentStore.getState();
    return {
      documentId: document.documentId ?? "mock-document",
      savedRevision: document.revision,
      currentRevision: document.revision,
      clean: true,
      path: path ?? `/Users/andrej/CAD/Projects/${document.title || "Untitled"}.onecad`,
      title: path ? basename(path) : document.title,
    };
  },
  async saveDocumentAs(_previewPng?: string | null): Promise<SaveOutcome | null> {
    await wait(40);
    return this.saveDocument("/Users/andrej/CAD/Projects/Untitled.onecad");
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
  async export3mf() {
    await wait(40);
    return "/Users/andrej/CAD/Projects/Untitled.3mf";
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
    let key = topoKey;
    if (!key && elementId) {
      const mapped = mockElementIdToTopoKey.get(elementId);
      if (mapped?.bodyId === bodyId) {
        key = mapped.topoKey;
      } else if (elementId.startsWith("el_")) {
        // Deterministic scan: find the face whose promoted id matches the stored
        // elementId so persisted face colors resolve after reload.
        const blob = mockBodyMesh(bodyId);
        const view = parseMeshPayload(blob);
        const topo = new TopoIndex(view.faceRanges, view.faceCount, view.faceIdOffsets, view.faceIdChars);
        for (let f = 0; f < view.faceCount; f++) {
          const tk = topo.idAt(f);
          if (!tk) continue;
          if (`el_${mockElementHash(`${bodyId}#${tk}`)}` === elementId) {
            key = tk;
            mockElementIdToTopoKey.set(elementId, { bodyId, topoKey: tk });
            break;
          }
        }
      }
    }
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

  /**
   * Component Library WP-0.1 mock. Reuses `elementInfo`'s same key resolution
   * (topoKey first, then elementId) and mesh-derived planar/edge metrics.
   *
   * MOCK-LANE HONESTY: the plane AND cylinder cases are real frames, each
   * MEASURED off the body's own MESH1 bytes — a planar face by its shared
   * triangle normal, a cylindrical one by `cylinderMetricsFromMesh` (facet
   * normals ⊥ a common axis + a least-squares circle fit, both checked). A
   * face that is neither still answers with its kind and `frame: null` rather
   * than fabricating an axis, and a curved EDGE still does too (the mock emits
   * circles as polylines, which are indistinguishable from any other curve
   * here). Real cylinder/circle frames are pinned against the OCCT worker in
   * `src-tauri/tests/component_ops.rs`; what this lane owns is the UI chain
   * that runs on one — hover → snap kind → auto-size → ghost → commit.
   */
  async classifyElement(
    bodyId: string,
    elementId: string,
    topoKey?: string,
  ): Promise<ClassifyResult | null> {
    await wait(MESH_LATENCY_MS);
    let key = topoKey;
    if (!key && elementId) {
      const mapped = mockElementIdToTopoKey.get(elementId);
      if (mapped?.bodyId === bodyId) key = mapped.topoKey;
    }
    if (!key) return null;
    const isEdge = key.startsWith("e:");
    const blob = mockBodyMesh(bodyId);

    if (isEdge) {
      const edge = edgeMetricsFromMesh(blob, key);
      if (!edge) return null;
      return {
        kind: "edge",
        surfaceType: "",
        curveType: edge.straight ? "line" : "other",
        frame: null,
      };
    }
    const face = faceMetricsFromMesh(blob, key);
    if (!face) return null;
    if (face.planar) {
      return {
        kind: "face",
        surfaceType: "plane",
        curveType: "",
        frame: { origin: face.center, normal: face.normal, axis: null, radius: null },
      };
    }
    const cylinder = cylinderMetricsFromMesh(blob, key);
    return {
      kind: "face",
      surfaceType: cylinder ? "cylinder" : "other",
      curveType: "",
      frame: cylinder
        ? { origin: cylinder.origin, normal: null, axis: cylinder.axis, radius: cylinder.radius }
        : null,
    };
  },

  // ── Component Library (WP-1.3/1.5) ──────────────────────────────────────
  // MOCK-LANE HONESTY, DEFAULT: no fabricated catalog. A fake component list
  // would outlive the mock and teach a UI bug ("nothing shows up") to pass
  // e2e — the same reasoning the Extensions ▸ Browse panel's "no registry
  // configured" empty state follows. `?mocklibrary=1` opts a fixture IN for
  // manual verification and the e2e lane — see `MOCK_LIBRARY_FIXTURE`'s own
  // comment for why this is the same pattern as `?mockimport=step`.

  async listLibraryComponents(): Promise<LibraryComponent[]> {
    await wait(MESH_LATENCY_MS);
    const seeded = mockLibraryEnabled() ? MOCK_LIBRARY_FIXTURES : [];
    return [...seeded, ...mockAuthored];
  },

  async reindexLibrary(): Promise<ReindexReport> {
    await wait(MESH_LATENCY_MS);
    const total = mockLibraryEnabled() ? MOCK_LIBRARY_FIXTURES.length : 0;
    return { total, indexed: total, skipped: [] };
  },

  async resolveComponentSource(
    componentId: string,
    componentVersion: string,
  ): Promise<PlaceComponentSource> {
    await wait(MESH_LATENCY_MS);
    const component = mockComponentByIdentity(componentId, componentVersion);
    if (!component) {
      throw new Error(
        `resolveComponentSource: unknown component ${componentId}@${componentVersion} — the mock lane knows the ?mocklibrary=1 fixture and this session's authored components`,
      );
    }
    // The fixture is a `generator` component, so there is nothing to stage —
    // which is the honest mock answer, not a shortcut: staging is a real
    // document-carrier + worker-workspace write that has no meaning in a lane
    // with neither. An AUTHORED mock component has no baked blob either
    // (`saveAsComponent` bakes nothing here), so it resolves as the fixture's
    // generator — same reuse `placeComponent` documents.
    return {
      kind: "generator",
      generatorId: component.generatorId ?? MOCK_LIBRARY_FIXTURE.generatorId ?? MOCK_LIBRARY_FIXTURE.id,
      generatorVersion: component.generatorVersion ?? MOCK_LIBRARY_FIXTURE.generatorVersion ?? 1,
    };
  },

  async placeComponent(
    componentId: string,
    componentVersion: string,
    translate: [number, number, number],
    rotate?: TransformRotationParams,
    params?: Record<string, ComponentParamValue>,
    mate?: PlaceComponentMate,
  ): Promise<ApplyOperationResult> {
    const component = mockComponentByIdentity(componentId, componentVersion);
    if (!component) {
      throw new Error(
        `placeComponent: unknown component ${componentId}@${componentVersion} — the mock lane knows the ?mocklibrary=1 fixture and this session's authored components`,
      );
    }
    documentStore.getState().regenStarted();
    try {
      await wait();
      assertFreeParams(component, params);
      const res = commitPlaceComponent(component, translate, rotate, params, mate);
      emitMockDocumentChanged({
        revision: res.revision,
        changedBodies: res.changedBodies,
        removedBodies: res.removedBodies,
      });
      // `withCursor` is what stamps the terminal, so the forced-terminal hook
      // (`setMockRegenTerminal`) reaches this lane too — a placement must be
      // provable against a failing regen on the mock lane like every other edit.
      return withCursor(res);
    } finally {
      documentStore.getState().regenSettled();
    }
  },

  /**
   * WP-2.4: unlike `detachComponent` (still genuinely nothing to detach from
   * in the mock lane), a placed instance's stored params ARE real here since
   * `commitPlaceComponent` writes them — so the role=free enforcement is
   * mirrored for real, not stubbed. What's NOT simulated: the mock's
   * synthetic mesh is a fixed demo shape regardless of size (`placeComponentGhostMesh`),
   * so a param edit changes the STORED value + the configurator's live
   * designation but not the rendered geometry — the real worker-backed lane
   * (`component_ops.rs`) is where a size change actually resizes the body.
   */
  /**
   * The mock lane has no kernel, so every component previews as the same
   * synthetic M6 SHCS mesh the placement ghost already uses. The FLOW (fetch →
   * parse → render → cache) is real; the shape is not, which is why the panel's
   * own tests assert wiring rather than geometry.
   */
  async componentPreviewMesh(
    componentId: string,
    _componentVersion: string,
    _params?: Record<string, ComponentParamValue>,
  ): Promise<{ bodyId: string; mesh: ArrayBuffer }[]> {
    await wait(MESH_LATENCY_MS);
    const identity = { center: [0, 0, 0] as const, axis: [0, 0, 1] as const, angleDeg: 0 };
    return [
      {
        bodyId: `preview_${componentId}`,
        mesh: placeComponentGhostMesh([0, 0, 0], {
          center: [...identity.center],
          axis: [...identity.axis],
          angleDeg: identity.angleDeg,
        }),
      },
    ];
  },

  async listTemplates(): Promise<ProjectTemplate[]> {
    await wait();
    return [...seededTemplates(), ...mockTemplates];
  },

  async saveAsTemplate(
    id: string,
    name: string,
    description?: string,
    _previewPng?: string | null,
  ): Promise<ProjectTemplate> {
    await wait();
    if (!id.trim()) throw new Error("saveAsTemplate: a template needs an id");
    if ([...seededTemplates(), ...mockTemplates].some((t) => t.id === id)) {
      throw new Error(`saveAsTemplate: template \`${id}\` already exists`);
    }
    const template: ProjectTemplate = { id, name, description };
    mockTemplates.push(template);
    return template;
  },

  /**
   * The FLOW is real (a new untitled document replaces the current one); the
   * starting geometry is not, because the mock lane has no frozen container to
   * instantiate. Same honesty rule `saveAsComponent` follows below.
   */
  async newFromTemplate(id: string): Promise<DocumentSnapshot> {
    await wait();
    if (![...seededTemplates(), ...mockTemplates].some((t) => t.id === id)) {
      throw new Error(`newFromTemplate: unknown template ${id}`);
    }
    return mockClient.newDocument();
  },

  /**
   * MOCK-LANE HONESTY: the package is recorded in the in-memory catalog so the
   * authoring FLOW is exercisable (the dialog, the menu item, the panel picking
   * it up afterwards), but nothing is baked and nothing is written — there is
   * no worker on this lane to export a solid from, and no library root to write
   * to. A placement of a mock-authored component therefore reuses the fixture
   * geometry, exactly like every other mock placement.
   */
  async saveAsComponent(
    bodyId: string,
    spec: NewComponentSpec,
    _previewPng?: string | null,
    // Accepted and ignored: nothing is baked on this lane, so there are never
    // several solids to fuse (WP-F1.2). Refusing it would make the mock reject
    // a call the real backend accepts.
    _unionSolids?: boolean,
  ): Promise<LibraryComponent> {
    await wait();
    if (!documentStore.getState().bodies[bodyId]) {
      throw new Error(`saveAsComponent: ${bodyId} is not a body in this document`);
    }
    if (!spec.id.includes(".")) {
      throw new Error(`saveAsComponent: id \`${spec.id}\` must be namespaced (<ns>.<name>)`);
    }
    if (mockAuthored.some((c) => c.id === spec.id && c.version === spec.version)) {
      throw new Error(`saveAsComponent: ${spec.id}@${spec.version} already exists`);
    }
    const authored: LibraryComponent = {
      id: spec.id,
      version: spec.version,
      name: spec.name,
      category: spec.category,
      tags: spec.tags,
      sourceKind: "document",
      revision: `sha256:${"a".repeat(64)}`,
      attachments: spec.attachments,
      // Declared free params ride through so the configurator is exercisable on
      // this lane too (WP-F1.3). Editing one still changes no geometry here —
      // there is no worker to re-bake with — which is the same honesty the bake
      // itself keeps above.
      parameters: Object.fromEntries(
        Object.entries(spec.parameters ?? {}).map(([name, p]) => [
          name,
          { role: "free" as const, key: p.key, value: p.value },
        ]),
      ),
      designation: spec.designation,
    };
    mockAuthored.push(authored);
    return authored;
  },

  /**
   * MOCK LIMIT, stated rather than faked: bulk STEP ingest needs a real STEP
   * reader plus the worker's `ExtractPrismProfile` canonicalization (SCHEMA
   * §7.3/§7.8 — WP-C), and this lane has neither. Every requested path
   * refuses BY NAME rather than fabricating a plausible catalog entry, so the
   * dialog's own wiring (batch call → per-row report → refresh-on-`ok`) is
   * exercisable without ever pretending a component was imported.
   */
  async ingestComponents(req: IngestComponentsRequest): Promise<IngestComponentsReport> {
    await wait(MESH_LATENCY_MS);
    return {
      libraryRoot: "<mock>",
      parts: req.paths.map((path) => ({
        path,
        status: "refused" as const,
        message: "MOCK LIMIT: component ingestion needs the OCCT worker",
      })),
    };
  },

  /** The mock lane cannot open a native dialog — `IngestComponentsDialog` falls
   *  back to its hidden `<input type="file" multiple>` when this resolves `[]`. */
  async pickComponentFiles(): Promise<string[]> {
    await wait(40);
    return [];
  },

  /** The mock lane has one fixture component and nothing to replace it with.
   *  Still typed to the W5 result shape so both impls stay in lockstep — the
   *  seam is what has to agree, not the (absent) behaviour. */
  async replaceComponent(
    recordId: string,
    componentId: string,
    _componentVersion: string,
    _params?: Record<string, ComponentParamValue>,
  ): Promise<ReplaceComponentResult> {
    await wait();
    throw new Error(
      `replaceComponent: not available on the mock lane (record ${recordId} → ${componentId})`,
    );
  },

  /** No versioned catalog on the mock lane, so nothing is ever newer. */
  async componentUpgradeAvailable(_recordId: string): Promise<ComponentUpgrade | null> {
    await wait();
    return null;
  },

  async setComponentParams(
    recordId: string,
    params: Record<string, ComponentParamValue>,
  ): Promise<ApplyOperationResult> {
    await wait();
    const stored = featureParams.get(recordId);
    if (!stored) {
      throw new Error(`setComponentParams: no params for record ${recordId}`);
    }
    const componentId = stored.componentId;
    // Resolved through the catalog, not compared against ONE id: every seeded
    // component's instances are configurable, and pinning this to the SHCS made
    // a bearing's or a motor's `length` edit fail on this lane only.
    const component =
      typeof componentId === "string" && typeof stored.componentVersion === "string"
        ? mockComponentByIdentity(componentId, stored.componentVersion)
        : null;
    if (!component) {
      throw new Error(`setComponentParams: record ${recordId} is not a placed component`);
    }
    for (const key of Object.keys(params)) {
      const spec = component.parameters[key];
      if (!spec) {
        throw new Error(`setComponentParams: unknown parameter \`${key}\` on ${componentId}`);
      }
      if (spec.role !== "free") {
        throw new Error(`setComponentParams: \`${key}\` is not a free parameter on ${componentId}`);
      }
    }

    const mergedParams: Record<string, ComponentParamValue> = {
      ...(stored.params as Record<string, ComponentParamValue>),
      ...params,
    };
    const source = stored.source as Record<string, unknown>;
    featureParams.set(recordId, {
      ...stored,
      params: mergedParams,
      source: { ...source, params: mergedParams },
    });

    mockRevision += 1;
    const doc = documentStore.getState();
    doc.applyChange({
      revision: mockRevision,
      features: mockFeatures.map(cloneFeature),
      bodies: doc.bodies,
      dirty: true,
      appliedOps: mockAppliedOps,
    });
    // The mock's synthetic mesh is size-independent, so no body actually
    // changes here — `noop` is the honest terminal, and `withCursor` still lets
    // the forced-terminal hook override it.
    return withCursor({
      revision: mockRevision,
      changedBodies: [],
      removedBodies: [],
      features: mockFeatures.map(cloneFeature),
      terminal: "noop",
    });
  },

  async detachComponent(): Promise<ApplyOperationResult> {
    throw new Error("detachComponent: the mock lane has no placed components yet (WP-1.4/1.5)");
  },

  /**
   * WP-I: the §9 `mateAxisReversed` repair, honestly. `commitPlaceComponent`
   * stores a REAL `mate` on this lane, so the record this rewrites is the same
   * one `getOperationParams` reads back — a panel test can assert the toggle and
   * the re-frozen axis for real instead of against a stub.
   *
   * What is NOT simulated: the mock has no kernel, so no mate ever REVERSES here
   * and the repair is never raised by the lane itself. The refusals are mirrored
   * exactly (`repair_mate_axis` + `DocumentSession::repair_mate_axis`) so the
   * mock cannot stay green on a call the real backend rejects.
   */
  async repairMateAxis(
    recordId: string,
    keepWorldDirection: boolean,
    resolvedAxis: [number, number, number],
    resolvedSidedness?: "pin" | "hole",
  ): Promise<ApplyOperationResult> {
    await wait();
    const stored = featureParams.get(recordId);
    if (!stored) {
      throw new Error(`repairMateAxis: no params for record ${recordId}`);
    }
    const mate = stored.mate as Record<string, unknown> | undefined;
    if (!mate) {
      throw new Error(`repairMateAxis: PlaceComponent ${recordId} has no mate`);
    }
    if (mate.kind !== "concentric" && mate.kind !== "concentricAndCoincident") {
      throw new Error(
        `repairMateAxis: PlaceComponent ${recordId} has a ${String(mate.kind)} mate — only a concentric mate has a target axis to re-freeze`,
      );
    }
    if (!resolvedAxis.every((n) => Number.isFinite(n))) {
      throw new Error(`repairMateAxis: resolvedAxis [${resolvedAxis.join(", ")}] must be finite`);
    }
    featureParams.set(recordId, {
      ...stored,
      mate: {
        ...mate,
        // `flipped` is the ONLY orientation bit — keeping the world direction
        // means absorbing the axis reversal into it.
        flipped: keepWorldDirection ? !mate.flipped : mate.flipped,
        targetAxis: resolvedAxis,
        // Absent evidence LEAVES the frozen sidedness alone, exactly as Rust does.
        ...(resolvedSidedness ? { targetSidedness: resolvedSidedness } : {}),
      },
    });

    mockRevision += 1;
    const doc = documentStore.getState();
    doc.applyChange({
      revision: mockRevision,
      features: mockFeatures.map(cloneFeature),
      bodies: doc.bodies,
      dirty: true,
      appliedOps: mockAppliedOps,
    });
    // The mock's synthetic mesh does not depend on the mate, so nothing moves.
    return withCursor({
      revision: mockRevision,
      changedBodies: [],
      removedBodies: [],
      features: mockFeatures.map(cloneFeature),
      terminal: "noop",
    });
  },

  // Deterministic mock promotion (Invariant 1: same pick → same id).
  async promoteSelection(
    bodyId: string,
    picks: PromotePick[],
    _snapshotId?: number,
  ): Promise<PromotedElement[]> {
    await wait(MESH_LATENCY_MS);
    return picks.map((p) => {
      const elementId = `el_${mockElementHash(`${bodyId}#${p.topoKey}`)}`;
      mockElementIdToTopoKey.set(elementId, { bodyId, topoKey: p.topoKey });
      return {
        topoKey: p.topoKey,
        elementId,
        kind: p.topoKey.startsWith("e:") ? "edge" : "face",
        bodyId,
      };
    });
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

  /**
   * `PrepareEdgeOp` (SCHEMA §7.6), mock lane.
   *
   * MOCK LIMIT — the IDENTITY closure: with no kernel there is no tangency
   * analysis, so every pick is its own tangent CONTOUR. `contour` is therefore the
   * pick's rank among the picks sorted by edge ordinal, which is exactly what §7.6
   * defines it as ("contour k is seeded by the k-th smallest picked edge ordinal")
   * for a body whose edges are all creases — the seed box.
   *
   * `adjacentFaces` IS real where the mock can derive it: the seed box's twelve
   * edges and six faces come from the same tables `makeBoxMesh` renders, so
   * `mockAdjacentFaces` reports the true pair, face-ordinal ascending. For every
   * other body the list is OMITTED rather than fabricated — the frontend then
   * refuses an asymmetric chamfer with a stated reason instead of committing one
   * that silently falls back to the ordinal reference face.
   *
   * The cross-body REFUSAL is real for the same reason it is in `prepareOffsetFace`:
   * it is a fact about the PICKS, which the mock can see.
   */
  async prepareEdgeOp(req: PrepareEdgeOpRequest): Promise<PrepareEdgeOpResult> {
    await wait(MESH_LATENCY_MS);
    const snapshotId = req.snapshotId ?? mockRevision;
    const bodies = [...new Set(req.pickedEdges.map((p) => p.bodyId ?? ""))];
    if (bodies.length > 1) {
      return {
        snapshotId,
        targetBodyId: "",
        edges: [],
        refusal: {
          code: "crossBody",
          message: `${req.mode}: every selected edge must belong to the same body`,
          edges: req.pickedEdges.map((p) => p.topoKey ?? p.elementId ?? ""),
        },
      };
    }
    const bodyId = bodies[0] ?? "";
    // Contour rank = position in the picks sorted by edge ORDINAL (SCHEMA §7.6),
    // de-duplicated. A key with no `e:<n>` ordinal sorts last, deterministically.
    const ordinalOf = (key: string): number => {
      const m = /^e:(\d+)$/.exec(key);
      return m ? Number(m[1]) : Number.MAX_SAFE_INTEGER;
    };
    const keys = [...new Set(req.pickedEdges.map((p) => p.topoKey ?? p.elementId ?? ""))];
    const contourOf = new Map(
      [...keys].sort((a, b) => ordinalOf(a) - ordinalOf(b)).map((key, i) => [key, i]),
    );
    return {
      snapshotId,
      targetBodyId: bodyId,
      edges: req.pickedEdges.map((p) => {
        const topoKey = p.topoKey ?? p.elementId ?? "";
        const adjacentFaces = mockAdjacentFaces(bodyId, topoKey);
        return {
          topoKey,
          picked: true,
          elementId: p.elementId ?? `el_mock_${topoKey.replace(":", "_")}`,
          bodyId,
          kind: "edge" as const,
          contour: contourOf.get(topoKey) ?? 0,
          // Omitted, never guessed, for a body the mock cannot describe.
          ...(adjacentFaces.length > 0 ? { adjacentFaces } : {}),
        };
      }),
      refusal: null,
    };
  },

  /**
   * MOCK LIMIT, stated rather than papered over: there is no kernel here, and
   * the ONLY honest source of a feasible range is a build that ran. So this
   * reports that nothing was measured — `confidence:"none"`, every bound null,
   * no intervals — which the clamp helper reads as "do not clamp", exactly the
   * behaviour the frontend had before this verb existed.
   *
   * Fabricating a plausible range here would be worse than useless: it would
   * make the mock lane pass tests about a guard the real lane implements
   * differently, and it would forbid values on the e2e model that the kernel
   * accepts. `stoppedReason:"budgetExhausted"` with `probesUsed:0` says it out
   * loud — the budget for a search that cannot run is zero.
   *
   * The cross-body REFUSAL is real, because it is a fact about the PICKS and the
   * mock can see those. That branch mirrors `prepareEdgeOp` exactly.
   */
  async analyzeEdgeOpRange(req: AnalyzeEdgeOpRangeRequest): Promise<AnalyzeEdgeOpRangeResult> {
    await wait(MESH_LATENCY_MS);
    const snapshotId = req.snapshotId ?? mockRevision;
    const bodies = [...new Set(req.pickedEdges.map((p) => p.bodyId ?? ""))];
    const unmeasured = {
      snapshotId,
      mode: req.mode,
      searchedRange: { min: 0, max: 0 },
      lowerBound: null,
      bestKnownMax: null,
      provenUpperBound: null,
      feasibleIntervals: [],
      intervalsTruncated: false,
      limitingEntities: [],
      confidence: "none" as const,
      monotonicObserved: true,
      probesUsed: 0,
      budgetExhausted: true,
      stoppedReason: "budgetExhausted" as const,
    };
    if (bodies.length > 1) {
      return {
        ...unmeasured,
        targetBodyId: "",
        edges: [],
        refusal: {
          code: "crossBody",
          message: `${req.mode}: every selected edge must belong to the same body`,
          edges: req.pickedEdges.map((p) => p.topoKey ?? p.elementId ?? ""),
        },
      };
    }
    return {
      ...unmeasured,
      targetBodyId: bodies[0] ?? "",
      edges: req.pickedEdges.map((p) => p.topoKey ?? p.elementId ?? ""),
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
    if (peeked) {
      const projections = sketchProjections.get(sketchId);
      return projections ? { ...peeked, projections } : peeked;
    }
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
      return { regionIdentityVersion: 3, regions };
    }
    if (documentStore.getState().sketches[sketchId]) {
      const plane = planeFor("XY");
      const regions = detectRegions(seededSketchRectangle());
      lane.cacheSketchPlane(sketchId, plane);
      lane.cacheFinishedRegions(sketchId, regions);
      return { regionIdentityVersion: 3, regions };
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
    sketchProjections.delete(id);
    // A deleted sketch no longer references its host datum — otherwise the
    // orphan-cleanup path (SketchController leaving mid-enter) would leave a
    // phantom blocker that makes the datum permanently undeletable.
    mockAttachSketchToDatum(id, null);
  },
  reattachSketch: mockReattachSketch,
  beginGesture: lane.beginGesture,
  solveDrag: lane.solveDrag,
  endGesture: lane.endGesture,
  projectToSketch: mockProjectToSketch,
  updateProjection: mockUpdateProjection,
  detachProjection: mockDetachProjection,
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
