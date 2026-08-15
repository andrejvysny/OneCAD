/*
 * Shared stack geometry for the two sketch-mode top chrome rows
 * (`FloatingToolbar`, `SketchChromeBar`). Both used to hardcode their own
 * `top-Npx` independently — resizing one silently broke the other's
 * alignment (Sketcher UX cleanup, Track A3). `SKETCH_CHROME_TOP` is DERIVED
 * from `TOOLBAR_TOP` + `TOOLBAR_ROW_HEIGHT` so there is exactly one source
 * for the stack, not two hand-copied numbers.
 */

/** `FloatingToolbar`'s top offset — was Tailwind's literal `top-3` (0.75rem). */
export const TOOLBAR_TOP = 12;

/**
 * `FloatingToolbar`'s rendered height once fused with `SketchChromeBar`
 * below it: 34px `ToolButton` + 8px vertical padding (`p-1`) + 1px top
 * border. The bottom border is dropped in sketch mode (`border-b-0`) so the
 * two rows share one hairline instead of stacking two.
 */
export const TOOLBAR_ROW_HEIGHT = 43;

/** `SketchChromeBar`'s top offset — flush against `FloatingToolbar`'s row. */
export const SKETCH_CHROME_TOP = TOOLBAR_TOP + TOOLBAR_ROW_HEIGHT;
