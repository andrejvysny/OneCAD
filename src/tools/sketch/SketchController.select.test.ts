/*
 * SketchController select-tool flow (jsdom): click-select (plain / shift toggle /
 * miss-clears) and point-handle drag through the gesture lane (begin → solve → end),
 * plus Esc-cancel restoring pre-drag geometry. Engine + client are faked (no WebGL /
 * no backend); screenToPlane is 1:1 so client coords ARE plane coords.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  DragSolveResult,
  SketchEntity,
  SketchPlane,
  SketchSession,
  SketchUpsertResult,
} from "@/ipc/types";
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

const ENTITIES: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }];

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
    // 1:1 mapping so client coords equal plane coords.
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
    // Isotropic 1px-per-unit metric: matches `planePixelWorld: 1` above, so a
    // plane distance IS a screen-pixel distance in these tests.
    planeScreenMetric: vi.fn(() => ({ m00: 1, m01: 0, m10: 0, m11: 1 })),
  };
}

function makeClientMock() {
  let seq = 0;
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
    beginGesture: vi.fn(() => Promise.resolve({ gestureId: 1, ready: true })),
    // Return types are stated rather than inferred: SP-2 added the optional
    // `curves`/`solvedCurves` channels, and a literal-inferred shape would refuse
    // any per-test override that reports a radius instead of a position.
    solveDrag: vi.fn(
      (target: [number, number]): Promise<DragSolveResult> =>
        Promise.resolve({
          gestureId: 1,
          seq: ++seq,
          status: "success",
          dof: 2,
          conflicting: [],
          positions: { "e1.Start": target },
          curves: {},
          solveMicros: 0,
          superseded: false,
        }),
    ),
    endGesture: vi.fn(
      (target?: [number, number]): Promise<SketchUpsertResult> =>
        Promise.resolve({
          sketchId: "sketch1",
          sketchRevision: 1,
          dof: 2,
          status: "UnderConstrained",
          solvedPositions: (target ? { "e1.Start": target } : {}) as Record<
            string,
            [number, number]
          >,
        }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController select tool", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
    // rAF fires synchronously so drag-solve scheduling is deterministic.
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
    // Enter an existing sketch, then arm the select tool.
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    toolStore.getState().setTool("select");
    await flush();
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    vi.unstubAllGlobals();
  });

  function mouse(type: string, x: number, y: number, button: number, buttons: number, mods?: { shiftKey?: boolean; metaKey?: boolean }): void {
    container.dispatchEvent(
      new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true, ...mods }),
    );
  }

  const selected = () => sketchSelectionStore.getState().selected;

  it("a click on a vertex selects that point handle", () => {
    mouse("pointerdown", 0, 0, 0, 1);
    mouse("pointerup", 0, 0, 0, 0);
    expect(selected()).toEqual([{ entityId: "e1", point: "Start" }]);
  });

  it("a click on empty space clears the selection", () => {
    sketchSelectionStore.getState().set([{ entityId: "e1", point: "Start" }]);
    mouse("pointerdown", 200, 200, 0, 1);
    mouse("pointerup", 200, 200, 0, 0);
    expect(selected()).toEqual([]);
  });

  it("Shift-click toggles the second handle into the selection", () => {
    mouse("pointerdown", 0, 0, 0, 1); // plain click → Start
    mouse("pointerup", 0, 0, 0, 0);
    mouse("pointerdown", 40, 0, 0, 1, { shiftKey: true }); // shift-click → +End
    mouse("pointerup", 40, 0, 0, 0, { shiftKey: true });
    expect(selected()).toEqual([
      { entityId: "e1", point: "Start" },
      { entityId: "e1", point: "End" },
    ]);
    // Shift-click End again toggles it off.
    mouse("pointerdown", 40, 0, 0, 1, { shiftKey: true });
    mouse("pointerup", 40, 0, 0, 0, { shiftKey: true });
    expect(selected()).toEqual([{ entityId: "e1", point: "Start" }]);
  });

  it("dragging a point handle runs begin → solve → end and keeps the selection", async () => {
    mouse("pointerdown", 0, 0, 0, 1); // arm on Start
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(true); // orbit suppressed
    mouse("pointermove", 30, 0, 0, 1); // past DRAG_PX → begin gesture
    await flush();

    // SP-2: a point drag now declares its target kind. The point ref is unchanged,
    // so the wire request stays byte-identical to the pre-SP-2 one (SCHEMA §7.4).
    expect(clientMock.beginGesture).toHaveBeenCalledWith(
      "sketch1",
      "e1.Start",
      expect.objectContaining({ kind: "point", entityId: "e1.Start", role: "Start" }),
    );
    expect(clientMock.solveDrag).toHaveBeenCalledWith([30, 0]);
    // The live preview moved Start via updateSketchSession (End untouched).
    const calls = engineMock.updateSketchSession.mock.calls;
    const preview = calls[calls.length - 1][1] as SketchEntity[];
    expect(preview[0].p0).toEqual([30, 0]);
    expect(preview[0].p1).toEqual([40, 0]);

    mouse("pointerup", 30, 0, 0, 0);
    await flush();

    expect(clientMock.endGesture).toHaveBeenCalledTimes(1);
    expect(clientMock.endGesture).toHaveBeenCalledWith([30, 0]);
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(false); // orbit restored
    // Committed to the session; selection stays on the dragged entity.
    expect(sketchStore.getState().session!.entities[0].p0).toEqual([30, 0]);
    expect(selected()).toEqual([{ entityId: "e1", point: "Start" }]);
  });

  // SP-0 D3 made `beginGesture` refusable at will: the backend now fences a drag
  // against an in-flight `getSketchRegions` on the same sketch (recoverable, "…
  // retry"). That turns a once-exotic rejection into an ordinary one, so the
  // failure path has to leave NOTHING armed — a stuck `dragArmed`/`dragStarting`
  // would swallow every later drag and never suppress-then-restore orbit again.
  it("a refused beginGesture degrades cleanly and the next drag still works", async () => {
    clientMock.beginGesture.mockImplementationOnce(() =>
      Promise.reject(new Error("beginGesture: a region query is in flight for sketch s1 — retry")),
    );

    mouse("pointerdown", 0, 0, 0, 1); // arm on Start
    mouse("pointermove", 30, 0, 0, 1); // past DRAG_PX → begin gesture (rejects)
    await flush();

    expect(clientMock.beginGesture).toHaveBeenCalledTimes(1);
    expect(clientMock.solveDrag).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toMatch(/Drag failed/);
    expect(engineMock.setSketchDrawingActive).toHaveBeenLastCalledWith(false); // orbit restored

    mouse("pointerup", 30, 0, 0, 0);
    await flush();
    expect(clientMock.endGesture).not.toHaveBeenCalled(); // no gesture to close
    expect(sketchStore.getState().session!.entities[0].p0).toEqual([0, 0]); // untouched

    // Not wedged: the very next drag on the same handle opens normally.
    mouse("pointerdown", 0, 0, 0, 1);
    mouse("pointermove", 30, 0, 0, 1);
    await flush();
    expect(clientMock.beginGesture).toHaveBeenCalledTimes(2);
    expect(clientMock.solveDrag).toHaveBeenCalledWith([30, 0]);
  });

  it("accumulates incremental solve deltas — a point omitted from a later response stays moved", async () => {
    // Real-worker shape: each response carries only points changed since the
    // PREVIOUS response. Response 1 moves Start AND drags coupled End; response 2
    // moves Start only — End must NOT snap back to its pre-drag pose.
    const responses: Record<string, [number, number]>[] = [
      { "e1.Start": [10, 0], "e1.End": [50, 0] },
      { "e1.Start": [20, 0] },
    ];
    let seq = 0;
    clientMock.solveDrag.mockImplementation(() =>
      Promise.resolve({
        gestureId: 1,
        seq: ++seq,
        status: "success",
        dof: 2,
        conflicting: [],
        positions: responses[Math.min(seq - 1, responses.length - 1)],
        solveMicros: 0,
        superseded: false,
      }),
    );
    mouse("pointerdown", 0, 0, 0, 1);
    mouse("pointermove", 10, 0, 0, 1);
    await flush();
    mouse("pointermove", 20, 0, 0, 1);
    await flush();

    const calls = engineMock.updateSketchSession.mock.calls;
    const preview = calls[calls.length - 1][1] as SketchEntity[];
    expect(preview[0].p0).toEqual([20, 0]); // latest delta
    expect(preview[0].p1).toEqual([50, 0]); // response-1 coupled move retained
  });

  it("idle pointer moves drive hover (set on hit, cleared on miss)", () => {
    mouse("pointermove", 0, 0, 0, 0); // no button — over e1.Start
    expect(sketchSelectionStore.getState().hover).toEqual({ entityId: "e1", point: "Start" });
    mouse("pointermove", 200, 200, 0, 0); // empty space
    expect(sketchSelectionStore.getState().hover).toBeNull();
  });

  it("Esc mid-drag ends the gesture and restores pre-drag geometry", async () => {
    mouse("pointerdown", 0, 0, 0, 1);
    mouse("pointermove", 30, 0, 0, 1);
    await flush(); // dragging live, preview moved to [30,0]

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flush();

    // Ended at the ORIGINAL position (no explicit cancel verb on the wire).
    expect(clientMock.endGesture).toHaveBeenCalledWith([0, 0]);
    // Geometry restored to pre-drag.
    expect(sketchStore.getState().session!.entities[0].p0).toEqual([0, 0]);
  });

  it("switching away from select clears the sketch selection", () => {
    mouse("pointerdown", 0, 0, 0, 1);
    mouse("pointerup", 0, 0, 0, 0);
    expect(selected()).toHaveLength(1);
    toolStore.getState().setTool("line");
    expect(selected()).toEqual([]);
  });
});

// ── L3 guard: locked reference geometry refuses to arm a drag (W2) ───────────

describe("SketchController select tool — locked reference geometry", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  /** A projected host-face segment (locked) plus an ordinary user line. */
  const LOCKED_SESSION: SketchEntity[] = [
    { id: "ref1", type: "Line", p0: [0, 0], p1: [40, 0], referenceLocked: true },
    { id: "e1", type: "Line", p0: [0, 60], p1: [40, 60] },
  ];

  beforeEach(async () => {
    resetStores();
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      cb(0);
      return 1;
    });
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    clientMock.enterSketch.mockResolvedValue({
      sketchId: "sketch1",
      plane: PLANE,
      entities: LOCKED_SESSION.map((e) => ({ ...e })),
      constraints: [],
      dof: 2,
      status: "UnderConstrained",
    });
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    toolStore.getState().setTool("select");
    await flush();
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    vi.unstubAllGlobals();
  });

  function mouse(type: string, x: number, y: number, button: number, buttons: number): void {
    container.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true }));
  }

  it("refuses to ARM a drag on a locked endpoint, and says why", async () => {
    engineMock.setSketchDrawingActive.mockClear(); // ignore the tool-arm calls from setup
    mouse("pointerdown", 0, 0, 0, 1); // over ref1.Start
    mouse("pointermove", 30, 0, 0, 1); // past DRAG_PX — would begin a gesture
    await flush();

    // No gesture opened: dragging locked geometry is a SetEntityPositions the
    // backend refuses, so it must never leave the frontend.
    expect(clientMock.beginGesture).not.toHaveBeenCalled();
    expect(clientMock.solveDrag).not.toHaveBeenCalled();
    // Never silent — a handle that ignores the pointer reads as a broken app.
    expect(viewportStore.getState().statusHint?.message).toMatch(/Reference geometry is locked/);
    // Orbit was never suppressed for a drag that cannot happen.
    expect(engineMock.setSketchDrawingActive).not.toHaveBeenCalledWith(true);
  });

  it("still SELECTS the locked handle on a click (locked is pickable + snappable)", () => {
    mouse("pointerdown", 0, 0, 0, 1);
    mouse("pointerup", 0, 0, 0, 0);
    expect(sketchSelectionStore.getState().selected).toEqual([{ entityId: "ref1", point: "Start" }]);
  });

  it("an UNLOCKED handle in the same session still drags normally", async () => {
    mouse("pointerdown", 0, 60, 0, 1); // e1.Start
    mouse("pointermove", 30, 60, 0, 1);
    await flush();
    expect(clientMock.beginGesture).toHaveBeenCalledWith(
      "sketch1",
      "e1.Start",
      expect.objectContaining({ kind: "point" }),
    );
  });
});

