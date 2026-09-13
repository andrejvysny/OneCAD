/*
 * ViewportMetrics — the ONE immutable snapshot of viewport size and display
 * scale (VP-HARDENING VP03, spec §7).
 *
 * Before this module every consumer re-derived its own numbers: `resize()` read
 * `clientWidth` and clamped `devicePixelRatio` inline, `renderFrame()` did the
 * same again a few hundred lines away, and the Picker did it a third time in
 * another file. Three readings of the same two values, taken at three different
 * moments, is how a fat line and its pick radius end up disagreeing.
 *
 * The rule is now: ONE `ViewportMetrics` field on the engine, written only by
 * `resize()` (a CSS-box change) and the DPR watcher / in-frame `syncDpr()` (a
 * display change with no CSS-box change). Everything else READS it.
 *
 * UNITS, and they are not interchangeable:
 *   - `cssWidth`/`cssHeight`   CSS (logical) pixels. This is what a fat line's
 *                              `resolution` uniform and every screen-space pick
 *                              radius are measured in — see `screenLineStyle.ts`
 *                              for why that is a property of the installed
 *                              three.js, not a choice.
 *   - `bufferWidth`/`Height`   physical drawing-buffer pixels. Screenshot
 *                              readback and render-target allocation only.
 *   - `dpr`                    the CAPPED renderer pixel ratio (`MAX_DPR`), i.e.
 *                              exactly what `renderer.setPixelRatio` was given —
 *                              NOT the raw `window.devicePixelRatio`.
 */
import { MAX_DPR } from "./dpr";

export interface ViewportMetrics {
  /** Bumped on every change; `1` for the first snapshot. Cheap identity for "did size/scale move". */
  readonly revision: number;
  /** Viewport width in CSS (logical) pixels. Never 0 — see {@link nextMetrics}. */
  readonly cssWidth: number;
  /** Viewport height in CSS (logical) pixels. Never 0 — see {@link nextMetrics}. */
  readonly cssHeight: number;
  /** Capped renderer pixel ratio (`min(rawDpr, MAX_DPR)`). Never 0. */
  readonly dpr: number;
  /** Drawing-buffer width in PHYSICAL pixels (`floor(cssWidth * dpr)`, as three sizes the canvas). */
  readonly bufferWidth: number;
  /** Drawing-buffer height in PHYSICAL pixels (`floor(cssHeight * dpr)`). */
  readonly bufferHeight: number;
  /** Bumped by the engine whenever the camera's projection MODE changes. */
  readonly projectionRevision: number;
}

export interface ViewportMetricsInput {
  /** Measured CSS width; a non-finite or non-positive value is clamped to 1. */
  readonly cssWidth: number;
  /** Measured CSS height; a non-finite or non-positive value is clamped to 1. */
  readonly cssHeight: number;
  /** RAW `window.devicePixelRatio`; capping to `MAX_DPR` happens here, once. */
  readonly rawDpr: number;
  /** The engine's current projection-mode revision. */
  readonly projectionRevision: number;
}

/**
 * A positive, finite dimension. A detached or unmeasured container reports 0,
 * and a 0 anywhere in this snapshot silently disables things rather than
 * failing: a `resolution` of (_, 0) divides a fat line's width by zero, and a
 * 0-height viewport divides the world-per-pixel metric by zero. Clamping to 1
 * keeps every downstream division finite and the frame visibly degenerate
 * instead of NaN.
 */
function positiveOr1(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 1;
}

/**
 * The next metrics snapshot, or `prev` VERBATIM when nothing moved.
 *
 * Returning the same object on a no-op is the point: callers compare by
 * identity (`next === prev`) to decide whether to touch the renderer at all,
 * so an in-frame DPR recheck on an unchanged display costs one comparison.
 */
export function nextMetrics(
  prev: ViewportMetrics | null,
  input: ViewportMetricsInput,
): ViewportMetrics {
  const cssWidth = positiveOr1(input.cssWidth);
  const cssHeight = positiveOr1(input.cssHeight);
  const dpr = Math.min(positiveOr1(input.rawDpr), MAX_DPR);
  const { projectionRevision } = input;

  if (
    prev &&
    prev.cssWidth === cssWidth &&
    prev.cssHeight === cssHeight &&
    prev.dpr === dpr &&
    prev.projectionRevision === projectionRevision
  ) {
    return prev;
  }

  return Object.freeze({
    revision: (prev?.revision ?? 0) + 1,
    cssWidth,
    cssHeight,
    dpr,
    // `floor`, matching `WebGLRenderer.setSize` (`canvas.width = Math.floor(width * _pixelRatio)`).
    bufferWidth: Math.floor(cssWidth * dpr),
    bufferHeight: Math.floor(cssHeight * dpr),
    projectionRevision,
  });
}
