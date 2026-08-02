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
import type {
  ApplyOperationResult,
  BeginGestureResult,
  DocumentChange,
  DocumentProjectionWire,
  DocumentSnapshot,
  DragSolveResult,
  ElementInfo,
  EnterSketchTarget,
  FeatureRecord,
  FinishSketchResult,
  Lod,
  NeedsRepairEvent,
  OperationOp,
  PromotedElement,
  PromotePick,
  RecentProject,
  RecoveryInfo,
  RegenFinished,
  ResolveRefRequest,
  ResolveRefResult,
  SketchConstraint,
  SketchEntity,
  SketchPlane,
  SketchPlaneKind,
  SketchRegion,
  SketchSession,
  SketchSolveStatus,
  SketchUpsertResult,
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
  cloneIdMap,
  createIdMap,
  frontendConflictingIds,
  frontendConstraintsFromDto,
  frontendEntitiesFromDto,
  frontendSolvedPositions,
  marshalUpsert,
  mintUuid,
  seedIdMapFromWire,
  type SketchIdMap,
} from "./sketchWireMap";
import { applyProjectionToStore } from "./projectionHydration";
import { documentStore, nextSketchName } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";

// ── Command + event names (must match src-tauri/src/api + events.rs) ──────────
const CMD = {
  listRecents: "list_recents",
  newDocument: "new_document",
  openDocument: "open_document",
  importStep: "import_step",
  closeDocument: "close_document",
  checkRecovery: "check_recovery",
  recoverDocument: "recover_document",
  saveDocument: "save_document",
  exportStepFile: "export_step_file",
  exportStlFile: "export_stl_file",
  exportObjFile: "export_obj_file",
  openFileDialog: "open_file_dialog",
  saveFileDialog: "save_file_dialog",
  getMesh: "get_mesh",
  getProjection: "get_projection",
  applyEditCommand: "apply_edit_command",
  getOperationParams: "get_operation_params",
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
  previewOp: "preview_op",
  resolveRefs: "resolve_refs",
  confirmExit: "confirm_exit",
  cancelExit: "cancel_exit",
} as const;

