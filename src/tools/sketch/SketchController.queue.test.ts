/*
 * SketchController mutation coordinator (C1): the controller routes commit + mirror
 * through the shared serialized queue, re-reading + rebasing the session INSIDE each
 * queued turn. Two rapid commits (deferred upserts) must BOTH land — proving the
 * second rebases on the first's settled result, not a stale capture. A commit racing
 * a mirror stays ordered too. Engine + client are faked (no WebGL / no backend).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { SketchEntity, SketchPlane, SketchSession, SketchUpsertResult } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = { kind: "XY", origin: [0, 0, 0], xAxis: [1, 0, 0], yAxis: [0, 1, 0], normal: [0, 0, 1] };

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

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController mutation queue", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: {
    enterSketch: ReturnType<typeof vi.fn>;
    cancelSketch: ReturnType<typeof vi.fn>;
    deleteSketch: ReturnType<typeof vi.fn>;
    endGesture: ReturnType<typeof vi.fn>;
    sketchUpsert: ReturnType<typeof vi.fn>;
  };
  let container: HTMLDivElement;
  let controller: SketchController;
  // Manually-resolved upsert deferreds — one per sketchUpsert call.
  let resolvers: Array<() => void>;

  const ENTITIES: SketchEntity[] = [
    { id: "src", type: "Line", p0: [0, 50], p1: [100, 50] }, // mirror source
    { id: "axis", type: "Line", p0: [0, 0], p1: [100, 0] }, // mirror axis (X axis)
  ];

  beforeEach(async () => {
    resetStores();
    resolvers = [];
    engineMock = makeEngineMock();
    clientMock = {
      enterSketch: vi.fn(
        (): Promise<SketchSession> =>
          Promise.resolve({
            sketchId: "sketch1",
            plane: PLANE,
            entities: ENTITIES.map((e) => ({ ...e })),
            constraints: [],
            dof: 8,
            status: "UnderConstrained",
          }),
      ),
      cancelSketch: vi.fn(() => Promise.resolve()),
      deleteSketch: vi.fn(() => Promise.resolve()),
      endGesture: vi.fn(() => Promise.resolve()),
      // Every upsert parks on a deferred resolver so we can order the two turns.
      sketchUpsert: vi.fn(
        () =>
          new Promise<SketchUpsertResult>((res) =>
            resolvers.push(() =>
              res({ sketchId: "sketch1", sketchRevision: 1, dof: 0, status: "UnderConstrained", solvedPositions: {} }),
            ),
          ),
      ),
    };
    container = document.createElement("div");
    document.body.appendChild(container);
    controller = new SketchController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
    });
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    // Advance the id counter past the seeded src/axis so minted ids don't collide.
    sketchStore.setState({ entitySeq: 2, constraintSeq: 0 });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
  });

  it("two rapid commits both land in the final session (second rebases on the first)", async () => {
    const c = controller as unknown as { commit(d: unknown[]): Promise<void> };
    void c.commit([{ type: "Line", p0: { x: 0, y: 200 }, p1: { x: 100, y: 200 } }]);
    void c.commit([{ type: "Line", p0: { x: 0, y: 300 }, p1: { x: 100, y: 300 } }]);

    await flush(); // commit#1 parked at its upsert; commit#2 queued behind it
    expect(resolvers).toHaveLength(1);
    resolvers[0]();

    await flush(); // commit#1 wrote; commit#2 now runs + parks
    expect(resolvers).toHaveLength(2);
    resolvers[1]();

    await flush(); // commit#2 wrote
    const ids = sketchStore.getState().session!.entities.map((e) => e.id);
    // Seeded src/axis + BOTH committed lines (e3, e4) — the second was NOT lost.
    expect(ids).toEqual(["src", "axis", "e3", "e4"]);
  });

  it("a commit racing a mirror stays ordered — both results are present", async () => {
    const start = sketchStore.getState().session!;
    const source = start.entities.find((e) => e.id === "src")!;
    const axis = start.entities.find((e) => e.id === "axis")!;

    const c = controller as unknown as {
      commit(d: unknown[]): Promise<void>;
      performMirror(s: SketchEntity[], a: SketchEntity): Promise<void>;
    };
    void c.commit([{ type: "Line", p0: { x: 0, y: 200 }, p1: { x: 100, y: 200 } }]);
    void c.performMirror([source], axis);

    await flush();
    resolvers[0]();
    await flush();
    resolvers[1]();
    await flush();

    const s = sketchStore.getState().session!;
    // Committed line (e3) + mirrored copy of src across the X axis (e4 at y = -50).
    expect(s.entities).toHaveLength(4);
    const mirrored = s.entities.find((e) => e.type === "Line" && e.p0?.[1] === -50);
    expect(mirrored).toBeDefined();
    expect(s.constraints.some((k) => k.type === "Symmetric")).toBe(true);
  });
});
