/*
 * ModelToolController Wave 2 (jsdom): boolean modes (auto-target vs targetPick +
 * Cut tint), multi-region N-op commit with stop-on-failure re-arm, and the revolve
 * N-loop with all-regions axis validity. Engine + client are faked; the debug
 * surface (deps.debug) exposes the FSM phase / boolean target for assertions.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
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
    setExtrudePreviewTint: vi.fn(),
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
      getSketch: vi.fn(() => Promise.resolve(makeSession(opts.entities))),
      beginPreview: vi.fn((_d: PreviewDraft) => Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` })),
      updatePreview: vi.fn(),
      endPreview: vi.fn(() => Promise.resolve(opts.endResults ? opts.endResults[endCall++] ?? ok(`b${endCall}`) : ok(`b${endCall++}`))),
      applyOperation: vi.fn((_op: OperationOp) => Promise.resolve(opts.applyResults ? opts.applyResults[applyCall++] ?? ok(`rv${applyCall}`) : ok(`rv${applyCall++}`))),
      applyEditCommand: vi.fn(() => Promise.resolve(ok("edit"))),
      getOperationParams: vi.fn(() => Promise.resolve({})),
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
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    // One visible body by default (seedMockDocument gives body1) → boolean available.
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
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
    expect(engineMock.setExtrudePreviewTint).toHaveBeenCalledWith("cut");
    controllerBoolean("NewBody");
    expect(engineMock.setExtrudePreviewTint).toHaveBeenLastCalledWith("normal");
    expect(debug().booleanTargetId).toBeNull();
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

  // ── multi-region N-op commit, stop-on-failure ─────────────────────────────────

  it("commits N regions sequentially; a mid-loop failure keeps the committed ones and re-arms the rest", async () => {
    // endPreview: region0 ok, region1 FAILS, region2 never reached.
    build({
      finish: () => Promise.resolve({ regions: [R0, R1, R2] }),
      endResults: [ok("body-a"), fail("worker exploded")],
    });
    await armExtrude();
    // Multi-select: toggle all three, then Enter confirms the selection.
    click(10, 10);
    await flush();
    click(130, 110);
    await flush();
    click(210, 210);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(3); // one session per region

    // Confirm the armed 3-region extrude.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush(); // let the re-arm beginPreview resolve

    // Stopped after the 2nd endPreview (region1 failed) — region2 never committed.
    expect(clientMock.endPreview).toHaveBeenCalledTimes(2);
    expect(controller.extrudeActive).toBe(true); // re-armed (work kept)
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toMatch(/Extrude 2 of 3 failed: worker exploded/);
    // Committed region0 dropped; failed region1 re-begun (4th beginPreview), region2 kept.
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(4);
    expect(clientMock.beginPreview.mock.calls[3][0].regionId).toBe("r1");
    expect(debug().regionCount).toBe(2);
  });

  it("commits N regions and, on full success, selects all bodies + auto-hides the sketch", async () => {
    // Register the sketch in the tree so the auto-hide has something to flip.
    documentStore.getState().addSketch({ id: "sk", name: "Sketch X", visible: true, dof: 0, status: "ok" });
    build({ finish: () => Promise.resolve({ regions: [R0, R1] }), endResults: [ok("body-a"), ok("body-b")] });
    await armExtrude();
    click(10, 10);
    await flush();
    click(130, 110);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" })); // confirm
    await flush();

    // Both committed → the reconcile waits for both bodies to load.
    bodyCbs.forEach((cb) => { cb("body-a"); cb("body-b"); });
    await flush();
    expect(controller.extrudeActive).toBe(false);
    expect(selectionStore.getState().selected).toEqual([
      { kind: "body", id: "body-a" },
      { kind: "body", id: "body-b" },
    ]);
    expect(documentStore.getState().sketches.sk.visible).toBe(false); // consumed sketch hidden
  });

  // ── revolve N-loop + all-regions axis validity ────────────────────────────────

  it("revolve: an axis that splits ONE selected region is rejected (all-regions validity)", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0, R1] }), entities: [AXIS_OK, AXIS_BAD] });
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

  it("revolve: a valid axis arms, then confirm loops one applyOperation per region", async () => {
    build({ finish: () => Promise.resolve({ regions: [R0, R1] }), entities: [AXIS_OK, AXIS_BAD] });
    toolStore.getState().setTool("revolve");
    await flush();
    click(10, 10);
    await flush();
    click(130, 110);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();

    // The valid axis (u=50) arms both regions.
    click(50, 50);
    await flush();
    expect(engineMock.showRevolvePreview).toHaveBeenCalledTimes(1);

    // Confirm → one Revolve op per region, same axis + angle.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(clientMock.applyOperation).toHaveBeenCalledTimes(2);
    const regionIds = clientMock.applyOperation.mock.calls.map((c) => {
      const op = c[0];
      if (op.opType !== "Revolve") throw new Error("expected Revolve");
      return op.regionId;
    });
    expect(regionIds).toEqual(["r0", "r1"]);
  });
});
