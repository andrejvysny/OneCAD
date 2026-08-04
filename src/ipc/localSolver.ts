/*
 * localSolver — the mock sketch lane + the (still-shared) op-preview lane.
 *
 * ── Current role (post R-WP12/F-WP9 — the sketch half is MOCK-ONLY) ──────────
 * The real solver lane is LIVE: `tauriClient` dispatches sketch verbs
 * (enter/upsert/finish/BeginGesture/SolveDrag/EndGesture, SCHEMA §7.4) as real
 * Tauri commands through Rust → the worker's PlaneGCS actor, and does NOT route
 * them through this module. The sketch functions here (identity-echo solve, DOF
 * heuristic + deterministic conflict detection via mockSketch.solveSketch) serve
 * ONLY the mock client — browser demos, vitest, and the Playwright mock lane.
 *
 * What IS still shared by both clients:
 *   - the op PREVIEW lane (beginPreview/updatePreview/endPreview — shared
 *     pacing/session state; Tauri injects exact kernel PreviewOp meshes), and
 *   - the plane/region cache seams (cacheSketchPlane/cacheFinishedRegions) the
 *     tauri client feeds so beginPreview can resolve profiles.
 *
 * The ONE thing that differs between mock and tauri is COMMIT: materializing a
 * previewed op. The mock commits into its local document model; the tauri client
 * commits through the real `apply_edit_command`. That difference is injected as
 * the `commit` dependency, so this lane stays commit-agnostic.
 */
import type {
  ApplyOperationResult,
  BeginGestureResult,
  DragSolveResult,
  EnterSketchTarget,
  OperationOp,
  PreviewDraft,
  PreviewFailure,
  PreviewParams,
  PreviewResult,
  PreviewSession,
  SketchConstraint,
  SketchEntity,
  SketchPlane,
  SketchRegion,
  SketchSession,
  SketchUpsertResult,
  Unsubscribe,
} from "./types";
import { makeExtrudeBodyMesh } from "./mockMeshes";
import { lookupMockFace, mockProjectedContent } from "./mockFaceGeometry";
import { detectRegions, planeFor, solveSketch } from "./mockSketch";
import { profileFromRegion, type PrismProfile } from "@/tools/preview/prismPreview";
import { buildPreviewOp, supportsPreview, type PreviewSessionState } from "./previewOps";
import { mintRecordId } from "./tauriCommandMap";
import { trace } from "@/debug/trace";

/** Simulated latency for sketch enter/finish round-trips (independent of the doc latency). */
const SKETCH_LATENCY_MS = 30;

const wait = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

function previewFailure(error: unknown): PreviewFailure {
  let kind: PreviewFailure["kind"] = "unknown";
  let message = error instanceof Error ? error.message : String(error);
  if (typeof error === "object" && error !== null) {
    const candidate = error as { kind?: unknown; message?: unknown };
    if (
      candidate.kind === "opFailed" ||
      candidate.kind === "invalidCommand" ||
      candidate.kind === "worker" ||
      candidate.kind === "noDocument" ||
      candidate.kind === "needsRepair" ||
      candidate.kind === "stalePreview"
    ) {
      kind = candidate.kind;
    }
    if (typeof candidate.message === "string") message = candidate.message;
  }
  return {
    kind,
    message,
    structural: kind !== "opFailed" && kind !== "stalePreview",
  };
}

/** Deep clone so callers can't mutate the lane's stored session. */
function cloneSession(s: SketchSession): SketchSession {
  return JSON.parse(JSON.stringify(s)) as SketchSession;
}

/**
 * One live preview session. Extends the builder-visible {@link PreviewSessionState}
 * (`ipc/previewOps.ts`, the single op mapper) with this lane's transport
 * bookkeeping. `plane`/`profile` are the MOCK lane's local-synthesis inputs and
 * exist only for the profile-based ops (Extrude/Revolve) — the backend lane never
 * reads them.
 */
