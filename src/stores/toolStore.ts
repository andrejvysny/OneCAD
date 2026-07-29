/*
 * Tool / mode store (F-WP3).
 *
 * Holds the editor mode (model ⇄ sketch), the active tool *per mode*, and a
 * minimal interaction-FSM phase (only idle/armed are exercised now; the drag /
 * preview / commit phases arrive with the tool-machine WP).
 *
 * `setMode` is the single mode-transition entry point — the titlebar toggle,
 * the `S` shortcut, a tree double-click, and the sketch chrome bar all route
 * through it, so entering/exiting sketch mode stays consistent (activeSketchId
 * + default tool + selection all move together).
 *
 * Bare `setMode('sketch')` (no id) is the "start a new sketch" intent: it leaves
 * activeSketchId null so the controller shows the plane picker; a tree/chrome
 * re-edit passes an explicit id and targets that existing sketch directly.
 */
import { createStore, useStore } from "zustand";
import { viewportStore } from "./viewportStore";
import { selectionStore } from "./selectionStore";
import { documentStore } from "./documentStore";

export type EditorMode = "model" | "sketch";

export type ModelTool =
  | "select"
  | "sketch"
  | "extrude"
  | "revolve"
  | "fillet"
  | "chamfer"
  | "boolean"
  | "shell"
  | "linearPattern"
  | "circularPattern"
  | "mirror";

export type SketchTool =
  | "select"
  | "line"
  | "rect"
  | "circle"
  | "arc"
  | "dimension"
  | "trim"
  | "mirror";

export type Tool = ModelTool | SketchTool;

/** Interaction FSM. Only idle/armed are used by the chrome WP. */
export type InteractionPhase =
  | "idle"
  | "armed"
  | "dragging"
  | "previewing"
  | "committing";

function phaseFor(tool: Tool): InteractionPhase {
  return tool === "select" ? "idle" : "armed";
}

export interface ToolState {
  mode: EditorMode;
  modelTool: ModelTool;
  sketchTool: SketchTool;
  phase: InteractionPhase;
  /** Enter/exit a mode. `sketchId` targets an existing sketch; omit it to start
   *  a new sketch (activeSketchId stays null → the controller shows the plane picker). */
  setMode(mode: EditorMode, sketchId?: string): void;
  /** Set the active tool for the *current* mode. */
  setTool(tool: Tool): void;
}

export const toolStore = createStore<ToolState>()((set, get) => ({
  mode: "model",
  modelTool: "select",
  sketchTool: "line",
  phase: "idle",

  setMode(mode, sketchId) {
    if (mode === "sketch") {
      // No id ⇒ new-sketch intent: leave activeSketchId null so the controller
      // shows the plane picker. An explicit id targets that existing sketch.
      const targetId = sketchId ?? null;
      set({ mode: "sketch", sketchTool: "line", phase: "idle" });
      viewportStore.getState().setActiveSketch(targetId);
      // Selecting the sketch keeps tree + inspector coherent with the chrome bar.
      if (targetId && documentStore.getState().sketches[targetId]) {
        selectionStore.getState().set([{ kind: "sketch", id: targetId }]);
      }
    } else {
      set({ mode: "model", modelTool: "select", phase: "idle" });
      viewportStore.getState().setActiveSketch(null);
    }
  },

  setTool(tool) {
    const { mode } = get();
    if (mode === "sketch") {
      set({ sketchTool: tool as SketchTool, phase: phaseFor(tool) });
    } else {
      set({ modelTool: tool as ModelTool, phase: phaseFor(tool) });
    }
  },
}));

/** Active tool for the current mode (the toolbar highlights this). */
export function activeTool(s: ToolState): Tool {
  return s.mode === "sketch" ? s.sketchTool : s.modelTool;
}

/** Typed selector hook over the vanilla store. */
export function useToolStore<T>(selector: (s: ToolState) => T): T {
  return useStore(toolStore, selector);
}
