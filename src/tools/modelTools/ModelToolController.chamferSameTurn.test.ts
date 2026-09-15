/*
 * ModelToolController — the chamfer ✓ that arrives in the SAME TURN as the edit
 * that made the chamfer asymmetric, while the arm's own work is still in flight.
 *
 * The chip's numeric fields apply their value and then confirm on one Enter, so
 * `onChamferAngle(30)` / `onDistance2(2.5)` and `onConfirm()` run in one
 * synchronous turn, on an arm that is still settling. TWO in-flight things can
 * then swallow the ✓, and both are pinned here:
 *
 *  1. THE MEASURED RANGE (SCHEMA §7.6). A deliberate arm or type flip issues
 *     `AnalyzeEdgeOpRange` and parks the chip on `validation.status === "pending"`
 *     until it answers. That is "not measured yet", NOT "refused" — a ✓ that lands
 *     in the window must wait for the verdict. Dropping it is invisible to the
 *     user and purely timing-dependent, which is what `e2e/chamfer-angle.spec.ts`
 *     and `e2e/filletChamfer.spec.ts` see as a missing feature row on a loaded
 *     machine and never see when the spec runs alone.
 *  2. THE REFERENCE-FACE SYNC (SCHEMA §7.3). The edit starts a promotion round
 *     trip plus, because `beginPreview` FREEZES `inputs[]`, a session close and
 *     REOPEN. The ✓ must materialize the session that sync built.
 *
 * `chamferReferenceFace.test.ts` pins (2) when every backend promise settles in a
 * couple of microtasks. This file holds them open on purpose so the confirm has to
 * survive both at once.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  ElementInfo,
  PreviewDraft,
  PreviewResult,
  PromotePick,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { __resetLogForTests } from "@/debug/log";

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

/**
 * A parking lot for backend promises. While `armed`, every wrapped call hands back
 * a promise that only settles when the test releases it — the widened window a
 * real round trip has and a `Promise.resolve()` mock does not.
 */
class Gate {
  armed = false;
  private queue: Array<() => void> = [];

  wrap<T>(value: T): Promise<T> {
    if (!this.armed) return Promise.resolve(value);
    return new Promise<T>((resolve) => {
      this.queue.push(() => resolve(value));
    });
  }

  /** Settle everything parked so far, in call order. */
  release(): void {
    const parked = this.queue;
    this.queue = [];
    for (const settle of parked) settle();
  }
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

function makeClientMock(gate: Gate, capturePreview: (cb: (r: PreviewResult) => void) => void) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capturePreview(cb);
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
    analyzeEdgeOpRange: vi.fn(() =>
      Promise.resolve({
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
      }),
    ),
    promoteSelection: vi.fn((bodyId: string, picks: PromotePick[]) =>
      gate.wrap(
        picks.map((p) => ({
          topoKey: p.topoKey,
          elementId: `el_${p.topoKey.replace(":", "_")}`,
          kind: p.topoKey.startsWith("e:") ? "edge" : "face",
          bodyId,
        })),
      ),
    ),
    elementInfo: vi.fn((bodyId: string, elementId: string, topoKey?: string) =>
      gate.wrap({
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
    beginPreview: vi.fn((_d: PreviewDraft) => {
      seq += 1;
      return gate.wrap({ sessionId: `pv-${seq}`, previewBodyId: `pb-${seq}` });
    }),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() =>
      Promise.resolve({
        radius: { value: 4 },
        edgeIds: ["el_e0"],
        edges: [{ primary: { bodyId: "body1", elementId: "el_e0", kind: "edge" } }],
      }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("ModelToolController — a chamfer ✓ in the same turn as the edit that armed it", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let previewCb: ((r: PreviewResult) => void) | null;
  let gate: Gate;

  function build(): void {
    engineMock = makeEngineMock();
    previewCb = null;
    gate = new Gate();
    clientMock = makeClientMock(gate, (cb) => {
      previewCb = cb;
    });
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
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

  /** Arm the unified edge tool on the box edge and flip it to Chamfer. */
  async function armChamfer(): Promise<void> {
    selectionStore.getState().set(EDGES);
    toolStore.getState().setTool("fillet");
    await flush();
    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();
    await flush();
  }

  /**
   * The draft of the session the ✓ actually MATERIALIZED — `beginPreview` freezes
   * `inputs[]`, so the committed op is this draft and not whatever the chip last
   * displayed. Asserting on the last `beginPreview` instead would pass even if the
   * ✓ had consumed a stale session opened before the edit.
   */
  function committedDraft(): PreviewDraft {
    const ended = (clientMock.endPreview.mock.calls as unknown as Array<[string, boolean]>).find(
      ([, commit]) => commit === true,
    );
    if (!ended) throw new Error("no session was committed");
    const index = (clientMock.beginPreview.mock.results as Array<{ value: unknown }>).findIndex(
      (_r, i) => `pv-${i + 1}` === ended[0],
    );
    if (index < 0) throw new Error(`committed session ${ended[0]} has no draft`);
    return clientMock.beginPreview.mock.calls[index][0];
  }

  /** Answer the newest `updatePreview` with a matching-epoch exact result. */
  function answerPreview(): void {
    const calls = clientMock.updatePreview.mock.calls as unknown as Array<
      [string, Record<string, unknown>, number]
    >;
    const last = calls[calls.length - 1];
    if (!last) return;
    previewCb?.({
      sessionId: last[0],
      epoch: last[2],
      bodyId: "preview",
      bodies: [],
      replacedBodyIds: ["body1"],
    });
  }

  /** Drain the parked backend promises until the confirm sequence has settled. */
  async function settle(): Promise<void> {
    for (let i = 0; i < 14; i += 1) {
      await flush();
      gate.release();
      answerPreview();
    }
  }

  it("an ANGLE typed and confirmed in one turn commits, range check still pending", async () => {
    build();
    await armChamfer();
    // From here every promotion and every `beginPreview` parks, so the sync the
    // chip edit starts is still in flight when the ✓ lands.
    gate.armed = true;
    // A deliberate type flip re-measures the range (a Fillet ceiling is not a
    // Chamfer ceiling), and the chip reads `pending` until that answer arrives.
    toolChipStore.getState().onEdgeOp?.("Fillet");
    toolChipStore.getState().onEdgeOp?.("Chamfer");

    toolChipStore.getState().onChamferAngle?.(30);
    toolChipStore.getState().onConfirm?.();
    await settle();

    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
    const draft = committedDraft();
    expect(draft.params.angleDeg).toBe(30);
    expect(draft.params.referenceFaces).toEqual([{ edgeId: "el_e0", faceId: "el_f_3" }]);
    expect(draft.inputs).toHaveLength(2);
  });

  it("a SECOND LEG typed and confirmed in one turn commits, same two races", async () => {
    build();
    await armChamfer();
    gate.armed = true;
    toolChipStore.getState().onEdgeOp?.("Fillet");
    toolChipStore.getState().onEdgeOp?.("Chamfer");

    toolChipStore.getState().onDistance2?.(2.5);
    toolChipStore.getState().onConfirm?.();
    await settle();

    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
    expect(committedDraft().params.distance2).toBe(2.5);
    expect(committedDraft().params.referenceFaces).toEqual([
      { edgeId: "el_e0", faceId: "el_f_3" },
    ]);
  });
});
