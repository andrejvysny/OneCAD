/*
 * ModelToolController Wave 2 (jsdom): boolean modes (auto-target vs targetPick +
 * Cut tint), multi-region N-op commit with stop-on-failure re-arm, and the revolve
 * N-loop with all-regions axis validity. Engine + client are faked; the debug
 * surface (deps.debug) exposes the FSM phase / boolean target for assertions.
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
  FinishSketchResult,
  OperationOp,
  PreviewDraft,
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
import type { BooleanMode } from "./modelToolMachine";

/** Drive a boolean segment through the chip handler the controller registered. */
function controllerBoolean(mode: BooleanMode): void {
  toolChipStore.getState().onBooleanMode?.(mode);
}

const PLANE: SketchPlane = { kind: "XY", origin: [0, 0, 0], xAxis: [1, 0, 0], yAxis: [0, 1, 0], normal: [0, 0, 1] };

/** Three disjoint square regions in plane (u,v) (identity screen→plane mapping). */
const R0: SketchRegion = { regionId: "r0", outerLoop: [], holes: [], previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] } };
const R1: SketchRegion = { regionId: "r1", outerLoop: [], holes: [], previewTriangles: { positions: [100, 100, 140, 100, 140, 140, 100, 140], indices: [0, 1, 2, 0, 2, 3] } };
const R2: SketchRegion = { regionId: "r2", outerLoop: [], holes: [], previewTriangles: { positions: [200, 200, 220, 200, 220, 220, 200, 220], indices: [0, 1, 2, 0, 2, 3] } };

// Axis lines for the revolve tests: valid at u=50 (crosses neither r0 nor r1),
// bad at u=10 (splits r0). Both far from r1.
const AXIS_OK = { id: "axOk", type: "Line" as const, p0: [50, -50] as [number, number], p1: [50, 200] as [number, number] };
const AXIS_BAD = { id: "axBad", type: "Line" as const, p0: [10, -50] as [number, number], p1: [10, 200] as [number, number] };

const ok = (bodyId: string): ApplyOperationResult => ({ revision: 1, features: [], changedBodies: [{ bodyId, meshKey: `${bodyId}#0` }], removedBodies: [] });
const fail = (msg: string): ApplyOperationResult => ({ revision: 1, features: [], changedBodies: [], removedBodies: [], errorMessage: msg });

function makeSession(entities: SketchSession["entities"] = []): SketchSession {
  return { sketchId: "sk", plane: PLANE, entities, constraints: [], dof: 0, status: "FullyConstrained" };
}

function makeEngineMock(probeBodyId: string | null = null) {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    setRegionSelected: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    planePixelWorld: vi.fn(() => 1),
    probePick: vi.fn(() => (probeBodyId ? { bodyId: probeBodyId } : null)),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => ({ origin: [0, 0, 100] as const, dir: [0, 0, -1] as const })),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => true),
    setPreviewBody: vi.fn(),
    clearPreviewBody: vi.fn(),
    showRevolveAxisCandidates: vi.fn(),
    setRevolveAxisHover: vi.fn(),
    showRevolvePreview: vi.fn(),
    setRevolveAngle: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

function debug(): Record<string, unknown> {
  return (window as unknown as { __extrudePreview?: Record<string, unknown> }).__extrudePreview ?? {};
}

