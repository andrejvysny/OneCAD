/*
 * SketchController dispose parity (C5). dispose() cancels the live sketch + nulls the
 * session, guards every pending rAF (a frame that fires after teardown is a no-op),
 * and closes an in-flight drag gesture on the wire. Engine + client are faked.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = { kind: "XY", origin: [0, 0, 0], xAxis: [1, 0, 0], yAxis: [0, 1, 0], normal: [0, 0, 1] };
const ENTITIES: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }];

function makeEngineMock() {
  return {
    setPlanePickerVisible: vi.fn(),
    planePickerHover: vi.fn(),
    planePickerHitTest: vi.fn(() => null),
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
    setSketchAngleReference: vi.fn(),
    setSketchAnglePreview: vi.fn(),
    setSketchSnap: vi.fn(),
    updateSketchSession: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
    // Isotropic 1px-per-unit metric: matches `planePixelWorld: 1` above, so a
    // plane distance IS a screen-pixel distance in these tests.
    planeScreenMetric: vi.fn(() => ({ m00: 1, m01: 0, m10: 0, m11: 1 })),
    getCameraDistance: vi.fn(() => 100),
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
    endGesture: vi.fn(() => Promise.resolve({ sketchId: "sketch1", sketchRevision: 1, dof: 2, status: "UnderConstrained", solvedPositions: {} })),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController dispose parity", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  function build(): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
  }

  beforeEach(() => {
    resetStores();
    // Default: synchronous rAF (drag scheduling is deterministic). Individual tests
    // that need to hold a frame re-stub with a capturing variant.
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return 1;
    });
  });

  afterEach(() => {
    container.remove();
    vi.unstubAllGlobals();
  });

  function mouse(type: string, x: number, y: number, button: number, buttons: number): void {
    container.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true }));
  }

  it("dispose mid-session cancels the sketch and nulls the session", async () => {
    build();
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    expect(sketchStore.getState().session).not.toBeNull();

    controller.dispose();

    expect(clientMock.cancelSketch).toHaveBeenCalledWith("sketch1");
    expect(sketchStore.getState().session).toBeNull();
  });

  it("a pending rAF that fires AFTER dispose is a no-op", async () => {
    // Capturing rAF: hold the scheduled frame so it can fire post-dispose.
    let rafCb: FrameRequestCallback | null = null;
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      rafCb = cb;
      return 1;
    });
    build();
    toolStore.getState().setMode("sketch", "sketch1"); // default tool = line (drawing)
    await flush();
    engineMock.setSketchSnap.mockClear();

    // A drawing-mode move schedules the onPointerMove frame (captured, not run).
    mouse("pointermove", 10, 10, 0, 0);
    expect(rafCb).toBeTypeOf("function");

    controller.dispose();
    rafCb!(0); // the frame fires after teardown

    // The disposed guard bailed before touching the engine.
    expect(engineMock.setSketchSnap).not.toHaveBeenCalled();
  });

  it("dispose mid-drag closes the gesture on the wire (endGesture)", async () => {
    build();
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    toolStore.getState().setTool("select");
    await flush();

    mouse("pointerdown", 0, 0, 0, 1); // arm on the e1.Start handle
    mouse("pointermove", 30, 0, 0, 1); // past DRAG_PX → begin gesture
    await flush();
    expect(clientMock.beginGesture).toHaveBeenCalled();

    controller.dispose();
    expect(clientMock.endGesture).toHaveBeenCalledTimes(1);
  });
});
