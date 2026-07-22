/*
 * ModelToolController commit-gesture wiring (jsdom, MODEL-HARDEN Wave 1). Proves
 * the professional commit gesture: a drag RELEASE keeps the tool armed (no commit,
 * chip stays); Enter / chip-✓ / click-away confirm; a FAILED commit stays armed,
 * surfaces the reason, and RE-ARMS the preview so the work is never lost; Esc /
 * tool-switch tears every open preview session down. Engine + client are faked.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
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

// One square region (fan layout, index 0 = centroid) so profileFromRegion accepts it.
const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }],
  removedBodies: [],
});

const failResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [],
  removedBodies: [],
  errorMessage: "worker exploded",
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
    planePixelWorld: vi.fn(() => 1),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setExtrudePreviewTint: vi.fn(),
    setRegionSelected: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => ({ origin: [0, 0, 100] as const, dir: [0, 0, -1] as const })),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => true),
    setPreviewBody: vi.fn(),
    clearPreviewBody: vi.fn(),
    showRevolveAxisCandidates: vi.fn(),
    setRevolveAxisHover: vi.fn(),
    showRevolvePreview: vi.fn(),
    setRevolveAngle: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    probePick: vi.fn(() => null),
  };
}

function makeClientMock(endResult: () => ApplyOperationResult) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn(() => () => {}),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketch: vi.fn(() => Promise.resolve(makeSession())),
    beginPreview: vi.fn((_d: PreviewDraft) => Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` })),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(endResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("ModelToolController commit gesture (Wave 1)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let bodyCbs: Array<(id: string) => void>;

  function build(endResult: () => ApplyOperationResult = okResult): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock(endResult);
    bodyCbs = [];
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: (cb) => {
        bodyCbs.push(cb);
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

  /** Arm the extrude tool on the single-region sketch and settle the async arm. */
  async function armExtrude(): Promise<void> {
    toolStore.getState().setTool("extrude");
    await flush();
  }

  it("release from a drag keeps the tool armed (no commit) and the chip stays", async () => {
    build();
    await armExtrude();
    expect(controller.extrudeActive).toBe(true);
    expect(toolChipStore.getState().kind).toBe("extrudeDepth");

    controller.forceExtrudeGrab(); // → dragging
    pointer("pointermove", 10, 40, 0, 1); // sets a depth from the ray
    pointer("pointerup", 10, 40, 0, 0); // release
    await flush();

    expect(clientMock.endPreview).not.toHaveBeenCalled(); // release NEVER commits
    expect(controller.extrudeActive).toBe(true); // still armed
    expect(toolStore.getState().phase).toBe("armed");
    expect(toolChipStore.getState().kind).toBe("extrudeDepth"); // chip stays editable
  });

  it("Enter confirms when the canvas is focused, but NOT while a chip input has focus", async () => {
    build();
    await armExtrude();

    // Input focused → the capture-phase Enter is skipped (no double-fire with the chip).
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();
    expect(clientMock.endPreview).not.toHaveBeenCalled();
    input.remove();

    // Canvas Enter (target = window) confirms → endPreview(commit).
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledTimes(1);
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
  });

  it("a failed commit stays armed, names the reason, and re-arms the preview", async () => {
    build(failResult);
    await armExtrude();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1); // arm

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" })); // confirm → fails
    await flush();
    await flush(); // let the re-arm beginPreview resolve

    expect(clientMock.endPreview).toHaveBeenCalledTimes(1);
    expect(controller.extrudeActive).toBe(true); // back to armed (work kept)
    expect(toolStore.getState().phase).toBe("armed");
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2); // preview RE-ARMED
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("worker exploded"); // reason named
  });

  it("a successful confirm tears down and selects the new body once it loads", async () => {
    build();
    await armExtrude();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);

    // Body reconcile: firing onBodyLoaded drops L1, selects the body, exits the tool.
    expect(bodyCbs.length).toBeGreaterThan(0);
    bodyCbs.forEach((cb) => cb("b1"));
    await flush();
    expect(controller.extrudeActive).toBe(false);
    expect(toolStore.getState().modelTool).toBe("select");
    expect(selectionStore.getState().selected).toEqual([{ kind: "body", id: "b1" }]);
  });

  it("a tool switch (Esc ladder tail) tears the open preview session down with endPreview(false)", async () => {
    build();
    await armExtrude();
    toolStore.getState().setTool("select"); // the cancelAll/tool-switch path
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), false);
    expect(controller.extrudeActive).toBe(false);
  });

  it("re-edit arms value + ✓/✕ only (no ⇔) and confirms through the same gesture", async () => {
    build();
    documentStore.setState({
      features: [{ id: "feat-ex", kind: "extrude", label: "Extrude", valueText: "20 mm", status: "ok" }],
    });
    controller.editExtrudeFeature("feat-ex");
    await flush();

    expect(controller.extrudeActive).toBe(true);
    expect(toolChipStore.getState().kind).toBe("extrudeDepth");
    expect(toolChipStore.getState().showSymmetric).toBe(false); // re-edit hides ⇔
    // The winning arm carried the featureId into the draft (param-only edit path).
    const calls = clientMock.beginPreview.mock.calls;
    const draft = calls[calls.length - 1][0] as PreviewDraft;
    expect(draft.params.featureId).toBe("feat-ex");

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
  });
});
