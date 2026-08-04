/*
 * SketchController — Enter ends the line chain (U4). Mirrors the existing
 * Esc-chain intercept: a capture-phase window keydown, gated on
 * `machine && machineState.anchors.length > 0` so a chain-less Enter falls
 * through to the global finishSketch shortcut (useShortcuts), and guarded
 * against a focused text input (e.g. the dimension chip) owning its own
 * Enter. Drives the machine state directly (the full click-driven draw flow
 * is covered by a later wave's SketchController.draw.test.ts) — engine +
 * client are faked (no WebGL / no backend).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { resetStores } from "@/test/resetStores";
import { lineTool, type ToolMachine, type ToolState } from "./toolMachine";

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
    moveChip: vi.fn(),
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

type Internals = { machine: ToolMachine | null; machineState: ToolState | null };

describe("SketchController — Enter ends the line chain (U4)", () => {
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

  const internals = (): Internals => controller as unknown as Internals;

  function keydown(target: EventTarget, key: string): KeyboardEvent {
    const evt = new KeyboardEvent("keydown", { key, cancelable: true, bubbles: true });
    target.dispatchEvent(evt);
    return evt;
  }

  it("Enter with an in-progress chain ends it: preview/ghost cleared, machineState reset", () => {
    const c = internals();
    c.machine = lineTool;
    c.machineState = {
      anchors: [
        { x: 0, y: 0 },
        { x: 40, y: 0 },
      ],
      cursor: { x: 40, y: 0 },
    };
    engineMock.setSketchPreview.mockClear();
    engineMock.setSketchGhost.mockClear();

    const evt = keydown(window, "Enter");

    expect(evt.defaultPrevented).toBe(true);
    expect(engineMock.setSketchPreview).toHaveBeenCalledWith([]);
    expect(engineMock.setSketchGhost).toHaveBeenCalledWith(null, null);
    expect(c.machineState).toEqual({ anchors: [], cursor: null });
  });

  it("a chain-less Enter (no anchors) is NOT intercepted", () => {
    const c = internals();
    c.machine = lineTool;
    c.machineState = { anchors: [], cursor: null };
    engineMock.setSketchPreview.mockClear();

    const evt = keydown(window, "Enter");

    expect(evt.defaultPrevented).toBe(false);
    expect(engineMock.setSketchPreview).not.toHaveBeenCalled();
    expect(c.machineState).toEqual({ anchors: [], cursor: null });
  });

  it("Enter typed into a text input is NOT intercepted, even mid-chain", () => {
    const c = internals();
    c.machine = lineTool;
    c.machineState = { anchors: [{ x: 0, y: 0 }], cursor: { x: 0, y: 0 } };
    engineMock.setSketchPreview.mockClear();

    const input = document.createElement("input");
    document.body.appendChild(input);
    const evt = keydown(input, "Enter");
    input.remove();

    expect(evt.defaultPrevented).toBe(false);
    expect(engineMock.setSketchPreview).not.toHaveBeenCalled();
    // The chain is untouched — the input owns this Enter, not the controller.
    expect(c.machineState).toEqual({ anchors: [{ x: 0, y: 0 }], cursor: { x: 0, y: 0 } });
  });
});
