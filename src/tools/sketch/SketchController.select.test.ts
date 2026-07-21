/*
 * SketchController select-tool flow (jsdom): click-select (plain / shift toggle /
 * miss-clears) and point-handle drag through the gesture lane (begin → solve → end),
 * plus Esc-cancel restoring pre-drag geometry. Engine + client are faked (no WebGL /
 * no backend); screenToPlane is 1:1 so client coords ARE plane coords.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

const ENTITIES: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }];

function makeEngineMock() {
  return {
    setPlanePickerVisible: vi.fn(),
    planePickerHover: vi.fn(),
    planePickerHitTest: vi.fn(() => null),
    enterSketch: vi.fn(),
    exitSketch: vi.fn(),
    setSketchDrawingActive: vi.fn(),
    setSketchPreview: vi.fn(),
    setSketchGhost: vi.fn(),
    setSketchTrimGhost: vi.fn(),
    setSketchSnap: vi.fn(),
    updateSketchSession: vi.fn(),
    // 1:1 mapping so client coords equal plane coords.
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
  };
}

function makeClientMock() {
  let seq = 0;
  return {
    enterSketch: vi.fn(
      (): Promise<SketchSession> =>
        Promise.resolve({
          sketchId: "sketch1",
          plane: PLANE,
          entities: ENTITIES.map((e) => ({ ...e })),
          constraints: [],
          dof: 2,
          status: "UnderConstrained",
        }),
    ),
    cancelSketch: vi.fn(() => Promise.resolve()),
    deleteSketch: vi.fn(() => Promise.resolve()),
    beginGesture: vi.fn(() => Promise.resolve({ gestureId: 1, ready: true })),
    solveDrag: vi.fn((target: [number, number]) =>
      Promise.resolve({
        gestureId: 1,
        seq: ++seq,
        status: "success",
        dof: 2,
        conflicting: [],
        positions: { "e1.Start": target },
        solveMicros: 0,
        superseded: false,
      }),
    ),
    endGesture: vi.fn((target?: [number, number]) =>
      Promise.resolve({
        sketchId: "sketch1",
        sketchRevision: 1,
        dof: 2,
        status: "UnderConstrained",
        solvedPositions: target ? { "e1.Start": target } : {},
      }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController select tool", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
    // rAF fires synchronously so drag-solve scheduling is deterministic.
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return 1;
    });
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
    // Enter an existing sketch, then arm the select tool.
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    toolStore.getState().setTool("select");
    await flush();
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    vi.unstubAllGlobals();
  });

  function mouse(type: string, x: number, y: number, button: number, buttons: number, mods?: { shiftKey?: boolean; metaKey?: boolean }): void {
    container.dispatchEvent(
      new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true, ...mods }),
    );
  }

  const selected = () => sketchSelectionStore.getState().selected;

  it("a click on a vertex selects that point handle", () => {
    mouse("pointerdown", 0, 0, 0, 1);
    mouse("pointerup", 0, 0, 0, 0);
    expect(selected()).toEqual([{ entityId: "e1", point: "Start" }]);
  });

  it("a click on empty space clears the selection", () => {
    sketchSelectionStore.getState().set([{ entityId: "e1", point: "Start" }]);
    mouse("pointerdown", 200, 200, 0, 1);
    mouse("pointerup", 200, 200, 0, 0);
    expect(selected()).toEqual([]);
  });

  it("Shift-click toggles the second handle into the selection", () => {
    mouse("pointerdown", 0, 0, 0, 1); // plain click → Start
    mouse("pointerup", 0, 0, 0, 0);
    mouse("pointerdown", 40, 0, 0, 1, { shiftKey: true }); // shift-click → +End
    mouse("pointerup", 40, 0, 0, 0, { shiftKey: true });
    expect(selected()).toEqual([
      { entityId: "e1", point: "Start" },
      { entityId: "e1", point: "End" },
    ]);
    // Shift-click End again toggles it off.
    mouse("pointerdown", 40, 0, 0, 1, { shiftKey: true });
    mouse("pointerup", 40, 0, 0, 0, { shiftKey: true });
    expect(selected()).toEqual([{ entityId: "e1", point: "Start" }]);
  });

  it("dragging a point handle runs begin → solve → end and keeps the selection", async () => {
    mouse("pointerdown", 0, 0, 0, 1); // arm on Start
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(true); // orbit suppressed
    mouse("pointermove", 30, 0, 0, 1); // past DRAG_PX → begin gesture
    await flush();

    expect(clientMock.beginGesture).toHaveBeenCalledWith("sketch1", "e1.Start");
    expect(clientMock.solveDrag).toHaveBeenCalledWith([30, 0]);
    // The live preview moved Start via updateSketchSession (End untouched).
    const calls = engineMock.updateSketchSession.mock.calls;
    const preview = calls[calls.length - 1][1] as SketchEntity[];
    expect(preview[0].p0).toEqual([30, 0]);
    expect(preview[0].p1).toEqual([40, 0]);

    mouse("pointerup", 30, 0, 0, 0);
    await flush();

    expect(clientMock.endGesture).toHaveBeenCalledTimes(1);
    expect(clientMock.endGesture).toHaveBeenCalledWith([30, 0]);
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(false); // orbit restored
    // Committed to the session; selection stays on the dragged entity.
    expect(sketchStore.getState().session!.entities[0].p0).toEqual([30, 0]);
    expect(selected()).toEqual([{ entityId: "e1", point: "Start" }]);
  });

  it("accumulates incremental solve deltas — a point omitted from a later response stays moved", async () => {
    // Real-worker shape: each response carries only points changed since the
    // PREVIOUS response. Response 1 moves Start AND drags coupled End; response 2
    // moves Start only — End must NOT snap back to its pre-drag pose.
    const responses = [
      { "e1.Start": [10, 0] as [number, number], "e1.End": [50, 0] as [number, number] },
      { "e1.Start": [20, 0] as [number, number] },
    ];
    let seq = 0;
    clientMock.solveDrag.mockImplementation(() =>
      Promise.resolve({
        gestureId: 1,
        seq: ++seq,
        status: "success",
        dof: 2,
        conflicting: [],
        positions: responses[Math.min(seq - 1, responses.length - 1)],
        solveMicros: 0,
        superseded: false,
      }),
    );
    mouse("pointerdown", 0, 0, 0, 1);
    mouse("pointermove", 10, 0, 0, 1);
    await flush();
    mouse("pointermove", 20, 0, 0, 1);
    await flush();

    const calls = engineMock.updateSketchSession.mock.calls;
    const preview = calls[calls.length - 1][1] as SketchEntity[];
    expect(preview[0].p0).toEqual([20, 0]); // latest delta
    expect(preview[0].p1).toEqual([50, 0]); // response-1 coupled move retained
  });

  it("idle pointer moves drive hover (set on hit, cleared on miss)", () => {
    mouse("pointermove", 0, 0, 0, 0); // no button — over e1.Start
    expect(sketchSelectionStore.getState().hover).toEqual({ entityId: "e1", point: "Start" });
    mouse("pointermove", 200, 200, 0, 0); // empty space
    expect(sketchSelectionStore.getState().hover).toBeNull();
  });

  it("Esc mid-drag ends the gesture and restores pre-drag geometry", async () => {
    mouse("pointerdown", 0, 0, 0, 1);
    mouse("pointermove", 30, 0, 0, 1);
    await flush(); // dragging live, preview moved to [30,0]

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flush();

    // Ended at the ORIGINAL position (no explicit cancel verb on the wire).
    expect(clientMock.endGesture).toHaveBeenCalledWith([0, 0]);
    // Geometry restored to pre-drag.
    expect(sketchStore.getState().session!.entities[0].p0).toEqual([0, 0]);
  });

  it("switching away from select clears the sketch selection", () => {
    mouse("pointerdown", 0, 0, 0, 1);
    mouse("pointerup", 0, 0, 0, 0);
    expect(selected()).toHaveLength(1);
    toolStore.getState().setTool("line");
    expect(selected()).toEqual([]);
  });
});
