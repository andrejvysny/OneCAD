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
import { paletteStore } from "@/stores/paletteStore";
import { createClient } from "@/ipc/client";
import {
  applyConstraint,
  deleteConstraints,
  deleteEntities,
  flushSketchMutations,
  redoSketch,
  setEntitiesConstruction,
  undoSketch,
} from "@/tools/sketch/sketchService";
import { visibleConstraints } from "@/features/inspector/ConstraintList";
import { applicableConstraintsFor } from "@/features/sketch/useApplicableConstraints";
import { CONSTRAINT_REQUIREMENT } from "@/features/sketch/constraintCatalog";
import { documentStore } from "@/stores/documentStore";
import type { SketchConstraintType } from "@/ipc/types";
import { sketchStore } from "@/stores/sketchStore";
import { getModelToolController } from "@/tools/modelTools/modelToolBridge";
import { activateTool } from "@/tools/activateTool";
import {
  closeProject,
  openDocumentDialog,
  saveDocument,
  saveDocumentAs,
} from "@/features/shell/fileActions";
import { viewportNavigation } from "@/modules/shell/viewportNavigation";
import { modelingContext } from "@/modules/modeling/selectionContext";
import { usePlatform, type Platform } from "@/platform";
import { logWarn } from "@/debug/log";
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

/** Esc ladder: cancel active tool → exit isolation → deselect → exit sketch mode
 *  (the last rung reports what it left — see {@link exitSketch}). */
function runCancel(): void {
  const tool = toolStore.getState();
  if (activeTool(tool) !== "select") {
    tool.setTool("select");
    return;
  }
  // Isolation outranks deselect: the selection is what the user isolated, so
  // clearing it first would strand them inside an isolate set with nothing
  // selected and no obvious way back.
  const viewport = viewportStore.getState();
  if (viewport.isolatedBodyIds !== null) {
    viewport.exitIsolate();
    return;
  }
  const sel = selectionStore.getState();
  if (sel.selected.length > 0) {
    sel.clear();
    return;
  }
  if (tool.mode === "sketch") {
    // A12: the ladder's last rung used to flip the mode itself, silently. It now
    // goes through the SAME exit path the Finish button and Enter use, tagged
    // with the reason — that tag is the whole seam: `exit()` sees only a mode
    // flip and cannot tell an Esc from a Finish, so the caller has to say.
    void exitSketch("escape");
  }
}

/**
 * Delete the current sketch selection. Point-picks delete their OWNING entity (a
 * `{entityId, point}` pick deletes the whole entity — V1 semantics), so collect
 * the distinct entity ids and hand them to `deleteEntities` (which also cascades
 * the referencing constraints). Selection is cleared once the delete settles.
 *
 * A CONSTRAINT pick (a canvas badge, plan item 5d) takes precedence and routes
 * through `deleteConstraints` — the same verb the inspector row's delete button
 * uses — and it inherits that list's protection: only a constraint the list
 * would show is deletable, so the machine `Fixed` pins a face-hosted sketch
 * projects onto locked reference geometry stay put (see `visibleConstraints`).
 * The two picks are mutually exclusive in the store, so this never has to guess.
 */
function runDeleteSketchSelection(): void {
  const sel = sketchSelectionStore.getState();
  if (sel.selectedConstraintId) {
    const session = sketchStore.getState().session;
    const deletable = session
      ? visibleConstraints(session.constraints, session.entities)
      : [];
    const target = deletable.find((c) => c.id === sel.selectedConstraintId);
    sketchSelectionStore.getState().setSelectedConstraint(null);
    if (target) void deleteConstraints(createClient(), [target.id]);
    return;
  }
  const entityIds = [...new Set(sel.selected.map((s) => s.entityId))];
  if (entityIds.length === 0) return;
  void deleteEntities(createClient(), entityIds).then(() => {
    sketchSelectionStore.getState().clear();
  });
}

/**
 * ⇧H/⇧V/⇧C/⇧E/⇧P/⇧M — apply one constraint kind to the current selection
 * (plan item 6). Goes through the SAME `evaluateApplicability` →
 * `applyConstraint` pair the Add-constraint menu and the context chips use
 * (`applicableConstraintsFor`), so a chord can never author something the menu
 * would have refused. Inapplicable ⇒ the kind's own requirement sentence as a
 * transient info hint, never a silent no-op.
 */
