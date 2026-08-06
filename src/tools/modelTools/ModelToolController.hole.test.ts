/*
 * ModelToolController — the HOLE tool (WP-C T3), jsdom.
 *
 * The hole is the first model tool whose gesture starts with a PICK and whose
 * params have CONDITIONAL blocks, so these specs pin the two things nothing else
 * covers:
 *
 *  1. VALIDATE BEFORE ARM. A non-planar face is refused inline (the
 *     sketch-on-face `tryEnterOnSelectedFace` precedent) instead of being sent to
 *     the kernel to come back as an errored history row. Planarity is read from
 *     the LOCAL mesh (`faceFrame`), so a pointer click costs no round trip.
 *  2. PREVIEW ≡ COMMIT, including the conditional blocks. The params pushed on the
 *     preview lane must be byte-equal to the ones the ✓ commits — the house rule
 *     `ipc/previewOps.ts` exists to enforce, restated here at the call site
 *     because the hole's params are assembled from FSM state rather than read off
 *     a single chip.
 *
 * `getEntry`/`faceFrame` are mocked: a real planarity answer needs a MESH1 view,
 * and what is under test is which ANSWER the controller acts on, not how the
 * answer is computed (`alignSolve.test.ts` owns that).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  OperationOp,
  PreviewDraft,
  PreviewResult,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

/** Flipped per-test: what `faceFrame` reports for the clicked face. */
let planar = true;

vi.mock("@/viewport/mesh/meshRegistry", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  getEntry: vi.fn(() => ({ faceIndex: { ordinalForId: () => 0 } })),
}));

vi.mock("@/tools/preview/alignSolve", async (orig) => ({
  ...(await orig<Record<string, unknown>>()),
  faceFrame: vi.fn(() => (planar ? { center: [10, 10, 25], normal: [0, 0, 1] } : null)),
}));

const okResult = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#2" }],
  removedBodies: [],
});

const FACE_HIT = {
  kind: "face" as const,
  bodyId: "body1",
  topoKey: "f:2",
  elementId: undefined,
  distance: 1,
  worldPos: { x: 10, y: 10, z: 25 },
};

function makeEngineMock() {
  return {
    setOrbitSuppressed: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    hitTransformGizmo: vi.fn(() => null),
    setExtrudeHandleHover: vi.fn(),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    setPreviewTint: vi.fn(),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    clearPreviewBody: vi.fn(),
    hideRegionPick: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    setDatumGhost: vi.fn(),
    mountChip: vi.fn(),
    unmountChip: vi.fn(),
    probePick: vi.fn(() => FACE_HIT as unknown as null),
  };
}

