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
import { canConfirmActiveTool } from "./activeToolPresentation";
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
    showValueHandlePath: vi.fn(),
    setValueHandleValue: vi.fn(),
    showValueWitness: vi.fn(),
    hideValueWitness: vi.fn(),
    showScreenValueHandle: vi.fn(),
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
    onDocumentChanged: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => null),
    finishSketch: vi.fn(() => Promise.resolve({ regions: [] })),
    getSketchRegions: vi.fn(() => Promise.resolve({ regions: [] })),
    prepareOffsetFace: vi.fn(prepare),
    // H10 resolves the display attachment through the same classification the
    // shell wall uses. `null` is the ordinary answer for a face the backend
    // cannot classify, and it is what the planar specs below want.
    classifyElement: vi.fn((_bodyId: string, _elementId: string): Promise<unknown> => Promise.resolve(null)),
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
    // Annotated to the CadClient signature, not to the seed literal: the re-edit
    // specs swap in stored params of other shapes (a V2 record, a legacy one).
    getOperationParams: vi.fn(
      (): Promise<Record<string, unknown>> =>
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
  offsetAttachment?: string;
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

  it("does NOT promote or adopt a closure after the snapshot fence refuses it", async () => {
    build(() => Promise.reject(new Error("prepared offset-face closure is stale — re-pick")));
    await arm();

    expect(debug().offsetFacePhase).toBe("idle");
    expect(clientMock.promoteSelection).not.toHaveBeenCalled();
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toContain("stale");
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
    // H10: the arrow rides `H(q) = P + q·n`, so the value moves it — the old
    // `showValueHandle(anchor + axis·d, axis)` re-anchored it per frame instead.
    expect(engineMock.showValueHandlePath).toHaveBeenCalledWith(
      { q0Mm: 0, point0Mm: [0, 0, 10], dPointDValue: [0, 0, 1] },
      2,
    );
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

  it("DEGRADES to a visible screen proxy (no geometric arrow or ghost) for a curved face", async () => {
    planar = false;
    build();
    await arm();
    expect(debug().offsetFacePhase).toBe("armed");
    expect(debug().offsetDegraded).toBe(true);
    expect(engineMock.showValueHandlePath).not.toHaveBeenCalled();
    // The pick's own anchor — the §5 ladder's second rung, never `[0,0,0]`.
    expect(engineMock.showScreenValueHandle).toHaveBeenCalledWith([0, 0, 10], 2);
    expect(debug().offsetAttachment).toBe("pick");
    expect(engineMock.showGhostPreviewMulti).not.toHaveBeenCalled();
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
  });

  it("does not start a degraded offset drag from empty viewport space", async () => {
    planar = false;
    build();
    await arm();
    engineMock.hitExtrudeHandle.mockReturnValue(false);

    container.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: 10, clientY: 10, button: 0, bubbles: true }),
    );

    expect(debug().offsetFacePhase).toBe("armed");
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

  it("GRABS for an absolute distance type too (H10, review R08)", async () => {
    // SCHEMA §7.3 already fixes the reference an absolute dimension is read
    // against, so the old refusal ("no zero to drag from") left `Total`, `Radius`
    // and `Diameter` typed-only for no reason the wire supports.
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
    expect(debug().offsetFacePhase).toBe("dragging");
  });

  it("refuses a press when the ladder reached NOTHING attributable", async () => {
    // No mesh frame, no pick anchor and no evidence anchor: §3 "No usable depth"
    // disables the drag instead of seating a handle on the world origin (N2).
    planar = false;
    build((req) => Promise.resolve({ ...closureOf(req), faces: [{ topoKey: "f:2", picked: true }] }));
    await arm([{ ...FACE, anchor: undefined }]);
    expect(debug().offsetAttachment).toBe("none");
    expect(engineMock.hideValueHandle).toHaveBeenCalled();
    expect(engineMock.showScreenValueHandle).not.toHaveBeenCalled();
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
    // A LEGACY record (no primaries) has nothing to re-author against, so the
    // version key must not appear and the ordinary hint stands.
    expect(cmd.op.params).not.toHaveProperty("resultPolicyVersion");
    expect(viewportStore.getState().statusHint?.message).toBe("Offset distance updated");
  });

  it("a re-edit is CONFIRMABLE: the stored faces keep the context, so Enter commits", async () => {
    // `showOffsetFaceChip` republishes the typed context on every arm, but a
    // re-edit has NO picks — `setTool("offsetFace")` fires `cancelOffsetFace`,
    // which clears `offsetFaces` and `offsetTargetBodyId`. Built from the live
    // picks alone the context is EMPTY, which `missingRequiredTargetMessage` reads
    // as "Affected body is no longer available": that disables the chip's ✓
    // (`ModelToolChips.tsx`) and drops Enter out of `armedConfirm`'s table, so an
    // OffsetFace edited from history could never be applied. Calling
    // `onConfirm?.()` directly (the spec above) cannot see this — it skips the gate.
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

    // The context names the RECORD's stored face + target body, not a fresh pick.
    const context = toolChipStore.getState().context;
    expect(context?.tool).toBe("offsetFace");
    expect(canConfirmActiveTool(toolChipStore.getState())).toBe(true);

    chipValue(6);
    // The REAL key path: capture-phase Enter on `window`, with no editable focus.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();
    await flush();

    const editCalls = clientMock.applyEditCommand.mock.calls as unknown[][];
    expect(editCalls).toHaveLength(1);
    const cmd = editCalls[0][0] as { cmd: string; record: string; op: { opType: string; params: Record<string, unknown> } };
    expect(cmd.cmd).toBe("updateOperationParams");
    expect(cmd.record).toBe("feat-1");
    expect(cmd.op.params.distance).toEqual({ value: 6 });
    expect(cmd.op.params.targetBodyId).toBe("body1");
  });

  // ── re-edit re-authoring: V2 → V3 (WP3-C5) ───────────────────────────────

  /** Seed the stored-params mock, then run one distance re-edit to completion. */
  async function reeditStored(
    stored: Record<string, unknown>,
  ): Promise<{ opType: string; params: Record<string, unknown> }> {
    build();
    clientMock.getOperationParams.mockImplementation(() => Promise.resolve(stored));
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
    chipValue(6);
    chipConfirm();
    await flush();
    await flush();
    const calls = clientMock.applyEditCommand.mock.calls as unknown[][];
    return (calls[calls.length - 1][0] as { op: { opType: string; params: Record<string, unknown> } })
      .op;
  }

  const V2_STORED = {
    faceIds: ["el-f2", "el-blend"],
    primaryFaceIds: ["el-f2"],
    resultPolicyVersion: 2,
    faces: [
      { primary: { bodyId: "body1", elementId: "el-f2", kind: "face" } },
      { primary: { bodyId: "body1", elementId: "el-blend", kind: "face" } },
    ],
    distance: { value: 2.5 },
    distanceType: "Offset",
    chainTangentFaces: true,
    targetBodyId: "body1",
  };

  it("re-authors a stored V2 record to V3 and SAYS SO in visible chrome", async () => {
    const op = await reeditStored({ ...V2_STORED });
    expect(op.params.resultPolicyVersion).toBe(3);
    expect(op.params.distance).toEqual({ value: 6 });
    // The frozen pair the new reading depends on is untouched.
    expect(op.params.faceIds).toEqual(["el-f2", "el-blend"]);
    expect(op.params.primaryFaceIds).toEqual(["el-f2"]);
    // The geometry this record rebuilds to CHANGES, so the user is told.
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Offset face re-authored: fillets are now preserved (V3)",
    );
  });

  it("does NOT announce anything when the record is already V3", async () => {
    const op = await reeditStored({ ...V2_STORED, resultPolicyVersion: 3 });
    expect(op.params.resultPolicyVersion).toBe(3);
    expect(viewportStore.getState().statusHint?.message).toBe("Offset distance updated");
  });

  it("leaves a V2 record with NO primaries alone — a version without them is refused", async () => {
    const op = await reeditStored({ ...V2_STORED, primaryFaceIds: [] });
    expect(op.params.resultPolicyVersion).toBe(2);
    expect(viewportStore.getState().statusHint?.message).toBe("Offset distance updated");
  });

  // ── the grab's mapping owns the drag (TODO.md SESSION 37 H3, R03) ────────────

  /**
   * The engine seam the real `ViewportEngine` exposes, reduced to what a grab
   * reads. `showValueHandle` drops the freeze exactly as `DragHandle.reset()` does,
   * so a glyph that stays frozen through the drag proves the controller re-froze it.
   */
  function withMapping(worldPerPx: { value: number }, kind: "world" | "proxy") {
    const frozen = { value: null as string | null };
    return Object.assign(engineMock, {
      valueHandleMapping: vi.fn(() =>
        kind === "world"
          ? {
              kind: "world" as const,
              q0Mm: 0,
              direction: [0, -1] as const,
              g0PxPerMm: 1 / worldPerPx.value,
              kPerMm: 0,
              validDeltaMm: [-Infinity, Infinity] as const,
              conditioning: 1,
            }
          : {
              kind: "proxy" as const,
              q0Mm: 0,
              direction: [0, -1] as const,
              mmPerPx: worldPerPx.value,
              reason: "poorScreenSensitivity" as const,
            },
      ),
      freezeValueHandle: vi.fn((m: { kind: string } | null) => (frozen.value = m ? m.kind : null)),
      showValueHandle: vi.fn(() => (frozen.value = null)),
      frozen,
    });
  }

  const drag = (type: string, y: number): void => {
    container.dispatchEvent(new PointerEvent(type, { clientX: 10, clientY: y, button: 0, buttons: type === "pointerup" ? 0 : 1, bubbles: true }));
  };

  it("an arrow that maps as a screen proxy drags vertically at the grab's scale, says so, and stays frozen", async () => {
    build();
    await arm();
    answerExactPreview();
    const armHint = viewportStore.getState().statusHint?.message;
    const engine = withMapping({ value: 0.25 }, "proxy");
    const start = debug().offsetDistance as number;

    drag("pointerdown", 100);
    drag("pointermove", 60); // 40 px UP

    expect(toolChipStore.getState().value).toBeCloseTo(start + 40 * 0.25, 9); // drag frames do not republish the debug surface
    expect(viewportStore.getState().statusHint?.message).toBe("Offset: drag vertically");
    // H10: the per-frame update is `setValueHandleValue`, which does not reset the
    // shared handle — so the grab's freeze survives with no re-assert to forget.
    expect(engine.setValueHandleValue).toHaveBeenCalledWith(toolChipStore.getState().value);
    expect(engine.showValueHandle).not.toHaveBeenCalled();
    expect(engine.frozen.value).toBe("proxy"); // the glyph never unfroze

    drag("pointerup", 60);
    expect(engine.freezeValueHandle).toHaveBeenLastCalledWith(null);
    expect(viewportStore.getState().statusHint?.message).toBe(armHint);
  });

  it("a DEGRADED offset proxy keeps the scale it grabbed with", async () => {
    planar = false;
    build();
    await arm();
    const scale = { value: 0.25 };
    withMapping(scale, "proxy");
    const start = debug().offsetDistance as number;

    drag("pointerdown", 100);
    drag("pointermove", 90);
    expect(toolChipStore.getState().value).toBeCloseTo(start + 10 * 0.25, 9);

    scale.value = 5;
    engineMock.planePixelWorld.mockReturnValue(9);
    drag("pointermove", 80);
    expect(toolChipStore.getState().value).toBeCloseTo(start + 20 * 0.25, 9);
  });

  it("Escape in the distance field reverts through the controller: the refusal and its status line go", async () => {
    build();
    await arm();
    chipType("Diameter");
    await flush();
    await flush();
    await flush();
    answerExactPreview();
    const armHint = viewportStore.getState().statusHint?.message;
    const start = debug().offsetDistance as number;

    chipValue(-5);
    expect(toolChipStore.getState().validation.status).toBe("invalid");
    expect(viewportStore.getState().statusHint?.severity).toBe("error");

    const revert = toolChipStore.getState().onRevertValue;
    expect(revert).toBeTypeOf("function");
    revert?.(start);

    expect(toolChipStore.getState().validation).toEqual({ status: "valid" });
    expect(toolChipStore.getState().valueError).toBe(false);
    expect(debug().offsetDistance).toBe(start);
    expect(viewportStore.getState().statusHint?.message).toBe(armHint);
  });
});

