/*
 * C7 / D9 — arming Fillet / Chamfer from a FACE selection.
 *
 * Finding T8 left Fillet with no reachable entry point: the edges could not be
 * clicked and the tool was disabled with a face selected (A-265) and with a body
 * selected (A-289). Picking is fixed in `Picker.ts`; this is the second half —
 * a selected face arms the tool by expanding to that face's boundary edges and
 * then running the ORDINARY prepared-closure path, so nothing downstream can
 * tell a face arm from four clicked edges.
 *
 * A BODY selection deliberately stays disabled: "fillet this body" has no
 * defensible edge set.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CadClient } from "@/ipc/client";
import type { PrepareEdgeOpRequest, PrepareEdgeOpResult, PreviewDraft } from "@/ipc/types";
import { encodeMesh1, makeBoxMesh } from "@/ipc/mockMeshes";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import {
  buildBodyObjects,
  disposeAll,
  swap,
  __resetRegistryForTests,
} from "@/viewport/mesh/meshRegistry";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { ModelToolController } from "./ModelToolController";

/** f:0 is the mock box's +X face; e:1/e:5/e:9/e:10 are the four edges it bounds. */
const FACE: EntityRef = {
  kind: "face",
  id: "body1#f:0",
  bodyId: "body1",
  topoKey: "f:0",
  anchor: { worldPoint: [40, 0, 0] },
};
const FACE_EDGES = ["e:1", "e:5", "e:9", "e:10"];

const BODY: EntityRef = { kind: "body", id: "body1" };

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

/** Echoes the picks back as the prepared closure — no tangent growth, so the
 *  assertions below describe the EXPANSION and nothing else. */
function clientMock() {
  let seq = 0;
  return {
    onPreviewResult: vi.fn(() => () => {}),
    onDocumentChanged: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => null),
    prepareEdgeOp: vi.fn((req: PrepareEdgeOpRequest): Promise<PrepareEdgeOpResult> =>
      Promise.resolve({
        snapshotId: 7,
        targetBodyId: "body_body1",
        edges: req.pickedEdges.map((pick) => ({
          topoKey: pick.topoKey ?? "",
          elementId: `el_${(pick.topoKey ?? "").replace(":", "_")}`,
          bodyId: "body_body1",
          kind: "edge" as const,
          picked: true,
        })),
        refusal: null,
      }),
    ),
    analyzeEdgeOpRange: vi.fn(() => Promise.reject(new Error("not measured"))),
    beginPreview: vi.fn((draft: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}`, draft }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(null)),
  };
}

const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

describe("ModelToolController — Fillet / Chamfer armed from a face (C7)", () => {
  let controller: ModelToolController;
  let container: HTMLDivElement;
  let client: ReturnType<typeof clientMock>;
  /* Every hint the arm published, in order: `openEdgeOpPreview` replaces the arm
   * hint with "Computing preview…" a microtask later, so the final value is not
   * the sentence under test. */
  let hints: string[];
  let unsubscribe: () => void;

  beforeEach(() => {
    resetStores();
    hints = [];
    unsubscribe = viewportStore.subscribe((state) => {
      const message = state.statusHint?.message;
      if (message && hints[hints.length - 1] !== message) hints.push(message);
    });
    swap("body1", buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1));
    container = document.createElement("div");
    document.body.appendChild(container);
    client = clientMock();
    controller = new ModelToolController({
      engine: engineMock() as unknown as ViewportEngine,
      client: client as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  });

  afterEach(() => {
    unsubscribe();
    controller.dispose();
    container.remove();
    disposeAll();
    __resetRegistryForTests();
  });

  async function arm(selection: EntityRef[]): Promise<void> {
    selectionStore.getState().set(selection);
    toolStore.getState().setTool("fillet");
    await flush();
  }

  it("prepares the closure over the selected face's four boundary edges", async () => {
    await arm([FACE]);
    expect(client.prepareEdgeOp).toHaveBeenCalledTimes(1);
    const sent = client.prepareEdgeOp.mock.calls[0][0];
    expect(sent.pickedEdges.map((e) => e.topoKey)).toEqual(FACE_EDGES);
    expect(sent.pickedEdges.map((e) => e.bodyId)).toEqual(FACE_EDGES.map(() => "body1"));
    // The ordinary path, unchanged: the tangent closure is still requested and
    // the preview still opens over what the worker prepared.
    expect(sent.chainTangentEdges).toBe(true);
    const draft = client.beginPreview.mock.calls[0][0] as PreviewDraft;
    expect(draft.params.edgeIds).toEqual(["el_e_1", "el_e_5", "el_e_9", "el_e_10"]);
  });

  it("arms, and says which face the edges came from", async () => {
    await arm([FACE]);
    expect(toolStore.getState().phase).toBe("armed");
    expect(hints).toContain(
      "Fillet 4 edges of Face 0 — drag or type radius · Enter or ✓ to apply",
    );
  });

  it("stays disabled for a BODY selection, and prepares nothing", async () => {
    await arm([BODY]);
    expect(client.prepareEdgeOp).not.toHaveBeenCalled();
    expect(client.beginPreview).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toBe("Select edges or a face");
  });

  it("names both entry points when nothing at all is selected", async () => {
    await arm([]);
    expect(client.prepareEdgeOp).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Select edges or a face, then Fillet",
    );
  });

  it("refuses with the edge-op copy when the face bounds no nameable edge", async () => {
    // A single triangle with no edge section at all — the shape of a seam-only
    // or non-manifold face, where the honest answer is "nothing to fillet"
    // rather than a guessed edge set.
    const seamless = parseMeshPayload(
      encodeMesh1({
        positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
        faces: [{ triangles: [[0, 1, 2]], id: "f:0" }],
      }),
    );
    swap("body2", buildBodyObjects(seamless, "body2", 1));
    await arm([{ ...FACE, id: "body2#f:0", bodyId: "body2" }]);
    expect(client.prepareEdgeOp).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Fillet unavailable: Face 0 has no edges to fillet",
    );
  });

  it("still prepares a plain EDGE pick from the picks themselves", async () => {
    await arm([
      { kind: "edge", id: "body1#e:5", bodyId: "body1", topoKey: "e:5", anchor: { worldPoint: [40, 0, 15] } },
    ]);
    const sent = client.prepareEdgeOp.mock.calls[0][0];
    expect(sent.pickedEdges.map((e) => e.topoKey)).toEqual(["e:5"]);
    // No " of Face …": a plain edge pick has no face provenance to report.
    expect(hints).toContain("Fillet 1 edge — drag or type radius · Enter or ✓ to apply");
  });
});
