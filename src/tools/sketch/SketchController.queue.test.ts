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
    getCameraDistance: vi.fn(() => 260),
    invalidate: vi.fn(),
    planePixelWorld: vi.fn(() => 1),
    // Isotropic 1px-per-unit metric: matches `planePixelWorld: 1` above, so a
    // plane distance IS a screen-pixel distance in these tests.
    planeScreenMetric: vi.fn(() => ({ m00: 1, m01: 0, m10: 0, m11: 1 })),
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

/*
 * ATOMIC CLICK INTENT (SNAP P1).
 *
 * The commit path used to read `this.altHeld` and a mutable `originSnapPoints`
 * list INSIDE its queued turn. Both describe the pointer as it is NOW, not as it
 * was when the click happened — so a fast burst assigned one click's evidence to
 * a different click's batch. The fix freezes an `AcceptedSnapIntent` in the
 * click's own synchronous stack; these tests drive the queue by hand so the
 * "later state leaks backwards" window is actually open.
 */
describe("SketchController click intent is atomic", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: Record<string, ReturnType<typeof vi.fn>>;
  let container: HTMLDivElement;
  let controller: SketchController;
  let resolvers: Array<() => void>;

  /** Drive the controller the way the pointer path does: snap → capture → step. */
  interface Probe {
    noteDecision(decision: unknown): void;
    dimCommitContext(): unknown;
    buildCommitConstraints(
      session: SketchSession,
      newEntities: SketchEntity[],
      specs: unknown,
      dims: unknown,
    ): { toolAuthored: Array<{ type: string; positions?: string[] }> };
    altHeld: boolean;
    anchorIntents: unknown[];
  }

  /** A minimal accepted-origin decision — the shape `decisionAt` returns. */
  const originSnap = {
    raw: { x: 0, y: 0 },
    point: { x: 0, y: 0 },
    accepted: [
      {
        id: "origin:0:0",
        source: "geometryPoint",
        kind: "origin",
        projection: { kind: "point", point: { x: 0, y: 0 } },
        previewPoint: { x: 0, y: 0 },
        claims: ["point"],
        errorPx: 0,
        semanticBiasPx: -2,
        scorePx: -2,
        label: "Origin",
        guides: [],
        refs: [],
        relationIntents: [{ kind: "FixedOrigin", refs: [] }],
      },
    ],
    rejected: [],
    primaryId: "origin:0:0",
    primaryKind: "origin",
    label: "Origin",
    guides: [],
    snapped: true,
    traceId: 1,
  };
  const freeSnap = {
    raw: { x: 40, y: 40 },
    point: { x: 40, y: 40 },
    accepted: [],
    rejected: [],
    primaryId: null,
    primaryKind: "none",
    label: null,
    guides: [],
    snapped: false,
    traceId: 2,
  };

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
            entities: [],
            constraints: [],
            dof: 0,
            status: "UnderConstrained",
          }),
      ),
      cancelSketch: vi.fn(() => Promise.resolve()),
      deleteSketch: vi.fn(() => Promise.resolve()),
      endGesture: vi.fn(() => Promise.resolve()),
      sketchUpsert: vi.fn(
        () =>
          new Promise<SketchUpsertResult>((res) =>
            resolvers.push(() =>
              res({
                sketchId: "sketch1",
                sketchRevision: 1,
                dof: 0,
                status: "UnderConstrained",
                solvedPositions: {},
              }),
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
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
  });

  const probe = (): Probe => controller as unknown as Probe;

  it("an origin snap belongs to the click that made it, not to a queued earlier one", async () => {
    const p = probe();
    // Click A: a FREE placement. Its context is captured now…
    p.noteDecision(freeSnap);
    const dimsA = p.dimCommitContext();
    // …then click B snaps to the origin BEFORE A's queued turn runs.
    p.noteDecision(originSnap);
    const dimsB = p.dimCommitContext();

    const session = sketchStore.getState().session!;
    const lineA: SketchEntity = { id: "a", type: "Line", p0: [40, 40], p1: [90, 40] };
    const lineB: SketchEntity = { id: "b", type: "Line", p0: [0, 0], p1: [50, 0] };

    const outA = p.buildCommitConstraints(session, [lineA], undefined, dimsA);
    const outB = p.buildCommitConstraints(session, [lineB], undefined, dimsB);

    // A must NOT have consumed B's origin evidence.
    expect(outA.toolAuthored.some((c) => c.type === "Fixed")).toBe(false);
    expect(outB.toolAuthored.some((c) => c.type === "Fixed")).toBe(true);
  });

  it("Alt released AFTER the click still suppresses that click's inference", () => {
    const p = probe();
    p.altHeld = true;
    p.noteDecision(freeSnap); // the click happens while Alt is down
    const dims = p.dimCommitContext();
    p.altHeld = false; // …and Alt comes up before the queued turn

    const session = sketchStore.getState().session!;
    // An exactly-horizontal line would normally infer Horizontal.
    const line: SketchEntity = { id: "a", type: "Line", p0: [0, 5], p1: [50, 5] };
    const out = p.buildCommitConstraints(session, [line], undefined, dims);
    expect(out.toolAuthored).toEqual([]);
  });

  it("Alt pressed AFTER the click leaves that click's normal inference intact", () => {
    const p = probe();
    p.altHeld = false;
    p.noteDecision(freeSnap);
    const dims = p.dimCommitContext();
    p.altHeld = true; // pressed only after the click was taken

    const session = sketchStore.getState().session!;
    const line: SketchEntity = { id: "a", type: "Line", p0: [0, 5], p1: [50, 5] };
    const out = p.buildCommitConstraints(session, [line], undefined, dims);
    expect(out.toolAuthored.some((c) => c.type === "Horizontal")).toBe(true);
  });

  it("an Alt-held click authors no origin anchor even when it snapped to the origin", () => {
    const p = probe();
    p.altHeld = true;
    p.noteDecision(originSnap);
    const dims = p.dimCommitContext();

    const session = sketchStore.getState().session!;
    const line: SketchEntity = { id: "a", type: "Line", p0: [0, 0], p1: [50, 0] };
    const out = p.buildCommitConstraints(session, [line], undefined, dims);
    expect(out.toolAuthored).toEqual([]);
  });

  it("a stale rAF move after a click cannot publish over the new phase", async () => {
    const raf = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((cb: FrameRequestCallback) => {
        pending.push(cb);
        return 0;
      });
    const pending: FrameRequestCallback[] = [];
    try {
      toolStore.getState().setTool("line");
      await flush();
      engineMock.setSketchPreview.mockClear();
      container.dispatchEvent(
        new PointerEvent("pointermove", { clientX: 10, clientY: 10, bubbles: true }),
      );
      expect(pending).toHaveLength(1);
      // The click lands BEFORE the queued frame runs.
      container.dispatchEvent(
        new PointerEvent("pointerdown", { clientX: 20, clientY: 20, button: 0, bubbles: true }),
      );
      container.dispatchEvent(
        new PointerEvent("pointerup", { clientX: 20, clientY: 20, button: 0, bubbles: true }),
      );
      const afterClick = engineMock.setSketchPreview.mock.calls.length;
      pending[0](0); // the stale frame finally runs
      // It published nothing: the epoch it captured is gone.
      expect(engineMock.setSketchPreview.mock.calls.length).toBe(afterClick);
    } finally {
      raf.mockRestore();
    }
  });

  it("a pointerleave clears the transient snap feedback without ending the gesture", async () => {
    toolStore.getState().setTool("line");
    await flush();
    const p = probe();
    p.noteDecision(originSnap);
    p.anchorIntents = [{}];
    engineMock.setSketchSnap.mockClear();
    container.dispatchEvent(new PointerEvent("pointerleave", { bubbles: false }));
    expect(engineMock.setSketchSnap).toHaveBeenCalledWith(null, false);
    // The armed gesture survives — leaving the viewport is not a cancel.
    expect(p.anchorIntents).toHaveLength(1);
  });
});
