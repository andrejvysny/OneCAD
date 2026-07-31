/*
 * Global keyboard handler (F-WP3). Installs one window keydown listener that
 * resolves bindings mode-scoped and runs the matched action against the stores.
 * Bails when a text input is focused and leaves OS/app chords (Cmd/Ctrl/Alt)
 * untouched so a later Save/Undo layer can own them.
 */
import { useEffect } from "react";
import { toolStore, activeTool } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { createClient } from "@/ipc/client";
import {
  deleteEntities,
  flushSketchMutations,
  redoSketch,
  setEntitiesConstruction,
  undoSketch,
} from "@/tools/sketch/sketchService";
import { sketchStore } from "@/stores/sketchStore";
import { getModelToolController } from "@/tools/modelTools/modelToolBridge";
import { activateTool } from "@/tools/activateTool";
import {
  closeProject,
  openDocumentDialog,
  saveDocument,
  saveDocumentAs,
} from "@/features/shell/fileActions";
import { resolveBinding, type ShortcutAction } from "./keymap";

function isEditableTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  const tag = el.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    el.isContentEditable
  );
}

/** Esc ladder: cancel active tool → deselect → exit sketch mode. */
function runCancel(): void {
  const tool = toolStore.getState();
  if (activeTool(tool) !== "select") {
    tool.setTool("select");
    return;
  }
  const sel = selectionStore.getState();
  if (sel.selected.length > 0) {
    sel.clear();
    return;
  }
  if (tool.mode === "sketch") {
    tool.setMode("model");
  }
}

/**
 * Delete the current sketch selection. Point-picks delete their OWNING entity (a
 * `{entityId, point}` pick deletes the whole entity — V1 semantics), so collect
 * the distinct entity ids and hand them to `deleteEntities` (which also cascades
 * the referencing constraints). Selection is cleared once the delete settles.
 * Constraint deletion is NOT keyboard-driven here (a later WP wires the list).
 */
function runDeleteSketchSelection(): void {
  const sel = sketchSelectionStore.getState();
  const entityIds = [...new Set(sel.selected.map((s) => s.entityId))];
  if (entityIds.length === 0) return;
  void deleteEntities(createClient(), entityIds).then(() => {
    sketchSelectionStore.getState().clear();
  });
}

/**
 * X — construction geometry (W1-B), two behaviours by selection state:
 *   - SELECTION non-empty → flip those entities. MIXED-SELECTION RULE (the single
 *     owner of it): the target is `!selected.every(construction)`, i.e. everything
 *     becomes construction UNLESS every picked entity already is, in which case
 *     everything flips back to real. Idempotent per press-pair, and a mixed pick
 *     never leaves the user guessing which half moved.
 *   - SELECTION empty → toggle the sticky construction DRAW mode instead.
 * Point-picks resolve to their OWNING entity (same V1 rule as delete).
 */
function runToggleConstruction(): void {
  const sel = sketchSelectionStore.getState().selected;
  const entityIds = [...new Set(sel.map((s) => s.entityId))];
  if (entityIds.length === 0) {
    sketchStore.getState().toggleConstructionMode();
    return;
  }
  const picked = sketchStore.getState().session?.entities.filter((e) => entityIds.includes(e.id)) ?? [];
  if (picked.length === 0) return;
  const target = !picked.every((e) => e.construction);
  void setEntitiesConstruction(createClient(), entityIds, target);
}

/**
 * Finish the active sketch → hand it to the model layer for profile selection
 * (Shapr3D flow). DRAIN the sketch mutation queue FIRST: a still-in-flight upsert
 * (fast last click / dimension edit) must settle before regions are computed, else
 * the profile is captured from a stale sketch. THEN flip mode — mode must flip
 * before setting the pending id: the ModelToolController consumes
 * pendingExtrudeSketch from a viewportStore subscription that guards on
 * mode === "model", so setting it while still in sketch mode would be observed
 * once, dropped, and never re-delivered.
 */
async function finishSketchAction(): Promise<void> {
  await flushSketchMutations();
  const tool = toolStore.getState();
  if (tool.mode !== "sketch") return; // left sketch mode while the queue drained
  const sketchId = viewportStore.getState().activeSketchId;
  tool.setMode("model");
  if (sketchId) viewportStore.getState().setPendingExtrude(sketchId);
}

export function runAction(action: ShortcutAction): void {
  const tool = toolStore.getState();
  switch (action.type) {
    case "tool":
      // AUTO-MODE: a tool key can cross the mode boundary (L in model mode
      // enters a sketch; E in sketch mode finishes + arms Extrude).
      void activateTool(action.tool);
      break;
    case "enterSketch":
      if (tool.mode === "model") tool.setMode("sketch");
      break;
    case "finishSketch":
      if (tool.mode === "sketch") void finishSketchAction();
      break;
    case "deleteSketchSelection":
      runDeleteSketchSelection();
      break;
    case "toggleConstruction":
      runToggleConstruction();
      break;
    case "cancel":
      runCancel();
      break;
    case "zoomFit":
      viewportStore.getState().zoomFit();
      break;
    case "home":
      viewportStore.getState().homeView();
      break;
  }
}

export function useShortcuts(): void {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      // Undo / redo own the ⌘Z / ⇧⌘Z (and Ctrl+Y) chords (F-WP7). In SKETCH mode
      // they drive the sketch-scoped undo (sketchService); in model mode, the
      // ModelToolController history — mode-gated so the two never cross-fire.
      const mod = e.metaKey || e.ctrlKey;
      const inSketch = toolStore.getState().mode === "sketch";
      if (mod && (e.key === "z" || e.key === "Z")) {
        e.preventDefault();
        if (inSketch) {
          if (e.shiftKey) void redoSketch(createClient());
          else void undoSketch(createClient());
        } else {
          const ctrl = getModelToolController();
          if (e.shiftKey) void ctrl?.redo();
          else void ctrl?.undo();
        }
        return;
      }
      if (mod && (e.key === "y" || e.key === "Y")) {
        e.preventDefault();
        if (inSketch) void redoSketch(createClient());
        else void getModelToolController()?.redo();
        return;
      }
      // File chords own ⌘S (Save) / ⇧⌘S (Save As) / ⌘O (Open) in every mode; they
      // route through the shared fileActions bridge (Rust owns dialogs + fs).
      if (mod && (e.key === "s" || e.key === "S")) {
        e.preventDefault();
        if (e.shiftKey) void saveDocumentAs();
        else void saveDocument();
        return;
      }
      if (mod && !e.shiftKey && (e.key === "o" || e.key === "O")) {
        e.preventDefault();
        void openDocumentDialog();
        return;
      }
      if (mod && !e.shiftKey && (e.key === "w" || e.key === "W")) {
        e.preventDefault();
        void closeProject();
        return;
      }
      // Leave remaining OS / app chords (Cmd/Ctrl/Alt) to their owners; Shift ok.
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (isEditableTarget(e.target)) return;
      if (e.repeat) return;
      const action = resolveBinding(e.key, e.shiftKey, toolStore.getState().mode);
      if (!action) return;
      // Delete/Backspace only swallow the key when a sketch entity is selected;
      // an empty selection falls through so the key keeps its default meaning.
      if (
        action.type === "deleteSketchSelection" &&
        sketchSelectionStore.getState().selected.length === 0
      ) {
        return;
      }
      e.preventDefault();
      runAction(action);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
