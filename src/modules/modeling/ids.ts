/*
 * Modeling contribution ids (ADR-0008).
 *
 * The registry needs open, namespaced identity; the modeling controller needs
 * exhaustiveness checking over its own closed unions. Both, at different layers:
 * these const maps carry literal types, so internal code stays strongly typed,
 * while what reaches the registry is an ordinary namespaced string.
 *
 * Ids are SCOPE-QUALIFIED because `select` and `mirror` exist in both unions and
 * mean different things — sketch `mirror` mirrors 2D geometry, model `mirror`
 * is a MirrorBody operation. Collapsing them would make one shadow the other.
 *
 * `ids.test.ts` asserts these maps cover every union member exactly once and
 * agree with the descriptor table, so the two representations cannot drift.
 */
import type { CommandId, ToolId } from "@/platform";
import type { ModelTool, SketchTool } from "@/stores/toolStore";

export const ModelingModelTools = {
  select: "onecad.modeling.tool.model.select",
  sketch: "onecad.modeling.tool.model.sketch",
  datum: "onecad.modeling.tool.model.datum",
  extrude: "onecad.modeling.tool.model.extrude",
  revolve: "onecad.modeling.tool.model.revolve",
  fillet: "onecad.modeling.tool.model.fillet",
  boolean: "onecad.modeling.tool.model.boolean",
  shell: "onecad.modeling.tool.model.shell",
  offsetFace: "onecad.modeling.tool.model.offsetFace",
  hole: "onecad.modeling.tool.model.hole",
  linearPattern: "onecad.modeling.tool.model.linearPattern",
  circularPattern: "onecad.modeling.tool.model.circularPattern",
  mirror: "onecad.modeling.tool.model.mirror",
  transform: "onecad.modeling.tool.model.transform",
  measure: "onecad.modeling.tool.model.measure",
} as const satisfies Record<ModelTool, string>;

export const ModelingSketchTools = {
  select: "onecad.modeling.tool.sketch.select",
  line: "onecad.modeling.tool.sketch.line",
  rect: "onecad.modeling.tool.sketch.rect",
  centerRect: "onecad.modeling.tool.sketch.centerRect",
  circle: "onecad.modeling.tool.sketch.circle",
  ellipse: "onecad.modeling.tool.sketch.ellipse",
  arc: "onecad.modeling.tool.sketch.arc",
  arc3p: "onecad.modeling.tool.sketch.arc3p",
  polygon: "onecad.modeling.tool.sketch.polygon",
  slot: "onecad.modeling.tool.sketch.slot",
  point: "onecad.modeling.tool.sketch.point",
  dimension: "onecad.modeling.tool.sketch.dimension",
  trim: "onecad.modeling.tool.sketch.trim",
  extend: "onecad.modeling.tool.sketch.extend",
  mirror: "onecad.modeling.tool.sketch.mirror",
  sketchFillet: "onecad.modeling.tool.sketch.sketchFillet",
  sketchOffset: "onecad.modeling.tool.sketch.sketchOffset",
} as const satisfies Record<SketchTool, string>;

/**
 * Actions that are NOT tools: they do something once rather than arming an
 * interaction mode (docs/ARCHITECTURE.md §6 — `Command = action, Tool = mode`).
 */
export const ModelingCommands = {
  enterSketch: "onecad.modeling.command.enterSketch",
  finishSketch: "onecad.modeling.command.finishSketch",
  cancel: "onecad.modeling.command.cancel",
  deleteSketchSelection: "onecad.modeling.command.deleteSketchSelection",
  toggleConstruction: "onecad.modeling.command.toggleConstruction",
  // Isolation masks BODIES, so it stays modeling. `zoomFit` and `home` do NOT:
  // they are view navigation and moved to `onecad.shell` (ARCHITECTURE.md §7).
  isolate: "onecad.modeling.command.isolate",
} as const;

export type ModelingCommandKey = keyof typeof ModelingCommands;

/**
 * Row actions offered by modeling's `TreeProvider`.
 *
 * Separate from `ModelingCommands` because these register with the EDITOR-mount
 * scope, alongside the tree provider that offers them — the tree is editor
 * chrome, and registering them at bootstrap would advertise commands whose UI is
 * not loaded. They act on the selected row: opening a row menu selects it first.
 */
export const ModelingTreeCommands = {
  deleteSketch: "onecad.modeling.command.deleteSketch",
  deleteDatum: "onecad.modeling.command.deleteDatum",
} as const;

export function modelToolId(tool: ModelTool): ToolId {
  return ModelingModelTools[tool] as ToolId;
}

export function sketchToolId(tool: SketchTool): ToolId {
  return ModelingSketchTools[tool] as ToolId;
}

export function modelingCommandId(key: ModelingCommandKey): CommandId {
  return ModelingCommands[key] as CommandId;
}
