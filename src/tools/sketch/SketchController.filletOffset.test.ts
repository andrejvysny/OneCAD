/*
 * SketchController — Sketch Fillet + Sketch Offset flows (WP-C T2b, jsdom).
 * Engine + client are faked (no WebGL / no backend); `screenToPlane` is 1:1 so
 * client coords ARE plane coords, and `planePixelWorld` is 1 so the pick reach
 * is 8 world units and the weld tolerance is 0.25.
 *
 *   - Fillet: arming opens the radius chip; a click on a corner authors ONE
 *     upsert that removes both legs and adds trimmed-leg + trimmed-leg + arc,
 *     welded to the arc by two Coincident constraints (and NO Tangent — see the
 *     verb's header for the PlaneGCS-redundancy reason); locked legs refuse;
 *     the two-pick path reaches the same corner; hover ghosts the result.
 *   - Offset: arming opens the distance chip; hover ghosts the parallel chain on
 *     the cursor's side; a click appends the copy with NO new constraints.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchConstraint, SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import type { DraftEntity } from "./toolMachine";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/** An L: two lines welded at the origin, long enough for a radius-5 fillet. */
const L_CORNER: SketchEntity[] = [
  { id: "e1", type: "Line", p0: [0, 0], p1: [100, 0] },
  { id: "e2", type: "Line", p0: [0, 0], p1: [0, 80] },
];

