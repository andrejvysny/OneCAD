/*
 * SketchController plane-pick phase (jsdom): bare sketch entry shows the plane
 * picker; a click on a quad creates the sketch on that plane and runs the normal
 * open-session flow; Esc cancels back to model. The engine + client are faked
 * (no WebGL / no backend) — only the methods the controller drives are stubbed.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickablePlane } from "@/viewport/engine/PlanePicker";
import type { CadClient } from "@/ipc/client";
import type { EnterSketchTarget, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { sketchStore } from "@/stores/sketchStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/** A fresh session: existing id → that id; a plane pick → a new backend id. */
function makeSession(target: EnterSketchTarget): SketchSession {
  const sketchId = typeof target === "string" ? target : "sketchNew";
  return { sketchId, plane: PLANE, entities: [], constraints: [], dof: 3, status: "UnderConstrained" };
}

function makeEngineMock(hit: () => PickablePlane | null) {
  return {
    setPlanePickerVisible: vi.fn(),
    planePickerHover: vi.fn(),
    planePickerHitTest: vi.fn(() => hit()),
    clearPlanePickerHover: vi.fn(),
    // DATUM W1: the plane-pick path consults the datum layer first.
    // W3: the plane-pick phase falls through to a body FACE (probePick).
    probePick: vi.fn(() => null),
    datumHitTest: vi.fn(() => null),
    setDatumHover: vi.fn(),
    enterSketch: vi.fn(),
    exitSketch: vi.fn(),
    setSketchDrawingActive: vi.fn(),
    setSketchPreview: vi.fn(),
    moveChip: vi.fn(),
    setSketchGhost: vi.fn(),
    setSketchTrimGhost: vi.fn(),
    setSketchSnap: vi.fn(),
  };
}

function makeClientMock() {
  return {
    enterSketch: vi.fn((target: EnterSketchTarget) => Promise.resolve(makeSession(target))),
    cancelSketch: vi.fn(() => Promise.resolve()),
    deleteSketch: vi.fn(() => Promise.resolve()),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController plane-pick", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;
  let hitKind: PickablePlane | null;

  beforeEach(() => {
    resetStores();
    hitKind = null;
    engineMock = makeEngineMock(() => hitKind);
    clientMock = makeClientMock();
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
  });

  function pointer(type: string, x: number, y: number, button: number, buttons: number): void {
    container.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true }));
  }

  it("bare sketch entry shows the plane picker and does NOT enter a session", async () => {
    toolStore.getState().setMode("sketch");
    await flush();

    expect(engineMock.setPlanePickerVisible).toHaveBeenCalledWith(true);
    expect(clientMock.enterSketch).not.toHaveBeenCalled();
    expect(viewportStore.getState().activeSketchId).toBeNull();
  });

  it("a click on a plane creates the sketch and opens the session", async () => {
    toolStore.getState().setMode("sketch");
    await flush();

    hitKind = "XZ";
    pointer("pointerdown", 100, 100, 0, 1);
    pointer("pointerup", 100, 100, 0, 0);
    await flush();

    expect(clientMock.enterSketch).toHaveBeenCalledTimes(1);
    expect(clientMock.enterSketch).toHaveBeenCalledWith({ newOnPlane: "XZ" });
    // Session wired into the stores + engine.
    expect(viewportStore.getState().activeSketchId).toBe("sketchNew");
    expect(documentStore.getState().sketches["sketchNew"]?.name).toBe("Sketch 6");
    expect(selectionStore.getState().selected).toEqual([{ kind: "sketch", id: "sketchNew" }]);
    expect(viewportStore.getState().projection).toBe("ortho");
    // Picker was hidden on confirm.
    expect(engineMock.setPlanePickerVisible).toHaveBeenLastCalledWith(false);
  });

  it("Esc during the pick cancels back to model without entering a session", async () => {
    toolStore.getState().setMode("sketch");
    await flush();
    expect(engineMock.setPlanePickerVisible).toHaveBeenCalledWith(true);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flush();

    expect(toolStore.getState().mode).toBe("model");
    expect(engineMock.setPlanePickerVisible).toHaveBeenLastCalledWith(false);
    expect(clientMock.enterSketch).not.toHaveBeenCalled();
  });

  it("mode-flip DURING the deferred enter cancels + deletes the fresh sketch (no row, no session)", async () => {
    // Control the enter resolution so we can flip the mode while it's pending.
    let resolveEnter!: (s: SketchSession) => void;
    const deferred = new Promise<SketchSession>((r) => {
      resolveEnter = r;
    });
    clientMock.enterSketch.mockImplementationOnce(() => deferred);

    toolStore.getState().setMode("sketch");
    await flush(); // beginPlanePick

    hitKind = "XZ";
    pointer("pointerdown", 100, 100, 0, 1);
    pointer("pointerup", 100, 100, 0, 0);
    await flush(); // confirmPlanePick → openSession is now awaiting the deferred enter
    expect(clientMock.enterSketch).toHaveBeenCalledWith({ newOnPlane: "XZ" });

    // User leaves for model mode BEFORE the enter resolves.
    toolStore.getState().setMode("model");
    await flush();

    // The enter finally resolves — openSession must tear down the orphan.
    resolveEnter(makeSession({ newOnPlane: "XZ" }));
    await flush();

    expect(clientMock.cancelSketch).toHaveBeenCalledWith("sketchNew");
    expect(clientMock.deleteSketch).toHaveBeenCalledWith("sketchNew"); // fresh-create cleanup
    // The late session was NOT wired in: no tree row, no active session.
    expect(documentStore.getState().sketches["sketchNew"]).toBeUndefined();
    expect(sketchStore.getState().session).toBeNull();
  });

  it("an explicit id skips the picker and enters that sketch directly", async () => {
    toolStore.getState().setMode("sketch", "sketch2");
    await flush();

    expect(engineMock.setPlanePickerVisible).not.toHaveBeenCalled();
    expect(clientMock.enterSketch).toHaveBeenCalledTimes(1);
    expect(clientMock.enterSketch).toHaveBeenCalledWith("sketch2");
    expect(viewportStore.getState().activeSketchId).toBe("sketch2");
  });
});
