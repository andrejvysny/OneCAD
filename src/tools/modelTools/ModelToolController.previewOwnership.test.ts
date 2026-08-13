/*
 * WP0.7 — MULTI-SESSION EXACT PREVIEW OWNERSHIP (jsdom).
 *
 * Arming a multi-region extrude opens ONE kernel preview session per region. They
 * share a throttle and an epoch, but nothing else: each session owns its own
 * candidate bodies, its own visibility claim over the bodies it replaces, and its
 * own failure state.
 *
 * The spec (`phase-0-safety-and-truth.md` WP0.7) asks for exactly this file:
 * "deliver two region responses in both orders, then fail and recover one secondary
 * session. All surviving candidates remain visible and failure state is tracked per
 * session/epoch rather than globally."
 *
 * Why it needs its own harness: every existing spec that can feed `onPreviewResult`
 * is single-region, and the multi-region specs stub the callback away — one of them
 * even hands every `beginPreview` the same constant sessionId, which makes two
 * sessions indistinguishable. Per-session behaviour cannot be observed through
 * either. This harness mints `pv-1`, `pv-2`, … and keeps the callback.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  PreviewDraft,
  PreviewParams,
  PreviewResult,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};
const R1: SketchRegion = {
  regionId: "r1",
  outerLoop: [],
  holes: [],
  previewTriangles: {
    positions: [100, 100, 140, 100, 140, 140, 100, 140],
    indices: [0, 1, 2, 0, 2, 3],
  },
};

const ok = (bodyId: string): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId, meshKey: `${bodyId}#0` }],
  removedBodies: [],
});

const session: SketchSession = {
  sketchId: "sk",
  plane: PLANE,
  entities: [],
  constraints: [],
  dof: 0,
  status: "FullyConstrained",
};

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    setRegionSelected: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    planePixelWorld: vi.fn(() => 1),
    probePick: vi.fn(() => null),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => ({ origin: [0, 0, 100] as const, dir: [0, 0, -1] as const })),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => true),
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
  };
}

function makeClientMock(capture: (cb: (r: PreviewResult) => void) => void) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capture(cb);
      return () => {};
    }),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0, R1] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0, R1] })),
    getSketch: vi.fn(() => Promise.resolve(session)),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn((_id: string, _params: PreviewParams, _epoch: number) => {}),
    endPreview: vi.fn((_id: string, _commit: boolean) => Promise.resolve(ok("b1"))),
    applyOperation: vi.fn(() => Promise.resolve(ok("adhoc"))),
    applyEditCommand: vi.fn(() => Promise.resolve(ok("edit"))),
    undo: vi.fn(() => Promise.resolve(ok("undone"))),
    getOperationParams: vi.fn((): Promise<Record<string, unknown>> => Promise.resolve({})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("multi-session exact preview ownership (WP0.7)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let previewCb: ((r: PreviewResult) => void) | null;

  function build(): void {
    engineMock = makeEngineMock();
    previewCb = null;
    clientMock = makeClientMock((cb) => {
      previewCb = cb;
    });
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  }

  /** Arm extrude on BOTH regions ⇒ two sessions, `pv-1` (primary) and `pv-2`. */
  async function armTwoRegions(): Promise<void> {
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
      { kind: "sketchRegion", id: "r1-ref", sketchId: "sk", regionId: "r1" },
    ]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);
  }

  /** The epoch the controller last pushed (both sessions share it). */
  function epoch(): number {
    const calls = clientMock.updatePreview.mock.calls;
    return calls[calls.length - 1][2] as number;
  }

  function candidate(sessionId: string, bodyId: string, replaced: string[]): PreviewResult {
    return {
      sessionId,
      epoch: epoch(),
      bodyId: "preview",
      bodies: [{ bodyId, mesh: makeBoxMesh() }],
      replacedBodyIds: replaced,
    };
  }

  function failure(sessionId: string, message: string): PreviewResult {
    return {
      sessionId,
      epoch: epoch(),
      bodyId: "preview",
      error: { kind: "opFailed", message, structural: true },
    };
  }

  /** The newest visibility claim the engine was handed, or `null` if never called. */
  function replacedClaim(): string[] | null {
    const calls = engineMock.setPreviewReplacedBodyIds.mock.calls;
    return calls.length ? (calls[calls.length - 1][0] as string[]) : null;
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    selectionStore.getState().set([]);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  it("keeps BOTH candidates when the primary answers first", async () => {
    build();
    await armTwoRegions();

    previewCb?.(candidate("pv-1", "cand-a", ["body-a"]));
    previewCb?.(candidate("pv-2", "cand-b", ["body-b"]));

    expect(engineMock.setPreviewBody).toHaveBeenCalledTimes(2);
    // The claim is the UNION of live sessions, not the newest answer alone —
    // recomputing it from one session would un-hide the other's replaced body.
    expect(replacedClaim()?.slice().sort()).toEqual(["body-a", "body-b"]);
  });

  it("keeps BOTH candidates when the SECONDARY answers first", async () => {
    build();
    await armTwoRegions();

    previewCb?.(candidate("pv-2", "cand-b", ["body-b"]));
    previewCb?.(candidate("pv-1", "cand-a", ["body-a"]));

    expect(engineMock.setPreviewBody).toHaveBeenCalledTimes(2);
    expect(replacedClaim()?.slice().sort()).toEqual(["body-a", "body-b"]);
    // Arrival order changes nothing: the primary's answer must not clear the
    // secondary's candidate. Once results are flowing, every clear is SCOPED to one
    // session's body ids — a bare `clearPreviewBody()` here wipes the other region.
    const clearsAfterResults = engineMock.clearPreviewBody.mock.calls.slice(-2);
    for (const call of clearsAfterResults) expect(call[0]).toBeInstanceOf(Array);
  });

  it("a secondary failure clears ONLY its own candidate and claim", async () => {
    build();
    await armTwoRegions();
    previewCb?.(candidate("pv-1", "cand-a", ["body-a"]));
    previewCb?.(candidate("pv-2", "cand-b", ["body-b"]));

    previewCb?.(failure("pv-2", "self-intersecting geometry"));

    // The surviving session keeps its claim; the failed one drops out of the union.
    expect(replacedClaim()).toEqual(["body-a"]);
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });

  it("a RECOVERED secondary unblocks the commit — failure state is per session", async () => {
    build();
    await armTwoRegions();
    previewCb?.(candidate("pv-1", "cand-a", ["body-a"]));
    previewCb?.(candidate("pv-2", "cand-b", ["body-b"]));
    previewCb?.(failure("pv-2", "self-intersecting geometry"));

    // The user nudges the drag; the secondary now answers with a good candidate.
    previewCb?.(candidate("pv-2", "cand-b2", ["body-b"]));

    expect(replacedClaim()?.slice().sort()).toEqual(["body-a", "body-b"]);

    // The commit must go through. Tracked globally, the secondary's stale failure
    // survives its own recovery and wedges Enter forever — with an error hint on
    // screen contradicting a preview that visibly works again.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    // The multi-region commit is sequential — one endPreview(true) per region.
    for (let i = 0; i < 8; i++) await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-2", true);
  });

  it("a still-failing session keeps blocking the commit", async () => {
    build();
    await armTwoRegions();
    previewCb?.(candidate("pv-1", "cand-a", ["body-a"]));
    previewCb?.(failure("pv-2", "self-intersecting geometry"));

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();
    // The negative control for the test above: recovery is what unblocks Enter,
    // not the mere passage of a second result.
    expect(clientMock.endPreview).not.toHaveBeenCalledWith("pv-1", true);
    expect(viewportStore.getState().statusHint?.message).toContain("self-intersecting geometry");
  });
});
