/*
 * SketchController — snap indicator clears on commit / gesture-end / tool
 * switch (audit A3). Before this fix the marker + hint chip from the last
 * accepted decision stayed on screen indefinitely (observed: a "Grid" chip
 * stuck at a just-committed circle's center, surviving a tool switch).
 *
 * Harness mirrors SketchController.draw.test.ts (real pointer dispatch,
 * screenToPlane 1:1, every frontend snap type off); rAF is stubbed to run
 * inline (SketchController.liveDim.test.ts's pattern) so a `move()` observably
 * calls `setSketchSnap` on the next line, with no async wait needed.
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
import { toolStore } from "@/stores/toolStore";
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
    planeScreenMetric: vi.fn(() => ({ m00: 1, m01: 0, m10: 0, m11: 1 })),
    getCameraDistance: vi.fn(() => 100),
  };
}

function okResult(sketchId: string): SketchUpsertResult {
  return {
    sketchId,
    sketchRevision: 1,
    dof: 0,
    status: "UnderConstrained",
    conflicting: [],
    solvedPositions: {},
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
      ): Promise<SketchUpsertResult> => Promise.resolve(okResult(sketchId)),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

// Every frontend snap TYPE off — these tests only care whether the LAST call
// is null, not which decision preceded it (a raw "nothing accepted" decision
// is still a non-null object, which is all a "did it clear" test needs).
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

describe("SketchController — snap indicator clears (A3)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
    disableSnapping();
    // Pointer-move is rAF-coalesced; run it inline so `move()` is observable
    // synchronously (SketchController.liveDim.test.ts's pattern).
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
    container.dispatchEvent(new MouseEvent("pointermove", { clientX: x, clientY: y, bubbles: true }));
  }

  function keydown(key: string): void {
    window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  }

  /** The point of the LAST decision `setSketchSnap` was called with (`null` when
   *  the indicator was cleared) — deliberately ignores the `showHints` arg, whose
   *  value depends on whether a live-dim chip happens to be open this exact tick. */
  const lastSnapPoint = (): unknown => {
    const calls = engineMock.setSketchSnap.mock.calls;
    return calls[calls.length - 1]?.[0];
  };

  it("a two-click line commit clears the indicator", async () => {
    click(0, 0);
    move(10, 0); // populates a live decision — the marker/hint the bug left stuck
    expect(lastSnapPoint()).not.toBeNull();
    click(10, 0); // commits the segment
    await flushSketchMutations();
    expect(engineMock.setSketchSnap).toHaveBeenLastCalledWith(null, false);
  });

  it("an Enter-commit via the live-dim chip clears the indicator", async () => {
    click(0, 0); // first anchor
    move(30, 0); // opens the live-dim chip AND populates a decision
    expect(lastSnapPoint()).not.toBeNull();
    keydown("Enter"); // commits through commitLiveDimGesture → applySteppedClick
    await flushSketchMutations();
    expect(engineMock.setSketchSnap).toHaveBeenLastCalledWith(null, false);
  });

  it("a tool switch clears the indicator", async () => {
    move(5, 0); // populates a decision under the default (line) tool
    expect(lastSnapPoint()).not.toBeNull();
    toolStore.getState().setTool("circle");
    await flush();
    expect(engineMock.setSketchSnap).toHaveBeenLastCalledWith(null, false);
  });

  it("Esc mid-chain (endChainGesture) clears the indicator", async () => {
    click(0, 0); // first anchor — chain armed
    click(50, 0); // commits leg 1, chain stays open
    await flushSketchMutations();
    move(60, 10); // populates a decision for the still-open chain
    expect(lastSnapPoint()).not.toBeNull();
    keydown("Escape"); // ends the chain without committing
    expect(engineMock.setSketchSnap).toHaveBeenLastCalledWith(null, false);
  });
});
