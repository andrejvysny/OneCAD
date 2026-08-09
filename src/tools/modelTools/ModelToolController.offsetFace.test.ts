/*
 * ModelToolController — OffsetFace lane (jsdom).
 *
 * What is actually load-bearing here is not the drag; it is the AUTHORING
 * TRANSACTION. Every other tool builds its params from what the user picked. This
 * one cannot: the kernel auto-propagates an offset across G1-tangent junctions and
 * cannot hold a tangent neighbour fixed, so the set that will really move has to be
 * computed by the worker (`PrepareOffsetFace`, SCHEMA §7.6) BEFORE anything is
 * authored, promoted to Rust-minted ids, and then FROZEN.
 *
 * So the specs below pin the FAIL-CLOSED points, one per way the transaction can
 * go wrong: a refusal must not arm, a cross-body pick must not arm, a failed
 * promotion must not arm, a document that moves must drop the arm, and a commit
 * whose exact preview never landed must be refused. Every one of those, if it
 * leaked, would write a record whose frozen faces nobody verified — which is the
 * whole defect class this op's design exists to eliminate.
 *
 * `faceFrame`/`getEntry` are mocked (as in `ModelToolController.hole.test.ts`):
 * the frame MATH is `alignSolve.test.ts`'s to prove, and what this file needs is
 * only the branch between the arrow lane and the degraded one.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  OffsetDistanceType,
  PrepareOffsetFaceRequest,
  PrepareOffsetFaceResult,
  PreviewDraft,
  PreviewResult,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { makeBoxMesh } from "@/ipc/mockMeshes";

/** Flipped per-test: whether the picked faces resolve to a planar frame. */
let planar = true;

vi.mock("@/viewport/mesh/meshRegistry", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  getEntry: vi.fn(() => ({
    faceIndex: { ordinalForId: () => 0 },
    geometry: {},
    view: { faceRanges: new Uint32Array([0, 2]) },
  })),
}));

vi.mock("@/tools/preview/alignSolve", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  faceFrame: vi.fn(() => (planar ? { center: [0, 0, 10], normal: [0, 0, 1] } : null)),
}));

const okResult = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#2" }],
  removedBodies: [],
});

const FACE: EntityRef = {
  kind: "face",
  id: "body1#f:2",
  bodyId: "body1",
  topoKey: "f:2",
  elementId: "el-face-2",
  anchor: { worldPoint: [0, 0, 10] },
};

const OTHER_BODY_FACE: EntityRef = {
  kind: "face",
  id: "body2#f:0",
  bodyId: "body2",
  topoKey: "f:0",
  elementId: "el-face-0",
  anchor: { worldPoint: [50, 0, 0] },
};

function makeEngineMock() {
  return {
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    planePixelWorld: vi.fn(() => 1),
    hitExtrudeHandle: vi.fn(() => true),
    hitTransformGizmo: vi.fn(() => null),
    setExtrudeHandleHover: vi.fn(),
    screenRay: vi.fn(() => ({ origin: [0, 0, 100] as const, dir: [0, 0, -1] as const })),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    setPreviewTint: vi.fn(),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    clearPreviewBody: vi.fn(),
    hideRegionPick: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    showValueHandle: vi.fn(),
    hideValueHandle: vi.fn(),
    setDatumGhost: vi.fn(),
    mountChip: vi.fn(),
    unmountChip: vi.fn(),
    probePick: vi.fn(() => null),
  };
}

/** The default handshake answer: the picks themselves, no chain, no opposite. */
const closureOf = (req: PrepareOffsetFaceRequest): PrepareOffsetFaceResult => ({
  snapshotId: 1,
  targetBodyId: req.pickedFaces[0]?.bodyId ?? "",
  faces: req.pickedFaces.map((p) => ({
    topoKey: p.topoKey ?? "f:2",
    picked: true,
    anchor: { worldPoint: [0, 0, 10] as [number, number, number] },
  })),
  currentDims: { radius: 10, thickness: 12 },
  refusal: null,
});

