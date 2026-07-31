/*
 * ModelToolController — Fillet / Chamfer / Shell kernel preview + armed-commit
 * gesture (jsdom).
 *
 * Before this wave both tools committed on the pointer RELEASE with no preview at
 * all: the user pressed, dragged a number, let go, and found out whether OCCT
 * accepted it by reading the history row that appeared. These specs pin the
 * replacement — an arm opens ONE preview session, the value tracks live, release
 * keeps the tool ARMED, and the ✓/Enter commit is BLOCKED while the kernel's
 * newest candidate is a failure (with the kernel's own words on the status line).
 *
 * The load-bearing one is `edgeIds`/`inputs` LOCKSTEP: `ipc/previewOps.ts`
 * refuses a draft whose typed edge inputs do not pair 1:1 with `params.edgeIds`,
 * because `tauriCommandMap.filletParams` silently drops the typed `params.edges`
 * on a mismatch and the worker then fails "Fillet requires body input".
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  PreviewDraft,
  PreviewResult,
  SemanticRef,
} from "@/ipc/types";
import { buildPreviewOp } from "@/ipc/previewOps";
import { toolStore } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const okResult = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#1" }],
  removedBodies: [],
});

const failResult = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [],
  removedBodies: [],
  errorMessage: "fillet radius too large for edge",
});

/** Two picked edges on one body, in click order (the order both fields must keep). */
const EDGES: EntityRef[] = [
  {
    kind: "edge",
    id: "body1#e:5",
    bodyId: "body1",
    topoKey: "e:5",
    elementId: "el-edge-5",
    anchor: { worldPoint: [1, 2, 3] },
  },
  {
    kind: "edge",
    id: "body1#e:9",
    bodyId: "body1",
    topoKey: "e:9",
    elementId: "el-edge-9",
    anchor: { worldPoint: [4, 5, 6] },
  },
];

const FACES: EntityRef[] = [
  {
    kind: "face",
    id: "body1#f:2",
    bodyId: "body1",
    topoKey: "f:2",
    elementId: "el-face-2",
    anchor: { worldPoint: [0, 0, 10] },
  },
];

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
    setOrbitSuppressed: vi.fn(),
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
    probePick: vi.fn(() => null),
  };
}

