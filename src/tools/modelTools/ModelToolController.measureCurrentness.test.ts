import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CadClient } from "@/ipc/client";
import type { ClassifyResult, DocumentChange, ElementInfo, PromotedElement } from "@/ipc/types";
import { measureStore } from "@/stores/measureStore";
import { documentStore } from "@/stores/documentStore";
import { resetStores } from "@/test/resetStores";
import { toolStore } from "@/stores/toolStore";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { ModelToolController } from "./ModelToolController";
import * as registry from "@/viewport/mesh/meshRegistry";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { makeBoxMesh } from "@/ipc/mockMeshes";

// `onToolChange` (armed by `toolStore.setState(...modelTool:"measure")` below)
// runs the full cancel sweep for every OTHER tool FSM first (cancelPreview,
// cancelFillet, cancelBoolean, ...), and `dispose()` does the same — each
// touching several engine methods unconditionally even though this suite only
// exercises Measure. Stub every property as a cached no-op rather than
// enumerating the sweep's engine surface.
function makeEngineMock() {
  const cache = new Map<PropertyKey, ReturnType<typeof vi.fn>>();
  return new Proxy(
    {},
    {
      get(_target, prop) {
        if (!cache.has(prop)) cache.set(prop, vi.fn());
        return cache.get(prop);
      },
    },
  );
}

