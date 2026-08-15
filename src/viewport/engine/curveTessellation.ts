/*
 * curveTessellation — how many segments a curve is drawn with (SNAP P4).
 *
 * A FIXED segment count is wrong in both directions. 64 segments on a circle
 * whose projected radius is 4000px leaves a 3px chord error — visibly a
 * polygon. The same 64 on a 6px arc is 58 wasted vertices and 58 wasted
 * `LineGeometry` slots per frame.
 *
 * The rule is a screen-space SAGITTA budget: pick the segment count that keeps
 * the maximum distance between the chord and the true arc under
 * {@link MAX_SAGITTA_PX}. For a circular arc of projected radius r subtending
 * θ per segment, the sagitta is `r(1 − cos(θ/2))`, so the largest θ that fits a
 * budget s is `2·acos(1 − s/r)`.
 *
 * ── Not a solver tolerance ───────────────────────────────────────────────────
 * This is a RENDERING budget and nothing else. Snap candidates, hit-testing and
 * the solver all work against the exact analytic curve; changing the segment
 * count must never move a snap point or a constraint.
 */
import { ellipseParams } from "@/tools/sketch/ellipseMath";
import type { SketchEntity } from "@/ipc/types";

/** Maximum chord-to-arc distance, in CSS pixels. */
export const MAX_SAGITTA_PX = 0.35;

/** Floor — below this a curve reads as a polygon however small it is. */
export const MIN_SEGMENTS = 8;
/** Floor for an ELLIPSE: uniform sampling in the eccentric angle is denser near
 *  the minor-axis ends, so a low count shows its seams earlier than a circle's. */
export const MIN_ELLIPSE_SEGMENTS = 24;
/** Hard cap. Reaching it is a legitimate state (an enormous curve at high zoom),
 *  not an error — the curve renders at the cap and a debug metric records it. */
export const MAX_SEGMENTS = 2048;

/**
 * Segment count for a circular sweep of `sweepRad` at projected radius `rPx`.
 *
 * A non-finite or non-positive radius (a degenerate curve, or no camera metric
 * yet) falls back to the floor rather than producing NaN.
 */
export function segmentsForArc(rPx: number, sweepRad: number, min = MIN_SEGMENTS): number {
  const sweep = Math.abs(sweepRad);
  if (!Number.isFinite(rPx) || !Number.isFinite(sweep) || sweep <= 0) return min;
  const r = Math.max(rPx, MAX_SAGITTA_PX);
  const thetaMax = 2 * Math.acos(clamp(1 - MAX_SAGITTA_PX / r, -1, 1));
  if (!(thetaMax > 0)) return MAX_SEGMENTS;
  return clamp(Math.ceil(sweep / thetaMax), min, MAX_SEGMENTS);
}

const clamp = (v: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, v));

/**
 * The segment count an entity should be drawn with, given how many CSS pixels
 * one plane unit currently projects to.
 *
 * `pxPerUnit` is a SCALAR here on purpose: the tessellation budget only needs
 * the worst-case (largest) projected scale, and using the anisotropic metric's
 * larger axis is the conservative choice — it can only over-tessellate.
 * Non-curve entities return 0 (they have no sweep to divide).
 */
export function segmentsForEntity(e: SketchEntity, pxPerUnit: number): number {
  const scale = Number.isFinite(pxPerUnit) && pxPerUnit > 0 ? pxPerUnit : 1;
  if (e.type === "Circle" && e.radius !== undefined) {
    return segmentsForArc(e.radius * scale, Math.PI * 2);
  }
  if (e.type === "Arc" && e.radius !== undefined && e.center && e.start && e.end) {
    const a0 = Math.atan2(e.start[1] - e.center[1], e.start[0] - e.center[0]);
    let a1 = Math.atan2(e.end[1] - e.center[1], e.end[0] - e.center[0]);
    while (a1 <= a0) a1 += Math.PI * 2;
    return segmentsForArc(e.radius * scale, a1 - a0);
  }
  if (e.type === "Ellipse") {
    const p = ellipseParams(e);
    if (!p) return MIN_ELLIPSE_SEGMENTS;
    // The LARGER semiaxis is the conservative radius: the tightest curvature on
    // an ellipse sits at the major-axis ends, where the osculating radius is
    // b²/a — but sampling is uniform in the eccentric angle, so the segment
    // count that satisfies the largest radius satisfies the whole curve.
    return segmentsForArc(Math.max(p.majorR, p.minorR) * scale, Math.PI * 2, MIN_ELLIPSE_SEGMENTS);
  }
  return 0;
}

/**
 * Should the geometry be rebuilt for a new segment count?
 *
 * Rebuilding a `LineGeometry` allocates and re-uploads a buffer, so following
 * every one-segment change during a zoom would churn every curve every frame
 * for a difference nobody can see. A 25% band is wide enough that a continuous
 * zoom rebuilds a handful of times rather than continuously, and narrow enough
 * that the sagitta never drifts far past its budget.
 */
export const SEGMENT_REBUILD_RATIO = 0.25;

export function needsRetessellation(current: number, wanted: number): boolean {
  if (current <= 0) return wanted > 0;
  return Math.abs(wanted - current) / current >= SEGMENT_REBUILD_RATIO;
}
