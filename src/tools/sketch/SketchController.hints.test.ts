/*
 * FP-S14 (docs/qa/UX_REVIEW_2026-09-14.md S14): sketch Select mode gets its own
 * status hint — previously it fell through the DRAW_TOOL_HINT fallback, which
 * (a) has no "select" entry and (b) is gated on the live-dimensions pref, so
 * Select showed nothing at all. Engine + client are faked (no WebGL / no
 * backend), mirroring `SketchController.select.test.ts`'s setup.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

const ENTITIES: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }];

const SELECT_HINT = "Click geometry to select · drag to move · Delete to remove";

function makeEngineMock() {
  return {
    setPlanePickerVisible: vi.fn(),
    planePickerHover: vi.fn(),
    planePickerHitTest: vi.fn(() => null),
    clearPlanePickerHover: vi.fn(),
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
    setSketchProjectedIds: vi.fn(),
    setSketchEntityStates: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
    getCameraDistance: vi.fn(() => 100),
    planeScreenMetric: vi.fn(() => ({ m00: 1, m01: 0, m10: 0, m11: 1 })),
  };
}

function makeClientMock() {
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
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController select-mode hint", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(() => {
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
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
  });

  it("hints Select on session entry (re-opening a sketch defaults to Select)", async () => {
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();

    expect(toolStore.getState().sketchTool).toBe("select");
    const hint = viewportStore.getState().statusHint;
    expect(hint?.message).toBe(SELECT_HINT);
    expect(hint?.sticky).toBe(true);
  });

  it("hints Select when switching back to it from a draw tool", async () => {
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();

    toolStore.getState().setTool("select");
    await flush();

    expect(viewportStore.getState().statusHint?.message).toBe(SELECT_HINT);
  });

  it("shows the Select hint even with live dimensions turned OFF", async () => {
    settingsStore.getState().setShow("liveDimensions", false);
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();

    expect(viewportStore.getState().statusHint?.message).toBe(SELECT_HINT);
  });
});
