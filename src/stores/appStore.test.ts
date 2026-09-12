/*
 * Unsaved-changes guard (appStore.requestClose / confirmClose). Every close/quit
 * path (TitleBar ×, ⌘W, the native window-close button, ⌘Q) funnels through
 * `requestClose`; a clean document bypasses the UnsavedChangesDialog entirely
 * (existing close behavior), a dirty one arms `pendingCloseIntent` and only
 * proceeds once `confirmClose` resolves it.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { appStore } from "./appStore";
import { documentStore } from "./documentStore";
import { mockClient, setMockRecovery } from "@/ipc/mockClient";
import { resetStores } from "@/test/resetStores";
import type { DocumentSnapshot } from "@/ipc/types";
import { selectionStore } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { repairStore } from "@/stores/repairStore";
import { viewportStore } from "@/stores/viewportStore";

/** Seed an "open editor" state with a given dirty flag (appStore's own fields —
 *  resetStores doesn't touch appStore, it's not one of the F-WP3 chrome stores). */
function openDocument(dirty: boolean): void {
  appStore.setState({
    screen: "editor",
    document: { documentId: "doc-1", runtimeSession: "runtime-1", title: "Untitled" },
    pendingCloseIntent: null,
  });
  documentStore.setState({ dirty });
}