function makeClientMock(
  capturePreview?: (cb: (r: PreviewResult) => void) => void,
  prepare: (req: PrepareOffsetFaceRequest) => Promise<PrepareOffsetFaceResult> = (req) =>
    Promise.resolve(closureOf(req)),
) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capturePreview?.(cb);
      return () => {};
    }),
    finishSketch: vi.fn(() => Promise.resolve({ regions: [] })),
    getSketchRegions: vi.fn(() => Promise.resolve({ regions: [] })),
    prepareOffsetFace: vi.fn(prepare),
    promoteSelection: vi.fn((bodyId: string, picks: { topoKey: string }[]) =>
      Promise.resolve(
        picks.map((p) => ({
          topoKey: p.topoKey,
          elementId: `el-${p.topoKey.replace(":", "")}`,
          kind: "face" as const,
          bodyId,
        })),
      ),
    ),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() =>
      Promise.resolve({
        faceIds: ["el-f2"],
        faces: [{ primary: { bodyId: "body1", elementId: "el-f2", kind: "face" } }],
        distance: { value: 2.5 },
        distanceType: "Offset",
        chainTangentFaces: true,
        targetBodyId: "body1",
      }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
/** Past the OffsetFace trailing floor (160ms) so a coalesced push goes out. */
const settleTrailing = (): Promise<void> => new Promise((r) => setTimeout(r, 260));

type Debug = {
  offsetFacePhase?: string;
  offsetDistance?: number;
  offsetDistanceType?: string;
  offsetChainTangent?: boolean;
  offsetFaceCount?: number;
  offsetPrepared?: number | null;
  offsetDegraded?: boolean;
  previewOwner?: string | null;
};
const debug = (): Debug =>
  (window as unknown as { __extrudePreview?: Debug }).__extrudePreview ?? {};

describe("ModelToolController OffsetFace", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let previewCb: ((r: PreviewResult) => void) | null;

  function build(prepare?: (req: PrepareOffsetFaceRequest) => Promise<PrepareOffsetFaceResult>): void {
    // ONE controller at a time: a second live one subscribes to the same stores
    // and would answer the next `setTool` alongside the one under test.
    controller?.dispose();
    engineMock = makeEngineMock();
    previewCb = null;
    clientMock = makeClientMock((cb) => {
      previewCb = cb;
    }, prepare);
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  }

  /** Select `faces`, arm the tool, settle the handshake + the preview open. */
  async function arm(faces: EntityRef[] = [FACE]): Promise<void> {
    selectionStore.getState().set(faces);
    toolStore.getState().setTool("offsetFace");
    await flush();
    await flush();
    await flush();
  }

  /** The newest `updatePreview(sessionId, params, epoch)` call, or undefined. */
  function lastUpdate(): [string, unknown, number] | undefined {
    const calls = clientMock.updatePreview.mock.calls;
    return calls[calls.length - 1] as [string, unknown, number] | undefined;
  }

  /** Settle the commit barrier by answering the newest epoch. */
  function answerExactPreview(): void {
    const call = lastUpdate();
    if (!call || !previewCb) return;
    previewCb({ sessionId: call[0] as string, epoch: call[2] as number, bodyId: "pb-1" });
  }

  /*
   * Every interaction below goes through the CHIP STORE, which is the real seam
   * the controller registers its handlers on — no private test hooks. A spec that
   * reached into the controller would prove the internals work, not the tool.
   */
  const chipConfirm = (): void => toolChipStore.getState().onConfirm?.();
  const chipValue = (v: number): void => toolChipStore.getState().onValue?.(v);
  const chipType = (t: OffsetDistanceType): void => toolChipStore.getState().onDistanceType?.(t);
  /** Fire ✓ and let the commit sequence reach its barrier. */
  async function confirmAndSettle(answer: boolean): Promise<void> {
    chipConfirm();
    await flush();
    if (answer) answerExactPreview();
    await flush();
    await flush();
    if (!answer) await new Promise((r) => setTimeout(r, 60)); // past the 30ms barrier
    await flush();
  }

  beforeEach(() => {
    resetStores();
    planar = true;
    __setExactPreviewTimeoutForTests(30);
    container = document.createElement("div");
    document.body.appendChild(container);
    // `__extrudePreview` is a window global that OUTLIVES a controller, so a test
    // whose controller never publishes would otherwise read the previous one's.
    delete (window as unknown as { __extrudePreview?: unknown }).__extrudePreview;
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  // ── arm: the handshake gate ──────────────────────────────────────────────

  it("asks for faces when nothing is selected and never calls prepare", async () => {
    build();
    toolStore.getState().setTool("offsetFace");
    await flush();
    expect(clientMock.prepareOffsetFace).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toBe("Select faces to offset, then Offset face");
    expect(debug().offsetFacePhase).toBe("idle");
  });

  it("REFUSES a cross-body selection locally — no round trip, no arm", async () => {
    build();
    await arm([FACE, OTHER_BODY_FACE]);
    expect(clientMock.prepareOffsetFace).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
    expect(debug().offsetFacePhase).toBe("idle");
  });

  it("arms from the handshake's CLOSURE, not from the picks", async () => {
    // The kernel adds a tangent neighbour the user never clicked. It is the
    // closure that gets frozen, so it is the closure the tool must arm on.
    build((req) =>
      Promise.resolve({
        ...closureOf(req),
        faces: [
          { topoKey: "f:2", picked: true, anchor: { worldPoint: [0, 0, 10] } },
          { topoKey: "f:7", picked: false, anchor: { worldPoint: [5, 0, 10] } },
        ],
      }),
    );
    await arm();
    expect(debug().offsetFacePhase).toBe("armed");
    expect(debug().offsetFaceCount).toBe(2);
    // The arm hint NAMES the chained face — the closure exceeding the picks has
    // no other surface. It is parked behind the "Computing preview…" state and
    // handed back the moment the kernel answers.
    answerExactPreview();
    expect(viewportStore.getState().statusHint?.message).toContain("+1 tangent");
    // The chained face had no ElementId, so it was PROMOTED before the arm.
    expect(clientMock.promoteSelection).toHaveBeenCalledWith("body1", [
      { topoKey: "f:7", anchor: { worldPoint: [5, 0, 10] } },
    ]);
  });

  it("does NOT arm on a REFUSAL and shows the worker's own reason", async () => {
    build((req) =>
      Promise.resolve({
        ...closureOf(req),
        targetBodyId: "",
        faces: [],
        refusal: {
          code: "chainMismatch",
          message: "the tangent closure exceeds the picked faces",
          faces: ["f:7"],
        },
      }),
    );
    await arm();
    expect(debug().offsetFacePhase).toBe("idle");
    // The refusal's own words survive — a generic message would throw away the
    // only explanation the user gets.
    expect(viewportStore.getState().statusHint?.message).toBe(
      "the tangent closure exceeds the picked faces",
    );
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
  });

  it("does NOT arm when a promotion is refused (no anchor-only fallback exists)", async () => {
    build((req) =>
      Promise.resolve({
        ...closureOf(req),
        faces: [{ topoKey: "f:9", picked: false, anchor: { worldPoint: [1, 1, 1] } }],
      }),
    );
    clientMock.promoteSelection.mockResolvedValueOnce([]);
    await arm();
    // A record's typed ref must carry an elementId equal to its own faceIds entry
    // (core validates the lockstep), so there is nothing degraded to fall back to.
    expect(debug().offsetFacePhase).toBe("idle");
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
  });

  it("opens ONE preview session owned by offsetFace, with the frozen refs as inputs", async () => {
    build();
    await arm();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    const draft = clientMock.beginPreview.mock.calls[0][0] as PreviewDraft;
    expect(draft.opType).toBe("OffsetFace");
    expect(draft.params).toMatchObject({
      distanceType: "Offset",
      chainTangentFaces: true,
      targetBodyId: "body1",
    });
    expect(draft.inputs).toEqual([
      {
        primary: { bodyId: "body1", elementId: "el-face-2", kind: "face" },
        anchor: { worldPoint: [0, 0, 10] },
      },
    ]);
    expect(debug().previewOwner).toBe("offsetFace");
  });

  it("shows the 3D arrow + the per-face ghost when the frames are planar", async () => {
    build();
    await arm();
    expect(debug().offsetDegraded).toBe(false);
    expect(engineMock.showValueHandle).toHaveBeenCalled();
    expect(engineMock.showGhostPreviewMulti).toHaveBeenCalled();
    const ghostCalls = engineMock.showGhostPreviewMulti.mock.calls;
    const items = ghostCalls[ghostCalls.length - 1]?.[0] as Array<{
      range?: { start: number; count: number };
    }>;
    // The ghost is a SLICE of the body's geometry (the moving faces), not a clone
    // of the whole solid — a whole-body ghost would read as a translation.
    expect(items[0].range).toEqual({ start: 0, count: 6 });
    // Orbit stays FREE: the arrow is hit-tested, so a press that misses it orbits.
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
  });

  it("DEGRADES to the screen drag (no arrow, no ghost) for a curved face", async () => {
    planar = false;
    build();
    await arm();
    expect(debug().offsetFacePhase).toBe("armed");
    expect(debug().offsetDegraded).toBe(true);
    expect(engineMock.showValueHandle).not.toHaveBeenCalled();
    expect(engineMock.showGhostPreviewMulti).not.toHaveBeenCalled();
    // Without a handle to miss, the tool claims every press — so orbit must go.
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(true);
  });

  // ── drag ──────────────────────────────────────────────────────────────────

  it("a released drag stays ARMED and commits nothing", async () => {
    build();
    await arm();
    container.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: 10, clientY: 10, button: 0, bubbles: true }),
    );
    expect(debug().offsetFacePhase).toBe("dragging");
    container.dispatchEvent(
      new PointerEvent("pointermove", { clientX: 10, clientY: 60, button: 0, bubbles: true }),
    );
    container.dispatchEvent(
      new PointerEvent("pointerup", { clientX: 10, clientY: 60, button: 0, bubbles: true }),
    );
    expect(debug().offsetFacePhase).toBe("armed");
    expect(clientMock.endPreview).not.toHaveBeenCalledWith(expect.anything(), true);
  });

  it("does NOT grab for an absolute distance type (there is no zero to drag from)", async () => {
    build();
    await arm();
    chipType("Diameter");
    await flush();
    await flush();
    await flush();
    expect(debug().offsetDistanceType).toBe("Diameter");
    container.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: 10, clientY: 10, button: 0, bubbles: true }),
    );
    expect(debug().offsetFacePhase).toBe("armed");
  });

  it("seeds an absolute type from the handshake's currentDims", async () => {
    build();
    await arm();
    chipType("Diameter");
    await flush();
    await flush();
    await flush();
    // `currentDims.radius` is 10 ⇒ the diameter opens at 20, not at a made-up default.
    expect(debug().offsetDistance).toBe(20);
  });

  it("a distance-type switch RE-RUNS the handshake (the closure depends on it)", async () => {
    build();
    await arm();
    expect(clientMock.prepareOffsetFace).toHaveBeenCalledTimes(1);
    chipType("Radius");
    await flush();
    await flush();
    await flush();
    expect(clientMock.prepareOffsetFace).toHaveBeenCalledTimes(2);
    expect(clientMock.prepareOffsetFace.mock.calls[1][0]).toMatchObject({ distanceType: "Radius" });
  });

  // ── commit ────────────────────────────────────────────────────────────────

  it("commits through endPreview once the exact candidate has landed", async () => {
    build();
    await arm();
    await settleTrailing();
    await confirmAndSettle(true);
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
    expect(toolStore.getState().modelTool).toBe("select");
    expect(engineMock.hideValueHandle).toHaveBeenCalled();
    expect(debug().offsetFacePhase).toBe("idle");
  });

  it("FAILS CLOSED when the exact preview never answers (the barrier timing out is not approval)", async () => {
    build();
    await arm();
    await settleTrailing();
    // No `answerExactPreview()` — the generic barrier resolves ok on timeout, and
    // for every other tool that is fine (the backend re-validates). Not here: the
    // frozen closure would reach the timeline unevaluated.
    await confirmAndSettle(false);
    expect(clientMock.endPreview).not.toHaveBeenCalledWith(expect.anything(), true);
    expect(debug().offsetFacePhase).toBe("armed");
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });

  it("records the handshake's revision as the commit gate, and drops it on cancel", async () => {
    build();
    await arm();
    // The gate `commitOffsetFace` checks: the closure was resolved against THIS
    // head, and a moved head can silently re-point its TopoKeys.
    expect(debug().offsetPrepared).toBe(documentStore.getState().revision);
    toolStore.getState().setTool("select");
    await flush();
    expect(debug().offsetPrepared).toBeNull();
  });

  it("refuses a degenerate |Offset| WITHOUT rewriting the number", async () => {
    build();
    await arm();
    chipValue(0.0002);
    chipConfirm();
    await flush();
    expect(clientMock.endPreview).not.toHaveBeenCalledWith(expect.anything(), true);
    // SCHEMA §7.3 forbids clamping: the value the user typed is still the value.
    expect(debug().offsetDistance).toBe(0.0002);
    expect(debug().offsetFacePhase).toBe("armed");
  });

  it("drops the arm when the DOCUMENT revision moves underneath it", async () => {
    build();
    await arm();
    expect(debug().offsetFacePhase).toBe("armed");
    documentStore.getState().applyChange({ revision: 99, features: [], bodies: {} });
    await flush();
    expect(debug().offsetFacePhase).toBe("idle");
    expect(toolStore.getState().modelTool).toBe("select");
    expect(engineMock.hideValueHandle).toHaveBeenCalled();
  });

  // ── L1 ghost lifecycle ────────────────────────────────────────────────────

  it("hides the L1 ghost once an exact candidate lands, and restores it on failure", async () => {
    build();
    await arm();
    await settleTrailing();
    const first = lastUpdate()!;
    engineMock.hideGhostPreview.mockClear();

    previewCb?.({
      sessionId: first[0] as string,
      epoch: first[2] as number,
      bodyId: "pb-1",
      // A REAL MESH1 blob: the ingest path parses it, so a stub would throw
      // before the ghost swap under test ever runs.
      mesh: makeBoxMesh(),
    });
    // The exact result supersedes the ghost — both at once would draw the faces twice.
    expect(engineMock.hideGhostPreview).toHaveBeenCalled();

    // A NEW value ⇒ a new epoch. The failure has to ride an epoch the throttle
    // really sent, or the lane discards it as stale before the failure path runs.
    engineMock.showGhostPreviewMulti.mockClear();
    chipValue(4);
    await settleTrailing();
    const second = lastUpdate()!;
    expect(second[2] as number).toBeGreaterThan(first[2] as number);
    previewCb?.({
      sessionId: second[0] as string,
      epoch: second[2] as number,
      bodyId: "pb-1",
      error: { kind: "opFailed", message: "offset produced no solid", structural: false },
    });
    // The candidate is gone; the ghost comes back rather than leaving the last
    // GOOD mesh on screen under a failure message.
    expect(engineMock.showGhostPreviewMulti).toHaveBeenCalled();
  });

  // ── cancel / re-edit ──────────────────────────────────────────────────────

  it("cancel hides the SHARED drag arrow (R3) and clears the chip", async () => {
    build();
    await arm();
    toolStore.getState().setTool("select");
    await flush();
    expect(engineMock.hideValueHandle).toHaveBeenCalled();
    expect(engineMock.hideGhostPreview).toHaveBeenCalled();
    expect(toolChipStore.getState().kind).toBe("none");
    expect(debug().offsetFacePhase).toBe("idle");
  });

  it("re-edit arms L1-only: no handshake, no preview, and commits a scalar merge", async () => {
    build();
    documentStore.setState({
      features: [
        {
          id: "feat-1",
          kind: "fillet",
          opType: "OffsetFace",
          label: "Offset face",
          valueText: "2.5 mm",
          primaryValue: 2.5,
          status: "ok",
        },
      ],
    });
    await controller.editOffsetFaceFeature("feat-1");
    await flush();
    // Re-preparing would compute a NEW closure against today's geometry, quietly
    // replacing the operative set the user authored.
    expect(clientMock.prepareOffsetFace).not.toHaveBeenCalled();
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(debug().offsetDistance).toBe(2.5);

    chipValue(6);
    chipConfirm();
    await flush();
    await flush();
    const editCalls = clientMock.applyEditCommand.mock.calls as unknown[][];
    const cmd = editCalls[editCalls.length - 1][0] as {
      cmd: string;
      record: string;
      op: { opType: string; params: Record<string, unknown> };
    };
    expect(cmd.cmd).toBe("updateOperationParams");
    expect(cmd.record).toBe("feat-1");
    expect(cmd.op.opType).toBe("OffsetFace");
    expect(cmd.op.params.distance).toEqual({ value: 6 });
    // The frozen closure + target survive the scalar patch verbatim.
    expect(cmd.op.params.faceIds).toEqual(["el-f2"]);
    expect(cmd.op.params.targetBodyId).toBe("body1");
  });
});
