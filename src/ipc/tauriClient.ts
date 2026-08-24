/*
 * tauriClient — the REAL CadClient over the Tauri command/event surface
 * (R-WP10/11/12).
 *
 * Implements the same `CadClient` interface as `mockClient`, mapping each method
 * onto a `#[tauri::command]` (`invoke`) or a backend event (`listen`). Constructed
 * ONLY inside a Tauri webview (see `createClient` in `client.ts`); dev-in-browser,
 * vitest and Playwright keep the mock.
 *
 * ── What is REAL vs the SEAM (F-WP9) ──────────────────────────────────────────
 *  REAL commands : lifecycle, dialogs, recents, get_mesh (MESH1 ArrayBuffer),
 *                  undo/redo, apply_edit_command; the SKETCH SOLVER lane
 *                  (enter/upsert/finish/cancel) + the DRAG gesture lane
 *                  (begin/solve/end, latest-wins) + promote_selection.
 *  REAL events   : document-changed, projection-updated (→ store hydration),
 *                  regen-finished (prompt correlation), sketch-solved.
 *  LOCAL SEAM    : the drag-time L2 PREVIEW lane. The real backend has NO cheap
 *                  preview verb (only apply/regen commits geometry), so L2 stays
 *                  on the local previewer — seam-marked "(backend preview verb
 *                  TBD)". A previewed op's COMMIT already routes through the real
 *                  `apply_edit_command`; only the drag-time exact mesh is local.
 *
 * ── Sync-over-async adapter ───────────────────────────────────────────────────
 * `applyOperation`/`undo`/`redo` return a SYNCHRONOUS `ApplyOperationResult`, but
 * the backend is event-driven: an edit returns the PRE-regen projection, then
 * regen publishes geometry LATER. Each edit correlates the command's projection
 * with the next `document-changed` (bodies) OR `regen-finished` (`{revision,
 * outcome}` — resolves the no-geometry / noop case promptly, replacing the old 8 s
 * wait; F-WP8 flag 3).
 *
 * ── Sketch marshalling ────────────────────────────────────────────────────────
 * The frontend authors inlined-coordinate entities with string ids; `sketch_upsert`
 * wants `SketchEditOp[]` (Rust typed doc form, UUID ids, point-referenced). The
 * pure `sketchWireMap` module bridges them, synthesizing points + keeping an
 * id-map per sketch (see its header). See the M2-gate notes for the round-trip
 * items this WP marshals but cannot end-to-end validate without the real worker.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { trace, traceWarn } from "@/debug/trace";
import { logDebug, logError, logWarn } from "@/debug/log";
import type { CadClient } from "./client";
import { parseOperationDiagnostics } from "./operationDiagnostics";
import type {
  ApplyOperationResult,
  BeginGestureResult,
  ClassifyResult,
  ComponentParamValue,
  ComponentUpgrade,
  NewComponentSpec,
  PlaceComponentMate,
  ProjectTemplate,
  ReplaceComponentReport,
  ReplaceComponentResult,
  VariableEditResult,
  CurveParams,
  DocumentChange,
  DocumentModule,
  DocumentVariable,
  DocumentProjectionWire,
  DocumentSnapshot,
  DragSolveResult,
  ElementInfo,
  LibraryComponent,
  PlaceComponentSource,
  MassProperties,
  ModuleState,
  EnterSketchTarget,
  FeatureDependencies,
  FeatureRecord,
  FinishSketchResult,
  GestureTarget,
  Lod,
  NeedsRepairEvent,
  OperationOp,
  OperationDiagnostic,
  PrepareOffsetFaceRequest,
  PrepareOffsetFaceResult,
  PrepareEdgeOpRequest,
  PrepareEdgeOpResult,
  AnalyzeEdgeOpRangeRequest,
  AnalyzeEdgeOpRangeResult,
  PromotedElement,
  PromotePick,
  RecentProject,
  OnRecovery,
  RecoveryInfo,
  RegenTerminal,
  ReindexReport,
  SaveOutcome,
  RegenFinished,
  ResolveRefRequest,
  ResolveRefResult,
  SketchConstraint,
  SketchConstraintType,
  SketchEntity,
  SketchEntityStates,
  SketchAttachTarget,
  SketchPlane,
  SketchPlaneKind,
  SketchRegion,
  SketchSession,
  SketchSolveStatus,
  SketchUpsertResult,
  TransformRotationParams,
  WorkerStatus,
} from "./types";
import { createLocalSolverLane } from "./localSolver";
import {
  operationToEditCommand,
  opLabelFor,
  editCommandLabel,
  wireOperation,
  type WireEditCommand,
} from "./tauriCommandMap";
import {
  buildAddSketch,
  buildAddSketchOnDatum,
  buildDeleteSketch,
  buildUpdateSketchAttachmentDatum,
  buildUpdateSketchAttachmentWorld,
  cloneIdMap,
  createIdMap,
  frontendConflictingIds,
  frontendConstraintsFromDto,
  frontendEntitiesFromDto,
  frontendEntityStates,
  frontendSolvedCurves,
  frontendSolvedPositions,
  marshalUpsert,
  mintUuid,
  seedIdMapFromWire,
  type SketchEditOp,
  type SketchIdMap,
} from "./sketchWireMap";
import { toWireDimensionValue } from "./angleUnits";
import { applyProjectionToStore } from "./projectionHydration";
import { documentStore, nextSketchName } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";

// ── Command + event names (must match src-tauri/src/api + events.rs) ──────────
const CMD = {
  listRecents: "list_recents",
  renameRecentProject: "rename_recent_project",
  deleteRecentProject: "delete_recent_project",
  revealInFileManager: "reveal_in_file_manager",
  newDocument: "new_document",
  openDocument: "open_document",
  importStep: "import_step",
  insertStep: "insert_step",
  importProject: "import_project",
  getModuleState: "get_module_state",
  setModuleState: "set_module_state",
  listDocumentModules: "list_document_modules",
  listVariables: "list_variables",
  upsertVariable: "upsert_variable",
  removeVariable: "remove_variable",
  closeDocument: "close_document",
  checkRecovery: "check_recovery",
  recoverDocument: "recover_document",
  saveDocument: "save_document",
  exportStepFile: "export_step_file",
  exportStlFile: "export_stl_file",
  exportObjFile: "export_obj_file",
  export3mfFile: "export_3mf_file",
  importFileDialog: "import_file_dialog",
  openFileDialog: "open_file_dialog",
  stepFileDialog: "step_file_dialog",
  saveFileDialog: "save_file_dialog",
  getMesh: "get_mesh",
  getProjection: "get_projection",
  applyEditCommand: "apply_edit_command",
  getOperationParams: "get_operation_params",
  featureDependencies: "feature_dependencies",
  canFoldTransform: "can_fold_transform",
  undo: "undo",
  redo: "redo",
  enterSketch: "enter_sketch",
  getSketch: "get_sketch",
  getSketchRegions: "get_sketch_regions",
  sketchUpsert: "sketch_upsert",
  finishSketch: "finish_sketch",
  cancelSketch: "cancel_sketch",
  beginGesture: "begin_gesture",
  solveDrag: "solve_drag",
  endGesture: "end_gesture",
  promoteSelection: "promote_selection",
  faceSketchPlane: "face_sketch_plane",
  addSketchOnFace: "add_sketch_on_face",
  elementInfo: "element_info",
  massProperties: "query_mass_properties",
  classifyElement: "classify_element",
  listLibraryComponents: "list_library_components",
  reindexLibrary: "reindex_library",
  resolveComponentSource: "resolve_component_source",
  placeComponent: "place_component",
  saveAsComponent: "save_as_component",
  componentPreviewMesh: "component_preview_mesh",
  listTemplates: "list_templates",
  saveAsTemplate: "save_as_template",
  newFromTemplate: "new_from_template",
  replaceComponent: "replace_component",
  componentUpgradeAvailable: "component_upgrade_available",
  setComponentParams: "set_component_params",
  detachComponent: "detach_component",
  prepareOffsetFace: "prepare_offset_face",
  prepareEdgeOp: "prepare_edge_op",
  analyzeEdgeOpRange: "analyze_edge_op_range",
  previewOp: "preview_op",
  resolveRefs: "resolve_refs",
  clearWorkerCircuit: "clear_worker_circuit",
  confirmExit: "confirm_exit",
  cancelExit: "cancel_exit",
} as const;

const EVT = {
  documentChanged: "document-changed",
  projectionUpdated: "projection-updated",
  regenStarted: "regen-started",
  regenFinished: "regen-finished",
  sketchSolved: "sketch-solved",
  workerStatus: "worker-status",
  needsRepair: "needs-repair",
  closeRequested: "close-requested",
} as const;

/** Local lane bookkeeping latency; exact Tauri L2 geometry comes from PreviewOp. */
const PREVIEW_LATENCY_MS = 16;

/**
 * `EditCommand`s that mutate body/sketch METADATA only. Rust maps every one of
 * them to `RegenHint::None` (edit/session.rs), so no regen runs, no
 * `document-changed` / `regen-finished` ever arrives, and the normal correlation
 * awaiter would sit out the full safety timeout. `applyEditCommand` takes the
 * command's own projection as the result instead. Keep this list in lockstep with
 * the `RegenHint::None` rows of that table.
 */
const METADATA_ONLY_CMDS: ReadonlySet<string> = new Set([
  "setVisibility",
  "renameBody",
  "renameSketch",
  "setBodyColor",
  // Datums are core-owned state that never crosses the OCW1 wire, so adding or
  // deleting one publishes a projection and fires NO regen either.
  "addDatumPlane",
  "deleteDatum",
]);

/**
 * Ultimate fallback for a regen's correlation if NEITHER `document-changed` nor
 * `regen-finished` arrives (should not happen — regen-finished fires on every
 * regen). Kept as a safety net; the prompt path is regen-finished.
 */
let regenTimeoutMs = 8000;
/** Test seam: shrink the correlation timeout so a no-event path resolves fast. */
export function __setRegenTimeoutForTests(ms: number): void {
  regenTimeoutMs = ms;
}

// ── Wire DTOs (camelCase; mirror src-tauri/src/dto.rs) ────────────────────────
interface DocumentSnapshotDto {
  documentId: string;
  title: string;
}
/** `RecoveryInfoDto` is field-identical to the frontend `RecoveryInfo`. */
interface RecoveryInfoDto {
  documentId: string;
  title?: string;
  originalPath?: string;
  autosavePath: string;
  modifiedMs: number;
}
interface DocumentProjectionDto extends DocumentProjectionWire {
  /** `FeatureDto` is field-identical to the frontend `FeatureRecord`. */
  features: FeatureRecord[];
}
interface SketchSessionDto {
  sketchId: string;
  plane: SketchPlane;
  entities: unknown;
  constraints: unknown;
  dof: number;
  status: SketchSolveStatus;
  /** Backend constraint uuids in conflict (SCHEMA §7.4); mapped to frontend ids. */
  conflicting?: string[];
  /** Per-entity constrained state keyed by backend ENTITY uuid (SCHEMA §7.4;
   *  Rust always serializes it, older payloads simply lack the key). Re-keyed to
   *  frontend ids here — unknown keys dropped. */
  entityStates?: SketchEntityStates;
}
/** `add_sketch_on_face` result (Rust `SketchOnFaceDto`; SKETCH-ON-FACE W1b).
 *  Deliberately lean — the sketch's geometry arrives through the normal
 *  `enter_sketch` path right after, so only the counts + the frozen frame ride
 *  here. `sketchId` echoes the id the CALLER minted (adopted verbatim). */
