/*
 * VP-HARDENING PR-03A/PR-03B — an exact preview is a real GPU resource.
 *
 * `ModelToolController.applyPreviewBodies` used to validate each candidate mesh
 * and build geometry from it without ever pricing the result, so a drag could
 * put arbitrarily many meshes on the GPU while admission reported the
 * document's bodies only. It now goes through `previewMesh.buildPreviewEntry`,
 * which plans, admits and builds as one step; a refusal SKIPS that candidate
 * exactly as a validation failure already did, because this runs inside a
 * timer-driven preview listener where a throw is an unhandled exception rather
 * than a caught failure (`ipc/localSolver.ts` fires results from a timer).
 *
 * Its own file, with its own harness: `ModelToolController.previewOwnership`
 * tests the same lane but its client mock predates the controller's
 * `onDocumentChanged` subscription, so every case in it throws in the
 * constructor. That is a pre-existing defect of that harness and not this
 * package's to repair.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  OperationOp,
  PreviewDraft,
  PreviewParams,
  PreviewResult,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { resetStores } from "@/test/resetStores";
import { __resetDocumentAdmissionForTests, getDocumentAdmission } from "@/viewport/mesh/meshAdmission";
import { disposeAll, __resetRegistryForTests } from "@/viewport/mesh/meshRegistry";
import { __resetPreviewMeshReportsForTests } from "@/viewport/mesh/previewMesh";
import { planMeshPreparation } from "@/viewport/mesh/meshPreparationPlan";
import { validateMeshView } from "@/viewport/mesh/validateMesh";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { __resetLogForTests, logSnapshot } from "@/debug/log";

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
const R1: SketchRegion = {
  regionId: "r1",
  outerLoop: [],
  holes: [],
  previewTriangles: {
    positions: [100, 100, 140, 100, 140, 140, 100, 140],
    indices: [0, 1, 2, 0, 2, 3],
  },
};

const ok = (bodyId: string): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId, meshKey: `${bodyId}#0` }],
  removedBodies: [],
});

const session: SketchSession = {
  sketchId: "sk",
  plane: PLANE,
  entities: [],
  constraints: [],
  dof: 0,
  status: "FullyConstrained",
};

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    setRegionSelected: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    planePixelWorld: vi.fn(() => 1),
    probePick: vi.fn(() => null),
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
  };
}

function makeClientMock(capture: (cb: (r: PreviewResult) => void) => void) {
  let seq = 0;
  const previewOps = new Map<string, OperationOp>();
  return {
    // The controller subscribes in its constructor; without this every
    // `new ModelToolController` throws before a single test runs.
    onDocumentChanged: vi.fn(() => () => {}),
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capture(cb);
      return () => {};
    }),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0, R1] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0, R1] })),
    getSketch: vi.fn(() => Promise.resolve(session)),
    beginPreview: vi.fn((d: PreviewDraft) => {
      const sessionId = `pv-${++seq}`;
      previewOps.set(sessionId, d as OperationOp);
      return Promise.resolve({ sessionId, previewBodyId: `pb-${seq}` });
    }),
    takePreviewOperation: vi.fn((id: string) => previewOps.get(id) ?? null),
    applyOperations: vi.fn((_ops: OperationOp[]) => Promise.resolve(ok("batch"))),
    updatePreview: vi.fn((_id: string, _params: PreviewParams, _epoch: number) => {}),
    endPreview: vi.fn((_id: string, _commit: boolean) => Promise.resolve(ok("b1"))),
    applyOperation: vi.fn(() => Promise.resolve(ok("adhoc"))),
    applyEditCommand: vi.fn(() => Promise.resolve(ok("edit"))),
    undo: vi.fn(() => Promise.resolve(ok("undone"))),
    getOperationParams: vi.fn((): Promise<Record<string, unknown>> => Promise.resolve({})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("exact preview admission", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let previewCb: ((r: PreviewResult) => void) | null;

  function build(): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock((cb) => {
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

  /** Arm extrude on ONE region, so exactly one preview session is live. */
  async function armOneRegion(): Promise<void> {
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
    ]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();
    expect(clientMock.beginPreview).toHaveBeenCalledTimes(1);
  }

  /** One preview result carrying `rows` as its candidate bodies. */
  function bodies(rows: Array<{ bodyId: string; mesh: ArrayBuffer }>): PreviewResult {
    const calls = clientMock.updatePreview.mock.calls;
    return {
      sessionId: "pv-1",
      epoch: calls[calls.length - 1][2] as number,
      bodyId: "preview",
      bodies: rows,
      replacedBodyIds: [],
    };
  }

  const previewWarnings = () =>
    logSnapshot().filter((e) => e.level === "warn" && e.tag === "preview");

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    selectionStore.getState().set([]);
    previewCb = null;
    container = document.createElement("div");
    document.body.appendChild(container);
    disposeAll();
    __resetRegistryForTests();
    __resetDocumentAdmissionForTests();
    __resetPreviewMeshReportsForTests();
    __resetLogForTests();
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
    disposeAll();
    __resetRegistryForTests();
    __resetDocumentAdmissionForTests();
    __resetLogForTests({ enabled: false });
  });

  it("charges every admitted candidate to the DOCUMENT budget", async () => {
    build();
    await armOneRegion();

    previewCb?.(bodies([
      { bodyId: "cand-a", mesh: makeBoxMesh() },
      { bodyId: "cand-b", mesh: makeBoxMesh() },
    ]));

    expect(engineMock.setPreviewBody).toHaveBeenCalledTimes(2);
    expect(getDocumentAdmission().snapshot().holdings).toBe(2);
    expect(previewWarnings()).toHaveLength(0);
  });

  it("skips a candidate the budget refuses, and never throws out of the listener", async () => {
    build();
    await armOneRegion();
    // Room for one of these candidates at a time, not two.
    const probe = validateMeshView(parseMeshPayload(makeBoxMesh()), "probe");
    if (!probe.ok) throw new Error("the mock box must validate");
    const unit = planMeshPreparation(probe.mesh, {}, "probe");
    if (!unit.ok) throw new Error("the mock box must be plannable");
    __resetDocumentAdmissionForTests({ estimatedGpuBytes: Math.floor(unit.plan.gpuBytes * 1.5) });

    expect(() =>
      previewCb?.(bodies([
        { bodyId: "cand-a", mesh: makeBoxMesh() },
        { bodyId: "cand-b", mesh: makeBoxMesh() },
      ])),
    ).not.toThrow();

    // The first candidate fits and is drawn; the second is skipped, and the
    // refusal is reported through the same throttled one-line-per-(body, code)
    // warn a malformed payload uses.
    expect(engineMock.setPreviewBody).toHaveBeenCalledTimes(1);
    expect(getDocumentAdmission().snapshot().holdings).toBe(1);
    expect(previewWarnings()).toHaveLength(1);
    expect(previewWarnings()[0].ctx).toMatchObject({ code: "gpu-budget" });
  });

  it("skips only the INVALID candidate and still installs the rest", async () => {
    build();
    await armOneRegion();
    const torn = makeBoxMesh();
    new DataView(torn).setUint32(0x00, 0x4d455349, true); // bad magic

    previewCb?.(bodies([
      { bodyId: "cand-a", mesh: torn },
      { bodyId: "cand-b", mesh: makeBoxMesh() },
    ]));

    expect(engineMock.setPreviewBody).toHaveBeenCalledTimes(1);
    expect(getDocumentAdmission().snapshot().holdings).toBe(1);
    expect(previewWarnings()[0].ctx).toMatchObject({ code: "bad-magic" });
  });
});