/** CCW rectangle in loop order. */
const RECT: SketchEntity[] = [
  { id: "r1", type: "Line", p0: [0, 0], p1: [100, 0] },
  { id: "r2", type: "Line", p0: [100, 0], p1: [100, 50] },
  { id: "r3", type: "Line", p0: [100, 50], p1: [0, 50] },
  { id: "r4", type: "Line", p0: [0, 50], p1: [0, 0] },
];

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
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
    // Isotropic 1px-per-unit metric: matches `planePixelWorld: 1` above, so a
    // plane distance IS a screen-pixel distance in these tests.
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
          entities: L_CORNER.map((e) => ({ ...e })),
          constraints: [],
          dof: 8,
          status: "UnderConstrained",
        }),
    ),
    cancelSketch: vi.fn(() => Promise.resolve()),
    deleteSketch: vi.fn(() => Promise.resolve()),
    sketchUpsert: vi.fn((sketchId: string) =>
      Promise.resolve({
        sketchId,
        sketchRevision: 1,
        dof: 0,
        status: "UnderConstrained" as const,
        solvedPositions: {},
      }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController — Sketch Fillet + Offset (WP-C T2b)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
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
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    sketchStore.setState({ entitySeq: 10, constraintSeq: 10 });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    vi.unstubAllGlobals();
  });

  function mouse(type: string, x: number, y: number, button: number, buttons: number): void {
    container.dispatchEvent(
      new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true }),
    );
  }
  const click = (x: number, y: number): void => {
    mouse("pointerdown", x, y, 0, 1);
    mouse("pointerup", x, y, 0, 0);
  };
  const hover = (x: number, y: number): void => mouse("pointermove", x, y, -1, 0);

  const seed = (entities: SketchEntity[], constraints: SketchConstraint[] = []): void => {
    const s = sketchStore.getState().session!;
    sketchStore.getState().setSession({ ...s, entities, constraints });
  };
  /** Entities/constraints of the LAST sketchUpsert call. */
  const upserted = (): { entities: SketchEntity[]; constraints: SketchConstraint[] } => {
    const calls = clientMock.sketchUpsert.mock.calls as unknown as [
      string,
      SketchEntity[],
      SketchConstraint[],
    ][];
    const last = calls[calls.length - 1];
    return { entities: last[1], constraints: last[2] };
  };
  const lastPreview = (): DraftEntity[] => {
    const calls = engineMock.setSketchPreview.mock.calls;
    return (calls.length ? calls[calls.length - 1][0] : []) as DraftEntity[];
  };

  // ── Fillet ─────────────────────────────────────────────────────────────────
  describe("Fillet", () => {
    beforeEach(async () => {
      toolStore.getState().setTool("sketchFillet");
      await flush();
    });

    it("arming opens the radius chip seeded at the default", () => {
      const chip = toolChipStore.getState();
      expect(chip.kind).toBe("sketchValue");
      expect(chip.label).toBe("Fillet");
      expect(chip.value).toBe(5);
      expect(viewportStore.getState().statusHint?.message).toContain("Fillet");
    });

    it("a click on the corner replaces both legs with trimmed legs + a tangent arc", async () => {
      click(1, 1); // within the 8-unit pick reach of the (0,0) corner
      await flush();
      expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(1);
      const { entities } = upserted();
      // Both legs gone, three fresh entities in their place.
      expect(entities.map((e) => e.id)).not.toContain("e1");
      expect(entities.map((e) => e.id)).not.toContain("e2");
      expect(entities).toHaveLength(3);
      const arc = entities.find((e) => e.type === "Arc")!;
      expect(arc.radius).toBe(5);
      expect(arc.center![0]).toBeCloseTo(5, 9);
      expect(arc.center![1]).toBeCloseTo(5, 9);
      // Legs trimmed back to the tangent points, far ends untouched.
      const lines = entities.filter((e) => e.type === "Line");
      expect(lines).toHaveLength(2);
      const alongX = lines.find((e) => e.p1![1] === 0)!;
      expect(alongX.p0![0]).toBeCloseTo(5, 9);
      expect(alongX.p1).toEqual([100, 0]);
    });

    it("welds the arc to BOTH legs with Coincident — and authors no Tangent", async () => {
      click(1, 1);
      await flush();
      const { entities, constraints } = upserted();
      const arc = entities.find((e) => e.type === "Arc")!;
      expect(constraints).toHaveLength(2);
      expect(constraints.every((c) => c.type === "Coincident")).toBe(true);
      expect(constraints.every((c) => c.entities.includes(arc.id))).toBe(true);
      // Each weld names a real line endpoint and a real arc endpoint.
      for (const c of constraints) {
        expect(c.positions).toHaveLength(2);
        expect(["Start", "End"]).toContain(c.positions![0]);
        expect(["Start", "End"]).toContain(c.positions![1]);
      }
      expect(constraints.some((c) => c.type === "Tangent")).toBe(false);
    });

    it("commits as ONE sketch undo step", async () => {
      const before = sketchStore.getState().undoStack.length;
      click(1, 1);
      await flush();
      expect(sketchStore.getState().undoStack.length).toBe(before + 1);
      expect(sketchStore.getState().lastUndoPush?.kind).toBe("sketchFillet");
    });

    it("keeps the far-end constraints and drops the corner weld + leg dimension", async () => {
      seed(L_CORNER, [
        { id: "c1", type: "Coincident", entities: ["e1", "e2"], positions: ["Start", "Start"] },
        { id: "c2", type: "Horizontal", entities: ["e1"] },
        { id: "c3", type: "Distance", entities: ["e1"], value: 100 },
        { id: "c4", type: "Vertical", entities: ["e2"] },
      ]);
      click(1, 1);
      await flush();
      const { entities, constraints } = upserted();
      const arc = entities.find((e) => e.type === "Arc")!;
      const kinds = constraints.filter((c) => !c.entities.includes(arc.id)).map((c) => c.type);
      // Horizontal + Vertical survive (remapped); the corner Coincident and the
      // length Distance are dropped.
      expect(kinds.sort()).toEqual(["Horizontal", "Vertical"]);
      // Remapped onto the SUCCESSOR ids, with fresh constraint ids.
      const live = new Set(entities.map((e) => e.id));
      for (const c of constraints) expect(c.entities.every((id) => live.has(id))).toBe(true);
      expect(constraints.some((c) => c.id === "c2")).toBe(false);
    });

    it("drops a constraint anchored at the CONSUMED endpoint but keeps the far one", async () => {
      seed(
        [...L_CORNER, { id: "e3", type: "Line", p0: [100, 0], p1: [100, 60] }],
        [
          // At the far end of e1 — must survive.
          { id: "k1", type: "Coincident", entities: ["e1", "e3"], positions: ["End", "Start"] },
          // At the consumed end of e1 (a stray point weld) — must go.
          { id: "k2", type: "Coincident", entities: ["e1", "e3"], positions: ["Start", "End"] },
        ],
      );
      click(1, 1);
      await flush();
      const { entities, constraints } = upserted();
      const arc = entities.find((e) => e.type === "Arc")!;
      const carried = constraints.filter((c) => !c.entities.includes(arc.id));
      expect(carried).toHaveLength(1);
      expect(carried[0].positions).toEqual(["End", "Start"]);
    });

    it("hovering a corner ghosts the exact result (2 trimmed legs + arc)", () => {
      hover(1, 1);
      const preview = lastPreview();
      expect(preview).toHaveLength(3);
      expect(preview.filter((d) => d.type === "Line")).toHaveLength(2);
      expect(preview.find((d) => d.type === "Arc")?.radius).toBe(5);
    });

    it("hovering nothing clears the ghost, and a miss never upserts", async () => {
      hover(1, 1);
      hover(50, 40); // far from any corner
      expect(lastPreview()).toEqual([]);
      click(50, 40);
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
    });

    it("a chip edit re-parameterises the NEXT fillet", async () => {
      toolChipStore.getState().onValue?.(12);
      expect(toolChipStore.getState().value).toBe(12);
      click(1, 1);
      await flush();
      expect(upserted().entities.find((e) => e.type === "Arc")!.radius).toBe(12);
    });

    it("refuses a radius the corner cannot fit, naming the maximum", async () => {
      seed([
        { id: "s1", type: "Line", p0: [0, 0], p1: [4, 0] },
        { id: "s2", type: "Line", p0: [0, 0], p1: [0, 4] },
      ]);
      click(1, 1);
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
      expect(viewportStore.getState().statusHint?.message).toMatch(/at most/);
    });

    it("refuses LOCKED reference geometry (the fillet mutates both legs)", async () => {
      seed([{ ...L_CORNER[0], referenceLocked: true }, L_CORNER[1]]);
      click(1, 1);
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
      expect(viewportStore.getState().statusHint?.message).toContain("Reference geometry is locked");
    });

    it("TWO-PICK: a line click with no corner under it arms, the partner applies", async () => {
      // (50, 0) is on e1, 50 units from the corner — nothing to auto-resolve.
      click(50, 0);
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
      expect(sketchSelectionStore.getState().selected).toEqual([{ entityId: "e1" }]);
      expect(viewportStore.getState().statusHint?.message).toContain("shares a corner");

      click(0, 40); // on e2, which shares the (0,0) corner with e1
      await flush();
      expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(1);
      expect(upserted().entities.some((e) => e.type === "Arc")).toBe(true);
      expect(sketchSelectionStore.getState().selected).toEqual([]);
    });

    it("Esc cancels a half-made two-pick before the global ladder sees it", () => {
      click(50, 0);
      const ev = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
      window.dispatchEvent(ev);
      expect(ev.defaultPrevented).toBe(true);
      expect(sketchSelectionStore.getState().selected).toEqual([]);
      // A second Esc has nothing left to cancel and falls through.
      const ev2 = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
      window.dispatchEvent(ev2);
      expect(ev2.defaultPrevented).toBe(false);
    });

    it("leaving the tool drops the chip and any pick", async () => {
      click(50, 0);
      toolStore.getState().setTool("select");
      await flush();
      expect(toolChipStore.getState().kind).toBe("none");
      expect(sketchSelectionStore.getState().selected).toEqual([]);
    });
  });

  // ── Offset ─────────────────────────────────────────────────────────────────
  describe("Offset", () => {
    beforeEach(async () => {
      seed(RECT);
      toolStore.getState().setTool("sketchOffset");
      await flush();
    });

    it("arming opens the distance chip seeded at the default", () => {
      expect(toolChipStore.getState().kind).toBe("sketchValue");
      expect(toolChipStore.getState().label).toBe("Offset");
      expect(toolChipStore.getState().value).toBe(5);
    });

    it("hovering INSIDE the loop ghosts the inset copy; outside ghosts the outset", () => {
      hover(50, 3); // just inside the bottom wall
      const inside = lastPreview();
      expect(inside).toHaveLength(4);
      const insideXs = inside.map((d) => d.p0!.x);
      expect(Math.min(...insideXs)).toBeCloseTo(5, 9);

      hover(50, -3); // just outside
      const outside = lastPreview();
      expect(Math.min(...outside.map((d) => d.p0!.x))).toBeCloseTo(-5, 9);
    });

    it("a click appends the parallel chain and authors NO constraints", async () => {
      hover(50, 3);
      click(50, 3);
      await flush();
      expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(1);
      const { entities, constraints } = upserted();
      expect(entities).toHaveLength(8); // 4 originals + 4 copies
      for (const r of RECT) expect(entities.some((e) => e.id === r.id)).toBe(true);
      expect(constraints).toEqual([]); // V1: the copy carries no link to its source
      // The copy is the exact inset rectangle: corners at ±5 off each wall.
      const copies = entities.slice(4);
      expect(copies.every((e) => e.type === "Line")).toBe(true);
      const xs = copies.flatMap((e) => [e.p0![0], e.p1![0]]);
      const ys = copies.flatMap((e) => [e.p0![1], e.p1![1]]);
      expect(Math.min(...xs)).toBeCloseTo(5, 9);
      expect(Math.max(...xs)).toBeCloseTo(95, 9);
      expect(Math.min(...ys)).toBeCloseTo(5, 9);
      expect(Math.max(...ys)).toBeCloseTo(45, 9);
    });

    it("commits as ONE sketch undo step", async () => {
      const before = sketchStore.getState().undoStack.length;
      click(50, 3);
      await flush();
      expect(sketchStore.getState().undoStack.length).toBe(before + 1);
      expect(sketchStore.getState().lastUndoPush?.kind).toBe("sketchOffset");
    });

    it("authors REGULAR geometry even from a construction chain", async () => {
      seed(RECT.map((e) => ({ ...e, construction: true })));
      click(50, 3);
      await flush();
      const copies = upserted().entities.slice(4);
      expect(copies.every((e) => !e.construction)).toBe(true);
    });

    it("a click on empty space is inert", async () => {
      click(50, 25);
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
    });

    it("refuses a distance that would collapse the chain, naming why", async () => {
      toolChipStore.getState().onValue?.(30); // > half the 50-unit height
      click(50, 3);
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
      expect(viewportStore.getState().statusHint?.message).toMatch(/Offset too large/);
    });

    it("leaving the tool drops the chip and the ghost", async () => {
      hover(50, 3);
      toolStore.getState().setTool("select");
      await flush();
      expect(toolChipStore.getState().kind).toBe("none");
      expect(lastPreview()).toEqual([]);
    });
  });
});
