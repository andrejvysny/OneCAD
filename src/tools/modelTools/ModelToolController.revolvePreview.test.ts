/*
 * Revolve kernel-preview lane (jsdom). Before this, Revolve had ONLY its L1
 * Rodrigues lathe: a Cut revolve dragged as a solid shell and never showed the
 * subtraction, and the commit authored a second, ad-hoc `applyOperation` that the
 * user had never seen previewed.
 *
 * These specs pin the fixed contract:
 *   (a) picking an AXIS is what makes the op well-formed → N lane sessions open
 *       (one per armed region) carrying typed Revolve drafts,
 *   (b) drag / chip / boolean-mode changes push epoch-BUMPED updates to every
 *       session (a Cut mode change is the frame that starts subtracting),
 *   (c) confirm = flush final params → exact-preview barrier → endPreview(true):
 *       the committed op IS the previewed candidate, never a second op,
 *   (d) a failed final preview blocks the commit, releases the lane, and RE-ARMS
 *       so the user's work is never lost,
 *   (e) axis reset / tool switch / dispose end EVERY session with endPreview(false),
 *   (f) a RE-EDIT opens no session at all (PreviewOp runs on HEAD — previewing a
 *       feature's own edit would double-apply it),
 *   (g) the ~0° confirm refusal survives the new commit path.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import {
  ModelToolController,
  __setBodyLoadTimeoutForTests,
  __setExactPreviewTimeoutForTests,
} from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  AxisRef,
  FinishSketchResult,
  PreviewDraft,
  PreviewParams,
  PreviewResult,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/** Two disjoint squares in plane (u,v); the axis at u=50 sits BETWEEN them, so it
 *  splits neither profile and is valid for both (Wave 2 all-regions validity). */
const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};
const R1: SketchRegion = {
  regionId: "r1",
  outerLoop: [],
  holes: [],
  previewTriangles: {
    positions: [100, 100, 140, 100, 140, 140, 100, 140],
    indices: [0, 1, 2, 0, 2, 3],
  },
};

const AXIS_OK = {
  id: "axOk",
  type: "Line" as const,
  p0: [50, -50] as [number, number],
  p1: [50, 200] as [number, number],
};

const ok = (bodyId: string): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId, meshKey: `${bodyId}#0` }],
  removedBodies: [],
});

function makeSession(entities: SketchSession["entities"]): SketchSession {
  return { sketchId: "sk", plane: PLANE, entities, constraints: [], dof: 0, status: "FullyConstrained" };
}

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    setRegionSelected: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    planePixelWorld: vi.fn(() => 1),
    probePick: vi.fn(() => null),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => ({ origin: [0, 0, 100] as const, dir: [0, 0, -1] as const })),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    clearPreviewBody: vi.fn(),
    showRevolveAxisCandidates: vi.fn(),
    setRevolveAxisHover: vi.fn(),
    showRevolvePreview: vi.fn(),
    setRevolveAngle: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
  };
}

interface ClientOpts {
  regions: SketchRegion[];
  /** Per-call `endPreview` results (index k = the k-th commit); default: success. */
  endResults?: ApplyOperationResult[];
  storedParams?: Record<string, unknown>;
  /** Sketch entities the axis candidates are resolved from (default: one real line). */
  axisEntities?: SketchSession["entities"];
}