// ── SP-2 direct manipulation: the three non-point gesture kinds ──────────────
//
// These pin the FE half of SCHEMA §7.4: what the controller ASKS for (the target
// kind) and what it DOES with the `curves` channel. The mock client is a
// kinematic echo — real constraint propagation and refusal live in the worker
// ctest + the real-worker cargo suite, not here.
describe("SketchController select tool — SP-2 gesture kinds", () => {
  const CURVE_ENTITIES: SketchEntity[] = [
    { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
    { id: "c1", type: "Circle", center: [0, 100], radius: 20 },
  ];

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
    clientMock.enterSketch.mockImplementation(() =>
      Promise.resolve({
        sketchId: "sketch1",
        plane: PLANE,
        entities: CURVE_ENTITIES.map((e) => ({ ...e })),
        constraints: [],
        dof: 5,
        status: "UnderConstrained" as const,
      }),
    );
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    toolStore.getState().setTool("select");
    await flush();
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

  it("grabbing a LINE's body asks for an entityBody drag anchored at the grab point", async () => {
    mouse("pointerdown", 20, 0, 0, 1); // mid-span: a body pick, not a handle
    mouse("pointermove", 20, 30, 0, 1);
    await flush();

    expect(clientMock.beginGesture).toHaveBeenCalledWith(
      "sketch1",
      "e1",
      expect.objectContaining({ kind: "entityBody", entityId: "e1", grab: [20, 0] }),
    );
  });

  it("grabbing a CIRCLE's ring asks for a radius drag", async () => {
    mouse("pointerdown", 20, 100, 0, 1); // on the ring (centre [0,100], r 20)
    mouse("pointermove", 30, 100, 0, 1);
    await flush();

    expect(clientMock.beginGesture).toHaveBeenCalledWith(
      "sketch1",
      "c1",
      expect.objectContaining({ kind: "radius", entityId: "c1" }),
    );
  });

  it("a radius drag moves NO point, so the preview comes from `curves` alone", async () => {
    clientMock.solveDrag.mockImplementation(() =>
      Promise.resolve({
        gestureId: 1,
        seq: 1,
        status: "success",
        dof: 5,
        conflicting: [],
        positions: {}, // a radius gesture reports no position at all
        curves: { c1: { radius: 33 } },
        solveMicros: 0,
        superseded: false,
      }),
    );
    mouse("pointerdown", 20, 100, 0, 1);
    mouse("pointermove", 33, 100, 0, 1);
    await flush();

    const calls = engineMock.updateSketchSession.mock.calls;
    const previewed = calls[calls.length - 1]?.[1] as SketchEntity[];
    expect(previewed.find((e) => e.id === "c1")?.radius).toBe(33);
  });

  it("commits the curves channel — a resized circle reaches the session, not just the worker", async () => {
    clientMock.endGesture.mockImplementation(() =>
      Promise.resolve({
        sketchId: "sketch1",
        sketchRevision: 2,
        dof: 5,
        status: "UnderConstrained" as const,
        solvedPositions: {},
        solvedCurves: { c1: { radius: 42 } },
      }),
    );
    mouse("pointerdown", 20, 100, 0, 1);
    mouse("pointermove", 42, 100, 0, 1);
    await flush();
    mouse("pointerup", 42, 100, 0, 0);
    await flush();

    const committed = sketchStore.getState().session?.entities ?? [];
    expect(committed.find((e) => e.id === "c1")?.radius).toBe(42);
  });

  it("Esc mid-radius-drag ends at the GRAB point, the zero-delta target", async () => {
    mouse("pointerdown", 20, 100, 0, 1);
    mouse("pointermove", 35, 100, 0, 1);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flush();

    // Not the handle's coordinate: a radius/entityBody gesture measures from `grab`,
    // so ending anywhere else would commit however far the user had already dragged.
    expect(clientMock.endGesture).toHaveBeenCalledWith([20, 100]);
  });
});
