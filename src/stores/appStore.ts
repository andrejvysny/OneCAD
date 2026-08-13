/*
 * App-level store: which screen is showing + the recent-projects projection.
 *
 * Pattern (per plan): zustand v5 *vanilla* store + a thin typed hook. Actions
 * call the CadClient and flip `screen` to 'editor' once a document snapshot
 * comes back. The editor itself is a later WP (placeholder for now).
 */
import { createStore, useStore } from "zustand";
import { createClient } from "@/ipc/client";
import { resetDocumentScopedUi } from "@/ipc/documentLifecycle";
import { documentStore, emptyDocument } from "@/stores/documentStore";
import { saveDocument } from "@/features/shell/fileActions";
import { logError } from "@/debug/log";
import type { DocumentSnapshot, RecentProject, RecoveryInfo } from "@/ipc/types";

const client = createClient();

type Screen = "start" | "editor";
type RecentsStatus = "idle" | "loading" | "ready" | "error";
type RecoveryStatus = "idle" | "loading" | "ready" | "error";

/**
 * Which close path is awaiting confirmation:
 *  - "close" — an in-app "Close Project" (TitleBar × / ⌘W): stays in the app,
 *    returns to the start screen.
 *  - "quit"  — an app-level exit (native window-close button / ⌘Q): Rust already
 *    prevented the actual close/exit server-side and is waiting on `confirmExit`/
 *    `cancelExit` (see `CadClient.onCloseRequested`).
 * `null` = no prompt pending.
 */
export type PendingCloseIntent = "close" | "quit" | "replacement" | null;

export interface AppState {
  screen: Screen;
  recents: RecentProject[];
  recentsStatus: RecentsStatus;
  /** The document opened when transitioning to the editor. */
  document: DocumentSnapshot | null;
  /** A crashed session's autosave offer (null once checked-and-empty or resolved). */
  recovery: RecoveryInfo | null;
  recoveryStatus: RecoveryStatus;
  /** Set when a close/quit lands against a DIRTY document — drives the
   *  UnsavedChangesDialog. See `requestClose` / `confirmClose`. */
  pendingCloseIntent: PendingCloseIntent;
  /**
   * Why the last start-screen import failed (null = none pending).
   *
   * The editor's status-bar hint is the wrong surface for this lane: a failed
   * import leaves the user ON the start screen, where the StatusBar is not
   * mounted, so the reason would have been invisible until some later document
   * was opened. `StartScreen` renders it inline instead. Cleared on the next
   * import attempt.
   */
  importError: string | null;

  loadRecents(): Promise<void>;
  /**
   * Rename a recent project's `.onecad` file (card menu). Returns `false` on
   * failure (empty name, name collision, fs error) — caught + logged here, not
   * thrown, so the card's inline-rename input can revert without a rejection
   * crossing a `void`-called action (same rule as `loadRecents`). Refreshes
   * `recents` on success.
   */
  renameRecent(path: string, newName: string): Promise<boolean>;
  /**
   * Move a recent project's `.onecad` file to the OS trash (card menu). Returns
   * `false` on failure, same reasoning as `renameRecent`. Refreshes `recents`
   * on success.
   */
  deleteRecent(path: string): Promise<boolean>;
  /** Reveal a recent project's file in the OS file manager (card menu). Best-
   *  effort — a failure is logged, not surfaced (nothing to revert in the UI). */
  revealRecent(path: string): Promise<void>;
  newProject(): Promise<void>;
  /** Starts a new untitled document from a template (spec §8). */
  newFromTemplate(id: string): Promise<void>;
  openProject(path: string): Promise<void>;
  openDialogAndOpen(): Promise<void>;
  importStep(): Promise<void>;
  importProject(): Promise<void>;
  importFromDialog(): Promise<void>;
  closeProject(): Promise<void>;
  checkRecovery(): Promise<void>;
  recoverDocument(): Promise<void>;
  discardRecovery(): Promise<void>;
  /**
   * Entry point for EVERY close/quit path (TitleBar ×, ⌘W, the native
   * window-close button, ⌘Q). A clean document proceeds immediately (bypassing
   * the dialog); a dirty one arms `pendingCloseIntent` so the
   * UnsavedChangesDialog can prompt.
   *
   * Re-entrant by design (the dialog blocks the pointer, not the shortcut lane).
   * An identical pending intent is ignored; a DIFFERENT one first releases a
   * pending "quit" via `cancelExit`, because that intent — and only that one —
   * holds Rust's `ExitGuard`, and dropping it unreleased makes the app unclosable.
   */
  requestClose(intent: "close" | "quit"): Promise<void>;
  /**
   * Guard a document-replacing continuation with the same Save / Don't Save /
   * Cancel dialog as close. The continuation is intentionally not run until the
   * user has either discarded or received a CLEAN authoritative save result.
   */
  requestReplacement(continuation: () => Promise<void>): Promise<void>;
  /**
   * Resolve a pending prompt from the UnsavedChangesDialog.
   *  - "cancel"  — clears the intent; for "quit" also releases the backend's
   *    re-entrancy guard (without exiting) so a later attempt prompts again.
   *  - "discard" — proceeds without saving.
   *  - "save"    — saves first (via the shared `fileActions.saveDocument` path);
   *    a save failure/cancel leaves the prompt open (the failure's error hint is
   *    already surfaced by `saveDocument` itself) so the user can retry or fall
   *    back to Cancel.
   */
  confirmClose(action: "save" | "discard" | "cancel"): Promise<void>;
}

