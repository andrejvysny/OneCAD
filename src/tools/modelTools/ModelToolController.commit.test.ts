/*
 * ModelToolController commit-gesture wiring (jsdom, MODEL-HARDEN Wave 1). Proves
 * the professional commit gesture: a drag RELEASE keeps the tool armed (no commit,
 * chip stays); Enter / chip-✓ / click-away confirm; a FAILED commit stays armed,
 * surfaces the reason, and RE-ARMS the preview so the work is never lost; Esc /
 * tool-switch tears every open preview session down. Engine + client are faked.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { Vector3 } from "three";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { PickHit } from "@/viewport/engine/Picker";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  PreviewDraft,
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
import { makeBoxMesh } from "@/ipc/mockMeshes";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

// One square region (fan layout, index 0 = centroid) so profileFromRegion accepts it.
const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }],
  removedBodies: [],
});

const failResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [],
  removedBodies: [],
  errorMessage: "worker exploded",
});

function makeSession(): SketchSession {
  return { sketchId: "sk", plane: PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
}

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    planePixelWorld: vi.fn(() => 1),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setRegionSelected: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => ({ origin: [0, 0, 100] as const, dir: [0, 0, -1] as const })),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => true),
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
    probePick: vi.fn<(clientX: number, clientY: number) => PickHit | null>(() => null),
  };
}

function makeClientMock(
  endResult: () => ApplyOperationResult,
  capturePreview?: (cb: (result: PreviewResult) => void) => void,
) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((cb: (result: PreviewResult) => void) => {
      capturePreview?.(cb);
      return () => {};
    }),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketch: vi.fn(() => Promise.resolve(makeSession())),
    beginPreview: vi.fn((_d: PreviewDraft) => Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` })),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(endResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
    promoteSelection: vi.fn(() => Promise.resolve([{ topoKey: "f:2", elementId: "el-face-2", kind: "face" }])),
    getOperationParams: vi.fn(() =>
      Promise.resolve({
        profile: { sketchId: "sk", regionId: "r0" },
        distance: { value: 20 },
      }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("ModelToolController commit gesture (Wave 1)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let bodyCbs: Array<(id: string) => void>;
  let previewCb: ((result: PreviewResult) => void) | null;

  function build(endResult: () => ApplyOperationResult = okResult): void {
    engineMock = makeEngineMock();
    previewCb = null;
    clientMock = makeClientMock(endResult, (cb) => {
      previewCb = cb;
    });
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

  function debug(): Record<string, unknown> {
    return (window as unknown as { __extrudePreview: Record<string, unknown> }).__extrudePreview;
  }

  beforeEach(() => {
    resetStores();
    // These specs assert commit SEQUENCING, not the exact-preview barrier itself
    // (that has its own describe below) — collapse the barrier to one macrotask.
    __setExactPreviewTimeoutForTests(0);
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "region-ref", sketchId: "sk", regionId: "r0" },
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

  /** Arm the extrude tool on the single-region sketch and settle the async arm. */
  async function armExtrude(): Promise<void> {
    toolStore.getState().setTool("extrude");
    await flush();
  }

  /**
   * The params of the LAST `updatePreview` send. A mid-arm send is throttled and
   * may coalesce, but `confirmExtrude` force-flushes the final epoch before it
   * commits, so after a confirm this IS the params the kernel builds from.
   */
  function lastSentParams(): Record<string, unknown> {
    const calls = clientMock.updatePreview.mock.calls;
    return calls[calls.length - 1]?.[1] as Record<string, unknown>;
  }

  it("release from a drag keeps the tool armed (no commit) and the chip stays", async () => {
    build();
    await armExtrude();
    expect(controller.extrudeActive).toBe(true);
    expect(toolChipStore.getState().kind).toBe("extrudeDepth");

    controller.forceExtrudeGrab(); // → dragging
    pointer("pointermove", 10, 40, 0, 1); // sets a depth from the ray
    pointer("pointerup", 10, 40, 0, 0); // release
    await flush();

    expect(clientMock.endPreview).not.toHaveBeenCalled(); // release NEVER commits
    expect(controller.extrudeActive).toBe(true); // still armed
    expect(toolStore.getState().phase).toBe("armed");
    expect(toolChipStore.getState().kind).toBe("extrudeDepth"); // chip stays editable
  });

  it("Enter confirms when the canvas is focused, but NOT while a chip input has focus", async () => {
    build();
    await armExtrude();

    // Input focused → the capture-phase Enter is skipped (no double-fire with the chip).
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();
    expect(clientMock.endPreview).not.toHaveBeenCalled();
    input.remove();

    // Canvas Enter (target = window) confirms → endPreview(commit). The confirm
    // first awaits the profile-record guarantee (finishSketch), so settle twice.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();
    expect(clientMock.finishSketch).toHaveBeenCalledWith("sk"); // record guarantee
    expect(clientMock.endPreview).toHaveBeenCalledTimes(1);
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
  });

  it("a failed commit stays armed, names the reason, and re-arms the preview", async () => {
    build(failResult);
    await armExtrude();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1); // arm

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" })); // confirm → fails
    await flush();
    await flush(); // let the re-arm beginPreview resolve

    expect(clientMock.endPreview).toHaveBeenCalledTimes(1);
    expect(controller.extrudeActive).toBe(true); // back to armed (work kept)
    expect(toolStore.getState().phase).toBe("armed");
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2); // preview RE-ARMED
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("worker exploded"); // reason named
    // The applied-but-failed command is rolled back so a retried ✓ cannot stack
    // duplicate errored records (observed: 20 stacked failed Extrudes).
    expect(clientMock.undo).toHaveBeenCalledTimes(1);
  });

  it("a retried confirm after a failure never stacks records (one rollback per attempt)", async () => {
    build(failResult);
    await armExtrude();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledTimes(2);
    expect(clientMock.undo).toHaveBeenCalledTimes(2); // every applied-but-failed commit rolled back
    expect(toolStore.getState().phase).toBe("armed"); // still armed, work kept
  });

  it("a successful confirm tears down and selects the new body once it loads", async () => {
    build();
    await armExtrude();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);

    // Body reconcile: firing onBodyLoaded drops L1, selects the body, exits the tool.
    expect(bodyCbs.length).toBeGreaterThan(0);
    bodyCbs.forEach((cb) => cb("b1"));
    await flush();
    expect(controller.extrudeActive).toBe(false);
    expect(toolStore.getState().modelTool).toBe("select");
    expect(selectionStore.getState().selected).toEqual([{ kind: "body", id: "b1" }]);
  });

  it("treats a full-deletion Cut as success and finishes with no stale selection", async () => {
    build(() => ({
      revision: 2,
      features: [],
      changedBodies: [],
      removedBodies: ["body1"],
    }));
    await armExtrude();
    toolChipStore.getState().onBooleanMode?.("Cut");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
    expect(toolStore.getState().modelTool).toBe("select");
    expect(selectionStore.getState().selected).toEqual([]);
    expect(documentStore.getState().bodies.body1).toBeUndefined();
    expect(viewportStore.getState().statusHint?.message).toBe("Cut completed");
  });

  it("blocks confirm after a stale preview, retries once, and clears only on success", async () => {
    build();
    await armExtrude();
    const first = clientMock.updatePreview.mock.calls[0];
    const sessionId = first[0] as string;
    const firstEpoch = first[2] as number;
    previewCb?.({
      sessionId,
      epoch: firstEpoch,
      bodyId: "preview",
      error: {
        kind: "stalePreview",
        message: "head changed",
        structural: false,
      },
    });

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(clientMock.endPreview).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toContain("Cannot confirm");

    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(clientMock.updatePreview).toHaveBeenCalledTimes(2);
    const retry = clientMock.updatePreview.mock.calls[1];
    previewCb?.({
      sessionId,
      epoch: retry[2] as number,
      bodyId: "preview",
      error: {
        kind: "stalePreview",
        message: "head changed again",
        structural: false,
      },
    });
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(clientMock.updatePreview).toHaveBeenCalledTimes(2);

    toolChipStore.getState().onValue?.(25);
    await new Promise((resolve) => setTimeout(resolve, 100));
    const latest =
      clientMock.updatePreview.mock.calls[clientMock.updatePreview.mock.calls.length - 1];
    previewCb?.({
      sessionId,
      epoch: latest[2] as number,
      bodyId: "preview",
      bodies: [],
      replacedBodyIds: ["body1"],
    });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith(sessionId, true);
  });

  it("renders every body returned by an exact split-Cut preview", async () => {
    build();
    await armExtrude();
    const update = clientMock.updatePreview.mock.calls[0];
    previewCb?.({
      sessionId: update[0] as string,
      epoch: update[2] as number,
      bodyId: "preview",
      bodies: [
        { bodyId: "split-a", mesh: makeBoxMesh() },
        { bodyId: "split-b", mesh: makeBoxMesh() },
      ],
      replacedBodyIds: ["body1"],
    });

    expect(engineMock.setPreviewBody).toHaveBeenCalledTimes(2);
    expect(engineMock.setPreviewReplacedBodyIds).toHaveBeenCalledWith(["body1"]);
  });

  it("a tool switch (Esc ladder tail) tears the open preview session down with endPreview(false)", async () => {
    build();
    await armExtrude();
    toolStore.getState().setTool("select"); // the cancelAll/tool-switch path
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), false);
    expect(controller.extrudeActive).toBe(false);
  });

  it("keeps a stale ToFace promotion in facePick without sending an anchor-only preview", async () => {
    build();
    documentStore.setState({ bodies: { body1: { id: "body1", name: "Body 1", visible: true } } });
    await armExtrude();
    toolChipStore.getState().onEndCondition?.("ToFace");
    await flush();
    expect(debug().phase).toBe("facePick");

    engineMock.probePick.mockReturnValue({
      kind: "face",
      bodyId: "body1",
      topoKey: "f:2",
      distance: 1,
      worldPos: new Vector3(2, 3, 4),
    });
    clientMock.promoteSelection.mockResolvedValueOnce([]);
    const sentBefore = clientMock.updatePreview.mock.calls.length;
    pointer("pointerdown", 10, 10, 0, 1);
    pointer("pointerup", 10, 10, 0, 0);
    await flush();

    expect(debug().phase).toBe("facePick");
    expect(debug().hasTargetFace).toBe(false);
    expect(clientMock.updatePreview).toHaveBeenCalledTimes(sentBefore);
    expect(viewportStore.getState().statusHint?.message).toBe("Selection is out of date — pick again");

    pointer("pointerdown", 10, 10, 0, 1);
    pointer("pointerup", 10, 10, 0, 0);
    await flush();
    expect(debug().phase).toBe("armed");
    expect(debug().hasTargetFace).toBe(true);
  });

  // ── commit-time exact-preview barrier (EXTRUDE-REGION-PARITY P3) ─────────────

  /** Session id + epoch of the FINAL params confirmExtrude pushed before committing. */
  function lastUpdate(): { sessionId: string; epoch: number } {
    const calls = clientMock.updatePreview.mock.calls;
    const last = calls[calls.length - 1];
    return { sessionId: last[0] as string, epoch: last[2] as number };
  }

  it("waits for the final exact preview before committing, then commits", async () => {
    __setExactPreviewTimeoutForTests(2000); // long — the barrier must be what resolves
    build();
    await armExtrude();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();

    // Barrier open: the final params went out, but nothing was committed yet.
    expect(clientMock.endPreview).not.toHaveBeenCalled();
    const { sessionId, epoch } = lastUpdate();

    previewCb?.({ sessionId, epoch, bodyId: "preview", bodies: [], replacedBodyIds: [] });
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith(sessionId, true);
  });

  it("a failed final exact preview blocks the commit, re-arms, and names the reason", async () => {
    __setExactPreviewTimeoutForTests(2000);
    build();
    await armExtrude();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    const { sessionId, epoch } = lastUpdate();

    previewCb?.({
      sessionId,
      epoch,
      bodyId: "preview",
      error: { kind: "opFailed", message: "self-intersecting profile", structural: false },
    });
    await flush();
    await flush(); // let the re-arm beginPreview resolve

    expect(clientMock.endPreview).not.toHaveBeenCalledWith(sessionId, true); // NO commit
    expect(clientMock.endPreview).toHaveBeenCalledWith(sessionId, false); // lane released
    expect(controller.extrudeActive).toBe(true); // re-armed, work kept
    expect(toolStore.getState().phase).toBe("armed");
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2);
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("self-intersecting profile");
  });

  it("proceeds with the commit when no exact preview ever arrives (bounded barrier)", async () => {
    __setExactPreviewTimeoutForTests(10);
    build();
    await armExtrude();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(clientMock.endPreview).not.toHaveBeenCalled(); // still waiting

    await new Promise((r) => setTimeout(r, 30)); // past the barrier timeout
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
  });

  it("cancels an armed extrude when its source sketch geometry changes", async () => {
    build();
    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch X",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v1",
    });
    await armExtrude();
    expect(controller.extrudeActive).toBe(true);
    const sessionId = clientMock.updatePreview.mock.calls[0][0] as string;

    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch X",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v2", // a sketch edit
    });
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith(sessionId, false);
    expect(controller.extrudeActive).toBe(false);
    expect(toolStore.getState().modelTool).toBe("select");
    expect(viewportStore.getState().statusHint?.message).toContain("Sketch changed");
  });

  it("keeps the arm when the sketch changes metadata but not its geometry token", async () => {
    build();
    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch X",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v1",
    });
    await armExtrude();

    documentStore.getState().setSketchSolve("sk", 4, "under"); // metadata-only change
    await flush();

    expect(clientMock.endPreview).not.toHaveBeenCalled();
    expect(controller.extrudeActive).toBe(true);
  });

  it("re-edit preserves every stored param and deep-merges only distance", async () => {
    build();
    const stored = {
      profile: { sketchId: "sk", regionId: "r0" },
      distance: { value: 20 },
      extrudeMode: "ToFace",
      booleanMode: "Cut",
      targetBodyId: "body1",
      targetFace: { element: { primary: { bodyId: "body1", elementId: "face-1", kind: "face" } } },
      draftAngleDeg: { value: 4 },
      twoDirections: true,
      extrudeMode2: "ThroughAll",
      distance2: { value: 7 },
    };
    clientMock.getOperationParams.mockResolvedValue(stored);
    documentStore.setState({
      features: [{ id: "feat-ex", kind: "extrude", label: "Extrude", valueText: "20 mm", status: "ok" }],
    });
    controller.editExtrudeFeature("feat-ex");
    await flush();

    expect(controller.extrudeActive).toBe(true);
    expect(toolChipStore.getState().kind).toBe("extrudeDepth");
    expect(toolChipStore.getState().showSymmetric).toBe(false); // re-edit hides ⇔
    // The winning arm carried the featureId into the draft (param-only edit path).
    const calls = clientMock.beginPreview.mock.calls;
    const draft = calls[calls.length - 1][0] as PreviewDraft;
    expect(draft.params.featureId).toBe("feat-ex");

    toolChipStore.getState().onValue?.(25);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), false);
    expect(clientMock.applyEditCommand).toHaveBeenCalledWith({
      cmd: "updateOperationParams",
      record: "feat-ex",
      op: {
        opType: "Extrude",
        params: { ...stored, distance: { value: 25 } },
      },
    });
  });

  // ── WP-C3: the draft angle reaches the kernel ──────────────────────────────
  //
  // `ExtrudeParams.draftAngleDeg` has crossed FE→core→worker since W-WP6
  // (`ExtrudeOp.cpp apply_draft` → `BRepOffsetAPI_DraftAngle`) with NO UI ever
  // authoring one. These pin the chip → FSM → preview/commit params chain.

  it("an authored draft angle rides the SAME params the preview and the commit use", async () => {
    build();
    await armExtrude();
    expect(toolChipStore.getState().showDraft).toBe(true);

    toolChipStore.getState().onDraftAngle?.(10);
    await flush();

    // The chip reads the value back from the FSM (never from the typed text).
    expect(toolChipStore.getState().draftAngleDeg).toBe(10);

    // The commit force-flushes the final epoch through the SAME lane session, so
    // the previewed drafted prism is exactly what materializes.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(lastSentParams().draftAngleDeg).toBe(10);
    await flush(); // the exact-preview barrier settles, then the session commits
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
  });

  it("an out-of-range draft is CLAMPED, and the chip shows what the op will carry", async () => {
    build();
    await armExtrude();

    toolChipStore.getState().onDraftAngle?.(200);
    await flush();
    expect(toolChipStore.getState().draftAngleDeg).toBe(89); // legacy ±89 limit

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(lastSentParams().draftAngleDeg).toBe(89);
  });

  it("a re-edit SEEDS the stored draft and deep-merges an edited one", async () => {
    build();
    const stored = {
      profile: { sketchId: "sk", regionId: "r0" },
      distance: { value: 20 },
      draftAngleDeg: { value: 6 },
    };
    clientMock.getOperationParams.mockResolvedValue(stored);
    documentStore.setState({
      features: [{ id: "feat-ex", kind: "extrude", label: "Extrude", valueText: "20 mm", status: "ok" }],
    });
    controller.editExtrudeFeature("feat-ex");
    await flush();

    // Seeded from the RECORD — a 0 seed would silently zero the feature on ✓.
    expect(toolChipStore.getState().draftAngleDeg).toBe(6);

    toolChipStore.getState().onDraftAngle?.(15);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();

    expect(clientMock.applyEditCommand).toHaveBeenCalledWith({
      cmd: "updateOperationParams",
      record: "feat-ex",
      op: {
        opType: "Extrude",
        params: { ...stored, distance: { value: 20 }, draftAngleDeg: { value: 15 } },
      },
    });
  });
});
