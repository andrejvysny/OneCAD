/*
 * dpr — the ONE renderer pixel-ratio cap, and nothing else.
 *
 * This module used to also own a CSS→device width conversion, on the belief
 * that a fat line's `linewidth` is measured in DEVICE pixels. It is not: the
 * installed `LineSegments2.onBeforeRender` overwrites `resolution` from
 * `renderer.getViewport()`, which is LOGICAL pixels, so a device-pixel width
 * draws `dpr` times too wide (finding R01). Screen-space line widths, their
 * `resolution`, and every screen-space pick radius are CSS pixels and are never
 * multiplied by the ratio — that contract, with its citations into
 * `node_modules/three`, lives in `screenLineStyle.ts`.
 *
 * What is left here is genuinely per-device: the ratio the DRAWING BUFFER is
 * allocated at. `ViewportMetrics` is the snapshot that carries it; a display
 * change with no CSS resize reaches the engine through `dprWatcher.ts`.
 */

/**
 * Renderer pixel-ratio cap. Above 2× the extra samples are invisible and the
 * fill cost is quadratic, so every consumer clamps here.
 */
export const MAX_DPR = 2;

/**
 * The current CAPPED device pixel ratio (1 outside a browser).
 *
 * This is the buffer-allocation ratio — `renderer.setPixelRatio`'s argument and
 * `ViewportMetrics.dpr`. It is NOT a multiplier for any width, radius, or
 * resolution.
 */
export function currentDpr(): number {
  const raw = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
  return Math.min(raw, MAX_DPR);
}

/** The RAW, uncapped `window.devicePixelRatio` (1 outside a browser). */
export function rawDpr(): number {
  return typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
}
