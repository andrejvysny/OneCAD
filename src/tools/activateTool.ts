/*
 * activateTool — AUTO-MODE tool dispatcher. The editor mode is DERIVED from the
 * user's intent (tool + context), never toggled by hand:
 *
 *   model mode + sketch-only tool (line/rect/centerRect/circle/ellipse/arc/polygon/
 *     slot/point/dimension/trim)
 *     ⇒ enter sketch mode WITH that tool armed (no active sketch ⇒ the usual
 *       entry flow: selected face → sketch-on-face, else the plane picker).
 *   sketch mode + model-only tool (extrude/revolve/fillet/boolean/shell/
 *     patterns)
 *     ⇒ finish the sketch (same drained-squash path as the Enter shortcut — the
 *       whole session stays ONE undo step), land in model mode WITH the tool
 *       armed. Extrude additionally rides the pendingExtrude handoff so the
 *       region pick auto-arms it (ModelToolController).
 *   context-local ids (select / mirror) and same-mode picks
 *     ⇒ plain setTool in the current mode. The model toolbar's "sketch" id stays
 *       the explicit new-sketch intent.
 *
 * Both the floating toolbar and the keyboard shortcuts route here, so mouse and
 * keyboard can never diverge.
 */
import { toolStore, type ModelTool, type SketchTool, type Tool } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { flushSketchMutations } from "@/tools/sketch/sketchService";

const SKETCH_ONLY: ReadonlySet<Tool> = new Set([
  "line",
  "rect",
  "centerRect",
  "circle",
  "ellipse",
  "arc",
  "polygon",
  "slot",
  "point",
  "dimension",
  "trim",
]);

const MODEL_ONLY: ReadonlySet<Tool> = new Set([
  // Datum planes are authored against WORLD/datum planes, which only exist in
  // model mode — pressing D inside a sketch finishes that sketch first (the
  // shared drained-squash path), exactly like Extrude.
  "datum",
  "extrude",
  "revolve",
  "fillet",
  "boolean",
  "shell",
  "linearPattern",
  "circularPattern",
  // Measure inspects SOLIDS, which only exist in model mode. A toolbar CLICK is
  // an explicit intent, so it finishes the sketch first like every other model
  // tool. Its ambient `?` KEYSTROKE is deliberately inert in sketch mode instead
  // — see NO_CROSS_MODE in shortcuts/keymap.ts.
  "measure",
]);

export function isSketchOnlyTool(tool: Tool): boolean {
  return SKETCH_ONLY.has(tool);
}

export function isModelOnlyTool(tool: Tool): boolean {
  return MODEL_ONLY.has(tool);
}

/**
 * Finish the active sketch and land in model mode with `tool` armed. Mirrors
 * finishSketchAction (useShortcuts): DRAIN the mutation queue first so a
 * still-in-flight upsert settles before regions/undo are computed, THEN flip —
 * the pendingExtrude consumer guards on mode === "model", so the handoff id is
 * only delivered after the flip.
 */
async function finishSketchToTool(tool: ModelTool): Promise<void> {
  await flushSketchMutations();
  const s = toolStore.getState();
  if (s.mode !== "sketch") return; // left sketch mode while the queue drained
  const sketchId = viewportStore.getState().activeSketchId;
  s.setMode("model", undefined, { tool });
  if (tool === "extrude" && sketchId) {
    viewportStore.getState().setPendingExtrude(sketchId);
  }
}

export async function activateTool(tool: Tool): Promise<void> {
  const s = toolStore.getState();
  // The model toolbar's "New sketch" id is an enter intent, not a real tool.
  if (tool === "sketch") {
    if (s.mode === "model") s.setMode("sketch");
    return;
  }
  if (s.mode === "model" && SKETCH_ONLY.has(tool)) {
    s.setMode("sketch", undefined, { tool: tool as SketchTool });
    return;
  }
  if (s.mode === "sketch" && MODEL_ONLY.has(tool)) {
    await finishSketchToTool(tool as ModelTool);
    return;
  }
  s.setTool(tool);
}
