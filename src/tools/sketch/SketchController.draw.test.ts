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
import { viewportStore } from "@/stores/viewportStore";
import { solveDof } from "@/ipc/mockSketch";
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

  // ── W2-B/C: tool-authored constraints reach the session with REAL ids ───────

  it("slot: 3 clicks commit 2 walls + 2 cap arcs welded by Coincident ×4 + Equal ×1", async () => {
    await setTool("slot");
    click(0, 0); // p0
    click(100, 0); // p1 — centerline
    click(50, 20); // width point ⇒ r = 20
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(4);
    expect(ents.map((e) => e.type)).toEqual(["Line", "Line", "Arc", "Arc"]);
    expect(ents[2]).toMatchObject({ center: [100, 0], radius: 20 });
    expect(ents[3]).toMatchObject({ center: [0, 0], radius: 20 });

    const cons = session().constraints;
    const welds = cons.filter((c) => c.type === "Coincident");
    const equals = cons.filter((c) => c.type === "Equal");
    expect(welds).toHaveLength(4);
    expect(equals).toHaveLength(1);
    // W0b: the caps are WELDED, not merely tangent — an entity-level Tangent on
    // an already-welded line/arc pair is first-order redundant (slot header).
    expect(cons.some((c) => c.type === "Tangent")).toBe(false);
    // The index refs resolved to the REAL minted ids of this batch, and each
    // weld carries the arc-endpoint role the marshaller needs.
    const [w1, w2, capP1, capP0] = ents.map((e) => e.id);
    expect(welds.map((c) => [c.entities, c.positions])).toEqual([
      [[w1, capP1], ["End", "End"]],
      [[w2, capP1], ["End", "Start"]],
      [[w1, capP0], ["Start", "Start"]],
      [[w2, capP0], ["Start", "End"]],
    ]);
    expect(equals[0].entities).toEqual([capP1, capP0]);
    // Intra-batch inference is suppressed: no duplicate Coincident/H-V.
    expect(cons).toHaveLength(5);

    // MOCK DOF (mockSketch's coarse table): 2 lines (4 each) + 2 arcs (5 each)
    // = 18 − 4 Coincident (2 each) − 1 Equal = 9. This one AGREES with the real
    // PlaneGCS lane for a worker-authored slot (the arc rules that bind the
    // minted endpoints are DOF-neutral); a slot round-tripped through the Rust
    // typed document reads 14 because the wire duplicates each arc's centre
    // point — pre-existing, tracked separately.
    expect(solveDof(ents, cons).dof).toBe(9);
  });

  it("polygon: commits n lines + a CONSTRUCTION circumcircle with the regularity set", async () => {
    await setTool("polygon");
    click(0, 0); // center
    click(50, 0); // vertex ⇒ r = 50
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(7); // 6 ring lines + circumcircle
    expect(ents.slice(0, 6).every((e) => e.type === "Line" && !e.construction)).toBe(true);
    const circle = ents[6];
    expect(circle).toMatchObject({ type: "Circle", center: [0, 0], construction: true });
    expect(circle.radius).toBeCloseTo(50, 9);

    const cons = session().constraints;
    expect(cons.filter((c) => c.type === "Coincident")).toHaveLength(6);
    expect(cons.filter((c) => c.type === "OnCurve")).toHaveLength(6);
    expect(cons.filter((c) => c.type === "Equal")).toHaveLength(5);
    expect(cons).toHaveLength(17); // nothing else — intra-batch inference dropped
    // Every OnCurve pins a line Start to the circumcircle (entity-level 2nd slot).
    for (const c of cons.filter((x) => x.type === "OnCurve")) {
      expect(c.entities[1]).toBe(circle.id);
      expect(c.positions).toEqual(["Start", ""]);
    }

    // MOCK-ONLY DOF: (4n + 3) − 2n − n − (n − 1) = 4.
    expect(solveDof(ents, cons).dof).toBe(4);
  });

  it("polygon: a digit key changes the side count + the status hint", async () => {
    await setTool("polygon");
    expect(viewportStore.getState().statusHint?.message).toBe("Polygon — 6 sides · 3–9 to change");

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "5", bubbles: true, cancelable: true }));
    expect(viewportStore.getState().statusHint?.message).toBe("Polygon — 5 sides · 3–9 to change");

    click(0, 0);
    click(50, 0);
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(6); // 5 ring lines + circumcircle
    expect(session().constraints.filter((c) => c.type === "Equal")).toHaveLength(4);
    // The count is sticky — the tool re-arms at 5 sides.
    expect(viewportStore.getState().statusHint?.message).toBe("Polygon — 5 sides · 3–9 to change");
  });

  it("point: each click commits a Point; a repeat click auto-constrains Coincident", async () => {
    await setTool("point");
    click(0, 0);
    await flushSketchMutations();
    expect(entities()).toHaveLength(1);
    expect(entities()[0]).toMatchObject({ type: "Point", p0: [0, 0] });

    click(0, 0); // exactly on the first point ⇒ Coincident inferred
    await flushSketchMutations();
    expect(entities()).toHaveLength(2);
    expect(session().constraints.filter((c) => c.type === "Coincident")).toHaveLength(1);
  });

  it("centerRect: center + corner commit the 4 mirrored lines", async () => {
    await setTool("centerRect");
    click(0, 0);
    click(50, 25);
    await flushSketchMutations();

    const ents = entities();
    expect(ents).toHaveLength(4);
    expect(ents[0]).toMatchObject({ type: "Line", p0: [-50, -25], p1: [50, -25] });
    expect(ents[2]).toMatchObject({ type: "Line", p0: [50, 25], p1: [-50, 25] });
    // No tool-authored set ⇒ the standard inference still fires (H/V + corners).
    const cons = session().constraints;
    expect(cons.some((c) => c.type === "Horizontal")).toBe(true);
    expect(cons.some((c) => c.type === "Vertical")).toBe(true);
    expect(cons.some((c) => c.type === "Coincident")).toBe(true);
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