export const appStore = createStore<AppState>()((set, get) => {
  let pendingReplacement: (() => Promise<void>) | null = null;
  const enter = (document: DocumentSnapshot) =>
    set({ screen: "editor", document });

  // Only an ALREADY-OPEN document has UI identities to invalidate. The first
  // entry must not clobber boot-seeded state (the mock lane opens with the
  // prototype's Sketch 2 selection).
  const resetIfReplacing = () => {
    if (get().document) resetDocumentScopedUi();
  };

  function importKind(path: string): "project" | "step" {
    const lower = path.toLowerCase();
    if (lower.endsWith(".step") || lower.endsWith(".stp")) return "step";
    return "project";
  }

  /** The actual close/quit action once nothing (or nothing further) blocks it —
   *  a clean-document bypass, or a confirmed discard/save. */
  const proceed = (intent: "close" | "quit"): Promise<void> =>
    intent === "quit" ? client.confirmExit() : get().closeProject();

  return {
    screen: "start",
    recents: [],
    recentsStatus: "idle",
    document: null,
    recovery: null,
    recoveryStatus: "idle",
    pendingCloseIntent: null,
    importError: null,

    /**
     * A failure lands in `recentsStatus: "error"` and is NOT re-thrown — same
     * rule as `importStep`. Three actions below call this as `void loadRecents()`
     * after a document swap, so a rejection here would surface as an unhandled
     * rejection at call sites that deliberately do not await it. The status is
     * the report; `StartScreen` renders the retry affordance off it.
     */
    async loadRecents() {
      set({ recentsStatus: "loading" });
      try {
        const recents = await client.listRecents();
        set({ recents, recentsStatus: "ready" });
      } catch (e) {
        logError("app", "listRecents FAILED", { error: e });
        set({ recentsStatus: "error" });
      }
    },

    async renameRecent(path, newName) {
      try {
        await client.renameRecentProject(path, newName);
        void get().loadRecents();
        return true;
      } catch (e) {
        logError("app", "renameRecent FAILED", { error: e });
        return false;
      }
    },

    async deleteRecent(path) {
      try {
        await client.deleteRecentProject(path);
        void get().loadRecents();
        return true;
      } catch (e) {
        logError("app", "deleteRecent FAILED", { error: e });
        return false;
      }
    },

    async revealRecent(path) {
      try {
        await client.revealInFileManager(path);
      } catch (e) {
        logError("app", "revealRecent FAILED", { error: e });
      }
    },

    async newProject() {
      await get().requestReplacement(async () => {
        resetIfReplacing();
        const document = await client.newDocument();
        enter(document);
      });
    },

    async newFromTemplate(id) {
      await get().requestReplacement(async () => {
        resetIfReplacing();
        // Deliberately NOT recorded in recents: nothing has been saved yet, and
        // an entry pointing into the library would reopen the template itself
        // rather than the user's copy.
        const document = await client.newFromTemplate(id);
        enter(document);
      });
    },

    async openProject(path) {
      await get().requestReplacement(async () => {
        resetIfReplacing();
        const document = await client.openDocument(path);
        enter(document);
        // The backend recorded this open in the recents store; refresh so the
        // start-screen list reflects it (newest first) next time it renders.
        void get().loadRecents();
      });
    },

    async openDialogAndOpen() {
      await get().requestReplacement(async () => {
        const path = await client.openFileDialog();
        if (!path) return; // cancelled — nothing swapped, so nothing to invalidate
        resetIfReplacing();
        const document = await client.openDocument(path);
        enter(document);
        void get().loadRecents();
      });
    },

    /**
     * START-SCREEN import: open a NEW document FROM a STEP file (the in-editor
     * "Import STEP…" is a different lane — `fileActions.insertStep` appends to the
     * open document instead of replacing it).
     *
     * A failure must not reject into the caller's `void importStep()`: it is a
     * routine outcome (an unreadable/invalid STEP file), so it is captured as
     * `importError` for the start screen to show and the user stays put.
     *
     * The dialog is `stepFileDialog`, NOT `openFileDialog`: the latter filters
     * `.onecad`, which made a STEP file unpickable from this button.
     */
    async importStep() {
      await get().requestReplacement(async () => {
        set({ importError: null });
        const path = await client.stepFileDialog();
        if (!path) return; // cancelled — silent, nothing to report
        try {
          resetIfReplacing();
          const document = await client.importStep(path);
          enter(document);
        } catch (e) {
          set({ importError: e instanceof Error ? e.message : String(e) });
        }
      });
    },

    async importProject() {
      await get().requestReplacement(async () => {
        const snap = await client.importProject();
        if (!snap) return; // cancelled — silent
        resetIfReplacing();
        enter(snap);
      });
    },

    async importFromDialog() {
      await get().requestReplacement(async () => {
        set({ importError: null });
        const path = await client.importFileDialog();
        if (!path) return; // cancelled — silent, nothing to report
        try {
          if (importKind(path) === "step") {
            resetIfReplacing();
            const document = await client.importStep(path);
            enter(document);
          } else {
            resetIfReplacing();
            const document = await client.openDocument(path);
            enter(document);
            void get().loadRecents();
          }
        } catch (e) {
          set({ importError: e instanceof Error ? e.message : String(e) });
        }
      });
    },

    async closeProject() {
      if (!get().document) return; // nothing open
      resetDocumentScopedUi();
      documentStore.getState().applySnapshot(emptyDocument());
      await client.closeDocument();
      set({ screen: "start", document: null });
      void get().loadRecents();
    },

    /** Captured, not re-thrown — see `loadRecents`. */
    async checkRecovery() {
      set({ recoveryStatus: "loading" });
      try {
        const recovery = await client.checkRecovery();
        set({ recovery, recoveryStatus: "ready" });
      } catch (e) {
        logError("app", "checkRecovery FAILED", { error: e });
        set({ recoveryStatus: "error" });
      }
    },

    async recoverDocument() {
      await get().requestReplacement(async () => {
        resetIfReplacing();
        const document = await client.recoverDocument(true);
        set({ recovery: null, recoveryStatus: "ready" });
        if (document) {
          enter(document);
        }
        // The recovered open counts as a recent; refresh the start-screen list.
        void get().loadRecents();
      });
    },

    async discardRecovery() {
      await client.recoverDocument(false);
      set({ recovery: null, recoveryStatus: "ready" });
    },

    async requestClose(intent) {
      const pending = get().pendingCloseIntent;
      // Same prompt already up: ignore. (The dialog blocks the pointer but NOT the
      // shortcut lane, so ⌘W/⌘Q can re-enter; Rust's ExitGuard likewise swallows a
      // repeat "quit" without re-emitting.)
      if (pending === intent) return;
      // A pending "quit" means Rust PREVENTED the native close/exit and is holding
      // `ExitGuard` until `confirmExit`/`cancelExit`. Replacing that intent without
      // releasing the guard orphans it — the app then quietly refuses every later
      // quit. Release it before the intent is replaced (or resolved by the
      // clean-document fast path below, which never reaches `confirmExit`).
      if (pending === "quit") await client.cancelExit();
      pendingReplacement = null;
      if (!documentStore.getState().dirty) {
        set({ pendingCloseIntent: null });
        await proceed(intent);
        return;
      }
      set({ pendingCloseIntent: intent });
    },

    async requestReplacement(continuation) {
      const pending = get().pendingCloseIntent;
      if (pending === "replacement") return;
      if (pending === "quit") await client.cancelExit();
      pendingReplacement = null;
      if (!get().document || !documentStore.getState().dirty) {
        set({ pendingCloseIntent: null });
        await continuation();
        return;
      }
      pendingReplacement = continuation;
      set({ pendingCloseIntent: "replacement" });
    },

    async confirmClose(action) {
      const intent = get().pendingCloseIntent;
      if (!intent) return; // nothing pending
      if (action === "cancel") {
        set({ pendingCloseIntent: null });
        pendingReplacement = null;
        if (intent === "quit") await client.cancelExit();
        return;
      }
      if (action === "save") {
        const outcome = await saveDocument();
        if (!outcome?.clean) return; // failure/cancel/racing edit — stay open
      }
      set({ pendingCloseIntent: null });
      if (intent === "replacement") {
        const continuation = pendingReplacement;
        pendingReplacement = null;
        await continuation?.();
        return;
      }
      await proceed(intent);
    },
  };
});

/** Typed selector hook over the vanilla store. */
export function useAppStore<T>(selector: (s: AppState) => T): T {
  return useStore(appStore, selector);
}
