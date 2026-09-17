/*
 * ModelToolController — edge-op preview PUBLICATION ORDER (review §4.1, A15, spec §9.4).
 *
 * Several `openEdgeOpPreview` calls can be in flight inside ONE arm generation: a
 * type flip and a same-turn chamfer sync, a failed-commit re-arm and a
 * [Flip reference]. The backend answers them in whatever order it likes, and
 * `armGen` alone cannot tell them apart. Pinned here, with every `beginPreview`
 * held on a deferred and released in REVERSE order:
 *
 *  - the NEWEST open is the one installed; every obsolete session is ended;
 *  - a fresh ✓ waits for the newest reopen and commits exactly once, from it;
 *  - an arm change under a waiting ✓ is reported, never dropped silently;
 *  - nothing is published or ended while a commit is materializing.
 *
 * No clock is used for ordering. `until` only yields microtasks: every backend
 * double here settles on a microtask, so its bound is a hang detector.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  ElementInfo,
  PreviewDraft,
  PreviewParams,
  PreviewResult,
  PreviewSession,
  PromotePick,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { viewportStore } from "@/stores/viewportStore";
import { operationAttemptStore } from "@/stores/operationAttemptStore";
import { resetStores } from "@/test/resetStores";
import { __resetLogForTests, logSnapshot } from "@/debug/log";
import { buildPreviewOp } from "@/ipc/previewOps";

const okResult = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#1" }],
  removedBodies: [],
});

const EDGES: EntityRef[] = [
  {
    kind: "edge",
    id: "body1#e:0",
    bodyId: "body1",
    topoKey: "e:0",
    elementId: "el_e0",
    anchor: { worldPoint: [1, 2, 3] },
  },
];

/** The edge's two adjacent faces, face-ordinal ascending (SCHEMA §7.6). */
const ADJACENT = ["f:3", "f:5"];

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

/** Yield microtasks until `predicate` holds (see the header: a hang detector, not a wait). */
async function until(label: string, predicate: () => boolean): Promise<void> {
  for (let i = 0; i < 2000; i += 1) {
    if (predicate()) return;
    await Promise.resolve();
  }
  throw new Error(`never reached: ${label}`);
}

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    planePixelWorld: vi.fn(() => 1),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setRegionSelected: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => ({ origin: [0, 0, 100] as const, dir: [0, 0, -1] as const })),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    clearPreviewBody: vi.fn(),
    showRevolveAxisCandidates: vi.fn(),
    setRevolveAxisHover: vi.fn(),
    showRevolvePreview: vi.fn(),
    setRevolveAngle: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    probePick: vi.fn(() => null),
  };
}

/** What the lane double does with the calls it receives; each test flips these. */
interface LaneControl {
  /** Park every `beginPreview` on a deferred the test releases. */
  holdOpens: boolean;
  /** Sessions whose exact answers are parked instead of delivered. */
  holdAnswersFor: Set<string>;
  /** Answer the COMMIT's final epoch with a kernel refusal. */
  refuseCommit: boolean;
  /** The refusal's kind (a `stalePreview` asks the lane to re-send once). */
  refusalKind?: "opFailed" | "stalePreview";
  /** Park every face promotion until the test releases them. */
  holdPromotes: boolean;
  /** Park every range analysis until the test releases them. */
  holdRanges: boolean;
}

interface OpenRecord {
  sessionId: string;
  draft: PreviewDraft;
  release: () => void;
  fail: (error: Error) => void;
}

