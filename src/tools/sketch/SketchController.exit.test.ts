/*
 * SketchController exit-order pin (EXTRUDE-COMMIT-FIX / REVOLVE-REGION-PARITY).
 *
 * Every keep-exit must run `cancelSketch` FIRST (worker-gesture teardown + the
 * take-once session squash) and `finishSketch` SECOND (solve + regions + the
 * `Sketch` TIMELINE RECORD upsert). The order is load-bearing, not cosmetic:
 *   - finish-then-cancel would squash AFTER the record was minted, and
 *   - cancel-only (the pre-postmortem behaviour) left every interactively drawn
 *     sketch RECORDLESS, so the regen planner failed every later modeling-op
 *     commit with "profile sketch not found in plan".
 *
 * The re-edit + arm paths now deliberately read regions PURELY
 * (`getSketchRegions`), so this exit is one of only two places that author the
 * record at all — the other being the commit-boundary guarantee. Pin it.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchEntity, SketchPlane, SketchSession } from "@/ipc/types";
import { __resetLogForTests, logSnapshot } from "@/debug/log";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
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
    setSketchProjectedIds: vi.fn(),
    screenToPlane: vi.fn((x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
    // Isotropic 1px-per-unit metric: matches `planePixelWorld: 1` above, so a
    // plane distance IS a screen-pixel distance in these tests.
    planeScreenMetric: vi.fn(() => ({ m00: 1, m01: 0, m10: 0, m11: 1 })),
    getCameraDistance: vi.fn(() => 100),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController exit ordering", () => {
  let container: HTMLDivElement;
  let controller: SketchController;
  let calls: string[];
  let clientMock: {
    enterSketch: ReturnType<typeof vi.fn>;
    cancelSketch: ReturnType<typeof vi.fn>;
    finishSketch: ReturnType<typeof vi.fn>;
  };

  beforeEach(() => {
    resetStores();
    calls = [];
    container = document.createElement("div");
    document.body.appendChild(container);
    clientMock = {
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
      cancelSketch: vi.fn((id: string) => {
        calls.push(`cancelSketch:${id}`);
        return Promise.resolve();
      }),
      finishSketch: vi.fn((id: string) => {
        calls.push(`finishSketch:${id}`);
        return Promise.resolve({ regions: [] });
      }),
    };
    controller = new SketchController({
      engine: makeEngineMock() as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
  });

  it("exiting sketch mode runs cancelSketch THEN finishSketch (record mint after the squash)", async () => {
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    expect(sketchStore.getState().session).not.toBeNull();

    toolStore.getState().setMode("model"); // keep-exit
    await flush();
    await flush();

    expect(calls).toEqual(["cancelSketch:sketch1", "finishSketch:sketch1"]);
    expect(sketchStore.getState().session).toBeNull();
  });

  it("a failing cancelSketch does not silently skip the record mint chain", async () => {
    // The exit is best-effort: a cancel rejection is caught and logged, but the
    // sequence must not leave the session dangling.
    clientMock.cancelSketch.mockImplementation((id: string) => {
      calls.push(`cancelSketch:${id}`);
      return Promise.reject(new Error("worker gone"));
    });
    // The failure now lands on the structured log lane (DEV-OBSERVABILITY Wave
    // F), whose gate vitest keeps closed — open it just for this assertion.
    __resetLogForTests();

    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    toolStore.getState().setMode("model");
    await flush();
    await flush();

    expect(calls).toEqual(["cancelSketch:sketch1"]); // finish never reached
    // ...but the failure is LOUD, never silent.
    expect(logSnapshot().filter((e) => e.level === "error" && e.tag === "sketch")).toHaveLength(1);
    expect(sketchStore.getState().session).toBeNull();
    __resetLogForTests({ enabled: false });
  });
});
