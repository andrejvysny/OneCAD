/*
 * Modeling key bindings — the SINGLE source of truth for the three keymap tables.
 *
 * `shortcuts/keymap.ts` derives `MODEL_KEYS` / `SKETCH_KEYS` / `GLOBAL_KEYS` from
 * this table and keeps the resolution rules (mode precedence, exact-chord match,
 * the cross-mode fallback and its opt-out). Ownership of WHICH keys exist belongs
 * to the module that owns the tools; ownership of HOW a key resolves stays with
 * the shortcut layer.
 *
 * The collision reasoning that used to live in `keymap.ts` is preserved per entry
 * — it is the part that is expensive to rediscover.
 */
import type { ShortcutAction } from "@/shortcuts/keymap";
import type { Tool } from "@/stores/toolStore";

export type BindingScope = "model" | "sketch" | "global";

export interface ModelingBindingDescriptor {
  /** Single printable key (compared case-insensitively) or a named key. */
  readonly key: string;
  readonly shift?: boolean;
  readonly action: ShortcutAction;
  readonly scope: BindingScope;
}

const tool = (t: Tool): ShortcutAction => ({ type: "tool", tool: t });

export const MODELING_BINDINGS: readonly ModelingBindingDescriptor[] = [
  // ── model ──────────────────────────────────────────────────────────────────
  { key: "v", action: tool("select"), scope: "model" },
  { key: "s", action: { type: "enterSketch" }, scope: "model" },
  // D = Datum plane. Free in the model table; sketch mode's own `d` (Dimension)
  // shadows it there, so the two never collide.
  { key: "d", action: tool("datum"), scope: "model" },
  { key: "e", action: tool("extrude"), scope: "model" },
  { key: "r", action: tool("revolve"), scope: "model" },
  { key: "f", action: tool("fillet"), scope: "model" },
  { key: "b", action: tool("boolean"), scope: "model" },
  // K/P/C/M are free in model mode; C/M also serve sketch tools in sketch mode,
  // resolved by mode exactly as R is (revolve vs rectangle).
  { key: "k", action: tool("shell"), scope: "model" },
  // OffsetFace — an exact shift chord.
  { key: "o", shift: true, action: tool("offsetFace"), scope: "model" },
  { key: "p", action: tool("linearPattern"), scope: "model" },
  { key: "c", action: tool("circularPattern"), scope: "model" },
  { key: "m", action: tool("mirror"), scope: "model" },
  // T = Move. Sketch mode's own `t` (Trim) shadows it there, exactly like D.
  // Claiming it here also stops model-mode `t` falling through cross-mode and
  // STARTING A SKETCH with Trim, which is what it did while the model table had
  // no `t` of its own.
  { key: "t", action: tool("transform"), scope: "model" },
  // Hole — an exact shift chord; plain `h` cannot be claimed here (it is Home).
  { key: "h", shift: true, action: tool("hole"), scope: "model" },
  // Measure. `?` (⇧/) is free in BOTH tables and reads as "what is this?".
  // Declared as an exact shift chord so a plain-key press can never claim it.
  // It is also in NO_CROSS_MODE — see `keymap.ts`.
  { key: "?", shift: true, action: tool("measure"), scope: "model" },
  // Isolate. MODEL-scoped on purpose and NOT a `tool` action, so the cross-mode
  // fallback can never fire it while drawing — isolating bodies mid-sketch would
  // hide the very geometry the sketch is being drawn against.
  { key: "i", shift: true, action: { type: "isolate" }, scope: "model" },

  // ── sketch ─────────────────────────────────────────────────────────────────
  { key: "v", action: tool("select"), scope: "sketch" },
  { key: "l", action: tool("line"), scope: "sketch" },
  { key: "r", action: tool("rect"), scope: "sketch" },
  // Exact-match shift chord — plain `r` above still resolves to `rect`, because
  // resolution compares `Boolean(binding.shift) === shift`.
  { key: "r", shift: true, action: tool("centerRect"), scope: "sketch" },
  { key: "c", action: tool("circle"), scope: "sketch" },
  { key: "o", action: tool("ellipse"), scope: "sketch" },
  { key: "a", action: tool("arc"), scope: "sketch" },
  // Exact shift chord (the ⇧R precedent), so plain `a` still resolves to the
  // centre-start-end arc.
  { key: "a", shift: true, action: tool("arc3p"), scope: "sketch" },
  { key: "g", action: tool("polygon"), scope: "sketch" },
  { key: "s", action: tool("slot"), scope: "sketch" },
  { key: "p", action: tool("point"), scope: "sketch" },
  { key: "d", action: tool("dimension"), scope: "sketch" },
  { key: "t", action: tool("trim"), scope: "sketch" },
  // Exact shift chord on Trim's own letter, so plain `t` still resolves to Trim
  // in sketch mode and to Transform in model mode.
  { key: "t", shift: true, action: tool("extend"), scope: "sketch" },
  { key: "m", action: tool("mirror"), scope: "sketch" },
  { key: "f", action: tool("sketchFillet"), scope: "sketch" },
  { key: "o", shift: true, action: tool("sketchOffset"), scope: "sketch" },
  // X flips construction geometry: with a sketch selection it flips those
  // entities, with none it toggles the sticky construction draw mode.
  // Sketch-scoped only — NOT a `tool` action, so the cross-mode fallback never
  // leaks it into model mode (where X is still free).
  { key: "x", action: { type: "toggleConstruction" }, scope: "sketch" },
  // Delete/Backspace remove the current sketch selection. Sketch-scoped only; in
  // model mode they fall through. The handler additionally lets the key fall
  // through when nothing is selected.
  { key: "Delete", action: { type: "deleteSketchSelection" }, scope: "sketch" },
  { key: "Backspace", action: { type: "deleteSketchSelection" }, scope: "sketch" },

  // ── global ─────────────────────────────────────────────────────────────────
  // Global in REACH, modeling in meaning. The two view-navigation chords that
  // used to sit here (`h` home, `⇧F` zoom-fit) belong to the host and moved to
  // `@/modules/shell/bindings` — see docs/ARCHITECTURE.md §7.
  { key: "Escape", action: { type: "cancel" }, scope: "global" },
  { key: "Enter", action: { type: "finishSketch" }, scope: "global" },
];

export function bindingsForScope(scope: BindingScope): readonly ModelingBindingDescriptor[] {
  return MODELING_BINDINGS.filter((b) => b.scope === scope);
}

/** The binding for a tool in its own scope, if it has one. */
export function bindingForTool(
  t: Tool,
  scope: BindingScope,
): ModelingBindingDescriptor | undefined {
  return MODELING_BINDINGS.find(
    (b) => b.scope === scope && b.action.type === "tool" && b.action.tool === t,
  );
}
