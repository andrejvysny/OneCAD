/*
 * VP-HARDENING PR-01 — a TOOL acquires an identity only for geometry the user is
 * actually looking at.
 *
 * `ModelToolController` is where a pick stops being a highlight and becomes an
 * operation input, so it is the last place a stale ordinal can still turn into a
 * minted ElementId. The rule it now enforces: a promotion from a selection ref
 * uses the proof captured when that ref was PICKED, and there is no proof-free
 * fallback — not even for a ref whose label the current mesh still happens to
 * resolve, which after a regen is the common case and the wrong element.
 *
 * Measure is the lane under test because it is the controller's ref-driven
 * promotion site (`measurePick`). The other five sites are hit-driven or
 * backend-driven and are covered where they live: `promote.test.ts` for the lane
 * itself, `ViewportRoot.test.tsx` for the click, `rebindPick.test.ts` for what a
 * regen does to a surviving ref.
 *
 * Its own file, with its own harness: the older `ModelToolController.*` harnesses
 * predate the controller's `onDocumentChanged` subscription, so every case in
 * them throws in the constructor (a pre-existing defect of those harnesses, not
 * this package's to repair) — this one follows
 * `ModelToolController.previewAdmission.test.ts` instead.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { ElementInfo } from "@/ipc/types";
import { STALE_PICK_HINT } from "@/ipc/promote";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { documentStore } from "@/stores/documentStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import { measureStore } from "@/stores/measureStore";
import {
  buildBodyObjects,
  disposeAll,
  setCurrentMeshPublication,
  swap,
  __resetRegistryForTests,
  type MeshEntry,
} from "@/viewport/mesh/meshRegistry";
import { attachPickProof } from "@/viewport/mesh/pickProof";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";

const PUBLICATION = {
  documentId: "doc-1",
  runtimeSession: "runtime-1",
  snapshotId: 7,
  generation: 3,
} as const;

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

function elementInfoNaming(elementId: string): ElementInfo {
  return {
    elementId,
    topoKey: "f:0",
    bodyId: "body1",
    kind: "face",
    surfaceType: 0,
    curveType: -1,
    center: [0, 0, 0],
    normal: [0, 0, 1],
    hasNormal: true,
    size: 1,
    magnitude: 1,
  };
}

function makeClientMock() {
  return {
    // The controller subscribes in its constructor; without this every
    // `new ModelToolController` throws before a single test runs.
    onDocumentChanged: vi.fn(() => () => {}),
    onPreviewResult: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => PUBLICATION as unknown as {
      documentId?: string; runtimeSession?: string; snapshotId?: number;
    }),
    promoteSelection: vi.fn(async () => [
      { topoKey: "f:0", elementId: "el_promoted", kind: "face", bodyId: "body1" },
    ]),
    elementInfo: vi.fn(async (_bodyId: string, elementId: string) =>
      elementInfoNaming(elementId || "el_unknown")),
    classifyElement: vi.fn(async () => null),
    prepareOffsetFace: vi.fn(async () => ({
      snapshotId: 7,
      targetBodyId: "body1",
      // The PICKED face plus one the tangent closure added — the second is a
      // label the user never clicked, which is why this whole lane is fenced by
      // the handshake's snapshot rather than by a pick proof.
      faces: [
        { topoKey: "f:0", picked: true },
        { topoKey: "f:1", picked: false },
      ],
      currentDims: {},
    })),
    beginPreview: vi.fn(async () => ({ sessionId: "pv-1", previewBodyId: "pb-1" })),
    updatePreview: vi.fn(),
    endPreview: vi.fn(async () => ({ revision: 1, features: [], changedBodies: [], removedBodies: [] })),
  };
}

/** The same surface `ModelToolController.previewAdmission.test.ts` stubs. */
function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    setRegionSelected: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn((_p: unknown, x: number, y: number) => ({ x, y })),
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

/** Install one body as the current publication and hand back its entry. */
function installBody(): MeshEntry {
  const entry = buildBodyObjects(
    parseMeshPayload(makeBoxMesh()), "body1", 1, undefined, undefined, PUBLICATION,
  );
  swap("body1", entry);
  setCurrentMeshPublication(PUBLICATION);
  documentStore.setState({
    documentId: "doc-1",
    runtimeSession: "runtime-1",
    geometrySource: "live",
    revision: 4,
    bodies: { body1: { id: "body1", name: "Body", visible: true } },
  });
  return entry;
}

function faceRef(elementId?: string): EntityRef {
  return {
    kind: "face",
    id: "body1#f:0",
    bodyId: "body1",
    topoKey: "f:0",
    elementId,
    anchor: { worldPoint: [1, 2, 3] },
  };
}

