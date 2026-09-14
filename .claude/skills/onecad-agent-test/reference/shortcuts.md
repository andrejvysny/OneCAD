# OneCAD keymap (source: `src/modules/modeling/bindings.ts`, `src/shortcuts/keymap.ts`, `src/shortcuts/useShortcuts.ts`, `src-tauri/src/menu.rs`)

## Global
`Escape` cancel ladder (tool → selection → sketch plane-pick) · `Enter` finish sketch / confirm tool ·
`H` home view · `⇧F` zoom to fit · `⌘S` save · `⌘O` open · `⌘W` close · `⌘K` command palette ·
`⌘Z` undo · `⌘Y` redo (undo/redo are suppressed while an input is focused; the others are not).
Native menu accelerators: `⌘N` new, `⌘O`, `⌘S`, `⇧⌘S` save as, `⌘Z`, `⇧⌘Z`.

## Model mode
V select · S new sketch · D datum plane · E extrude · R revolve · F fillet/chamfer · B combine ·
K shell · ⇧O offset face · P linear pattern · C circular pattern · M mirror · T move · ⇧H hole ·
⇧G gear · ? measure (tooltip shows "?"; the binding is the shifted slash key) · ⇧I isolate · ⇧X section view.

## Sketch mode
V select · L line · R rectangle · ⇧R center rectangle · C circle · O ellipse · A arc · ⇧A 3-point arc ·
G polygon · S slot · P point · D dimension · T trim · ⇧T extend · M mirror · F 2D fillet · ⇧O offset ·
J project edges · X construction toggle · Delete/Backspace delete selection ·
constraints ⇧H horizontal, ⇧V vertical, ⇧C coincident, ⇧E equal, ⇧P parallel, ⇧M midpoint.

## Traps
- **Cross-mode fallback.** A letter bound only in the *other* mode still fires: pressing `E`
  inside a sketch **finishes the sketch and arms Extrude** (`keymap.ts` `resolveBinding`). Finish
  with `Enter` deliberately, then press `E`. Measure (`⇧?`) is exempt.
- Plain letters are ignored while an editable element has focus; `keyboard_type_text` refuses
  non-editable focus unless `allowShortcuts:true`, precisely so typed text never becomes tool
  presses.
- `Escape` in plane-pick phase discards the pending sketch (one undo step is NOT created — the
  sketch never existed). `Cancel` in the chrome bar discards an entered sketch.
- `⌘W` triggers the app's close-confirm flow; the harness stops the app via `session_stop`, not `⌘W`.