interface SketchOnFaceDto {
  sketchId: string;
  plane: Omit<SketchPlane, "kind">;
  entityCount: number;
  constraintCount: number;
  faceCount: number;
  hasClosedBoundary: boolean;
  projectedBoundaryVersion: number;
}
interface SketchUpsertDto {
  sketchId: string;
  sketchRevision: number;
  dof: number;
  status: SketchSolveStatus;
  /** Backend constraint uuids in conflict (SCHEMA §7.4); mapped to frontend ids. */
  conflicting?: string[];
  solvedPositions: Record<string, [number, number]>;
  /** CHANGED curve members, keyed by backend entity uuid (SCHEMA §7.4 `curves`;
   *  Rust always serializes it, older payloads simply lack the key). */
  solvedCurves?: Record<string, CurveParams>;
  /** Per-entity constrained state keyed by backend ENTITY uuid (SCHEMA §7.4). */
  entityStates?: SketchEntityStates;
}
interface BeginGestureDto {
  gestureId: number;
  ready: boolean;
  /** The GESTURE-FIXED per-entity map, keyed by backend ENTITY uuid (SCHEMA
   *  §7.4). `SolveDrag` carries none — `DragSolveDto` has no counterpart, and
   *  that omission is normative, not an oversight. */
  entityStates?: SketchEntityStates;
}
/** `BeginGesture.drag` as the Rust command deserializes it (`GestureTargetArgs`,
 *  camelCase): `entity` is a BACKEND uuid, `role` one of the §7.4 tokens. */
interface GestureTargetArgs {
  kind: GestureTarget["kind"];
  entity: string;
  role?: string;
  grab?: [number, number];
}
interface DragSolveDto {
  gestureId: number;
  seq: number;
  status: string;
  dof: number;
  conflicting: string[];
  positions: Record<string, [number, number]>;
  /** CHANGED curve members, keyed by backend entity uuid (SCHEMA §7.4 `curves`). */
  curves?: Record<string, CurveParams>;
  solveMicros: number;
  superseded: boolean;
}
interface SketchRegionDto {
  regionId: string;
  outerLoop: string[];
  holes: string[][];
  previewTriangles?: { positions: number[]; indices: number[]; holesSubtracted?: number };
}
interface FinishSketchDto {
  regionIdentityVersion: number;
  regions: SketchRegionDto[];
}
interface PromotedElementDto {
  topoKey: string;
  elementId: string;
  kind: string;
  bodyId: string;
}

/** Last `sketch-solved` payload seen (test/debug seam; the command return is the
 *  authoritative sync result — the event mirrors it for out-of-band subscribers). */
let lastSketchSolved: SketchUpsertDto | null = null;
/** Test seam: read the most recent `sketch-solved` event payload. */
export function __lastSketchSolvedForTests(): SketchUpsertDto | null {
  return lastSketchSolved;
}

/** Normalize a rejected command (Rust `ApiError {kind, message}`) into an Error. */
function toClientError(e: unknown): Error {
  if (e && typeof e === "object" && "kind" in e) {
    const { kind, message, diagnostics } = e as {
      kind: string;
      message?: string;
      diagnostics?: unknown;
    };
    const err = new Error(message ? `${kind}: ${message}` : kind);
    const typed = err as Error & { kind?: string; diagnostics?: OperationDiagnostic[] };
    typed.kind = kind;
    typed.diagnostics = parseOperationDiagnostics(diagnostics);
    return err;
  }
  return e instanceof Error ? e : new Error(String(e));
}

/**
 * Commands NOT logged by `call<T>` (DEV-OBSERVABILITY Wave F):
 *  • `solve_drag` fires per drag frame — logging it violates the no-drag-frequency
 *    policy and would bury every other event in the ring.
 *  • `log_event` IS the forwarding channel; logging it recurses.
 */
const IPC_LOG_EXEMPT = new Set<string>(["solve_drag", "log_event"]);

/**
 * A one-line SHAPE summary of the command args — never the payload. Ids and
 * flags are the correlation keys worth having; a sketch's entity array or a
 * mesh buffer is not, and would blow the ctx cap for nothing.
 */
function summarizeValue(v: unknown): unknown {
  if (v === null || v === undefined) return v;
  switch (typeof v) {
    case "string":
      return v.length > 80 ? `${v.slice(0, 80)}…` : v;
    case "number":
    case "boolean":
      return v;
    case "object":
      break;
    default:
      return `[${typeof v}]`;
  }
  if (Array.isArray(v)) return `Array(${v.length})`;
  if (ArrayBuffer.isView(v)) return `${v.constructor.name}(${v.byteLength}B)`;
  const keys = Object.keys(v as object);
  return `{${keys.slice(0, 8).join(",")}${keys.length > 8 ? ",…" : ""}}`;
}

function summarizeArgs(args?: Record<string, unknown>): Record<string, unknown> | undefined {
  if (args === undefined) return undefined;
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(args)) out[k] = summarizeValue(v);
  return out;
}

/**
 * Thin invoke wrapper that surfaces backend `ApiError`s as JS Errors.
 *
 * ALSO the frontend's single IPC chokepoint for observability: every real-lane
 * command round-trip lands one `ipc` event with its duration, so the FE side of
 * a commit lines up with the Rust `ipc::request::handler` span for the same
 * command. The mock lane has no Rust side to correlate with and keeps its
 * existing hand-placed trace sites.
 */
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const t0 = performance.now();
  try {
    const result = await invoke<T>(cmd, args);
    if (!IPC_LOG_EXEMPT.has(cmd)) {
      logDebug("ipc", cmd, {
        cmd,
        durMs: Math.round((performance.now() - t0) * 100) / 100,
        args: summarizeArgs(args),
      });
    }
    return result;
  } catch (e) {
    const err = toClientError(e);
    if (!IPC_LOG_EXEMPT.has(cmd)) {
      logWarn("ipc", `${cmd} FAILED`, {
        cmd,
        durMs: Math.round((performance.now() - t0) * 100) / 100,
        kind: (err as Error & { kind?: string }).kind ?? err.name,
        message: err.message,
      });
    }
    throw err;
  }
}

