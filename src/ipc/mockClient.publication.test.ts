import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  emitMockDocumentChanged,
  mockClient,
  resetMockDocument,
} from "./mockClient";
import { documentStore, seedMockDocument } from "@/stores/documentStore";

describe("mock geometry publication metadata", () => {
  beforeEach(() => {
    documentStore.setState(seedMockDocument());
    resetMockDocument();
  });
  afterEach(() => vi.useRealTimers());

  it("fills positive matching snapshot and mesh generation, retained for attach-after-event", () => {
    const seen: Parameters<Parameters<typeof mockClient.onDocumentChanged>[0]>[0][] = [];
    const off = mockClient.onDocumentChanged((change) => seen.push(change));
    emitMockDocumentChanged({
      revision: 6,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:coarse:99" }],
      removedBodies: [],
    });
    off();

    expect(seen).toHaveLength(1);
    const publication = seen[0];
    expect(publication.snapshotId).toBeGreaterThan(0);
    expect(publication.changedBodies[0]?.meshKey).toBe(`body1:coarse:${publication.snapshotId}`);
    expect(publication.documentId).toBe("mock-document");
    expect(publication.runtimeSession).toBe("mock-runtime");
    expect(mockClient.getCurrentMeshPublication()).toEqual(publication);
  });

  it("keeps same-session publications monotonic and separates a new runtime", () => {
    const first: number[] = [];
    const off = mockClient.onDocumentChanged((change) => first.push(change.snapshotId ?? 0));
    emitMockDocumentChanged({ revision: 6, changedBodies: [{ bodyId: "body1", meshKey: "body1:coarse:1" }], removedBodies: [] });
    emitMockDocumentChanged({ revision: 7, changedBodies: [{ bodyId: "body1", meshKey: "body1:coarse:2" }], removedBodies: [] });
    off();
    expect(first[1]).toBeGreaterThan(first[0] ?? 0);

    documentStore.setState({ documentId: "mock-document-2", runtimeSession: "mock-runtime-2" });
    emitMockDocumentChanged({ revision: 1, changedBodies: [{ bodyId: "body1", meshKey: "body1:coarse:1" }], removedBodies: [] });
    const publication = mockClient.getCurrentMeshPublication();
    expect(publication?.documentId).toBe("mock-document-2");
    expect(publication?.runtimeSession).toBe("mock-runtime-2");
    expect(publication?.snapshotId).toBeGreaterThan(first[1] ?? 0);
  });

  it("preserves explicitly supplied malformed or stale metadata", () => {
    const explicit = {
      revision: 2,
      documentId: "old-document",
      runtimeSession: "old-runtime",
      snapshotId: 0,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:coarse:0" }],
      removedBodies: [],
    };
    emitMockDocumentChanged(explicit);
    expect(mockClient.getCurrentMeshPublication()).toEqual(explicit);
  });

  it("refuses a scoped read after the authoritative snapshot advances", async () => {
    emitMockDocumentChanged({
      revision: 2,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:coarse:1" }],
      removedBodies: [],
    });
    const publication = mockClient.getCurrentMeshPublication()!;
    const fence = {
      documentId: publication.documentId!,
      runtimeSession: publication.runtimeSession!,
      snapshotId: publication.snapshotId!,
    };
    emitMockDocumentChanged({
      revision: 3,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:coarse:2" }],
      removedBodies: [],
    });

    await expect(mockClient.massProperties("body1", fence)).rejects.toThrow("stale geometry");
  });

  it("refuses element and classification responses overtaken during provider latency", async () => {
    vi.useFakeTimers();
    emitMockDocumentChanged({
      revision: 2,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:coarse:1" }],
      removedBodies: [],
    });
    const publication = mockClient.getCurrentMeshPublication()!;
    const fence = {
      documentId: publication.documentId!,
      runtimeSession: publication.runtimeSession!,
      snapshotId: publication.snapshotId!,
    };
    const element = mockClient.elementInfo("body1", "", "f:0", fence);
    const classification = mockClient.classifyElement("body1", "", "f:0", fence);
    emitMockDocumentChanged({
      revision: 3,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:coarse:2" }],
      removedBodies: [],
    });
    const elementRefusal = expect(element).rejects.toThrow("stale geometry");
    const classificationRefusal = expect(classification).rejects.toThrow("stale geometry");
    await vi.runAllTimersAsync();
    await Promise.all([elementRefusal, classificationRefusal]);
  });
});
