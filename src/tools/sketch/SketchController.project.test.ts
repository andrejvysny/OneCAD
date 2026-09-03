/*
 * SketchController — the Project-edges gesture (WP-P, jsdom).
 *
 * Engine + client are faked (no WebGL, no backend). `probePick` is scripted per
 * click, which is what makes "click a body edge" expressible without a raycast.
 *
 * Covers the ladder the pure `projectTool` tests cannot see: which clicks reach
 * the accumulator, that Enter COMMITS instead of finishing the sketch, that the
 * first Esc only clears, and that a commit re-hydrates the session (the whole
 * reason the tool re-enters the sketch at all — a freshly projected entity's
 * backend uuid is not in the id-map until it does).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchController } from "./SketchController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ProjectToSketchRequest,
  ProjectToSketchResult,
  SketchPlane,
  SketchSession,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import { setViewportEngine } from "@/viewport/engineBridge";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/** What a `probePick` answers with — the shape `projectPickFrom` reads. */
const hit = (kind: "face" | "edge", topoKey: string, bodyId = "body1") => ({
  bodyId,
  kind,
  topoKey,
  elementId: undefined,
  distance: 1,
  worldPos: { x: 1, y: 2, z: 3 },
});

function makeEngineMock() {
  return {
    setPlanePickerVisible: vi.fn(),
    planePickerHover: vi.fn(),
    planePickerHitTest: vi.fn(() => null),
    clearPlanePickerHover: vi.fn(),
    probePick: vi.fn((): ReturnType<typeof hit> | null => null),
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
    planeScreenMetric: vi.fn(() => ({ m00: 1, m01: 0, m10: 0, m11: 1 })),
  };
}

/** Session the client hands back. Mutated between enters so the SECOND enter
 *  (the re-hydration) can carry the projections the first one did not. */
let session: SketchSession;

