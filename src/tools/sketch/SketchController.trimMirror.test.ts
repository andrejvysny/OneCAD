/*
 * SketchController Trim + Mirror tool flow (jsdom). Engine + client are faked (no
 * WebGL / no backend); screenToPlane is 1:1 so client coords ARE plane coords.
 *   - Trim: a click on an entity → deleteEntities → one sketchUpsert with the
 *     entity removed; a miss → no upsert.
 *   - Mirror: Phase A click selects (empty selection); Phase B axis pick (a LINE
 *     body not in the set) → one sketchUpsert with session + mirrored copies and
 *     the new Symmetric constraints; an axis IN the selection is rejected (no-op).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchConstraint, SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

// e1: source line (y=10); e2: horizontal mirror axis (the X axis).
const ENTITIES: SketchEntity[] = [
  { id: "e1", type: "Line", p0: [0, 10], p1: [20, 10] },
  { id: "e2", type: "Line", p0: [0, 0], p1: [100, 0] },
];

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
          entities: ENTITIES.map((e) => ({ ...e })),
          constraints: [],
          dof: 4,
          status: "UnderConstrained",
        }),
    ),
    cancelSketch: vi.fn(() => Promise.resolve()),
    deleteSketch: vi.fn(() => Promise.resolve()),
    sketchUpsert: vi.fn(
      (sketchId: string, _e: SketchEntity[], _c: SketchConstraint[]) =>
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

describe("SketchController — Trim + Mirror", () => {
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
    // Advance the id counters past the seeded e1/e2 so mirrored ids don't collide.
    sketchStore.setState({ entitySeq: 2, constraintSeq: 0 });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    vi.unstubAllGlobals();
  });

  function mouse(
    type: string,
    x: number,
    y: number,
    button: number,
    buttons: number,
    mods?: { shiftKey?: boolean; metaKey?: boolean },
  ): void {
    container.dispatchEvent(
      new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true, ...mods }),
    );
  }
  const click = (x: number, y: number, mods?: { shiftKey?: boolean }): void => {
    mouse("pointerdown", x, y, 0, 1, mods);
    mouse("pointerup", x, y, 0, 0, mods);
  };
  const selected = () => sketchSelectionStore.getState().selected;

  // ── Trim ──────────────────────────────────────────────────────────────────
  describe("Trim", () => {
    beforeEach(async () => {
      toolStore.getState().setTool("trim");
      await flush();
    });

    it("a click on an entity deletes it (one upsert, entity removed)", async () => {
      click(10, 10); // on the e1 line body (midpoint region)
      await flush();
      expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(1);
      const [, entities] = clientMock.sketchUpsert.mock.calls[0];
      expect((entities as SketchEntity[]).map((e) => e.id)).toEqual(["e2"]); // e1 gone
      expect(sketchStore.getState().session!.entities.map((e) => e.id)).toEqual(["e2"]);
    });

    it("a click on empty space is a no-op (no upsert)", async () => {
      click(300, 300);
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
    });

    it("shows the trim hint on arming", () => {
      expect(viewportStore.getState().statusHint?.message).toContain("Click an entity to delete");
    });

    it("idle mouse move does NOT set hover (trim hover ships in a later wave)", () => {
      mouse("pointermove", 10, 10, 0, 0); // idle move over the e1 line body
      expect(sketchSelectionStore.getState().hover).toBeNull();
    });
  });

  // ── Mirror ────────────────────────────────────────────────────────────────
  describe("Mirror", () => {
    beforeEach(async () => {
      toolStore.getState().setTool("mirror");
      await flush();
    });

    it("Phase A: clicks with an empty selection pick entities to mirror", () => {
      expect(selected()).toEqual([]); // starts empty (Phase A)
      click(10, 10); // e1 body
      expect(selected()).toEqual([{ entityId: "e1" }]);
    });

    it("Phase B: an axis pick (a line not in the set) mirrors — one upsert, expected arrays", async () => {
      click(10, 10); // Phase A → select e1
      expect(selected()).toEqual([{ entityId: "e1" }]);
      click(50, 0); // Phase B → pick e2 as the mirror axis
      await flush();

      expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(1);
      const [, entities, constraints] = clientMock.sketchUpsert.mock.calls[0] as [
        string,
        SketchEntity[],
        SketchConstraint[],
      ];
      // session (e1,e2) + one mirrored copy of e1.
      expect(entities.map((e) => e.id)).toEqual(["e1", "e2", "e3"]);
      const copy = entities[2];
      expect(copy.type).toBe("Line");
      expect(copy.p0).toEqual([0, -10]); // reflected across the X axis
      expect(copy.p1).toEqual([20, -10]);
      // Two Symmetric constraints tying e1↔e3 across the axis e2.
      expect(constraints.map((c) => c.type)).toEqual(["Symmetric", "Symmetric"]);
      expect(constraints[0]).toMatchObject({ entities: ["e1", "e3", "e2"], positions: ["Start", "Start"] });
      expect(constraints[1]).toMatchObject({ entities: ["e1", "e3", "e2"], positions: ["End", "End"] });
      // Selection switched to the mirrored copy; tool stays in mirror (repeatable).
      expect(selected()).toEqual([{ entityId: "e3" }]);
      expect(toolStore.getState().sketchTool).toBe("mirror");
    });

    it("rejects an axis that is IN the selection (no-op, no upsert)", async () => {
      click(50, 0); // Phase A → select e2 (the intended axis line)
      expect(selected()).toEqual([{ entityId: "e2" }]);
      click(50, 0); // Phase B → e2 is in the set → rejected
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
    });

    it("switching away from mirror clears the in-progress pick set", () => {
      click(10, 10);
      expect(selected()).toHaveLength(1);
      toolStore.getState().setTool("select");
      expect(selected()).toEqual([]);
    });

    it("idle mouse move sets sketchSelectionStore hover (on hit, cleared on miss)", () => {
      mouse("pointermove", 10, 10, 0, 0); // idle move over the e1 line body
      expect(sketchSelectionStore.getState().hover).toEqual({ entityId: "e1" });
      mouse("pointermove", 300, 300, 0, 0); // empty space
      expect(sketchSelectionStore.getState().hover).toBeNull();
    });

    it("Esc (global ladder) returns to the select tool", async () => {
      click(10, 10);
      // The controller does not intercept Esc for mirror; the global shortcut ladder
      // resets the tool. Simulate that reset directly (useShortcuts owns the listener).
      toolStore.getState().setTool("select");
      await flush();
      expect(toolStore.getState().sketchTool).toBe("select");
      expect(selected()).toEqual([]);
    });
  });
});
