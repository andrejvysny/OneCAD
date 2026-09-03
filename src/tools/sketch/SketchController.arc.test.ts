/*
 * SketchController × the line tool's TANGENT-ARC mode (SP-4 W3, jsdom).
 *
 * The pure halves are covered elsewhere — `arcMath` owns the geometry,
 * `toolMachine` owns the FSM, `autoConstrain` owns the weld gate. What is under
 * test HERE is only what the controller decides:
 *   - the plain-`A` capture SHADOW and exactly when it claims the key (armed on a
 *     known tangent, no modifiers — and ⇧A must always reach the arc3p tool),
 *   - the DRAG-to-arc gesture: a drag off the last anchor promotes the mode, and
 *     its release commits instead of being discarded as a non-click,
 *   - the chain SEED (`entityEndTangent` off existing geometry) that makes arc
 *     mode reachable on the first leg of a chain started on a drawn endpoint,
 *   - that the committed batch WELDS (Coincident) and authors NO entity-level
 *     Tangent — the redundancy PlaneGCS would report as OverConstrained.
 *
 * Harness mirrors SketchController.liveDim.test.ts: faked engine + client, 1:1
 * screenToPlane so client coords ARE plane coords, snapping off, rAF inline,
 * commits drained through `flushSketchMutations`.
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
import { sketchStore } from "@/stores/sketchStore";
import { settingsStore } from "@/stores/settingsStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import { flushSketchMutations } from "./sketchService";
import type { ToolState } from "./toolMachine";

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
    setSketchProjectedIds: vi.fn(),
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

/** Every frontend snap type off — the anchors below are already exact. */
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

