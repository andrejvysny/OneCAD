/*
 * The ONE undo/redo router (WP-U1).
 *
 * ⌘Z reaches the app from more than one place — the global keydown listener, the
 * ⌘K palette, and (T10) a native Edit-menu accelerator — and each of them used
 * to decide for itself what "undo" meant. That is how the Addendum C session lost
 * every undo: one path was mode-aware, the others did not exist. Everything now
 * calls `runUndo` / `runRedo`, so there is exactly one answer.
 *
 * Two rules, in order:
 *
 * 1. A focused text field keeps NATIVE text undo. `document.execCommand("undo")`
 *    is the only way to reach the webview's per-field edit stack, and it is
 *    deliberately terminal: if it returns false we do NOTHING. Falling through to
 *    the document history there would revert the model because the user tried to
 *    take back a typo — silently, with the focus still in the field.
 * 2. Otherwise it is mode-scoped: sketch mode drives the sketch-scoped stack
 *    (`sketchService`), model mode the document history (`ModelToolController`).
 */
import { createClient } from "@/ipc/client";
import { isEditableTarget } from "@/shortcuts/editableTarget";
import { toolStore } from "@/stores/toolStore";
import { redoSketch, undoSketch } from "@/tools/sketch/sketchService";
import { getModelToolController } from "@/tools/modelTools/modelToolBridge";

/**
 * Hand the chord to the focused text field, if there is one.
 * Returns whether this keystroke was consumed by native text editing.
 */
function nativeTextEdit(dir: "undo" | "redo"): boolean {
  if (typeof document === "undefined") return false;
  if (!isEditableTarget(document.activeElement)) return false;
  // `execCommand` is deprecated but unreplaced: there is no other API that
  // reaches a webview's native per-field undo stack. A `false` return (WebKit
  // reports it for an empty stack) still counts as consumed — see rule 1.
  try {
    document.execCommand(dir);
  } catch {
    // A browser that refuses the call has no text stack to reach either.
  }
  return true;
}

/** Undo: native text undo inside a field, else the mode's history. */
export async function runUndo(): Promise<void> {
  if (nativeTextEdit("undo")) return;
  if (toolStore.getState().mode === "sketch") {
    await undoSketch(createClient());
    return;
  }
  await getModelToolController()?.undo();
}

/** Redo: native text redo inside a field, else the mode's history. */
export async function runRedo(): Promise<void> {
  if (nativeTextEdit("redo")) return;
  if (toolStore.getState().mode === "sketch") {
    await redoSketch(createClient());
    return;
  }
  await getModelToolController()?.redo();
}