interface LaneSession extends PreviewSessionState {
  previewBodyId: string;
  plane?: SketchPlane;
  profile?: PrismProfile;
  lastEpoch: number;
  previewInFlight: boolean;
  queuedPreview: { params: PreviewParams; epoch: number } | null;
}

export interface LocalSolverDeps {
  /**
   * Materialize a previewed op. The mock wires this to its local document model;
   * the tauri client wires it to the real `apply_edit_command` path. Must emit
   * the client's own `document-changed` (both do) so meshes flow the usual way.
   */
  commit(op: OperationOp): Promise<ApplyOperationResult>;
  /** Live document latency (ms) used to pace the drag-time L2 preview mesh. */
  latencyMs(): number;
  /**
   * OPTIONAL backend preview (MODEL-OPS W3). When present, the drag-time exact
   * mesh comes from the real kernel (`PreviewOp` — the op run against a throwaway
   * copy of the head), so a Cut actually subtracts and Revolve/Fillet/Shell get a
   * real preview instead of none.
   *
   * When ABSENT — the mock client, which has no worker — the lane falls back to
   * the local prism synthesis it always used. That is why this is an injected
   * dependency rather than a fork of the lane: session bookkeeping, epochs and
   * `emitPreviewResult` stay in ONE place serving both clients.
   *
   * Rejects with a typed failure when the kernel cannot produce a candidate; the
   * lane publishes that reason while the controller retains the last good mesh.
   */
  previewOp?(
    op: OperationOp,
    sketchId: string | undefined,
  ): Promise<{
    bodies: { bodyId: string; mesh: ArrayBuffer }[];
    changedBodies?: string[];
    deletedBodies?: string[];
    needsRepair?: unknown[];
  }>;
}

/**
 * The public lane surface — exactly the CadClient sketch + preview methods, plus
 * `resolveProfileInput` (so a caller's own op-commit path can reuse the finished
 * region → {plane, profile}) and split reset seams.
 */
export interface LocalSolverLane {
  enterSketch(target: EnterSketchTarget): Promise<SketchSession>;
  sketchUpsert(
    sketchId: string,
    entities: SketchEntity[],
    constraints: SketchConstraint[],
  ): Promise<SketchUpsertResult>;
  finishSketch(sketchId: string): Promise<{ regions: SketchRegion[] }>;
  cancelSketch(sketchId: string): Promise<void>;
  beginGesture(sketchId: string, dragPointId: string): Promise<BeginGestureResult>;
  solveDrag(target: [number, number]): Promise<DragSolveResult | null>;
  endGesture(finalTarget?: [number, number]): Promise<SketchUpsertResult>;
  /** True if a lane session already exists for this sketch id (mock re-entry parity). */
  hasSession(sketchId: string): boolean;
  /** Read-only clone of a live lane session (always-visible static layer read), or null. */
  peekSession(sketchId: string): SketchSession | null;
  /** Drop a single sketch's lane session + revision (mock DeleteSketch compensation). */
  dropSession(sketchId: string): void;
  beginPreview(draft: PreviewDraft): Promise<PreviewSession>;
  updatePreview(sessionId: string, params: PreviewParams, epoch: number): void;
  endPreview(sessionId: string, commit: boolean): Promise<ApplyOperationResult | null>;
  onPreviewResult(cb: (r: PreviewResult) => void): Unsubscribe;
  /** Resolve a finished sketch region → the {plane, profile} a profile-based op consumes. */
  resolveProfileInput(
    sketchId: string | undefined,
    regionId: string | undefined,
  ): { plane: SketchPlane; profile: PrismProfile };
  /** @deprecated Alias of {@link resolveProfileInput} kept for existing callers (mockClient). */
  resolveExtrudeInput(
    sketchId: string | undefined,
    regionId: string | undefined,
  ): { plane: SketchPlane; profile: PrismProfile };
  /** Resolve a sketch LINE entity → its (u,v) endpoints (revolve axis synthesis). */
  resolveSketchLine(
    sketchId: string | undefined,
    lineId: string | undefined,
  ): { a: [number, number]; b: [number, number] } | null;
  /** Seam: feed a real enter_sketch plane so beginPreview resolves it (tauri). */
  cacheSketchPlane(sketchId: string, plane: SketchPlane): void;
  /** Seam: feed real finish_sketch regions so beginPreview builds the profile (tauri). */
  cacheFinishedRegions(sketchId: string, regions: SketchRegion[]): void;
  /** Forget sketch sessions (mirrors the mock's `resetMockSketches`). */
  resetSketches(): void;
  /** Forget preview sessions + finished regions (mirrors `resetMockDocument`). */
  resetPreview(): void;
}

