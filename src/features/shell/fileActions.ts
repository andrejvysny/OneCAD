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
import { getViewportEngine } from "@/viewport/engineBridge";
import type { SaveOutcome } from "@/ipc/types";

const client = createClient();

/**
 * Best-effort viewport thumbnail for the container's `preview.png`.
 *
 * EXPLICIT saves only, and that is the whole design: this reads the live GPU
 * canvas, so it can only run where a user action already implies a painted
 * viewport. Autosave is Rust-side and never comes through here, so it simply
 * keeps whatever preview the container already had.
 *
 * Returns `undefined` — never throws, never blocks — when no engine is mounted
 * (start screen, jsdom, headless), on WebGPU, or on any capture failure. The
 * save proceeds regardless; the backend applies the same "a screenshot must
 * never cost the user their work" rule to a malformed or oversized payload.
 */
function capturePreview(): string | undefined {
  try {
    return getViewportEngine()?.captureThumbnail() ?? undefined;
  } catch {
    return undefined;
  }
}

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

function refreshRecents(): void {
  void appStore.getState().loadRecents();
}

function adoptSaveOutcome(outcome: SaveOutcome): void {
  documentStore.getState().applySaveOutcome(outcome);
  const message = outcome.clean
    ? `Saved ${outcome.title}`
    : `Saved ${outcome.title} — newer changes remain unsaved`;
  viewportStore.getState().setStatusHint(message, outcome.clean ? undefined : { sticky: true });
  refreshRecents();
}

/**
 * ⌘S: save to the known path; a never-saved document falls back to Save As.
 * Resolves `true` on a completed save (the UnsavedChangesDialog's "Save" action
 * reads this to decide whether it may proceed with the close/quit); `false` on
 * any failure or a cancelled Save As dialog — the error hint (if any) is already
 * surfaced here, so callers just need the boolean.
 */
export async function saveDocument(): Promise<SaveOutcome | null> {
  try {
    const outcome = await client.saveDocument(undefined, capturePreview());
    adoptSaveOutcome(outcome);
    return outcome;
  } catch (e) {
    if (isNoPathError(e)) {
      return saveDocumentAs();
    }
    errorHint(`Save failed: ${message(e)}`);
    return null;
  }
}

/** ⇧⌘S: dialog + save. A cancelled dialog is a no-op (no hint), resolving `false`
 *  (see `saveDocument`'s return contract). */
export async function saveDocumentAs(): Promise<SaveOutcome | null> {
  try {
    // Captured BEFORE the dialog opens — a native modal can obscure or blank the
    // webview, and after it closes the on-demand canvas may not have repainted.
    const preview = capturePreview();
    const outcome = await client.saveDocumentAs(preview);
    if (!outcome) return null; // cancelled
    adoptSaveOutcome(outcome);
    return outcome;
  } catch (e) {
    errorHint(`Save failed: ${message(e)}`);
    return null;
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

/**
 * Import STEP…: append a STEP import to the OPEN document (STEP-IMPORT WP-A).
 *
 * NOT the start-screen "Import STEP…" — that one OPENS a new document from the
 * file (`appStore.importStep`). This appends an `ImportStep` history record to
 * the document already on screen, so nothing is swapped and nothing is lost.
 *
 * Rust owns the dialog; a cancel resolves null and is a no-op (no hint), matching
 * every other dialog-backed action here. The document itself updates through the
 * normal projection/regen path — the result is read only for the hint.
 */
export async function insertStep(): Promise<void> {
  try {
    const res = await client.insertStep();
    if (!res) return; // cancelled
    if (res.errorMessage) {
      errorHint(`Import failed: ${res.errorMessage}`);
      return;
    }
    transientHint(bodyCountHint(res.changedBodies.length));
  } catch (e) {
    errorHint(`Import failed: ${message(e)}`);
  }
}

/**
 * Import Project…: append another `.onecad` document's timeline to the OPEN
 * document. Rust owns the `.onecad` dialog. A cancelled dialog is a no-op.
 */
export async function importProject(): Promise<void> {
  try {
    const snap = await client.importProject();
    if (!snap) return; // cancelled
    transientHint("Project imported");
  } catch (e) {
    errorHint(`Import project failed: ${message(e)}`);
  }
}

/** "Imported 1 body" / "Imported 3 bodies" — the import success confirmation. */
function bodyCountHint(count: number): string {
  return `Imported ${count} ${count === 1 ? "body" : "bodies"}`;
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
