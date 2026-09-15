/*
 * handleProjection — where a world axis actually points ON SCREEN, and how
 * trustworthy that answer is.
 *
 * PURE matrix math on number triples and a flat 16-number matrix, so it unit-
 * tests with no THREE and no WebGL, exactly like `transformDrag` and
 * `depthProjection`. The engine supplies `projectionMatrix * matrixWorldInverse`;
 * nothing here knows what a camera is.
 *
 * WHY THIS EXISTS. `DragHandle.orient()` derives the arrow's screen angle by
 * rotating the world axis into CAMERA space and taking `atan2(y, x)` of the
 * result. That is exact for an ORTHOGRAPHIC camera, where projection is linear
 * and every world direction maps to one screen direction no matter where it
 * sits. Under PERSPECTIVE it is not: the projection divides by `w`, so the
 * screen direction of a world axis depends on the ANCHOR's position in the
 * frustum as well as its orientation. An axis anchored near the edge of a wide-
 * FOV viewport draws along one direction and drags along another, and the two
 * diverge further the further the anchor sits from the optical axis.
 *
 * The fix is the real derivative. For anchor `P`, unit direction `a` and
 * view-projection `M`, with `c = M·[P,1]` and `d = M·[a,0]`, the clip-space
 * point is `c` and moving one world unit along `a` adds `d`. NDC is `c.xy/c.w`,
 * so by the quotient rule the NDC derivative along `a` is
 *
 *     dNDC = (d.xy · c.w − c.xy · d.w) / c.w²
 *
 * and CSS pixels follow by the viewport half-extents, with Y flipped because
 * NDC +Y is up and CSS +Y is down.
 *
 * CONDITIONING, NOT JUST DIRECTION. The same numbers answer "should this axis be
 * dragged at all". `pxPerWorld · worldPerPixel` is dimensionless and equals
 * |sin∠(axis, view ray)| to first order: 1 for an axis square-on to the screen,
 * 0 for one pointing straight at the camera. That single scalar is what selects
 * the world-axis mapping or an honest screen-space proxy, and it is chosen ONCE
 * at grab so a gesture can never swap strategies underneath the user.
 *
 * DEGENERATE VIEWS ARE REFUSED, NOT APPROXIMATED — `null` is the only failure
 * signal, as in `transformDrag`. A caller that gets `null` shows a proxy or
 * holds its previous value; it never renders a fabricated direction.
 */
import type { Vec3 } from "./depthProjection";

/**
 * A view-projection matrix in THREE's `Matrix4.elements` order — COLUMN-major,
 * so `m[0], m[4], m[8], m[12]` is the first ROW.
 */
export type Mat4 = readonly number[];

/**
 * Below this clip `w` the anchor is on or behind the camera plane and the
 * perspective divide is meaningless. Matches the spirit of `screenScale`'s
 * `MIN_DEPTH`: refuse rather than divide by ~0.
 */
const MIN_CLIP_W = 1e-6;

/**
 * Conditioning floor below which a world-axis drag is not worth offering.
 *
 * Deliberately ABOVE `transformDrag.MIN_VIEW_SIN` (0.05 ≈ 3°). That constant is
 * the point where a projection becomes numerically destructive and must refuse;
 * this one is the point where a handle should stop pretending and hand the user
 * a labelled screen proxy instead. A control that is merely *hard* to use is a
 * worse outcome than one that honestly changes mapping, so the handle gives up
 * first — 0.08 ≈ 4.6°.
 */
export const MIN_AXIS_CONDITIONING = 0.08;

/** The projected behaviour of one world axis at one anchor, in CSS pixels. */
export interface AxisProjection {
  /**
   * CSS px travelled per world unit along the axis, as `[dx, dy]`. This is the
   * derivative, not a difference of two projected points, so it is exact at the
   * anchor rather than averaged over a step.
   */
  readonly derivative: readonly [number, number];
  /** `|derivative|` — CSS px per world unit. Always finite and > 0. */
  readonly pxPerWorld: number;
  /**
   * Unit screen direction the glyph must be drawn along. This is the SAME vector
   * the drag maps against, which is the whole point: one projected description
   * feeds both, so the arrow can never point somewhere the drag does not go.
   */
  readonly direction: readonly [number, number];
}

const finite3 = (v: Vec3): boolean =>
  Number.isFinite(v[0]) && Number.isFinite(v[1]) && Number.isFinite(v[2]);

