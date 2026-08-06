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
import { mockClient } from "@/ipc/mockClient";
import { resetStores } from "@/test/resetStores";

/** Seed an "open editor" state with a given dirty flag (appStore's own fields —
 *  resetStores doesn't touch appStore, it's not one of the F-WP3 chrome stores). */
function openDocument(dirty: boolean): void {
  appStore.setState({
    screen: "editor",
    document: { documentId: "doc-1", title: "Untitled" },
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
});

describe("appStore start-screen loads — failure is captured, never re-thrown", () => {
  beforeEach(() => {
    resetStores();
    appStore.setState({ recents: [], recentsStatus: "idle", recovery: null, recoveryStatus: "idle" });
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