function makeClientMock(opts: ClientOpts, capture?: (cb: (r: PreviewResult) => void) => void) {
  let seq = 0;
  let endCall = 0;
  return {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capture?.(cb);
      return () => {};
    }),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: opts.regions })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: opts.regions })),
    getSketch: vi.fn(() => Promise.resolve(makeSession(opts.axisEntities ?? [AXIS_OK]))),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn((_id: string, _params: PreviewParams, _epoch: number) => {}),
    endPreview: vi.fn((_id: string, _commit: boolean) =>
      Promise.resolve(opts.endResults ? (opts.endResults[endCall++] ?? ok(`b${endCall}`)) : ok(`b${++endCall}`)),
    ),
    applyOperation: vi.fn(() => Promise.resolve(ok("adhoc"))),
    applyEditCommand: vi.fn(() => Promise.resolve(ok("edit"))),
    undo: vi.fn(() => Promise.resolve(ok("undone"))),
    getOperationParams: vi.fn(
      (): Promise<Record<string, unknown>> => Promise.resolve(opts.storedParams ?? {}),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
const waitMs = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

function debug(): Record<string, unknown> {
  return (window as unknown as { __extrudePreview?: Record<string, unknown> }).__extrudePreview ?? {};
}

describe("ModelToolController revolve kernel preview", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let previewCb: ((r: PreviewResult) => void) | null;

  function build(opts: ClientOpts): void {
    engineMock = makeEngineMock();
    previewCb = null;
    clientMock = makeClientMock(opts, (cb) => {
      previewCb = cb;
    });
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0); // most specs assert sequencing, not the barrier
    selectionStore.getState().set([]);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
    __setBodyLoadTimeoutForTests(4000);
  });

  function pointer(type: string, x: number, y: number, button: number, buttons: number): void {
    container.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true }));
  }

  function click(x: number, y: number): void {
    pointer("pointerdown", x, y, 0, 1);
    pointer("pointerup", x, y, 0, 0);
  }

  /** Arm revolve on `sk` and pick the single axis candidate → armed + sessions open. */
  async function armAndPickAxis(): Promise<void> {
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();
    click(50, 50); // identity screen→plane: lands on the u=50 axis line
    await flush();
    await flush();
  }

  /** Confirm the multi-region pick (toggle both regions + Enter), then pick the axis. */
  async function armTwoRegionsAndPickAxis(): Promise<void> {
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();
    click(10, 10); // toggle r0
    await flush();
    click(130, 110); // toggle r1
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    click(50, 50);
    await flush();
    await flush();
  }

  /** Session id + epoch of the newest `updatePreview` the controller pushed. */
  function lastUpdate(): { sessionId: string; params: PreviewParams; epoch: number } {
    const calls = clientMock.updatePreview.mock.calls;
    const last = calls[calls.length - 1];
    return { sessionId: last[0], params: last[1], epoch: last[2] };
  }

  // ── (a) the axis is what opens the lane ──────────────────────────────────────

  it("opens NO session during axis pick, then one typed Revolve draft per region on the pick", async () => {
    build({ regions: [R0, R1] });
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();
    click(10, 10);
    await flush();
    click(130, 110);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();

    // axisPick: the draft has no axis yet, so nothing may be opened.
    expect(engineMock.showRevolveAxisCandidates).toHaveBeenCalledTimes(1);
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(debug().previewOwner).toBeNull();

    click(50, 50);
    await flush();
    await flush();

    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);
    const drafts = clientMock.beginPreview.mock.calls.map((c) => c[0]);
    expect(drafts.map((d) => d.opType)).toEqual(["Revolve", "Revolve"]);
    expect(drafts.map((d) => d.sketchId)).toEqual(["sk", "sk"]);
    expect(drafts.map((d) => d.regionId)).toEqual(["r0", "r1"]);
    for (const d of drafts) {
      expect(d.params.angleDeg).toBe(360);
      expect(d.params.booleanMode).toBe("NewBody");
      expect(d.params.axis).toEqual({ kind: "sketchLine", sketchId: "sk", lineId: "axOk" } as AxisRef);
      expect(d.params.targetBodyId).toBeUndefined();
      expect(d.params.featureId).toBeUndefined();
    }
    // The lane is now revolve-owned, and the initial exact L2 went out to BOTH.
    expect(debug().previewOwner).toBe("revolve");
    const initial = clientMock.updatePreview.mock.calls;
    expect(initial.map((c) => c[0])).toEqual(["pv-1", "pv-2"]);
    expect(initial.map((c) => c[2])).toEqual([1, 1]); // one shared epoch
  });

  // W1-B pin: a CONSTRUCTION line is a first-class revolve axis (the classic
  // centerline workflow). Axis candidates filter on `type === "Line"` only — the
  // construction flag excludes geometry from REGIONS, never from being an axis.
  it("accepts a CONSTRUCTION line as an axis candidate", async () => {
    build({ regions: [R0], axisEntities: [{ ...AXIS_OK, construction: true }] });
    await armAndPickAxis();

    // It was OFFERED as a candidate (the engine gets the endpoints only)…
    const candidates = engineMock.showRevolveAxisCandidates.mock.calls[0][1] as {
      a: [number, number];
      b: [number, number];
    }[];
    expect(candidates).toEqual([{ a: AXIS_OK.p0, b: AXIS_OK.p1 }]);
    // …and the pick BOUND it: the lathe + a kernel preview session opened on a
    // draft whose axis names the construction line. Nothing about construction
    // geometry blocks it — it only leaves region detection.
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1);
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    const draft = clientMock.beginPreview.mock.calls[0][0] as PreviewDraft;
    expect(draft.params.axis as AxisRef).toMatchObject({ kind: "sketchLine", lineId: "axOk" });
  });

  it("keeps the L1 lathe under the exact preview (the drag handle never disappears)", async () => {
    build({ regions: [R0] });
    await armAndPickAxis();
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1); // L1 still drawn
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
  });

  // ── (b) drag / chip / boolean-mode push epoch-bumped updates ─────────────────

  it("an angle DRAG pushes a newer epoch carrying the dragged angle", async () => {
    build({ regions: [R0] });
    await armAndPickAxis();
    const first = lastUpdate();
    expect(first.epoch).toBe(1);
    expect(first.params.angleDeg).toBe(360);

    // Answer the in-flight epoch so the throttle can release the next send.
    previewCb?.({ sessionId: first.sessionId, epoch: first.epoch, bodyId: "pb-1", bodies: [] });

    pointer("pointerdown", 200, 100, 0, 1);
    pointer("pointermove", 110, 100, 0, 1); // leftward drag past DRAG_PX → angle drag
    await waitMs(220); // past the throttle's trailing window

    const latest = lastUpdate();
    expect(latest.epoch).toBeGreaterThan(first.epoch);
    expect(latest.params.angleDeg).not.toBe(360);
    // The chip mirrors the dragged angle — the exact candidate carries the SAME one.
    expect(latest.params.angleDeg).toBe(toolChipStore.getState().value);
  });

  it("a chip angle edit pushes a newer epoch carrying the typed angle", async () => {
    build({ regions: [R0] });
    await armAndPickAxis();
    const first = lastUpdate();
    previewCb?.({ sessionId: first.sessionId, epoch: first.epoch, bodyId: "pb-1", bodies: [] });

    toolChipStore.getState().onValue?.(90);
    await waitMs(220);

    const latest = lastUpdate();
    expect(latest.epoch).toBeGreaterThan(first.epoch);
    expect(latest.params.angleDeg).toBe(90);
  });

  it("switching to Cut re-sends the candidate with the boolean target (the subtraction frame)", async () => {
    build({ regions: [R0] });
    await armAndPickAxis();
    const first = lastUpdate();
    previewCb?.({ sessionId: first.sessionId, epoch: first.epoch, bodyId: "pb-1", bodies: [] });

    // seedMockDocument leaves body1 the sole visible body → Cut auto-targets it.
    toolChipStore.getState().onBooleanMode?.("Cut");
    await waitMs(220);

    const latest = lastUpdate();
    expect(latest.epoch).toBeGreaterThan(first.epoch);
    expect(latest.params.booleanMode).toBe("Cut");
    expect(latest.params.targetBodyId).toBe("body1");
    expect(engineMock.setPreviewTint).toHaveBeenCalledWith("cut"); // Cut tint reaches revolve
  });

  // ── (c) confirm = flush → barrier → endPreview(true) ─────────────────────────

  it("confirm flushes the final params, waits for the exact candidate, then commits IT", async () => {
    __setExactPreviewTimeoutForTests(2000); // long — the barrier must be what resolves
    build({ regions: [R0] });
    await armAndPickAxis();
    toolChipStore.getState().onValue?.(120);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();

    // The final params went out as a fresh epoch; nothing committed yet.
    const final = lastUpdate();
    expect(final.params.angleDeg).toBe(120);
    expect(clientMock.endPreview).not.toHaveBeenCalled();

    previewCb?.({ sessionId: final.sessionId, epoch: final.epoch, bodyId: "pb-1", bodies: [] });
    await flush();
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
    expect(clientMock.applyOperation).not.toHaveBeenCalled(); // never a second ad-hoc op
  });

  it("the success hint SURVIVES finishRevolve's reset to select", async () => {
    // Hint-clobber regression: `finishRevolve` published "Revolved" and THEN called
    // setTool("select"), whose synchronous sweep cleared it.
    __setBodyLoadTimeoutForTests(0); // no mesh ingest here — finish on the bounded wait
    build({ regions: [R0] });
    await armAndPickAxis();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    for (let i = 0; i < 6; i++) await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
    expect(toolStore.getState().modelTool).toBe("select");
    expect(viewportStore.getState().statusHint?.message).toBe("Revolved");
  });

  it("proceeds with the commit when no exact candidate ever arrives (bounded barrier)", async () => {
    __setExactPreviewTimeoutForTests(10);
    build({ regions: [R0] });
    await armAndPickAxis();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();
    expect(clientMock.endPreview).not.toHaveBeenCalled(); // still on the barrier

    await waitMs(60); // past the barrier timeout
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
  });

  it("commits one previewed candidate per region, in region order", async () => {
    build({ regions: [R0, R1] });
    await armTwoRegionsAndPickAxis();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    for (let i = 0; i < 6; i++) await flush();

    const committed = clientMock.endPreview.mock.calls.filter((c) => c[1] === true);
    expect(committed.map((c) => c[0])).toEqual(["pv-1", "pv-2"]);
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
  });

  // ── (d) a failed final preview blocks + re-arms ──────────────────────────────

  it("a failed final exact preview blocks the commit, releases the lane, and re-arms", async () => {
    __setExactPreviewTimeoutForTests(2000);
    build({ regions: [R0] });
    await armAndPickAxis();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();
    const final = lastUpdate();

    previewCb?.({
      sessionId: final.sessionId,
      epoch: final.epoch,
      bodyId: "pb-1",
      error: { kind: "opFailed", message: "axis intersects the profile", structural: false },
    });
    for (let i = 0; i < 4; i++) await flush();

    expect(clientMock.endPreview).not.toHaveBeenCalledWith("pv-1", true); // NO commit
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", false); // lane released
    expect(debug().revolvePhase).toBe("armed"); // work kept
    expect(toolStore.getState().phase).toBe("armed");
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2); // session RE-ARMED
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("axis intersects the profile");
  });

  it("a failed commit result rolls back, stays armed, and re-arms the lane session", async () => {
    build({
      regions: [R0],
      endResults: [{ revision: 1, features: [], changedBodies: [], removedBodies: [], errorMessage: "worker exploded" }],
    });
    await armAndPickAxis();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    for (let i = 0; i < 6; i++) await flush();

    expect(clientMock.undo).toHaveBeenCalledTimes(1); // errored record popped
    expect(debug().revolvePhase).toBe("armed");
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);
    expect(viewportStore.getState().statusHint?.message).toContain("worker exploded");
  });

  // ── (e) every teardown path releases every session ───────────────────────────

  it("resetting the axis closes every session (no well-formed candidate without one)", async () => {
    build({ regions: [R0, R1] });
    await armTwoRegionsAndPickAxis();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);

    toolChipStore.getState().onResetAxis?.();
    await flush();

    const released = clientMock.endPreview.mock.calls.filter((c) => c[1] === false);
    expect(released.map((c) => c[0]).sort()).toEqual(["pv-1", "pv-2"]);
    expect(debug().previewOwner).toBeNull();
  });

  it("a tool switch ends every open session with endPreview(false)", async () => {
    build({ regions: [R0, R1] });
    await armTwoRegionsAndPickAxis();
    clientMock.endPreview.mockClear();

    toolStore.getState().setTool("select");
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledTimes(2);
    for (const call of clientMock.endPreview.mock.calls) expect(call[1]).toBe(false);
    expect(debug().previewOwner).toBeNull();
  });

  it("dispose ends every open session with endPreview(false) — no lane leak", async () => {
    build({ regions: [R0, R1] });
    await armTwoRegionsAndPickAxis();
    clientMock.endPreview.mockClear();

    controller.dispose();

    expect(clientMock.endPreview).toHaveBeenCalledTimes(2);
    for (const call of clientMock.endPreview.mock.calls) expect(call[1]).toBe(false);
  });

  it("a sketch geometryToken bump under the armed revolve releases the sessions", async () => {
    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch X",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v1",
    });
    build({ regions: [R0] });
    await armAndPickAxis();
    clientMock.endPreview.mockClear();

    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch X",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v2", // the sketch was edited underneath the arm
    });
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(viewportStore.getState().statusHint?.message).toContain("Sketch changed");
  });

  // ── (f) a re-edit is L1-only ─────────────────────────────────────────────────

  it("a feature RE-EDIT opens NO preview session (PreviewOp runs on HEAD)", async () => {
    documentStore.setState({
      sketches: { sk: { id: "sk", name: "Sketch", visible: true, dof: 0, status: "ok", geometryToken: "t" } },
      features: [{ id: "rev-1", kind: "revolve", label: "Revolve", valueText: "90°", status: "ok" }],
    });
    build({
      regions: [R0],
      storedParams: {
        profile: { sketchId: "sk", regionId: "r0" },
        angleDeg: { value: 90 },
        axis: { kind: "sketchLine", sketchId: "sk", lineId: "axOk" },
      },
    });

    controller.editRevolveFeature("rev-1");
    for (let i = 0; i < 4; i++) await flush();

    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1); // L1 lathe only
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(clientMock.updatePreview).not.toHaveBeenCalled();
    expect(debug().previewOwner).toBeNull();

    // …and the commit is still the angle-only deep-merge, never a lane commit.
    toolChipStore.getState().onConfirm?.();
    for (let i = 0; i < 4; i++) await flush();
    expect(clientMock.applyEditCommand).toHaveBeenCalledTimes(1);
    expect(clientMock.endPreview).not.toHaveBeenCalled();
  });

  // ── (g) the degenerate-angle refusal survives the new commit path ────────────

  it("a ~0° confirm is still refused — nothing previewed is committed", async () => {
    build({ regions: [R0] });
    await armAndPickAxis();

    toolChipStore.getState().onValue?.(0.2);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    for (let i = 0; i < 4; i++) await flush();

    expect(clientMock.endPreview).not.toHaveBeenCalled();
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
    expect(debug().revolvePhase).toBe("armed");
    expect(viewportStore.getState().statusHint?.message).toMatch(/non-zero angle/i);
  });

  it("a structural preview failure blocks confirm until a good candidate replaces it", async () => {
    build({ regions: [R0] });
    await armAndPickAxis();
    const first = lastUpdate();
    previewCb?.({
      sessionId: first.sessionId,
      epoch: first.epoch,
      bodyId: "pb-1",
      error: { kind: "needsRepair", message: "axis line unresolved", structural: true },
    });

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    for (let i = 0; i < 4; i++) await flush();

    expect(clientMock.endPreview).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toContain("Cannot confirm");
    expect(debug().revolvePhase).toBe("armed");
  });
});