export function createTauriClient(): CadClient {
  // Fan-out for app-level subscribers.
  const docChangeListeners = new Set<(c: DocumentChange) => void>();
  const projectionListeners = new Set<(p: DocumentProjectionWire) => void>();
  const workerStatusListeners = new Set<(s: WorkerStatus) => void>();
  const needsRepairListeners = new Set<(e: NeedsRepairEvent) => void>();
  const closeRequestedListeners = new Set<() => void>();
  // Latest published snapshot id for promote_selection — carried by every
  // `document-changed` event (SCHEMA §7.5). Picks resolve against the snapshot the
  // fetched mesh was tessellated at (Invariant 4). Starts at 0 (nothing published).
  let currentSnapshotId = 0;

  // ── Commit-result correlation (MODEL-HARDEN W0.5 + hardening) ────────────────
  // An awaiter for a commit whose post-apply projection revision is R resolves on the
  // completion that covers R. There are TWO awaiter classes:
  //
  //  • NON-recordId (undo / redo / raw EditCommand): resolves on the FIRST covering
  //    signal — document-changed{revision >= R} (bodies) or regen-finished{failed|noop}
  //    (empty/message). published document-changed carries the bodies, so a published
  //    regen-finished is ignored for this class.
  //
  //  • recordId (a fresh AddOperation via applyOperation): a published regen can contain
  //    THIS op's FAILED step while still publishing OTHER records' bodies via
  //    document-changed. So this class NEVER settles off document-changed alone — it
  //    buffers the covering change and settles on the covering regen-finished sibling,
  //    which is authoritative: `failedSteps` containing our recordId ⇒ FAILURE (empty +
  //    that step's message) even though bodies were published; otherwise SUCCESS scoped
  //    to `affectedBodies[recordId]` (finding 1 + 2). Exactly-once: a recordId-awaiter
  //    settles only in resolveRegenFinished; document-changed only buffers it.
  //
  //  • documentScope (W5 — the variable commands): the edit names no single record
  //    because it dirties the WHOLE timeline (`variable_outcome` ⇒ `DirtyRange(0,
  //    len)`), so every step really is rebuilt by it and ANY failed step is this
  //    edit's own failure — there is nothing to misattribute. Same mechanics as the
  //    recordId class (buffer the change, settle on the covering regen-finished),
  //    only the failure predicate widens from "my record" to "any step". Scoping is
  //    likewise skipped: the change stays the full list.
  //
  // Under rapid commits a stale earlier report can never resolve a LATER commit: its
  // `sourceRevision` is below that commit's R (or it is superseded and ignored).
  interface Resolved {
    change: DocumentChange | null;
    revision: number;
    terminal: RegenTerminal;
    errorMessage?: string;
    diagnostics?: OperationDiagnostic[];
  }
  interface Awaiter {
    targetRev: number | null; // R; null until the command returns its projection
    recordId?: string; // set for a fresh AddOperation commit (finding 1/2 scoping)
    documentScope?: boolean; // W5: any failed step is this edit's failure
    pendingChange: DocumentChange | null; // buffered covering change (recordId path)
    resolve(r: Resolved | null): void;
    timer: ReturnType<typeof setTimeout>;
  }
  /** True for the two classes that settle on regen-finished, not on the change. */
  const settlesOnFinish = (a: Awaiter): boolean =>
    a.recordId !== undefined || a.documentScope === true;
  /** Which correlation class an edit opts into. Empty ⇒ the first-covering-signal
   *  default (undo/redo and the raw commands that name no record). */
  interface ApplyEditScope extends Pick<Awaiter, "recordId" | "documentScope"> {
    /**
     * "This command fired NO regen" — read off its own returned projection, after
     * the fact, for a command whose `RegenHint` is CONDITIONAL.
     *
     * `METADATA_ONLY_CMDS` handles the static case; this handles the dynamic one.
     * A variable edit is `RegenHint::ToEnd` over a real timeline but
     * `metadata_only` when the timeline is EMPTY (`variable_outcome`), so nothing
     * is enqueued and no `regen-finished` will ever arrive — the awaiter would sit
     * out the full safety timeout and then report a `timeout` FAILURE for the most
     * ordinary act there is: adding the first variable to a fresh document.
     */
    noRegenWhen?: (projection: DocumentProjectionDto) => boolean;
  }
  const awaiters = new Set<Awaiter>();
  // The newest published change — buffered so a covering publish landing in the
  // window between the command returning and `setTarget` still resolves the awaiter.
  let lastPublishedChange: DocumentChange | null = null;

  function settle(a: Awaiter, r: Resolved): void {
    clearTimeout(a.timer);
    awaiters.delete(a);
    a.resolve(r);
  }

  /**
   * Scope a covering change to ONLY this record's bodies (finding 2). With
   * `affectedBodies[recordId]` present, changedBodies becomes exactly those ids —
   * mapped to their meshKey-bearing ref from the document-changed payload (or a
   * synthesized ref when the id is not in it). Absent ⇒ the full change (mock lane).
   */
  function scopeChange(
    change: DocumentChange | null,
    ids: string[] | undefined,
    revision: number,
  ): DocumentChange | null {
    if (ids === undefined) return change; // fallback: today's full list
    const changedBodies = ids.map(
      (id) => change?.changedBodies.find((b) => b.bodyId === id) ?? { bodyId: id, meshKey: id },
    );
    return { revision, changedBodies, removedBodies: change?.removedBodies ?? [] };
  }

  /** A published document-changed. NON-recordId awaiters settle off it; recordId
   *  awaiters only BUFFER it (they settle on the regen-finished sibling). */
  function resolvePublished(change: DocumentChange): void {
    for (const a of [...awaiters]) {
      if (a.targetRev === null || change.revision < a.targetRev) continue;
      if (settlesOnFinish(a)) {
        a.pendingChange = change; // authoritative outcome comes with regen-finished
      } else {
        settle(a, { change, revision: change.revision, terminal: "published" });
      }
    }
  }

  /** A regen-finished terminal. superseded/cancelled are ignored for every awaiter
   *  (a newer regen covers R). For a NON-recordId awaiter, published is ignored too
   *  (its document-changed sibling carries the bodies). A recordId — or W5
   *  documentScope — awaiter settles here on ANY covering completion: failedSteps ⇒
   *  failure, else scoped success. */
  function resolveRegenFinished(rf: RegenFinished): void {
    if (rf.outcome === "superseded" || rf.outcome === "cancelled") return;
    const source = rf.sourceRevision ?? rf.revision;
    for (const a of [...awaiters]) {
      if (a.targetRev === null || source < a.targetRev) continue;
      if (settlesOnFinish(a)) {
        // Which failure counts: THIS record's step for a recordId awaiter, ANY step
        // for a documentScope one (its edit dirtied every step, so every step's
        // failure is its own).
        const failed =
          a.recordId !== undefined
            ? rf.failedSteps?.find((s) => s.recordId === a.recordId)
            : rf.failedSteps?.[0];
        if (failed || rf.outcome === "failed") {
          // This op's step errored (even if the regen published other bodies), or the
          // whole regen failed ⇒ FAILURE: empty bodies + the reason.
          settle(a, {
            change: null,
            revision: rf.revision,
            terminal: "failed",
            errorMessage: failed?.message ?? rf.message,
            diagnostics: failed?.diagnostics ?? rf.diagnostics,
          });
        } else {
          // Success — scope to this op's own bodies (incl. split children).
          // A record left in NeedsRepair published no geometry of its own, but it
          // did not fail either: it settles as `needsRepair` so the consumer shows
          // the repair affordance instead of a success hint. Never an error.
          // A documentScope awaiter names no record, so there is nothing to scope
          // to and nothing to look up in `repairSteps` — it takes the full change.
          const change = a.pendingChange ?? lastPublishedChange;
          const record = a.recordId;
          const needsRepair =
            record !== undefined && (rf.repairSteps?.includes(record) ?? false);
          settle(a, {
            change:
              record === undefined
                ? change
                : scopeChange(change, rf.affectedBodies?.[record], rf.revision),
            revision: rf.revision,
            terminal: needsRepair
              ? "needsRepair"
              : rf.outcome === "noop"
                ? "noop"
                : "published",
          });
        }
      } else if (rf.outcome !== "published") {
        // failed / noop for a raw command → empty bodies (+ message on failure).
        settle(a, {
          change: null,
          revision: rf.revision,
          terminal: rf.outcome === "failed" ? "failed" : "noop",
          errorMessage: rf.outcome === "failed" ? rf.message : undefined,
          diagnostics: rf.outcome === "failed" ? rf.diagnostics : undefined,
        });
      }
    }
  }

  /** Document replacement (new/open/import/recover): drop every buffered change +
   *  pending awaiter so the NEW document's first commit can never settle off the
   *  OLD document's state (finding 3). */
  function resetCorrelation(): void {
    lastPublishedChange = null;
    currentSnapshotId = 0;
    // …and any in-flight regen: the driver returns early (without emitting) when
    // the document closes mid-drive, so a busy count kept across a replacement
    // would stick at 1 and leave "Rebuilding…" on screen forever.
    documentStore.getState().regenIdle();
    for (const a of [...awaiters]) {
      clearTimeout(a.timer);
      awaiters.delete(a);
      a.resolve(null);
    }
  }

  function onDocumentChangedEvent(change: DocumentChange): void {
    trace(
      "ipc",
      `document-changed: rev=${change.revision} changed=[${change.changedBodies.map((b) => b.bodyId).join(", ")}] ` +
        `removed=[${change.removedBodies.join(", ")}]`,
    );
    // Adopt the published snapshot id so promoteSelection scopes picks correctly.
    if (change.snapshotId && change.snapshotId > 0) currentSnapshotId = change.snapshotId;
    lastPublishedChange = change;
    for (const cb of [...docChangeListeners]) cb(change);
    resolvePublished(change);
  }

  function onRegenStartedEvent(): void {
    documentStore.getState().regenStarted();
  }

  /**
   * Prefix of the sticky status-bar hint {@link onRegenFinishedEvent} raises for an
   * UNCORRELATED regen failure. Exported so the handler can recognize — and a test
   * can pin — that a later clean publish clears exactly this hint and no other.
   */
  const REGEN_FAILED_HINT_PREFIX = "Geometry rebuild failed — ";

  function onRegenFinishedEvent(rf: RegenFinished): void {
    // Paired with `regen-started`, CLAMPED at zero in the store: a no-op regen
    // finishes without ever having started, so completions outnumber starts.
    documentStore.getState().regenSettled();
    const failed = rf.failedSteps ?? [];
    if (failed.length > 0 || rf.outcome === "failed") {
      traceWarn(
        "ipc",
        `regen-finished: outcome=${rf.outcome} rev=${rf.revision} srcRev=${rf.sourceRevision ?? "?"} ` +
          `message=${rf.message ?? "none"} failedSteps=${failed.length}`,
        failed,
      );
      // An UNCORRELATED failure (no commit awaiter — e.g. the open-regen replay,
      // or a worker-restart replay) has no controller to surface it: without this
      // hint the only sign is a tinted history row nobody is looking at, and the
      // viewport just quietly shows nothing.
      if (awaiters.size === 0) {
        const reason = rf.message ?? failed[0]?.message ?? "see the history list";
        viewportStore.getState().setStatusHint(`${REGEN_FAILED_HINT_PREFIX}${reason}`, {
          severity: "error",
          sticky: true,
        });
      }
    } else {
      trace(
        "ipc",
        `regen-finished: outcome=${rf.outcome} rev=${rf.revision} srcRev=${rf.sourceRevision ?? "?"}`,
      );
      // Only a clean publish clears the sticky failure hint (this branch already
      // excludes publishes carrying failedSteps): superseded/cancelled/noop rebuilt
      // nothing, so the error must stay on screen. Clear only OUR hint — the
      // exitIsolate discipline.
      if (
        rf.outcome === "published" &&
        viewportStore.getState().statusHint?.message.startsWith(REGEN_FAILED_HINT_PREFIX)
      ) {
        viewportStore.getState().setStatusHint(null);
      }
    }
    resolveRegenFinished(rf);
  }

  function onProjectionUpdatedEvent(p: DocumentProjectionWire): void {
    applyProjectionToStore(p); // hydrate documentStore (revision-reconciled)
    for (const cb of [...projectionListeners]) cb(p);
  }

  function onWorkerStatusEvent(s: WorkerStatus): void {
    for (const cb of [...workerStatusListeners]) cb(s);
  }

  function onNeedsRepairEvent(e: NeedsRepairEvent): void {
    for (const cb of [...needsRepairListeners]) cb(e);
  }

  function onCloseRequestedEvent(): void {
    for (const cb of [...closeRequestedListeners]) cb();
  }

  /** Await the correlated regen completion for a commit. Register BEFORE invoking
   *  (so no event is missed), then `setTarget(R)` once the projection revision is
   *  known. `recordId` (a fresh AddOperation) opts into the failedSteps / affectedBodies
   *  scoping path; `documentScope` (W5) opts into the any-failed-step one. Resolves to
   *  null on the safety timeout. */
  function awaitNextChange(opts: ApplyEditScope = {}): {
    promise: Promise<Resolved | null>;
    setTarget(rev: number): void;
    cancel(): void;
  } {
    let awaiter!: Awaiter;
    const promise = new Promise<Resolved | null>((resolve) => {
      const timer = setTimeout(() => {
        awaiters.delete(awaiter);
        resolve(null);
      }, regenTimeoutMs);
      awaiter = {
        targetRev: null,
        recordId: opts.recordId,
        documentScope: opts.documentScope,
        pendingChange: null,
        resolve,
        timer,
      };
      awaiters.add(awaiter);
    });
    return {
      promise,
      setTarget(rev: number) {
        awaiter.targetRev = rev;
        // Missed-event race: a covering publish may already have arrived between the
        // command returning and this call — reconcile against the buffer.
        if (lastPublishedChange && lastPublishedChange.revision >= rev) {
          if (settlesOnFinish(awaiter)) {
            // recordId / documentScope settle on the regen-finished sibling — buffer only.
            awaiter.pendingChange = lastPublishedChange;
          } else {
            settle(awaiter, {
              change: lastPublishedChange,
              revision: lastPublishedChange.revision,
              terminal: "published",
            });
          }
        }
      },
      cancel() {
        clearTimeout(awaiter.timer);
        awaiters.delete(awaiter);
      },
    };
  }

  // Persistent event listeners. Memoized as a PROMISE (not a started-flag): the
  // old boolean let a subscriber registered while `listen()` was still pending
  // believe the bridge was live — any backend event in that window (e.g. the
  // open-regen's `document-changed`) was silently lost, and a lost mesh event
  // means a body that never renders. Await the promise wherever ordering
  // matters; a bridge failure resolves `false` (and retries on the next call)
  // instead of vanishing.
  let eventsReady: Promise<boolean> | null = null;
  const unlisteners: UnlistenFn[] = [];
  function ensureEvents(): Promise<boolean> {
    eventsReady ??= (async () => {
      try {
        unlisteners.push(
          await listen<DocumentChange>(EVT.documentChanged, (e) => onDocumentChangedEvent(e.payload)),
          await listen<DocumentProjectionDto>(EVT.projectionUpdated, (e) => onProjectionUpdatedEvent(e.payload)),
          await listen<void>(EVT.regenStarted, () => onRegenStartedEvent()),
          await listen<RegenFinished>(EVT.regenFinished, (e) => onRegenFinishedEvent(e.payload)),
          await listen<SketchUpsertDto>(EVT.sketchSolved, (e) => {
            lastSketchSolved = e.payload;
          }),
          await listen<WorkerStatus>(EVT.workerStatus, (e) => onWorkerStatusEvent(e.payload)),
          await listen<NeedsRepairEvent>(EVT.needsRepair, (e) => onNeedsRepairEvent(e.payload)),
          await listen<void>(EVT.closeRequested, () => onCloseRequestedEvent()),
        );
        return true;
      } catch (e) {
        // A missing event bridge must not break command-only flows (vitest mocks
        // invoke only) — but it must not be silent either: without events no
        // mesh ever refreshes.
        traceWarn("ipc", "event bridge failed to attach (will retry on next use)", e);
        eventsReady = null;
        return false;
      }
    })();
    return eventsReady;
  }
  // Eager start: the pre-regen `projection-updated` of an open/new fires DURING
  // the command round-trip, so the listeners must be attaching before any
  // command can possibly be invoked on this client.
  void ensureEvents();

  /**
   * Run an edit command and correlate its regen into an ApplyOperationResult.
   *
   * `recordId` (a fresh AddOperation's minted id, or an existing record a re-edit
   * targets) opts the awaiter into the failedSteps / affectedBodies scoping path
   * (findings 1/2); `documentScope` (W5) opts into the any-failed-step path used
   * by edits that dirty the whole timeline.
   *
   * `unwrap` exists for the ONE command whose payload is not a bare projection
   * (`replace_component` returns `{ projection, report }`): the correlation is
   * identical, only the envelope differs, so it is a parameter rather than a
   * second copy of this function. `applyEdit` below is the identity case.
   */
  async function applyEditRaw<T>(
    cmd: string,
    args: Record<string, unknown>,
    opLabel: string | undefined,
    scope: ApplyEditScope,
    unwrap: (raw: T) => DocumentProjectionDto,
  ): Promise<{ raw: T; result: ApplyOperationResult }> {
    const { recordId } = scope;
    await ensureEvents();
    trace("ipc", `applyEdit: cmd=${cmd} record=${recordId ?? "none"} label=${opLabel ?? "?"}`);
    const awaiter = awaitNextChange(scope);
    let raw: T;
    let projection: DocumentProjectionDto;
    try {
      raw = await call<T>(cmd, args);
      projection = unwrap(raw);
    } catch (e) {
      awaiter.cancel();
      traceWarn("ipc", `applyEdit: cmd=${cmd} record=${recordId ?? "none"} invoke THREW`, e);
      throw e;
    }
    // Conditionally regen-less (see `ApplyEditScope.noRegenWhen`): the command
    // succeeded and enqueued nothing, so the pre-regen projection IS the whole
    // answer. `noop`, never `timeout` — nothing failed, there was simply nothing
    // to rebuild.
    if (scope.noRegenWhen?.(projection) === true) {
      awaiter.cancel();
      trace("ipc", `applyEdit: cmd=${cmd} fired no regen — settling noop off the projection`);
      return {
        raw,
        result: {
          revision: projection.revision,
          changedBodies: [],
          removedBodies: [],
          features: projection.features,
          opLabel,
          appliedOps: projection.appliedOps,
          totalOps: projection.totalOps,
          terminal: "noop",
        },
      };
    }
    // Finding 4 / H7a: a fresh op joins the timeline AT the rollback cursor
    // (`atCursor: true`, unconditional — see `operationToEditCommand`), so under
    // normal operation it lands inside the applied prefix and regenerates. The
    // POSITIONAL check below (not a count comparison) is what actually decides
    // "inert": a count-only check (`appliedOps < totalOps`) would also fire for
    // this record's own legitimate mid-history insert, since a rolled-back
    // document still has OTHER (unrelated, pre-existing) drafts beyond the new
    // cursor — that count staying nonzero is expected and must not be reported
    // as this commit's failure. Only THIS record's own index relative to
    // appliedOps tells whether it landed inside the applied prefix (regenerates)
    // or past it (permanently inert — a safety net; should not occur once every
    // authoring path sends atCursor:true, but a stale build or a future
    // append-based caller must still be caught here rather than stall on the
    // regen-correlation timeout).
    if (recordId !== undefined) {
      const idx = projection.features.findIndex((f) => f.id === recordId);
      const appliedOps = projection.appliedOps;
      const inert = idx >= 0 && typeof appliedOps === "number" && idx >= appliedOps;
      if (inert) {
        awaiter.cancel();
        traceWarn(
          "ipc",
          `applyEdit: cmd=${cmd} record=${recordId ?? "none"} added PAST the cursor ` +
            `(index=${idx}, appliedOps=${appliedOps}) — no regen fires`,
        );
        return {
          raw,
          result: {
            revision: projection.revision,
            changedBodies: [],
            removedBodies: [],
            features: projection.features,
            opLabel,
            appliedOps: projection.appliedOps,
            totalOps: projection.totalOps,
            errorMessage: "Operation added while history is rolled back — roll forward to apply",
            terminal: "noop",
          },
        };
      }
      // A commit that landed INSIDE the applied prefix but with drafts still
      // beyond it means the document was rolled back when it was authored (a
      // frontier commit always leaves appliedOps === totalOps). It is NOT inert
      // — it regenerates normally — but "added a feature and nothing visibly
      // changed at the end of the tree" reads as a no-op without this hint.
      if (
        idx >= 0 &&
        typeof appliedOps === "number" &&
        typeof projection.totalOps === "number" &&
        appliedOps < projection.totalOps
      ) {
        viewportStore.getState().setStatusHint(
          `Inserted at step ${idx + 1} of ${projection.totalOps} — roll to end to rebuild later features`,
          { severity: "info" },
        );
      }
    }
    // Correlate the regen against THIS commit's post-apply revision R (exact under
    // rapid commits — see the awaiter table). Synchronous right after the await, so
    // no event can interleave before the target is set.
    awaiter.setTarget(projection.revision);
    const resolved = await awaiter.promise;
    if (resolved === null) {
      traceWarn(
        "ipc",
        `applyEdit: cmd=${cmd} record=${recordId ?? "none"} correlation TIMED OUT (rev=${projection.revision})`,
      );
    } else {
      trace(
        "ipc",
        `applyEdit: cmd=${cmd} record=${recordId ?? "none"} resolved rev=${resolved.revision} ` +
          `changed=${resolved.change?.changedBodies.length ?? 0} error=${resolved.errorMessage ?? "none"}`,
      );
    }
    const result: ApplyOperationResult = {
      revision: resolved?.revision ?? projection.revision,
      changedBodies: resolved?.change?.changedBodies ?? [],
      removedBodies: resolved?.change?.removedBodies ?? [],
      // Features stay sourced from the pre-regen command projection: the
      // projection-updated listener hydrates the store with the post-regen timeline
      // independently, and the mock lane resolves synchronously (no post-regen
      // projection to swap in). A correlated FAILURE additionally threads its reason.
      features: projection.features,
      opLabel,
      // H7b: the CURSOR rides with those same features. A rollback's whole visible
      // effect is the cursor move, and waiting for the post-regen projection left
      // the rows un-grayed for a full worker round-trip (and, for a `RegenHint::
      // None` command, forever).
      appliedOps: projection.appliedOps,
      totalOps: projection.totalOps,
      terminal: resolved?.terminal ?? "timeout",
    };
    if (resolved?.errorMessage) result.errorMessage = resolved.errorMessage;
    if (resolved?.diagnostics) result.diagnostics = resolved.diagnostics;
    return { raw, result };
  }

  /** The identity case of {@link applyEditRaw} — the command's payload IS the
   *  projection, which is every edit command but `replace_component`. */
  async function applyEdit(
    cmd: string,
    args: Record<string, unknown>,
    opLabel: string | undefined,
    scope: ApplyEditScope = {},
  ): Promise<ApplyOperationResult> {
    const { result } = await applyEditRaw<DocumentProjectionDto>(
      cmd,
      args,
      opLabel,
      scope,
      (p) => p,
    );
    return result;
  }

  /**
   * The two variable WRITES (W5 result truth) — `upsert_variable` /
   * `remove_variable`, which were the last kernel-touching commands exempt from
   * the `regenOutcome` doctrine.
   *
   * Ordinary `applyEdit`, plus the one thing that makes the result honest: the
   * saved table is lifted off the projection the command returned, so a
   * `terminal: "failed"` carries the variable the document REALLY holds. The
   * write is never rolled back and never rethrown — the variable was saved; only
   * the rebuild that followed failed, and the caller reports both.
   */
  async function variableEdit(
    cmd: string,
    args: Record<string, unknown>,
    opLabel: string,
  ): Promise<VariableEditResult> {
    const { raw, result } = await applyEditRaw<DocumentProjectionDto>(
      cmd,
      args,
      opLabel,
      {
        documentScope: true,
        // `variable_outcome` is `metadata_only` on an EMPTY timeline and
        // `dirty_to_end` otherwise, so `totalOps === 0` is the exact mirror of
        // "no regen was enqueued" — the same fact the backend decided on, read
        // off the projection it returned rather than guessed at.
        noRegenWhen: (p) => p.totalOps === 0,
      },
      (p) => p,
    );
    return { ...result, variables: raw.variables ?? [] };
  }

  /**
   * `insert_step` — append a STEP import to the OPEN document (STEP-IMPORT WP-A).
   *
   * Same correlation contract as `applyEdit` (register the awaiter BEFORE invoking,
   * then target the returned projection's revision), with ONE difference that is
   * why it cannot just delegate: Rust owns the file dialog, so a CANCELLED dialog
   * comes back as a null projection with no revision to correlate against. The
   * awaiter must be cancelled there or it would sit out the full safety timeout.
   */
  async function insertStep(): Promise<ApplyOperationResult | null> {
    await ensureEvents();
    const awaiter = awaitNextChange();
    let projection: DocumentProjectionDto | null;
    try {
      projection = await call<DocumentProjectionDto | null>(CMD.insertStep);
    } catch (e) {
      awaiter.cancel();
      traceWarn("ipc", "insertStep: invoke THREW", e);
      throw e;
    }
    if (!projection) {
      awaiter.cancel(); // dialog cancelled — nothing was appended, no regen fires
      return null;
    }
    awaiter.setTarget(projection.revision);
    const resolved = await awaiter.promise;
    const result: ApplyOperationResult = {
      revision: resolved?.revision ?? projection.revision,
      changedBodies: resolved?.change?.changedBodies ?? [],
      removedBodies: resolved?.change?.removedBodies ?? [],
      // Pre-regen timeline, as in `applyEdit`: projection-updated hydrates the
      // post-regen one independently.
      features: projection.features,
      opLabel: "Import",
      terminal: resolved?.terminal ?? "timeout",
    };
    if (resolved?.errorMessage) result.errorMessage = resolved.errorMessage;
    return result;
  }

  async function applyOperation(op: OperationOp): Promise<ApplyOperationResult> {
    const command = operationToEditCommand(op);
    // BOTH shapes name a timeline record, so both opt into failedSteps /
    // repairSteps / affectedBodies correlation: a fresh op mints its recordId
    // (`addOperation.record.recordId`), a parametric re-edit targets an existing
    // one (`updateOperationParams.record`).
    //
    // Before U1 only the fresh shape did. `failed_steps_of` keys on `rec.record_id`
    // for ANY errored record (document_runtime.rs), so the re-edit correlation was
    // available all along and simply unused — which meant a published-overall regen
    // whose FAILING step was this very record settled as a success, and every
    // re-edit family then reported it as one. No wire change was needed.
    const recordId =
      command.cmd === "addOperation"
        ? command.record.recordId
        : command.cmd === "updateOperationParams"
          ? command.record
          : undefined;
    return applyEdit(CMD.applyEditCommand, { command }, opLabelFor(op), { recordId });
  }

  // ── Sketch solver lane state (frontend id ↔ backend UUID via sketchWireMap) ──
  const sketchMaps = new Map<string, SketchIdMap>();

  /** The frontend-facing id for an enter target (kept as the session id). With no
   *  explicit `sketchId`, mint a real UUID up front (not a display-only `sk-N`) so
   *  it can double as the backend `SketchId` — see `ensureBackendSketch`'s `adoptId`. */
  function frontendIdFor(target: EnterSketchTarget): string {
    if (typeof target === "string") return target;
    return target.sketchId ?? mintUuid();
  }

  /**
   * What a NEW sketch is attached to, when it is not a bare world plane. Both
   * variants carry a `plane` that the BACKEND authored (`face_sketch_plane` /
   * `resolve_datum_frame`) and that rides the wire as `plane.kind: "custom"`.
   */
  type SketchHost =
    | {
        kind: "face";
        bodyId: string;
        elementId: string;
        topoKey?: string;
        plane: SketchPlane;
        worldPoint?: [number, number, number];
      }
    | { kind: "datum"; datumId: string; plane: SketchPlane };

  /** Resolve (or lazily create) the backend sketch for a frontend id + plane.
   *  `adoptId`, when given, IS used as the backend `SketchId` instead of minting a
   *  second UUID — the `{newOnPlane}`-without-`sketchId` lane passes its already-
   *  minted `frontendId` here so `session.sketchId` matches the projection-store
   *  key (Rust keys sketches by backend UUID). The explicit-`sketchId` lane passes
   *  no `adoptId` and keeps minting a separate backend UUID, unchanged. */
  async function ensureBackendSketch(
    frontendId: string,
    planeKind: SketchPlaneKind,
    adoptId?: string,
    host?: SketchHost,
  ): Promise<SketchIdMap> {
    const existing = sketchMaps.get(frontendId);
    if (existing) return existing;
    // New sketch: mint a real SketchId (or adopt the caller's), register it via
    // AddSketch, then enter.
    const backendSketchId = adoptId ?? mintUuid();
    const map = createIdMap(backendSketchId, planeKind);
    sketchMaps.set(frontendId, map);
    const name = nextSketchName(documentStore.getState().sketches);
    // A HOST-FACE sketch is created by its OWN command (SKETCH-ON-FACE W2), not by
    // an `AddSketch` edit assembled here. `add_sketch_on_face` does what the
    // frontend structurally cannot: two worker round-trips (the kernel-exact face
    // frame, then the boundary projected in the basis Rust derives from it) before
    // a single `AddSketch` carrying the projected boundary as `referenceLocked`
    // entities pinned by `Fixed` constraints. Building the command here instead
    // would seed an EMPTY host-face sketch with `projectedBoundaryVersion: 0`.
    //
    // The frontend-minted `backendSketchId` is adopted VERBATIM as the document
    // SketchId (the id-adoption rule), so the follow-up `enter_sketch` below reads
    // back the very sketch this created — projected entities included.
    if (host?.kind === "face") {
      await call<SketchOnFaceDto>(CMD.addSketchOnFace, {
        snapshotId: currentSnapshotId,
        // The `body_<uuid>` worker wire form (`wire::parse_body_id`), same as
        // faceSketchPlane / promoteSelection.
        bodyId: host.bodyId.startsWith("body_") ? host.bodyId : `body_${host.bodyId}`,
        elementId: host.elementId,
        topoKey: host.topoKey ?? null,
        sketchId: backendSketchId,
        name,
      });
      return map;
    }
    // Create the backend sketch. This fires an edit + regen; the sketch appears in
    // the tree via projection-updated hydration.
    const command =
      host?.kind === "datum"
        ? // DATUM W1: a `custom` basis on the wire plus a typed attachment. The
          // basis is the datum's backend-RESOLVED frame, carried verbatim (never
          // re-derived here) and FROZEN with the sketch (V1 policy).
          buildAddSketchOnDatum(backendSketchId, name, host.plane, host.datumId)
        : buildAddSketch(backendSketchId, name, planeKind);
    await call<DocumentProjectionDto>(CMD.applyEditCommand, { command });
    return map;
  }

  // One preview lane owns pacing/session state. Exact meshes come from PreviewOp;
  // commit routes the same candidate through apply_edit_command.
  const lane = createLocalSolverLane({
    commit: applyOperation,
    latencyMs: () => PREVIEW_LATENCY_MS,
    // MODEL-OPS W3: the drag-time exact mesh now comes from the REAL kernel.
    // Nothing is committed — the worker runs the op on a throwaway copy of its
    // head — so a Cut preview actually subtracts and every op is previewable.
    previewOp: async (op, sketchId) => {
      const res = await call<{
        snapshotId: number;
        bodies: { bodyId: string; mesh: number[] | ArrayBuffer }[];
        changedBodies?: string[];
        deletedBodies?: string[];
        needsRepair?: unknown[];
      }>(CMD.previewOp, {
        op: wireOperation(op),
        sketchId: sketchId ?? null,
        lod: "coarse",
        expectedSnapshotId: currentSnapshotId || null,
      });
      return {
        bodies: res.bodies.map((b) => ({
          bodyId: b.bodyId,
          mesh: b.mesh instanceof ArrayBuffer ? b.mesh : new Uint8Array(b.mesh).buffer,
        })),
        changedBodies: res.changedBodies,
        deletedBodies: res.deletedBodies,
        needsRepair: res.needsRepair,
      };
    },
  });

  async function enterSketch(target: EnterSketchTarget): Promise<SketchSession> {
    await ensureEvents();
    const frontendId = frontendIdFor(target);
    const planeKind: SketchPlaneKind =
      typeof target === "string"
        ? "XY"
        : "newOnFace" in target || "newOnDatum" in target
          ? "custom"
          : target.newOnPlane;
    // Re-entry after reopen: a persisted sketch id is present in the projection
    // store but the in-memory id-map is empty (documentStore.sketches keys ARE valid
    // backend SketchId strings — the Rust projection uses `id.to_string()`). Adopt
    // the id AS the backend SketchId and enter it directly — do NOT mint a new id /
    // fire AddSketch, which would create a duplicate EMPTY sketch beside the real one.
    // Plane "XY" is only the map's bookkeeping default; the real plane comes back in
    // the enter_sketch DTO.
    let map: SketchIdMap;
    // Track whether THIS call committed a fresh AddSketch (a new backend sketch that
    // will be orphaned if the follow-up enter_sketch rejects). Creations are
    // serialized upstream by the SketchController's `entering` flag, so no concurrent
    // AddSketch races this bookkeeping.
    let firedAddSketch = false;
    if (
      typeof target === "string" &&
      !sketchMaps.has(frontendId) &&
      documentStore.getState().sketches[frontendId]
    ) {
      map = createIdMap(frontendId, "XY");
      sketchMaps.set(frontendId, map);
    } else {
      // No explicit sketchId → frontendId is a freshly minted UUID (frontendIdFor);
      // adopt it as the backend SketchId too so the created sketch's session.sketchId
      // (== frontendId, returned below) matches the projection-store key.
      const adoptId = typeof target !== "string" && target.sketchId === undefined ? frontendId : undefined;
      // ensureBackendSketch fires AddSketch ONLY when there is no existing map.
      firedAddSketch = !sketchMaps.has(frontendId);
      let host: SketchHost | undefined;
      if (typeof target !== "string" && "newOnFace" in target) {
        host = {
          kind: "face",
          bodyId: target.newOnFace.bodyId,
          elementId: target.newOnFace.elementId,
          topoKey: target.newOnFace.topoKey,
          worldPoint: target.newOnFace.worldPoint,
          plane: target.plane,
        };
      } else if (typeof target !== "string" && "newOnDatum" in target) {
        host = { kind: "datum", datumId: target.newOnDatum.datumId, plane: target.plane };
      }
      map = await ensureBackendSketch(frontendId, planeKind, adoptId, host);
    }
    let dto: SketchSessionDto;
    try {
      dto = await call<SketchSessionDto>(CMD.enterSketch, { sketchId: map.backendSketchId });
    } catch (e) {
      // The AddSketch already committed but enter_sketch rejected (e.g. worker down):
      // the empty sketch is orphaned in the document, and every retry would mint
      // another. AWAIT a DeleteSketch compensation to remove it before rethrowing so
      // the tree stays clean and the id-map doesn't leak.
      if (firedAddSketch) {
        try {
          await deleteSketch(frontendId);
        } catch (cleanupErr) {
          logError("sketch", "enterSketch orphan cleanup (DeleteSketch) FAILED", {
            sketchId: map.backendSketchId,
            error: cleanupErr,
          });
          const orig = e instanceof Error ? e : new Error(String(e));
          orig.message = `${orig.message} (cleanup failed — empty sketch left in tree)`;
          throw orig;
        }
      }
      throw e;
    }
    const entities = frontendEntitiesFromDto(dto.entities);
    // Re-entry hydration: the backend returns the sketch's real constraints in the
    // worker-wire form (Rust `wire_constraint`, field-identical to SketchConstraint);
    // reverse-map them so the inspector shows live constraints (kills the []-seam).
    const constraints = frontendConstraintsFromDto(dto.constraints, dto.entities);
    // Seed the id-map from the returned wire so the NEXT upsert diffs against the
    // real geometry (zero add ops) instead of re-adding every entity. Idempotent +
    // a no-op for a fresh sketch (empty entities/constraints).
    seedIdMapFromWire(map, dto.entities, dto.constraints);
    // Feed the plane into the local preview lane so beginPreview resolves it.
    lane.cacheSketchPlane(frontendId, dto.plane);
    return {
      sketchId: frontendId, // keep the frontend id; the map holds the backend UUID
      plane: dto.plane,
      entities,
      constraints,
      dof: dto.dof,
      status: dto.status,
      // Map the entering solve's conflicting uuids → frontend ids (after seeding the
      // id-map so map.constraint is populated). Seeds the store's conflictingIds.
      conflicting: frontendConflictingIds(map, dto.conflicting),
      // Same seeding order dependency, on `map.entity` instead: the per-entity
      // map is keyed by wire ENTITY uuid, so re-keying it before
      // `seedIdMapFromWire` above would drop every entry of a re-entered sketch.
      entityStates: frontendEntityStates(map, dto.entityStates),
    };
  }

  async function sketchUpsert(
    sketchId: string,
    entities: SketchEntity[],
    constraints: SketchConstraint[],
  ): Promise<SketchUpsertResult> {
    const map = sketchMaps.get(sketchId);
    if (!map) throw new Error(`sketchUpsert: unknown sketch ${sketchId} (enter first)`);
    // Transactional: marshal against a CLONE so the id-map bookkeeping (minted /
    // dropped ids) is only committed onto the live map AFTER the RPC resolves. A
    // rejected upsert throws below and leaves the live map untouched, so the next
    // upsert faithfully re-adds the entities the backend never accepted.
    const draft = cloneIdMap(map);
    const ops = marshalUpsert(draft, { entities, constraints });
    const dto = await call<SketchUpsertDto>(CMD.sketchUpsert, { sketchId: map.backendSketchId, ops });
    sketchMaps.set(sketchId, draft); // commit the clone (only on success)
    return {
      sketchId, // frontend id
      sketchRevision: dto.sketchRevision,
      dof: dto.dof,
      status: dto.status,
      // Map conflicting constraint uuids → frontend ids against the COMMITTED clone.
      conflicting: frontendConflictingIds(draft, dto.conflicting),
      // F-WP9 fix: the worker keys solvedPositions by backend POINT-entity UUID;
      // reverse-map them to the frontend `entityId.point` keys the SketchController
      // applies (via the COMMITTED clone's id-map). Unknown keys are dropped.
      solvedPositions: frontendSolvedPositions(draft, dto.solvedPositions),
      // SP-2 `curves` — a solve can move a curve PARAMETER (a Tangent propagating
      // into a radius) with no point moving at all.
      solvedCurves: frontendSolvedCurves(draft, dto.solvedCurves),
      // §7.4 `entityStates`, same entity keyspace as `curves` and read off the
      // same COMMITTED clone — an entity this upsert just minted is in `draft`,
      // not in the pre-call `map`.
      entityStates: frontendEntityStates(draft, dto.entityStates),
    };
  }

  /**
   * H3b — one `SetDimension` against a sketch the user only SELECTED.
   *
   * Both ids fall back to themselves when the sketch was never entered this session
   * (`?? sketchId` / `?? constraintId`), the same idiom `finishSketch`/`getSketch`
   * use: a never-entered sketch's store key IS the backend UUID, and the constraint
   * ids `getSketch` hands the inspector are the backend's own.
   */
  async function setSketchDimension(
    sketchId: string,
    constraintId: string,
    type: SketchConstraintType,
    value: number,
  ): Promise<SketchUpsertResult> {
    const map = sketchMaps.get(sketchId);
    const ops: SketchEditOp[] = [
      {
        op: "setDimension",
        constraint: map?.constraint.get(constraintId) ?? constraintId,
        // deg→rad for an Angle, pass-through otherwise. NEVER re-derived here —
        // `angleUnits` is the one seam the three marshalling sites share (BUG-2).
        value: { value: toWireDimensionValue(type, value) },
      },
    ];
    const dto = await call<SketchUpsertDto>(CMD.sketchUpsert, {
      sketchId: map?.backendSketchId ?? sketchId,
      ops,
    });
    // Keep the marshaller's diff cache in step: without this, entering the sketch
    // later and upserting would re-emit a SetDimension for a value the backend
    // already holds (harmless, but it is exactly the drift the cache exists to stop).
    map?.constraintValue.set(constraintId, value);
    return {
      sketchId,
      sketchRevision: dto.sketchRevision,
      dof: dto.dof,
      status: dto.status,
      // No map ⇒ the wire ids ARE the ids the caller knows; pass them through.
      conflicting: map ? frontendConflictingIds(map, dto.conflicting) : (dto.conflicting ?? []),
      solvedPositions: map ? frontendSolvedPositions(map, dto.solvedPositions) : {},
      solvedCurves: map ? frontendSolvedCurves(map, dto.solvedCurves) : {},
    };
  }

  async function finishSketch(sketchId: string): Promise<FinishSketchResult> {
    // No id-map entry ⇒ a sketch never ENTERED this session (reopened document /
    // model-mode record guarantee). Its store key IS the backend UUID (projection
    // hydration), so use it directly — same fallback as getSketchRegions.
    const backendSketchId = sketchMaps.get(sketchId)?.backendSketchId ?? sketchId;
    const dto = await call<FinishSketchDto>(CMD.finishSketch, { sketchId: backendSketchId });
    const regions = dto.regions.map((r): SketchRegion => ({
      regionId: r.regionId,
      regionIdentityVersion: dto.regionIdentityVersion,
      outerLoop: r.outerLoop,
      holes: r.holes,
      previewTriangles: r.previewTriangles,
    }));
    lane.cacheFinishedRegions(sketchId, regions); // feed the local L2 preview
    return { regionIdentityVersion: dto.regionIdentityVersion, regions };
  }

  /** Read-only region derivation for model-mode selection and unopened sketches. */
  async function getSketchRegions(sketchId: string): Promise<FinishSketchResult> {
    const backendSketchId = sketchMaps.get(sketchId)?.backendSketchId ?? sketchId;
    const dto = await call<FinishSketchDto>(CMD.getSketchRegions, { sketchId: backendSketchId });
    const regions = dto.regions.map((r): SketchRegion => ({
      regionId: r.regionId,
      regionIdentityVersion: dto.regionIdentityVersion,
      outerLoop: r.outerLoop,
      holes: r.holes,
      previewTriangles: r.previewTriangles,
    }));
    lane.cacheFinishedRegions(sketchId, regions);
    return { regionIdentityVersion: dto.regionIdentityVersion, regions };
  }

  /** Pure read of a sketch's authoritative geometry (always-visible layer). Does NOT
   *  open a session / fire AddSketch / seed the id-map — the frozen `get_sketch`
   *  contract returns the SAME wire shape as `enter_sketch`; unknown ids reject. */
  async function getSketch(sketchId: string): Promise<SketchSession> {
    // Same frontend-id → backend-UUID resolution as finishSketch / getSketchRegions /
    // deleteSketch: an ENTERED sketch's frontend id is NOT a backend SketchId. The
    // `?? sketchId` fallback covers a never-entered sketch, whose store key IS the
    // backend UUID (projection hydration).
    const backendSketchId = sketchMaps.get(sketchId)?.backendSketchId ?? sketchId;
    const dto = await call<SketchSessionDto>(CMD.getSketch, { sketchId: backendSketchId });
    // Feed the plane into the local preview lane so a following beginPreview resolves
    // it — exactly as enterSketch does (:491). Model-mode arms now read via getSketch
    // (MODEL-HARDEN W0.5); without this an L2 extrude/revolve preview would fall back
    // to the XY plane.
    lane.cacheSketchPlane(sketchId, dto.plane);
    return {
      sketchId,
      plane: dto.plane,
      entities: frontendEntitiesFromDto(dto.entities),
      constraints: frontendConstraintsFromDto(dto.constraints, dto.entities),
      dof: dto.dof,
      status: dto.status,
      // A pure static read opens no session + seeds no id-map, so backend uuids can't
      // be mapped; the display layer carries no conflict tint (backend returns []).
      conflicting: [],
    };
  }

  async function cancelSketch(sketchId: string): Promise<void> {
    const map = sketchMaps.get(sketchId);
    if (!map) return; // never entered — nothing to cancel
    await call<void>(CMD.cancelSketch, { sketchId: map.backendSketchId });
  }

  /** Compensation/cleanup: remove a sketch from the document via a `DeleteSketch`
   *  EditCommand (same invoke shape as AddSketch — returns the pre-regen projection).
   *  Drops the local id-map entry ONLY after the command resolves, so a rejected
   *  delete leaves the map (and the sketch) intact for the caller to react to. */
  async function deleteSketch(sketchId: string): Promise<void> {
    const backendSketchId = sketchMaps.get(sketchId)?.backendSketchId ?? sketchId;
    await call<DocumentProjectionDto>(CMD.applyEditCommand, {
      command: buildDeleteSketch(backendSketchId),
    });
    sketchMaps.delete(sketchId);
  }

  /**
   * REATTACH (H9): `UpdateSketchAttachment` onto a world plane or datum.
   *
   * CORRELATED (not metadata-only): the core dirties from the sketch's producing
   * step to the end (`RegenHint::ToEnd`), so every downstream feature rebuilds on
   * the new frame and the result must carry that regen — same class as
   * `DeleteSketch`, which is likewise kept out of `METADATA_ONLY_CMDS`.
   *
   * A DATUM target sends the datum's backend-resolved frame from the projection.
   * The core overwrites it with the same frame anyway (`stamp_datum_plane`); the
   * point of sending it is that this entry point and `add_sketch_on_datum` stay
   * byte-identical rather than one of them becoming a second basis authority.
   */
  async function reattachSketch(
    sketchId: string,
    target: SketchAttachTarget,
  ): Promise<ApplyOperationResult> {
    const backendSketchId = sketchMaps.get(sketchId)?.backendSketchId ?? sketchId;
    let command;
    if (target.kind === "world") {
      command = buildUpdateSketchAttachmentWorld(backendSketchId, target.plane);
    } else {
      const datum = documentStore.getState().datums[target.datumId];
      if (!datum) throw new Error(`reattachSketch: unknown datum ${target.datumId}`);
      // Same refusal `enterSketch`'s datum arm makes: an unresolved datum has no
      // frame, and sending an identity basis would silently plant a wrong one.
      if (!datum.resolvedValid) {
        throw new Error(`reattachSketch: datum '${datum.name}' has an unresolved frame`);
      }
      command = buildUpdateSketchAttachmentDatum(
        backendSketchId,
        {
          origin: [...datum.plane.origin] as [number, number, number],
          xAxis: [...datum.plane.xAxis] as [number, number, number],
          yAxis: [...datum.plane.yAxis] as [number, number, number],
          normal: [...datum.plane.normal] as [number, number, number],
        },
        target.datumId,
      );
    }
    return applyEdit(CMD.applyEditCommand, { command }, "Reattach Sketch");
  }

  // ── Sketch drag gesture (latest-wins) ─────────────────────────────────────
  let dragSketchId: string | null = null;
  let dragMaxSeq = 0;

  /**
   * Marshal a frontend {@link GestureTarget} into the Rust command's
   * `GestureTargetArgs` (SCHEMA §7.4), translating `entityId` per kind:
   *
   *  - `point` — the same ref ladder `dragPoint` uses (synthesized point → entity
   *    → raw pass-through); `pointId` wins on that path anyway, so an unmapped ref
   *    is no worse than today's.
   *  - every other kind — a real ENTITY uuid, or THROW. A non-uuid would be
   *    refused at the command boundary (`EntityId::from_str`) after the gesture
   *    state was already armed; failing here names the id that could not be mapped.
   */
  function wireGestureTarget(map: SketchIdMap, target: GestureTarget): GestureTargetArgs {
    const { kind, entityId, role, grab } = target;
    const entity =
      kind === "point"
        ? (map.point.get(entityId) ?? map.entity.get(entityId) ?? entityId)
        : map.entity.get(entityId);
    if (!entity) {
      throw new Error(`beginGesture: ${kind} target ${entityId} is not a mapped sketch entity`);
    }
    return { kind, entity, ...(role ? { role } : {}), ...(grab ? { grab } : {}) };
  }

  async function beginGesture(
    sketchId: string,
    dragPointId: string,
    target?: GestureTarget,
  ): Promise<BeginGestureResult> {
    await ensureEvents();
    const map = sketchMaps.get(sketchId);
    if (!map) throw new Error(`beginGesture: unknown sketch ${sketchId} (enter first)`);
    // Translate the frontend point ref → backend point UUID (a synthesized point,
    // a Point entity, or an already-real uuid pass-through).
    const dragPoint = map.point.get(dragPointId) ?? map.entity.get(dragPointId) ?? dragPointId;
    // Marshal BEFORE arming the gesture state: an unmappable target must leave no
    // half-open drag behind for the next solveDrag/endGesture to reconcile against.
    const args = target ? wireGestureTarget(map, target) : undefined;
    dragSketchId = sketchId;
    dragMaxSeq = 0;
    const dto = await call<BeginGestureDto>(CMD.beginGesture, {
      sketchId: map.backendSketchId,
      dragPoint,
      // Omitted entirely when absent — Rust's `Option<GestureTargetArgs>` then
      // defaults to the plain point drag and the request stays pre-SP-2 identical.
      ...(args ? { target: args } : {}),
    });
    // The gesture-fixed §7.4 map, re-keyed to frontend ids. Read off the LIVE
    // map: BeginGesture mints nothing, so there is no clone to commit here.
    return {
      gestureId: dto.gestureId,
      ready: dto.ready,
      entityStates: frontendEntityStates(map, dto.entityStates),
    };
  }

  async function solveDrag(target: [number, number]): Promise<DragSolveResult | null> {
    // Fire-and-forget: the caller does NOT await serially. Responses reconcile
    // latest-wins by seq — a stale/superseded seq is dropped (returns null).
    const dto = await call<DragSolveDto>(CMD.solveDrag, { target });
    if (dto.superseded || dto.seq <= dragMaxSeq) return null; // stale — drop
    dragMaxSeq = dto.seq;
    // Reverse-map backend point UUIDs → frontend `entityId.point` keys, same as
    // endGesture — the drag PREVIEW consumes these per frame, and unmapped keys
    // would silently move nothing against the real worker.
    const map = dragSketchId ? sketchMaps.get(dragSketchId) : undefined;
    return {
      gestureId: dto.gestureId,
      seq: dto.seq,
      status: dto.status,
      dof: dto.dof,
      // BUG fix: the worker keys `conflicting` by backend constraint UUID; map them to
      // frontend ids (previously passed raw UUIDs the UI could never match to a row).
      conflicting: map ? frontendConflictingIds(map, dto.conflicting) : [],
      positions: map ? frontendSolvedPositions(map, dto.positions) : dto.positions,
      // SP-2: the `curves` channel is keyed by backend ENTITY uuid (not point
      // uuid) — reverse-map it the same way. A `radius` drag's whole result
      // arrives here, with `positions` legitimately empty.
      curves: map ? frontendSolvedCurves(map, dto.curves) : {},
      solveMicros: dto.solveMicros,
      superseded: dto.superseded,
    };
  }

  async function endGesture(finalTarget?: [number, number]): Promise<SketchUpsertResult> {
    const sketchId = dragSketchId;
    dragSketchId = null;
    dragMaxSeq = 0;
    const dto = await call<SketchUpsertDto>(CMD.endGesture, { finalTarget: finalTarget ?? null });
    const map = sketchId ? sketchMaps.get(sketchId) : undefined;
    return {
      sketchId: sketchId ?? dto.sketchId,
      sketchRevision: dto.sketchRevision,
      dof: dto.dof,
      status: dto.status,
      // Map conflicting constraint uuids → frontend ids (unknown dropped).
      conflicting: map ? frontendConflictingIds(map, dto.conflicting) : [],
      // Reverse-map backend point UUIDs → frontend `entityId.point` keys (F-WP9).
      solvedPositions: map ? frontendSolvedPositions(map, dto.solvedPositions) : dto.solvedPositions,
      // SP-2 `curves`, baselined at BeginGesture: an arc's solved radius/angles and
      // a radius drag's whole result ride ONLY here — `solvedPositions` cannot carry
      // them (the Rust Arc owns no endpoint entities, a radius moves no point).
      solvedCurves: map ? frontendSolvedCurves(map, dto.solvedCurves) : {},
      // §7.4: the ECHO of the BeginGesture map, so the consumer's held copy is
      // replaced by an identical one. `{}` without an id-map, matching every
      // other channel here — an unmappable end is unknown, not a guess.
      entityStates: map ? frontendEntityStates(map, dto.entityStates) : {},
    };
  }

  // ── Topology repair (dry-run resolve + raw edit commands; M4b) ─────────────
  async function resolveRefs(refs: ResolveRefRequest[]): Promise<ResolveRefResult[]> {
    // `resolve_refs` wants `{snapshotId, refs: [{refId, ...ElementRef}]}`; each ref
    // flattens the (optional) primary/anchor. The lean `needs-repair` event carries
    // no ElementRef, so callers usually pass `refId` only and the backend resolves
    // the stored ref by id. (SEAM: if the backend requires a full ElementRef the
    // needs-repair event must surface it — reported.)
    const requestedSnapshotId = refs[0]?.snapshotId ?? currentSnapshotId;
    const out = await call<ResolveRefResult[]>(CMD.resolveRefs, {
      snapshotId: requestedSnapshotId,
      refs: refs.map((r) => ({ refId: r.refId, primary: r.primary, anchor: r.anchor })),
    });
    return out.map((r) => ({ ...r, candidates: r.candidates ?? [] }));
  }

  async function applyEditCommand(command: WireEditCommand): Promise<ApplyOperationResult> {
    if (METADATA_ONLY_CMDS.has(command.cmd)) {
      // Metadata-only: `RegenHint::None` on the Rust side (edit/session.rs dirty/regen
      // table), so `apply_edit_command` publishes a projection and NO regen ever runs.
      // Registering a correlation awaiter would therefore stall on the 8 s safety
      // timeout for every eye click. Take the pre-regen projection as the whole answer
      // — the same shape `deleteSketch` uses. NOTE: DeleteSketch is deliberately NOT in
      // this set: it dirties the timeline (`ToEnd`) and must stay correlated.
      const projection = await call<DocumentProjectionDto>(CMD.applyEditCommand, { command });
      return {
        revision: projection.revision,
        changedBodies: [],
        removedBodies: [],
        features: projection.features,
        opLabel: editCommandLabel(command),
        appliedOps: projection.appliedOps,
        totalOps: projection.totalOps,
      };
    }
    // A scalar re-edit names an existing timeline record, so it opts into the same
    // per-record correlation a fresh op gets (U1) — without it, a regen that
    // published other steps while THIS record errored settled as a success and
    // every re-edit family reported it as one.
    //
    // Deliberately NOT correlated: `removeOperation` (the record is gone, so
    // `failedSteps` can never name it), `setOperationSuppression` and
    // `setRollback` (their effect is on DOWNSTREAM steps, so scoping the change to
    // the named record's own bodies would drop exactly the bodies that moved).
    const recordId = command.cmd === "updateOperationParams" ? command.record : undefined;
    return applyEdit(CMD.applyEditCommand, { command }, editCommandLabel(command), { recordId });
  }

  /** H2 escape hatch: forget the worker's poison keys (returns how many). */
  async function clearWorkerCircuit(): Promise<number> {
    return call<number>(CMD.clearWorkerCircuit);
  }

  async function getOperationParams(recordId: string): Promise<Record<string, unknown>> {
    // Read-only: the stored op's params (EditCommand `op.params` serde shape), so a
    // scalar re-edit can deep-merge without clobbering axis / openFaces / edges.
    return call<Record<string, unknown>>(CMD.getOperationParams, { recordId });
  }

  async function featureDependencies(featureId: string): Promise<FeatureDependencies> {
    // Read-only lineage query (H10): upstream/downstream closures from the core
    // dependency graph, for the delete/suppress dependent count and the inspector's
    // "Depends on / Used by" section.
    return call<FeatureDependencies>(CMD.featureDependencies, { recordId: featureId });
  }

  async function canFoldTransform(bodyId: string): Promise<string | null> {
    // Read-only lineage query (`api::can_fold_transform`, arg key `bodyId`).
    // `Option<String>` on the Rust side, so `null` is the ordinary "append a
    // fresh record" answer — never an error.
    return call<string | null>(CMD.canFoldTransform, { bodyId });
  }

  // ── Promotion (pick → ElementId) ──────────────────────────────────────────
  /**
   * Resolve a picked planar face to its sketch plane. The backend reads the
   * KERNEL's face descriptor (`QueryElement`) and applies the lock-tested
   * in-plane axis rule, so the basis is authoritative and replay-stable — a
   * frame derived here from a tessellated triangle normal would be neither.
   * A non-planar face is rejected by the backend, surfacing as an ApiError.
   *
   * `topoKey` is forwarded when the caller has it: a just-promoted, not-yet-
   * consumed `elementId` is not yet in the worker's on-demand element-map
   * partition, so the backend tries the topoKey rung first (same ladder as
   * `elementInfo`) before falling back to `elementId`.
   */
  async function faceSketchPlane(
    bodyId: string,
    elementId: string,
    topoKey?: string,
  ): Promise<SketchPlane> {
    // Same `body_<uuid>` wire form promoteSelection uses (document-changed hands
    // the frontend a bare uuid).
    const wireBodyId = bodyId.startsWith("body_") ? bodyId : `body_${bodyId}`;
    const dto = await call<{
      origin: [number, number, number];
      xAxis: [number, number, number];
      yAxis: [number, number, number];
      normal: [number, number, number];
    }>(CMD.faceSketchPlane, {
      snapshotId: currentSnapshotId,
      bodyId: wireBodyId,
      elementId,
      topoKey: topoKey ?? null,
    });
    return { kind: "custom", ...dto };
  }

  /**
   * Read one element's kernel descriptor (MEASURE V1a). Read-only: it mints
   * nothing and never touches history.
   *
   * Both handles are forwarded when available — Rust walks the two-rung ladder
   * (topoKey first, then elementId) because a promoted-but-unused ElementId is
   * not yet in the worker's on-demand partition. `null` means "not present in
   * this snapshot", which the caller treats as a dropped pick, not an error.
   */
  async function elementInfo(
    bodyId: string,
    elementId: string,
    topoKey?: string,
  ): Promise<ElementInfo | null> {
    // Same `body_<uuid>` wire form promoteSelection uses (document-changed hands
    // the frontend a bare uuid).
    const wireBodyId = bodyId.startsWith("body_") ? bodyId : `body_${bodyId}`;
    return call<ElementInfo | null>(CMD.elementInfo, {
      snapshotId: currentSnapshotId,
      bodyId: wireBodyId,
      elementId,
      topoKey: topoKey ?? null,
    });
  }

  /**
   * One body's exact kernel mass properties (WP-C1). Read-only.
   *
   * No `snapshotId` is sent — a BodyId is durable across snapshots, so the
   * backend answers against the current head (see `api::query_mass_properties`).
   * An unknown body REJECTS; there is no `null` arm to absorb.
   */
  async function massProperties(bodyId: string): Promise<MassProperties> {
    // Same `body_<uuid>` wire form promoteSelection uses (document-changed hands
    // the frontend a bare uuid).
    const wireBodyId = bodyId.startsWith("body_") ? bodyId : `body_${bodyId}`;
    return call<MassProperties>(CMD.massProperties, { bodyId: wireBodyId });
  }

  /**
   * Surface/curve classification of a picked face/edge + a seatable frame
   * (Component Library WP-0.1). Read-only, always the current head — issued
   * on every hover frame during a live drag, so no `snapshotId` is sent (same
   * reasoning as `massProperties`).
   */
  async function classifyElement(
    bodyId: string,
    elementId: string,
    topoKey?: string,
  ): Promise<ClassifyResult | null> {
    // Same `body_<uuid>` wire form promoteSelection uses (document-changed hands
    // the frontend a bare uuid).
    const wireBodyId = bodyId.startsWith("body_") ? bodyId : `body_${bodyId}`;
    return call<ClassifyResult | null>(CMD.classifyElement, {
      bodyId: wireBodyId,
      elementId,
      topoKey: topoKey ?? null,
    });
  }

  // ── Component Library (WP-1.3) ────────────────────────────────────────

  async function listLibraryComponents(): Promise<LibraryComponent[]> {
    return call<LibraryComponent[]>(CMD.listLibraryComponents, {});
  }

  async function reindexLibrary(): Promise<ReindexReport> {
    return call<ReindexReport>(CMD.reindexLibrary, {});
  }

  async function resolveComponentSource(
    componentId: string,
    componentVersion: string,
  ): Promise<PlaceComponentSource> {
    return call<PlaceComponentSource>(CMD.resolveComponentSource, {
      componentId,
      componentVersion,
    });
  }

  async function placeComponent(
    componentId: string,
    componentVersion: string,
    translate: [number, number, number],
    rotate?: TransformRotationParams,
    params?: Record<string, ComponentParamValue>,
    mate?: PlaceComponentMate,
  ): Promise<ApplyOperationResult> {
    // `rotate` used to be dropped here — the real backend placed every
    // component unrotated regardless of the flip gesture (`A` key), only
    // masked because the mock lane's `commitPlaceComponent` DOES honor it.
    // `params` (WP-A3) is the same class of bug waiting to happen: the ghost
    // previews the auto-sized screw through `source.params`, so a commit that
    // dropped it would place a different size than the user saw.
    //
    // `applyEdit`, not a bare `call`: the command returns a DocumentProjection
    // and fires a regen, so it is an EDIT by every definition the rest of the
    // app uses. Going through `call` meant the placement never learned its own
    // regen terminal — a component whose mate failed downstream reported the
    // same silent success as one that seated perfectly (U1 result-truth).
    return applyEdit(
      CMD.placeComponent,
      { componentId, componentVersion, translate, rotate, params, mate },
      "PlaceComponent",
    );
  }

  async function componentPreviewMesh(
    componentId: string,
    componentVersion: string,
    params?: Record<string, ComponentParamValue>,
  ): Promise<{ bodyId: string; mesh: ArrayBuffer }[]> {
    const res = await call<{ bodies: { bodyId: string; mesh: number[] | ArrayBuffer }[] }>(
      CMD.componentPreviewMesh,
      { componentId, componentVersion, params },
    );
    // Same normalization `previewOp` does: Tauri hands a JSON number array over
    // the IPC boundary unless the payload is already an ArrayBuffer.
    return res.bodies.map((b) => ({
      bodyId: b.bodyId,
      mesh: b.mesh instanceof ArrayBuffer ? b.mesh : new Uint8Array(b.mesh).buffer,
    }));
  }

  async function listTemplates(): Promise<ProjectTemplate[]> {
    return call<ProjectTemplate[]>(CMD.listTemplates, {});
  }

  async function saveAsTemplate(
    id: string,
    name: string,
    description?: string,
    previewPng?: string | null,
  ): Promise<ProjectTemplate> {
    return call<ProjectTemplate>(CMD.saveAsTemplate, { id, name, description, previewPng });
  }

  async function newFromTemplate(id: string): Promise<DocumentSnapshot> {
    return call<DocumentSnapshot>(CMD.newFromTemplate, { id });
  }

  async function saveAsComponent(
    bodyId: string,
    spec: NewComponentSpec,
    previewPng?: string | null,
    unionSolids?: boolean,
  ): Promise<LibraryComponent> {
    return call<LibraryComponent>(CMD.saveAsComponent, {
      bodyId,
      spec,
      previewPng,
      unionSolids,
    });
  }

  /**
   * A replace is an `UpdateOperationParams` on an EXISTING record that re-bakes
   * the component's geometry, so it correlates on that `recordId` exactly like
   * `setComponentParams` — the bare `call` it used meant a swap whose re-bake
   * failed reported the same silent success as one that seated (W5 result truth).
   *
   * Its payload is the one edit envelope that is NOT a bare projection
   * (`{ projection, report }`), which is what `applyEditRaw`'s `unwrap` is for.
   */
  async function replaceComponent(
    recordId: string,
    componentId: string,
    componentVersion: string,
    params?: Record<string, ComponentParamValue>,
  ): Promise<ReplaceComponentResult> {
    const { raw, result } = await applyEditRaw<{
      projection: DocumentProjectionDto;
      report: ReplaceComponentReport;
    }>(
      CMD.replaceComponent,
      { recordId, componentId, componentVersion, params },
      "ReplaceComponent",
      { recordId },
      (r) => r.projection,
    );
    return { ...result, report: raw.report ?? {} };
  }

  async function componentUpgradeAvailable(recordId: string): Promise<ComponentUpgrade | null> {
    return call<ComponentUpgrade | null>(CMD.componentUpgradeAvailable, { recordId });
  }

  /* Both are edits on an EXISTING record, so they correlate on that `recordId`
     (see `placeComponent` for why these left the bare-`call` lane). */
  async function setComponentParams(
    recordId: string,
    params: Record<string, ComponentParamValue>,
  ): Promise<ApplyOperationResult> {
    return applyEdit(CMD.setComponentParams, { recordId, params }, "SetComponentParams", {
      recordId,
    });
  }

  async function detachComponent(recordId: string): Promise<ApplyOperationResult> {
    return applyEdit(CMD.detachComponent, { recordId }, "DetachComponent", { recordId });
  }

  /**
   * `PrepareOffsetFace` (SCHEMA §7.6) — the read-only OffsetFace authoring
   * handshake. Snapshot-FENCED, unlike the advisory §7.5 reads: the closure it
   * returns is about to be FROZEN into a document record, so a stale head is
   * refused rather than answered against different topology.
   *
   * `snapshotId` defaults to the transport's own `currentSnapshotId` — the same
   * value `promoteSelection` fences with. A caller that has no snapshot to hand
   * (every caller today) must not invent one.
   *
   * A REFUSAL comes back inside the result, not as a rejection (`ok:true`
   * semantics); only a malformed request or a dead worker rejects.
   */
  async function prepareOffsetFace(req: PrepareOffsetFaceRequest): Promise<PrepareOffsetFaceResult> {
    const snapshotId = req.snapshotId ?? currentSnapshotId;
    const result = await call<PrepareOffsetFaceResult>(CMD.prepareOffsetFace, {
      snapshotId,
      // Same `body_<uuid>` wire form promoteSelection uses; `document-changed`
      // hands the frontend a bare uuid. The two address rungs are MUTUALLY
      // EXCLUSIVE on the wire (the command rejects a pick carrying both), so an
      // empty string is normalized away rather than sent as a present-but-blank key.
      pickedFaces: req.pickedFaces.map((p) => ({
        bodyId: p.bodyId ? (p.bodyId.startsWith("body_") ? p.bodyId : `body_${p.bodyId}`) : null,
        topoKey: p.elementId ? null : (p.topoKey ?? null),
        elementId: p.elementId ?? null,
      })),
      chainTangentFaces: req.chainTangentFaces,
      distanceType: req.distanceType,
    });
    if (result.snapshotId !== snapshotId || currentSnapshotId !== snapshotId) {
      throw new Error("prepared offset-face closure is stale — re-pick");
    }
    return result;
  }

  async function prepareEdgeOp(req: PrepareEdgeOpRequest): Promise<PrepareEdgeOpResult> {
    const snapshotId = req.snapshotId ?? currentSnapshotId;
    const result = await call<PrepareEdgeOpResult>(CMD.prepareEdgeOp, {
      snapshotId,
      mode: req.mode,
      pickedEdges: req.pickedEdges.map((p) => ({
        bodyId: p.bodyId ? (p.bodyId.startsWith("body_") ? p.bodyId : `body_${p.bodyId}`) : null,
        topoKey: p.elementId ? null : (p.topoKey ?? null),
        elementId: p.elementId ?? null,
      })),
      chainTangentEdges: req.chainTangentEdges,
    });
    if (result.snapshotId !== snapshotId || currentSnapshotId !== snapshotId) {
      throw new Error("prepared edge closure is stale — re-pick");
    }
    return result;
  }

  async function analyzeEdgeOpRange(
    req: AnalyzeEdgeOpRangeRequest,
  ): Promise<AnalyzeEdgeOpRangeResult> {
    const snapshotId = req.snapshotId ?? currentSnapshotId;
    const result = await call<AnalyzeEdgeOpRangeResult>(CMD.analyzeEdgeOpRange, {
      snapshotId,
      mode: req.mode,
      pickedEdges: req.pickedEdges.map((p) => ({
        bodyId: p.bodyId ? (p.bodyId.startsWith("body_") ? p.bodyId : `body_${p.bodyId}`) : null,
        topoKey: p.elementId ? null : (p.topoKey ?? null),
        elementId: p.elementId ?? null,
      })),
      chainTangentEdges: req.chainTangentEdges,
      range: req.range ?? null,
    });
    // Same fence as prepareEdgeOp, and for a stronger reason: this answer is
    // about to CLAMP what the user may type. A range measured on a head they are
    // no longer looking at would silently forbid a value that now fits.
    if (result.snapshotId !== snapshotId || currentSnapshotId !== snapshotId) {
      throw new Error("edge range answer is stale — re-pick");
    }
    return result;
  }

  async function promoteSelection(
    bodyId: string,
    picks: PromotePick[],
    snapshotId = currentSnapshotId,
  ): Promise<PromotedElement[]> {
    // promote_selection wants the `body_<uuid>` wire form; document-changed hands
    // the frontend a bare uuid, so prefix it here (get_mesh keeps the bare form).
    const wireBodyId = bodyId.startsWith("body_") ? bodyId : `body_${bodyId}`;
    const out = await call<PromotedElementDto[]>(CMD.promoteSelection, {
      snapshotId,
      bodyId: wireBodyId,
      picks: picks.map((p) => ({ topoKey: p.topoKey, anchor: p.anchor })),
    });
    return out.map((e) => ({ topoKey: e.topoKey, elementId: e.elementId, kind: e.kind, bodyId: e.bodyId }));
  }

  return {
    async listRecents(): Promise<RecentProject[]> {
      return call<RecentProject[]>(CMD.listRecents);
    },
    async renameRecentProject(path: string, newName: string): Promise<void> {
      await call<void>(CMD.renameRecentProject, { path, newName });
    },
    async deleteRecentProject(path: string): Promise<void> {
      await call<void>(CMD.deleteRecentProject, { path });
    },
    async revealInFileManager(path: string): Promise<void> {
      await call<void>(CMD.revealInFileManager, { path });
    },
    async newDocument(): Promise<DocumentSnapshot> {
      // Events BEFORE the command: the pre-regen projection (and a fast open-regen's
      // document-changed) fires during/right after the round-trip.
      await ensureEvents();
      const snap = await call<DocumentSnapshotDto>(CMD.newDocument);
      resetCorrelation(); // drop the OLD document's buffered change + pending awaiters
      return snap;
    },
    async openDocument(path: string, onRecovery?: OnRecovery): Promise<DocumentSnapshot> {
      await ensureEvents();
      // `onRecovery` is undefined on the ordinary open; Rust rejects with
      // `recoveryPending` if this path turns out to have unresolved autosaved work.
      const snap = await call<DocumentSnapshotDto>(CMD.openDocument, { path, onRecovery });
      resetCorrelation();
      return snap;
    },
    async importStep(path: string): Promise<DocumentSnapshot> {
      await ensureEvents();
      const snap = await call<DocumentSnapshotDto>(CMD.importStep, { path });
      resetCorrelation();
      return snap;
    },
    async importProject(): Promise<DocumentSnapshot | null> {
      await ensureEvents();
      const snap = await call<DocumentSnapshotDto | null>(CMD.importProject);
      if (snap) resetCorrelation();
      return snap;
    },
    async importFileDialog(): Promise<string | null> {
      return call<string | null>(CMD.importFileDialog);
    },
    insertStep,
    // Module-owned document state (ADR-0004). The payload is passed through
    // untouched in both directions — this lane has no schema for it.
    getModuleState(moduleId: string): Promise<ModuleState | null> {
      return call<ModuleState | null>(CMD.getModuleState, { moduleId });
    },
    setModuleState(
      moduleId: string,
      state: { schemaVersion: number; payload: unknown } | null,
    ): Promise<void> {
      return call<void>(CMD.setModuleState, {
        moduleId,
        schemaVersion: state?.schemaVersion ?? null,
        payload: state?.payload ?? null,
      });
    },
    listDocumentModules(): Promise<DocumentModule[]> {
      return call<DocumentModule[]>(CMD.listDocumentModules);
    },
    // WP-VE.2. All three answer with the table AFTER the edit, so the section
    // never has to guess at the result of its own write; the backend owns
    // validation (name grammar, duplicates, unknown-remove) and the client adds
    // none of its own — a second, drifting copy of the rule is how a UI starts
    // accepting names the document refuses.
    //
    // W5: the two WRITES go through `applyEdit`, not a bare `call`. They return a
    // `DocumentProjection` and schedule a regen, so they are edits by every
    // definition the rest of the app uses, and the table now rides that
    // projection. `documentScope` because a variable edit dirties `[0, len)` —
    // it names no single record, and every step it rebuilt is its own to report.
    listVariables(): Promise<DocumentVariable[]> {
      return call<DocumentVariable[]>(CMD.listVariables);
    },
    upsertVariable: (name: string, value: number) =>
      variableEdit(CMD.upsertVariable, { name, value }, "SetVariable"),
    removeVariable: (name: string) =>
      variableEdit(CMD.removeVariable, { name }, "RemoveVariable"),
    async closeDocument(): Promise<void> {
      await call<void>(CMD.closeDocument);
      resetCorrelation();
    },
    async checkRecovery(): Promise<RecoveryInfo[]> {
      return call<RecoveryInfoDto[]>(CMD.checkRecovery);
    },
    async recoverDocument(
      documentId: string,
      accept: boolean,
    ): Promise<DocumentSnapshot | null> {
      if (accept) await ensureEvents(); // a recover-open emits like an open
      const snap = await call<DocumentSnapshotDto | null>(CMD.recoverDocument, {
        documentId,
        accept,
      });
      if (accept) resetCorrelation(); // a discard (accept:false) opens no new document
      return snap;
    },
    async openFileDialog(): Promise<string | null> {
      return call<string | null>(CMD.openFileDialog);
    },
    async stepFileDialog(): Promise<string | null> {
      // Rust filters step/stp here; `open_file_dialog` filters `.onecad` and would
      // make a STEP file unpickable.
      return call<string | null>(CMD.stepFileDialog);
    },

    async saveDocument(path?: string, previewPng?: string | null): Promise<SaveOutcome> {
      // `path` null ⇒ the backend reuses the last save path (an unsaved document
      // with no path rejects with an io error; the caller falls back to Save As).
      // `previewPng` null ⇒ no thumbnail this save; the container keeps whatever
      // preview it already had. tauri v2 maps `previewPng` → Rust `preview_png`.
      return call<SaveOutcome>(CMD.saveDocument, {
        path: path ?? null,
        previewPng: previewPng ?? null,
      });
    },
    async saveDocumentAs(previewPng?: string | null): Promise<SaveOutcome | null> {
      const path = await call<string | null>(CMD.saveFileDialog);
      if (!path) return null; // cancelled
      return call<SaveOutcome>(CMD.saveDocument, { path, previewPng: previewPng ?? null });
    },
    async exportStep(): Promise<string | null> {
      // Rust owns the `.step` save dialog + the worker ExportStep verb; a cancel
      // resolves to null (no path written).
      return call<string | null>(CMD.exportStepFile, { path: null });
    },
    async exportStl(): Promise<string | null> {
      // Rust owns the `.stl` save dialog + the worker ExportStl verb; a cancel
      // resolves to null (no path written).
      return call<string | null>(CMD.exportStlFile, { path: null });
    },
    async exportObj(): Promise<string | null> {
      // Rust owns the `.obj` save dialog + the worker ExportObj verb; a cancel
      // resolves to null (no path written).
      return call<string | null>(CMD.exportObjFile, { path: null });
    },
    async export3mf(): Promise<string | null> {
      // Rust owns the `.3mf` save dialog + the Rust-side 3MF writer (no worker
      // verb); a cancel resolves to null (no path written).
      return call<string | null>(CMD.export3mfFile, { path: null });
    },

    onWorkerStatus(cb: (s: WorkerStatus) => void): () => void {
      void ensureEvents();
      workerStatusListeners.add(cb);
      return () => workerStatusListeners.delete(cb);
    },

    onNeedsRepair(cb: (e: NeedsRepairEvent) => void): () => void {
      void ensureEvents();
      needsRepairListeners.add(cb);
      return () => needsRepairListeners.delete(cb);
    },

    onCloseRequested(cb: () => void): () => void {
      void ensureEvents();
      closeRequestedListeners.add(cb);
      return () => closeRequestedListeners.delete(cb);
    },

    async confirmExit(): Promise<void> {
      await call<void>(CMD.confirmExit);
    },
    async cancelExit(): Promise<void> {
      await call<void>(CMD.cancelExit);
    },

    async getBodyMesh(bodyId: string, lod: Lod): Promise<ArrayBuffer> {
      // Rust `get_mesh` returns `tauri::ipc::Response` → invoke resolves an
      // ArrayBuffer (MESH1 bytes verbatim, zero-copy). generation null = latest.
      return call<ArrayBuffer>(CMD.getMesh, { bodyId, lod, generation: null });
    },

    onDocumentChanged(cb: (change: DocumentChange) => void): () => void {
      void ensureEvents();
      docChangeListeners.add(cb);
      return () => docChangeListeners.delete(cb);
    },

    onProjectionUpdated(cb: (p: DocumentProjectionWire) => void): () => void {
      void ensureEvents();
      projectionListeners.add(cb);
      return () => projectionListeners.delete(cb);
    },

    async getProjection(): Promise<DocumentProjectionWire> {
      // One-shot authoritative pull (missed-event race on mount); reconciled by
      // revision exactly like the projection-updated event path.
      const p = await call<DocumentProjectionDto>(CMD.getProjection);
      applyProjectionToStore(p);
      return p;
    },

    applyOperation,

    async undo(): Promise<ApplyOperationResult> {
      return applyEdit(CMD.undo, {}, undefined);
    },
    async redo(): Promise<ApplyOperationResult> {
      return applyEdit(CMD.redo, {}, undefined);
    },

    // ── Sketch solver lane + drag gesture + promotion (REAL commands) ─────────
    enterSketch,
    getSketch,
    getSketchRegions,
    sketchUpsert,
    setSketchDimension,
    finishSketch,
    cancelSketch,
    deleteSketch,
    reattachSketch,
    beginGesture,
    solveDrag,
    endGesture,
    promoteSelection,
    faceSketchPlane,
    elementInfo,
    massProperties,
    classifyElement,
    listLibraryComponents,
    reindexLibrary,
    resolveComponentSource,
    placeComponent,
    saveAsComponent,
    componentPreviewMesh,
    listTemplates,
    saveAsTemplate,
    newFromTemplate,
    replaceComponent,
    componentUpgradeAvailable,
    setComponentParams,
    detachComponent,
    prepareOffsetFace,
    prepareEdgeOp,
    analyzeEdgeOpRange,
    resolveRefs,
    applyEditCommand,
    clearWorkerCircuit,
    getOperationParams,
    featureDependencies,
    canFoldTransform,

    // ── Two-level preview (local seam; backend preview verb TBD) ──────────────
    beginPreview: lane.beginPreview,
    updatePreview: lane.updatePreview,
    endPreview: lane.endPreview,
    onPreviewResult: lane.onPreviewResult,
  };
}
