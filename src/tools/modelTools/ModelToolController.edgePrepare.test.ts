import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  PrepareEdgeOpResult,
  PreviewDraft,
  PreviewResult,
} from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";

const PICKS: EntityRef[] = [
  {
    kind: "edge",
    id: "body1#e:1",
    bodyId: "body1",
    topoKey: "e:1",
    anchor: { worldPoint: [1, 0, 0] },
  },
  {
    kind: "edge",
    id: "body1#e:2",
    bodyId: "body1",
    topoKey: "e:2",
    anchor: { worldPoint: [2, 0, 0] },
  },
];

const okResult = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#2" }],
  removedBodies: [],
});

function engineMock(): Record<string, unknown> {
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
    screenRay: vi.fn(() => null),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    showRevolveAxisCandidates: vi.fn(),
    setRevolveAxisHover: vi.fn(),
    showRevolvePreview: vi.fn(),
    setRevolveAngle: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    clearPreviewBody: vi.fn(),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    probePick: vi.fn(() => null),
  };
}

function clientMock(capture: (cb: (result: PreviewResult) => void) => void) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((cb: (result: PreviewResult) => void) => {
      capture(cb);
      return () => {};
    }),
    finishSketch: vi.fn(() => Promise.resolve({ regions: [] })),
    getSketchRegions: vi.fn(() => Promise.resolve({ regions: [] })),
    prepareEdgeOp: vi.fn((): Promise<PrepareEdgeOpResult> =>
      Promise.resolve({
        snapshotId: 7,
        targetBodyId: "body_body1",
        edges: [
          { topoKey: "e:1", elementId: "el_1", bodyId: "body_body1", kind: "edge", picked: true, anchor: { worldPoint: [1, 0, 0] as [number, number, number] } },
          { topoKey: "e:2", elementId: "el_2", bodyId: "body_body1", kind: "edge", picked: true, anchor: { worldPoint: [2, 0, 0] as [number, number, number] } },
          { topoKey: "e:3", elementId: "el_3", bodyId: "body_body1", kind: "edge", picked: false, anchor: { worldPoint: [3, 0, 0] as [number, number, number] } },
        ],
        refusal: null,
      }),
    ),
    promoteSelection: vi.fn((_body: string, picks: { topoKey: string }[]) =>
      Promise.resolve(
        picks.map((pick) => ({
          topoKey: pick.topoKey,
          elementId: `el_${pick.topoKey.slice(2)}`,
          kind: "edge",
          bodyId: "body1",
        })),
      ),
    ),
    beginPreview: vi.fn((draft: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}`, draft }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    undo: vi.fn(() => Promise.resolve(okResult())),
  };
}

const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

describe("ModelToolController prepared edge closure", () => {
  let controller: ModelToolController;
  let container: HTMLDivElement;
  let client: ReturnType<typeof clientMock>;
  let preview: ((result: PreviewResult) => void) | null;

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(20);
    container = document.createElement("div");
    document.body.appendChild(container);
    preview = null;
    client = clientMock((cb) => {
      preview = cb;
    });
    controller = new ModelToolController({
      engine: engineMock() as unknown as ViewportEngine,
      client: client as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  async function arm(): Promise<void> {
    selectionStore.getState().set(PICKS);
    toolStore.getState().setTool("fillet");
    await flush();
  }

  it("adopts the atomically promoted closure and keeps it through Fillet-to-Chamfer swap", async () => {
    await arm();
    expect(client.prepareEdgeOp).toHaveBeenCalledTimes(1);
    expect(client.promoteSelection).not.toHaveBeenCalled();
    const fillet = client.beginPreview.mock.calls[0][0] as PreviewDraft;
    expect(fillet.params.edgeIds).toEqual(["el_1", "el_2", "el_3"]);
    expect(fillet.params.tangentClosureVersion).toBe(1);

    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();
    const chamfer = client.beginPreview.mock.calls[1][0] as PreviewDraft;
    expect(chamfer.params.edgeIds).toEqual(fillet.params.edgeIds);
    expect(chamfer.params.tangentClosureVersion).toBe(1);
    expect(client.prepareEdgeOp).toHaveBeenCalledTimes(1);
  });

  it("refuses before promotion or preview when preparation declines", async () => {
    client.prepareEdgeOp.mockResolvedValueOnce({
      snapshotId: 7,
      targetBodyId: "",
      edges: [],
      refusal: { code: "chainMismatch", message: "exact picks cannot be honored", edges: ["e:1"] },
    });
    await arm();
    expect(client.promoteSelection).not.toHaveBeenCalled();
    expect(client.beginPreview).not.toHaveBeenCalled();
    expect(toolChipStore.getState().kind).toBe("none");
    expect(viewportStore.getState().statusHint?.message).toContain("exact picks cannot be honored");
  });

  it("fails the whole arm when prepared identity evidence is incomplete", async () => {
    client.prepareEdgeOp.mockResolvedValueOnce({
      snapshotId: 7,
      targetBodyId: "body_body1",
      edges: [{ topoKey: "e:1", elementId: "", bodyId: "body_body1", kind: "edge", picked: true }],
      refusal: null,
    });
    await arm();
    expect(client.beginPreview).not.toHaveBeenCalled();
    expect(toolChipStore.getState().kind).toBe("none");
    expect(viewportStore.getState().statusHint?.message).toContain("was not promoted");
  });

  it("blocks commit when the final exact preview never confirms", async () => {
    await arm();
    toolChipStore.getState().onConfirm?.();
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(client.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(client.endPreview).not.toHaveBeenCalledWith("pv-1", true);
    expect(viewportStore.getState().statusHint?.message).toContain("did not confirm");
    expect(preview).not.toBeNull();
  });

  it("commits only after the final exact epoch confirms", async () => {
    await arm();
    toolChipStore.getState().onConfirm?.();
    const calls = client.updatePreview.mock.calls;
    const update = calls[calls.length - 1];
    preview?.({
      sessionId: update?.[0] as string,
      epoch: update?.[2] as number,
      bodyId: "preview",
      bodies: [],
      replacedBodyIds: [],
    });
    await flush();
    expect(client.endPreview).toHaveBeenCalledWith("pv-1", true);
  });

  it("refuses commit after the prepared document revision changes", async () => {
    await arm();
    documentStore.getState().applyChange({ revision: 9 });
    await flush();
    expect(client.endPreview).toHaveBeenCalledWith("pv-1", false);
    expect(toolStore.getState().modelTool).toBe("select");
  });
});