describe("ModelToolController Wave 2", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let bodyCbs: Array<(id: string) => void>;

  function makeClientMock(opts: {
    finish: () => Promise<FinishSketchResult>;
    endResults?: ApplyOperationResult[];
    applyResults?: ApplyOperationResult[];
    entities?: SketchSession["entities"];
  }) {
    let endCall = 0;
    let applyCall = 0;
    let seq = 0;
    return {
      onPreviewResult: vi.fn(() => () => {}),
      finishSketch: vi.fn(opts.finish),
      getSketchRegions: vi.fn(opts.finish),
      getSketch: vi.fn(() => Promise.resolve(makeSession(opts.entities))),
      beginPreview: vi.fn((_d: PreviewDraft) => Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` })),
      updatePreview: vi.fn(),
      endPreview: vi.fn(() => Promise.resolve(opts.endResults ? opts.endResults[endCall++] ?? ok(`b${endCall}`) : ok(`b${endCall++}`))),
      applyOperation: vi.fn((_op: OperationOp) => Promise.resolve(opts.applyResults ? opts.applyResults[applyCall++] ?? ok(`rv${applyCall}`) : ok(`rv${applyCall++}`))),
      applyEditCommand: vi.fn(() => Promise.resolve(ok("edit"))),
      getOperationParams: vi.fn(
        (): Promise<Record<string, unknown>> =>
          Promise.resolve({
          profile: { sketchId: "sk", regionId: "r0" },
          distance: { value: 10 },
          }),
      ),
    };
  }

  function build(clientOpts: Parameters<typeof makeClientMock>[0], probeBodyId: string | null = null): void {
    engineMock = makeEngineMock(probeBodyId);
    clientMock = makeClientMock(clientOpts);
    bodyCbs = [];
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: (cb) => {
        bodyCbs.push(cb);
        return () => {};
      },
      debug: true,
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
    // One visible body by default (seedMockDocument gives body1) → boolean available.
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  function click(x: number, y: number): void {
    container.dispatchEvent(new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }));
    container.dispatchEvent(new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }));
  }

  async function armExtrude(): Promise<void> {
    toolStore.getState().setTool("extrude");
    await flush();
  }

  // ── boolean modes ───────────────────────────────────────────────────────────

  it("Add with exactly ONE visible body auto-targets it (no targetPick); Cut tints destructive", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0] }) });
    await armExtrude();
    expect(debug().phase).toBe("armed");

    // seedMockDocument → body1 is the sole visible body → Add auto-targets it.
    controllerBoolean("Add");
    await flush();
    expect(debug().phase).toBe("armed"); // auto-target — no targetPick
    expect(debug().booleanTargetId).toBe("body1");

    controllerBoolean("Cut");
    expect(engineMock.setPreviewTint).toHaveBeenCalledWith("cut");
    controllerBoolean("NewBody");
    expect(engineMock.setPreviewTint).toHaveBeenLastCalledWith("normal");
    expect(debug().booleanTargetId).toBeNull();
  });

  it("W3: arming a tool leaves isolation; disarming back to select does NOT", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0] }) });

    // An op previews against bodies the isolate mask hides, and the preview's own
    // hide/restore snapshot would fight that mask — so arming drops isolation.
    viewportStore.setState({ isolatedBodyIds: ["body1"] });
    await armExtrude();
    expect(viewportStore.getState().isolatedBodyIds).toBeNull();
    // The arm's own prompt still has the last word (exitIsolate ran BEFORE it).
    expect(viewportStore.getState().statusHint?.message).toBeTruthy();

    // Going back to `select` is a DISARM — Esc's ladder owns isolation there, so
    // one Esc must not silently do both rungs.
    viewportStore.setState({ isolatedBodyIds: ["body1"] });
    toolStore.getState().setTool("select");
    await flush();
    expect(viewportStore.getState().isolatedBodyIds).toEqual(["body1"]);
  });

  it("Add with >1 visible body enters targetPick; a body click adopts the target", async () => {
    // A second visible body → Add must ask which one.
    documentStore.setState({ bodies: { body1: { id: "body1", name: "B1", visible: true }, body2: { id: "body2", name: "B2", visible: true } } });
    build({ finish: () => Promise.resolve({ regions: [R0] }) }, "body2");
    await armExtrude();

    controllerBoolean("Add");
    await flush();
    expect(debug().phase).toBe("targetPick");
    expect(viewportStore.getState().statusHint?.message).toMatch(/Click a body to Add to/);

    // A plain click resolves the target through the engine pick path (probePick → body2).
    click(30, 30);
    await flush();
    expect(debug().phase).toBe("armed");
    expect(debug().booleanTargetId).toBe("body2");
  });

  it("Add with NO visible body is inert (segment disabled)", async () => {
    documentStore.setState({ bodies: {} });
    build({ finish: () => Promise.resolve({ regions: [R0] }) });
    await armExtrude();
    controllerBoolean("Add");
    expect(debug().phase).toBe("armed");
    expect(debug().booleanTargetId).toBeNull();
  });

  // ── exact single-region commit, stop-on-failure ───────────────────────────────

  it("a failed exact-region commit re-arms the same region", async () => {
    build({
      finish: () => Promise.resolve({ regions: [R0, R1, R2] }),
      endResults: [fail("worker exploded")],
    });
    await armExtrude();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush(); // let the re-arm beginPreview resolve

    expect(clientMock.endPreview).toHaveBeenCalledTimes(1);
    expect(controller.extrudeActive).toBe(true);
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toMatch(/Extrude failed: worker exploded/);
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);
    expect(clientMock.beginPreview.mock.calls[1][0].regionId).toBe("r0");
    expect(debug().regionCount).toBe(1);
  });

  it("commits one exact region, selects the result, and auto-hides the sketch", async () => {
    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch X",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v1",
    });
    build({ finish: () => Promise.resolve({ regions: [R0, R1] }), endResults: [ok("body-a")] });
    await armExtrude();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();

    bodyCbs.forEach((cb) => cb("body-a"));
    await flush();
    expect(controller.extrudeActive).toBe(false);
    expect(selectionStore.getState().selected).toEqual([{ kind: "body", id: "body-a" }]);
    expect(documentStore.getState().sketches.sk.visible).toBe(false); // consumed sketch hidden
  });

  // ── revolve N-loop + all-regions axis validity ────────────────────────────────

  it("revolve: an axis that splits ONE selected region is rejected (all-regions validity)", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0, R1] }), entities: [AXIS_OK, AXIS_BAD] });
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    click(10, 10);
    await flush();
    click(130, 110);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(engineMock.showRevolveAxisCandidates).toHaveBeenCalledTimes(1);

    // The bad axis (u=10) splits r0 → rejected: no lathe preview, an error hint.
    click(10, 50);
    await flush();
    expect(engineMock.showRevolvePreview).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toMatch(/1 of 2 regions fail/);
  });

  it("revolve: a valid axis arms + opens one lane session per region, then confirm commits each", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0, R1] }), entities: [AXIS_OK, AXIS_BAD] });
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    click(10, 10);
    await flush();
    click(130, 110);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();

    // The valid axis (u=50) arms both regions AND opens their preview sessions —
    // the op is well-formed exactly when an axis exists.
    click(50, 50);
    await flush();
    await flush();
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1);
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);
    const drafts = clientMock.beginPreview.mock.calls.map((c) => c[0]);
    expect(drafts.map((d) => d.opType)).toEqual(["Revolve", "Revolve"]);
    expect(drafts.map((d) => d.regionId)).toEqual(["r0", "r1"]);

    // Confirm → each region commits its OWN previewed candidate (no ad-hoc second op).
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();
    await flush();
    await flush();
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
    const committed = (clientMock.endPreview.mock.calls as unknown[][]).filter((c) => c[1] === true);
    expect(committed.map((c) => c[0])).toEqual(["pv-1", "pv-2"]);
  });

  // ── MODEL-HARDEN findings ─────────────────────────────────────────────────────

  const waitMs = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

  it("selects the UNION of an op's scoped bodies incl split children (finding 2)", async () => {
    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch X",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v1",
    });
    // A single region whose op returns TWO bodies (a Cut split child).
    build({
      finish: () => Promise.resolve({ regions: [R0] }),
      endResults: [
        {
          revision: 1,
          features: [],
          changedBodies: [
            { bodyId: "s0", meshKey: "s0#1" },
            { bodyId: "s1", meshKey: "s1#1" },
          ],
          removedBodies: [],
        },
      ],
    });
    await armExtrude();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" })); // confirm single region
    await flush();
    bodyCbs.forEach((cb) => {
      cb("s0");
      cb("s1");
    });
    await flush();
    expect(controller.extrudeActive).toBe(false);
    expect(selectionStore.getState().selected).toEqual([
      { kind: "body", id: "s0" },
      { kind: "body", id: "s1" },
    ]);
  });

  it("a click-away on an SVG target inside a toolbar button does NOT commit (finding 5)", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0] }) });
    await armExtrude();
    expect(debug().phase).toBe("armed");

    // A toolbar icon: an <svg>/<path> (NOT an HTMLElement) inside a <button>.
    const button = document.createElement("button");
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    svg.appendChild(path);
    button.appendChild(svg);
    document.body.appendChild(button);

    // A true click targeting the SVG path — the OLD instanceof-HTMLElement gate let this
    // through and committed; `.closest("button")` now excludes it.
    path.dispatchEvent(new MouseEvent("pointerdown", { button: 0, clientX: 5, clientY: 5, bubbles: true }));
    path.dispatchEvent(new MouseEvent("pointerup", { button: 0, clientX: 5, clientY: 5, bubbles: true }));
    await flush();

    expect(clientMock.endPreview).not.toHaveBeenCalled();
    expect(debug().phase).toBe("armed");
    button.remove();
  });

  it("dispose ends the single open preview session with endPreview(false) (finding 6, N=1)", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0] }) });
    await armExtrude();
    clientMock.endPreview.mockClear();
    controller.dispose();
    expect(clientMock.endPreview).toHaveBeenCalledTimes(1);
    expect((clientMock.endPreview.mock.calls[0] as unknown[])[1]).toBe(false);
  });

  it("dispose ends EVERY open preview session with endPreview(false) (finding 6, N sessions)", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0, R1, R2] }) });
    await armExtrude();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    clientMock.endPreview.mockClear();
    controller.dispose();
    expect(clientMock.endPreview).toHaveBeenCalledTimes(1);
    for (const call of clientMock.endPreview.mock.calls) expect((call as unknown[])[1]).toBe(false);
  });

  it("a superseded re-edit revolve arm does NOT resurrect the preview after getOperationParams (finding 7)", async () => {
    selectionStore.getState().set([]); // no sketch selected → setTool('revolve') won't fresh-arm
    documentStore.setState({
      features: [{ id: "rev-1", kind: "revolve", label: "Revolve", valueText: "90°", status: "ok" }],
    });
    build({ finish: () => Promise.resolve({ regions: [R0] }), entities: [AXIS_OK] });
    let resolveParams!: (v: Record<string, unknown>) => void;
    clientMock.getOperationParams.mockReturnValue(new Promise((r) => (resolveParams = r)));

    controller.editRevolveFeature("rev-1");
    await flush(); // paused at the getOperationParams await (the FIRST await of the re-edit)
    toolStore.getState().setTool("select"); // supersede: invalidateArm() bumps armGen + cancelRevolve
    await flush();
    resolveParams({}); // the stale params resolve LATE
    await flush();

    // The guard after the await must have dropped the stale arm — no lathe preview, not armed.
    expect(engineMock.showRevolvePreview).not.toHaveBeenCalled();
    expect(debug().revolvePhase).not.toBe("armed");
  });

  it("finishes after the bounded wait when a committed body never loads (finding 8)", async () => {
    __setBodyLoadTimeoutForTests(10);
    try {
      build({ finish: () => Promise.resolve({ regions: [R0] }), endResults: [ok("never-loads")] });
      await armExtrude();
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" })); // confirm
      await flush();
      // The body never fires onBodyLoaded — the tool is still committing.
      expect(toolStore.getState().modelTool).toBe("extrude");
      await waitMs(30); // past the bounded wait
      expect(toolStore.getState().modelTool).toBe("select"); // finished anyway (no hang)
      expect(selectionStore.getState().selected).toEqual([{ kind: "body", id: "never-loads" }]);
    } finally {
      __setBodyLoadTimeoutForTests(4000);
    }
  });

  it("editExtrudeFeature while already armed ends the open lane session before re-arming (finding 10)", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0] }) });
    documentStore.setState({
      features: [{ id: "ex-1", kind: "extrude", label: "Extrude", valueText: "20 mm", status: "ok" }],
    });
    await armExtrude(); // one open preview session
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    clientMock.endPreview.mockClear();

    controller.editExtrudeFeature("ex-1"); // tool is ALREADY extrude → setTool is a no-op
    await flush();
    // The prior session was ended (finding 10 — no leak), and a fresh one armed.
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), false);
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);
  });

  it("clears stale L2 preview bodies before re-arming after a partial failure (finding 11)", async () => {
    build({
      finish: () => Promise.resolve({ regions: [R0, R1, R2] }),
      endResults: [fail("boom")],
    });
    await armExtrude();
    engineMock.clearPreviewBody.mockClear();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" })); // confirm fails
    await flush();
    await flush(); // let the re-arm resolve

    // The committed + failed sessions' stale L2 handles were cleared before re-arming.
    expect(engineMock.clearPreviewBody).toHaveBeenCalled();
    expect(controller.extrudeActive).toBe(true); // re-armed
  });

  it("revolve partial failure rebuilds the lathe preview for the REMAINING region (finding 12)", async () => {
    build({
      finish: () => Promise.resolve({ regions: [R0, R1] }),
      entities: [AXIS_OK, AXIS_BAD],
      endResults: [ok("rev-a"), fail("boom")],
    });
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    click(10, 10);
    await flush();
    click(130, 110);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" })); // confirm 2 regions
    await flush();
    click(50, 50); // valid axis → armed (first showRevolvePreview) + 2 lane sessions
    await flush();
    await flush();
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1);
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" })); // confirm → region0 ok, region1 fails
    await flush();
    await flush();
    await flush();
    await flush();
    // The stop-on-failure path re-armed and rebuilt the lathe preview for the remaining region.
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(2);
    expect(viewportStore.getState().statusHint?.message).toMatch(/Revolve 2 of 2 failed/);
    // …and re-opened the FAILED region's lane session so the work is never lost.
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(3);
  });

  it("confirmRevolve at a ~0° angle is refused with a 'non-zero angle' hint (finding 13)", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0] }), entities: [AXIS_OK] });
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    click(50, 50); // pick the valid axis → armed at 360°
    await flush();
    expect(debug().revolvePhase).toBe("armed");

    // Drive the angle to ~0 via the chip, then confirm (Enter).
    toolChipStore.getState().onValue?.(0.2);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();

    expect(clientMock.applyOperation).not.toHaveBeenCalled(); // no commit
    expect(debug().revolvePhase).toBe("armed"); // stays armed
    expect(viewportStore.getState().statusHint?.message).toMatch(/non-zero angle/i);
  });
});
