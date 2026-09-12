import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CadClient } from "@/ipc/client";
import type { DocumentChange, ElementInfo } from "@/ipc/types";
import { measureStore } from "@/stores/measureStore";
import { documentStore } from "@/stores/documentStore";
import { resetStores } from "@/test/resetStores";
import { toolStore } from "@/stores/toolStore";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { ModelToolController } from "./ModelToolController";
import * as registry from "@/viewport/mesh/meshRegistry";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { makeBoxMesh } from "@/ipc/mockMeshes";

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
      engine: {} as ViewportEngine,
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
      engine: {} as ViewportEngine,
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
});
