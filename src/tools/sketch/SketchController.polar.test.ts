/*
 * SketchController ↔ polar tracking + snap radius (SP-5 W6/W7).
 *
 * The engine-side geometry is unit-tested in snapEngine.test.ts; what is pinned
 * HERE is the wiring the popover switches actually reach:
 *   - `snapAt` forwards the pref (`enablePolar`), the fan centre (the chain's
 *     LAST anchor) and the reference direction (chainTangent, else the chord),
 *   - the snap radius reaches BOTH the snap ladder and the pick tolerance,
 *   - a `polar` snap suppresses dimension rounding (the regression pin — the
 *     {none, grid} allow-list already does this, and must keep doing it: a guide
 *     tier places the point deliberately, and rounding would move it off).
 * Engine + client are faked (no WebGL / no backend).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import * as snapEngine from "./snapEngine";
import type { SnapOptions, SnapResult } from "./snapEngine";
import { SNAP_RADIUS_PX } from "./snapRadius";
import type { ToolState } from "./toolMachine";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { settingsStore } from "@/stores/settingsStore";
import { sketchStore } from "@/stores/sketchStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = { kind: "XY", origin: [0, 0, 0], xAxis: [1, 0, 0], yAxis: [0, 1, 0], normal: [0, 0, 1] };

function makeSession(entities: SketchEntity[]): SketchSession {
  return {
    sketchId: "sketch1",
    plane: PLANE,
    entities,
    constraints: [],
    dof: 0,
    status: "UnderConstrained",
  };
}

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
    getCameraDistance: vi.fn(() => 100),
  };
}

function makeClientMock() {
  return {
    enterSketch: vi.fn((): Promise<SketchSession> => Promise.resolve(makeSession([]))),
    cancelSketch: vi.fn(() => Promise.resolve()),
    deleteSketch: vi.fn(() => Promise.resolve()),
  };
}

/** The private surface these assertions drive (all of it controller-internal). */
type Internals = {
  snapAt(clientX: number, clientY: number): SnapResult | null;
  pickTol(): number;
  dimQuantumNow(): unknown;
  machineState: ToolState | null;
  lastSnap: SnapResult | null;
};