function runApplyConstraint(type: SketchConstraintType): void {
  const session = sketchStore.getState().session;
  if (!session) return;
  const selected = sketchSelectionStore.getState().selected;
  const applicable = applicableConstraintsFor(selected, session.entities).find(
    (a) => a.type === type,
  );
  if (!applicable) {
    viewportStore.getState().setStatusHint(CONSTRAINT_REQUIREMENT[type]);
    return;
  }
  void applyConstraint(createClient(), applicable);
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
 * WHY the sketch is being left. The mode flip itself carries no intent —
 * `SketchController.exit()` runs the identical cancel+finish teardown either way
 * — so the reason travels as an argument, never as a module flag some other
 * caller could leave stale (audit A12).
 *   - `finish`  Enter, the Finish button, the command: hand the profile to the
 *               model layer (arms the Extrude handoff).
 *   - `escape`  the Esc ladder's last rung: an escape hatch, so no handoff — but
 *               no longer silent either.
 */
type SketchExitReason = "finish" | "escape";

/** What the user is told on the way out. An Esc off an EMPTY sketch is the one
 *  path that must not claim "Finished": nothing was drawn, and the sketch is
 *  kept (exit() still mints its timeline record), so it may not claim a discard
 *  either. */
function exitHint(reason: SketchExitReason, name: string, entityCount: number): string {
  if (reason === "escape" && entityCount === 0) return `Closed ${name} — nothing drawn`;
  return `Finished ${name} — ${entityCount} ${entityCount === 1 ? "entity" : "entities"}`;
}

/**
 * Leave the active sketch → hand it to the model layer for profile selection
 * (Shapr3D flow). DRAIN the sketch mutation queue FIRST: a still-in-flight upsert
 * (fast last click / dimension edit) must settle before regions are computed, else
 * the profile is captured from a stale sketch. THEN flip mode — mode must flip
 * before setting the pending id: the ModelToolController consumes
 * pendingExtrudeSketch from a viewportStore subscription that guards on
 * mode === "model", so setting it while still in sketch mode would be observed
 * once, dropped, and never re-delivered.
 *
 * Leaving is otherwise SILENT (audit A12) — the shell just flips back to
 * model mode — so the last step names what was left and how big it was
 * (plan item 10a). It never overwrites an error the exit itself raised: a
 * failed drain leaves its own hint, and a confirmation on top of it would say
 * the edit landed when it did not.
 */
async function exitSketch(reason: SketchExitReason): Promise<void> {
  const hintBefore = viewportStore.getState().statusHint;
  await flushSketchMutations();
  const tool = toolStore.getState();
  if (tool.mode !== "sketch") return; // left sketch mode while the queue drained
  // Counted AFTER the drain — a still-in-flight upsert is part of what is being
  // finished, so the pre-drain number would under-report the last click.
  const entityCount = sketchStore.getState().session?.entities.length ?? 0;
  const sketchId = viewportStore.getState().activeSketchId;
  const name = (sketchId ? documentStore.getState().sketches[sketchId]?.name : null) ?? "sketch";
  tool.setMode("model");
  // Only an EXPLICIT finish arms the Extrude handoff. Esc means "get me out",
  // and answering it with a profile prompt would be the opposite of an escape.
  if (reason === "finish" && sketchId) viewportStore.getState().setPendingExtrude(sketchId);
  const hintAfter = viewportStore.getState().statusHint;
  const raisedAnError = hintAfter !== hintBefore && hintAfter?.severity === "error";
  if (raisedAnError) return;
  viewportStore.getState().setStatusHint(exitHint(reason, name, entityCount));
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
      if (tool.mode === "sketch") void exitSketch("finish");
      break;
    case "deleteSketchSelection":
      runDeleteSketchSelection();
      break;
    case "toggleConstruction":
      runToggleConstruction();
      break;
    case "applyConstraint":
      runApplyConstraint(action.constraint);
      break;
    case "cancel":
      runCancel();
      break;
    // Through the shell's navigation implementation, NOT the store directly:
    // the command and the keystroke must be the same call, or the two drift.
    case "zoomFit":
      viewportNavigation.zoomFit();
      break;
    case "isolate":
      // Isolation stays modeling — it masks BODIES, which the host knows nothing about.
      viewportStore.getState().toggleIsolate();
      break;
    case "home":
      viewportNavigation.home();
      break;
  }
}

export function useShortcuts(): void {
  const platform = usePlatform();

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
      // ⌘K opens the command palette. Handled ABOVE the editable-target bail on
      // purpose: it is the one chord that must work while a field has focus,
      // including the palette's own input (where it toggles back shut).
      if (mod && !e.shiftKey && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        paletteStore.getState().toggle();
        return;
      }
      // Leave remaining OS / app chords (Cmd/Ctrl/Alt) to their owners; Shift ok.
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (isEditableTarget(e.target)) return;
      if (e.repeat) return;
      const action = resolveBinding(e.key, e.shiftKey, toolStore.getState().mode);
      if (!action) {
        // Nothing in the built-in tables claims this chord — ask the registries.
        // This is the only lane a contributed `defaultShortcut` can fire through,
        // and it deliberately runs SECOND: the frozen keymap contract describes
        // the built-in answers, and a registration must not be able to shadow one.
        runRegisteredShortcut(platform, e);
        return;
      }
      // Delete/Backspace only swallow the key when a sketch entity OR a
      // constraint badge is selected; an empty selection falls through so the
      // key keeps its default meaning.
      if (action.type === "deleteSketchSelection") {
        const s = sketchSelectionStore.getState();
        if (s.selected.length === 0 && s.selectedConstraintId === null) return;
      }
      e.preventDefault();
      runAction(action);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [platform]);
}

/**
 * Fire a chord claimed by a registration rather than by the built-in tables.
 *
 * An AMBIGUOUS chord activates nothing and says so. Picking a winner by load
 * order is the failure this architecture exists to prevent, and a keystroke that
 * silently ran the wrong addon's command would be the worst place to allow it.
 */
function runRegisteredShortcut(platform: Platform, e: KeyboardEvent): void {
  const chord = { key: e.key, shift: e.shiftKey };
  // The SAME context resolution runs against is what the target is executed with
  // — a scope-gated command that resolves and then refuses is worse than one
  // that never resolved.
  const context = modelingContext();
  const scopes = context.scopes;

  const target = platform.shortcuts.resolve(chord, scopes);
  if (!target) {
    const conflict = platform.shortcuts.conflictFor(chord, scopes);
    if (conflict) {
      logWarn("shortcut", "chord claimed by more than one contribution — ignored", {
        key: chord.key,
        shift: chord.shift,
        candidates: conflict.candidates.map((c) => c.id),
      });
    }
    return;
  }

  e.preventDefault();
  void platform.shortcuts.run(target, context);
}
