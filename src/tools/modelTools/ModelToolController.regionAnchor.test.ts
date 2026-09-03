/*
 * regionAnchor at pick time (SCHEMA §7.3 "Region anchor", kernel-hardening WP-B):
 * an Extrude authored from a picked region carries `profile.regionAnchor` in its
 * draft, and a re-edit of `distance` keeps the STORED anchor byte-identical
 * rather than recomputing it (SINGLE-mapper rule; `updateScalarParamsCommand`'s
 * shallow merge is what makes that hold — see `tauriCommandMap.test.ts` for the
 * wire-level half of this same proof).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickHit } from "@/viewport/engine/Picker";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  PreviewDraft,
  PreviewResult,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

// 20×20 square, fan-triangulated into two EQUAL-area triangles — regionAnchorOf
// picks the FIRST (0,1,2), centroid (40/3, 20/3).
const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};
const EXPECTED_ANCHOR: [number, number] = [40 / 3, 20 / 3];

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }],
  removedBodies: [],
});

function makeSession(): SketchSession {
  return { sketchId: "sk", plane: PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
}

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
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
    probePick: vi.fn<(clientX: number, clientY: number) => PickHit | null>(() => null),
  };
}

function makeClientMock() {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((_cb: (result: PreviewResult) => void) => () => {}),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketch: vi.fn(() => Promise.resolve(makeSession())),
    beginPreview: vi.fn((_d: PreviewDraft) => Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` })),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn((): Promise<Record<string, unknown>> =>
      Promise.resolve({
        profile: { sketchId: "sk", regionId: "r0" },
        distance: { value: 20 },
      }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("ModelToolController regionAnchor (SCHEMA §7.3 WP-B)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  function build(): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "region-ref", sketchId: "sk", regionId: "r0" },
    ]);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  it("a fresh pick derives regionAnchor from the region's largest-area fill triangle", async () => {
    build();
    toolStore.getState().setTool("extrude");
    await flush();

    expect(clientMock.beginPreview).toHaveBeenCalled();
    const draft = clientMock.beginPreview.mock.calls[0][0] as PreviewDraft;
    expect(draft.regionAnchor).toEqual(EXPECTED_ANCHOR);
  });

  it("a re-edit reads the PERSISTED anchor off the stored record, not the current fill", async () => {
    build();
    // The stored record's anchor deliberately differs from what `regionAnchorOf(R0)`
    // would compute NOW — proving the re-edit path never recomputes it.
    const storedAnchor: [number, number] = [1.5, -2.25];
    clientMock.getOperationParams.mockResolvedValue({
      profile: { sketchId: "sk", regionId: "r0", regionAnchor: storedAnchor },
      distance: { value: 20 },
    });
    documentStore.setState({
      features: [{ id: "feat-ex", kind: "extrude", label: "Extrude", valueText: "20 mm", status: "ok" }],
    });
    controller.editExtrudeFeature("feat-ex");
    await flush();

    const draft = clientMock.beginPreview.mock.calls[clientMock.beginPreview.mock.calls.length - 1][0] as PreviewDraft;
    expect(draft.regionAnchor).toEqual(storedAnchor);
    expect(draft.regionAnchor).not.toEqual(EXPECTED_ANCHOR);

    toolChipStore.getState().onValue?.(25);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();

    // The committed patch touches ONLY the changed scalars — `profile` (and its
    // regionAnchor) rides through the shallow merge byte-identical to storage.
    expect(clientMock.applyEditCommand).toHaveBeenCalledWith({
      cmd: "updateOperationParams",
      record: "feat-ex",
      op: {
        opType: "Extrude",
        params: {
          profile: { sketchId: "sk", regionId: "r0", regionAnchor: storedAnchor },
          distance: { value: 25 },
          draftAngleDeg: { value: 0 },
        },
      },
    });
  });
});