function makeClientMock(
  stored: Record<string, unknown> | undefined,
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
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn((_op: OperationOp) => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
    promoteSelection: vi.fn(() => Promise.resolve([{ topoKey: "f:2", elementId: "el-face-2" }])),
    getOperationParams: vi.fn(() => Promise.resolve(stored ?? {})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
/** Past the 200 ms hole/shell trailing floor, so a coalesced push actually goes out. */
const settleTrailing = (): Promise<void> => new Promise((r) => setTimeout(r, 320));

describe("ModelToolController — Hole", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let previewCb: ((r: PreviewResult) => void) | null;

  function debug(): Record<string, unknown> {
    return (window as unknown as { __extrudePreview: Record<string, unknown> }).__extrudePreview;
  }

  function build(stored?: Record<string, unknown>): void {
    engineMock = makeEngineMock();
    previewCb = null;
    clientMock = makeClientMock(stored, (cb) => {
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

  /** A click (down+up at the same point) inside the viewport container. */
  function clickAt(x = 40, y = 40): void {
    container.dispatchEvent(
      new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, bubbles: true }),
    );
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, bubbles: true }),
    );
  }

  async function armHole(): Promise<void> {
    toolStore.getState().setTool("hole");
    await flush();
    clickAt();
    await flush();
  }

  /** The newest `updatePreview` dispatch. */
  function lastUpdate(): { sessionId: string; epoch: number; params: Record<string, unknown> } {
    const calls = clientMock.updatePreview.mock.calls;
    const last = calls[calls.length - 1];
    return {
      sessionId: last[0] as string,
      params: last[1] as Record<string, unknown>,
      epoch: last[2] as number,
    };
  }

  function answerPreview(): void {
    const { sessionId, epoch } = lastUpdate();
    previewCb?.({ sessionId, epoch, bodyId: "preview", bodies: [], replacedBodyIds: ["body1"] });
  }

  /**
   * Let the trailing window open, then ANSWER the outstanding candidate. The lane
   * holds one epoch in flight at a time, so without the answer every later
   * `sendPreview` coalesces behind it and `lastUpdate()` would keep reporting the
   * arm's params (the edge-op/shell specs pump the same way).
   */
  async function pump(): Promise<void> {
    await settleTrailing();
    answerPreview();
    await flush();
  }

  beforeEach(() => {
    resetStores();
    planar = true;
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  // ── the two-phase gesture ───────────────────────────────────────────────────

  it("activating the tool waits for a face click before arming anything", async () => {
    build();
    toolStore.getState().setTool("hole");
    await flush();
    expect(debug().holePhase).toBe("facePick");
    expect(debug().holePoint).toBeNull();
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(toolChipStore.getState().kind).toBe("none");
    expect(viewportStore.getState().statusHint?.message).toMatch(/flat face/i);
  });

  it("a planar face click arms at the clicked point, mints the seat, and previews it", async () => {
    build();
    await armHole();
    expect(debug().holePhase).toBe("armed");
    expect(debug().holePoint).toEqual([10, 10, 25]);
    expect(debug().holeBodyId).toBe("body1");
    // The TopoKey is promoted to a Rust-minted ElementId — only a minted id
    // carries the identity the resolution ladder can rebind.
    expect(clientMock.promoteSelection).toHaveBeenCalledWith("body1", [
      { topoKey: "f:2", anchor: { worldPoint: [10, 10, 25] } },
    ]);
    expect(toolChipStore.getState().kind).toBe("hole");

    const draft = clientMock.beginPreview.mock.calls[0][0];
    expect(draft.opType).toBe("Hole");
    expect(draft.params).toMatchObject({
      targetBodyId: "body1",
      point: [10, 10, 25],
      holeType: "simple",
      depth: null, // a fresh hole is through-all
    });
    // SCHEMA §7.3 `inputs`: host body, then host face.
    expect(draft.inputs?.[0].primary).toEqual({ bodyId: "body1", kind: "body" });
    expect(draft.inputs?.[1].primary).toEqual({
      bodyId: "body1",
      elementId: "el-face-2",
      kind: "face",
    });
  });

  it("REFUSES a non-planar face inline and stays in facePick", async () => {
    build();
    planar = false;
    await armHole();
    expect(debug().holePhase).toBe("facePick");
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toMatch(/not flat/i);
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });

  it("a second click on the SAME face MOVES the hole — no second session", async () => {
    build();
    await armHole();
    await pump();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);

    engineMock.probePick.mockReturnValue({
      ...FACE_HIT,
      elementId: "el-face-2",
      worldPos: { x: 4, y: 5, z: 25 },
    } as unknown as null);
    clickAt(60, 60);
    await flush();

    expect(debug().holePhase).toBe("armed");
    expect(debug().holePoint).toEqual([4, 5, 25]);
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
    expect(clientMock.promoteSelection).toHaveBeenCalledTimes(1); // no re-promotion
    await settleTrailing();
    expect(lastUpdate().params.point).toEqual([4, 5, 25]);
  });

  // ── chips + conditional blocks ──────────────────────────────────────────────

  it("the chip cluster publishes the conditional block of the ACTIVE profile only", async () => {
    build();
    await armHole();
    await pump();
    const chip = () => toolChipStore.getState();
    expect(chip().holeType).toBe("simple");

    chip().onHoleType?.("counterbore");
    await flush();
    expect(chip().holeType).toBe("counterbore");
    await pump();
    chip().onCbDiameter?.(14);
    await pump();
    chip().onCbDepth?.(7);
    await settleTrailing();
    expect(lastUpdate().params).toMatchObject({
      holeType: "counterbore",
      cbDiameter: 14,
      cbDepth: 7,
      csDiameter: null,
      csAngleDeg: null,
    });

    // Flip to countersink: the cb block leaves the RECORD but not the tool state.
    await pump();
    chip().onHoleType?.("countersink");
    await pump();
    chip().onCsAngle?.(82);
    await settleTrailing();
    expect(lastUpdate().params).toMatchObject({
      holeType: "countersink",
      csAngleDeg: 82,
      cbDiameter: null,
      cbDepth: null,
    });
    expect(debug().holeCbDiameter).toBe(14); // still remembered
  });

  it("a standards pick fills raw millimetres into the params", async () => {
    build();
    await armHole();
    await pump();
    toolChipStore.getState().onHoleType?.("counterbore");
    await flush();
    toolChipStore.getState().onStandard?.("M6", "normal");
    await settleTrailing();
    // ISO 273 medium clearance + the DIN 974-1 row-1 counterbore for an M6 SHCS.
    expect(lastUpdate().params).toMatchObject({
      diameter: 6.6,
      cbDiameter: 11,
      cbDepth: 6.8,
    });
    // The thread designation is a FRONTEND label and never travels.
    expect(JSON.stringify(lastUpdate().params)).not.toContain("M6");
  });

  it("the depth chip authors a blind depth and returns to through-all", async () => {
    build();
    await armHole();
    await pump();
    toolChipStore.getState().onDepth?.(12);
    await settleTrailing();
    expect(lastUpdate().params.depth).toBe(12);
    await pump();
    toolChipStore.getState().onDepth?.(null);
    await settleTrailing();
    expect(lastUpdate().params.depth).toBeNull();
  });

  // ── commit ──────────────────────────────────────────────────────────────────

  it("the ✓ commits the PREVIEWED params, byte-equal", async () => {
    build();
    await armHole();
    await pump();
    toolChipStore.getState().onValue?.(5.5);
    await settleTrailing();
    answerPreview();
    const previewed = { ...lastUpdate().params };

    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
    // The final flush before the commit carries exactly what was previewed.
    expect(lastUpdate().params).toEqual(previewed);
    expect(lastUpdate().params.diameter).toBe(5.5);
    expect(debug().holePhase).toBe("idle");
    expect(toolStore.getState().modelTool).toBe("select");
    expect(toolChipStore.getState().kind).toBe("none");
  });

  it("Enter commits the armed hole too", async () => {
    build();
    await armHole();
    await settleTrailing();
    answerPreview();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
  });

  it("the chip ✕ cancels without committing and releases the session", async () => {
    build();
    await armHole();
    toolChipStore.getState().onCancel?.();
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(clientMock.endPreview).not.toHaveBeenCalledWith("pv-1", true);
    expect(debug().holePhase).toBe("idle");
  });

  it("a tool switch releases the open session", async () => {
    build();
    await armHole();
    toolStore.getState().setTool("select");
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(debug().holePhase).toBe("idle");
  });

  // ── re-edit ─────────────────────────────────────────────────────────────────

  const STORED = {
    targetBodyId: "body1",
    face: { primary: { bodyId: "body1", elementId: "el-face-2", kind: "face" } },
    point: [3, 4, 25],
    holeType: "countersink",
    diameter: { value: 8.4 },
    depth: { value: 16 },
    cbDiameter: null,
    cbDepth: null,
    csDiameter: { value: 16.4 },
    csAngleDeg: { value: 82 },
  };

  it("a re-edit seeds EVERY field from the stored params — the seat is never re-picked", async () => {
    build(STORED);
    documentStore.getState().applyChange({
      revision: 1,
      features: [
        { id: "F1", kind: "boolean", opType: "Hole", label: "Hole", valueText: "Ø8.4", status: "ok" },
      ],
      dirty: false,
    });

    await controller.editHoleFeature("F1");
    await flush();

    expect(debug().holePhase).toBe("armed");
    expect(debug().holeEdit).toBe("F1");
    expect(debug().holeType).toBe("countersink");
    expect(debug().holeDiameter).toBe(8.4);
    expect(debug().holeDepth).toBe(16);
    expect(debug().holePoint).toEqual([3, 4, 25]);
    expect(debug().holeCsAngleDeg).toBe(82);
    expect(toolChipStore.getState().kind).toBe("hole");
    expect(toolChipStore.getState().value).toBe(8.4);
    // A re-edit previews NOTHING: PreviewOp runs against the current head, so
    // previewing an existing feature would double-apply it.
    expect(clientMock.beginPreview).not.toHaveBeenCalled();
  });

  it("a re-edit commits the WHOLE params (a patch cannot delete a conditional block)", async () => {
    build(STORED);
    documentStore.getState().applyChange({
      revision: 1,
      features: [
        { id: "F1", kind: "boolean", opType: "Hole", label: "Hole", valueText: "Ø8.4", status: "ok" },
      ],
      dirty: false,
    });
    await controller.editHoleFeature("F1");
    await flush();

    // Flip countersink → simple: the stored `cs*` MUST leave the record, which a
    // scalar-patch merge could never express.
    toolChipStore.getState().onHoleType?.("simple");
    await flush();
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    expect(clientMock.applyOperation).toHaveBeenCalledTimes(1);
    const op = clientMock.applyOperation.mock.calls[0]![0];
    expect(op.opType).toBe("Hole");
    expect(op.featureId).toBe("F1");
    expect(op.params).toMatchObject({
      holeType: "simple",
      diameter: 8.4,
      depth: 16,
      csDiameter: null,
      csAngleDeg: null,
      cbDiameter: null,
      cbDepth: null,
    });
    // A re-edit never opens a preview session, so nothing to end.
    expect(clientMock.endPreview).not.toHaveBeenCalled();
    expect(debug().holePhase).toBe("idle");
  });

  it("refuses a re-edit whose stored params are unavailable rather than arming blind", async () => {
    build({});
    documentStore.getState().applyChange({
      revision: 1,
      features: [
        { id: "F1", kind: "boolean", opType: "Hole", label: "Hole", valueText: "Ø8.4", status: "ok" },
      ],
      dirty: false,
    });
    await controller.editHoleFeature("F1");
    await flush();
    expect(debug().holePhase).toBe("idle");
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });

  // ── the seat pick is MODAL: it owns the pointer, and says what it would take ──
  //
  // Before this the tool asked for a click with no feedback at all — the picker
  // is inactive while a model tool is armed, so the ordinary hover tint is gone
  // and the user clicked blind. Both halves are pinned here: the tint (through
  // the SAME `selectionStore.setHover` the select tool writes — one hover
  // writer) and the LMB-orbit gate that stops a jittery click spinning the
  // camera instead of placing the hole.

  /** A pointer move over the viewport (no press — pure hover). */
  function moveTo(x = 40, y = 40): void {
    container.dispatchEvent(
      new MouseEvent("pointermove", { clientX: x, clientY: y, bubbles: true }),
    );
  }

  it("hover-tints the face under the pointer while picking the seat", async () => {
    build();
    toolStore.getState().setTool("hole");
    await flush();
    moveTo();

    const hover = selectionStore.getState().hover;
    expect(hover?.kind).toBe("face");
    expect(hover?.bodyId).toBe("body1");
    expect(hover?.topoKey).toBe("f:2");
    expect(hover?.id).toBe("body1#f:2");
    expect(hover?.anchor?.worldPoint).toEqual([10, 10, 25]);
  });

  it("does NOT tint a curved face — the click refuses it, so the hover must too", async () => {
    build();
    planar = false;
    toolStore.getState().setTool("hole");
    await flush();
    moveTo();

    expect(selectionStore.getState().hover).toBeNull();
  });

  it("keeps hovering while ARMED (a later click MOVES the hole) and clears on teardown", async () => {
    build();
    await armHole();
    moveTo(50, 50);
    expect(debug().holePhase).toBe("armed");
    expect(selectionStore.getState().hover?.topoKey).toBe("f:2");

    // Leaving the tool drops a hover this controller owns…
    toolStore.getState().setTool("select");
    await flush();
    expect(selectionStore.getState().hover).toBeNull();

    // …and never one it does not.
    const foreign = { kind: "face" as const, id: "other#f:1", bodyId: "other", topoKey: "f:1" };
    selectionStore.getState().setHover(foreign);
    toolStore.getState().setTool("hole");
    await flush();
    toolStore.getState().setTool("select");
    await flush();
    expect(selectionStore.getState().hover).toEqual(foreign);
  });

  it("suppresses LMB orbit for BOTH pick phases and restores it on every exit", async () => {
    build();
    toolStore.getState().setTool("hole");
    await flush();
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(true);

    // A successful seat pick keeps it suppressed: the tool stays modal, because
    // every later click MOVES the hole.
    clickAt();
    await flush();
    expect(debug().holePhase).toBe("armed");
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(true);

    // Tool switch (the cancel funnel) hands the camera back.
    toolStore.getState().setTool("select");
    await flush();
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
  });

  it("restores orbit after a COMMIT, not only after a cancel", async () => {
    build();
    await armHole();
    await pump();
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    expect(debug().holePhase).toBe("idle");
    expect(engineMock.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
    expect(selectionStore.getState().hover).toBeNull();
  });

  it("restores orbit when the viewport disposes the controller mid-pick", async () => {
    build();
    toolStore.getState().setTool("hole");
    await flush();
    engineMock.setOrbitSuppressed.mockClear();

    controller.dispose();

    // The ENGINE outlives a controller remount, so a stranded suppression would
    // leave the camera unable to orbit with no tool on screen to explain it.
    expect(engineMock.setOrbitSuppressed).toHaveBeenCalledWith(false);
    expect(selectionStore.getState().hover).toBeNull();
  });
});