function makeClientMock(lane: LaneControl) {
  const opens: OpenRecord[] = [];
  const parkedPromotes: Array<() => void> = [];
  const parkedRanges: Array<() => void> = [];
  const parkedAnswers = new Map<string, Array<() => void>>();
  let previewCb: ((r: PreviewResult) => void) | null = null;

  const answer = (sessionId: string, epoch: number): void => {
    const refuse = lane.refuseCommit && toolStore.getState().phase === "committing";
    previewCb?.({
      sessionId,
      epoch,
      bodyId: "preview",
      ...(refuse
        ? { error: { kind: lane.refusalKind ?? "opFailed", message: "kernel refused the chamfer", structural: false } }
        : { bodies: [], replacedBodyIds: [] }),
    });
  };

  const client = {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      previewCb = cb;
      return () => {};
    }),
    onDocumentChanged: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => null),
    finishSketch: vi.fn(() => Promise.resolve({ regions: [] })),
    getSketchRegions: vi.fn(() => Promise.resolve({ regions: [] })),
    prepareEdgeOp: vi.fn(() =>
      Promise.resolve({
        snapshotId: 7,
        targetBodyId: "body1",
        edges: EDGES.map((edge) => ({
          topoKey: edge.topoKey ?? "",
          elementId: edge.elementId ?? "",
          bodyId: "body1",
          kind: "edge" as const,
          picked: true,
          anchor: edge.anchor,
          contour: 0,
          adjacentFaces: ADJACENT,
        })),
        refusal: null,
      }),
    ),
    analyzeEdgeOpRange: vi.fn(() => {
      const range = {
        snapshotId: 7,
        mode: "Chamfer" as const,
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
        refusal: null,
      };
      if (!lane.holdRanges) return Promise.resolve(range);
      const gate = deferred<typeof range>();
      parkedRanges.push(() => gate.resolve(range));
      return gate.promise;
    }),
    promoteSelection: vi.fn((bodyId: string, picks: PromotePick[]) => {
      const promoted = picks.map((p) => ({
        topoKey: p.topoKey,
        elementId: `el_${p.topoKey.replace(":", "_")}`,
        kind: p.topoKey.startsWith("e:") ? "edge" : "face",
        bodyId,
      }));
      if (!lane.holdPromotes) return Promise.resolve(promoted);
      const gate = deferred<typeof promoted>();
      parkedPromotes.push(() => gate.resolve(promoted));
      return gate.promise;
    }),
    elementInfo: vi.fn((bodyId: string, elementId: string, topoKey?: string) =>
      Promise.resolve({
        elementId,
        topoKey: topoKey ?? "",
        bodyId,
        kind: "face",
        surfaceType: 0,
        curveType: -1,
        center: topoKey === "f:5" ? [50, 50, 50] : [10, 20, 30],
        normal: [0, 0, 1],
        hasNormal: true,
        size: 10,
        magnitude: 100,
      } as ElementInfo),
    ),
    beginPreview: vi.fn((draft: PreviewDraft) => {
      const sessionId = `pv-${opens.length + 1}`;
      const session: PreviewSession = { sessionId, previewBodyId: `pb-${opens.length + 1}` };
      let fail!: (error: Error) => void;
      const gate = deferred<PreviewSession>();
      const failing = new Promise<PreviewSession>((resolve, reject) => {
        fail = reject;
        void gate.promise.then(resolve);
      });
      opens.push({ sessionId, draft, release: () => gate.resolve(session), fail });
      if (!lane.holdOpens) gate.resolve(session);
      return failing;
    }),
    // A responsive kernel: every request is answered on a microtask — AFTER the
    // caller registered its commit barrier — unless the test parks that session.
    updatePreview: vi.fn((sessionId: string, _params: PreviewParams, epoch: number) => {
      if (lane.holdAnswersFor.has(sessionId)) {
        const parked = parkedAnswers.get(sessionId) ?? [];
        parked.push(() => answer(sessionId, epoch));
        parkedAnswers.set(sessionId, parked);
        return;
      }
      queueMicrotask(() => answer(sessionId, epoch));
    }),
    endPreview: vi.fn((_sessionId: string, _commit: boolean) => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };

  const releaseAnswers = (sessionId: string): void => {
    lane.holdAnswersFor.delete(sessionId);
    const parked = parkedAnswers.get(sessionId) ?? [];
    parkedAnswers.delete(sessionId);
    for (const deliver of parked) deliver();
  };

  const releasePromotes = (): void => {
    lane.holdPromotes = false;
    for (const release of parkedPromotes.splice(0)) release();
  };

  const releaseRanges = (): void => {
    for (const release of parkedRanges.splice(0)) release();
  };

  return { client, opens, releaseAnswers, releasePromotes, parkedPromotes, releaseRanges, parkedRanges };
}

interface ControllerInternals {
  previewSessions: Array<{ session: PreviewSession; draft: PreviewDraft }>;
  chamferSync: Promise<void>;
  chamferFaceRefs: Map<string, unknown>;
  chamferFlipped: boolean;
  chamferPairs: { pairs: Array<{ edgeId: string; faceId: string }> } | null;
  chamferSyncPending: number;
  edgeOpOpening: unknown;
  fillet: { phase: string; edgeOp: string; radius: number; distance2: number | null; angleDeg: number | null };
  filletDraft: number | null;
  armGen: number;
  throttle: { setTrailingMs(ms: number): void; readonly inFlight: number | null };
  openEdgeOpPreview(gen: number): Promise<void>;
}

