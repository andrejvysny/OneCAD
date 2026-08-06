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
 *
 * ── The drag gesture here is a KINEMATIC ECHO (SP-2) ─────────────────────────
 * `beginGesture`/`solveDrag`/`endGesture` honour the §7.4 target KINDS — the
 * handle keys and `curves` members each kind reports are exactly the real
 * worker's — but they only ever re-derive geometry from the cursor. There is NO
 * constraint propagation (a coincident neighbour does not follow, a Tangent does
 * not push a radius), NO solver refusal (a locked entity is not rejected, an
 * arc's sweep floor is not enforced) and no DOF change. Those behaviours are the
 * real PlaneGCS lane's and are covered by `src-tauri/tests` + the worker ctest
 * suite; asserting them against this module would be asserting the mock. What
 * this lane exists to prove is the PLUMBING: the shapes flow end to end and the
 * UI can be driven with no backend at all.
 */
import type {
  ApplyOperationResult,
  BeginGestureResult,
  CurveParams,
  DragSolveResult,
  EnterSketchTarget,
  GestureTarget,
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
import { planeFor, solveSketch } from "./mockSketch";
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
  beginGesture(
    sketchId: string,
    dragPointId: string,
    target?: GestureTarget,
  ): Promise<BeginGestureResult>;
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

// ── Mock drag gesture — the §7.4 target kinds, echoed kinematically ──────────

/** SCHEMA §7.4 radius floor (`MIN_GEOMETRY_SIZE`, mm): a radius drag saturates here. */
const MIN_GEOMETRY_SIZE = 0.01;

/** One open gesture: the kind plus the per-kind offsets captured ONCE at Begin. */
interface MockGesture {
  sketchId: string;
  dragPointId: string;
  nextSeq: number;
  kind: GestureTarget["kind"];
  /** The grabbed entity (frontend id); null on the legacy point path. */
  entityId: string | null;
  role: "Start" | "End" | "Center" | null;
  /** `entityBody`: every owned handle's pose at Begin (targets are ALWAYS derived
   *  from these, so a dropped frame cannot accumulate drift). */
  baseline: [string, [number, number]][];
  /** `entityBody`: the anchor `target − grab` is measured from. */
  grab: [number, number] | null;
  /** `radius`: `|grab − center| − radius`, captured at Begin (0 when `grab` absent). */
  radiusOffset: number;
}

/** Every point handle an entity OWNS, keyed `"<entityId>.<Position>"`, in §7.4
 *  handle order (`center` < `start` < `end`, `p0` < `p1`). */
function ownedHandles(e: SketchEntity): [string, [number, number]][] {
  const out: [string, [number, number]][] = [];
  const push = (role: string, p: [number, number] | undefined): void => {
    if (p) out.push([`${e.id}.${role}`, p]);
  };
  switch (e.type) {
    case "Point":
      push("Start", e.p0);
      break;
    case "Line":
      push("Start", e.p0);
      push("End", e.p1);
      break;
    case "Circle":
    case "Ellipse":
      push("Center", e.center);
      break;
    case "Arc":
      push("Center", e.center);
      push("Start", e.start);
      push("End", e.end);
      break;
  }
  return out;
}

/** Capture the gesture's per-kind offsets ONCE, at Begin — §7.4 requires that a
 *  live gesture cannot drift as it is dragged. */
function openGesture(
  sketchId: string,
  dragPointId: string,
  entities: SketchEntity[],
  target?: GestureTarget,
): MockGesture {
  const kind = target?.kind ?? "point";
  const entityId = target?.entityId ?? null;
  const entity = entityId === null ? undefined : entities.find((e) => e.id === entityId);
  // Absent `grab` on an entityBody anchors the FIRST owned handle (it teleports).
  const baseline = kind === "entityBody" && entity ? ownedHandles(entity) : [];
  const grab = target?.grab ?? (baseline.length > 0 ? baseline[0][1] : null);
  const center = entity?.center;
  const radiusOffset =
    kind === "radius" && center && target?.grab && entity?.radius !== undefined
      ? Math.hypot(target.grab[0] - center[0], target.grab[1] - center[1]) - entity.radius
      : 0;
  return {
    sketchId,
    dragPointId,
    nextSeq: 1,
    kind,
    entityId,
    role: target?.role ?? null,
    baseline,
    grab,
    radiusOffset,
  };
}

/** What one drag step reports: moved point handles + CHANGED curve members. */
interface GestureEcho {
  positions: Record<string, [number, number]>;
  curves: Record<string, CurveParams>;
}

/** A fresh empty echo. Never a shared constant: a caller that MERGES a drag
 *  response into an accumulator would otherwise poison every later gesture. */
const noEcho = (): GestureEcho => ({ positions: {}, curves: {} });

/**
 * The per-kind echo. `point` is the pre-SP-2 identity echo, unchanged. A non-point
 * kind whose entity cannot be resolved reports NOTHING rather than degrading to a
 * point drag — §7.4 forbids the degrade (it would move a handle nobody grabbed);
 * the real worker answers `REF_UNRESOLVED` at Begin, which this lane cannot.
 */
function echoGesture(
  g: MockGesture,
  entity: SketchEntity | undefined,
  target: [number, number],
): GestureEcho {
  if (g.kind === "point") return { positions: { [g.dragPointId]: target }, curves: {} };
  if (g.kind === "entityBody") {
    if (!g.grab) return noEcho();
    const [dx, dy] = [target[0] - g.grab[0], target[1] - g.grab[1]];
    const positions: Record<string, [number, number]> = {};
    for (const [key, p] of g.baseline) positions[key] = [p[0] + dx, p[1] + dy];
    return { positions, curves: {} };
  }
  const center = entity?.center;
  if (!entity || !center || g.entityId === null) return noEcho();
  const reach = Math.hypot(target[0] - center[0], target[1] - center[1]);
  if (g.kind === "radius") {
    const radius = Math.max(reach - g.radiusOffset, MIN_GEOMETRY_SIZE);
    return { positions: {}, curves: { [g.entityId]: { radius } } };
  }
  // arcEnd: the grabbed endpoint teleports to the cursor; radius + that endpoint's
  // angle follow from it (the sibling endpoint follows via `applySolvedCurves`).
  const role = g.role === "End" ? "End" : "Start";
  const angle = Math.atan2(target[1] - center[1], target[0] - center[0]);
  const swept: CurveParams =
    role === "End" ? { radius: reach, endAngle: angle } : { radius: reach, startAngle: angle };
  return { positions: { [`${g.entityId}.${role}`]: target }, curves: { [g.entityId]: swept } };
}

export function createLocalSolverLane(deps: LocalSolverDeps): LocalSolverLane {
  // Sketch-lane state.
  const sketchSessions = new Map<string, SketchSession>();
  const sketchPlanes = new Map<string, SketchPlane>();
  const sketchRevisions = new Map<string, number>();
  let nextSketchSeq = 1;
  const finishedRegions = new Map<string, SketchRegion[]>();

  // Drag-gesture state (kinematic echo — see the module header).
  let gestureSeq = 0;
  let activeGesture: MockGesture | null = null;

  /** The gesture's grabbed entity, read LIVE from the session (never cached: an
   *  upsert mid-gesture would strand a stale copy). */
  function gestureEntity(g: MockGesture): SketchEntity | undefined {
    if (g.entityId === null) return undefined;
    return sketchSessions.get(g.sketchId)?.entities.find((e) => e.id === g.entityId);
  }

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
      // The local solver is an identity solve (echoes positions) — nothing moved,
      // and with no propagation nothing reshapes a curve either.
      return {
        sketchId,
        sketchRevision: rev,
        dof,
        status,
        conflicting,
        solvedPositions: {},
        solvedCurves: {},
      };
    },

    async finishSketch(sketchId: string): Promise<{ regions: SketchRegion[] }> {
      await wait(SKETCH_LATENCY_MS);
      const session = sketchSessions.get(sketchId);
      if (session) {
        // Dynamic: mockRegions pulls in three's ShapeUtils/Vector2, and this
        // lane is shared with the real (tauri) client, which never calls this
        // path — keeping the import here, not static, keeps three out of the
        // entry chunk on the real lane.
        const { detectRegions } = await import("./mockRegions");
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

    // ── Drag gesture (kinematic echo, per §7.4 target kind) ────────────────────
    async beginGesture(
      sketchId: string,
      dragPointId: string,
      target?: GestureTarget,
    ): Promise<BeginGestureResult> {
      await wait(0);
      const entities = sketchSessions.get(sketchId)?.entities ?? [];
      activeGesture = openGesture(sketchId, dragPointId, entities, target);
      return { gestureId: ++gestureSeq, ready: true };
    },

    async solveDrag(target: [number, number]): Promise<DragSolveResult | null> {
      await wait(0);
      const g = activeGesture;
      if (!g) return null;
      const seq = g.nextSeq++;
      const session = sketchSessions.get(g.sketchId);
      const echo = echoGesture(g, gestureEntity(g), target);
      return {
        gestureId: gestureSeq,
        seq,
        status: "success",
        dof: session?.dof ?? 0,
        conflicting: [],
        positions: echo.positions,
        curves: echo.curves,
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
      const echo = g && finalTarget ? echoGesture(g, gestureEntity(g), finalTarget) : noEcho();
      return {
        sketchId,
        sketchRevision: rev,
        dof: session?.dof ?? 0,
        status: session?.status ?? "FullyConstrained",
        conflicting: [], // mock lane never conflicts (deterministic)
        solvedPositions: echo.positions,
        solvedCurves: echo.curves,
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
