/*
 * documentLifecycle — the document-scoped UI invalidation and its wiring into the
 * new / open / recover flows. The ORDERING matters: the reset must run before the
 * document swap so mounted controllers (ModelToolController) cancel their preview
 * sessions while the outgoing document's refs still resolve, which is why each
 * flow is observed at its synchronous prefix (screen still pre-swap).
 */
import { describe, it, expect, beforeEach } from "vitest";
import { resetDocumentScopedUi } from "./documentLifecycle";
import { appStore } from "@/stores/appStore";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { setMockRecovery } from "./mockClient";
import { resetStores } from "@/test/resetStores";

/** Put the UI into a state that only makes sense for the OUTGOING document. */
function dirtyDocumentScopedUi(): void {
  toolStore.getState().setMode("sketch", "sketch2");
  viewportStore.getState().setPendingExtrude("sketch2");
  selectionStore.getState().set([{ kind: "body", id: "body1" }]);
  selectionStore.getState().setHover({ kind: "body", id: "body1" });
}

function expectDocumentScopedUiClean(): void {
  expect(toolStore.getState().mode).toBe("model");
  expect(viewportStore.getState().pendingExtrudeSketch).toBeNull();
  expect(selectionStore.getState().selected).toEqual([]);
  expect(selectionStore.getState().hover).toBeNull();
}

describe("resetDocumentScopedUi", () => {
  beforeEach(() => {
    resetStores();
    appStore.setState({ screen: "start", document: null });
  });

  it("clears the tool mode, the pending extrude handoff, and the selection", () => {
    dirtyDocumentScopedUi();
    resetDocumentScopedUi();
    expectDocumentScopedUiClean();
  });
});

describe("document lifecycle wiring", () => {
  // The reset only fires when a document is ALREADY open — first boot must not
  // clobber boot-seeded state (the mock lane's prototype Sketch 2 selection).
  const openedDocument = { docId: "doc-prev" } as unknown as import("./types").DocumentSnapshot;

  beforeEach(() => {
    resetStores();
    appStore.setState({ screen: "editor", document: openedDocument, recovery: null });
  });

  it("resets BEFORE the new-document swap when replacing an open document", async () => {
    dirtyDocumentScopedUi();
    const pending = appStore.getState().newProject();

    // Synchronous prefix: the reset already ran, the swap has not.
    expectDocumentScopedUiClean();
    expect(appStore.getState().document).toBe(openedDocument);

    await pending;
    expect(appStore.getState().document).not.toBe(openedDocument);
  });

  it("resets BEFORE the open-document swap when replacing an open document", async () => {
    dirtyDocumentScopedUi();
    const pending = appStore.getState().openProject("/tmp/thing.ocad");

    expectDocumentScopedUiClean();
    expect(appStore.getState().document).toBe(openedDocument);

    await pending;
    expect(appStore.getState().document).not.toBe(openedDocument);
  });

  it("resets BEFORE the crash-recovery swap when replacing an open document", async () => {
    setMockRecovery({
      autosavePath: "/tmp/autosave.ocad",
      originalPath: "/tmp/thing.ocad",
      modifiedMs: Date.now(),
    });
    await appStore.getState().checkRecovery();
    dirtyDocumentScopedUi();
    const pending = appStore.getState().recoverDocument();

    expectDocumentScopedUiClean();

    await pending;
    expect(appStore.getState().screen).toBe("editor");
  });

  it("first boot (no document open) keeps boot-seeded UI state", async () => {
    appStore.setState({ screen: "start", document: null });
    selectionStore.getState().set([{ kind: "sketch", id: "sketch2" }]);

    await appStore.getState().newProject();

    expect(appStore.getState().screen).toBe("editor");
    expect(selectionStore.getState().selected).toEqual([{ kind: "sketch", id: "sketch2" }]);
  });
});
