/*
 * W1-B: the sticky construction DRAW modifier (sketchStore.constructionMode).
 *
 * The tool machines are mode-blind — `SketchController.decorate` is the single
 * place the flag is OR-ed onto their output, applied to BOTH the engine preview
 * (what the user sees dashed) and the committed drafts (what gets authored), so
 * the two can never disagree. Same faked engine/client + 1:1 screenToPlane as
 * SketchController.draw.test.ts; rAF fires synchronously so a pointermove's
 * preview is observable without waiting a frame.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  SketchConstraint,
  SketchEntity,
  SketchPlane,
  SketchSession,
  SketchUpsertResult,
} from "@/ipc/types";
import type { DraftEntity } from "./toolMachine";
import { toolStore, type SketchTool } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { settingsStore } from "@/stores/settingsStore";
import { resetStores } from "@/test/resetStores";
import { flushSketchMutations } from "./sketchService";

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
    sketchUpsert: vi.fn(
      (
        sketchId: string,
        _entities: SketchEntity[],
        _constraints: SketchConstraint[],
      ): Promise<SketchUpsertResult> =>
        Promise.resolve({
          sketchId,
          sketchRevision: 1,
          dof: 0,
          status: "UnderConstrained",
          conflicting: [],
          solvedPositions: {},
        }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

function disableSnapping(): void {
  const s = settingsStore.getState();
  for (const key of [
    "grid",
    "sketchGuideLines",
    "sketchGuidePoints",
    "quadrant",
    "intersection",
    "onCurve",
  ] as const) {
    s.setSnap(key, false);
  }
}

describe("SketchController — construction draw mode", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
    disableSnapping();
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
    toolStore.getState().setMode("sketch", "sketch1"); // default tool = line
    await flush();
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    vi.unstubAllGlobals();
  });

  function click(x: number, y: number): void {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
    );
  }

  function move(x: number, y: number): void {
    container.dispatchEvent(
      new MouseEvent("pointermove", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
    );
  }

  async function setTool(tool: SketchTool): Promise<void> {
    toolStore.getState().setTool(tool);
    await flush();
  }

  const entities = (): SketchEntity[] => sketchStore.getState().session!.entities;
  /** The drafts of the newest `setSketchPreview` call. */
  const lastPreview = (): DraftEntity[] => {
    const calls = engineMock.setSketchPreview.mock.calls;
    return (calls[calls.length - 1]?.[0] ?? []) as DraftEntity[];
  };

  it("mode OFF: committed geometry carries no construction flag", async () => {
    click(0, 0);
    click(50, 0);
    await flushSketchMutations();
    expect(entities()).toHaveLength(1);
    expect(entities()[0].construction).toBeFalsy();
  });

  it("mode ON: every committed entity is construction (line + rect)", async () => {
    sketchStore.getState().toggleConstructionMode();
    expect(sketchStore.getState().constructionMode).toBe(true);

    click(0, 0);
    click(50, 0);
    await flushSketchMutations();
    expect(entities()[0].construction).toBe(true);

    await setTool("rect");
    click(100, 100);
    click(150, 150);
    await flushSketchMutations();
    const rect = entities().slice(1);
    expect(rect).toHaveLength(4);
    expect(rect.every((e) => e.construction === true)).toBe(true);
  });

  it("mode ON: the rubber-band PREVIEW is decorated too (what you see is what commits)", async () => {
    click(0, 0);
    move(50, 0);
    expect(lastPreview()[0]?.construction).toBeFalsy();

    sketchStore.getState().toggleConstructionMode();
    move(50, 10);
    expect(lastPreview()[0]?.construction).toBe(true);
  });

  it("toggling back OFF stops decorating (sticky, not one-shot, and reversible)", async () => {
    sketchStore.getState().toggleConstructionMode();
    click(0, 0);
    click(50, 0);
    click(50, 50); // a SECOND segment in the same chain — sticky, not consumed
    await flushSketchMutations();
    expect(entities().every((e) => e.construction === true)).toBe(true);

    sketchStore.getState().toggleConstructionMode();
    await setTool("circle");
    click(200, 200);
    click(210, 200);
    await flushSketchMutations();
    const all = entities();
    const circle = all[all.length - 1];
    expect(circle.type).toBe("Circle");
    expect(circle.construction).toBeFalsy();
  });

  it("mode ON: the arc's own construction radius rubber-band is unaffected", async () => {
    sketchStore.getState().toggleConstructionMode();
    await setTool("arc");
    click(0, 0); // center
    move(30, 0); // radius rubber-band: the machine already marks it construction
    expect(lastPreview().every((d) => d.construction === true)).toBe(true);
  });
});
