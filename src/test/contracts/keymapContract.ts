/*
 * FROZEN keyboard contract — a literal copy of the three binding tables and the
 * cross-mode opt-out set as shipped before the Platform refactor.
 * See ./README.md before editing.
 *
 * AMENDED 2026-08-20 (Sketch UX plan item 6, a deliberate PRODUCT change, not a
 * probe fix): the sketch table gains six `applyConstraint` chords —
 * ⇧H/⇧V/⇧C/⇧E/⇧P/⇧M — following Shapr3D's Shift+letter constraint convention.
 * Perpendicular and Tangent are deliberately excluded (⇧T is already Extend and
 * neither has a free mnemonic). Nothing existing moved: the only overlap is ⇧H,
 * which sketch mode previously reached only through the cross-mode fallback to
 * the model Hole tool; model ⇧H = Hole is unchanged.
 *
 * AMENDED 2026-09-01 (Section view WP, a deliberate PRODUCT change, not a probe
 * fix — recorded in TODO.md § DAILY DRIVER v2 keymap table): the model table
 * gains ⇧X = `toggleSection`. ⇧X was free in BOTH tables; sketch mode's plain
 * `x` (construction) is untouched, and neither action is a `tool`, so the
 * cross-mode fallback cannot carry either one into the other mode.
 */
import type { KeyBinding } from "@/shortcuts/keymap";
import type { Tool } from "@/stores/toolStore";

export const MODEL_KEYS_CONTRACT: readonly KeyBinding[] = [
  { key: "v", action: { type: "tool", tool: "select" } },
  { key: "s", action: { type: "enterSketch" } },
  { key: "d", action: { type: "tool", tool: "datum" } },
  { key: "e", action: { type: "tool", tool: "extrude" } },
  { key: "r", action: { type: "tool", tool: "revolve" } },
  { key: "f", action: { type: "tool", tool: "fillet" } },
  { key: "b", action: { type: "tool", tool: "boolean" } },
  { key: "k", action: { type: "tool", tool: "shell" } },
  { key: "o", shift: true, action: { type: "tool", tool: "offsetFace" } },
  { key: "p", action: { type: "tool", tool: "linearPattern" } },
  { key: "c", action: { type: "tool", tool: "circularPattern" } },
  { key: "m", action: { type: "tool", tool: "mirror" } },
  { key: "t", action: { type: "tool", tool: "transform" } },
  { key: "h", shift: true, action: { type: "tool", tool: "hole" } },
  { key: "g", shift: true, action: { type: "tool", tool: "gear" } },
  { key: "?", shift: true, action: { type: "tool", tool: "measure" } },
  { key: "i", shift: true, action: { type: "isolate" } },
  { key: "x", shift: true, action: { type: "toggleSection" } },
];

export const SKETCH_KEYS_CONTRACT: readonly KeyBinding[] = [
  { key: "v", action: { type: "tool", tool: "select" } },
  { key: "l", action: { type: "tool", tool: "line" } },
  { key: "r", action: { type: "tool", tool: "rect" } },
  { key: "r", shift: true, action: { type: "tool", tool: "centerRect" } },
  { key: "c", action: { type: "tool", tool: "circle" } },
  { key: "o", action: { type: "tool", tool: "ellipse" } },
  { key: "a", action: { type: "tool", tool: "arc" } },
  { key: "a", shift: true, action: { type: "tool", tool: "arc3p" } },
  { key: "g", action: { type: "tool", tool: "polygon" } },
  { key: "s", action: { type: "tool", tool: "slot" } },
  { key: "p", action: { type: "tool", tool: "point" } },
  { key: "d", action: { type: "tool", tool: "dimension" } },
  { key: "t", action: { type: "tool", tool: "trim" } },
  { key: "t", shift: true, action: { type: "tool", tool: "extend" } },
  { key: "m", action: { type: "tool", tool: "mirror" } },
  { key: "f", action: { type: "tool", tool: "sketchFillet" } },
  { key: "o", shift: true, action: { type: "tool", tool: "sketchOffset" } },
  { key: "x", action: { type: "toggleConstruction" } },
  { key: "Delete", action: { type: "deleteSketchSelection" } },
  { key: "Backspace", action: { type: "deleteSketchSelection" } },
  { key: "h", shift: true, action: { type: "applyConstraint", constraint: "Horizontal" } },
  { key: "v", shift: true, action: { type: "applyConstraint", constraint: "Vertical" } },
  { key: "c", shift: true, action: { type: "applyConstraint", constraint: "Coincident" } },
  { key: "e", shift: true, action: { type: "applyConstraint", constraint: "Equal" } },
  { key: "p", shift: true, action: { type: "applyConstraint", constraint: "Parallel" } },
  { key: "m", shift: true, action: { type: "applyConstraint", constraint: "Midpoint" } },
];

export const GLOBAL_KEYS_CONTRACT: readonly KeyBinding[] = [
  { key: "Escape", action: { type: "cancel" } },
  { key: "Enter", action: { type: "finishSketch" } },
  { key: "h", action: { type: "home" } },
  { key: "f", shift: true, action: { type: "zoomFit" } },
];

/** Tools whose key is inert outside their own mode (no cross-mode fallback). */
export const NO_CROSS_MODE_CONTRACT: readonly Tool[] = ["measure"];
