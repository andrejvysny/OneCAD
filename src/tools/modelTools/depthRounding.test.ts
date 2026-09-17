/*
 * Extrude depth-drag rounding (jsdom — 2026-09-14 UX review T7).
 *
 * Dragging the depth handle produced `52.09 mm` while sketch dimension rounding
 * is ON by default — two rounding policies in one app. The drag now uses the same
 * quantum a cursor-placed sketch dimension does (`dimQuantum` off the minor grid
 * step at the current camera distance, in the DISPLAY unit, converted back to
 * mm), gated on the SAME `snapTo.dimensionRound` preference. Typed values stay
 * exact; no new setting.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  PreviewDraft,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { settingsStore } from "@/stores/settingsStore";
import { toolChipStore } from "@/stores/toolChipStore";
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

/** The raw depth the fake ray projects onto the normal axis (the review's number). */
const RAW_DEPTH = 52.09;

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }],
  removedBodies: [],
});

function makeSession(): SketchSession {
  return { sketchId: "sk", plane: PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
}

/** `chooseGridStep(100).minor` is 5 mm ⇒ `dimQuantum` is 0.5 mm in a mm session. */
const CAMERA_DISTANCE = 100;

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
    setRegionSelected: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    planePixelWorld: vi.fn(() => 1),
    getCameraDistance: vi.fn(() => CAMERA_DISTANCE),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    // Parallel to the normal axis, so `axisDepthFromRay` falls back to projecting
    // the ray ORIGIN onto it — a deterministic raw depth of RAW_DEPTH.
    screenRay: vi.fn(() => ({ origin: [0, 0, RAW_DEPTH] as const, dir: [0, 0, -1] as const })),
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

function makeClientMock() {
  let seq = 0;
  return {
    onPreviewResult: vi.fn(() => () => {}),
    onDocumentChanged: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => null),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketch: vi.fn(() => Promise.resolve(makeSession())),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("extrude depth drag rounds like a sketch dimension (T7)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  async function armExtrude(): Promise<void> {
    engineMock = makeEngineMock();
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: makeClientMock() as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
    ]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();
  }

  /** One drag frame off the fake ray, with the grab basis zeroed. */
  function dragOnce(): void {
    controller.forceExtrudeGrab();
    container.dispatchEvent(
      new MouseEvent("pointermove", { clientX: 5, clientY: 5, buttons: 1, bubbles: true }),
    );
  }

  beforeEach(() => {
    resetStores(); // forces snapTo.dimensionRound OFF
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v1",
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  it("rounds a dragged depth onto the sketch quantum when the preference is on", async () => {
    settingsStore.getState().setSnap("dimensionRound", true);
    await armExtrude();
    dragOnce();
    expect(toolChipStore.getState().value).toBeCloseTo(52.0, 9);
  });

  it("leaves the raw dragged depth alone when the preference is off", async () => {
    await armExtrude();
    dragOnce();
    expect(toolChipStore.getState().value).toBeCloseTo(RAW_DEPTH, 9);
  });

  it("rounds in mm AFTER the unit conversion, so an inch session lands on round inches", async () => {
    settingsStore.getState().setSnap("dimensionRound", true);
    settingsStore.getState().setDisplayUnit("in");
    await armExtrude();
    dragOnce();
    // dimQuantum(5 mm, "in") = 0.01 in = 0.254 mm ⇒ 52.09 → 205 × 0.254.
    expect(toolChipStore.getState().value).toBeCloseTo(52.07, 6);
    expect(toolChipStore.getState().value).not.toBeCloseTo(52.0, 6);
  });

  it("keeps a TYPED depth exact — rounding is a drag-lane aid only", async () => {
    settingsStore.getState().setSnap("dimensionRound", true);
    await armExtrude();
    toolChipStore.getState().onValue?.(52.09);
    expect(toolChipStore.getState().value).toBe(52.09);
  });

  /*
   * Review §4.3, TODO.md SESSION 37 H3: absolute snapping used to run on the FIRST
   * move even with zero travel, so merely touching the arrow quantized a typed
   * value. Below the drag threshold the value is exactly the start.
   */
  it("a typed off-grid depth survives 20 grab/release cycles with sub-threshold jitter; a deliberate move snaps", async () => {
    settingsStore.getState().setSnap("dimensionRound", true);
    await armExtrude();
    engineMock.hitExtrudeHandle.mockReturnValue(true);
    // Perpendicular to +Z: 10 px of travel is 1 mm along the axis.
    (engineMock.screenRay as ReturnType<typeof vi.fn>).mockImplementation((_x: number, y: number) => ({
      origin: [0, 0, (100 - y) / 10] as const,
      dir: [1, 0, 0] as const,
    }));
    toolChipStore.getState().onValue?.(12.37);
    const ptr = (type: string, x: number, y: number): void => {
      container.dispatchEvent(
        new MouseEvent(type, { clientX: x, clientY: y, button: 0, buttons: type === "pointerup" ? 0 : 1, bubbles: true }),
      );
    };

    for (let i = 0; i < 20; i++) {
      ptr("pointerdown", 50, 100);
      ptr("pointermove", 52, 98);
      ptr("pointermove", 49, 103);
      ptr("pointerup", 49, 103);
    }
    expect(toolChipStore.getState().value).toBe(12.37);

    ptr("pointerdown", 50, 100);
    ptr("pointermove", 50, 90); // +1 mm, past the threshold
    expect(toolChipStore.getState().value).toBeCloseTo(13.5, 9); // 13.37 onto the 0.5 mm quantum
  });
});