export function createLocalSolverLane(deps: LocalSolverDeps): LocalSolverLane {
  // Sketch-lane state.
  const sketchSessions = new Map<string, SketchSession>();
  const sketchPlanes = new Map<string, SketchPlane>();
  const sketchRevisions = new Map<string, number>();
  let nextSketchSeq = 1;
  const finishedRegions = new Map<string, SketchRegion[]>();

  // Drag-gesture state (mock identity drag — echoes the target position).
  let gestureSeq = 0;
  let activeGesture: { sketchId: string; dragPointId: string; nextSeq: number } | null = null;

  // Preview-lane state.
  const previewSessions = new Map<string, LaneSession>();
  const previewListeners = new Set<(r: PreviewResult) => void>();
  let nextSessionSeq = 1;

  function emitPreviewResult(r: PreviewResult): void {
    for (const cb of [...previewListeners]) cb(r);
  }

  function needsRepairMessage(items: unknown[]): string {
    const first = items[0];
    if (typeof first !== "object" || first === null) {
      return `Preview needs ${items.length} reference repair${items.length === 1 ? "" : "s"}`;
    }
    const detail = first as { refId?: unknown; reason?: unknown; message?: unknown };
    const ref = typeof detail.refId === "string" ? ` ${detail.refId}` : "";
    const reason =
      typeof detail.message === "string"
        ? detail.message
        : typeof detail.reason === "string"
          ? detail.reason
          : "unresolved reference";
    return `Preview needs repair${ref}: ${reason}`;
  }

  function resolveProfileInput(
    sketchId: string | undefined,
    regionId: string | undefined,
  ): { plane: SketchPlane; profile: PrismProfile } {
    if (!sketchId) throw new Error("Profile preview requires sketchId");
    if (!regionId) throw new Error("Profile preview requires regionId");
    const regions = finishedRegions.get(sketchId);
    if (!regions) {
      throw new Error(`No solved regions cached for sketch ${sketchId}`);
    }
    const region = regions.find((candidate) => candidate.regionId === regionId);
    if (!region) {
      const available = regions.map((candidate) => candidate.regionId).join(", ") || "none";
      throw new Error(`Unknown region ${regionId} for sketch ${sketchId}; available: ${available}`);
    }
    const plane = sketchSessions.get(sketchId)?.plane ?? sketchPlanes.get(sketchId);
    if (!plane) throw new Error(`No sketch plane cached for ${sketchId}`);
    const profile = region ? profileFromRegion(region) : null;
    if (!profile) {
      throw new Error(`Region ${regionId} has no complete triangulated profile`);
    }
    return { plane, profile };
  }

  function resolveSketchLine(
    sketchId: string | undefined,
    lineId: string | undefined,
  ): { a: [number, number]; b: [number, number] } | null {
    if (!sketchId || !lineId) return null;
    const session = sketchSessions.get(sketchId);
    const line = session?.entities.find((e) => e.id === lineId && e.type === "Line");
    if (!line?.p0 || !line?.p1) return null;
    return { a: line.p0, b: line.p1 };
  }

  function dispatchBackendPreview(
    sessionId: string,
    s: LaneSession,
    params: PreviewParams,
    epoch: number,
  ): void {
    if (!deps.previewOp) return;
    s.previewInFlight = true;
    const bodyId = s.previewBodyId;
    let op: OperationOp;
    try {
      op = buildPreviewOp({ ...s, latestParams: { ...params } });
    } catch (error) {
      s.previewInFlight = false;
      emitPreviewResult({ sessionId, epoch, bodyId, error: previewFailure(error) });
      return;
    }
    void deps
      .previewOp(op, s.sketchId)
      .then((result) => {
        if (previewSessions.get(sessionId) !== s) return;
        if (result.needsRepair && result.needsRepair.length > 0) {
          emitPreviewResult({
            sessionId,
            epoch,
            bodyId,
            error: {
              kind: "needsRepair",
              message: needsRepairMessage(result.needsRepair),
              structural: true,
              evidence: result.needsRepair,
            },
          });
          return;
        }
        if (result.bodies.length === 0 && (result.deletedBodies?.length ?? 0) === 0) {
          emitPreviewResult({
            sessionId,
            epoch,
            bodyId,
            error: {
              kind: "opFailed",
              message: "Preview produced no changed body",
              structural: false,
            },
          });
          return;
        }
        emitPreviewResult({
          sessionId,
          epoch,
          bodyId,
          bodies: result.bodies,
          replacedBodyIds: [
            ...(result.changedBodies ?? []),
            ...(result.deletedBodies ?? []),
          ],
        });
      })
      .catch((error: unknown) => {
        if (previewSessions.get(sessionId) !== s) return;
        emitPreviewResult({ sessionId, epoch, bodyId, error: previewFailure(error) });
      })
      .finally(() => {
        if (previewSessions.get(sessionId) !== s) return;
        s.previewInFlight = false;
        const next = s.queuedPreview;
        s.queuedPreview = null;
        if (next) dispatchBackendPreview(sessionId, s, next.params, next.epoch);
      });
  }

  return {
    async enterSketch(target: EnterSketchTarget): Promise<SketchSession> {
      await wait(SKETCH_LATENCY_MS);
      let id: string;
      let planeKind: SketchSession["plane"]["kind"] = "XY";
      // A face- or DATUM-hosted sketch carries its own explicit basis (the
      // backend-resolved frame the caller passed through); a world-plane one
      // derives it from the named kind (`planeFor`).
      let explicitPlane: SketchPlane | null = null;
      // A NEW host-face sketch is seeded with the face's projected boundary as
      // LOCKED reference geometry — the real lane gets it from
      // `add_sketch_on_face` + the `enter_sketch` DTO, so a mock session without
      // it would be structurally different (no region to extrude, no locked
      // geometry to guard) and every mock-lane test of this flow false-green.
      let seed: { entities: SketchEntity[]; constraints: SketchConstraint[] } | null = null;
      if (typeof target === "string") {
        id = target;
      } else if ("newOnFace" in target) {
        explicitPlane = target.plane;
        planeKind = "custom";
        id = target.sketchId ?? `sk-${nextSketchSeq++}`;
        const found = lookupMockFace(
          target.newOnFace.bodyId,
          target.newOnFace.elementId,
          target.newOnFace.topoKey,
        );
        if (found.kind === "planar") seed = mockProjectedContent(found.geometry);
      } else if ("newOnDatum" in target) {
        // DATUM W1. This one branch covers the mock lane end to end — the mock
        // client shares this lane, and only `commit` differs between the two.
        explicitPlane = target.plane;
        planeKind = "custom";
        id = target.sketchId ?? `sk-${nextSketchSeq++}`;
      } else {
        planeKind = target.newOnPlane;
        id = target.sketchId ?? `sk-${nextSketchSeq++}`;
      }
      let session = sketchSessions.get(id);
      if (!session) {
        const entities = seed?.entities ?? [];
        const constraints = seed?.constraints ?? [];
        // Fixed-pinned locked geometry contributes ZERO net DOF, so a freshly
        // projected boundary still reports FullyConstrained (see `solveDof`).
        const { dof, status, conflicting } = solveSketch(entities, constraints);
        session = {
          sketchId: id,
          plane: explicitPlane ?? planeFor(planeKind),
          entities,
          constraints,
          dof,
          status,
          conflicting,
        };
        sketchSessions.set(id, session);
        sketchPlanes.set(id, session.plane);
        sketchRevisions.set(id, 0);
      }
      return cloneSession(session);
    },

    async sketchUpsert(
      sketchId: string,
      entities: SketchEntity[],
      constraints: SketchConstraint[],
    ): Promise<SketchUpsertResult> {
      // Near-synchronous: drawing must feel instant; the DOF badge refreshes live.
      await wait(0);
      const prev = sketchSessions.get(sketchId);
      const { dof, status, conflicting } = solveSketch(entities, constraints);
      const session: SketchSession = {
        sketchId,
        plane: prev?.plane ?? planeFor("XY"),
        entities,
        constraints,
        dof,
        status,
        conflicting,
      };
      sketchSessions.set(sketchId, session);
      sketchPlanes.set(sketchId, session.plane);
      const rev = (sketchRevisions.get(sketchId) ?? 0) + 1;
      sketchRevisions.set(sketchId, rev);
      // The local solver is an identity solve (echoes positions) — nothing moved.
      return { sketchId, sketchRevision: rev, dof, status, conflicting, solvedPositions: {} };
    },

    async finishSketch(sketchId: string): Promise<{ regions: SketchRegion[] }> {
      await wait(SKETCH_LATENCY_MS);
      const session = sketchSessions.get(sketchId);
      if (session) {
        const regions = detectRegions(session.entities);
        finishedRegions.set(sketchId, regions); // cache for extrude synthesis
        return { regions };
      }
      // Sessionless finish (the model-mode record guarantee): answer from the
      // cache instead of clobbering it with [] — the armed preview lane still
      // resolves its profile from finishedRegions on a re-arm.
      return { regions: finishedRegions.get(sketchId) ?? [] };
    },

    async cancelSketch(_sketchId: string): Promise<void> {
      await wait(0);
      activeGesture = null;
      // Real backend discards scratch; the lane keeps the last committed session.
    },

    // ── Drag gesture (mock identity drag: solveDrag echoes the target) ─────────
    async beginGesture(sketchId: string, dragPointId: string): Promise<BeginGestureResult> {
      await wait(0);
      activeGesture = { sketchId, dragPointId, nextSeq: 1 };
      return { gestureId: ++gestureSeq, ready: true };
    },

    async solveDrag(target: [number, number]): Promise<DragSolveResult | null> {
      await wait(0);
      const g = activeGesture;
      if (!g) return null;
      const seq = g.nextSeq++;
      const session = sketchSessions.get(g.sketchId);
      return {
        gestureId: gestureSeq,
        seq,
        status: "success",
        dof: session?.dof ?? 0,
        conflicting: [],
        positions: { [g.dragPointId]: target },
        solveMicros: 0,
        superseded: false,
      };
    },

    async endGesture(finalTarget?: [number, number]): Promise<SketchUpsertResult> {
      await wait(SKETCH_LATENCY_MS);
      const g = activeGesture;
      activeGesture = null;
      const sketchId = g?.sketchId ?? "";
      const session = sketchId ? sketchSessions.get(sketchId) : undefined;
      const rev = (sketchRevisions.get(sketchId) ?? 0) + 1;
      if (sketchId) sketchRevisions.set(sketchId, rev);
      return {
        sketchId,
        sketchRevision: rev,
        dof: session?.dof ?? 0,
        status: session?.status ?? "FullyConstrained",
        conflicting: [], // mock lane never conflicts (deterministic)
        solvedPositions: g && finalTarget ? { [g.dragPointId]: finalTarget } : {},
      };
    },

    async beginPreview(draft: PreviewDraft): Promise<PreviewSession> {
      const sessionId = `pv-${nextSessionSeq++}`;
      const previewBodyId = `preview:${sessionId}`;
      // Support IS builder membership (ipc/previewOps.ts) — no opType branch here.
      if (!supportsPreview(draft.opType)) {
        throw new Error(`Preview session does not support ${draft.opType}`);
      }
      // Only the PROFILE-based ops bind a sketch region up-front. Fillet/Chamfer/
      // Shell/Boolean/Hole operate on existing solids and carry no {plane, profile},
      // so resolving one for them would reject every legitimate draft.
      let plane: SketchPlane | undefined;
      let profile: PrismProfile | undefined;
      if (draft.opType === "Extrude" || draft.opType === "Revolve") {
        ({ plane, profile } = resolveProfileInput(draft.sketchId, draft.regionId));
      }
      previewSessions.set(sessionId, {
        opId: draft.opId ?? mintRecordId(),
        opType: draft.opType,
        editFeatureId:
          typeof draft.params.featureId === "string" ? draft.params.featureId : undefined,
        previewBodyId,
        sketchId: draft.sketchId,
        regionId: draft.regionId,
        inputs: draft.inputs,
        plane,
        profile,
        latestParams: { ...draft.params },
        lastEpoch: 0,
        previewInFlight: false,
        queuedPreview: null,
      });
      return { sessionId, previewBodyId };
    },

    updatePreview(sessionId: string, params: PreviewParams, epoch: number): void {
      const s = previewSessions.get(sessionId);
      if (!s) return;
      // Extrude updates are complete immutable snapshots. Replacing instead of
      // merging drops stale target-face/body fields when the user changes mode.
      s.latestParams = { ...params };
      s.lastEpoch = epoch;

      // PreviewOp runs against the current head, not the feature's predecessor.
      // Re-editing there would double-apply the feature, so re-edits use L1 only.
      if (s.editFeatureId) return;

      // BACKEND lane (MODEL-OPS W3): the real kernel runs the candidate op on a
      // throwaway copy of the head, so the mesh IS what a commit would produce —
      // Cut subtracts, and every opType is previewable, not just Extrude.
      if (deps.previewOp) {
        if (s.previewInFlight) {
          s.queuedPreview = { params: { ...params }, epoch };
        } else {
          dispatchBackendPreview(sessionId, s, params, epoch);
        }
        return;
      }

      // LOCAL fallback (the mock client — no worker to ask). The lane NEVER fakes
      // geometry it cannot honestly synthesize: an op with no local synthesis
      // publishes an EMPTY, non-committed result under the same epoch instead of
      // going silent, so a caller's per-epoch bookkeeping (commit barrier, the
      // "computing preview" pending flag) still settles. The visible preview stays
      // whatever L1 the tool draws itself (revolve's lathe, the pattern ghosts).
      const bodyId = s.previewBodyId;
      const { plane, profile } = s;
      // Extrude: the one op with a real local prism synthesis.
      if (s.opType === "Extrude" && plane && profile) {
        const distance = Number(s.latestParams.distance ?? 0);
        setTimeout(() => {
          if (!previewSessions.has(sessionId)) return; // session ended → drop stale
          const mesh = makeExtrudeBodyMesh(profile, plane, distance);
          emitPreviewResult({ sessionId, epoch, bodyId, mesh });
        }, deps.latencyMs());
        return;
      }
      // Boolean: no mock CSG, but half the preview IS expressible without geometry
      // — the tool body is consumed. Publish that hide truthfully, with no mesh.
      if (s.opType === "Boolean") {
        const toolBodyId =
          typeof s.latestParams.toolBodyId === "string" && s.latestParams.toolBodyId
            ? [s.latestParams.toolBodyId]
            : [];
        setTimeout(() => {
          if (!previewSessions.has(sessionId)) return;
          emitPreviewResult({ sessionId, epoch, bodyId, bodies: [], replacedBodyIds: toolBodyId });
        }, deps.latencyMs());
        return;
      }
      // Revolve / Fillet / Chamfer / Shell / Hole: no local synthesis. Revolve
      // already has its L1 lathe (and the lane holds no axis to build one from
      // anyway); the edge, shell and hole ops have no mock kernel at all — a hole
      // in particular would need real CSG against the host mesh. Settle the epoch,
      // show nothing.
      setTimeout(() => {
        if (!previewSessions.has(sessionId)) return;
        emitPreviewResult({ sessionId, epoch, bodyId });
      }, deps.latencyMs());
    },

    async endPreview(sessionId: string, commit: boolean): Promise<ApplyOperationResult | null> {
      const s = previewSessions.get(sessionId);
      previewSessions.delete(sessionId);
      if (!s || !commit) {
        await wait(0);
        return null;
      }
      if (s.editFeatureId) {
        throw new Error("Feature re-edit must use a scalar update command");
      }
      const op = buildPreviewOp(s);
      trace(
        "lane",
        `endPreview COMMIT: session=${sessionId} op=${op.opType} opId=${op.opId ?? "?"} ` +
          `sketch=${"sketchId" in op ? op.sketchId : "?"} region=${"regionId" in op ? op.regionId : "?"}`,
        op.params,
      );
      const res = await deps.commit(op);
      trace(
        "lane",
        `endPreview COMMIT result: rev=${res.revision} changed=${res.changedBodies.length} ` +
          `removed=${res.removedBodies.length} error=${res.errorMessage ?? "none"}`,
      );
      // Deliver a committed signal under the FINAL epoch so the tool reconciles
      // (drops L1 once the matching-epoch result exists). The controller reads
      // only `epoch` for a committed result — the real body mesh flows through the
      // normal document-changed path — so the mesh field is intentionally empty.
      const committedBodyId = res.changedBodies[0]?.bodyId;
      if (committedBodyId) {
        emitPreviewResult({
          sessionId,
          epoch: s.lastEpoch,
          bodyId: committedBodyId,
          mesh: new ArrayBuffer(0),
          committed: true,
        });
      }
      return res;
    },

    onPreviewResult(cb: (r: PreviewResult) => void): Unsubscribe {
      previewListeners.add(cb);
      return () => previewListeners.delete(cb);
    },

    resolveProfileInput,
    // Legacy name kept as an alias — mockClient's op-commit path calls it.
    resolveExtrudeInput: resolveProfileInput,
    resolveSketchLine,

    hasSession(sketchId: string): boolean {
      return sketchSessions.has(sketchId);
    },

    peekSession(sketchId: string): SketchSession | null {
      const s = sketchSessions.get(sketchId);
      return s ? cloneSession(s) : null;
    },

    dropSession(sketchId: string): void {
      sketchSessions.delete(sketchId);
      sketchPlanes.delete(sketchId);
      sketchRevisions.delete(sketchId);
    },

    cacheSketchPlane(sketchId: string, plane: SketchPlane): void {
      const existing = sketchSessions.get(sketchId);
      sketchPlanes.set(sketchId, plane);
      if (existing) sketchSessions.set(sketchId, { ...existing, plane });
    },

    cacheFinishedRegions(sketchId: string, regions: SketchRegion[]): void {
      finishedRegions.set(sketchId, regions);
    },

    resetSketches(): void {
      sketchSessions.clear();
      sketchPlanes.clear();
      sketchRevisions.clear();
      nextSketchSeq = 1;
    },

    resetPreview(): void {
      previewSessions.clear();
      finishedRegions.clear();
      nextSessionSeq = 1;
    },
  };
}