function makeClientMock() {
  return {
    enterSketch: vi.fn((): Promise<SketchSession> => Promise.resolve({ ...session })),
    cancelSketch: vi.fn(() => Promise.resolve()),
    deleteSketch: vi.fn(() => Promise.resolve()),
    finishSketch: vi.fn(() => Promise.resolve({ sketchId: "sketch1", regions: [] })),
    sketchUpsert: vi.fn((sketchId: string) =>
      Promise.resolve({
        sketchId,
        sketchRevision: 1,
        dof: 0,
        status: "UnderConstrained" as const,
        solvedPositions: {},
      }),
    ),
    projectToSketch: vi.fn((req: ProjectToSketchRequest): Promise<ProjectToSketchResult> =>
      Promise.resolve({
        sketchId: req.sketchId,
        snapshotId: 7,
        entities: [],
        pointCount: 0,
        refusals: [],
      }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("SketchController — Project edges (WP-P)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: SketchController;

  beforeEach(async () => {
    resetStores();
    session = {
      sketchId: "sketch1",
      plane: PLANE,
      entities: [],
      constraints: [],
      dof: 0,
      status: "UnderConstrained",
    };
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
    // The re-hydration lives in `sketchService` (React reaches it too), so it
    // publishes through the engine BRIDGE, not `deps.engine` — register the same
    // double there or the projected-marker write goes nowhere.
    setViewportEngine(engineMock as unknown as ViewportEngine);
    toolStore.getState().setMode("sketch", "sketch1");
    await flush();
    toolStore.getState().setTool("project");
    await flush();
  });

  afterEach(() => {
    setViewportEngine(null);
    controller.dispose();
    container.remove();
    vi.unstubAllGlobals();
  });

  function mouse(type: string, x: number, y: number, button: number, buttons: number): void {
    container.dispatchEvent(
      new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true }),
    );
  }
  const clickHit = (h: ReturnType<typeof hit> | null): void => {
    engineMock.probePick.mockReturnValueOnce(h);
    mouse("pointerdown", 10, 10, 0, 1);
    mouse("pointerup", 10, 10, 0, 0);
  };
  const key = (k: string): KeyboardEvent => {
    const ev = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true });
    window.dispatchEvent(ev);
    return ev;
  };
  const requests = (): ProjectToSketchRequest[] =>
    clientMock.projectToSketch.mock.calls.map((c) => c[0] as ProjectToSketchRequest);

  it("arming names the gesture in the status hint", () => {
    expect(viewportStore.getState().statusHint?.message).toMatch(/^Project — click a body edge/);
  });

  it("accumulates picks, and a second click on one DROPS it", () => {
    clickHit(hit("edge", "e:0"));
    clickHit(hit("edge", "e:1"));
    expect(viewportStore.getState().statusHint?.message).toMatch(/2 picked/);
    expect(selectionStore.getState().selected.map((r) => r.id)).toEqual([
      "body1#e:0",
      "body1#e:1",
    ]);

    clickHit(hit("edge", "e:0"));
    expect(viewportStore.getState().statusHint?.message).toMatch(/1 picked/);
    expect(selectionStore.getState().selected.map((r) => r.id)).toEqual(["body1#e:1"]);
  });

  it("a click on nothing pickable says so and holds the pick set", () => {
    clickHit(hit("edge", "e:0"));
    clickHit(null);
    expect(viewportStore.getState().statusHint?.message).toMatch(/nothing pickable there/);
    expect(selectionStore.getState().selected).toHaveLength(1);
  });

  it("Enter sends one request per (body, mode) group, each source anchored", async () => {
    clickHit(hit("edge", "e:0"));
    clickHit(hit("face", "f:3"));
    const ev = key("Enter");
    expect(ev.defaultPrevented, "Enter must not reach the global finishSketch").toBe(true);
    await flush();
    await flush();

    const reqs = requests();
    expect(reqs).toHaveLength(2);
    expect(reqs.map((r) => r.mode)).toEqual(["edges", "faceOutline"]);
    expect(reqs[0].sources[0]).toMatchObject({
      bodyId: "body1",
      topoKey: "e:0",
      anchor: { worldPoint: [1, 2, 3] },
    });
    // The sketch is NOT finished by that Enter.
    expect(clientMock.finishSketch).not.toHaveBeenCalled();
    expect(toolStore.getState().mode).toBe("sketch");
  });

  it("Enter with NO picks falls through to the global Enter (finish sketch)", () => {
    const ev = key("Enter");
    expect(ev.defaultPrevented).toBe(false);
    expect(clientMock.projectToSketch).not.toHaveBeenCalled();
  });

  it("commits, re-hydrates the session, and drops back to Select", async () => {
    clickHit(hit("edge", "e:0"));
    const entersBefore = clientMock.enterSketch.mock.calls.length;
    // The re-hydrating enter is what makes the new entity reachable at all.
    session = {
      ...session,
      entities: [{ id: "p1", type: "Line", p0: [0, 0], p1: [10, 0], referenceLocked: true }],
      projections: {
        p1: {
          sourceBodyId: "body_1",
          sourceElementId: "el1",
          sourceKind: "edge",
          sourceOrdinal: 0,
          projectedHash: "0",
        },
      },
    };
    key("Enter");
    await flush();
    await flush();

    expect(clientMock.enterSketch.mock.calls.length).toBe(entersBefore + 1);
    expect(Object.keys(sketchStore.getState().session?.projections ?? {})).toEqual(["p1"]);
    expect(engineMock.setSketchProjectedIds).toHaveBeenLastCalledWith(["p1"]);
    expect(viewportStore.getState().statusHint?.message).toBe("Projected 1 source into the sketch");
    expect(toolStore.getState().sketchTool).toBe("select");
  });

  it("reports a refusal in ONE line, not one per source", async () => {
    clientMock.projectToSketch.mockImplementationOnce((req: ProjectToSketchRequest): Promise<ProjectToSketchResult> =>
      Promise.resolve({
        sketchId: req.sketchId,
        snapshotId: 7,
        entities: [],
        pointCount: 0,
        refusals: [
          { bodyId: "body1", elementId: "e", topoKey: "e:0", code: "unsupportedCurve", message: "MOCK LIMIT: curved" },
        ],
      }),
    );
    clickHit(hit("edge", "e:0"));
    clickHit(hit("edge", "e:1"));
    key("Enter");
    await flush();
    await flush();

    expect(viewportStore.getState().statusHint?.message).toBe(
      "Projected 1 of 2; 1 refused: MOCK LIMIT: curved",
    );
  });

  it("the FIRST Esc only clears the picks; the tool stays armed", () => {
    clickHit(hit("edge", "e:0"));
    const ev = key("Escape");
    expect(ev.defaultPrevented).toBe(true);
    expect(toolStore.getState().sketchTool).toBe("project");
    expect(viewportStore.getState().statusHint?.message).toMatch(/^Project — click a body edge/);
    expect(selectionStore.getState().selected).toHaveLength(0);

    // …and the SECOND falls through to the global Esc ladder.
    expect(key("Escape").defaultPrevented).toBe(false);
  });

  it("leaving the tool drops the picks and gives the selection back to the sketch", async () => {
    clickHit(hit("edge", "e:0"));
    toolStore.getState().setTool("select");
    await flush();
    expect(selectionStore.getState().selected).toEqual([{ kind: "sketch", id: "sketch1" }]);
  });
});
