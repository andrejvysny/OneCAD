import { beforeEach, describe, expect, it } from "vitest";
import { mockClient, resetMockDocument } from "@/ipc/mockClient";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
import { runDemoFlags } from "./devDemos";

const deps = (vpdemoCylinder: boolean) => ({
  engine: {} as never,
  client: mockClient,
  container: document.createElement("div"),
  modelToolController: {} as never,
  vpdemo: true,
  vpdemoCylinder,
  sketchdemo: false,
  toolsdemo: false,
});

describe("viewport demos", () => {
  beforeEach(() => {
    documentStore.setState(seedMockDocument());
    resetMockDocument();
  });

  it("publishes the seeded document revision for the box demo", () => {
    let seenRevision = -1;
    const off = mockClient.onDocumentChanged((change) => { seenRevision = change.revision; });
    runDemoFlags(deps(false));
    off();
    expect(seenRevision).toBe(documentStore.getState().revision);
  });

  it("publishes the post-seed revision when adding the cylinder demo body", () => {
    let seenRevision = -1;
    const off = mockClient.onDocumentChanged((change) => { seenRevision = change.revision; });
    runDemoFlags(deps(true));
    off();
    expect(seenRevision).toBe(documentStore.getState().revision);
  });
});
