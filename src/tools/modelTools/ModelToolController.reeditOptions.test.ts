/*
 * Extrude RE-EDIT exposes its full option set (jsdom — 2026-09-14 UX review T6,
 * decision D12).
 *
 * Before this, a re-edit opened value + draft only: `Blind / Through all / To
 * next / To face` and `New Body / Add / Cut` were hidden, so an Extrude could
 * never be changed from Add to Cut after the fact. The three flags are now true
 * on a re-edit and SEEDED from the stored record (the `storedScalar` pattern the
 * draft angle already uses), a legacy record without `extrudeMode`/`booleanMode`
 * opens on Blind / NewBody, `canUseBodyEnds` is recomputed against the CURRENT
 * document, and crossing the NewBody boundary warns that downstream features may
 * need repair (D12).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  PreviewDraft,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import type { WireEditCommand } from "@/ipc/tauriCommandMap";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};

const okResult = (): ApplyOperationResult => ({
  revision: 4,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#1" }],
  removedBodies: [],
});

function makeSession(): SketchSession {
  return { sketchId: "sk", plane: PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
}

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
    setRegionSelected: vi.fn(),
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
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => null),
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

function makeClientMock(stored: Record<string, unknown>) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn(() => () => {}),
    onDocumentChanged: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => null),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketch: vi.fn(() => Promise.resolve(makeSession())),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn((_cmd: WireEditCommand) => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() => Promise.resolve({ ...stored })),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

const extrudeFeature = (id = "feat-ex") => ({
  id,
  kind: "extrude" as const,
  opType: "Extrude",
  label: "Extrude",
  valueText: "20 mm",
  status: "ok" as const,
});

/** The params a committed Cut with a picked target stored. */
const STORED_CUT = {
  profile: { sketchId: "sk", regionId: "r0" },
  distance: { value: 15 },
  draftAngleDeg: { value: 0 },
  extrudeMode: "Blind",
  booleanMode: "Cut",
  targetBodyId: "body1",
};

/** A pre-W1 record: neither `extrudeMode` nor `booleanMode` was ever written. */
const STORED_LEGACY = {
  profile: { sketchId: "sk", regionId: "r0" },
  distance: { value: 12 },
};

describe("Extrude re-edit exposes end condition / symmetric / boolean (T6, D12)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  function build(stored: Record<string, unknown>): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock(stored);
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "Body 1", visible: true } },
      features: [extrudeFeature()],
    });
    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v1",
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  it("publishes all three option flags and seeds them from the stored record", async () => {
    build(STORED_CUT);
    controller.editExtrudeFeature("feat-ex");
    await flush();
    await flush();

    const chip = toolChipStore.getState();
    expect(chip.kind).toBe("extrudeDepth");
    expect(chip.showEndConditions).toBe(true);
    expect(chip.showSymmetric).toBe(true);
    expect(chip.showBooleanSegments).toBe(true);
    // Seeded from the RECORD, not from a fresh material probe.
    expect(chip.booleanMode).toBe("Cut");
    expect(chip.endCondition).toBe("Blind");
    expect(chip.symmetric).toBe(false);
    // Recomputed for the CURRENT document: body1 is visible, so the body-reaching
    // end conditions are offered.
    expect(chip.canUseBodyEnds).toBe(true);
    expect(chip.canBoolean).toBe(true);
  });

  it("reads a stored Symmetric record back as the symmetric toggle, not as an end condition", async () => {
    build({ ...STORED_CUT, extrudeMode: "Symmetric" });
    controller.editExtrudeFeature("feat-ex");
    await flush();
    await flush();

    expect(toolChipStore.getState().symmetric).toBe(true);
    expect(toolChipStore.getState().endCondition).toBe("Blind");
  });

  it("seeds a stored ThroughAll back onto the end-condition segment", async () => {
    build({ ...STORED_CUT, extrudeMode: "ThroughAll" });
    controller.editExtrudeFeature("feat-ex");
    await flush();
    await flush();

    expect(toolChipStore.getState().endCondition).toBe("ThroughAll");
    expect(toolChipStore.getState().symmetric).toBe(false);
  });

  it("opens a LEGACY record (no extrudeMode / booleanMode) on Blind + NewBody", async () => {
    build(STORED_LEGACY);
    controller.editExtrudeFeature("feat-ex");
    await flush();
    await flush();

    const chip = toolChipStore.getState();
    expect(chip.endCondition).toBe("Blind");
    expect(chip.booleanMode).toBe("NewBody");
    expect(chip.symmetric).toBe(false);
    expect(chip.showBooleanSegments).toBe(true);
  });

  it("hides the body-reaching end conditions when the current document has no visible body", async () => {
    build(STORED_LEGACY);
    documentStore.setState({ bodies: {} });
    controller.editExtrudeFeature("feat-ex");
    await flush();
    await flush();

    expect(toolChipStore.getState().canUseBodyEnds).toBe(false);
    expect(toolChipStore.getState().canBoolean).toBe(false);
  });

  it("warns that downstream features may need repair when the re-edit crosses the NewBody boundary (D12)", async () => {
    build(STORED_CUT);
    controller.editExtrudeFeature("feat-ex");
    await flush();
    await flush();

    toolChipStore.getState().onBooleanMode?.("NewBody");
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Downstream features may need repair",
    );
    expect(viewportStore.getState().statusHint?.severity).toBe("warn");
  });

  it("does not warn while the re-edit stays on the stored side of the NewBody boundary", async () => {
    build(STORED_CUT);
    controller.editExtrudeFeature("feat-ex");
    await flush();
    await flush();
    viewportStore.getState().setStatusHint(null);

    toolChipStore.getState().onBooleanMode?.("Add"); // Cut → Add, both non-NewBody
    expect(viewportStore.getState().statusHint?.message).not.toBe(
      "Downstream features may need repair",
    );
  });

  it("writes the changed end condition and boolean mode back on Apply", async () => {
    build(STORED_CUT);
    controller.editExtrudeFeature("feat-ex");
    await flush();
    await flush();

    toolChipStore.getState().onBooleanMode?.("Add");
    await toolChipStore.getState().onEndCondition?.("ThroughAll");
    await flush();
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    expect(clientMock.applyEditCommand).toHaveBeenCalled();
    const cmd = clientMock.applyEditCommand.mock.calls[0][0] as unknown as {
      cmd: string;
      record: string;
      op: { opType: string; params: Record<string, unknown> };
    };
    expect(cmd.cmd).toBe("updateOperationParams");
    expect(cmd.record).toBe("feat-ex");
    expect(cmd.op.params.extrudeMode).toBe("ThroughAll");
    expect(cmd.op.params.booleanMode).toBe("Add");
    // Unrelated stored keys still round-trip verbatim.
    expect(cmd.op.params.profile).toEqual({ sketchId: "sk", regionId: "r0" });
  });

  it("writes Symmetric back as the extrudeMode when the toggle is on", async () => {
    build(STORED_CUT);
    controller.editExtrudeFeature("feat-ex");
    await flush();
    await flush();

    toolChipStore.getState().onSymmetric?.(true);
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    const cmd = clientMock.applyEditCommand.mock.calls[0][0] as unknown as {
      op: { params: Record<string, unknown> };
    };
    expect(cmd.op.params.extrudeMode).toBe("Symmetric");
  });
});