const EVT = {
  documentChanged: "document-changed",
  projectionUpdated: "projection-updated",
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
}
interface BeginGestureDto {
  gestureId: number;
  ready: boolean;
}
interface DragSolveDto {
  gestureId: number;
  seq: number;
  status: string;
  dof: number;
  conflicting: string[];
  positions: Record<string, [number, number]>;
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
    const { kind, message } = e as { kind: string; message?: string };
    const err = new Error(message ? `${kind}: ${message}` : kind);
    (err as Error & { kind?: string }).kind = kind;
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
  // Under rapid commits a stale earlier report can never resolve a LATER commit: its
  // `sourceRevision` is below that commit's R (or it is superseded and ignored).
  interface Resolved {
    change: DocumentChange | null;
    revision: number;
    errorMessage?: string;
  }
  interface Awaiter {
    targetRev: number | null; // R; null until the command returns its projection
    recordId?: string; // set for a fresh AddOperation commit (finding 1/2 scoping)
    pendingChange: DocumentChange | null; // buffered covering change (recordId path)
    resolve(r: Resolved | null): void;
    timer: ReturnType<typeof setTimeout>;
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
      if (a.recordId !== undefined) {
        a.pendingChange = change; // authoritative outcome comes with regen-finished
      } else {
        settle(a, { change, revision: change.revision });
      }
    }
  }

  /** A regen-finished terminal. superseded/cancelled are ignored for every awaiter
   *  (a newer regen covers R). For a NON-recordId awaiter, published is ignored too
   *  (its document-changed sibling carries the bodies). A recordId awaiter settles
   *  here on ANY covering completion: failedSteps ⇒ failure, else scoped success. */
  function resolveRegenFinished(rf: RegenFinished): void {
    if (rf.outcome === "superseded" || rf.outcome === "cancelled") return;
    const source = rf.sourceRevision ?? rf.revision;
    for (const a of [...awaiters]) {
      if (a.targetRev === null || source < a.targetRev) continue;
      if (a.recordId !== undefined) {
        const failed = rf.failedSteps?.find((s) => s.recordId === a.recordId);
        if (failed || rf.outcome === "failed") {
          // This op's step errored (even if the regen published other bodies), or the
          // whole regen failed ⇒ FAILURE: empty bodies + the reason.
          settle(a, { change: null, revision: rf.revision, errorMessage: failed?.message ?? rf.message });
        } else {
          // Success — scope to this op's own bodies (incl. split children).
          const change = a.pendingChange ?? lastPublishedChange;
          settle(a, {
            change: scopeChange(change, rf.affectedBodies?.[a.recordId], rf.revision),
            revision: rf.revision,
          });
        }
      } else if (rf.outcome !== "published") {
        // failed / noop for a raw command → empty bodies (+ message on failure).
        settle(a, {
          change: null,
          revision: rf.revision,
          errorMessage: rf.outcome === "failed" ? rf.message : undefined,
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

  function onRegenFinishedEvent(rf: RegenFinished): void {
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
        viewportStore.getState().setStatusHint(`Geometry rebuild failed — ${reason}`, {
          severity: "error",
          sticky: true,
        });
      }
    } else {
      trace(
        "ipc",
        `regen-finished: outcome=${rf.outcome} rev=${rf.revision} srcRev=${rf.sourceRevision ?? "?"}`,
      );
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
   *  scoping path. Resolves to null on the safety timeout. */
  function awaitNextChange(recordId?: string): {
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
      awaiter = { targetRev: null, recordId, pendingChange: null, resolve, timer };
      awaiters.add(awaiter);
    });
    return {
      promise,
      setTarget(rev: number) {
        awaiter.targetRev = rev;
        // Missed-event race: a covering publish may already have arrived between the
        // command returning and this call — reconcile against the buffer.
        if (lastPublishedChange && lastPublishedChange.revision >= rev) {
          if (awaiter.recordId !== undefined) {
            // recordId path settles on the regen-finished sibling — buffer only.
            awaiter.pendingChange = lastPublishedChange;
          } else {
            settle(awaiter, { change: lastPublishedChange, revision: lastPublishedChange.revision });
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

  /** Run an edit command and correlate its regen into an ApplyOperationResult.
   *  `recordId` (a fresh AddOperation's minted id) opts the awaiter into the
   *  failedSteps / affectedBodies scoping path (findings 1/2). */
  async function applyEdit(
    cmd: string,
    args: Record<string, unknown>,
    opLabel: string | undefined,
    recordId?: string,
  ): Promise<ApplyOperationResult> {
    await ensureEvents();
    trace("ipc", `applyEdit: cmd=${cmd} record=${recordId ?? "none"} label=${opLabel ?? "?"}`);
    const awaiter = awaitNextChange(recordId);
    let projection: DocumentProjectionDto;
    try {
      projection = await call<DocumentProjectionDto>(cmd, args);
    } catch (e) {
      awaiter.cancel();
      traceWarn("ipc", `applyEdit: cmd=${cmd} record=${recordId ?? "none"} invoke THREW`, e);
      throw e;
    }
    // Finding 4: a fresh op appended while the timeline is rolled back joins the
    // timeline as a DRAFT beyond the rollback bar (appliedOps < totalOps) — no regen
    // fires for it, so waiting would stall on the safety timeout. Detect it from the
    // returned projection and resolve immediately as a (soft) failure the controller
    // surfaces.
    if (
      recordId !== undefined &&
      typeof projection.appliedOps === "number" &&
      typeof projection.totalOps === "number" &&
      projection.appliedOps < projection.totalOps
    ) {
      awaiter.cancel();
      traceWarn(
        "ipc",
        `applyEdit: cmd=${cmd} record=${recordId ?? "none"} added while ROLLED BACK ` +
          `(appliedOps=${projection.appliedOps}/${projection.totalOps}) — no regen fires`,
      );
      return {
        revision: projection.revision,
        changedBodies: [],
        removedBodies: [],
        features: projection.features,
        opLabel,
        errorMessage: "Operation added while history is rolled back — roll forward to apply",
      };
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
    };
    if (resolved?.errorMessage) result.errorMessage = resolved.errorMessage;
    return result;
  }

  async function applyOperation(op: OperationOp): Promise<ApplyOperationResult> {
    const command = operationToEditCommand(op);
    // A fresh op mints a recordId (addOperation); a parametric re-edit
    // (updateOperationParams) targets an existing record and does NOT — only the
    // former opts into failedSteps / affectedBodies correlation.
    const recordId = command.cmd === "addOperation" ? command.record.recordId : undefined;
    return applyEdit(CMD.applyEditCommand, { command }, opLabelFor(op), recordId);
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
      outerLoop: r.outerLoop,
      holes: r.holes,
      previewTriangles: r.previewTriangles,
    }));
    lane.cacheFinishedRegions(sketchId, regions); // feed the local L2 preview
    return { regions };
  }

  /** Read-only region derivation for model-mode selection and unopened sketches. */
  async function getSketchRegions(sketchId: string): Promise<FinishSketchResult> {
    const backendSketchId = sketchMaps.get(sketchId)?.backendSketchId ?? sketchId;
    const dto = await call<FinishSketchDto>(CMD.getSketchRegions, { sketchId: backendSketchId });
    const regions = dto.regions.map((r): SketchRegion => ({
      regionId: r.regionId,
      outerLoop: r.outerLoop,
      holes: r.holes,
      previewTriangles: r.previewTriangles,
    }));
    lane.cacheFinishedRegions(sketchId, regions);
    return { regions };
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

  // ── Sketch drag gesture (latest-wins) ─────────────────────────────────────
  let dragSketchId: string | null = null;
  let dragMaxSeq = 0;

  async function beginGesture(sketchId: string, dragPointId: string): Promise<BeginGestureResult> {
    await ensureEvents();
    const map = sketchMaps.get(sketchId);
    if (!map) throw new Error(`beginGesture: unknown sketch ${sketchId} (enter first)`);
    // Translate the frontend point ref → backend point UUID (a synthesized point,
    // a Point entity, or an already-real uuid pass-through).
    const dragPoint = map.point.get(dragPointId) ?? map.entity.get(dragPointId) ?? dragPointId;
    dragSketchId = sketchId;
    dragMaxSeq = 0;
    const dto = await call<BeginGestureDto>(CMD.beginGesture, {
      sketchId: map.backendSketchId,
      dragPoint,
    });
    return { gestureId: dto.gestureId, ready: dto.ready };
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
    };
  }

  // ── Topology repair (dry-run resolve + raw edit commands; M4b) ─────────────
  async function resolveRefs(refs: ResolveRefRequest[]): Promise<ResolveRefResult[]> {
    // `resolve_refs` wants `{snapshotId, refs: [{refId, ...ElementRef}]}`; each ref
    // flattens the (optional) primary/anchor. The lean `needs-repair` event carries
    // no ElementRef, so callers usually pass `refId` only and the backend resolves
    // the stored ref by id. (SEAM: if the backend requires a full ElementRef the
    // needs-repair event must surface it — reported.)
    const out = await call<ResolveRefResult[]>(CMD.resolveRefs, {
      snapshotId: currentSnapshotId,
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
      };
    }
    return applyEdit(CMD.applyEditCommand, { command }, editCommandLabel(command));
  }

  async function getOperationParams(recordId: string): Promise<Record<string, unknown>> {
    // Read-only: the stored op's params (EditCommand `op.params` serde shape), so a
    // scalar re-edit can deep-merge without clobbering axis / openFaces / edges.
    return call<Record<string, unknown>>(CMD.getOperationParams, { recordId });
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

  async function promoteSelection(bodyId: string, picks: PromotePick[]): Promise<PromotedElement[]> {
    // promote_selection wants the `body_<uuid>` wire form; document-changed hands
    // the frontend a bare uuid, so prefix it here (get_mesh keeps the bare form).
    const wireBodyId = bodyId.startsWith("body_") ? bodyId : `body_${bodyId}`;
    const out = await call<PromotedElementDto[]>(CMD.promoteSelection, {
      snapshotId: currentSnapshotId,
      bodyId: wireBodyId,
      picks: picks.map((p) => ({ topoKey: p.topoKey, anchor: p.anchor })),
    });
    return out.map((e) => ({ topoKey: e.topoKey, elementId: e.elementId, kind: e.kind, bodyId: e.bodyId }));
  }

  return {
    async listRecents(): Promise<RecentProject[]> {
      return call<RecentProject[]>(CMD.listRecents);
    },
    async newDocument(): Promise<DocumentSnapshot> {
      // Events BEFORE the command: the pre-regen projection (and a fast open-regen's
      // document-changed) fires during/right after the round-trip.
      await ensureEvents();
      const snap = await call<DocumentSnapshotDto>(CMD.newDocument);
      resetCorrelation(); // drop the OLD document's buffered change + pending awaiters
      return snap;
    },
    async openDocument(path: string): Promise<DocumentSnapshot> {
      await ensureEvents();
      const snap = await call<DocumentSnapshotDto>(CMD.openDocument, { path });
      resetCorrelation();
      return snap;
    },
    async importStep(path: string): Promise<DocumentSnapshot> {
      await ensureEvents();
      const snap = await call<DocumentSnapshotDto>(CMD.importStep, { path });
      resetCorrelation();
      return snap;
    },
    async closeDocument(): Promise<void> {
      await call<void>(CMD.closeDocument);
      resetCorrelation();
    },
    async checkRecovery(): Promise<RecoveryInfo | null> {
      return call<RecoveryInfoDto | null>(CMD.checkRecovery);
    },
    async recoverDocument(accept: boolean): Promise<DocumentSnapshot | null> {
      if (accept) await ensureEvents(); // a recover-open emits like an open
      const snap = await call<DocumentSnapshotDto | null>(CMD.recoverDocument, { accept });
      if (accept) resetCorrelation(); // a discard (accept:false) opens no new document
      return snap;
    },
    async openFileDialog(): Promise<string | null> {
      return call<string | null>(CMD.openFileDialog);
    },

    async saveDocument(path?: string): Promise<void> {
      // `path` null ⇒ the backend reuses the last save path (an unsaved document
      // with no path rejects with an io error; the caller falls back to Save As).
      await call<void>(CMD.saveDocument, { path: path ?? null });
    },
    async saveDocumentAs(): Promise<string | null> {
      const path = await call<string | null>(CMD.saveFileDialog);
      if (!path) return null; // cancelled
      await call<void>(CMD.saveDocument, { path });
      return path;
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
    finishSketch,
    cancelSketch,
    deleteSketch,
    beginGesture,
    solveDrag,
    endGesture,
    promoteSelection,
    faceSketchPlane,
    elementInfo,
    resolveRefs,
    applyEditCommand,
    getOperationParams,

    // ── Two-level preview (local seam; backend preview verb TBD) ──────────────
    beginPreview: lane.beginPreview,
    updatePreview: lane.updatePreview,
    endPreview: lane.endPreview,
    onPreviewResult: lane.onPreviewResult,
  };
}
