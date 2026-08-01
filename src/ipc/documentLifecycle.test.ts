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
import { repairStore } from "@/stores/repairStore";
import { sketchStore } from "@/stores/sketchStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { setMockRecovery } from "./mockClient";
import { resetStores } from "@/test/resetStores";

/**
 * Put the UI into a state that only makes sense for the OUTGOING document. These
 * dirtying calls must go straight at the stores (never through
 * `resetDocumentScopedUi` itself) so the assertions below prove PRODUCTION code
 * resets them — the global vitest `resetStores()` in `beforeEach` runs BEFORE this
 * dirtying, so it cannot mask a missing reset in the actual flow under test.
 */
function dirtyDocumentScopedUi(): void {
  toolStore.getState().setMode("sketch", "sketch2");
  viewportStore.getState().setPendingExtrude("sketch2");
  selectionStore.getState().set([{ kind: "body", id: "body1" }]);
  selectionStore.getState().setHover({ kind: "body", id: "body1" });
  repairStore.getState().applyEvent({
    revision: 3,
    items: [{ opId: "f3", refId: "f3.input1", reason: "ambiguous", candidateCount: 2 }],
  });
  repairStore.getState().openPanel();
  sketchStore.getState().pushUndoSnapshot({ entities: [], constraints: [] });
  sketchStore.getState().setConflicting(["c1"]);
  toolChipStore.getState().showFillet(2, [0, 0, 0], () => {});
  // Isolation names bodies of the OUTGOING document (W3) — a carried-over set
  // would hide every body of the incoming one, since no id can match.
  viewportStore.setState({ isolatedBodyIds: ["body1"] });
}

function expectDocumentScopedUiClean(): void {
  expect(toolStore.getState().mode).toBe("model");
  expect(viewportStore.getState().pendingExtrudeSketch).toBeNull();
  expect(selectionStore.getState().selected).toEqual([]);
  expect(selectionStore.getState().hover).toBeNull();
  expect(repairStore.getState().items).toEqual([]);
  expect(repairStore.getState().panelOpen).toBe(false);
  expect(sketchStore.getState().undoStack).toEqual([]);
  expect(sketchStore.getState().conflictingIds).toEqual([]);
  expect(toolChipStore.getState().kind).toBe("none");
  expect(viewportStore.getState().isolatedBodyIds).toBeNull();
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
