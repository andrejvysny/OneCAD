/*
 * The Revolve BODY-EDGE AXIS is deliberately NOT exposed in the UI (roadmap A4).
 *
 * `AxisRef` has two variants. The kernel and core implement both; the tool authors
 * only `sketchLine` (`src/ipc/types.ts`). That decision is recorded as
 * `uiExposure: "hidden"` on the `Revolve / body-edge axis` contract row, and a
 * recorded decision is worth exactly as much as the check that keeps it true — the
 * row said `"exposed"` for a whole phase while no code path could produce one.
 *
 * So this file pins the decision from BOTH sides:
 *   (a) nothing the tool sends — preview draft, live update, or commit — carries an
 *       `edge` axis, on any lane;
 *   (b) a record that ALREADY holds a typed edge axis survives a re-edit: not
 *       authoring the variant must not mean destroying it;
 *   (c) the contract row agrees with (a).
 *
 * Exposing the variant later means changing (a) and (b) deliberately, which is the
 * point — the tests are the gate the WP1.5 evidence hangs on.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import contracts from "../../../docs/qa/modeling-operation-contracts.json";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  PreviewDraft,
  PreviewParams,
  PreviewResult,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

const REGION: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};

/** The one line the sketch offers as an axis candidate; u=50 misses the region. */
const AXIS_LINE = {
  id: "axLine",
  type: "Line" as const,
  p0: [50, -50] as [number, number],
  p1: [50, 200] as [number, number],
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
  entities: [AXIS_LINE],
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
  };
}

function makeClientMock(storedParams?: Record<string, unknown>) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((_cb: (r: PreviewResult) => void) => () => {}),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [REGION] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [REGION] })),
    getSketch: vi.fn(() => Promise.resolve(session)),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn((_id: string, _params: PreviewParams, _epoch: number) => {}),
    endPreview: vi.fn((_id: string, _commit: boolean) => Promise.resolve(ok("b1"))),
    applyOperation: vi.fn(() => Promise.resolve(ok("adhoc"))),
    applyEditCommand: vi.fn(() => Promise.resolve(ok("edit"))),
    undo: vi.fn(() => Promise.resolve(ok("undone"))),
    getOperationParams: vi.fn(
      (): Promise<Record<string, unknown>> => Promise.resolve(storedParams ?? {}),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

/** Every `axis` value the controller handed the client, from every lane. */
function axesSentTo(client: ReturnType<typeof makeClientMock>): unknown[] {
  const drafts = client.beginPreview.mock.calls.map((c) => (c[0] as PreviewDraft).params.axis);
  const updates = client.updatePreview.mock.calls.map((c) => (c[1] as PreviewParams).axis);
  return [...drafts, ...updates];
}

describe("Revolve body-edge axis is UI-hidden", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  function build(storedParams?: Record<string, unknown>): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock(storedParams);
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    selectionStore.getState().set([]);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  function click(x: number, y: number): void {
    const fire = (type: string, buttons: number, button: number): void => {
      container.dispatchEvent(
        new MouseEvent(type, { clientX: x, clientY: y, button, buttons, bubbles: true }),
      );
    };
    fire("pointerdown", 1, 0);
    fire("pointerup", 0, 0);
  }

  it("authors only the sketchLine variant — no lane can produce an edge axis", async () => {
    build();
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();
    click(50, 50); // the u=50 axis candidate
    await flush();
    await flush();

    const axes = axesSentTo(clientMock);
    expect(axes.length).toBeGreaterThan(0); // non-vacuous: something WAS sent
    for (const axis of axes) {
      expect(axis).toMatchObject({ kind: "sketchLine", sketchId: "sk", lineId: "axLine" });
    }

    // …and the commit is the previewed candidate, so it carries the same axis.
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();
    expect(clientMock.endPreview).toHaveBeenCalledWith("pv-1", true);
    for (const axis of axesSentTo(clientMock)) {
      expect((axis as { kind: string }).kind).toBe("sketchLine");
    }
    // The ad-hoc path is not a back door either.
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
  });

  it("PRESERVES a stored edge axis through a re-edit — hidden must not mean destroyed", async () => {
    documentStore.setState({
      sketches: {
        sk: { id: "sk", name: "Sketch 1", visible: true, dof: 0, status: "ok", geometryToken: "t1" },
      },
      features: [{ id: "rev-1", kind: "revolve", label: "Revolve", valueText: "90°", status: "ok" }],
    });
    build({
      profile: { sketchId: "sk", regionId: "r0" },
      angleDeg: { value: 90 },
      axis: { kind: "edge", bodyId: "body_1", edgeId: "el_axis" },
    });

    controller.editRevolveFeature("rev-1");
    await flush();
    await flush();
    await flush();
    toolChipStore.getState().onValue?.(120);
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();

    // The re-edit commits an ANGLE-ONLY deep merge. `axis` must not appear at all:
    // sending the first sketch-line candidate instead would silently move the
    // revolution axis off the body edge the record was built on.
    expect(clientMock.applyEditCommand).toHaveBeenCalledTimes(1);
    const cmd = (clientMock.applyEditCommand.mock.calls as unknown[][])[0][0] as {
      op: { params: Record<string, unknown> };
    };
    expect(cmd.op.params.angleDeg).toEqual({ value: 120 });
    expect(cmd.op.params.axis).toMatchObject({ kind: "edge", edgeId: "el_axis" });
  });

  it("the contract row records the decision this file enforces", () => {
    const rows = (contracts as unknown as { rows: Record<string, unknown>[] }).rows;
    const row = rows.find((r) => r.operation === "Revolve" && r.mode === "body-edge axis");
    expect(row).toBeDefined();
    expect(row?.supportStatus).toBe("supported"); // core + worker DO implement it
    expect(row?.uiExposure).toBe("hidden"); // the UI does not author it
  });
});
