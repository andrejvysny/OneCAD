/*
 * transformDrag — the pointer→placement projections behind the transform gizmo
 * (WP-B W2). PURE vector math on number triples, so it unit-tests with no THREE
 * and no WebGL, exactly like `depthProjection` (which owns the same closest-
 * approach idea for the extrude handle).
 *
 * Three gestures, three projections, all measured against the gizmo's CURRENT
 * origin (the frozen pivot plus the placement's translation — the gizmo rides
 * the body):
 *
 *   axis arrow  → closest approach between the pointer ray and the axis line,
 *                 reported as a signed distance from the origin;
 *   plane quad  → the ray's intersection with the plane through the origin,
 *                 reported as the world delta from the origin (its component
 *                 along the plane normal is zero by construction);
 *   ring        → the angle of that same plane hit inside a right-handed
 *                 in-plane frame, accumulated across frames so a sweep past the
 *                 ±180° seam keeps counting instead of jumping.
 *
 * DEGENERATE VIEWS ARE REFUSED, NOT APPROXIMATED. Every projection divides by a
 * quantity that vanishes as the view lines up with the axis / grazes the plane,
 * and the failure mode there is not "slightly wrong" — it is a value that flies
 * to ±∞ (or NaN) and yanks the body across the scene on one frame. Each entry
 * point returns `null` in that band and the caller holds the previous value, so
 * a bad camera angle makes the handle inert rather than destructive. Every
 * return path is finite-checked; `null` is the ONLY failure signal.
 */
import type { Vec3 } from "./depthProjection";

const DEG = 180 / Math.PI;

/**
 * How square-on the pointer ray must be before a projection is trusted:
 * |sin∠(ray, axis)| for an arrow, |cos∠(ray, plane normal)| for a plane/ring.
 * 0.05 ≈ 3° off degenerate, where the projection error is already ~20× the
 * pointer motion that caused it.
 */
export const MIN_VIEW_SIN = 0.05;

/** Default drag snap steps (Shift selects the fine tier). */
export const TRANSLATE_SNAP_MM = 1;
export const TRANSLATE_FINE_MM = 0.1;
export const ROTATE_SNAP_DEG = 15;
export const ROTATE_FINE_DEG = 1;

/** A pointer ray in WORLD space — the shape `ViewportEngine.screenRay` returns. */
export interface PointerRay {
  origin: Vec3;
  dir: Vec3;
}

const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a: Vec3, b: Vec3): Vec3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
const finite3 = (v: Vec3): boolean => Number.isFinite(v[0]) && Number.isFinite(v[1]) && Number.isFinite(v[2]);

/** Unit vector, or `null` for a non-finite or zero-length input (never NaN out). */
function unit(v: Vec3): Vec3 | null {
  if (!finite3(v)) return null;
  const l = Math.hypot(v[0], v[1], v[2]);
  if (!(l > 1e-12)) return null;
  return [v[0] / l, v[1] / l, v[2] / l];
}

/** Both ends of a projection validated once: unit ray direction + unit axis. */
function rayAndAxis(ray: PointerRay, axis: Vec3): { d: Vec3; n: Vec3 } | null {
  if (!finite3(ray.origin)) return null;
  const d = unit(ray.dir);
  const n = unit(axis);
  return d && n ? { d, n } : null;
}

/**
 * Signed distance along unit `axis` from `pivot` to the pointer ray's closest
 * approach. `null` when the ray is within {@link MIN_VIEW_SIN} of parallel to
 * the axis (looking down the arrow), where the closest-approach parameter is
 * unbounded.
 *
 * The caller drags with DIFFERENCES: sample once at grab, again each frame, and
 * add `now − grab` to the stored component. That cancels the arbitrary offset
 * between the grab point and the arrow's origin, so the arrow does not jump to
 * the cursor on the first frame.
 */
export function axisDragDelta(ray: PointerRay, pivot: Vec3, axis: Vec3): number | null {
  const ok = rayAndAxis(ray, axis);
  if (!ok || !finite3(pivot)) return null;
  const { d, n } = ok;
  const b = dot(n, d);
  const sin2 = 1 - b * b;
  if (sin2 < MIN_VIEW_SIN * MIN_VIEW_SIN) return null;
  const r = sub(pivot, ray.origin);
  const t = (b * dot(d, r) - dot(n, r)) / sin2;
  return Number.isFinite(t) ? t : null;
}

/**
 * Where the pointer ray meets the plane through `pivot` with unit `normal`.
 * `null` when the ray grazes the plane (within {@link MIN_VIEW_SIN} of parallel)
 * or the hit lies BEHIND the pointer — past the horizon the intersection flips
 * to the far side of the camera and the body would teleport.
 */