/** Unit vector, or `null` for a non-finite or zero-length input (never NaN out). */
function unit(v: Vec3): Vec3 | null {
  if (!finite3(v)) return null;
  const l = Math.hypot(v[0], v[1], v[2]);
  if (!(l > 1e-12)) return null;
  return [v[0] / l, v[1] / l, v[2] / l];
}

/** `M · [x, y, z, w]`, returning only the components the divide needs. */
function transform(
  m: Mat4,
  x: number,
  y: number,
  z: number,
  w: number,
): { x: number; y: number; w: number } {
  return {
    x: m[0] * x + m[4] * y + m[8] * z + m[12] * w,
    y: m[1] * x + m[5] * y + m[9] * z + m[13] * w,
    w: m[3] * x + m[7] * y + m[11] * z + m[15] * w,
  };
}

/**
 * The exact CSS-pixel derivative of `dir` at `anchor`.
 *
 * `dir` need not arrive normalized — it is normalized here, so the result is
 * always "per world unit" regardless of what the caller passed.
 *
 * Returns `null` when the matrix or inputs are non-finite, the direction is
 * degenerate, the anchor is on or behind the camera plane, or the projected
 * motion is too small to carry a direction at all.
 */
export function projectAxis(
  viewProj: Mat4,
  anchor: Vec3,
  dir: Vec3,
  viewportWidth: number,
  viewportHeight: number,
): AxisProjection | null {
  if (viewProj.length < 16) return null;
  for (let i = 0; i < 16; i++) if (!Number.isFinite(viewProj[i])) return null;
  if (!finite3(anchor)) return null;
  if (!(viewportWidth > 0) || !(viewportHeight > 0)) return null;

  const a = unit(dir);
  if (!a) return null;

  const c = transform(viewProj, anchor[0], anchor[1], anchor[2], 1);
  // A direction is a vector, not a point: w = 0, so the translation column is
  // deliberately not applied.
  const d = transform(viewProj, a[0], a[1], a[2], 0);

  if (!(c.w > MIN_CLIP_W)) return null; // on or behind the camera plane
  if (!Number.isFinite(d.x) || !Number.isFinite(d.y) || !Number.isFinite(d.w)) return null;

  const invW2 = 1 / (c.w * c.w);
  const ndcDx = (d.x * c.w - c.x * d.w) * invW2;
  const ndcDy = (d.y * c.w - c.y * d.w) * invW2;

  // NDC spans [-1, 1] across the viewport, hence the HALF extents; CSS +Y runs
  // down while NDC +Y runs up, hence the negation on Y alone.
  const dx = (viewportWidth / 2) * ndcDx;
  const dy = -(viewportHeight / 2) * ndcDy;
  if (!Number.isFinite(dx) || !Number.isFinite(dy)) return null;

  const pxPerWorld = Math.hypot(dx, dy);
  if (!(pxPerWorld > 1e-9)) return null; // exactly end-on: no direction exists

  return {
    derivative: [dx, dy],
    pxPerWorld,
    direction: [dx / pxPerWorld, dy / pxPerWorld],
  };
}

/**
 * How square-on this axis is to the screen, in `[0, 1]`.
 *
 * `worldPerPixel` is the caller's own scale at the SAME anchor (the engine's
 * `screenScale.worldPerPixel`), so the product cancels units and leaves the
 * geometric term. Feeding a scale measured at a DIFFERENT point — the orbit
 * target, say — silently biases the answer, which is the same trap
 * `ViewportEngine.planePixelWorld` sets for anything off-pivot.
 */
export function axisConditioning(projection: AxisProjection, worldPerPixel: number): number {
  if (!Number.isFinite(worldPerPixel) || worldPerPixel <= 0) return 0;
  const c = projection.pxPerWorld * worldPerPixel;
  return Number.isFinite(c) ? Math.min(c, 1) : 0;
}

/**
 * Whether a world-axis drag is trustworthy here, or the caller should fall back
 * to a labelled screen-space proxy.
 *
 * Call this ONCE, at grab. Re-evaluating mid-gesture is what produces a handle
 * that changes gain under the user's hand halfway through a drag.
 */
export function axisIsWellConditioned(
  projection: AxisProjection,
  worldPerPixel: number,
  floor: number = MIN_AXIS_CONDITIONING,
): boolean {
  return axisConditioning(projection, worldPerPixel) >= floor;
}
