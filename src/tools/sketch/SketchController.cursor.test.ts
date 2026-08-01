/*
 * SketchController — per-tool container cursor (U8). Draw tools (line/rect/
 * circle/arc/dimension) show a crosshair for aiming; pick tools (select/trim/
 * mirror) keep the default arrow. Reset to "" on exit()/dispose() so leaving
 * sketch mode never strands a crosshair over the model viewport. Engine +
 * client are faked (no WebGL / no backend).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore, type SketchTool } from "@/stores/toolStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

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
    setSketchGhost: vi.fn(),
    setSketchTrimGhost: vi.fn(),
    setSketchSnap: vi.fn(),
    updateSketchSession: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
  };
}

function makeClientMock() {
  return {
    enterSketch: vi.fn(
      (): Promise<SketchSession> =>
        Promise.resolve({
          sketchId: "sketch1",
          plane: PLANE,
          entities: [],
          constraints: [],
          dof: 0,
          status: "UnderConstrained",
        }),
    ),
    cancelSketch: vi.fn(() => Promise.resolve()),
    deleteSketch: vi.fn(() => Promise.resolve()),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController — per-tool cursor (U8)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
    toolStore.getState().setMode("sketch", "sketch1"); // default tool = line
    await flush();
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
  });

  it("draw tools (line/rect/centerRect/circle/arc/polygon/slot/point/dimension) show a crosshair", async () => {
    expect(container.style.cursor).toBe("crosshair"); // default tool = line
    for (const tool of [
      "rect",
      "centerRect",
      "circle",
      "arc",
      "polygon",
      "slot",
      "point",
      "dimension",
    ] as SketchTool[]) {
      toolStore.getState().setTool(tool);
      await flush();
      expect(container.style.cursor).toBe("crosshair");
    }
  });

  it("pick tools (select/trim/mirror) keep the default arrow cursor", async () => {
    for (const tool of ["select", "trim", "mirror"] as SketchTool[]) {
      toolStore.getState().setTool(tool);
      await flush();
      expect(container.style.cursor).toBe("default");
    }
  });

  it("exit() resets the cursor", async () => {
    expect(container.style.cursor).toBe("crosshair");
    toolStore.getState().setMode("model");
    await flush();
    expect(container.style.cursor).toBe("");
  });

  it("dispose() resets the cursor", () => {
    expect(container.style.cursor).toBe("crosshair");
    controller.dispose();
    expect(container.style.cursor).toBe("");
  });
});