/*
 * H10 — WHERE AN OFFSET'S VALUE ATTACHES, at the controller seam
 * (docs/design/astra/modeling-handle-attachment.md §5 "Offset: …", review R08).
 *
 * The pure geometry is `faceOffset.test.ts`'s to prove. What these specs pin is
 * the WIRING: which classification each distance type asks for, which path it
 * installs, what the witness is allowed to claim, and the fact that a handle is
 * drawn only where it can be dragged.
 */
describe("ModelToolController OffsetFace — attachment (H10)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  /** A 6 mm BORE: the inner-wall case the drag-inversion counterexample uses. */
  const BORE: EntityRef = {
    kind: "face",
    id: "body1#f:9",
    bodyId: "body1",
    topoKey: "f:9",
    elementId: "el-face-9",
    anchor: { worldPoint: [6, 0, 0] },
  };
  const boreClosure = (req: PrepareOffsetFaceRequest): PrepareOffsetFaceResult => ({
    snapshotId: 1,
    targetBodyId: req.pickedFaces[0]?.bodyId ?? "",
    faces: [{ topoKey: "f:9", picked: true, anchor: { worldPoint: [6, 0, 0] as [number, number, number] } }],
    currentDims: { radius: 6 },
    refusal: null,
  });
  const cylinder = {
    kind: "face",
    surfaceType: "cylinder",
    curveType: "",
    frame: {
      origin: [0, 0, 0] as [number, number, number],
      normal: null,
      axis: [0, 0, 1] as [number, number, number],
      radius: 6,
      sidedness: "hole" as const,
    },
  };
  const plane = (origin: [number, number, number]) => ({
    kind: "face",
    surfaceType: "plane",
    curveType: "",
    frame: { origin, normal: [0, 0, 1] as [number, number, number], axis: null, radius: null },
  });

  function build(
    prepare: (req: PrepareOffsetFaceRequest) => Promise<PrepareOffsetFaceResult>,
  ): void {
    controller?.dispose();
    engineMock = makeEngineMock();
    clientMock = makeClientMock(undefined, prepare);
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  }

  async function arm(faces: EntityRef[]): Promise<void> {
    selectionStore.getState().set(faces);
    toolStore.getState().setTool("offsetFace");
    await flush();
    await flush();
    await flush();
  }

  async function switchType(t: OffsetDistanceType): Promise<void> {
    toolChipStore.getState().onDistanceType?.(t);
    await flush();
    await flush();
    await flush();
  }

  /** The newest path the arrow was seated on. */
  const lastPath = (): { q0Mm: number; point0Mm: number[]; dPointDValue: number[] } | undefined => {
    const calls = engineMock.showValueHandlePath.mock.calls;
    return calls[calls.length - 1]?.[0] as never;
  };
  /** The newest witness list the engine was handed, normalized to an array. */
  const lastWitnesses = (): { meaning: string; label: string; fromMm: number[]; toMm: number[] }[] => {
    const calls = engineMock.showValueWitness.mock.calls;
    const arg = calls[calls.length - 1]?.[0] as never;
    return (Array.isArray(arg) ? arg : arg ? [arg] : []) as never;
  };

  /** Every status line published during a spec — the arm hint is transient. */
  let hints: string[] = [];
  let unsubscribeHints: (() => void) | undefined;

  beforeEach(() => {
    planar = true;
    resetStores();
    container = document.createElement("div");
    document.body.appendChild(container);
    hints = [];
    unsubscribeHints = viewportStore.subscribe((state) => {
      const message = state.statusHint?.message;
      if (message && hints[hints.length - 1] !== message) hints.push(message);
    });
  });

  afterEach(() => {
    unsubscribeHints?.();
    controller?.dispose();
    container.remove();
    toolStore.getState().setTool("select");
  });

  it("a signed Offset on a BORE rides σ·r̂ — into the material, not outward", async () => {
    planar = false; // a cylinder has no planar frame; the classification answers
    build((req) => Promise.resolve(boreClosure(req)));
    clientMock.classifyElement.mockResolvedValue(cylinder as never);
    await arm([BORE]);

    expect(debug().offsetAttachment).toBe("cylinder");
    // H(q) = C + (R0 + σq)·r̂, seated at the CLASSIFIED radius, σ = −1 for a hole.
    expect(lastPath()).toEqual({ q0Mm: 0, point0Mm: [6, 0, 0], dPointDValue: [-1, 0, 0] });
  });

  it("Radius on the SAME bore grows OUTWARD — the §7 drag-inversion counterexample", async () => {
    planar = false;
    build((req) => Promise.resolve(boreClosure(req)));
    clientMock.classifyElement.mockResolvedValue(cylinder as never);
    await arm([BORE]);
    await switchType("Radius");

    expect(debug().offsetDistanceType).toBe("Radius");
    // Seeded from the kernel's own measurement, not a made-up default.
    expect(debug().offsetDistance).toBe(6);
    // dH/dR = +r̂ — the OPPOSITE of the bore's material-outward normal (−r̂, which
    // the signed-Offset path above rides). Driving Radius by the material normal
    // would run this drag backwards; σ only maps to the kernel's signed `d`.
    expect(lastPath()).toEqual({ q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [1, 0, 0] });
    // The witness spans the centreline to the radial target and claims no more.
    expect(lastWitnesses()).toEqual([
      expect.objectContaining({ meaning: "targetConstruction", fromMm: [0, 0, 0], toMm: [6, 0, 0] }),
    ]);
  });

  it("Diameter carries the ½ in its derivative, so the same drag moves half as far", async () => {
    planar = false;
    build((req) => Promise.resolve(boreClosure(req)));
    clientMock.classifyElement.mockResolvedValue(cylinder as never);
    await arm([BORE]);
    await switchType("Diameter");

    expect(debug().offsetDistance).toBe(12); // 2 × currentDims.radius
    expect(lastPath()).toEqual({ q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [0.5, 0, 0] });
    // Ø is drawn as the supporting cylinder's own diameter construction.
    expect(lastWitnesses()).toEqual([
      expect.objectContaining({ fromMm: [-6, 0, 0], toMm: [6, 0, 0], meaning: "targetConstruction" }),
    ]);
  });

  it("an absolute type whose classification REFUSES keeps a labelled proxy, not an arrow", async () => {
    planar = false;
    build((req) => Promise.resolve(boreClosure(req)));
    clientMock.classifyElement.mockResolvedValue(null);
    await arm([BORE]);
    await switchType("Radius");

    expect(engineMock.showValueHandlePath).not.toHaveBeenCalled();
    expect(engineMock.showScreenValueHandle).toHaveBeenLastCalledWith([6, 0, 0], 6);
    expect(debug().offsetAttachment).toBe("pick");
    // The status line says what the control can actually do — never "drag the
    // arrow" when there is no arrow (audit N3).
    expect(hints.some((h) => h.includes("drag visible distance control or type"))).toBe(true);
    expect(hints.some((h) => h.includes("drag the arrow"))).toBe(false);
  });

  it("Total draws the PREPARED reference thickness beside its target", async () => {
    // Selected plane at z = 10, opposite at z = 6, prepared t0 = 4 ⇒ B = (0,0,6).
    build((req) =>
      Promise.resolve({
        ...closureOf(req),
        currentDims: { thickness: 4 },
        oppositeFace: {
          topoKey: "f:3",
          picked: false,
          anchor: { worldPoint: [0, 0, 6] as [number, number, number] },
        },
      }),
    );
    clientMock.classifyElement.mockImplementation((_b: string, elementId: string) =>
      Promise.resolve((elementId === "el-f3" ? plane([0, 0, 6]) : plane([0, 0, 10])) as never),
    );
    await arm([FACE]);
    await switchType("Total");

    expect(debug().offsetDistanceType).toBe("Total");
    expect(debug().offsetDistance).toBe(4); // seeded from currentDims.thickness
    expect(lastPath()).toEqual({ q0Mm: 0, point0Mm: [0, 0, 6], dPointDValue: [0, 0, 1] });

    toolChipStore.getState().onValue?.(6);
    expect(lastWitnesses()).toEqual([
      // P ↔ B is the 4 mm the PREPARE measured…
      expect.objectContaining({ meaning: "measuredReference", fromMm: [0, 0, 10], toMm: [0, 0, 6] }),
      // …and B ↔ H(T) is the 6 mm nobody has built yet.
      expect.objectContaining({ meaning: "targetConstruction", fromMm: [0, 0, 6], toMm: [0, 0, 12] }),
    ]);
  });

  it("a MULTI-face closure drags a shared PARAMETER, not a moving centroid", async () => {
    const SECOND: EntityRef = { ...FACE, id: "body1#f:4", topoKey: "f:4", elementId: "el-face-4" };
    build((req) => Promise.resolve(closureOf(req)));
    await arm([FACE, SECOND]);

    expect(debug().offsetFaceCount).toBe(2);
    expect(debug().offsetAttachment).toBe("shared");
    // A = mean(reference points), a = offsetAxisFor(normals). Both faces share the
    // mocked frame, so the mean is that frame's own centre and normal.
    expect(lastPath()).toEqual({ q0Mm: 0, point0Mm: [0, 0, 10], dPointDValue: [0, 0, 1] });
    // No per-face motion is claimed: a V3 closure can contain rebuilt blends and
    // fixed supports whose roles the frontend does not have.
    expect(lastWitnesses()).toEqual([
      expect.objectContaining({ meaning: "parameterConstruction" }),
    ]);
    expect(lastWitnesses()[0].label).toContain("shared by 2 faces");
  });

  it("draws NO L1 ghost for an absolute type — `distance` there is a position", async () => {
    build((req) => Promise.resolve({ ...closureOf(req), currentDims: { radius: 6 } }));
    await arm([FACE]);
    expect(engineMock.showGhostPreviewMulti).toHaveBeenCalled(); // the signed Offset does
    engineMock.showGhostPreviewMulti.mockClear();

    await switchType("Radius");
    toolChipStore.getState().onValue?.(9);
    // Translating the face by 9 mm would draw a 9 mm move for a 3 mm change.
    expect(engineMock.showGhostPreviewMulti).not.toHaveBeenCalled();
  });

  it("offers no Radius/Diameter on a planar closure, and draws no handle for one", async () => {
    build((req) => Promise.resolve({ ...closureOf(req), currentDims: { thickness: 12 } }));
    await arm([FACE]);
    expect(toolChipStore.getState().distanceTypes).toEqual(["Offset", "Total"]);
  });

  it("a distance-type switch does not leave the previous type's handle on screen", async () => {
    // Total IS offered here (opposite + prepared thickness), but neither face
    // classifies, so it has no path — and the Offset arrow that was on screen for
    // the previous type must not survive the switch.
    build((req) =>
      Promise.resolve({
        ...closureOf(req),
        currentDims: { thickness: 4 },
        oppositeFace: { topoKey: "f:3", picked: false },
      }),
    );
    await arm([FACE]);
    expect(engineMock.showValueHandlePath).toHaveBeenCalledTimes(1);
    engineMock.hideValueWitness.mockClear();

    await switchType("Total");
    expect(debug().offsetDistanceType).toBe("Total");
    expect(engineMock.showValueHandlePath).toHaveBeenCalledTimes(1); // no new arrow
    expect(engineMock.hideValueWitness).toHaveBeenCalled(); // the old claim is gone
    expect(engineMock.showScreenValueHandle).toHaveBeenLastCalledWith([0, 0, 10], 4);
  });
});