function makeClientMock(
  endResult: () => ApplyOperationResult,
  capturePreview?: (cb: (r: PreviewResult) => void) => void,
) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capturePreview?.(cb);
      return () => {};
    }),
    finishSketch: vi.fn(() => Promise.resolve({ regions: [] })),
    getSketchRegions: vi.fn(() => Promise.resolve({ regions: [] })),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(endResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() =>
      Promise.resolve({
        radius: { value: 2 },
        edgeIds: ["el-edge-5"],
        edges: [{ primary: { bodyId: "body1", elementId: "el-edge-5", kind: "edge" } }],
      }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
/**
 * Wait past the edge-op/shell trailing floor (160/200ms) so a coalesced params
 * push actually goes out. These tools pace SLOWER than extrude's 80ms default —
 * a fillet rebuild is dearer than a prism sweep — so the trailing poke needs real
 * time, not just a microtask drain.
 */
const settleTrailing = (): Promise<void> => new Promise((r) => setTimeout(r, 320));

describe("ModelToolController edge-op + shell kernel preview", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let previewCb: ((r: PreviewResult) => void) | null;

  function build(endResult: () => ApplyOperationResult = okResult): void {
    engineMock = makeEngineMock();
    previewCb = null;
    clientMock = makeClientMock(endResult, (cb) => {
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
    __setExactPreviewTimeoutForTests(0); // these specs assert sequencing, not the barrier
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  async function armFillet(kind: "fillet" | "chamfer" = "fillet"): Promise<void> {
    selectionStore.getState().set(EDGES);
    toolStore.getState().setTool(kind);
    await flush();
  }

  async function armShell(): Promise<void> {
    selectionStore.getState().set(FACES);
    toolStore.getState().setTool("shell");
    await flush();
  }

  /** The newest `updatePreview` dispatch (session + epoch + params). */
  function lastUpdate(): { sessionId: string; epoch: number; params: Record<string, unknown> } {
    const calls = clientMock.updatePreview.mock.calls;
    const last = calls[calls.length - 1];
    return {
      sessionId: last[0] as string,
      params: last[1] as Record<string, unknown>,
      epoch: last[2] as number,
    };
  }

  /** Publish a successful exact candidate for the newest dispatched epoch. */
  function answerPreview(): void {
    const { sessionId, epoch } = lastUpdate();
    previewCb?.({ sessionId, epoch, bodyId: "preview", bodies: [], replacedBodyIds: ["body1"] });
  }

  /** Publish a failure for the newest dispatched epoch. */
  function failPreview(message: string, structural = true): void {
    const { sessionId, epoch } = lastUpdate();
    previewCb?.({
      sessionId,
      epoch,
      bodyId: "preview",
      error: { kind: structural ? "invalidCommand" : "opFailed", message, structural },
    });
  }

  // ── arm: one session, dual fields in lockstep ────────────────────────────────

  it("arming an edge op opens ONE session whose inputs and edgeIds are in lockstep", async () => {
    build();
    await armFillet();

    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    const draft = clientMock.beginPreview.mock.calls[0][0] as PreviewDraft;
    expect(draft.opType).toBe("Fillet");
    // Same source array, same order — this is the invariant previewOps enforces.
    expect(draft.params.edgeIds).toEqual(["e:5", "e:9"]);
    expect((draft.inputs ?? []).map((r: SemanticRef) => r.primary.elementId)).toEqual([
      "el-edge-5",
      "el-edge-9",
    ]);
    expect((draft.inputs ?? []).every((r: SemanticRef) => r.primary.kind === "edge")).toBe(true);
    expect((draft.inputs ?? []).every((r: SemanticRef) => r.primary.bodyId === "body1")).toBe(true);

    // The canonical mapper accepts the draft — a lockstep break would THROW here,
    // which is exactly what the lane turns into a structural failure.
    expect(() =>
      buildPreviewOp({ opId: "op-1", opType: "Fillet", inputs: draft.inputs, latestParams: draft.params }),
    ).not.toThrow();

    // …and the first exact request went out under this session.
    expect(clientMock.updatePreview).toHaveBeenCalledTimes(1);
    expect(lastUpdate().sessionId).toBe("pv-1");
  });

  it("Chamfer arms the same lane under its own opType", async () => {
    build();
    await armFillet("chamfer");
    const draft = clientMock.beginPreview.mock.calls[0][0] as PreviewDraft;
    expect(draft.opType).toBe("Chamfer");
    expect(draft.params.mode).toBe("Chamfer");
  });

  it("arming a shell opens ONE session bound to the picked faces + their body", async () => {
    build();
    await armShell();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    const draft = clientMock.beginPreview.mock.calls[0][0] as PreviewDraft;
    expect(draft.opType).toBe("Shell");
    expect(draft.params.openFaces).toEqual(["el-face-2"]);
    expect(draft.params.targetBodyId).toBe("body1");
  });

  it('publishes "Computing preview…" on arm and clears it on the first answer', async () => {
    build();
    await armFillet();
    // No L1 exists for an edge op, so the wait IS announced (unlike extrude).
    expect(viewportStore.getState().statusHint?.message).toBe("Computing preview…");

    answerPreview();
    expect(viewportStore.getState().statusHint?.message).toContain("Fillet 2 edges");
  });

  // ── live value edits ─────────────────────────────────────────────────────────

  it("a chip edit pushes a NEW epoch carrying the new radius", async () => {
    build();
    await armFillet();
    answerPreview(); // free the throttle's in-flight slot
    const first = lastUpdate();

    toolChipStore.getState().onValue?.(5);
    await settleTrailing();
    const next = lastUpdate();
    expect(next.epoch).toBeGreaterThan(first.epoch);
    expect(next.params.radius).toBe(5);
    expect(next.params.edgeIds).toEqual(["e:5", "e:9"]); // selection unchanged
  });

  it("a drag release keeps the tool ARMED and never commits", async () => {
    build();
    await armFillet();
    answerPreview();

    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: 10, clientY: 40, button: 0, buttons: 1, bubbles: true }),
    );
    expect(toolStore.getState().phase).toBe("dragging");
    container.dispatchEvent(
      new MouseEvent("pointermove", { clientX: 10, clientY: 20, button: 0, buttons: 1, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: 10, clientY: 20, button: 0, buttons: 0, bubbles: true }),
    );
    await flush();

    expect(clientMock.endPreview).not.toHaveBeenCalled(); // release NEVER commits
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
    expect(toolStore.getState().phase).toBe("armed");
    expect(toolChipStore.getState().kind).toBe("filletRadius"); // chip stays editable
    expect(toolStore.getState().modelTool).toBe("fillet");
  });

  // ── the headline win: a refused candidate blocks the commit ──────────────────

  it("a preview failure BLOCKS ✓ and surfaces the kernel's own reason", async () => {
    build();
    await armFillet();
    failPreview("BRepFilletAPI: radius exceeds edge length");
    expect(viewportStore.getState().statusHint?.message).toContain(
      "Fillet preview failed: BRepFilletAPI: radius exceeds edge length",
    );

    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    expect(clientMock.endPreview).not.toHaveBeenCalled(); // nothing reached the timeline
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toContain(
      "Cannot apply invalid preview: BRepFilletAPI: radius exceeds edge length",
    );
    expect(toolStore.getState().modelTool).toBe("fillet"); // still armed on the same edges
  });

  it("the failure latch clears on a newer good candidate, and ✓ then commits", async () => {
    build();
    await armFillet();
    failPreview("radius exceeds edge length");

    // Fix it: a new radius, a good answer — the latch must drop.
    toolChipStore.getState().onValue?.(1);
    await settleTrailing();
    answerPreview();
    expect(viewportStore.getState().statusHint?.severity).not.toBe("error");

    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
  });

  it("a shell preview failure blocks its ✓ too", async () => {
    build();
    await armShell();
    failPreview("thickness exceeds the thinnest wall");

    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();
    expect(clientMock.endPreview).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toContain(
      "Cannot apply invalid preview: thickness exceeds the thinnest wall",
    );
  });

  // ── commit ───────────────────────────────────────────────────────────────────

  it("Enter flushes the final params as the newest epoch, then commits the session", async () => {
    build();
    await armFillet();
    answerPreview();
    toolChipStore.getState().onValue?.(3);
    await settleTrailing();
    answerPreview();
    const beforeCommit = lastUpdate().epoch;

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();

    const final = lastUpdate();
    expect(final.epoch).toBeGreaterThan(beforeCommit); // final params re-flushed
    expect(final.params.radius).toBe(3);
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
    expect(clientMock.applyOperation).not.toHaveBeenCalled(); // the SESSION materialized it
    expect(toolStore.getState().modelTool).toBe("select");
    expect(viewportStore.getState().statusHint?.message).toBe("Filleted 2 edges");
    expect(toolChipStore.getState().kind).toBe("none");
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
  });

  it("Enter commits an armed shell the same way", async () => {
    build();
    await armShell();
    answerPreview();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
    expect(viewportStore.getState().statusHint?.message).toBe("Shelled");
  });

  it("Enter is skipped while a chip input has focus (no double-commit)", async () => {
    build();
    await armFillet();
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();
    expect(clientMock.endPreview).not.toHaveBeenCalled();
    input.remove();
  });

  it("a failed commit stays armed, names the reason, and re-arms the preview", async () => {
    build(failResult);
    await armFillet();
    answerPreview();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);

    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
    // The command applied but its regen failed → the errored row is rolled back so
    // a retried ✓ cannot stack duplicates.
    expect(clientMock.undo).toHaveBeenCalledTimes(1);
    expect(toolStore.getState().phase).toBe("armed"); // work kept
    expect(toolStore.getState().modelTool).toBe("fillet");
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(2); // preview RE-ARMED
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("fillet radius too large for edge");
  });

  // ── cancel / teardown ────────────────────────────────────────────────────────

  it("the Esc-ladder tail (tool switch) releases the session with endPreview(false)", async () => {
    build();
    await armFillet();
    toolStore.getState().setTool("select");
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(clientMock.endPreview).not.toHaveBeenCalledWith("pv-1", true);
    expect(toolChipStore.getState().kind).toBe("none");
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
  });

  it("the chip ✕ cancels without committing", async () => {
    build();
    await armShell();
    toolChipStore.getState().onCancel?.();
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(toolStore.getState().modelTool).toBe("select");
  });

  it("dispose ends every open session (a remount must not leak the lane)", async () => {
    build();
    await armFillet();
    controller.dispose();
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", false);
  });

  // ── stale model ──────────────────────────────────────────────────────────────

  it("a document-revision bump cancels an armed edge op with a re-select hint", async () => {
    build();
    await armFillet();
    documentStore.getState().applyChange({ revision: 42 }); // an external edit / undo
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(toolStore.getState().modelTool).toBe("select");
    expect(viewportStore.getState().statusHint?.message).toBe("Model changed — re-select edges");
  });

  it("a document-revision bump cancels an armed shell with a re-select hint", async () => {
    build();
    await armShell();
    documentStore.getState().applyChange({ revision: 42 });
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(viewportStore.getState().statusHint?.message).toBe("Model changed — re-select faces");
  });

  it("the tool's OWN commit does not trip the stale-model guard", async () => {
    build();
    await armFillet();
    answerPreview();
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    // The commit bumped the revision (applyResult) while phase was `committing`.
    expect(viewportStore.getState().statusHint?.message).toBe("Filleted 2 edges");
  });

  // ── re-edit stays L1-only ────────────────────────────────────────────────────

  it("a parametric re-edit opens NO session and commits through the scalar patch", async () => {
    build();
    documentStore.setState({
      features: [
        { id: "feat-fi", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" },
      ],
    });
    await controller.editEdgeOpFeature("feat-fi", "Fillet");
    await flush();

    // PreviewOp runs against the CURRENT head, so previewing an existing feature
    // would double-apply it — a re-edit is deliberately preview-free.
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(toolChipStore.getState().kind).toBe("filletRadius");
    expect(toolChipStore.getState().onConfirm).toBeNull(); // no ✓ cluster on a re-edit

    toolChipStore.getState().onValue?.(4);
    await flush();
    expect(clientMock.applyEditCommand).toHaveBeenCalledWith({
      cmd: "updateOperationParams",
      record: "feat-fi",
      op: {
        opType: "Fillet",
        params: {
          radius: { value: 4 },
          edgeIds: ["el-edge-5"],
          edges: [{ primary: { bodyId: "body1", elementId: "el-edge-5", kind: "edge" } }],
        },
      },
    });
    expect(clientMock.endPreview).not.toHaveBeenCalled();
  });

  it("editEdgeOpFeature refuses a Shell row that merely shares the folded `fillet` kind", async () => {
    build();
    documentStore.setState({
      features: [
        { id: "feat-sh", kind: "fillet", opType: "Shell", label: "Shell", valueText: "2.0 mm", status: "ok" },
      ],
    });
    await controller.editEdgeOpFeature("feat-sh", "Fillet");
    await flush();
    expect(toolChipStore.getState().kind).toBe("none");
    expect(clientMock.getOperationParams).not.toHaveBeenCalled();
  });
});