export function planeDragPoint(ray: PointerRay, pivot: Vec3, normal: Vec3): Vec3 | null {
  const ok = rayAndAxis(ray, normal);
  if (!ok || !finite3(pivot)) return null;
  const { d, n } = ok;
  const denom = dot(n, d);
  if (Math.abs(denom) < MIN_VIEW_SIN) return null;
  const t = dot(n, sub(pivot, ray.origin)) / denom;
  if (!Number.isFinite(t) || t <= 0) return null;
  const p: Vec3 = [ray.origin[0] + d[0] * t, ray.origin[1] + d[1] * t, ray.origin[2] + d[2] * t];
  return finite3(p) ? p : null;
}

/**
 * World delta from `pivot` to the ray's hit on the plane through it. Lies IN the
 * plane by construction (its component along `normal` is zero), which is what
 * makes a plane drag a genuine two-axis translation.
 */
export function planeDragDelta(ray: PointerRay, pivot: Vec3, normal: Vec3): Vec3 | null {
  const p = planeDragPoint(ray, pivot, normal);
  return p ? sub(p, pivot) : null;
}

/**
 * A right-handed in-plane frame for a ring about unit `axis`: `u × v = axis`, so
 * a POSITIVE angle from u toward v is a counter-clockwise rotation about +axis —
 * the same handedness as the Rodrigues rotation `placementMatrix` builds. Get
 * that backwards and every ring drag spins the body the wrong way.
 *
 * The reference vector is chosen away from `axis` so the cross product never
 * degenerates.
 */
export function ringBasis(axis: Vec3): { u: Vec3; v: Vec3 } | null {
  const n = unit(axis);
  if (!n) return null;
  const ref: Vec3 = Math.abs(n[2]) < 0.9 ? [0, 0, 1] : [1, 0, 0];
  const u = unit(cross(ref, n));
  if (!u) return null;
  return { u, v: cross(n, u) };
}

/**
 * Angle (DEGREES, in (−180, 180]) of the pointer's hit on the ring plane,
 * measured in {@link ringBasis}. `null` for a grazing view or a hit exactly on
 * the pivot, where there is no direction to measure.
 */
export function ringAngleDeg(ray: PointerRay, pivot: Vec3, axis: Vec3): number | null {
  const basis = ringBasis(axis);
  const p = basis ? planeDragPoint(ray, pivot, axis) : null;
  if (!basis || !p) return null;
  const w = sub(p, pivot);
  const x = dot(w, basis.u);
  const y = dot(w, basis.v);
  if (Math.hypot(x, y) < 1e-9) return null;
  const a = Math.atan2(y, x) * DEG;
  return Number.isFinite(a) ? a : null;
}

/** Shortest signed difference `from → to`, wrapped into (−180, 180]. */
export function angleDeltaDeg(fromDeg: number, toDeg: number): number {
  if (!Number.isFinite(fromDeg) || !Number.isFinite(toDeg)) return 0;
  let d = (toDeg - fromDeg) % 360;
  if (d > 180) d -= 360;
  if (d <= -180) d += 360;
  return d;
}

/**
 * Continuous angle accumulation across drag frames. `ringAngleDeg` is wrapped,
 * so a sweep through ±180° reads as a −360° jump unless each frame contributes
 * only its SHORTEST step — which is what lets a ring drag pass 180° and keep
 * climbing instead of flipping sign.
 */
export function accumulateAngleDeg(totalDeg: number, prevDeg: number, nowDeg: number): number {
  if (!Number.isFinite(totalDeg)) return 0;
  return totalDeg + angleDeltaDeg(prevDeg, nowDeg);
}

/**
 * Round away the binary-fraction dust a `n · step` snap leaves (0.1 × 3 ≠ 0.3),
 * and normalise `-0` to `0` — a plane drag's out-of-plane component lands on a
 * signed zero, which would otherwise show up in the authored params.
 */
function quantize(v: number, step: number): number {
  if (!Number.isFinite(v)) return 0;
  if (!(step > 0)) return v;
  const q = Math.round((Math.round(v / step) * step) * 1e6) / 1e6;
  return q === 0 ? 0 : q;
}

/** Snap a dragged distance: 1mm by default, 0.1mm on the fine (Shift) tier. */
export function snapTranslate(mm: number, fine = false): number {
  return quantize(mm, fine ? TRANSLATE_FINE_MM : TRANSLATE_SNAP_MM);
}

/** Snap every component of a dragged world delta (plane drags move two axes). */
export function snapTranslateVec(v: Vec3, fine = false): Vec3 {
  return [snapTranslate(v[0], fine), snapTranslate(v[1], fine), snapTranslate(v[2], fine)];
}

/** Snap a dragged angle: 15° by default, 1° on the fine (Shift) tier. */
export function snapRotateDeg(deg: number, fine = false): number {
  return quantize(deg, fine ? ROTATE_FINE_DEG : ROTATE_SNAP_DEG);
}
