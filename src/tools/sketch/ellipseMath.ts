/*
 * ellipseMath — PURE parametric-ellipse geometry (W3 P3).
 *
 * ONE source of ellipse math for every consumer: the tool FSM (preview +
 * commit), the renderer (`entityPolyline`), the snap/hit lane
 * (`nearestOnCurve`), marquee selection, and the mock region detector. Keeping
 * it here is what makes "the preview is exactly what commits" checkable by
 * data — every surface samples the SAME parametrization.
 *
 * ── Parametrization ──────────────────────────────────────────────────────────
 * With centre `c`, semi-major `a` (along the rotated local +X), semi-minor `b`
 * (along local +Y) and rotation `θ` (radians CCW from plane +X):
 *
 *   P(t) = c + a·cos t · (cos θ, sin θ) + b·sin t · (−sin θ, cos θ)
 *
 * `t` is the ECCENTRIC angle, NOT the polar angle — it is a parametrization, so
 * uniform sampling in `t` is denser near the minor-axis ends. That is fine for
 * every consumer here (rendering, hit tolerance, region fill).
 *
 * ── Normalization (a ≥ b), the legacy rule ───────────────────────────────────
 * The FRONTEND pre-normalizes so `minorR > majorR` is never authored or sent.
 * Ported from the oracle `OneCAD-CPP/src/core/sketch/tools/EllipseTool.cpp:88-99`
 * (and its identical preview copy at :151-169): when the computed minor exceeds
 * the major, SWAP the two radii and add π/2 to the rotation — the same ellipse,
 * re-labelled. The oracle then folds the rotation into (−π, π]; we fold into
 * [0, 2π) instead (the domain agreed with the backend for this WP) and we fold
 * ALWAYS, not only after a swap, so `rotation` has exactly one canonical value
 * per drawn ellipse regardless of which click path produced it. Both windows
 * describe identical geometry — cos/sin are 2π-periodic.
 */

/** Polyline/sample resolution for an ellipse (renderer, hit-test, marquee, fill).
 *  Matches the worker's own sampling density for ellipse boundaries. */
export const ELLIPSE_SEGMENTS = 72;

/** A resolved ellipse in plane (u,v) coords. `rotation` is radians in [0, 2π). */
export interface EllipseParams {
  center: [number, number];
  majorR: number;
  minorR: number;
  rotation: number;
}

/** Entity-shaped input (`SketchEntity` or a draft's field subset). */
interface EllipseLike {
  type: string;
  center?: [number, number];
  majorR?: number;
  minorR?: number;
  rotation?: number;
}

/** Fold an angle into [0, 2π). */
export function wrapTwoPi(a: number): number {
  const TWO_PI = Math.PI * 2;
  const r = a % TWO_PI;
  return r < 0 ? r + TWO_PI : r;
}

/**
 * The legacy major≥minor normalization (see the module header). Returns the
 * canonical `(majorR, minorR, rotation)` triple for the requested radii, with
 * `rotation` folded into [0, 2π).
 */
export function normalizeEllipse(
  majorR: number,
  minorR: number,
  rotation: number,
): { majorR: number; minorR: number; rotation: number } {
  if (minorR > majorR) {
    return { majorR: minorR, minorR: majorR, rotation: wrapTwoPi(rotation + Math.PI / 2) };
  }
  return { majorR, minorR, rotation: wrapTwoPi(rotation) };
}

/** Resolve an entity/draft to `EllipseParams`, or null when it is not a
 *  fully-populated Ellipse. Radii are NOT re-normalized here — everything that
 *  reaches this point has already been normalized by the tool. */
export function ellipseParams(e: EllipseLike): EllipseParams | null {
  if (e.type !== "Ellipse") return null;
  if (!e.center || e.majorR === undefined || e.minorR === undefined) return null;
  return { center: e.center, majorR: e.majorR, minorR: e.minorR, rotation: e.rotation ?? 0 };
}

/** Point on the ellipse at eccentric angle `t` (radians). */
export function ellipsePointAt(e: EllipseParams, t: number): [number, number] {
  const cos = Math.cos(e.rotation);
  const sin = Math.sin(e.rotation);
  const x = e.majorR * Math.cos(t);
  const y = e.minorR * Math.sin(t);
  return [e.center[0] + x * cos - y * sin, e.center[1] + x * sin + y * cos];
}

/**
 * Sample the ellipse into a CLOSED ring of `segments` points (the first point is
 * NOT repeated at the end — callers close with `% length`).
 */
export function sampleEllipse(e: EllipseParams, segments = ELLIPSE_SEGMENTS): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  for (let i = 0; i < segments; i++) out.push(ellipsePointAt(e, (i / segments) * Math.PI * 2));
  return out;
}

/**
 * Axis-aligned half-extents of the ROTATED ellipse (its exact bounding box is
 * `center ± (ex, ey)`):
 *   ex = √((a·cos θ)² + (b·sin θ)²),  ey = √((a·sin θ)² + (b·cos θ)²)
 * (the support function of the ellipse along +X / +Y — exact, not a sampled
 * approximation, so a window-marquee bbox test is tight at any rotation).
 */
export function ellipseExtents(e: EllipseParams): { ex: number; ey: number } {
  const cos = Math.cos(e.rotation);
  const sin = Math.sin(e.rotation);
  return {
    ex: Math.hypot(e.majorR * cos, e.minorR * sin),
    ey: Math.hypot(e.majorR * sin, e.minorR * cos),
  };
}

/** Local ternary-search refinement window (iterations shrink the bracket by
 *  (2/3)^N — 40 iterations ⇒ ~1e-7 of one sample interval, far under any
 *  on-screen pick tolerance). */
const REFINE_ITERATIONS = 40;

/**
 * Nearest point ON the ellipse to `p`: a coarse `segments`-point scan for the
 * best eccentric angle, then a ternary search over the bracketing interval
 * (squared distance is unimodal there once the coarse minimum is isolated).
 *
 * Sampling — rather than solving the quartic — is deliberate: it is the same
 * approach the worker takes for ellipse boundaries, it degrades gracefully for
 * near-degenerate radii, and it keeps this module free of root-finding.
 */
export function nearestOnEllipse(
  p: { x: number; y: number },
  e: EllipseParams,
  segments = ELLIPSE_SEGMENTS,
): { x: number; y: number } {
  const d2 = (t: number): number => {
    const [x, y] = ellipsePointAt(e, t);
    return (x - p.x) * (x - p.x) + (y - p.y) * (y - p.y);
  };
  const step = (Math.PI * 2) / segments;
  let bestT = 0;
  let bestD = Infinity;
  for (let i = 0; i < segments; i++) {
    const t = i * step;
    const d = d2(t);
    if (d < bestD) {
      bestD = d;
      bestT = t;
    }
  }
  // Ternary search inside [bestT − step, bestT + step].
  let lo = bestT - step;
  let hi = bestT + step;
  for (let k = 0; k < REFINE_ITERATIONS; k++) {
    const m1 = lo + (hi - lo) / 3;
    const m2 = hi - (hi - lo) / 3;
    if (d2(m1) <= d2(m2)) hi = m2;
    else lo = m1;
  }
  const [x, y] = ellipsePointAt(e, (lo + hi) / 2);
  return { x, y };
}
