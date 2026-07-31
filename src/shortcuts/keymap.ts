/*
 * Keyboard bindings (F-WP3) — data-driven, mode-scoped.
 *
 * Model:  V select · S new-sketch (enters sketch mode) · E extrude · R revolve
 *         · F fillet · B combine/boolean
 * Sketch: V select · L line · R rectangle · ⇧R center-rectangle · C circle
 *         · O ellipse · A arc · G polygon · S slot · P point · D dimension
 *         · T trim · M mirror · X construction (flip selection / sticky mode)
 * Global: Esc cancel-ladder · Enter finish-sketch (sketch mode) · H home (stub)
 *         · Shift+F zoom-to-fit
 *
 * COLLISIONS (all deliberate, all resolved by `mode` or by a chord — never by
 * guessing):
 *   - `R` means different tools per mode (revolve vs rectangle).
 *   - `⇧R` (exact shift match) is the sketch center-rectangle. Model mode has no
 *     ⇧R of its own, so it falls through cross-mode and starts a sketch with it.
 *   - `F` collides between the Fillet tool (model toolbar) and zoom-to-fit (nav
 *     pill): the toolbar tool wins plain `F`, zoom-to-fit moves to ⇧F.
 *   - `P` in SKETCH mode is the Point tool, which SHADOWS the cross-mode fallback
 *     to model `P` = Linear pattern (W2-B, deliberate: a draw tool must beat a
 *     mode-crossing model op while the user is drawing). Model `P` is unchanged.
 *   - `S` in SKETCH mode is the Slot tool. Model `S` = new-sketch stays intact
 *     because mode bindings win, and `enterSketch` is not a `tool` action, so the
 *     cross-mode fallback could never have claimed sketch `S` anyway.
 *   - `G` was free in both modes; sketch owns it (Polygon) and model mode reaches
 *     it through the normal cross-mode fallback.
 *   - `O` (Ellipse, W3 P3) was likewise free in BOTH tables, so sketch owns it and
 *     model mode reaches it cross-mode. Deliberately NOT `E`: that letter is the
 *     model Extrude binding, i.e. the cross-mode "finish this sketch and extrude"
 *     handoff — rebinding it in sketch mode would shadow the handoff.
 *
 * AUTO-MODE: a key bound only in the OTHER mode resolves cross-mode (tool
 * actions only) so shortcuts drive the automatic mode switch — see
 * `resolveBinding` and `tools/activateTool.ts`.
 */
import type { EditorMode, Tool } from "@/stores/toolStore";

export type ShortcutAction =
  | { type: "tool"; tool: Tool }
  | { type: "enterSketch" }
  | { type: "finishSketch" }
  | { type: "deleteSketchSelection" }
  | { type: "toggleConstruction" }
  | { type: "cancel" }
  | { type: "zoomFit" }
  | { type: "home" };

export interface KeyBinding {
  /** Single printable key (compared case-insensitively) or a named key. */
  key: string;
  shift?: boolean;
  action: ShortcutAction;
}

export const MODEL_KEYS: KeyBinding[] = [
  { key: "v", action: { type: "tool", tool: "select" } },
  { key: "s", action: { type: "enterSketch" } },
  { key: "e", action: { type: "tool", tool: "extrude" } },
  { key: "r", action: { type: "tool", tool: "revolve" } },
  { key: "f", action: { type: "tool", tool: "fillet" } },
  // H for cHamfer: F is taken by fillet and ⇧F by zoom-fit, and C/M are already
  // spoken for by the pattern/mirror tools.
  { key: "h", action: { type: "tool", tool: "chamfer" } },
  { key: "b", action: { type: "tool", tool: "boolean" } },
  // M6b model ops (K/P/C/M are free in model mode; C/M also serve sketch tools
  // in sketch mode, resolved by `mode` exactly as R does — revolve vs rectangle).
  { key: "k", action: { type: "tool", tool: "shell" } },
  { key: "p", action: { type: "tool", tool: "linearPattern" } },
  { key: "c", action: { type: "tool", tool: "circularPattern" } },
  { key: "m", action: { type: "tool", tool: "mirror" } },
];

export const SKETCH_KEYS: KeyBinding[] = [
  { key: "v", action: { type: "tool", tool: "select" } },
  { key: "l", action: { type: "tool", tool: "line" } },
  { key: "r", action: { type: "tool", tool: "rect" } },
  // W2-B: exact-match shift chord — plain `r` above still resolves to `rect`
  // because `resolveBinding` compares `Boolean(b.shift) === shift`.
  { key: "r", shift: true, action: { type: "tool", tool: "centerRect" } },
  { key: "c", action: { type: "tool", tool: "circle" } },
  { key: "o", action: { type: "tool", tool: "ellipse" } },
  { key: "a", action: { type: "tool", tool: "arc" } },
  { key: "g", action: { type: "tool", tool: "polygon" } },
  { key: "s", action: { type: "tool", tool: "slot" } },
  { key: "p", action: { type: "tool", tool: "point" } },
  { key: "d", action: { type: "tool", tool: "dimension" } },
  { key: "t", action: { type: "tool", tool: "trim" } },
  { key: "m", action: { type: "tool", tool: "mirror" } },
  // X flips construction geometry (W1-B): with a sketch selection it flips those
  // entities, with none it toggles the sticky construction draw mode. Sketch-scoped
  // only — the action is NOT a `tool`, so the cross-mode fallback never leaks it
  // into model mode (where X is still free).
  { key: "x", action: { type: "toggleConstruction" } },
  // Delete/Backspace remove the current sketch selection. Sketch-scoped only;
  // in model mode they fall through (no binding). The handler additionally lets
  // the key fall through when nothing is selected (see useShortcuts).
  { key: "Delete", action: { type: "deleteSketchSelection" } },
  { key: "Backspace", action: { type: "deleteSketchSelection" } },
];

export const GLOBAL_KEYS: KeyBinding[] = [
  { key: "Escape", action: { type: "cancel" } },
  { key: "Enter", action: { type: "finishSketch" } },
  { key: "h", action: { type: "home" } },
  { key: "f", shift: true, action: { type: "zoomFit" } },
];

export function modeKeys(mode: EditorMode): KeyBinding[] {
  return mode === "sketch" ? SKETCH_KEYS : MODEL_KEYS;
}

/**
 * Resolve a raw key + shift + mode to an action. Mode bindings win over global
 * ones so tool letters take precedence. Returns null when nothing matches.
 *
 * AUTO-MODE cross-mode fallback: a key bound ONLY in the other mode's table
 * resolves to its *tool* action — the dispatcher (`activateTool`) performs the
 * mode switch (L in model mode starts a sketch with Line; E in sketch mode
 * finishes and arms Extrude). Shared letters never leak: the current mode's
 * table already claimed them (R = rect in sketch, revolve in model). Fallback
 * is restricted to `tool` actions, so sketch-only Delete/Backspace semantics
 * can never fire in model mode.
 */
export function resolveBinding(
  key: string,
  shift: boolean,
  mode: EditorMode,
): ShortcutAction | null {
  const norm = key.length === 1 ? key.toLowerCase() : key;
  const same = (b: KeyBinding) => b.key === norm && Boolean(b.shift) === shift;
  const hit = [...modeKeys(mode), ...GLOBAL_KEYS].find(same);
  if (hit) return hit.action;
  const cross = modeKeys(mode === "sketch" ? "model" : "sketch").find(same);
  return cross && cross.action.type === "tool" ? cross.action : null;
}