describe("appStore unsaved-changes guard", () => {
  beforeEach(() => {
    resetStores();
    openDocument(false);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("a clean document bypasses the dialog: requestClose('close') closes immediately", async () => {
    await appStore.getState().requestClose("close");

    expect(appStore.getState().pendingCloseIntent).toBeNull();
    expect(appStore.getState().screen).toBe("start");
    expect(appStore.getState().document).toBeNull();
  });

  it("a dirty document arms the intent and does NOT close", async () => {
    openDocument(true);

    await appStore.getState().requestClose("close");

    expect(appStore.getState().pendingCloseIntent).toBe("close");
    expect(appStore.getState().screen).toBe("editor");
    expect(appStore.getState().document).not.toBeNull();
  });

  it("confirmClose('cancel') clears the intent and leaves the document open", async () => {
    openDocument(true);
    await appStore.getState().requestClose("close");

    await appStore.getState().confirmClose("cancel");

    expect(appStore.getState().pendingCloseIntent).toBeNull();
    expect(appStore.getState().screen).toBe("editor");
    expect(appStore.getState().document).not.toBeNull();
  });

  it("confirmClose('discard') closes without saving", async () => {
    const saveSpy = vi.spyOn(mockClient, "saveDocument");
    openDocument(true);
    await appStore.getState().requestClose("close");

    await appStore.getState().confirmClose("discard");

    expect(saveSpy).not.toHaveBeenCalled();
    expect(appStore.getState().pendingCloseIntent).toBeNull();
    expect(appStore.getState().screen).toBe("start");
    expect(appStore.getState().document).toBeNull();
  });

  it("confirmClose('save') saves then closes on success", async () => {
    const saveSpy = vi.spyOn(mockClient, "saveDocument");
    openDocument(true);
    await appStore.getState().requestClose("close");

    await appStore.getState().confirmClose("save");

    expect(saveSpy).toHaveBeenCalledTimes(1);
    expect(appStore.getState().pendingCloseIntent).toBeNull();
    expect(appStore.getState().screen).toBe("start");
  });

  it("confirmClose('save') keeps the dialog open on a save failure", async () => {
    vi.spyOn(mockClient, "saveDocument").mockRejectedValueOnce(new Error("disk full"));
    openDocument(true);
    await appStore.getState().requestClose("close");

    await appStore.getState().confirmClose("save");

    expect(appStore.getState().pendingCloseIntent).toBe("close");
    expect(appStore.getState().screen).toBe("editor");
    expect(appStore.getState().document).not.toBeNull();
  });

  it("confirmClose('save') keeps the dialog open when an edit races a successful write", async () => {
    vi.spyOn(mockClient, "saveDocument").mockResolvedValueOnce({
      documentId: "doc-1",
      savedRevision: 3,
      currentRevision: 4,
      clean: false,
      path: "/tmp/doc.onecad",
      title: "doc",
    });
    openDocument(true);
    await appStore.getState().requestClose("close");

    await appStore.getState().confirmClose("save");

    expect(appStore.getState().pendingCloseIntent).toBe("close");
    expect(appStore.getState().screen).toBe("editor");
    expect(documentStore.getState().dirty).toBe(true);
  });

  it("quit intent: a clean document bypasses the dialog and calls confirmExit directly", async () => {
    const confirmExitSpy = vi.spyOn(mockClient, "confirmExit");

    await appStore.getState().requestClose("quit");

    expect(confirmExitSpy).toHaveBeenCalledTimes(1);
    expect(appStore.getState().pendingCloseIntent).toBeNull();
  });

  it("quit intent: a dirty document arms the intent without calling confirmExit", async () => {
    const confirmExitSpy = vi.spyOn(mockClient, "confirmExit");
    openDocument(true);

    await appStore.getState().requestClose("quit");

    expect(confirmExitSpy).not.toHaveBeenCalled();
    expect(appStore.getState().pendingCloseIntent).toBe("quit");
  });

  it("quit intent: confirmClose('cancel') releases the backend guard via cancelExit, no exit", async () => {
    const cancelExitSpy = vi.spyOn(mockClient, "cancelExit");
    const confirmExitSpy = vi.spyOn(mockClient, "confirmExit");
    openDocument(true);
    await appStore.getState().requestClose("quit");

    await appStore.getState().confirmClose("cancel");

    expect(cancelExitSpy).toHaveBeenCalledTimes(1);
    expect(confirmExitSpy).not.toHaveBeenCalled();
    expect(appStore.getState().pendingCloseIntent).toBeNull();
    // "quit" never touches the in-app screen/document — only the OS-level exit.
    expect(appStore.getState().screen).toBe("editor");
  });

  it("quit intent: confirmClose('discard') calls confirmExit, not the in-app closeDocument", async () => {
    const confirmExitSpy = vi.spyOn(mockClient, "confirmExit");
    const closeDocumentSpy = vi.spyOn(mockClient, "closeDocument");
    openDocument(true);
    await appStore.getState().requestClose("quit");

    await appStore.getState().confirmClose("discard");

    expect(confirmExitSpy).toHaveBeenCalledTimes(1);
    expect(closeDocumentSpy).not.toHaveBeenCalled();
    expect(appStore.getState().pendingCloseIntent).toBeNull();
  });

  // ── re-entrant requestClose (the dialog blocks the pointer, not shortcuts) ──

  it("a repeated identical intent is ignored (no duplicate cancelExit, intent unchanged)", async () => {
    const cancelExitSpy = vi.spyOn(mockClient, "cancelExit");
    openDocument(true);
    await appStore.getState().requestClose("quit");

    await appStore.getState().requestClose("quit");

    expect(cancelExitSpy).not.toHaveBeenCalled();
    expect(appStore.getState().pendingCloseIntent).toBe("quit");
  });

  it("a second ⌘W during a quit prompt releases the backend guard before replacing the intent", async () => {
    const cancelExitSpy = vi.spyOn(mockClient, "cancelExit");
    const confirmExitSpy = vi.spyOn(mockClient, "confirmExit");
    openDocument(true);
    await appStore.getState().requestClose("quit");

    await appStore.getState().requestClose("close");

    // The orphan bug: without this, Rust's ExitGuard stays latched forever and
    // every later quit is silently swallowed.
    expect(cancelExitSpy).toHaveBeenCalledTimes(1);
    expect(confirmExitSpy).not.toHaveBeenCalled();
    expect(appStore.getState().pendingCloseIntent).toBe("close");
    expect(appStore.getState().screen).toBe("editor");
  });

  it("save-then-close during a quit prompt releases the guard and takes the clean fast path", async () => {
    const cancelExitSpy = vi.spyOn(mockClient, "cancelExit");
    const confirmExitSpy = vi.spyOn(mockClient, "confirmExit");
    openDocument(true);
    await appStore.getState().requestClose("quit");

    documentStore.setState({ dirty: false }); // saved out-of-band (⌘S while prompted)
    await appStore.getState().requestClose("close");

    expect(cancelExitSpy).toHaveBeenCalledTimes(1);
    expect(confirmExitSpy).not.toHaveBeenCalled();
    expect(appStore.getState().pendingCloseIntent).toBeNull();
    expect(appStore.getState().screen).toBe("start");
    expect(appStore.getState().document).toBeNull();
  });

  it("a pending in-app 'close' upgraded to 'quit' needs no cancelExit (no guard was held)", async () => {
    const cancelExitSpy = vi.spyOn(mockClient, "cancelExit");
    openDocument(true);
    await appStore.getState().requestClose("close");

    await appStore.getState().requestClose("quit");

    expect(cancelExitSpy).not.toHaveBeenCalled();
    expect(appStore.getState().pendingCloseIntent).toBe("quit");
  });

  it("confirmClose is a no-op with nothing pending", async () => {
    await appStore.getState().confirmClose("discard");

    expect(appStore.getState().pendingCloseIntent).toBeNull();
    expect(appStore.getState().screen).toBe("editor");
    expect(appStore.getState().document).not.toBeNull();
  });

  it("dirty Open prompts before its native dialog, and cancel preserves the editor", async () => {
    const openDialog = vi.spyOn(mockClient, "openFileDialog");
    openDocument(true);

    await appStore.getState().openDialogAndOpen();

    expect(appStore.getState().pendingCloseIntent).toBe("replacement");
    expect(openDialog).not.toHaveBeenCalled();

    await appStore.getState().confirmClose("cancel");

    expect(openDialog).not.toHaveBeenCalled();
    expect(appStore.getState().screen).toBe("editor");
    expect(appStore.getState().document?.documentId).toBe("doc-1");
  });

  it("discarding a dirty Open runs the chosen replacement exactly once", async () => {
    const openDialog = vi.spyOn(mockClient, "openFileDialog").mockResolvedValue("/tmp/next.onecad");
    const openDocumentSpy = vi.spyOn(mockClient, "openDocument");
    openDocument(true);

    await appStore.getState().openDialogAndOpen();
    await appStore.getState().confirmClose("discard");

    expect(openDialog).toHaveBeenCalledTimes(1);
    expect(openDocumentSpy).toHaveBeenCalledWith(
      "/tmp/next.onecad",
      undefined,
      expect.objectContaining({ beforeAdopt: expect.any(Function) }),
    );
    expect(appStore.getState().pendingCloseIntent).toBeNull();
  });

  it("ignores a second clean replacement while the first continuation is in flight", async () => {
    let resolveOpen!: (document: DocumentSnapshot) => void;
    const openDocumentSpy = vi.spyOn(mockClient, "openDocument").mockReturnValue(
      new Promise((resolve) => {
        resolveOpen = resolve;
      }),
    );
    const first = appStore.getState().openProject("/tmp/first.onecad");
    const second = appStore.getState().openProject("/tmp/second.onecad");

    await Promise.resolve();
    expect(openDocumentSpy).toHaveBeenCalledTimes(1);
    resolveOpen({ documentId: "next", runtimeSession: "runtime-next", title: "Next" });
    await Promise.all([first, second]);
  });

  it("runs a dirty replacement continuation once when discard is confirmed twice", async () => {
    let resolveOpen!: (document: DocumentSnapshot) => void;
    const openDocumentSpy = vi.spyOn(mockClient, "openDocument").mockReturnValue(
      new Promise((resolve) => {
        resolveOpen = resolve;
      }),
    );
    openDocument(true);
    await appStore.getState().openProject("/tmp/next.onecad");
    const first = appStore.getState().confirmClose("discard");
    const second = appStore.getState().confirmClose("discard");
    await Promise.resolve();
    expect(openDocumentSpy).toHaveBeenCalledTimes(1);
    resolveOpen({ documentId: "next", runtimeSession: "runtime-next", title: "Next" });
    await Promise.all([first, second]);
  });

  it("releases the replacement flight after failure so a retry can run", async () => {
    const openDocumentSpy = vi
      .spyOn(mockClient, "openDocument")
      .mockRejectedValueOnce(new Error("unreadable"))
      .mockResolvedValueOnce({ documentId: "next", runtimeSession: "runtime-next", title: "Next" });

    await expect(appStore.getState().openProject("/tmp/bad.onecad")).rejects.toThrow("unreadable");
    await expect(appStore.getState().openProject("/tmp/good.onecad")).resolves.toBeUndefined();
    expect(openDocumentSpy).toHaveBeenCalledTimes(2);
  });

  it("releases a synchronously rejected continuation for retry", async () => {
    const openDocumentSpy = vi
      .spyOn(mockClient, "openDocument")
      .mockImplementationOnce(() => {
        throw new Error("sync failure");
      })
      .mockResolvedValueOnce({ documentId: "next", runtimeSession: "runtime-next", title: "Next" });

    await expect(appStore.getState().openProject("/tmp/bad.onecad")).rejects.toThrow("sync failure");
    await expect(appStore.getState().openProject("/tmp/good.onecad")).resolves.toBeUndefined();
    expect(openDocumentSpy).toHaveBeenCalledTimes(2);
  });

  it("blocks synchronous re-entry while a replacement continuation starts", async () => {
    let reentry!: Promise<void>;
    const openDocumentSpy = vi.spyOn(mockClient, "openDocument").mockImplementation(async (path) => {
      if (path === "/tmp/first.onecad") {
        reentry = appStore.getState().openProject("/tmp/reentrant.onecad");
      }
      return { documentId: "next", runtimeSession: "runtime-next", title: "Next" };
    });

    await appStore.getState().openProject("/tmp/first.onecad");
    await reentry;
    expect(openDocumentSpy).toHaveBeenCalledTimes(1);
  });

  it("keeps document-scoped UI intact when open reports recovery pending", async () => {
    const selected = [{ kind: "body" as const, id: "body-a" }];
    selectionStore.setState({ selected, hover: selected[0] });
    toolStore.setState({ mode: "sketch" });
    sketchStore.setState({ constructionMode: true });
    repairStore.setState({ panelOpen: true, expandedRefId: "ref-a" });
    viewportStore.setState({ isolatedBodyIds: ["body-a"] });
    vi.spyOn(mockClient, "openDocument").mockRejectedValueOnce({ kind: "recoveryPending" });

    await appStore.getState().openProject("/tmp/recovery.onecad");

    expect(selectionStore.getState().selected).toEqual(selected);
    expect(toolStore.getState().mode).toBe("sketch");
    expect(sketchStore.getState().constructionMode).toBe(true);
    expect(repairStore.getState().panelOpen).toBe(true);
    expect(viewportStore.getState().isolatedBodyIds).toEqual(["body-a"]);
  });

  it("keeps document-scoped UI intact when close fails", async () => {
    const selected = [{ kind: "body" as const, id: "body-a" }];
    selectionStore.setState({ selected, hover: selected[0] });
    toolStore.setState({ mode: "sketch" });
    sketchStore.setState({ constructionMode: true });
    repairStore.setState({ panelOpen: true, expandedRefId: "ref-a" });
    viewportStore.setState({ isolatedBodyIds: ["body-a"] });
    vi.spyOn(mockClient, "closeDocument").mockRejectedValueOnce(new Error("close failed"));

    await expect(appStore.getState().closeProject()).rejects.toThrow("close failed");

    expect(selectionStore.getState().selected).toEqual(selected);
    expect(toolStore.getState().mode).toBe("sketch");
    expect(sketchStore.getState().constructionMode).toBe(true);
    expect(repairStore.getState().panelOpen).toBe(true);
    expect(viewportStore.getState().isolatedBodyIds).toEqual(["body-a"]);
  });

  it("releases a native quit guard when it conflicts with an active replacement", async () => {
    let resolveOpen!: (document: DocumentSnapshot) => void;
    const openDocumentSpy = vi.spyOn(mockClient, "openDocument").mockReturnValue(
      new Promise((resolve) => {
        resolveOpen = resolve;
      }),
    );
    const cancelExitSpy = vi.spyOn(mockClient, "cancelExit");
    const closeProjectSpy = vi.spyOn(mockClient, "closeDocument");
    openDocument(true);
    await appStore.getState().openProject("/tmp/next.onecad");
    const replacement = appStore.getState().confirmClose("discard");
    await Promise.resolve();

    await appStore.getState().requestClose("quit");
    expect(cancelExitSpy).toHaveBeenCalledTimes(1);
    expect(closeProjectSpy).not.toHaveBeenCalled();
    expect(openDocumentSpy).toHaveBeenCalledTimes(1);

    resolveOpen({ documentId: "next", runtimeSession: "runtime-next", title: "Next" });
    await replacement;
  });
});

describe("appStore start-screen loads — failure is captured, never re-thrown", () => {
  beforeEach(() => {
    resetStores();
    appStore.setState({ recents: [], recentsStatus: "idle", recovery: [], recoveryStatus: "idle" });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("loadRecents records `error` and resolves (a rejection would land in every void call site)", async () => {
    vi.spyOn(mockClient, "listRecents").mockRejectedValue(new Error("backend down"));

    await expect(appStore.getState().loadRecents()).resolves.toBeUndefined();

    expect(appStore.getState().recentsStatus).toBe("error");
    expect(appStore.getState().recents).toEqual([]);
  });

  it("checkRecovery records `error` and resolves", async () => {
    vi.spyOn(mockClient, "checkRecovery").mockRejectedValue(new Error("backend down"));

    await expect(appStore.getState().checkRecovery()).resolves.toBeUndefined();

    expect(appStore.getState().recoveryStatus).toBe("error");
  });

  it("a retry after a failure recovers to `ready`", async () => {
    const spy = vi
      .spyOn(mockClient, "listRecents")
      .mockRejectedValueOnce(new Error("backend down"));
    await appStore.getState().loadRecents();
    expect(appStore.getState().recentsStatus).toBe("error");

    spy.mockResolvedValueOnce([]);
    await appStore.getState().loadRecents();

    expect(appStore.getState().recentsStatus).toBe("ready");
  });
});

describe("appStore crash recovery — the offer must survive a failure", () => {
  const OFFER = {
    documentId: "44444444-4444-4444-4444-444444444444",
    title: "Bracket",
    originalPath: "/docs/Bracket.onecad",
    autosavePath: "/x/autosave/foo.onecad",
    modifiedMs: 1_700_000_000_000,
  };

  beforeEach(() => {
    resetStores();
    setMockRecovery(null);
    appStore.setState({
      screen: "start",
      document: null,
      recovery: [],
      recoveryStatus: "idle",
      recoveryPendingId: null,
      recoveryConflict: null,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    setMockRecovery(null);
  });

  /**
   * Both actions are wired straight to `onClick` handlers typed `() => void`, so a
   * rejection would surface as an unhandled promise rejection with no UI feedback
   * whatsoever — on the one screen standing between the user and their unsaved work.
   */
  it("a failed restore is captured, not thrown, and leaves the offer on screen", async () => {
    setMockRecovery(OFFER);
    await appStore.getState().checkRecovery();
    vi.spyOn(mockClient, "recoverDocument").mockRejectedValueOnce(new Error("corrupt container"));

    await expect(appStore.getState().recoverDocument(OFFER.documentId)).resolves.toBeUndefined();

    expect(appStore.getState().recoveryStatus).toBe("error");
    expect(appStore.getState().recovery).toHaveLength(1);
    expect(appStore.getState().recoveryPendingId).toBeNull();
    expect(appStore.getState().screen).toBe("start");
  });

  it("a failed discard is captured too", async () => {
    setMockRecovery(OFFER);
    await appStore.getState().checkRecovery();
    vi.spyOn(mockClient, "recoverDocument").mockRejectedValueOnce(new Error("read-only fs"));

    await expect(appStore.getState().discardRecovery(OFFER.documentId)).resolves.toBeUndefined();

    expect(appStore.getState().recoveryStatus).toBe("error");
    expect(appStore.getState().recovery).toHaveLength(1);
  });

  it("a second decision while one is in flight is ignored", async () => {
    setMockRecovery(OFFER);
    await appStore.getState().checkRecovery();
    const spy = vi.spyOn(mockClient, "recoverDocument");

    const first = appStore.getState().recoverDocument(OFFER.documentId);
    await appStore.getState().recoverDocument(OFFER.documentId); // the double click
    await first;

    expect(spy).toHaveBeenCalledTimes(1);
  });
});

describe("appStore — opening a file an autosave shadows", () => {
  const OFFER = {
    documentId: "55555555-5555-5555-5555-555555555555",
    title: "Bracket",
    originalPath: "/docs/Bracket.onecad",
    autosavePath: "/x/autosave/foo.onecad",
    modifiedMs: 1_700_000_000_000,
  };

  beforeEach(async () => {
    resetStores();
    appStore.setState({
      screen: "start",
      document: null,
      recovery: [],
      recoveryStatus: "idle",
      recoveryPendingId: null,
      recoveryConflict: null,
    });
    setMockRecovery(OFFER);
    await appStore.getState().checkRecovery();
  });

  afterEach(() => setMockRecovery(null));

  /**
   * The destructive click. The recovery banner sits ABOVE a fully clickable recents
   * list, so opening the stale file was always one click away — and it destroyed the
   * autosave silently, because the reopened document carries the same id and its
   * first autosave overwrites the crash container.
   */
  it("prompts instead of opening, and opens nothing", async () => {
    await appStore.getState().openProject("/docs/Bracket.onecad");

    expect(appStore.getState().recoveryConflict).toEqual({
      path: "/docs/Bracket.onecad",
      offer: OFFER,
    });
    expect(appStore.getState().screen).toBe("start");
    expect(appStore.getState().recovery).toHaveLength(1);
  });

  it("Cancel leaves both the file and the autosave alone", async () => {
    await appStore.getState().openProject("/docs/Bracket.onecad");
    await appStore.getState().resolveRecoveryConflict("cancel");

    expect(appStore.getState().recoveryConflict).toBeNull();
    expect(appStore.getState().screen).toBe("start");
    expect(appStore.getState().recovery).toHaveLength(1);
  });

  it("Restore recovers the newer work rather than the file", async () => {
    await appStore.getState().openProject("/docs/Bracket.onecad");
    await appStore.getState().resolveRecoveryConflict("restore");

    expect(appStore.getState().screen).toBe("editor");
    expect(appStore.getState().recovery).toEqual([]);
  });

  it("Open Saved Version discards the autosave — deliberately, and only then", async () => {
    await appStore.getState().openProject("/docs/Bracket.onecad");
    await appStore.getState().resolveRecoveryConflict("openSaved");

    expect(appStore.getState().screen).toBe("editor");
    expect(appStore.getState().recovery).toEqual([]);
    expect(await mockClient.checkRecovery()).toEqual([]);
  });

  it("the OS file dialog gets the same guard — it reaches the same files", async () => {
    vi.spyOn(mockClient, "openFileDialog").mockResolvedValueOnce("/docs/Bracket.onecad");

    await appStore.getState().openDialogAndOpen();

    expect(appStore.getState().recoveryConflict?.path).toBe("/docs/Bracket.onecad");
    expect(appStore.getState().screen).toBe("start");
  });

  it("a path no offer names opens normally", async () => {
    await appStore.getState().openProject("/docs/Other.onecad");

    expect(appStore.getState().recoveryConflict).toBeNull();
    expect(appStore.getState().screen).toBe("editor");
  });
});
