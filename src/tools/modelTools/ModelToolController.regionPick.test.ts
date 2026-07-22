/*
 * ModelToolController multi-region pick (jsdom): a finished sketch with >1 closed
 * region hands pointer ownership to the controller (orbit suppressed) so the user
 * clicks WHICH region to extrude/revolve; the picked regionId flows into the
 * draft (extrude) / commit op (revolve). A single region keeps the instant
 * auto-arm path. Esc cancels + restores orbit. A stale finishSketch result (the
 * arm was re-triggered) is dropped. Engine + client are faked (no WebGL / backend);
 * only the methods the controller drives are stubbed.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  OperationOp,
  PreviewDraft,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

// Two disjoint square regions, plane (u,v). Fan layout (index 0 = centroid) so
// profileFromRegion accepts each. r0 near origin; r1 far off.
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

// A vertical axis line to the LEFT of r1 (valid: does not cross the profile).
const AXIS_LINE = { id: "axis1", type: "Line" as const, p0: [95, 100] as [number, number], p1: [95, 140] as [number, number] };

const OP_RESULT: ApplyOperationResult = {
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }],
  removedBodies: [],
};

function makeSession(): SketchSession {
  return { sketchId: "sk", plane: PLANE, entities: [AXIS_LINE], constraints: [], dof: 0, status: "FullyConstrained" };
}

function makeEngineMock() {
  return {
    // Region pick facade (under test).
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
    // Identity plane mapping: client (x,y) → plane (u,v) so the region fixtures live
    // directly in pointer space.
    screenToPlaneOn: vi.fn((_plane: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    setRegionSelected: vi.fn(),
    planePixelWorld: vi.fn(() => 1),
    probePick: vi.fn(() => null),
    // Extrude preview.
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setExtrudePreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => null),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    setPreviewBody: vi.fn(),
    clearPreviewBody: vi.fn(),
    // Revolve preview.
    showRevolveAxisCandidates: vi.fn(),
    setRevolveAxisHover: vi.fn(),
    showRevolvePreview: vi.fn(),
    setRevolveAngle: vi.fn(),
    hideRevolvePreview: vi.fn(),
    // Cancel-path teardown (fired on every tool switch).
    hideGhostPreview: vi.fn(),
  };
}

function makeClientMock(finish: () => Promise<FinishSketchResult>) {
  return {
    onPreviewResult: vi.fn(() => () => {}),
    finishSketch: vi.fn(finish),
    // Model-mode arms read the sketch via getSketch (pure read; MODEL-HARDEN W0.5).
    getSketch: vi.fn(() => Promise.resolve(makeSession())),
    enterSketch: vi.fn(() => Promise.resolve(makeSession())),
    beginPreview: vi.fn((_draft: PreviewDraft) => Promise.resolve({ sessionId: "sess", previewBodyId: "pb" })),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(OP_RESULT)),
    applyOperation: vi.fn((_op: OperationOp) => Promise.resolve(OP_RESULT)),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("ModelToolController region pick", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let onBodyLoadedCbs: Array<(id: string) => void>;

  function build(finish: () => Promise<FinishSketchResult>): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock(finish);
    onBodyLoadedCbs = [];
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: (cb) => {
        onBodyLoadedCbs.push(cb);
        return () => {};
      },
    });
  }

  beforeEach(() => {
    resetStores();
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
  });

  function pointer(type: string, x: number, y: number, button: number, buttons: number): void {
    container.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true }));
  }

  /** A plain click gesture (down + up, no move) at a client point. */
  function click(x: number, y: number): void {
    pointer("pointerdown", x, y, 0, 1);
    pointer("pointerup", x, y, 0, 0);
  }

  it("(a) two regions → MULTI-select; toggling region 2 then Enter arms extrude with regions[1].regionId", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    toolStore.getState().setTool("extrude");
    await flush();

    // Region pick is active: layer shown, orbit suppressed, NOT yet armed.
    expect(engineMock.showRegionPick).toHaveBeenCalledTimes(1);
    expect(engineMock.showRegionPick.mock.calls[0][1]).toEqual([R0, R1]);
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(true);
    expect(clientMock.beginPreview).not.toHaveBeenCalled();

    // Hover tints the region under the pointer.
    pointer("pointermove", 130, 110, 0, 0);
    expect(engineMock.setRegionHover).toHaveBeenLastCalledWith("r1");

    // Wave 2: a click TOGGLES membership (does NOT arm) — the selection retints r1.
    click(130, 110);
    await flush();
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(engineMock.setRegionSelected).toHaveBeenLastCalledWith(["r1"]);

    // Enter confirms the selection → arms extrude with regions[1].regionId in the draft.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();

    expect(engineMock.hideRegionPick).toHaveBeenCalled();
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    const draft = clientMock.beginPreview.mock.calls[0][0];
    expect(draft.opType).toBe("Extrude");
    expect(draft.regionId).toBe("r1");
    expect(controller.extrudeActive).toBe(true);
  });

  it("(a-multi) toggling BOTH regions then Enter arms N=2 extrude sessions (one per region)", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    toolStore.getState().setTool("extrude");
    await flush();

    click(10, 10); // r0
    await flush();
    click(130, 110); // r1
    await flush();
    expect(engineMock.setRegionSelected).toHaveBeenLastCalledWith(["r0", "r1"]);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();

    // One preview session per region — regionIds preserved in click order.
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);
    const drafts = clientMock.beginPreview.mock.calls.map((c) => c[0].regionId);
    expect(drafts).toEqual(["r0", "r1"]);
    expect(controller.extrudeActive).toBe(true);
  });

  it("(a-dblclick) a double-click on a region = select-only-it + confirm (accelerator)", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    toolStore.getState().setTool("extrude");
    await flush();

    // First toggles r0 IN; the immediate second click (same region) confirms r0 alone.
    click(10, 10);
    click(10, 10);
    await flush();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    expect(clientMock.beginPreview.mock.calls[0][0].regionId).toBe("r0");
    expect(controller.extrudeActive).toBe(true);
  });

  it("(a2) revolve two regions → toggle region 2 + Enter → axis pick → 360° commit sends regions[1].regionId", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    toolStore.getState().setTool("revolve");
    await flush();
    expect(engineMock.showRegionPick).toHaveBeenCalledTimes(1);

    // Toggle region 2 + Enter → enters axis-pick (region pick precedes the axis phase).
    click(130, 110);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(engineMock.showRevolveAxisCandidates).toHaveBeenCalledTimes(1);

    // Pick the axis line (click near it), then a plain click commits the default 360°.
    click(95, 120);
    await flush();
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1);

    click(200, 200);
    await flush();

    expect(clientMock.applyOperation).toHaveBeenCalledTimes(1);
    const op = clientMock.applyOperation.mock.calls[0][0];
    expect(op.opType).toBe("Revolve");
    // Narrow the OperationOp union to the Revolve variant (which carries regionId).
    if (op.opType !== "Revolve") throw new Error("expected a Revolve op");
    expect(op.regionId).toBe("r1");
  });

  it("(b) single region keeps the instant auto-arm path (no region pick)", async () => {
    build(() => Promise.resolve({ regions: [R0] }));
    toolStore.getState().setTool("extrude");
    await flush();

    expect(engineMock.showRegionPick).not.toHaveBeenCalled();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    const draft = clientMock.beginPreview.mock.calls[0][0];
    expect(draft.regionId).toBe("r0");
    expect(controller.extrudeActive).toBe(true);
  });

  it("(c) Esc during region pick cancels cleanly and restores orbit", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    toolStore.getState().setTool("extrude");
    await flush();
    expect(engineMock.showRegionPick).toHaveBeenCalledTimes(1);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flush();

    expect(engineMock.hideRegionPick).toHaveBeenCalled();
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
    expect(toolStore.getState().modelTool).toBe("select");
    // A later click resolves nothing (pick torn down) — no arm.
    click(130, 110);
    await flush();
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
  });

  it("(d) a stale finishSketch result (arm re-triggered) is dropped — no second region pick", async () => {
    let resolveFirst!: (r: FinishSketchResult) => void;
    const firstFinish = new Promise<FinishSketchResult>((r) => {
      resolveFirst = r;
    });
    let call = 0;
    build(() => {
      call += 1;
      return call === 1 ? firstFinish : Promise.resolve({ regions: [R0, R1] });
    });

    // Arm 1: finishSketch is in flight.
    toolStore.getState().setTool("extrude");
    await flush();
    expect(engineMock.showRegionPick).not.toHaveBeenCalled();

    // Re-trigger the arm (select → extrude) while arm 1 is still pending.
    toolStore.getState().setTool("select");
    toolStore.getState().setTool("extrude");
    await flush(); // arm 2: finishSketch resolved → enterSketch → region pick
    await flush();
    expect(engineMock.showRegionPick).toHaveBeenCalledTimes(1);

    // The stale arm-1 result finally lands — it must NOT open a second region pick.
    resolveFirst({ regions: [R0, R1] });
    await flush();
    expect(engineMock.showRegionPick).toHaveBeenCalledTimes(1);
  });
});
