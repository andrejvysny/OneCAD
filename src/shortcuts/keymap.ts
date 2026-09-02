/*
 * Keyboard bindings (F-WP3) — data-driven, mode-scoped.
 *
 * Model:  V select · S new-sketch (enters sketch mode) · D datum plane · E extrude
 *         · R revolve · F fillet · B combine/boolean · K shell · ⇧O offset face
 *         · T move/rotate body · ⇧I isolate selection · ⇧X section view
 * Sketch: V select · L line · R rectangle · ⇧R center-rectangle · C circle
 *         · O ellipse · ⇧O offset · A arc · ⇧A 3-point arc · G polygon · S slot
 *         · P point
 *         · D dimension · T trim · ⇧T extend · M mirror · F fillet
 *         · X construction (flip selection / sticky mode)
 *         · ⇧H/⇧V/⇧C/⇧E/⇧P/⇧M apply Horizontal/Vertical/Coincident/Equal/
 *           Parallel/Midpoint to the selection
 * Global: Esc cancel-ladder · Enter finish-sketch (sketch mode) · H home (stub)
 *         · Shift+F zoom-to-fit (frames the SELECTION when it names bodies)
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
 *   - `T` in SKETCH mode is the Trim tool; model `T` = Move/rotate a body (WP-B
 *     W1). Mode-resolved exactly like `R` and `D`.
 *   - `D` in SKETCH mode is the Dimension tool; model `D` = Datum plane (DATUM W1).
 *     Mode-resolved exactly like `R` (revolve vs rectangle) — sketch mode's own
 *     table claims `d` first, so the cross-mode fallback never reaches the datum
 *     tool while drawing. Pressing D in model mode is unambiguous.
 *   - `O` (Ellipse, W3 P3) was likewise free in BOTH tables, so sketch owns it and
 *     model mode reaches it cross-mode. Deliberately NOT `E`: that letter is the
 *     model Extrude binding, i.e. the cross-mode "finish this sketch and extrude"
 *     handoff — rebinding it in sketch mode would shadow the handoff.
 *   - `F` in SKETCH mode is the 2D Fillet tool (WP-C T2b); model `F` = the 3D
 *     Fillet/Chamfer tool. SAME operation, one dimension apart, so one letter
 *     means one thing in both modes — mode-resolved exactly like `T`/`D`/`R`.
 *     This deliberately RETIRES the old cross-mode `f`-while-sketching → model
 *     fillet fallback (which finished the sketch): a 2D fillet is what `F` means
 *     while you are drawing, and the toolbar click still reaches the 3D one.
 *   - `⇧H` (Hole, WP-C T3) is an exact shift chord. Plain `h` is the GLOBAL
 *     `home` binding, and mode keys WIN over global (`resolveBinding` searches the
 *     mode table first), so a model-mode `h` would silently shadow home — which
 *     `e2e/filletChamfer.spec.ts` explicitly asserts must not happen. `H` is the
 *     only mnemonic letter for "hole", so it takes the chord rather than one of the
 *     mnemonic-free leftovers (`w`/`j`/`n`/`q`), exactly as Offset took `⇧O` rather
 *     than surrender `O` to Ellipse. Free in the sketch table, so sketch mode
 *     reaches it through the ordinary cross-mode fallback.
 *   - `⇧A` (3-point arc, SP-4) is an exact shift chord on the letter its own
 *     tool family already owns — the two arcs are one shape drawn two ways, so
 *     they share a letter the way `⇧R` shares `R` with the rectangle. Free in
 *     BOTH tables, so model mode reaches it through the ordinary cross-mode
 *     fallback (it is a sketch draw tool, so that starts a sketch with it armed).
 *     Plain `a` is deliberately NOT in this table: `SketchController` shadows it
 *     in CAPTURE phase to toggle the line tool's tangent-arc mode, but ONLY while
 *     a chain is armed on a known tangent — outside that it falls through here
 *     untouched, and the shadow requires NO shift so `⇧A` always reaches arc3p.
 *   - `⇧T` (Extend, SP-4) is an exact shift chord on Trim's own letter, same
 *     reasoning as `⇧A`: Extend IS Trim inverted, so the pair shares a letter.
 *     Plain `t` is untouched in BOTH modes (sketch Trim, model Transform). `⇧T`
 *     is free in the model table, so model mode reaches Extend through the
 *     ordinary cross-mode fallback (a sketch edit tool ⇒ it starts a sketch).
 *   - `⇧O` (Offset, WP-C T2b) is an exact shift chord, like `⇧R`/`⇧?`/`⇧I`, so
 *     plain `o` still resolves to Ellipse. Offset's conventional key IS `O` in
 *     other CAD; Ellipse already holds it here, so Offset takes the chord rather
 *     than a mnemonic-free letter.
 *   - `⇧O` is now claimed in BOTH tables: sketch ⇧O is the 2D Offset tool, model
 *     ⇧O is the 3D Offset face (SCHEMA §7.3). SAME operation, one dimension
 *     apart, so one chord means one thing in both modes — mode-resolved exactly
 *     like `F` (2D vs 3D fillet), `T`, `D` and `R`. This deliberately RETIRES the
 *     old cross-mode `⇧O`-in-model-mode → sketch-offset fallback (which STARTED A
 *     SKETCH): with a face selected, ⇧O in model mode now means "offset that
 *     face", which is what a modeller pressing it there is asking for.
 *   - `⇧X` (section view) is an exact shift chord in the MODEL table. Sketch mode
 *     owns plain `x` (construction geometry), and that is a `toggleConstruction`
 *     action, not a `tool`, so neither key can cross the mode boundary: ⇧X while
 *     drawing does nothing rather than slicing the model the sketch sits on.
 *   - `⇧H`/`⇧V`/`⇧C`/`⇧E`/`⇧P`/`⇧M` (constraint apply, plan item 6 — the
 *     Shapr3D Shift+letter convention) are sketch-scoped `applyConstraint`
 *     actions, NOT `tool` actions, so the cross-mode fallback can never leak
 *     them into model mode. Only `⇧H` overlaps an existing chord: sketch mode
 *     used to reach the model Hole tool through the cross-mode fallback, which
 *     it no longer does — mid-sketch, ⇧H means "make this line horizontal",
 *     and model `⇧H` = Hole is untouched (mode tables are searched first).
 *     Perpendicular and Tangent are deliberately NOT bound: ⇧T is already
 *     Extend and neither has a free mnemonic (audit Part C item 6).
 *
 * AUTO-MODE: a key bound only in the OTHER mode resolves cross-mode (tool
 * actions only) so shortcuts drive the automatic mode switch — see
 * `resolveBinding` and `tools/activateTool.ts`.
 */