describe("SketchController — polar tracking + snap radius wiring", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;
  let snapSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    resetStores();
    // The test lane resets polar OFF (see resetStores); this lane is the one
    // that opts back in, exactly as the app does.
    settingsStore.getState().setSnap("polarTracking", true);
    snapSpy = vi.spyOn(snapEngine, "computeSnap");
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
    sketchStore.getState().setSession(makeSession([]));
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    snapSpy.mockRestore();
  });

  const internals = (): Internals => controller as unknown as Internals;

  /** Drive the machine state directly — the pointer ladder is covered elsewhere;
   *  what matters here is which anchors/tangent `snapAt` reads. */
  function arm(anchors: Point2[], chainTangent?: Point2): void {
    internals().machineState = { anchors, cursor: null, ...(chainTangent ? { chainTangent } : {}) };
  }

  const lastOpts = (): SnapOptions => snapSpy.mock.calls[snapSpy.mock.calls.length - 1][2] as SnapOptions;

  it("forwards the pref, the last anchor and the snap radius", () => {
    arm([{ x: 1, y: 2 }, { x: 10, y: 2 }]);
    internals().snapAt(20, 20);

    const opts = lastOpts();
    expect(opts.enablePolar).toBe(true);
    expect(opts.polarAnchor).toEqual({ x: 10, y: 2 });
    expect(opts.snapPx).toBe(SNAP_RADIUS_PX.m);
  });

  it("forwards enablePolar:false when the pref is off", () => {
    settingsStore.getState().setSnap("polarTracking", false);
    arm([{ x: 0, y: 0 }]);
    internals().snapAt(5, 5);
    expect(lastOpts().enablePolar).toBe(false);
  });

  it("passes a NULL anchor with no gesture in progress (the tier stays inert)", () => {
    internals().snapAt(5, 5);
    expect(lastOpts().polarAnchor).toBeNull();

    arm([]);
    internals().snapAt(5, 5);
    expect(lastOpts().polarAnchor).toBeNull();
  });

  it("derives the reference direction from the last two anchors", () => {
    arm([{ x: 0, y: 0 }, { x: 3, y: 4 }]);
    internals().snapAt(9, 9);
    expect(lastOpts().polarRefDir).toEqual({ x: 0.6, y: 0.8 });
  });

  it("has NO reference direction from a single anchor (nothing to be parallel to)", () => {
    arm([{ x: 0, y: 0 }]);
    internals().snapAt(9, 9);
    expect(lastOpts().polarRefDir).toBeNull();
  });

  it("PREFERS the machine's chainTangent over the chord (an arc's exit ≠ its chord)", () => {
    // Chord would be +x; the recorded exit direction is +y. The arc actually
    // leaves along +y, so fanning off the chord would offer a line the chain
    // never travelled.
    arm([{ x: 0, y: 0 }, { x: 10, y: 0 }], { x: 0, y: 1 });
    internals().snapAt(9, 9);
    expect(lastOpts().polarRefDir).toEqual({ x: 0, y: 1 });
  });

  it("snaps a real cursor onto the 45° ray off the anchor", () => {
    arm([{ x: 0, y: 0 }]);
    const snap = internals().snapAt(30, 26)!;
    expect(snap.kind).toBe("polar");
    expect(snap.point.x).toBeCloseTo(28, 9);
    expect(snap.point.y).toBeCloseTo(28, 9);
  });

  // THE PIN. `dimQuantumNow`'s allow-list is {none, grid}: a polar snap already
  // placed the point ON a deliberate ray, and rounding it afterwards would slide
  // it off — silently, since the guide would still be drawn. If a future edit
  // widens that allow-list, this fails.
  it("suppresses dimension rounding while the last snap is polar", () => {
    settingsStore.getState().setSnap("dimensionRound", true);
    const priv = internals();

    priv.lastSnap = { point: { x: 0, y: 0 }, kind: "none", label: null, guides: [], snapped: false };
    expect(priv.dimQuantumNow()).not.toBeNull(); // control: rounding IS live here

    priv.lastSnap = { point: { x: 1, y: 1 }, kind: "polar", label: "45°", guides: [], snapped: true };
    expect(priv.dimQuantumNow()).toBeNull();
  });

  it("pickTol tracks the snap-radius preference (and the zoom)", () => {
    expect(internals().pickTol()).toBe(SNAP_RADIUS_PX.m);

    settingsStore.getState().setSnapRadius("l");
    expect(internals().pickTol()).toBe(SNAP_RADIUS_PX.l);

    settingsStore.getState().setSnapRadius("s");
    expect(internals().pickTol()).toBe(SNAP_RADIUS_PX.s);

    // Screen-scaled, like every other tolerance: 5px at 2 world-units per pixel.
    engineMock.planePixelWorld.mockReturnValue(2);
    expect(internals().pickTol()).toBe(SNAP_RADIUS_PX.s * 2);
  });

  it("the snap ladder gets the SAME radius the pick tolerance uses", () => {
    settingsStore.getState().setSnapRadius("l");
    internals().snapAt(5, 5);
    expect(lastOpts().snapPx).toBe(SNAP_RADIUS_PX.l);
    expect(internals().pickTol()).toBe(SNAP_RADIUS_PX.l);
  });

  it("a LARGER radius reaches an endpoint an M radius misses", () => {
    sketchStore.getState().setSession(makeSession([{ id: "l1", type: "Line", p0: [0, 0], p1: [40, 0] }]));
    // 10 world units past the endpoint: outside 8px, inside 12px.
    expect(internals().snapAt(50, 0)!.kind).not.toBe("endpoint");

    settingsStore.getState().setSnapRadius("l");
    expect(internals().snapAt(50, 0)!.kind).toBe("endpoint");
  });
});