describe("SketchController — tangent-arc mode (SP-4 W3)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
    disableSnapping();
    // The pointer-move lane is rAF-coalesced; run it inline so a move is
    // observable on the next line.
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

  // ── pointer / keyboard primitives ─────────────────────────────────────────

  const down = (x: number, y: number): void => {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }),
    );
  };
  const up = (x: number, y: number): void => {
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }),
    );
  };
  const click = (x: number, y: number): void => {
    down(x, y);
    up(x, y);
  };
  /** A move with LMB HELD (the drag lane) unless `buttons` says otherwise. */
  const move = (x: number, y: number, buttons = 0): void => {
    container.dispatchEvent(new MouseEvent("pointermove", { clientX: x, clientY: y, buttons, bubbles: true }));
  };

  /** A key typed over the VIEWPORT (target = window, never an input). */
  function type(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
    const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
    window.dispatchEvent(e);
    return e;
  }

  type Internals = { machineState: ToolState | null };
  const machineState = (): ToolState => (controller as unknown as Internals).machineState!;
  const arcMode = (): boolean | undefined => machineState().arcMode;
  const session = (): SketchSession => sketchStore.getState().session!;
  const entities = (): SketchEntity[] => session().entities;
  const hint = (): string | undefined => viewportStore.getState().statusHint?.message;
  const lastUpsertConstraints = (): SketchConstraint[] => {
    const calls = clientMock.sketchUpsert.mock.calls;
    return calls[calls.length - 1][2];
  };

  /** Draw one horizontal leg (0,0) → (40,0); the chain stays armed on (40,0)
   *  with `chainTangent` = +U, which is what makes arc mode reachable. */
  async function chainOneLeg(): Promise<void> {
    click(0, 0);
    click(40, 0);
    await flushSketchMutations();
  }

  // ── the plain-`A` capture shadow ──────────────────────────────────────────

  describe("the plain-`A` shadow", () => {
    it("claims the key mid-chain and flips arc mode", async () => {
      await chainOneLeg();
      const e = type("a");
      expect(arcMode()).toBe(true);
      expect(e.defaultPrevented).toBe(true);
      // …and flips back.
      type("a");
      expect(arcMode()).toBe(false);
    });

    it("PASSES THROUGH with no anchors placed — nothing to be tangent to", () => {
      const e = type("a");
      expect(e.defaultPrevented).toBe(false);
      expect(machineState().arcMode).toBeUndefined();
    });

    it("PASSES THROUGH on the first anchor of a chain drawn in empty space", () => {
      // One anchor, no committed leg, and no entity ends there ⇒ no seed tangent.
      click(0, 0);
      const e = type("a");
      expect(e.defaultPrevented).toBe(false);
      expect(machineState().arcMode).toBeUndefined();
    });

    it("⇧A mid-chain does NOT toggle — that chord belongs to the arc3p tool", async () => {
      await chainOneLeg();
      const e = type("A", { shiftKey: true });
      expect(e.defaultPrevented).toBe(false);
      expect(machineState().arcMode).toBeUndefined();
    });

    it("a modified `A` (⌘/^/⌥) is never the arc verb", async () => {
      await chainOneLeg();
      for (const init of [{ metaKey: true }, { ctrlKey: true }, { altKey: true }]) {
        const e = type("a", init);
        expect(e.defaultPrevented).toBe(false);
      }
      expect(machineState().arcMode).toBeUndefined();
    });

    it("is inert under a non-line draw tool", async () => {
      toolStore.getState().setTool("rect");
      await flush();
      click(0, 0);
      const e = type("a");
      expect(e.defaultPrevented).toBe(false);
    });
  });

  // ── the chain SEED off existing geometry ──────────────────────────────────

  it("seeds the tangent off an EXISTING entity endpoint the chain starts on", async () => {
    await chainOneLeg();
    type("Escape"); // end the chain; the drawn line stays in the session
    expect(entities()).toHaveLength(1);

    // A fresh chain whose FIRST anchor is that line's end: the machine has no
    // chainTangent of its own, so the controller's `entityEndTangent` seed is the
    // only thing that can make arc mode reachable — and it does.
    click(40, 0);
    expect(machineState().anchors).toHaveLength(1);
    const e = type("a");
    expect(e.defaultPrevented).toBe(true);
    expect(arcMode()).toBe(true);
  });

  it("refuses to guess at an AMBIGUOUS junction (two entities end there)", async () => {
    // An L: (0,0)→(40,0)→(40,40). Restarting a chain at the shared corner (40,0)
    // has TWO candidate directions, so `entityEndTangent` returns null.
    click(0, 0);
    click(40, 0);
    click(40, 40);
    await flushSketchMutations();
    type("Escape");
    expect(entities()).toHaveLength(2);

    click(40, 0);
    const e = type("a");
    expect(e.defaultPrevented).toBe(false);
    expect(machineState().arcMode).toBeUndefined();
  });

  // ── the DRAG gesture ──────────────────────────────────────────────────────

  describe("drag-to-arc", () => {
    it("a drag off the last anchor promotes arc mode and its release COMMITS the arc", async () => {
      await chainOneLeg();
      down(40, 0);
      move(50, 5, 1); // past DRAG_PX with LMB held ⇒ promote
      expect(arcMode()).toBe(true);
      move(80, 40, 1);
      up(80, 40);
      await flushSketchMutations();

      const arc = entities()[1];
      expect(arc.type).toBe("Arc");
      // Tangency at the shared corner: the radius there is perpendicular to +U.
      expect(arc.center![0]).toBeCloseTo(40, 9);
      // The chain kept going, and the mode auto-cleared (one arc per gesture).
      expect(machineState().anchors).toHaveLength(3);
      expect(arcMode()).toBe(false);
    });

    it("a plain click (no drag) still commits a straight segment", async () => {
      await chainOneLeg();
      click(80, 0);
      await flushSketchMutations();
      expect(entities()[1].type).toBe("Line");
      expect(machineState().arcMode).toBeUndefined();
    });

    it("does not promote a drag with no tangent to leave along", () => {
      click(0, 0);
      down(0, 0);
      move(30, 30, 1);
      expect(machineState().arcMode).toBeUndefined();
    });

    it("does not promote a drag under a non-line tool", async () => {
      toolStore.getState().setTool("rect");
      await flush();
      click(0, 0);
      down(0, 0);
      move(30, 30, 1);
      expect(machineState().arcMode).toBeUndefined();
    });
  });

  // ── what the commit AUTHORS ───────────────────────────────────────────────

  it("welds the arc to the line with Coincident — and authors NO Tangent", async () => {
    await chainOneLeg();
    type("a");
    click(80, 40);
    await flushSketchMutations();

    expect(entities()[1].type).toBe("Arc");
    const cs = lastUpsertConstraints();
    const weld = cs.find((c) => c.type === "Coincident");
    expect(weld).toBeDefined();
    expect(weld!.entities).toEqual([entities()[1].id, entities()[0].id]);
    // The weld makes an entity-level Tangent(line, arc) redundant in PlaneGCS —
    // authoring it would render as a false OverConstrained (worker ctest
    // test_endpoint_weld_makes_entity_tangent_redundant).
    expect(cs.some((c) => c.type === "Tangent")).toBe(false);
  });

  // ── the 3-state hint ──────────────────────────────────────────────────────

  it("the status hint walks idle → arcable → arc mode", async () => {
    expect(hint()).toContain("Line — click to place");
    await chainOneLeg();
    expect(hint()).toContain("drag or press A for a tangent arc");
    type("a");
    expect(hint()).toContain("Tangent arc — click to place");
    type("a");
    expect(hint()).toContain("drag or press A for a tangent arc");
  });

  it("ending the chain drops the arc affordance from the hint", async () => {
    await chainOneLeg();
    type("Escape");
    expect(hint()).toContain("Line — click to place");
  });
});
