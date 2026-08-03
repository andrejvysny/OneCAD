/*
 * ModelToolController multi-region pick (jsdom): a finished sketch with >1 closed
 * region hands pointer ownership to the controller (orbit suppressed) so the user
 * clicks WHICH region to extrude/revolve; the picked regionId flows into the
 * draft (extrude) / commit op (revolve). A single region keeps the instant
 * auto-arm path. Esc cancels + restores orbit. A stale finishSketch result (the
 * arm was re-triggered) is dropped. Engine + client are faked (no WebGL / backend);
 * only the methods the controller drives are stubbed.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  OperationOp,
  PreviewDraft,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

// Two disjoint square regions, plane (u,v). Fan layout (index 0 = centroid) so
// profileFromRegion accepts each. r0 near origin; r1 far off.
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
  previewTriangles: { positions: [100, 100, 140, 100, 140, 140, 100, 140], indices: [0, 1, 2, 0, 2, 3] },
};

// A region the profile builder REFUSES (degenerate triangulation) — the pick has
// nothing extrudable to offer, so it must refuse with a hint instead of arming.
const R_DEGENERATE: SketchRegion = {
  regionId: "rbad",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0], indices: [] },
};

// A vertical axis line to the LEFT of r1 (valid: does not cross the profile).
const AXIS_LINE = { id: "axis1", type: "Line" as const, p0: [95, 100] as [number, number], p1: [95, 140] as [number, number] };

const OP_RESULT: ApplyOperationResult = {
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }],
  removedBodies: [],
};

function makeSession(): SketchSession {
  return { sketchId: "sk", plane: PLANE, entities: [AXIS_LINE], constraints: [], dof: 0, status: "FullyConstrained" };
}

function makeEngineMock() {
  return {
    // Region pick facade (under test).
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
    // Identity plane mapping: client (x,y) → plane (u,v) so the region fixtures live
    // directly in pointer space.
    screenToPlaneOn: vi.fn((_plane: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    setRegionSelected: vi.fn(),
    planePixelWorld: vi.fn(() => 1),
    probePick: vi.fn(() => null),
    // Extrude preview.
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => null),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    setPreviewBody: vi.fn(),
    clearPreviewBody: vi.fn(),
    // Revolve preview.
    showRevolveAxisCandidates: vi.fn(),
    setRevolveAxisHover: vi.fn(),
    showRevolvePreview: vi.fn(),
    setRevolveAngle: vi.fn(),
    hideRevolvePreview: vi.fn(),
    // Cancel-path teardown (fired on every tool switch).
    hideGhostPreview: vi.fn(),
  };
}

function makeClientMock(finish: () => Promise<FinishSketchResult>) {
  return {
    onPreviewResult: vi.fn(() => () => {}),
    finishSketch: vi.fn(finish),
    getSketchRegions: vi.fn(finish),
    // Model-mode arms read the sketch via getSketch (pure read; MODEL-HARDEN W0.5).
    getSketch: vi.fn(() => Promise.resolve(makeSession())),
    enterSketch: vi.fn(() => Promise.resolve(makeSession())),
    beginPreview: vi.fn((_draft: PreviewDraft) => Promise.resolve({ sessionId: "sess", previewBodyId: "pb" })),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(OP_RESULT)),
    applyOperation: vi.fn((_op: OperationOp) => Promise.resolve(OP_RESULT)),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("ModelToolController region pick", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let onBodyLoadedCbs: Array<(id: string) => void>;

  function build(finish: () => Promise<FinishSketchResult>): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock(finish);
    onBodyLoadedCbs = [];
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: (cb) => {
        onBodyLoadedCbs.push(cb);
        return () => {};
      },
    });
  }

  beforeEach(() => {
    resetStores();
    // No preview results are delivered here (onPreviewResult is a bare stub), so
    // collapse the commit-time exact-preview barrier to one macrotask.
    __setExactPreviewTimeoutForTests(0);
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
    ]);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  function pointer(type: string, x: number, y: number, button: number, buttons: number): void {
    container.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true }));
  }

  /** A plain click gesture (down + up, no move) at a client point. */
  function click(x: number, y: number): void {
    pointer("pointerdown", x, y, 0, 1);
    pointer("pointerup", x, y, 0, 0);
  }

  it("(a) exact persistent region selection arms the non-first region directly", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r1-ref", sketchId: "sk", regionId: "r1" },
    ]);
    toolStore.getState().setTool("extrude");
    await flush();

    expect(engineMock.showRegionPick).not.toHaveBeenCalled();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    const draft = clientMock.beginPreview.mock.calls[0][0];
    expect(draft.opType).toBe("Extrude");
    expect(draft.regionId).toBe("r1");
    expect(controller.extrudeActive).toBe(true);
  });

  it("(a-multi) multiple selected regions are rejected (extrude is single-profile)", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
      { kind: "sketchRegion", id: "r1-ref", sketchId: "sk", regionId: "r1" },
    ]);
    toolStore.getState().setTool("extrude");
    await flush();

    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(controller.extrudeActive).toBe(false);
    expect(viewportStore.getState().statusHint?.message).toMatch(/exactly one/);
  });

  it("(a-whole) whole-sketch selection opens the region PICK — never a guessed profile", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();

    // Tool-first: the pick UI opens over BOTH regions; nothing armed, nothing guessed.
    expect(clientMock.getSketchRegions).toHaveBeenCalledWith("sk");
    expect(engineMock.showRegionPick).toHaveBeenCalled();
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(controller.extrudeActive).toBe(false);
  });

  it("(a-whole-click) a click on a region during the extrude pick arms it immediately", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();
    expect(engineMock.showRegionPick).toHaveBeenCalled();

    click(120, 120); // inside R1's fixture square (identity screen→plane mapping)
    await flush();
    await flush();

    expect(controller.extrudeActive).toBe(true);
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    const draft = clientMock.beginPreview.mock.calls[0][0];
    expect(draft.regionId).toBe("r1"); // the CLICKED region, not the first
  });

  it("(a2) revolve two regions → toggle region 2 + Enter → axis pick opens the lane on regions[1] → 360° commit", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    expect(engineMock.showRegionPick).toHaveBeenCalledTimes(1);

    // Toggle region 2 + Enter → enters axis-pick (region pick precedes the axis phase).
    click(130, 110);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(engineMock.showRevolveAxisCandidates).toHaveBeenCalledTimes(1);

    // Pick the axis line (click near it) → the arm ALSO opens the kernel-preview lane
    // session, bound to the exact picked region.
    click(95, 120);
    await flush();
    await flush();
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1);
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    const draft = clientMock.beginPreview.mock.calls[0][0];
    expect(draft.opType).toBe("Revolve");
    expect(draft.regionId).toBe("r1"); // the TOGGLED region, not the first

    // A plain click away commits the default 360° — through the lane session, so the
    // previewed candidate IS what materializes (never a second ad-hoc op).
    click(200, 200);
    await flush();
    await flush();
    await flush();

    expect(clientMock.applyOperation).not.toHaveBeenCalled();
    expect(clientMock.endPreview).toHaveBeenCalledWith("sess", true);
  });

  it("(b) one selected region arms without a transient region picker", async () => {
    build(() => Promise.resolve({ regions: [R0] }));
    toolStore.getState().setTool("extrude");
    await flush();

    expect(engineMock.showRegionPick).not.toHaveBeenCalled();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    const draft = clientMock.beginPreview.mock.calls[0][0];
    expect(draft.regionId).toBe("r0");
    expect(controller.extrudeActive).toBe(true);
  });

  it("(c) Esc during region pick cancels cleanly and restores orbit", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    expect(engineMock.showRegionPick).toHaveBeenCalledTimes(1);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await flush();

    expect(engineMock.hideRegionPick).toHaveBeenCalled();
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
    expect(toolStore.getState().modelTool).toBe("select");
    // A later click resolves nothing (pick torn down) — no arm.
    click(130, 110);
    await flush();
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
  });

  it("(c2) a pick with ZERO extrudable regions refuses with a hint that SURVIVES the reset to select", async () => {
    // Hint-clobber regression: the refusal used to publish the hint and THEN call
    // setTool("select"), whose synchronous tool-change sweep cleared it — the user
    // saw an inert toolbar and no reason at all.
    build(() => Promise.resolve({ regions: [R_DEGENERATE] }));
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();

    expect(engineMock.showRegionPick).not.toHaveBeenCalled();
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(toolStore.getState().modelTool).toBe("select");
    expect(viewportStore.getState().statusHint?.message).toBe("No closed region to extrude");
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });

  it("(c3) the same refusal on the REVOLVE pick keeps its hint too", async () => {
    build(() => Promise.resolve({ regions: [R_DEGENERATE, { ...R_DEGENERATE, regionId: "rbad2" }] }));
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();

    expect(engineMock.showRegionPick).not.toHaveBeenCalled();
    expect(toolStore.getState().modelTool).toBe("select");
    expect(viewportStore.getState().statusHint?.message).toBe("No closed region to revolve");
  });

  it("(d) a stale finishSketch result (arm re-triggered) is dropped — no second region pick", async () => {
    let resolveFirst!: (r: FinishSketchResult) => void;
    const firstFinish = new Promise<FinishSketchResult>((r) => {
      resolveFirst = r;
    });
    let call = 0;
    build(() => {
      call += 1;
      return call === 1 ? firstFinish : Promise.resolve({ regions: [R0, R1] });
    });

    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    // Arm 1: finishSketch is in flight.
    toolStore.getState().setTool("revolve");
    await flush();
    expect(engineMock.showRegionPick).not.toHaveBeenCalled();

    // Re-trigger the arm (select → extrude) while arm 1 is still pending.
    toolStore.getState().setTool("select");
    toolStore.getState().setTool("revolve");
    await flush(); // arm 2: finishSketch resolved → enterSketch → region pick
    await flush();
    expect(engineMock.showRegionPick).toHaveBeenCalledTimes(1);

    // The stale arm-1 result finally lands — it must NOT open a second region pick.
    resolveFirst({ regions: [R0, R1] });
    await flush();
    expect(engineMock.showRegionPick).toHaveBeenCalledTimes(1);
  });

  // ── WP-C3: revolve region parity ───────────────────────────────────────────
  //
  // Revolve used to ignore a typed `sketchRegion` selection entirely: it read a
  // whole SKETCH and re-derived a profile from it, so pressing Revolve with a
  // region already picked could revolve a different region than the one on
  // screen (and the old lane reached that sketch through `finishSketch`, which
  // needs a live backend session and therefore THREW on a reopened document).
  // Acquisition is now the same pure `getSketchRegions` read extrude uses.

  it("(e) a selected region binds EXACTLY that region — never regions[0]", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r1-ref", sketchId: "sk", regionId: "r1" },
    ]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();

    // No region picker: the selection already named the profile.
    expect(engineMock.showRegionPick).not.toHaveBeenCalled();
    expect(clientMock.getSketchRegions).toHaveBeenCalledWith("sk");
    expect(engineMock.showRevolveAxisCandidates).toHaveBeenCalledTimes(1);

    click(95, 120); // the axis line to the LEFT of r1
    await flush();
    await flush();
    const draft = clientMock.beginPreview.mock.calls[0][0];
    expect(draft.opType).toBe("Revolve");
    expect(draft.regionId).toBe("r1");
  });

  it("(f) arming on a REOPENED sketch never calls finishSketch (no live session needed)", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    // A sketch that was never ENTERED in this session: the old lane's finishSketch
    // is exactly what used to throw here. Make it fail loudly so any surviving
    // call is a red test rather than a silent dependency.
    clientMock.finishSketch = vi.fn(() => Promise.reject(new Error("no live session")));
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
    ]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();

    expect(clientMock.finishSketch).not.toHaveBeenCalled();
    expect(clientMock.getSketchRegions).toHaveBeenCalledWith("sk");
    expect(clientMock.getSketch).toHaveBeenCalledWith("sk");
    expect(engineMock.showRevolveAxisCandidates).toHaveBeenCalledTimes(1);
  });

  it("(g) a STALE selected regionId refuses loudly with the available ids, arming nothing", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "gone-ref", sketchId: "sk", regionId: "r_gone" },
    ]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();

    expect(engineMock.showRevolveAxisCandidates).not.toHaveBeenCalled();
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(toolStore.getState().modelTool).toBe("select");
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Revolve region r_gone is stale or invalid; available: r0, r1",
    );
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });

  it("(h) N selected regions keep the multi-region commit loop (revolve is not single-profile)", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
      { kind: "sketchRegion", id: "r1-ref", sketchId: "sk", regionId: "r1" },
    ]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();
    expect(engineMock.showRegionPick).not.toHaveBeenCalled(); // the selection IS the pick

    click(95, 120); // axis line — valid for BOTH fixtures (they sit to its right)
    await flush();
    await flush();
    // One preview session per bound region, in selection order.
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);
    expect(clientMock.beginPreview.mock.calls.map((c) => c[0].regionId)).toEqual(["r0", "r1"]);
  });

  it("(i) regions from TWO sketches refuse rather than guessing which sketch wins", async () => {
    build(() => Promise.resolve({ regions: [R0, R1] }));
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
      { kind: "sketchRegion", id: "other", sketchId: "sk2", regionId: "r1" },
    ]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();

    expect(clientMock.getSketchRegions).not.toHaveBeenCalled();
    expect(engineMock.showRevolveAxisCandidates).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toMatch(/one sketch/);
  });
});