describe("ModelToolController — edge-op preview publication order", () => {
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let lane: LaneControl;
  let mock: ReturnType<typeof makeClientMock>;

  function build(): void {
    lane = { holdOpens: false, holdAnswersFor: new Set(), refuseCommit: false, holdPromotes: false, holdRanges: false };
    mock = makeClientMock(lane);
    controller = new ModelToolController({
      engine: makeEngineMock() as unknown as ViewportEngine,
      client: mock.client as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  }

  beforeEach(() => {
    resetStores();
    // Long enough that no barrier ever times out inside a test: the order under
    // test must come from the deferreds, never from a timer.
    __setExactPreviewTimeoutForTests(60_000);
    container = document.createElement("div");
    document.body.appendChild(container);
    documentStore.setState({ revision: 1 });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
    __resetLogForTests({ enabled: false });
  });

  const internals = (): ControllerInternals => controller as unknown as ControllerInternals;
  const installed = (): string | null => internals().previewSessions[0]?.session.sessionId ?? null;
  const endCalls = (): Array<[string, boolean]> =>
    mock.client.endPreview.mock.calls as unknown as Array<[string, boolean]>;
  const endedCount = (id: string, commit: boolean): number =>
    endCalls().filter(([s, c]) => s === id && c === commit).length;
  const commits = (): string[] => endCalls().filter(([, c]) => c).map(([s]) => s);
  const draftOf = (id: string): PreviewDraft => {
    const open = mock.opens.find((o) => o.sessionId === id);
    if (!open) throw new Error(`no beginPreview for ${id}`);
    return open.draft;
  };
  const updatesFor = (id: string): Array<[string, PreviewParams, number]> =>
    (mock.client.updatePreview.mock.calls as unknown as Array<[string, PreviewParams, number]>).filter(
      ([s]) => s === id,
    );
  const faceIdOf = (draft: PreviewDraft): string | undefined =>
    (draft.params.referenceFaces as Array<{ faceId: string }> | undefined)?.[0]?.faceId;
  const attemptPhase = (): string | undefined => operationAttemptStore.getState().attempt?.phase;
  const hint = (): string => viewportStore.getState().statusHint?.message ?? "";
  const lifecycle = (): string => toolChipStore.getState().previewLifecycle.status;
  const warned = (pattern: RegExp): boolean =>
    logSnapshot().some((e) => e.level === "warn" && pattern.test(e.msg));
  /** Nothing queued on the reference-face chain and no edge-op open in flight. */
  const quiescent = (): boolean => internals().chamferSyncPending === 0 && internals().edgeOpOpening === null;

  /** Whether the real lane's builder accepts what the session was frozen with plus its newest params. */
  function buildsOnRealLane(id: string): string | null {
    const draft = draftOf(id);
    const sent = updatesFor(id);
    try {
      buildPreviewOp({
        opId: "op",
        opType: draft.opType,
        inputs: draft.inputs,
        latestParams: { ...(sent[sent.length - 1]?.[1] ?? draft.params) },
      });
      return null;
    } catch (error) {
      return (error as Error).message;
    }
  }

  /** Everything a press may not change while a commit is materializing. */
  function armSnapshot(): Record<string, unknown> {
    const c = internals();
    const chip = toolChipStore.getState();
    return {
      edgeOp: c.fillet.edgeOp,
      radius: c.fillet.radius,
      distance2: c.fillet.distance2,
      angleDeg: c.fillet.angleDeg,
      draft: c.filletDraft,
      flipped: c.chamferFlipped,
      pairs: c.chamferPairs?.pairs ?? null,
      chip: [chip.edgeOp, chip.value, chip.distance2, chip.chamferAngleDeg, chip.validation.status],
    };
  }

  /**
   * A FIFO backend slower than local work: answer the OLDEST parked promotion only
   * once every continuation the previous answer unblocked has run (a microtask
   * drain models that latency; nothing here waits on a clock), until the lane idles.
   */
  async function answerPromotesInOrder(): Promise<void> {
    for (;;) {
      for (let i = 0; i < 500; i += 1) await Promise.resolve();
      const next = mock.parkedPromotes.shift();
      if (!next) break;
      next();
    }
    await until("idle lane", quiescent);
  }

  /** Release one held open and wait until its continuation installed or ended it. */
  async function releaseOpen(id: string): Promise<void> {
    const open = mock.opens.find((o) => o.sessionId === id);
    if (!open) throw new Error(`no beginPreview for ${id}`);
    open.release();
    await until(`${id} installed or ended`, () => installed() === id || endedCount(id, false) > 0);
  }

  /** Every session begun and not installed was ended with endPreview(false), once. */
  function expectOnlyNewestLive(newest: string): void {
    expect(installed()).toBe(newest);
    expect(internals().previewSessions).toHaveLength(1);
    expect(endedCount(newest, false)).toBe(0);
    for (const open of mock.opens) {
      if (open.sessionId === newest) continue;
      expect(endedCount(open.sessionId, false), `${open.sessionId} ended once`).toBe(1);
    }
  }

  async function armFillet(): Promise<void> {
    selectionStore.getState().set(EDGES);
    toolStore.getState().setTool("fillet");
    await until("fillet arm installed pv-1", () => installed() === "pv-1");
  }

  async function armAsymmetricChamfer(distance2: number): Promise<string> {
    await armFillet();
    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await until("chamfer reopen installed", () => installed() === "pv-2");
    toolChipStore.getState().onDistance2?.(distance2);
    await until("pairs reopen installed", () => installed() === "pv-3" && faceIdOf(draftOf("pv-3")) === "el_f_3");
    await until("range verdict", () => toolChipStore.getState().validation.status === "valid");
    return "pv-3";
  }

  it("(a) equal-leg flip + second leg, reverse resolution: the NEWEST draft is installed and committed once", async () => {
    build();
    await armFillet();
    lane.holdOpens = true;

    toolChipStore.getState().onEdgeOp?.("Chamfer"); // A = pv-2, frozen equal-leg
    toolChipStore.getState().onDistance2?.(2.5); // sync → B = pv-3, carries the pair
    await until("sync opened B", () => mock.opens.length === 3);
    expect(draftOf("pv-2").params.distance2).toBeUndefined();
    expect(draftOf("pv-3").params.distance2).toBe(2.5);

    await releaseOpen("pv-3");
    await releaseOpen("pv-2");
    expectOnlyNewestLive("pv-3");

    toolChipStore.getState().onConfirm?.();
    await until("commit settled", () => attemptPhase() === "completed" || attemptPhase() === "failed");

    expect(commits()).toEqual(["pv-3"]);
    expect(mock.client.applyOperation).not.toHaveBeenCalled();
    const committed = draftOf("pv-3");
    expect(committed.params.distance2).toBe(2.5);
    expect(committed.params.referenceFaces).toEqual([{ edgeId: "el_e0", faceId: "el_f_3" }]);
    expect(committed.inputs).toHaveLength(2);
  });

  it("(b) type flips with pairs then [Flip reference], reverse resolution: the newest reference wins", async () => {
    build();
    await armAsymmetricChamfer(1);
    lane.holdOpens = true;
    const before = mock.opens.length;

    toolChipStore.getState().onEdgeOp?.("Fillet"); // re-authors (drops) the pairs, then reopens
    toolChipStore.getState().onEdgeOp?.("Chamfer"); // re-authors f:3, then reopens
    await until("chamfer reopen begun", () =>
      mock.opens.slice(before).some((o) => o.draft.opType === "Chamfer" && faceIdOf(o.draft) === "el_f_3"),
    );
    toolChipStore.getState().onChamferFlip?.(); // sync → f:5, reopens
    await until("flip reopen begun", () => mock.opens.some((o) => faceIdOf(o.draft) === "el_f_5"));

    const held = mock.opens.slice(before).map((o) => o.sessionId);
    for (const id of [...held].reverse()) await releaseOpen(id);

    const newest = held[held.length - 1];
    expect(faceIdOf(draftOf(newest))).toBe("el_f_5");
    expect(draftOf(newest).opType).toBe("Chamfer");
    expectOnlyNewestLive(newest);
  });

  it("(c) a failed-commit re-arm racing a [Flip reference]: the flip's session wins, one live", async () => {
    build();
    const armed = await armAsymmetricChamfer(1);
    lane.refuseCommit = true;
    lane.holdOpens = true;

    toolChipStore.getState().onConfirm?.();
    await until("commit failed", () => attemptPhase() === "failed");
    await until("re-arm begun", () => mock.opens.length === 4);
    lane.refuseCommit = false;
    expect(endedCount(armed, false)).toBe(1); // the refused session was released

    toolChipStore.getState().onChamferFlip?.();
    await until("flip reopen begun", () => mock.opens.length === 5);
    expect(faceIdOf(draftOf("pv-4"))).toBe("el_f_3");
    expect(faceIdOf(draftOf("pv-5"))).toBe("el_f_5");

    await releaseOpen("pv-5");
    await releaseOpen("pv-4");
    expectOnlyNewestLive("pv-5");
  });

  it("(d) equal-leg flip + immediate ✓ (twice): ONE commit, from the reopen, with the typed size", async () => {
    build();
    await armFillet();
    lane.holdOpens = true;

    toolChipStore.getState().onEdgeOp?.("Chamfer"); // A = pv-2, held
    // The chip's Enter applies the value and confirms in the same turn; a second ✓
    // lands while the first is still waiting.
    toolChipStore.getState().onValue?.(3);
    toolChipStore.getState().onConfirm?.();
    toolChipStore.getState().onConfirm?.();
    await until("range verdict", () => toolChipStore.getState().validation.status === "valid");

    await releaseOpen("pv-2");
    await until("commit settled", () => attemptPhase() === "completed" || attemptPhase() === "failed");

    expect(viewportStore.getState().statusHint?.message ?? "").not.toMatch(/preview is unavailable/);
    expect(commits()).toEqual(["pv-2"]);
    const pv2Updates = updatesFor("pv-2");
    const finalUpdate = pv2Updates[pv2Updates.length - 1];
    expect(finalUpdate?.[1].radius).toBe(3);
    expect(finalUpdate?.[1].mode).toBe("Chamfer");
    expect(mock.opens).toHaveLength(2); // no failure re-arm opened another session
    expect(mock.client.applyOperation).not.toHaveBeenCalled();
    await until("tool reset", () => toolStore.getState().modelTool === "select");
  });

  it("(d2) asymmetric type flip + immediate ✓: waits for the pair resolution AND its reopen", async () => {
    build();
    await armAsymmetricChamfer(1);
    toolChipStore.getState().onEdgeOp?.("Fillet"); // drops the pairs, reopens a Fillet
    await until("fillet reopen installed", () => installed() === "pv-4");
    // A promotion the arm has not cached (a real round trip), held open across the ✓.
    internals().chamferFaceRefs.clear();
    lane.holdPromotes = true;

    toolChipStore.getState().onEdgeOp?.("Chamfer"); // distance2 retained → pairs re-authored first
    toolChipStore.getState().onConfirm?.();
    await until("promotion parked", () => mock.parkedPromotes.length === 1);
    await until("range verdict", () => toolChipStore.getState().validation.status === "valid");

    mock.releasePromotes();
    await until("commit settled", () => attemptPhase() === "completed" || attemptPhase() === "failed");

    expect(viewportStore.getState().statusHint?.message ?? "").not.toMatch(/preview is unavailable/);
    expect(commits()).toEqual(["pv-5"]);
    expect(draftOf("pv-5").opType).toBe("Chamfer");
    expect(draftOf("pv-5").params.referenceFaces).toEqual([{ edgeId: "el_e0", faceId: "el_f_3" }]);
    expect(draftOf("pv-5").inputs).toHaveLength(2);
  });

  it("(e) a tool switch under a waiting ✓: the confirm says so, the late open is ended, nothing installed", async () => {
    build();
    await armFillet();
    lane.holdOpens = true;

    toolChipStore.getState().onEdgeOp?.("Chamfer"); // pv-2, held
    await until("range verdict", () => toolChipStore.getState().validation.status === "valid");
    toolChipStore.getState().onConfirm?.();
    toolStore.getState().setTool("select");

    await releaseOpen("pv-2");
    await until(
      "confirm resolved",
      () => /not applied/.test(viewportStore.getState().statusHint?.message ?? "") || mock.opens.length > 2,
    );

    expect(mock.opens).toHaveLength(2); // the ✓ neither committed nor re-armed a preview
    expect(viewportStore.getState().statusHint?.message).toMatch(/Chamfer not applied/);
    expect(endedCount("pv-2", false)).toBe(1);
    expect(installed()).toBeNull();
    expect(updatesFor("pv-2")).toHaveLength(0);
    expect(commits()).toEqual([]);
    expect(mock.client.applyOperation).not.toHaveBeenCalled();
  });

  it("(e2) a type flip while the ✓ waits on the range verdict: the drop is logged, the new arm keeps its line", async () => {
    build();
    __resetLogForTests();
    await armFillet();
    lane.holdRanges = true;
    toolChipStore.getState().onEdgeOp?.("Chamfer"); // its range verdict is held
    await until("chamfer reopen installed", () => installed() === "pv-2");
    toolChipStore.getState().onConfirm?.(); // waits on that verdict

    lane.holdRanges = false;
    lane.holdOpens = true; // the new arm's reopen stays out, so nothing else rewrites the line
    toolChipStore.getState().onEdgeOp?.("Fillet"); // a new arm generation
    await until("confirm resolved", () => warned(/not applied/) || attemptPhase() !== undefined);
    expect(hint()).toMatch(/^Fillet 1 edge/);

    mock.releaseRanges();
    await releaseOpen("pv-3");
    expect(hint()).not.toMatch(/not applied/);
    expect(attemptPhase()).toBeUndefined();
    expect(commits()).toEqual([]);
    expect(installed()).toBe("pv-3");
  });

  it("(f1) a superseded open landing while committing is ended; the committing session and its barrier survive", async () => {
    build();
    await armFillet();
    lane.holdOpens = true;
    toolChipStore.getState().onEdgeOp?.("Chamfer"); // A = pv-2, held
    toolChipStore.getState().onDistance2?.(2.5); // B = pv-3, held
    await until("sync opened B", () => mock.opens.length === 3);
    await releaseOpen("pv-3");
    await until("range verdict", () => toolChipStore.getState().validation.status === "valid");

    lane.holdAnswersFor.add("pv-3");
    toolChipStore.getState().onConfirm?.();
    await until("committing on pv-3", () => toolStore.getState().phase === "committing" && updatesFor("pv-3").length > 1);
    const updatesBefore = updatesFor("pv-3").length;

    await releaseOpen("pv-2");
    expect(endedCount("pv-2", false)).toBe(1);
    expect(updatesFor("pv-2")).toHaveLength(0);
    expect(endedCount("pv-3", false)).toBe(0);
    expect(installed()).toBe("pv-3");
    expect(updatesFor("pv-3")).toHaveLength(updatesBefore);
    expect(attemptPhase()).toBe("applying"); // the barrier is still waiting

    mock.releaseAnswers("pv-3");
    await until("commit settled", () => attemptPhase() === "completed" || attemptPhase() === "failed");
    expect(commits()).toEqual(["pv-3"]);
    expect(attemptPhase()).toBe("completed");
  });

  it("(f2) [Flip reference] and a type segment while committing end nothing and clear no barrier", async () => {
    build();
    const armed = await armAsymmetricChamfer(1);
    lane.holdAnswersFor.add(armed);
    toolChipStore.getState().onConfirm?.();
    await until("committing", () => toolStore.getState().phase === "committing" && updatesFor(armed).length > 1);
    const opensBefore = mock.opens.length;

    toolChipStore.getState().onChamferFlip?.();
    await internals().chamferSync;
    toolChipStore.getState().onEdgeOp?.("Fillet");

    expect(endedCount(armed, false)).toBe(0);
    expect(installed()).toBe(armed);
    expect(mock.opens).toHaveLength(opensBefore);
    expect(attemptPhase()).toBe("applying");

    mock.releaseAnswers(armed);
    await until("commit settled", () => attemptPhase() === "completed" || attemptPhase() === "failed");
    expect(commits()).toEqual([armed]);
    expect(faceIdOf(draftOf(armed))).toBe("el_f_3");
  });

  it("(f3) an open that is the NEWEST but lands while committing is ended, never installed", async () => {
    // No public path starts an open while committing (the entry guards above), so
    // the publication gate is driven directly: it must hold for any future caller.
    build();
    await armFillet();
    lane.holdAnswersFor.add("pv-1");
    toolChipStore.getState().onConfirm?.();
    await until("committing", () => toolStore.getState().phase === "committing" && updatesFor("pv-1").length > 1);

    lane.holdOpens = true;
    const late = internals().openEdgeOpPreview(internals().armGen);
    await releaseOpen("pv-2");
    await late;

    expect(endedCount("pv-2", false)).toBe(1);
    expect(updatesFor("pv-2")).toHaveLength(0);
    expect(installed()).toBe("pv-1");
    expect(endedCount("pv-1", false)).toBe(0);
    expect(attemptPhase()).toBe("applying");

    mock.releaseAnswers("pv-1");
    await until("commit settled", () => attemptPhase() === "completed" || attemptPhase() === "failed");
    expect(commits()).toEqual(["pv-1"]);
  });

  it("(g) a newer open landing over an installed session ENDS it — never a bare overwrite", async () => {
    build();
    await armFillet();
    await internals().openEdgeOpPreview(internals().armGen);

    expectOnlyNewestLive("pv-2");
  });

  // ── H4b: review batch R ──────────────────────────────────────────────────────

  it("(r1) while committing, segment / second leg / angle / flip / size change nothing and nothing more is sent", async () => {
    build();
    const armed = await armAsymmetricChamfer(1);
    internals().throttle.setTrailingMs(0); // no pacing floor: a queued send would go out on the next answer
    lane.holdAnswersFor.add(armed);
    const before = armSnapshot();

    toolChipStore.getState().onConfirm?.();
    await until("committing", () => toolStore.getState().phase === "committing" && updatesFor(armed).length > 1);
    const sentAtCommit = updatesFor(armed).length;

    toolChipStore.getState().onChamferFlip?.();
    await internals().chamferSync;
    toolChipStore.getState().onDistance2?.(2);
    await internals().chamferSync;
    toolChipStore.getState().onChamferAngle?.(30);
    await internals().chamferSync;
    toolChipStore.getState().onEdgeOp?.("Fillet");
    toolChipStore.getState().onValue?.(4);
    expect(armSnapshot()).toEqual(before);

    mock.releaseAnswers(armed); // the kernel answers the commit's final epoch
    await until("commit settled", () => attemptPhase() === "completed" || attemptPhase() === "failed");

    const order = mock.client.updatePreview.mock.invocationCallOrder;
    const calls = mock.client.updatePreview.mock.calls as unknown as Array<[string, PreviewParams, number]>;
    const commitIndex = endCalls().findIndex(([id, commit]) => id === armed && commit);
    const commitOrder = mock.client.endPreview.mock.invocationCallOrder[commitIndex];
    const sentBeforeCommit = calls.filter(([id], i) => id === armed && order[i] < commitOrder);
    expect(sentBeforeCommit).toHaveLength(sentAtCommit);
    for (const [, params] of sentBeforeCommit) {
      expect(faceIdOf({ params } as PreviewDraft)).toBe("el_f_3");
      expect(params.distance2).toBe(1);
    }
    expect(buildsOnRealLane(armed)).toBeNull();
    expect(commits()).toEqual([armed]);
  });

  it("(r1b) a stalePreview refusal of the commit's final epoch sends no retry into the committing session", async () => {
    build();
    const armed = await armAsymmetricChamfer(1);
    internals().throttle.setTrailingMs(0); // a retry would go out at once
    lane.refuseCommit = true;
    lane.refusalKind = "stalePreview";
    lane.holdAnswersFor.add(armed);
    toolChipStore.getState().onConfirm?.();
    await until("committing", () => toolStore.getState().phase === "committing" && updatesFor(armed).length > 1);
    const sentAtCommit = updatesFor(armed).length;

    mock.releaseAnswers(armed);
    await until("commit failed", () => attemptPhase() === "failed");
    expect(updatesFor(armed)).toHaveLength(sentAtCommit);
  });

  it("(r3b) a type flip's lookup whose inputs change mid-flight never begins a session its params disagree with", async () => {
    build();
    await armAsymmetricChamfer(1);
    toolChipStore.getState().onEdgeOp?.("Fillet");
    await until("fillet reopen installed", () => installed() === "pv-4");
    internals().chamferFaceRefs.clear(); // real promotion round trips from here
    lane.holdPromotes = true;
    toolChipStore.getState().onEdgeOp?.("Chamfer"); // second leg retained: the lookup's promotion parks
    await until("lookup promotion parked", () => mock.parkedPromotes.length === 1);
    toolChipStore.getState().onDistance2?.(null); // …and the asymmetry goes while it is out

    await answerPromotesInOrder();

    for (const open of mock.opens) expect(buildsOnRealLane(open.sessionId), open.sessionId).toBeNull();
    const id = installed();
    if (!id) throw new Error("no session installed");
    expect(draftOf(id).opType).toBe("Chamfer");
    expect(draftOf(id).inputs).toHaveLength(1);
  });

  it("(r8) a FAILED commit hands back exactly the pre-commit arm, whatever was pressed while applying", async () => {
    build();
    await armAsymmetricChamfer(1);
    const before = armSnapshot();
    lane.refuseCommit = true;
    lane.holdAnswersFor.add("pv-3");

    toolChipStore.getState().onConfirm?.();
    await until("committing", () => toolStore.getState().phase === "committing" && updatesFor("pv-3").length > 1);
    toolChipStore.getState().onEdgeOp?.("Fillet");
    toolChipStore.getState().onDistance2?.(2);
    await internals().chamferSync;
    toolChipStore.getState().onChamferAngle?.(30);
    await internals().chamferSync;
    toolChipStore.getState().onChamferFlip?.();
    await internals().chamferSync;
    toolChipStore.getState().onValue?.(0.05); // a refused size would leave a draft and a verdict behind

    mock.releaseAnswers("pv-3");
    await until("commit failed and re-armed", () => attemptPhase() === "failed" && installed() === "pv-4");
    lane.refuseCommit = false;

    expect(armSnapshot()).toEqual(before);
    expect(toolStore.getState().phase).toBe("armed");
    expect(draftOf("pv-4").opType).toBe("Chamfer");
    expect(faceIdOf(draftOf("pv-4"))).toBe("el_f_3");
    expect(draftOf("pv-4").params.distance2).toBe(1);
  });

  it("(r2) a size refused while the ✓ waits on a reopen is refused with its message, never committed", async () => {
    build();
    await armFillet();
    lane.holdOpens = true;
    toolChipStore.getState().onEdgeOp?.("Chamfer"); // pv-2 held
    await until("range verdict", () => toolChipStore.getState().validation.status === "valid");
    toolChipStore.getState().onValue?.(3);
    toolChipStore.getState().onConfirm?.(); // waits on pv-2
    toolChipStore.getState().onValue?.(0.00001); // below the authoring minimum

    await releaseOpen("pv-2");
    await until("confirm resolved", () => /Cannot confirm/.test(hint()) || attemptPhase() !== undefined);

    expect(hint()).toMatch(/^Cannot confirm Chamfer: Must be at least/);
    expect(attemptPhase()).toBeUndefined();
    expect(commits()).toEqual([]);
  });

  it("(r2b) a size refused while the ✓ waits on the range verdict is refused with its message", async () => {
    build();
    await armFillet();
    lane.holdRanges = true;
    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await until("chamfer reopen installed", () => installed() === "pv-2");
    toolChipStore.getState().onConfirm?.(); // waits on the held verdict
    toolChipStore.getState().onValue?.(0.00001);
    mock.releaseRanges();
    await until("confirm resolved", () => /Cannot confirm/.test(hint()) || attemptPhase() !== undefined);

    expect(hint()).toMatch(/^Cannot confirm Chamfer: Must be at least/);
    expect(commits()).toEqual([]);
  });

  it("(r3) promotion round trips racing type flips and a cleared second leg (FIFO backend) never install a mismatched session", async () => {
    build();
    await armFillet();
    lane.holdPromotes = true;
    toolChipStore.getState().onEdgeOp?.("Chamfer"); // equal-leg: no lookup
    await until("chamfer reopen installed", () => installed() === "pv-2");
    toolChipStore.getState().onDistance2?.(2); // sync: its promotion parks
    await until("sync promotion parked", () => mock.parkedPromotes.length === 1);
    toolChipStore.getState().onEdgeOp?.("Fillet");
    toolChipStore.getState().onEdgeOp?.("Chamfer"); // second leg retained → a lookup of its own
    toolChipStore.getState().onDistance2?.(null); // …which the user then clears

    await answerPromotesInOrder();

    const id = installed();
    if (!id) throw new Error("no session installed");
    expect(internals().fillet.distance2).toBeNull();
    expect(internals().chamferPairs).toBeNull();
    expect(draftOf(id).opType).toBe("Chamfer");
    expect(draftOf(id).params.referenceFaces).toBeUndefined();
    expect(draftOf(id).inputs).toHaveLength(1);
    expect(buildsOnRealLane(id)).toBeNull();
  });

  it("(r4) a ✓ waiting on an arm the user left lets go at once and never posts over the new arm", async () => {
    build();
    __resetLogForTests();
    await armFillet();
    lane.holdOpens = true;
    toolChipStore.getState().onEdgeOp?.("Chamfer"); // pv-2 held
    await until("range verdict", () => toolChipStore.getState().validation.status === "valid");
    toolChipStore.getState().onConfirm?.(); // waits on pv-2
    toolStore.getState().setTool("select");
    selectionStore.getState().set(EDGES);
    toolStore.getState().setTool("fillet"); // a new arm, whose open (pv-3) is held too
    await until("new arm's open begun", () => mock.opens.length === 3);

    await until("the ✓ let go of the old arm", () => warned(/not applied/));
    await releaseOpen("pv-2");
    await releaseOpen("pv-3");
    await until("idle", quiescent);

    expect(hint()).toMatch(/^Fillet 1 edge/);
    expect(commits()).toEqual([]);
    expect(installed()).toBe("pv-3");
  });

  it("(r5) a waiting ✓ is visible: closing the lane reads pending, the ✓ says it is waiting", async () => {
    build();
    await armFillet();
    await until("arm preview answered", () => lifecycle() === "valid");
    lane.holdOpens = true;

    toolChipStore.getState().onEdgeOp?.("Chamfer"); // closes pv-1, pv-2 held
    expect(lifecycle()).toBe("pending");
    await until("range verdict", () => toolChipStore.getState().validation.status === "valid");
    toolChipStore.getState().onConfirm?.();
    expect(hint()).toBe("Waiting for preview…");
    expect(lifecycle()).toBe("pending");
    toolChipStore.getState().onConfirm?.(); // a second ✓ joins the same wait
    expect(hint()).toBe("Waiting for preview…");

    await releaseOpen("pv-2");
    await until("commit settled", () => attemptPhase() === "completed" || attemptPhase() === "failed");
    expect(commits()).toEqual(["pv-2"]);
  });

  it("(r5b) a same-arm reopen after several answered sends reads pending, then valid again", async () => {
    build();
    await armFillet();
    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await until("chamfer reopen installed", () => installed() === "pv-2");
    internals().throttle.setTrailingMs(0);
    for (const size of [2, 3, 4]) {
      toolChipStore.getState().onValue?.(size);
      await until(`size ${size} answered`, () => internals().throttle.inFlight === null && lifecycle() === "valid");
    }
    lane.holdOpens = true;

    toolChipStore.getState().onDistance2?.(1); // the pair appears: same arm, lane closed and reopened
    await until("pairs reopen begun", () => mock.opens.length === 3);
    expect(lifecycle()).toBe("pending");

    await releaseOpen("pv-3");
    await until("reopen answered", () => internals().throttle.inFlight === null && updatesFor("pv-3").length > 0);
    expect(lifecycle()).toBe("valid");
  });

  it("(r6) an aborted history re-edit does not take the live arm or its waiting ✓", async () => {
    build();
    documentStore.setState({
      features: [{ id: "feat-b", kind: "boolean", opType: "Boolean", label: "Boolean", valueText: "", status: "ok" }],
    });
    await armFillet();
    lane.holdOpens = true;
    toolChipStore.getState().onEdgeOp?.("Chamfer"); // pv-2 held
    await until("range verdict", () => toolChipStore.getState().validation.status === "valid");
    toolChipStore.getState().onConfirm?.();

    await controller.editBooleanFeature("feat-b"); // stored params unusable → refused
    expect(hint()).toMatch(/Cannot re-edit boolean/);

    await releaseOpen("pv-2");
    await until("confirm resolved", () => attemptPhase() !== undefined || /not applied/.test(hint()));
    await until("commit settled", () => attemptPhase() === "completed" || attemptPhase() === "failed");
    expect(commits()).toEqual(["pv-2"]);
  });

  it("(r6b) every re-edit that refuses its stored params leaves the arm generation alone", async () => {
    build();
    const feature = (id: string, kind: string, opType: string) => ({ id, kind, opType, label: opType, valueText: "", status: "ok" });
    documentStore.setState({
      features: [
        feature("f-hole", "boolean", "Hole"),
        feature("f-gear", "boolean", "Gear"),
        feature("f-lin", "boolean", "LinearPattern"),
        feature("f-circ", "boolean", "CircularPattern"),
        feature("f-mir", "boolean", "MirrorBody"),
        feature("f-move", "boolean", "TransformBody"),
        feature("f-bool", "boolean", "Boolean"),
        feature("f-ex", "extrude", "Extrude"),
        feature("f-rev", "revolve", "Revolve"),
      ] as never,
    });
    const c = controller as unknown as Record<string, (id: string) => Promise<void> | void>;
    const cases: Array<[string, string, RegExp]> = [
      ["editHoleFeature", "f-hole", /Cannot re-edit this hole/],
      ["editGearFeature", "f-gear", /Cannot re-edit this gear/],
      ["editLinearPatternFeature", "f-lin", /Cannot re-edit linear pattern/],
      ["editCircularPatternFeature", "f-circ", /Cannot re-edit circular pattern/],
      ["editMirrorFeature", "f-mir", /Cannot re-edit mirror/],
      ["editTransformFeature", "f-move", /Cannot re-edit move/],
      ["editBooleanFeature", "f-bool", /Cannot re-edit boolean/],
      ["editExtrudeFeature", "f-ex", /Extrude profile is missing/],
      ["editRevolveFeature", "f-rev", /Revolve profile is missing/],
    ];
    for (const [method, id, refusal] of cases) {
      viewportStore.getState().setStatusHint(null);
      const armGen = internals().armGen;
      await c[method].call(controller, id);
      await until(`${method} refused`, () => refusal.test(hint()));
      expect(internals().armGen, method).toBe(armGen);
    }
  });

  it("(m12) only the NEWEST open's refusal is reported; a superseded one's is not", async () => {
    build();
    __resetLogForTests();
    await armFillet();
    lane.holdOpens = true;
    toolChipStore.getState().onEdgeOp?.("Chamfer"); // A = pv-2, held
    toolChipStore.getState().onDistance2?.(1); // B = pv-3, held
    await until("sync opened B", () => mock.opens.length === 3);

    mock.opens[1].fail(new Error("stale lane refusal"));
    await releaseOpen("pv-3");
    expect(warned(/preview session failed: stale lane refusal/)).toBe(false);

    toolChipStore.getState().onChamferFlip?.(); // C = pv-4, the newest
    await until("flip opened C", () => mock.opens.length === 4);
    mock.opens[3].fail(new Error("newest lane refusal"));
    await until("newest refusal reported", () => warned(/preview session failed: newest lane refusal/));
  });

  it("(m13) a type flip whose lookup is overtaken by a later reopen never begins a stale session", async () => {
    build();
    await armAsymmetricChamfer(1);
    internals().chamferFaceRefs.clear(); // real promotion round trips from here
    lane.holdPromotes = true;
    toolChipStore.getState().onChamferFlip?.(); // S1 runs: its promotion parks
    await until("S1 promotion parked", () => mock.parkedPromotes.length === 1);
    toolChipStore.getState().onChamferFlip?.(); // S2 queued behind S1
    toolChipStore.getState().onEdgeOp?.("Fillet"); // the flip's lookup queues behind S2
    const opensBefore = mock.opens.length;

    await answerPromotesInOrder();

    const begun = mock.opens.slice(opensBefore);
    expect(begun).toHaveLength(1); // S2 reopened for the Fillet; the flip's own open never began
    expect(begun[0].draft.opType).toBe("Fillet");
    expect(installed()).toBe(begun[0].sessionId);
  });
});
