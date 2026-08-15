/*
 * dpr — the ONE device-pixel-ratio cap and its CSS→device conversion (SNAP P4).
 *
 * Fat lines (`LineMaterial`) measure `linewidth` in DEVICE pixels, in the same
 * units as the `resolution` uniform. Two things therefore have to agree: the
 * cap the renderer's `setPixelRatio` uses, and the cap a width conversion uses.
 * They used to be two constants in two files kept "in step by hand", and the
 * width was computed ONCE at module evaluation — so a window dragged to a
 * different-DPR display kept the old width forever.
 *
 * Both problems are the same problem: nobody owned the number. This module does.
 */

/**
 * Renderer pixel-ratio cap. Above 2× the extra samples are invisible and the
 * fill cost is quadratic, so every consumer clamps here.
 */
export const MAX_DPR = 2;

/** The current capped device pixel ratio (1 outside a browser). */
export function currentDpr(): number {
  const raw = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
  return Math.min(raw, MAX_DPR);
}

/**
 * CSS px → device px for a fat-line `linewidth`.
 *
 * Takes the ratio EXPLICITLY so the caller can recompute on a DPR change
 * instead of baking one value in at import time. Omitting it reads the live
 * value, which is right for a one-off call and wrong for a module constant.
 */
export function cssToDevice(cssPx: number, dpr: number = currentDpr()): number {
  return cssPx * dpr;
}