describe("ModelToolController measurement currentness", () => {
  let controller: ModelToolController;
  let publish: ((change: DocumentChange) => void) | undefined;
  let container: HTMLDivElement;

  beforeEach(() => {
    resetStores();
    registry.disposeAll();
    container = document.createElement("div");
    document.body.appendChild(container);
    const client = {
      onPreviewResult: () => () => {},
      onDocumentChanged: (cb: (change: DocumentChange) => void) => {
        publish = cb;
        return () => {};
      },
      getCurrentMeshPublication: () => null,
    } as unknown as CadClient;
    controller = new ModelToolController({
      engine: makeEngineMock() as unknown as ViewportEngine,
      client,
      container,
      onBodyLoaded: () => () => {},
      debug: false,
    });
    toolStore.setState({ mode: "model", modelTool: "measure" });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    vi.restoreAllMocks();
  });

  it("clears picks on a same-session publication even without a projection write", () => {
    measureStore.setState({
      picks: [{ bodyId: "body1" }] as never,
      summary: null,
    });
    publish?.({
      documentId: "doc",
      runtimeSession: "runtime",
      snapshotId: 2,
      revision: 1,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:2" }],
      removedBodies: [],
    });
    expect(measureStore.getState().picks).toEqual([]);
  });

  it("clears picks when the runtime changes at the same advisory revision", () => {
    documentStore.setState({ documentId: "doc", runtimeSession: "runtime-a", revision: 4 });
    measureStore.setState({ picks: [{ bodyId: "body1" }] as never, summary: null });

    documentStore.setState({ documentId: "doc", runtimeSession: "runtime-b", revision: 4 });

    expect(measureStore.getState().picks).toEqual([]);
  });

  it("drops a delayed element read when the installed mesh is replaced", async () => {
    controller.dispose();
    const provenance = {
      documentId: "doc",
      runtimeSession: "runtime",
      snapshotId: 7,
      generation: 3,
    } as const;
    const entry = registry.buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body_1", 1, undefined, undefined, provenance,
    );
    registry.swap("body_1", entry);
    registry.setCurrentMeshPublication(provenance);
    documentStore.setState({
      documentId: "doc",
      runtimeSession: "runtime",
      geometrySource: "live",
      bodies: { body_1: { id: "body_1", name: "Body", visible: true } },
    });
    let resolve!: (value: ElementInfo) => void;
    const elementInfo = vi.fn(() => new Promise<ElementInfo>((done) => { resolve = done; }));
    const client = {
      onPreviewResult: () => () => {},
      onDocumentChanged: () => () => {},
      getCurrentMeshPublication: () => ({ ...provenance, revision: 1, changedBodies: [], removedBodies: [] }),
      elementInfo,
    } as unknown as CadClient;
    controller = new ModelToolController({
      engine: makeEngineMock() as unknown as ViewportEngine,
      client,
      container,
      onBodyLoaded: () => () => {},
      debug: false,
    });
    toolStore.setState({ mode: "model", modelTool: "measure" });
    const pending = controller.measurePick({
      kind: "face", bodyId: "body_1", id: "face", elementId: "el_1", topoKey: "f:0",
    });
    expect(elementInfo).toHaveBeenCalledOnce();
    registry.swap(
      "body_1",
      registry.buildBodyObjects(
        parseMeshPayload(makeBoxMesh()), "body_1", 2, undefined, undefined, provenance,
      ),
    );
    resolve({
      bodyId: "body_1", elementId: "el_1", topoKey: "f:0", kind: "face",
      surfaceType: 0, curveType: -1, center: [0, 0, 0], normal: [0, 0, 1],
      hasNormal: true, size: 1, magnitude: 1,
    });
    await pending;
    expect(measureStore.getState().picks).toEqual([]);
  });

  it("drops a delayed promotion when a new publication lands mid-flight", async () => {
    controller.dispose();
    const provenance = {
      documentId: "doc",
      runtimeSession: "runtime",
      snapshotId: 7,
      generation: 3,
    } as const;
    const entry = registry.buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body_1", 1, undefined, undefined, provenance,
    );
    registry.swap("body_1", entry);
    registry.setCurrentMeshPublication(provenance);
    documentStore.setState({
      documentId: "doc",
      runtimeSession: "runtime",
      geometrySource: "live",
      bodies: { body_1: { id: "body_1", name: "Body", visible: true } },
    });
    let snapshotId = 7;
    let onDocChanged: ((change: DocumentChange) => void) | undefined;
    let resolvePromote!: (value: PromotedElement[]) => void;
    const promoteSelection = vi.fn(
      () => new Promise<PromotedElement[]>((done) => { resolvePromote = done; }),
    );
    const elementInfo = vi.fn<CadClient["elementInfo"]>();
    const classifyElement = vi.fn<CadClient["classifyElement"]>();
    const client = {
      onPreviewResult: () => () => {},
      onDocumentChanged: (cb: (change: DocumentChange) => void) => {
        onDocChanged = cb;
        return () => {};
      },
      // `fenceStillCurrent()` re-reads this after the promotion await
      // (ModelToolController.ts ~1402-1408) — reflect the moved snapshot.
      getCurrentMeshPublication: () => ({
        documentId: "doc",
        runtimeSession: "runtime",
        snapshotId,
      }),
      promoteSelection,
      elementInfo,
      classifyElement,
    } as unknown as CadClient;
    controller = new ModelToolController({
      engine: makeEngineMock() as unknown as ViewportEngine,
      client,
      container,
      onBodyLoaded: () => () => {},
      debug: false,
    });
    toolStore.setState({ mode: "model", modelTool: "measure" });

    // No `elementId` on the ref, so `measurePick` awaits `promoteOne` before
    // `elementInfo`/`classifyElement` (ModelToolController.ts ~1414-1426).
    const pending = controller.measurePick({
      kind: "face", bodyId: "body_1", id: "face", topoKey: "f:0",
    });
    expect(promoteSelection).toHaveBeenCalledOnce();

    // The world moves underneath the in-flight promotion: a new snapshot
    // publishes on the same document/runtime session. Both the mesh
    // publication the guard re-reads AND the same `onDocumentChanged` ->
    // `cancelMeasure` path the "clears picks on a same-session publication"
    // test above exercises.
    snapshotId = 8;
    onDocChanged?.({
      documentId: "doc",
      runtimeSession: "runtime",
      snapshotId: 8,
      revision: 2,
      changedBodies: [{ bodyId: "body_1", meshKey: "body_1:fine:8" }],
      removedBodies: [],
    });

    resolvePromote([{ bodyId: "body_1", elementId: "el_1", topoKey: "f:0", kind: "face" }]);
    await pending;

    // The `gen !== this.measureGen || !fenceStillCurrent()` guard right after
    // the promotion await (ModelToolController.ts ~1424) must drop this pick.
    expect(measureStore.getState().picks).toEqual([]);
    expect(elementInfo).not.toHaveBeenCalled();
    expect(classifyElement).not.toHaveBeenCalled();
  });

  it("drops a delayed classification when the installed mesh is replaced", async () => {
    controller.dispose();
    const provenance = {
      documentId: "doc",
      runtimeSession: "runtime",
      snapshotId: 7,
      generation: 3,
    } as const;
    const entry = registry.buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body_1", 1, undefined, undefined, provenance,
    );
    registry.swap("body_1", entry);
    registry.setCurrentMeshPublication(provenance);
    documentStore.setState({
      documentId: "doc",
      runtimeSession: "runtime",
      geometrySource: "live",
      bodies: { body_1: { id: "body_1", name: "Body", visible: true } },
    });
    let resolveClassify!: (value: ClassifyResult | null) => void;
    const classifyElement = vi.fn(
      () => new Promise<ClassifyResult | null>((done) => { resolveClassify = done; }),
    );
    // Resolves immediately — the elementId is already minted (`el_1`), so
    // `measurePick` skips `promoteOne` entirely and goes straight to
    // `elementInfo`, which lands before the mesh is replaced below.
    const elementInfo = vi.fn(
      async (): Promise<ElementInfo> => ({
        bodyId: "body_1", elementId: "el_1", topoKey: "f:0", kind: "face",
        surfaceType: 0, curveType: -1, center: [0, 0, 0], normal: [0, 0, 1],
        hasNormal: true, size: 1, magnitude: 1,
      }),
    );
    const client = {
      onPreviewResult: () => () => {},
      onDocumentChanged: () => () => {},
      getCurrentMeshPublication: () => ({ ...provenance, revision: 1, changedBodies: [], removedBodies: [] }),
      elementInfo,
      classifyElement,
    } as unknown as CadClient;
    controller = new ModelToolController({
      engine: makeEngineMock() as unknown as ViewportEngine,
      client,
      container,
      onBodyLoaded: () => () => {},
      debug: false,
    });
    toolStore.setState({ mode: "model", modelTool: "measure" });

    const pending = controller.measurePick({
      kind: "face", bodyId: "body_1", id: "face", elementId: "el_1", topoKey: "f:0",
    });
    // Let `elementInfo` resolve while the installed mesh is still the one the
    // pick was proven against.
    await vi.waitFor(() => expect(classifyElement).toHaveBeenCalledOnce());

    // Installed geometry is replaced (new generation) while the classify
    // read is still in flight — the same swap the "drops a delayed element
    // read" test above uses to invalidate `installedProofIsCurrent`.
    registry.swap(
      "body_1",
      registry.buildBodyObjects(
        parseMeshPayload(makeBoxMesh()), "body_1", 2, undefined, undefined, provenance,
      ),
    );

    resolveClassify({
      kind: "face", surfaceType: "cylinder", curveType: "",
      frame: { origin: [0, 0, 0], normal: null, axis: [0, 0, 1], radius: 4 },
    });
    await pending;

    // The `gen !== this.measureGen || !fenceStillCurrent()` guard right
    // after the classify await (ModelToolController.ts ~1450) must drop
    // this pick — no Ø/R value from the stale classification may reach the
    // store.
    expect(measureStore.getState().picks).toEqual([]);
  });
});
