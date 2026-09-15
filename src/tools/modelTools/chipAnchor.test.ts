/*
 * Tool-chip anchoring off the profile (jsdom — 2026-09-14 UX review T1).
 *
 * The chip used to sit ON the region centroid: with Extrude armed it floated
 * directly over the sketch region the user was being asked to click, a press it
 * absorbed was reported nowhere, and the count stayed `0 regions`. The anchor is
 * now pushed along the plane's in-plane +x past the profile's own half-extent,
 * and a press the chip absorbs while a region is still required says so once.
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
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/** One square region, (u,v) 0..20 — centroid (10,10), +u half-extent 10. */
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
  previewTriangles: { positions: [100, 100, 140, 100, 140, 140, 100, 140], indices: [0, 1, 2, 0, 2, 3] },
};

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
    setRegionSelected: vi.fn(),
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
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => null),
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

function makeClientMock(regions: SketchRegion[]) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn(() => () => {}),
    onDocumentChanged: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => null),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions })),
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

describe("tool chip anchors BESIDE the profile (T1)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  function build(regions: SketchRegion[]): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock(regions);
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

  it("puts the ARMED chip at least the profile half-extent off the region centroid", async () => {
    build([R0]);
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
    ]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();

    const world = toolChipStore.getState().worldPos;
    expect(world).not.toBeNull();
    // Region centroid is (10, 10, 0); the +u half-extent is 10.
    const [x, y] = world as [number, number, number];
    expect(Math.hypot(x - 10, y - 10)).toBeGreaterThanOrEqual(10);
    // The push is along the plane's in-plane +x, so v is untouched.
    expect(y).toBeCloseTo(10, 6);
    expect(x).toBeGreaterThan(20); // clear of the region's own +u edge
  });

  it("puts the REGION-PICK chip beside the regions it is asking the user to click", async () => {
    build([R0, R1]);
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();

    expect(engineMock.showRegionPick).toHaveBeenCalled();
    const world = toolChipStore.getState().worldPos as [number, number, number];
    // Combined (u,v) bbox over both squares is 0..140; its midpoint is (70, 70).
    expect(world[0]).toBeGreaterThan(140);
    expect(world[1]).toBeCloseTo(70, 6);
  });

  it("reports a click the chip absorbed while a region is still required — once", async () => {
    build([R0, R1]);
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();

    const chip = document.createElement("div");
    chip.dataset.testid = "model-tool-chip";
    const body = document.createElement("span");
    chip.appendChild(body);
    container.appendChild(chip);

    body.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: 10, clientY: 10, button: 0, bubbles: true }),
    );
    expect(viewportStore.getState().statusHint?.message).toBe(
      "The chip absorbed that click — click the region beside it",
    );
    expect(viewportStore.getState().statusHint?.severity).toBe("warn");

    // One-shot: a second absorbed press does not re-publish over whatever the
    // user is being told now.
    viewportStore.getState().setStatusHint("Select regions to extrude", { sticky: true });
    body.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: 10, clientY: 10, button: 0, bubbles: true }),
    );
    expect(viewportStore.getState().statusHint?.message).toBe("Select regions to extrude");
  });

  it("stays silent when the absorbed press lands on one of the chip's own controls", async () => {
    build([R0, R1]);
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();
    viewportStore.getState().setStatusHint("Select regions to extrude", { sticky: true });

    const chip = document.createElement("div");
    chip.dataset.testid = "model-tool-chip";
    const confirm = document.createElement("button");
    chip.appendChild(confirm);
    container.appendChild(chip);

    confirm.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: 10, clientY: 10, button: 0, bubbles: true }),
    );
    expect(viewportStore.getState().statusHint?.message).toBe("Select regions to extrude");
  });

  it("keeps an absorbed press out of the region behind it (no fall-through pick)", async () => {
    build([R0, R1]);
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();

    const chip = document.createElement("div");
    chip.dataset.testid = "model-tool-chip";
    const body = document.createElement("span");
    chip.appendChild(body);
    container.appendChild(chip);

    // (10,10) is inside r0 under the identity screen→plane mapping.
    body.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: 10, clientY: 10, button: 0, bubbles: true }),
    );
    body.dispatchEvent(
      new MouseEvent("pointerup", { clientX: 10, clientY: 10, button: 0, bubbles: true }),
    );
    expect(toolChipStore.getState().count).toBe(0);
  });
});
