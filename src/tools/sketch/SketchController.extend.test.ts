/*
 * SketchController Extend tool flow (jsdom). Engine + client are faked (no WebGL /
 * no backend); screenToPlane is 1:1 so client coords ARE plane coords.
 *   - arming publishes the sticky Extend hint;
 *   - hover ghosts the span the click would GAIN on the ADDITIVE preview lane and
 *     NEVER on the destructive trim-ghost lane;
 *   - a click commits one sketchUpsert with the target replaced by a fresh id;
 *   - a miss is inert (no upsert, ghost cleared) and an entity with nowhere to
 *     grow says so instead of failing silently;
 *   - locked reference geometry is refused, loudly, before any upsert.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchConstraint, SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/** e1: the target segment. e2: the wall its end can grow out to at x=20. */
const ENTITIES: SketchEntity[] = [
  { id: "e1", type: "Line", p0: [0, 0], p1: [10, 0] },
  { id: "e2", type: "Line", p0: [20, -50], p1: [20, 50] },
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
    setSketchSnap: vi.fn(),
    updateSketchSession: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
  };
}

function makeClientMock(entities: SketchEntity[]) {
  return {
    enterSketch: vi.fn(
      (): Promise<SketchSession> =>
        Promise.resolve({
          sketchId: "sketch1",
          plane: PLANE,
          entities: entities.map((e) => ({ ...e })),
          constraints: [],
          dof: 4,
          status: "UnderConstrained",
        }),
    ),
    cancelSketch: vi.fn(() => Promise.resolve()),
    deleteSketch: vi.fn(() => Promise.resolve()),
    sketchUpsert: vi.fn((sketchId: string, _e: SketchEntity[], _c: SketchConstraint[]) =>
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

describe("SketchController — Extend", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  async function mount(entities: SketchEntity[]): Promise<void> {
    engineMock = makeEngineMock();
    clientMock = makeClientMock(entities);
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    sketchStore.setState({ entitySeq: 2, constraintSeq: 0 });
    toolStore.getState().setTool("extend");
    await flush();
  }

  beforeEach(() => {
    resetStores();
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return 1;
    });
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
  /** Last argument handed to the ADDITIVE preview channel. */
  const lastPreview = (): unknown => {
    const calls = engineMock.setSketchPreview.mock.calls;
    return calls.length ? calls[calls.length - 1][0] : undefined;
  };

  describe("over ordinary geometry", () => {
    beforeEach(() => mount(ENTITIES));

    it("shows the sticky Extend hint on arming", () => {
      expect(viewportStore.getState().statusHint?.message).toContain("Extend —");
      expect(viewportStore.getState().statusHint?.sticky).toBe(true);
    });

    it("hover ghosts ONLY the gained span, on the preview lane — never the trim ghost", () => {
      engineMock.setSketchTrimGhost.mockClear();
      mouse("pointermove", 9, 0, 0, 0); // idle hover over e1's far half
      const drafts = lastPreview() as { type: string; p0: { x: number }; p1: { x: number } }[];
      expect(drafts).toHaveLength(1);
      expect(drafts[0].type).toBe("Line");
      expect(drafts[0].p0.x).toBeCloseTo(10, 9); // the OLD end …
      expect(drafts[0].p1.x).toBeCloseTo(20, 9); // … out to the wall
      // The destructive channel must stay untouched: nothing is being removed.
      expect(engineMock.setSketchTrimGhost).not.toHaveBeenCalled();
    });

    it("a miss clears the ghost", () => {
      mouse("pointermove", 9, 0, 0, 0); // ghost set
      mouse("pointermove", 300, 300, 0, 0); // empty space
      expect(lastPreview()).toEqual([]);
    });

    it("a click commits ONE upsert with the target REPLACED by a fresh id", async () => {
      click(9, 0);
      await flush();
      expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(1);
      const [, entities] = clientMock.sketchUpsert.mock.calls[0];
      const sent = entities as SketchEntity[];
      expect(sent).toHaveLength(2);
      expect(sent.map((e) => e.id)).not.toContain("e1"); // id changed ⇒ the wire sees it
      const grown = sent.find((e) => e.id !== "e2")!;
      expect(grown.p0).toEqual([0, 0]);
      expect(grown.p1![0]).toBeCloseTo(20, 9);
    });

    it("a click on empty space is inert (no upsert)", async () => {
      click(300, 300);
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
    });

    it("an end that reaches nothing says so instead of failing silently", async () => {
      // e2's own top end continues to x=20, y>50 — e1 stops at x=10, so the
      // continuation meets nothing that is actually drawn there.
      click(20, 45);
      await flush();
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
      expect(viewportStore.getState().statusHint?.message).toMatch(/Nothing to extend to/);
    });

    it("switching away from Extend clears the ghost", () => {
      mouse("pointermove", 9, 0, 0, 0);
      engineMock.setSketchPreview.mockClear();
      toolStore.getState().setTool("select");
      expect(engineMock.setSketchPreview).toHaveBeenCalledWith([]);
    });
  });

  describe("over LOCKED reference geometry", () => {
    // ref1: a projected host-face segment (locked). e3: the user's own copy of it
    // one unit up, with the same wall to grow out to.
    beforeEach(() =>
      mount([
        { ...ENTITIES[0], id: "ref1", referenceLocked: true },
        ENTITIES[1],
        { id: "e3", type: "Line", p0: [0, 20], p1: [10, 20] },
      ]),
    );

    it("REFUSES the extend and says why (no upsert)", async () => {
      click(9, 0); // on ref1
      await flush();
      await flush();
      expect(clientMock.sketchUpsert).not.toHaveBeenCalled();
      expect(viewportStore.getState().statusHint?.message).toMatch(
        /Reference geometry is locked.*cannot be extended/,
      );
      expect(sketchStore.getState().session!.entities).toHaveLength(3);
    });

    it("still extends the UNLOCKED entity in the same session", async () => {
      click(9, 20); // on e3
      await flush();
      expect(clientMock.sketchUpsert).toHaveBeenCalledTimes(1);
    });
  });
});
