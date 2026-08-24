/*
 * ModelToolController — the DISTANCE-ANGLE chamfer (SCHEMA §7.3) end to end in
 * the frontend: author, re-edit seed, the params patch that switches modes, and
 * the measured-range clamp it suppresses.
 *
 * Three rules are load-bearing here and each has a lane below.
 *
 *  1. EXCLUSION. `angleDeg` and `distance2` are the two chamfer modes and core
 *     refuses a record carrying both by name, so the emitting seam sends at most
 *     one and a mode switch REMOVES the other key from the re-edit base (a patch
 *     cannot delete a key — `updateScalarParamsCommand` merges shallowly).
 *  2. DEGREES. This lane is degrees on both sides; `src/ipc/angleUnits.ts` is the
 *     SKETCH radians seam and must not appear on it.
 *  3. CLAMP SUPPRESSION. `AnalyzeEdgeOpRange` (SCHEMA §7.6) is an EQUAL-LEG
 *     oracle. Once the chamfer carries a second distance or an angle it is a
 *     different solid, so the measured bound describes an op the user is not
 *     authoring and must not be enforced — including when the mode is chosen
 *     AFTER the bound arrived.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  AnalyzeEdgeOpRangeResult,
  ApplyOperationResult,
  PreviewDraft,
  PreviewResult,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { __resetLogForTests } from "@/debug/log";

const okResult = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#1" }],
  removedBodies: [],
});

const EDGES: EntityRef[] = [
  {
    kind: "edge",
    id: "body1#e:5",
    bodyId: "body1",
    topoKey: "e:5",
    elementId: "el-edge-5",
    anchor: { worldPoint: [1, 2, 3] },
  },
];

/** A bracketed answer: 0.5 mm floor, 3 mm ceiling — an EQUAL-LEG measurement. */
const RANGE: AnalyzeEdgeOpRangeResult = {
  snapshotId: 1,
  mode: "Chamfer",
  targetBodyId: "body_body1",
  edges: ["e:5"],
  searchedRange: { min: 0.1, max: 10 },
  lowerBound: 0.5,
  bestKnownMax: 3,
  provenUpperBound: 4,
  feasibleIntervals: [{ lower: 0.5, upper: 3 }],
  intervalsTruncated: false,
  limitingEntities: [],
  confidence: "bracketed",
  monotonicObserved: true,
  probesUsed: 6,
  budgetExhausted: false,
  stoppedReason: "converged",
  refusal: null,
};

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
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
    probePick: vi.fn(() => null),
  };
}

function makeClientMock(capturePreview?: (cb: (r: PreviewResult) => void) => void) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capturePreview?.(cb);
      return () => {};
    }),
    finishSketch: vi.fn(() => Promise.resolve({ regions: [] })),
    getSketchRegions: vi.fn(() => Promise.resolve({ regions: [] })),
    prepareEdgeOp: vi.fn(() =>
      Promise.resolve({
        snapshotId: 1,
        targetBodyId: "body_body1",
        edges: EDGES.map((edge) => ({
          topoKey: edge.topoKey ?? "",
          elementId: "el-edge-5",
          bodyId: "body_body1",
          kind: "edge" as const,
          picked: true,
          anchor: edge.anchor,
        })),
        refusal: null,
      }),
    ),
    analyzeEdgeOpRange: vi.fn(() => Promise.resolve(RANGE)),
    promoteSelection: vi.fn(() => Promise.resolve([])),
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
        radius: { value: 2 },
        edgeIds: ["el-edge-5"],
        edges: [{ primary: { bodyId: "body1", elementId: "el-edge-5", kind: "edge" } }],
      }),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
/** Past the armed edge-op trailing floor (160ms) so a coalesced push goes out. */
const settleTrailing = (): Promise<void> => new Promise((r) => setTimeout(r, 320));

