/*
 * SketchController snap-candidate cache (perf): entity-derived snap candidates
 * (guide points, quadrant points, the O(n²) all-pairs intersection scan) are
 * precomputed once per sketch edit via `buildSnapCache`, not re-derived on every
 * pointer move. Session entity arrays are replaced immutably on every commit
 * (applySolvedPositions returns the SAME reference iff nothing moved), so
 * reference equality of `session.entities` is a valid, cheap invalidation key.
 * This asserts `snapAt` rebuilds the cache only when that reference changes.
 * Engine + client are faked (no WebGL / no backend).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import * as snapEngine from "./snapEngine";
import type { SnapResult } from "./snapEngine";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
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
    enterSketch: vi.fn(),
    exitSketch: vi.fn(),
    setSketchDrawingActive: vi.fn(),
    setSketchPreview: vi.fn(),
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

type Internals = { snapAt(clientX: number, clientY: number): SnapResult | null };

describe("SketchController — snap candidate cache invalidation", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;
  let buildSpy: ReturnType<typeof vi.spyOn>;

  const ENTITIES_A: SketchEntity[] = [
    { id: "l1", type: "Line", p0: [-30, 5], p1: [30, 5] },
    { id: "l2", type: "Line", p0: [5, -30], p1: [5, 30] },
  ];
  const ENTITIES_B: SketchEntity[] = [{ id: "c1", type: "Circle", center: [0, 0], radius: 20 }];

  beforeEach(() => {
    resetStores();
    buildSpy = vi.spyOn(snapEngine, "buildSnapCache");
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    buildSpy.mockRestore();
  });

  const internals = (): Internals => controller as unknown as Internals;

  it("rebuilds the cache only once for repeated snapAt calls against the SAME entities reference", () => {
    sketchStore.getState().setSession(makeSession(ENTITIES_A));
    buildSpy.mockClear();

    internals().snapAt(6, 6);
    internals().snapAt(21, 1);
    internals().snapAt(0, 0);

    expect(buildSpy).toHaveBeenCalledTimes(1);
    expect(buildSpy).toHaveBeenCalledWith(ENTITIES_A);
  });

  it("rebuilds the cache when the session's entity array reference changes", () => {
    sketchStore.getState().setSession(makeSession(ENTITIES_A));
    buildSpy.mockClear();

    internals().snapAt(6, 6);
    expect(buildSpy).toHaveBeenCalledTimes(1);

    // A new array reference (as a real commit produces) invalidates the cache,
    // even though this second array is unchanged content-wise from the first call.
    sketchStore.getState().setSession(makeSession(ENTITIES_B));
    internals().snapAt(21, 1);
    expect(buildSpy).toHaveBeenCalledTimes(2);
    expect(buildSpy).toHaveBeenLastCalledWith(ENTITIES_B);

    // Same reference again ⇒ no further rebuild.
    internals().snapAt(19, 2);
    expect(buildSpy).toHaveBeenCalledTimes(2);
  });

  it("never rebuilds while there is no session (cache key stays null, matching)", () => {
    // No session set — sessionEntities is `null` on every call, same as the
    // controller's initial (unset) cache key, so this must never rebuild —
    // strictly better than "rebuild once": there's nothing to cache.
    buildSpy.mockClear();

    internals().snapAt(6, 6);
    internals().snapAt(7, 7);
    internals().snapAt(8, 8);

    expect(buildSpy).not.toHaveBeenCalled();
  });
});
