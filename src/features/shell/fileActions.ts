/*
 * File actions — the shared bridge the TitleBar File menu + the ⌘S/⇧⌘S/⌘O
 * shortcuts both call. Each routes through the memoized CadClient (Rust owns the
 * dialogs + filesystem), surfaces success/errors through the F-WP9 status-hint
 * pattern (`viewportStore.setStatusHint`), and refreshes the recents projection
 * after an open/save so the persisted store and the start screen stay in sync.
 */
import { createClient } from "@/ipc/client";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore } from "@/stores/documentStore";
import { appStore } from "@/stores/appStore";

const client = createClient();

/** Show a self-clearing success hint (the store auto-dismisses non-sticky hints). */
function transientHint(message: string): void {
  viewportStore.getState().setStatusHint(message);
}

/** Surface a recoverable failure — sticky, so it stays until the next status change. */
function errorHint(message: string): void {
  viewportStore.getState().setStatusHint(message, { severity: "error", sticky: true });
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** The backend reports a missing save path as an `io` ApiError — Save then Save-As. */
function isNoPathError(e: unknown): boolean {
  return message(e).toLowerCase().includes("no save path");
}

/** File stem for a path (drops directories + extension), for the "Saved ⟨name⟩" hint. */
function baseName(path: string): string {
  const file = path.split(/[\\/]/).pop() ?? path;
  return file.replace(/\.[^.]+$/, "");
}

function docName(): string {
  return documentStore.getState().title || "document";
}

function refreshRecents(): void {
  void appStore.getState().loadRecents();
}

/**
 * ⌘S: save to the known path; a never-saved document falls back to Save As.
 * Resolves `true` on a completed save (the UnsavedChangesDialog's "Save" action
 * reads this to decide whether it may proceed with the close/quit); `false` on
 * any failure or a cancelled Save As dialog — the error hint (if any) is already
 * surfaced here, so callers just need the boolean.
 */
export async function saveDocument(): Promise<boolean> {
  try {
    await client.saveDocument();
    transientHint(`Saved ${docName()}`);
    refreshRecents();
    return true;
  } catch (e) {
    if (isNoPathError(e)) {
      return saveDocumentAs();
    }
    errorHint(`Save failed: ${message(e)}`);
    return false;
  }
}

/** ⇧⌘S: dialog + save. A cancelled dialog is a no-op (no hint), resolving `false`
 *  (see `saveDocument`'s return contract). */
export async function saveDocumentAs(): Promise<boolean> {
  try {
    const path = await client.saveDocumentAs();
    if (!path) return false; // cancelled
    transientHint(`Saved ${baseName(path)}`);
    refreshRecents();
    return true;
  } catch (e) {
    errorHint(`Save failed: ${message(e)}`);
    return false;
  }
}

/** Export STEP…: dialog (Rust) + worker export. A cancelled dialog is a no-op. */
export async function exportStep(): Promise<void> {
  try {
    const path = await client.exportStep();
    if (!path) return; // cancelled
    transientHint(`Exported ${baseName(path)}`);
  } catch (e) {
    errorHint(`Export failed: ${message(e)}`);
  }
}

/** Export STL…: dialog (Rust) + worker mesh export. A cancelled dialog is a no-op. */
export async function exportStl(): Promise<void> {
  try {
    const path = await client.exportStl();
    if (!path) return; // cancelled
    transientHint(`Exported ${baseName(path)}`);
  } catch (e) {
    errorHint(`Export failed: ${message(e)}`);
  }
}

/** Export OBJ…: dialog (Rust) + worker mesh export. A cancelled dialog is a no-op. */
export async function exportObj(): Promise<void> {
  try {
    const path = await client.exportObj();
    if (!path) return; // cancelled
    transientHint(`Exported ${baseName(path)}`);
  } catch (e) {
    errorHint(`Export failed: ${message(e)}`);
  }
}

/** ⌘O: native open dialog + open (+ recents refresh); stays on the editor shell. */
export async function openDocumentDialog(): Promise<void> {
  try {
    await appStore.getState().openDialogAndOpen();
  } catch (e) {
    errorHint(`Open failed: ${message(e)}`);
  }
}

/**
 * ⌘W / TitleBar ×: close the open project and return to the start screen. Routed
 * through `appStore.requestClose` (the ONE shared entry point every close/quit
 * path uses) so a dirty document prompts via the UnsavedChangesDialog instead of
 * discarding work unconditionally.
 */
export async function closeProject(): Promise<void> {
  try {
    await appStore.getState().requestClose("close");
  } catch (e) {
    errorHint(`Close failed: ${message(e)}`);
  }
}