describe("ModelToolController — the promotion proof gate (PR-01)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  function build(): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
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
    selectionStore.getState().set([]);
    container = document.createElement("div");
    document.body.appendChild(container);
    disposeAll();
    __resetRegistryForTests();
    build();
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    disposeAll();
    __resetRegistryForTests();
    setCurrentMeshPublication(null);
  });

  it("promotes a ref whose pick-time proof is CURRENT, fenced by that proof", async () => {
    const entry = installBody();
    const ref = faceRef();
    attachPickProof(ref, { entry, kind: "face", topoKey: "f:0" });

    await controller.measurePick(ref);
    await flush();

    expect(clientMock.promoteSelection).toHaveBeenCalledWith(
      "body1",
      [{ topoKey: "f:0", kind: "face", anchor: { worldPoint: [1, 2, 3] } }],
      7,
      "runtime-1",
    );
    expect(clientMock.elementInfo).toHaveBeenCalledWith(
      "body1", "el_promoted", "f:0", expect.objectContaining({ snapshotId: 7 }),
    );
    expect(measureStore.getState().picks).toHaveLength(1);
  });

  it("refuses a ref whose proof names a REPLACED entry — no promotion, no measurement", async () => {
    // The body regenerated after the pick: the label `f:0` still resolves in the
    // new table, and would resolve to a different face. The proof is what makes
    // that detectable; without it the promotion looks perfectly ordinary.
    const picked = installBody();
    const ref = faceRef();
    attachPickProof(ref, { entry: picked, kind: "face", topoKey: "f:0" });
    swap("body1", buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body1", 2, undefined, undefined, PUBLICATION,
    ));

    await controller.measurePick(ref);
    await flush();

    expect(clientMock.promoteSelection).not.toHaveBeenCalled();
    expect(clientMock.elementInfo).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toBe(STALE_PICK_HINT);
    expect(measureStore.getState().picks).toHaveLength(0);
  });

  it("refuses a ref whose body is stale inspection-only (failed replacement)", async () => {
    const entry = installBody();
    entry.displayState = "stale-inspection-only";
    const ref = faceRef();
    attachPickProof(ref, { entry, kind: "face", topoKey: "f:0" });

    await controller.measurePick(ref);
    await flush();

    expect(clientMock.promoteSelection).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toBe(STALE_PICK_HINT);
  });

  it("refuses an unpromoted ref that carries NO proof at all", async () => {
    // Restored from persistence, or synthesised by a tool. `getEntry(bodyId)`
    // would happily hand back the installed entry here and the old code promoted
    // against it; there is no such fallback any more.
    installBody();

    await controller.measurePick(faceRef());
    await flush();

    expect(clientMock.promoteSelection).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toBe(STALE_PICK_HINT);
  });

  it("still measures a proofless ref that carries a persistent ElementId", async () => {
    // What a regen reconcile leaves behind (`rebindPick`): the proof is cleared,
    // and the promoted id — the only thing that survived — is the address. No
    // promotion is needed, so none is made.
    installBody();

    await controller.measurePick(faceRef("el_kept"));
    await flush();

    expect(clientMock.promoteSelection).not.toHaveBeenCalled();
    expect(clientMock.elementInfo).toHaveBeenCalledWith(
      "body1", "el_kept", "f:0", expect.objectContaining({ snapshotId: 7 }),
    );
    expect(measureStore.getState().picks).toHaveLength(1);
    expect(viewportStore.getState().statusHint?.message).not.toBe(STALE_PICK_HINT);
  });

  /*
   * R1(a) MAJOR 3 — an OffsetFace record's `faceIds` are authored identities.
   *
   * `promoteOffsetEvidence` short-circuits on the ElementId the matching
   * SELECTION ref already carries. That id is only as good as the publication it
   * was minted against: on a body whose replacement failed it names an element
   * of a snapshot the document has moved past, and the record would store it as
   * the face the op operates on. The fenced lane must run instead.
   */
  it("does not reuse a selection ref's ElementId for an offset closure without a current proof", async () => {
    const picked = installBody();
    const ref = faceRef("el_stale");
    attachPickProof(ref, { entry: picked, kind: "face", topoKey: "f:0" });
    picked.displayState = "stale-inspection-only";
    selectionStore.getState().set([ref]);

    toolStore.getState().setTool("offsetFace");
    await flush();
    await flush();

    // Both closure faces go through the snapshot-fenced authoritative lane —
    // the picked one included, because nothing vouches for its cached id.
    expect(clientMock.promoteSelection).toHaveBeenCalledWith("body1", [
      expect.objectContaining({ topoKey: "f:0" }),
    ], 7);
    expect(clientMock.promoteSelection).toHaveBeenCalledWith("body1", [
      expect.objectContaining({ topoKey: "f:1" }),
    ], 7);
  });

  it("reuses a selection ref's ElementId when its pick-time proof IS current", async () => {
    const picked = installBody();
    const ref = faceRef("el_fresh");
    attachPickProof(ref, { entry: picked, kind: "face", topoKey: "f:0" });
    selectionStore.getState().set([ref]);

    toolStore.getState().setTool("offsetFace");
    await flush();
    await flush();

    // Only the CHAINED face needs the wire: the picked one's id was proved.
    expect(clientMock.promoteSelection).toHaveBeenCalledTimes(1);
    expect(clientMock.promoteSelection).toHaveBeenCalledWith("body1", [
      expect.objectContaining({ topoKey: "f:1" }),
    ], 7);
  });

  it("still measures an ElementId-bearing ref whose proof names a REPLACED entry", async () => {
    // A colour edit or a visibility flip republishes the SAME topology, so
    // `meshSync` deliberately does not reconcile the selection for it — the ref
    // stays selected and must stay usable. Its pick-time proof now names a
    // retired entry, which is exactly why the proof gates PROMOTION only: the
    // persistent id needs no ordinal, so there is nothing to prove.
    const picked = installBody();
    const ref = faceRef("el_kept");
    attachPickProof(ref, { entry: picked, kind: "face", topoKey: "f:0" });
    swap("body1", buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body1", 2, undefined, undefined, PUBLICATION,
    ));

    await controller.measurePick(ref);
    await flush();

    expect(clientMock.promoteSelection).not.toHaveBeenCalled();
    expect(clientMock.elementInfo).toHaveBeenCalledWith(
      "body1", "el_kept", "f:0", expect.objectContaining({ snapshotId: 7 }),
    );
    expect(measureStore.getState().picks).toHaveLength(1);
  });
});
