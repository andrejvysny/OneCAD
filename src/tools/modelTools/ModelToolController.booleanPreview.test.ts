/*
 * Boolean kernel-preview wiring (jsdom). A standalone Boolean now opens a real
 * kernel-preview lane session the moment target+tool are both picked (mirrors
 * confirmExtrude's barrier), replacing the old "commit blind, hope it matches"
 * flow with the SAME candidate-preview == commit contract Extrude already proves
 * (SCHEMA §7.6). These specs pin:
 *   - arming opens exactly ONE session with both body refs + the live params;
 *   - an operation swap (Union/Cut/Intersect) re-sends the preview at a fresh
 *     epoch and flips the Cut tint;
 *   - confirm flushes the final params as the newest epoch, waits for the
 *     matching exact candidate, then commits that SAME lane session;
 *   - a barrier failure blocks the commit, re-arms, and names the kernel reason;
 *   - Esc/tool-switch tears the session down with `endPreview(false)` and lets the
 *     engine restore any bodies `setPreviewReplacedBodyIds` hid;
 *   - a re-edit (`editBooleanFeature`, operation-swap-only) never opens a session —
 *     the tool body is normally already consumed, so there is nothing honest to
 *     preview;
 *   - dispose while armed never leaks the open session.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult, PreviewDraft, PreviewResult } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#0" }],
  removedBodies: [],
});

function makeEngineMock() {
  return {
    showGhostPreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    probePick: vi.fn(() => null),
    hideExtrudePreview: vi.fn(),
    clearPreviewBody: vi.fn(),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    setPreviewTint: vi.fn(),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    hideRevolvePreview: vi.fn(),
    hideRegionPick: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
  };
}

function makeClientMock(capturePreview?: (cb: (result: PreviewResult) => void) => void) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((cb: (result: PreviewResult) => void) => {
      capturePreview?.(cb);
      return () => {};
    }),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() =>
      Promise.resolve({ operation: "Union", targetBodyId: "body1", toolBodyId: "body2" }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

const TWO_BODIES = {
  body1: { id: "body1", name: "Body 1", visible: true },
  body2: { id: "body2", name: "Body 2", visible: true },
};

describe("ModelToolController boolean kernel preview", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let previewCb: ((result: PreviewResult) => void) | null;

  function build(): void {
    engineMock = makeEngineMock();
    previewCb = null;
    clientMock = makeClientMock((cb) => {
      previewCb = cb;
    });
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  }

  beforeEach(() => {
    resetStores();
    // These specs assert commit SEQUENCING, not the exact-preview barrier itself —
    // collapse it to one macrotask (mirrors ModelToolController.commit.test.ts).
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    documentStore.setState({ bodies: { ...TWO_BODIES } });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  /** Select body1 as target, arm the boolean tool, pick body2 as the tool body,
   *  and settle the async arm (the kernel-preview session opens here). */
  async function armBoolean(): Promise<void> {
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    toolStore.getState().setTool("boolean");
    controller.forceBooleanPick("body2");
    await flush();
  }

  it("arming opens exactly ONE session with both body refs and the live params", async () => {
    build();
    await armBoolean();

    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    expect(clientMock.beginPreview).toHaveBeenCalledWith({
      opType: "Boolean",
      inputs: [
        { primary: { bodyId: "body1", kind: "body" } },
        { primary: { bodyId: "body2", kind: "body" } },
      ],
      params: { operation: "Union", targetBodyId: "body1", toolBodyId: "body2" },
    });
    // The initial exact request went out (leading edge — no drag to wait on).
    expect(clientMock.updatePreview).toHaveBeenCalledTimes(1);
    const [sessionId, params] = clientMock.updatePreview.mock.calls[0];
    expect(sessionId).toBe("pv-1");
    expect(params).toEqual({ operation: "Union", targetBodyId: "body1", toolBodyId: "body2" });
    expect(engineMock.setPreviewTint).toHaveBeenCalledWith("normal");
  });

  it("an operation change re-sends the preview at a fresh epoch and flips the Cut tint", async () => {
    build();
    await armBoolean();
    const first = clientMock.updatePreview.mock.calls[0];
    const [sessionId, , firstEpoch] = first;
    // Free the throttle's in-flight slot so the op-change send can go out.
    previewCb?.({ sessionId, epoch: firstEpoch, bodyId: "preview", bodies: [], replacedBodyIds: [] });
    await flush();

    toolChipStore.getState().onOp?.("Cut");
    await flush();

    expect(engineMock.setPreviewTint).toHaveBeenLastCalledWith("cut");
    const calls = clientMock.updatePreview.mock.calls;
    const last = calls[calls.length - 1];
    expect(last[2]).toBeGreaterThan(firstEpoch);
    expect(last[1]).toEqual({ operation: "Cut", targetBodyId: "body1", toolBodyId: "body2" });

    // Union flips it back.
    toolChipStore.getState().onOp?.("Union");
    expect(engineMock.setPreviewTint).toHaveBeenLastCalledWith("normal");
  });

  it("confirm flushes the final params as the newest epoch, waits for the exact match, then commits", async () => {
    __setExactPreviewTimeoutForTests(2000); // long — the barrier must be what resolves
    build();
    await armBoolean();

    toolChipStore.getState().onApply?.();
    await flush();
    expect(clientMock.endPreview).not.toHaveBeenCalled(); // barrier open, nothing committed yet

    const calls = clientMock.updatePreview.mock.calls;
    const last = calls[calls.length - 1];
    const [sessionId, , epoch] = last;
    previewCb?.({ sessionId, epoch, bodyId: "preview", bodies: [], replacedBodyIds: ["body2"] });
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith(sessionId, true);
    expect(toolStore.getState().modelTool).toBe("select");
    expect(selectionStore.getState().selected).toEqual([{ kind: "body", id: "body1" }]);
  });

  it("the success hint SURVIVES the commit's reset to select", async () => {
    // Hint-clobber regression: the tail published "Union applied" and then called
    // setTool("select"), whose synchronous sweep cleared it — the confirmation never
    // reached the status bar.
    __setExactPreviewTimeoutForTests(2000);
    build();
    await armBoolean();

    toolChipStore.getState().onApply?.();
    await flush();
    const calls = clientMock.updatePreview.mock.calls;
    const [sessionId, , epoch] = calls[calls.length - 1];
    previewCb?.({ sessionId, epoch, bodyId: "preview", bodies: [], replacedBodyIds: ["body2"] });
    await flush();

    expect(toolStore.getState().modelTool).toBe("select");
    expect(viewportStore.getState().statusHint?.message).toBe("Union applied");
  });

  it("a re-edit's success hint survives its reset to select too", async () => {
    build();
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "Body 1", visible: true } },
      features: [
        { id: "feat-bool", kind: "boolean", opType: "Boolean", label: "Union", valueText: "", status: "ok" },
      ],
    });

    await controller.editBooleanFeature("feat-bool");
    await flush();
    toolChipStore.getState().onApply?.();
    await flush();

    expect(toolStore.getState().modelTool).toBe("select");
    expect(viewportStore.getState().statusHint?.message).toBe("Boolean changed to Union");
  });

  it("a failed final exact preview blocks the commit, re-arms, and names the kernel reason", async () => {
    __setExactPreviewTimeoutForTests(2000);
    build();
    await armBoolean();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    // The re-arm's beginPreview hangs (never resolves): once it lands, the fresh
    // session's own "Computing preview…" hint would overwrite the error hint below —
    // that overwrite is real, correct, later behavior, not what THIS spec pins.
    clientMock.beginPreview.mockImplementationOnce(() => new Promise(() => {}));

    toolChipStore.getState().onApply?.();
    await flush();
    const calls = clientMock.updatePreview.mock.calls;
    const last = calls[calls.length - 1];
    const [sessionId, , epoch] = last;
    previewCb?.({
      sessionId,
      epoch,
      bodyId: "preview",
      error: { kind: "opFailed", message: "self-intersecting geometry", structural: false },
    });
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith(sessionId, false); // lane released
    expect(clientMock.endPreview).not.toHaveBeenCalledWith(sessionId, true); // NEVER committed
    expect(toolStore.getState().phase).toBe("armed"); // re-armed, work kept
    expect(toolStore.getState().modelTool).toBe("boolean");
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2); // preview RE-ARMED (in flight)
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("self-intersecting geometry");
  });

  it("Esc/tool switch tears the open session down with endPreview(false) and restores hidden bodies", async () => {
    build();
    await armBoolean();

    toolStore.getState().setTool("select"); // the cancelAll/tool-switch (Esc-ladder tail) path
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), false);
    expect(engineMock.clearPreviewBody).toHaveBeenCalled(); // restores any hidden target/tool bodies
  });

  it("a re-edit (editBooleanFeature) never opens a preview session", async () => {
    build();
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "Body 1", visible: true } }, // tool body retired
      features: [
        { id: "feat-bool", kind: "boolean", opType: "Boolean", label: "Union", valueText: "", status: "ok" },
      ],
    });

    await controller.editBooleanFeature("feat-bool");
    await flush();

    expect(toolStore.getState().modelTool).toBe("boolean");
    expect(toolStore.getState().phase).toBe("armed");
    expect(toolChipStore.getState().kind).toBe("booleanOp");
    expect(clientMock.beginPreview).not.toHaveBeenCalled();

    // Committing the re-edit still routes through the scalar update, not the lane.
    toolChipStore.getState().onApply?.();
    await flush();
    expect(clientMock.applyEditCommand).toHaveBeenCalledTimes(1);
    expect(clientMock.endPreview).not.toHaveBeenCalled();
  });

  it("dispose while armed ends the open session — no leaked lane session", async () => {
    build();
    await armBoolean();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);

    controller.dispose();
    controller = undefined as unknown as ModelToolController; // afterEach must not double-dispose

    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), false);
  });

  // ── the tool-body pick is MODAL ────────────────────────────────────────────
  //
  // "Pick the tool body to combine" used to be the whole of the feedback: the
  // picker is inactive while a model tool is armed, so nothing lit up under the
  // pointer and an LMB drag orbited the camera instead of resolving the click.

  /** Point at `bodyId` and move the pointer (no press — pure hover). */
  function hoverBody(bodyId: string): void {
    (engineMock.probePick as unknown as { mockReturnValue(v: unknown): void }).mockReturnValue({
      bodyId,
      kind: "face",
      topoKey: "f:0",
      distance: 1,
      worldPos: { x: 1, y: 2, z: 3 },
    });
    container.dispatchEvent(
      new MouseEvent("pointermove", { clientX: 10, clientY: 10, bubbles: true }),
    );
  }

  function startPick(): void {
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    toolStore.getState().setTool("boolean");
  }

  it("hover-tints the WHOLE candidate body — a body pick, not a face pick", () => {
    build();
    startPick();
    hoverBody("body2");

    const hover = selectionStore.getState().hover;
    expect(hover?.kind).toBe("body");
    expect(hover?.id).toBe("body2");
  });

  it("does NOT tint the target body — combining a body with itself is refused", () => {
    build();
    startPick();
    hoverBody("body1");
    expect(selectionStore.getState().hover).toBeNull();
  });

  it("drops the hover and restores orbit once the tool body is picked", async () => {
    build();
    startPick();
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(true);
    hoverBody("body2");
    expect(selectionStore.getState().hover).not.toBeNull();

    controller.forceBooleanPick("body2");
    await flush();

    // Armed boolean is chip-driven — no drag of its own — so orbit goes back.
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
    expect(selectionStore.getState().hover).toBeNull();
  });

  it("restores orbit on a tool switch out of an unresolved pick", async () => {
    build();
    startPick();
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(true);

    toolStore.getState().setTool("select");
    await flush();
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
  });
});