describe("ModelToolController — distance-angle chamfer", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let previewCb: ((r: PreviewResult) => void) | null;

  function build(opts?: { debug?: boolean }): void {
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
      debug: opts?.debug,
    });
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
    __resetLogForTests({ enabled: false });
  });

  /** Arm the unified edge tool and flip it to Chamfer through the chip segment. */
  async function armChamfer(): Promise<void> {
    selectionStore.getState().set(EDGES);
    toolStore.getState().setTool("fillet");
    await flush();
    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();
    await flush();
  }

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

  // ── author ────────────────────────────────────────────────────────────────

  it("an authored angle reaches the preview params in DEGREES, and clears the second leg", async () => {
    build();
    await armChamfer();
    answerPreview();

    toolChipStore.getState().onDistance2?.(2.5);
    await settleTrailing();
    answerPreview();
    expect(lastUpdate().params.distance2).toBe(2.5);

    toolChipStore.getState().onChamferAngle?.(30);
    await settleTrailing();
    const params = lastUpdate().params;
    // Degrees, unconverted: `angleUnits` is the sketch radians seam, not this lane.
    expect(params.angleDeg).toBe(30);
    // …and the mode it replaced is GONE from the op, not merely hidden in the UI.
    expect("distance2" in params).toBe(false);
    // The chip shows both read-backs, so the cleared field empties visibly.
    expect(toolChipStore.getState().chamferAngleDeg).toBe(30);
    expect(toolChipStore.getState().distance2).toBeNull();
  });

  it("an out-of-domain angle authors NOTHING (the FSM's (0,180) domain wins)", async () => {
    build();
    await armChamfer();
    answerPreview();

    toolChipStore.getState().onChamferAngle?.(180);
    await settleTrailing();
    expect("angleDeg" in lastUpdate().params).toBe(false);
    expect(toolChipStore.getState().chamferAngleDeg).toBeNull();
  });

  it("a flip to Fillet stops EMITTING the angle without discarding it", async () => {
    build();
    await armChamfer();
    toolChipStore.getState().onChamferAngle?.(30);
    await settleTrailing();

    toolChipStore.getState().onEdgeOp?.("Fillet");
    await flush();
    await flush();
    answerPreview();
    toolChipStore.getState().onValue?.(1.5);
    await settleTrailing();
    // SCHEMA §7.3: a Fillet may not carry it, and core rejects one that does.
    expect("angleDeg" in lastUpdate().params).toBe(false);

    // Flipping back hands the user's own number straight back.
    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();
    await flush();
    answerPreview();
    toolChipStore.getState().onValue?.(1.5);
    await settleTrailing();
    expect(lastUpdate().params.angleDeg).toBe(30);
  });

  // ── re-edit ───────────────────────────────────────────────────────────────

  /** Arm the re-edit on a committed Chamfer row with `stored` params. */
  async function reeditChamfer(stored: Record<string, unknown>): Promise<void> {
    clientMock.getOperationParams.mockResolvedValueOnce(
      stored as unknown as Awaited<ReturnType<typeof clientMock.getOperationParams>>,
    );
    documentStore.setState({
      features: [
        {
          id: "feat-ch",
          kind: "fillet",
          opType: "Chamfer",
          label: "Chamfer",
          valueText: "1.0 mm ∠30.0°",
          status: "ok",
        },
      ],
    });
    await controller.editEdgeOpFeature("feat-ch", "Chamfer");
    await flush();
  }

  const STORED_ANGLE = {
    radius: { value: 1 },
    angleDeg: { value: 30 },
    edgeIds: ["el-edge-5"],
    edges: [{ primary: { bodyId: "body1", elementId: "el-edge-5", kind: "edge" } }],
  };

  it("a re-edit re-opens the STORED angle on the chip (never parsed from valueText)", async () => {
    build();
    await reeditChamfer(STORED_ANGLE);

    const chip = toolChipStore.getState();
    // `radiusFromValueText` reads only the LEADING number, which is d1 in every mode.
    expect(chip.value).toBe(1);
    expect(chip.chamferAngleDeg).toBe(30);
    expect(chip.distance2).toBeNull();
    expect(chip.edgeOp).toBe("Chamfer");
  });

  it("a re-edit commits the angle as a scalar patch, edge binding untouched", async () => {
    build();
    await reeditChamfer(STORED_ANGLE);

    toolChipStore.getState().onChamferAngle?.(45);
    toolChipStore.getState().onConfirm?.();
    await flush();

    expect(clientMock.applyEditCommand).toHaveBeenCalledTimes(1);
    expect(clientMock.applyEditCommand).toHaveBeenCalledWith({
      cmd: "updateOperationParams",
      record: "feat-ch",
      op: {
        opType: "Chamfer",
        params: {
          radius: { value: 1 },
          angleDeg: { value: 45 },
          edgeIds: ["el-edge-5"],
          edges: [{ primary: { bodyId: "body1", elementId: "el-edge-5", kind: "edge" } }],
        },
      },
    });
  });

  it("switching modes in a re-edit REMOVES the stored key it replaced", async () => {
    // A shallow merge cannot delete a key, so the base has to lose it — otherwise
    // the record would carry both modes and core refuses that by name.
    build();
    await reeditChamfer(STORED_ANGLE);

    toolChipStore.getState().onDistance2?.(2.5);
    toolChipStore.getState().onConfirm?.();
    await flush();

    const [cmd] = clientMock.applyEditCommand.mock.calls[0] as unknown as [
      { op: { params: Record<string, unknown> } },
    ];
    expect(cmd.op.params.distance2).toEqual({ value: 2.5 });
    expect("angleDeg" in cmd.op.params).toBe(false);
  });

  it("clearing the angle in a re-edit removes the key (back to equal-leg)", async () => {
    build();
    await reeditChamfer(STORED_ANGLE);

    toolChipStore.getState().onChamferAngle?.(null);
    toolChipStore.getState().onConfirm?.();
    await flush();

    const [cmd] = clientMock.applyEditCommand.mock.calls[0] as unknown as [
      { op: { params: Record<string, unknown> } },
    ];
    expect("angleDeg" in cmd.op.params).toBe(false);
    expect("distance2" in cmd.op.params).toBe(false);
  });

  it("a re-edit FLIPPED to Fillet drops both chamfer modes from the record", async () => {
    build();
    await reeditChamfer(STORED_ANGLE);

    toolChipStore.getState().onEdgeOp?.("Fillet");
    await flush();
    toolChipStore.getState().onConfirm?.();
    await flush();

    const [cmd] = clientMock.applyEditCommand.mock.calls[0] as unknown as [
      { op: { opType: string; params: Record<string, unknown> } },
    ];
    expect(cmd.op.opType).toBe("Fillet");
    expect("angleDeg" in cmd.op.params).toBe(false);
  });

  // ── measured-range clamp suppression (SCHEMA §7.6) ────────────────────────

  it("the EQUAL-LEG bound clamps a plain chamfer, and either mode LIFTS it", async () => {
    build();
    await armChamfer();
    await flush(); // let the fire-and-forget range analysis land
    expect(clientMock.analyzeEdgeOpRange).toHaveBeenCalled();
    // The measurements the ARM issued (one per deliberate type choice); every
    // assertion below is about what the chip edits add to this, which is nothing.
    const measured = clientMock.analyzeEdgeOpRange.mock.calls.length;

    // Equal-leg: the measured 3 mm ceiling is the op the analysis actually built.
    toolChipStore.getState().onValue?.(10);
    expect(toolChipStore.getState().value).toBe(3);

    // A second leg makes it a different solid — the bound no longer describes it.
    toolChipStore.getState().onDistance2?.(2.5);
    toolChipStore.getState().onValue?.(10);
    expect(toolChipStore.getState().value).toBe(10);

    // …and so does an angle, chosen AFTER the bound had already arrived.
    toolChipStore.getState().onChamferAngle?.(30);
    toolChipStore.getState().onValue?.(12);
    expect(toolChipStore.getState().value).toBe(12);

    // Clearing the mode restores the clamp: the bound was never discarded, only
    // held inapplicable while the op was not the one it measured.
    toolChipStore.getState().onChamferAngle?.(null);
    toolChipStore.getState().onValue?.(10);
    expect(toolChipStore.getState().value).toBe(3);

    // No re-measure on any of it — a chip edit must not issue OCCT builds.
    expect(clientMock.analyzeEdgeOpRange).toHaveBeenCalledTimes(measured);
  });
});