import type { SketchConstraintType } from "@/ipc/types";
import type { EditorMode, Tool } from "@/stores/toolStore";
import { bindingsForScope, type BindingScope } from "@/modules/modeling/bindings";
import { SHELL_GLOBAL_BINDINGS } from "@/modules/shell/bindings";

export type ShortcutAction =
  | { type: "tool"; tool: Tool }
  | { type: "enterSketch" }
  | { type: "finishSketch" }
  | { type: "deleteSketchSelection" }
  | { type: "toggleConstruction" }
  /** Apply one constraint kind to the current sketch selection (⇧H/⇧V/…). */
  | { type: "applyConstraint"; constraint: SketchConstraintType }
  | { type: "cancel" }
  | { type: "zoomFit" }
  | { type: "isolate" }
  | { type: "toggleSection" }
  | { type: "home" };

export interface KeyBinding {
  /** Single printable key (compared case-insensitively) or a named key. */
  key: string;
  shift?: boolean;
  action: ShortcutAction;
}

/*
 * The three tables are DERIVED from `@/modules/modeling/bindings` — the module
 * that owns the tools owns which keys exist. What stays here is how a key
 * RESOLVES: mode precedence, exact-chord matching, and the cross-mode fallback
 * with its opt-out. The exported shapes are unchanged, so every call site and
 * every test keeps working.
 */
function tableFor(scope: BindingScope): KeyBinding[] {
  return bindingsForScope(scope).map(({ key, shift, action }) =>
    shift === undefined ? { key, action } : { key, shift, action },
  );
}

export const MODEL_KEYS: KeyBinding[] = tableFor("model");

export const SKETCH_KEYS: KeyBinding[] = tableFor("sketch");

/*
 * Two owners, one table. Modeling's globals are global in reach but modeling in
 * meaning (cancel, finish sketch); the shell's are view navigation, which
 * docs/ARCHITECTURE.md §7 puts with the host. Concatenated in that order, which
 * is the shipped order.
 *
 * Still a STATIC merge, not a read of the command registry: `useShortcuts`
 * installs a window keydown handler outside React, so a registry-driven keymap
 * is its own change (TODO.md § flagged seams).
 */
export const GLOBAL_KEYS: KeyBinding[] = [...tableFor("global"), ...SHELL_GLOBAL_BINDINGS];

export function modeKeys(mode: EditorMode): KeyBinding[] {
  return mode === "sketch" ? SKETCH_KEYS : MODEL_KEYS;
}

/**
 * Tools whose key is INERT outside their own mode — the opt-out from the
 * cross-mode fallback below.
 *
 * The fallback is a real convenience for AUTHORING tools: pressing E while
 * drawing means "I am done, extrude this", and `activateTool` finishing the
 * sketch is precisely what the user asked for. It is NOT appropriate for a
 * READ-ONLY tool. `?` in sketch mode would otherwise resolve cross-mode to
 * Measure and `finishSketchToTool` would END the user's sketch — an irreversible
 * side effect (it squashes the session into one undo step) triggered by a
 * keystroke that promised only to look at something.
 *
 * A toolbar CLICK on Measure still finishes the sketch, consistent with every
 * other model tool: that is an explicit, visible intent. Only the ambient
 * keystroke is suppressed. Members must also be listed in `activateTool`'s
 * MODEL_ONLY (or SKETCH_ONLY) set for that click path to behave.
 */
const NO_CROSS_MODE: ReadonlySet<Tool> = new Set<Tool>(["measure"]);

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
 * can never fire in model mode — and further restricted by `NO_CROSS_MODE`,
 * which keeps a read-only tool's key from silently finishing a sketch.
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
  if (!cross || cross.action.type !== "tool") return null;
  return NO_CROSS_MODE.has(cross.action.tool) ? null : cross.action;
}
