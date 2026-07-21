/*
 * SketchController draw-tool pointer path (jsdom): real pointerdown/pointerup
 * dispatch on the container drives click → snapAt → machine.step → commit for
 * line/rect/circle/arc, mirroring the click-driven flow enterChain.test.ts
 * deferred here. Engine + client are faked (no WebGL / no backend); screenToPlane
 * is 1:1 so client coords ARE plane coords, and every frontend snap type is
 * disabled (settingsStore.snapTo) so raw click coords pass through computeSnap
 * unchanged — snapping itself is covered by snapEngine.test.ts /
 * SketchController.snapCache.test.ts, this file's job is the pointer→commit wire.
 * Commits are queued (enqueueSketchMutation); every test drains the queue via
 * `flushSketchMutations` before asserting the session.
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
    // Identity solve (mirrors the mock lane): echoes entities/constraints back,
    // no movement, so applySolvedPositions is a no-op.
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

// Disable every frontend snap type so computeSnap returns the raw point
// unchanged — this file exercises the pointer→commit wire, not snapping.
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

describe("SketchController draw tools (pointer path)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
    disableSnapping();
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

  function click(x: number, y: number): void {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
    );
  }

  async function setTool(tool: SketchTool): Promise<void> {
    toolStore.getState().setTool(tool);
    await flush();
  }

  const session = (): SketchSession => sketchStore.getState().session!;
  const entities = (): SketchEntity[] => session().entities;

  type Internals = { machineState: { anchors: Array<{ x: number; y: number }> } | null };
  const internals = (): Internals => controller as unknown as Internals;

  it("line: 3 clicks commit 2 segments, auto-constrained H/V, undo stack grows", async () => {
    expect(sketchStore.getState().undoStack).toHaveLength(0);

    click(0, 0); // first anchor — no commit
    click(50, 0); // commits the horizontal segment
    click(50, 50); // commits the vertical segment
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(2);
    expect(ents[0]).toMatchObject({ type: "Line", p0: [0, 0], p1: [50, 0] });
    expect(ents[1]).toMatchObject({ type: "Line", p0: [50, 0], p1: [50, 50] });

    const cons = session().constraints;
    expect(cons.some((c) => c.type === "Horizontal")).toBe(true);
    expect(cons.some((c) => c.type === "Vertical")).toBe(true);

    expect(sketchStore.getState().undoStack.length).toBeGreaterThan(0);
  });

  it("rect: 2 corner clicks commit 4 lines", async () => {
    await setTool("rect");
    click(0, 0);
    click(50, 50);
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(4);
    expect(ents.every((e) => e.type === "Line")).toBe(true);
  });

  it("circle: 2 clicks (center, edge) commit 1 circle with the radius to that edge", async () => {
    await setTool("circle");
    click(0, 0);
    click(30, 40); // 3-4-5 triangle scaled ×10 ⇒ radius 50
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(1);
    expect(ents[0].type).toBe("Circle");
    expect(ents[0].center).toEqual([0, 0]);
    expect(ents[0].radius).toBeCloseTo(50, 9);
  });

  it("arc: 3 clicks (center, start, end) commit 1 arc", async () => {
    await setTool("arc");
    click(0, 0);
    click(50, 0);
    click(0, 50);
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(1);
    expect(ents[0].type).toBe("Arc");
    expect(ents[0].center).toEqual([0, 0]);
    expect(ents[0].radius).toBeCloseTo(50, 9);
    expect(ents[0].start).toEqual([50, 0]);
    expect(ents[0].end?.[0]).toBeCloseTo(0, 9);
    expect(ents[0].end?.[1]).toBeCloseTo(50, 9);
  });

  it("circle degenerate: a same-point 2nd click is a no-op (armed); a 3rd click at a real radius commits", async () => {
    await setTool("circle");
    click(0, 0); // center anchor
    click(0, 0); // degenerate — radius 0 < minSize, no commit
    await flushSketchMutations();

    expect(entities()).toHaveLength(0);
    expect(internals().machineState?.anchors).toEqual([{ x: 0, y: 0 }]); // still armed on center

    click(30, 40); // real radius — commits
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(1);
    expect(ents[0].radius).toBeCloseTo(50, 9);
  });

  it("chain close: clicking back on the first anchor closes the loop", async () => {
    click(0, 0); // A — first anchor
    click(50, 0); // B — commits A→B
    click(50, 50); // C — commits B→C
    click(0, 0); // back to A — closes the loop, commits C→A
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(3);
    expect(ents[2]).toMatchObject({ p0: [50, 50], p1: [0, 0] });
    expect(internals().machineState?.anchors).toEqual([]); // chain done
  });

  it("Enter mid-chain ends it without a stray commit", async () => {
    click(0, 0); // A
    click(50, 0); // commits A→B (chain stays open)
    await flushSketchMutations();
    expect(entities()).toHaveLength(1);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushSketchMutations();

    expect(entities()).toHaveLength(1); // unchanged — no stray commit
    expect(internals().machineState?.anchors).toEqual([]);
  });

  it("minSize scales with planePixelWorld (screen-constant reject radius)", async () => {
    // Default planePixelWorld=1 ⇒ minSize=4 world units; a 5-unit gap commits.
    click(0, 0);
    click(5, 0);
    await flushSketchMutations();
    expect(entities()).toHaveLength(1);

    // Zoom out (coarser world-per-pixel) ⇒ minSize=8; the SAME 5-unit gap from
    // the still-open chain's last anchor is now sub-threshold — treated as the
    // same point, no 2nd segment.
    engineMock.planePixelWorld.mockReturnValue(2);
    click(10, 0);
    await flushSketchMutations();
    expect(entities()).toHaveLength(1); // unchanged
  });
});
