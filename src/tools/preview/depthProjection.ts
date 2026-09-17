/*
 * Extrude depth projection (PURE vector math, framework-free).
 *
 * The drag handle sits at the region centroid and points along the plane normal.
 * As the pointer drags, we want the depth = how far along the normal axis the
 * pointer's ray reaches. We take the closest-approach point between the pointer
 * ray and the (infinite) normal axis line, and return the SIGNED distance of that
 * point from the centroid along the normal. Sign gives the direction-flip for
 * free (drag "through zero" ⇒ the prism grows the other way).
 *
 * Kept as plain number-triples so it unit-tests with no THREE / WebGL.
 */
export type Vec3 = [number, number, number];

const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const len = (a: Vec3): number => Math.hypot(a[0], a[1], a[2]);
export function normalize(a: Vec3): Vec3 {
  const l = len(a) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
}

/**
 * Signed depth along `axisDir` (unit) from `axisPoint`, taken at the closest
 * approach between the pointer ray and the axis line. `axisDir` MUST be unit; the
 * ray direction need not be. Parallel rays fall back to projecting the ray origin
 * onto the axis.
 */
export function axisDepthFromRay(
  rayOrigin: Vec3,
  rayDir: Vec3,
  axisPoint: Vec3,
  axisDir: Vec3,
): number {
  const d1 = axisDir; // line 1 (unit)
  const d2 = rayDir; // line 2
  const r = sub(axisPoint, rayOrigin);
  const a = dot(d1, d1); // == 1 for a unit axis
  const e = dot(d2, d2);
  const f = dot(d2, r);
  const c = dot(d1, r);
  const b = dot(d1, d2);
  const denom = a * e - b * b;
  // Parallel (or degenerate ray): project the ray origin onto the axis.
  if (denom <= 1e-9) return -c / (a || 1);
  return (b * f - c * e) / denom;
}

/**
 * Apply the direction / symmetry modifiers to a raw signed depth.
 *  - `symmetric`: the prism grows both ways and `depth` is the TOTAL span, half
 *    to each side — `PreviewMesh.setDepth` scales by `|depth|` and offsets by
 *    `-|depth|/2`, and `ModelToolController.extrudeHeadWorld` puts the handle on
 *    the `+|depth|/2` face the worker builds. The value is unchanged either way;
 *    symmetry is carried as a MODE, not as a different number.
 *  - `flip`: negate (an explicit UI flip, distinct from dragging through zero).
 * Returns the depth the op + preview should use, always the drag magnitude with
 * its sign.
 */
export function resolveDepth(raw: number, opts: { flip?: boolean } = {}): number {
  return opts.flip ? -raw : raw;
}

/** Snap a depth to the nearest grid step (hold-free coarse snapping aid). */
export function snapDepth(depth: number, step: number): number {
  if (step <= 0) return depth;
  return Math.round(depth / step) * step;
}

/**
 * A grab-relative drag basis: the value the gesture started from and the input
 * coordinate (world units) it was grabbed at. A frame reports
 * `start + (input − grab)`, so only the pointer's own travel moves the value.
 */
export interface DragBasis {
  readonly start: number;
  readonly grab: number;
}

/**
 * One frame of a grab-relative drag bounded below by `floor`.
 *
 * At the bound the basis is REBASED onto the current input rather than clamped: a
 * pointer that overshoots by 50 units and turns around must move the value on its
 * first reversing sample, not after 50 units of dead travel (wind-up).
 */
export function flooredDrag(
  basis: DragBasis,
  input: number,
  floor: number,
): { value: number; basis: DragBasis } {
  const value = basis.start + (input - basis.grab);
  if (value >= floor) return { value, basis };
  return { value: floor, basis: { start: floor, grab: input } };
}

/**
 * How an extrude drag maps its input onto the stored depth (TODO.md SESSION 37,
 * decision D-S2):
 *  - `oneSided` — the signed depth follows the input 1:1 and crosses zero freely.
 *  - `symmetricAxis` — the input moves the head, which sits at `|depth|/2`, so the
 *    physical HALF-span follows the pointer and the Total moves at twice its rate.
 *  - `symmetricTotal` — a screen proxy has no head to track; the input drives the
 *    Total at gain 1.
 * Both symmetric modes clamp the span at 0 (rebasing) and carry the caller's sign
 * hint: the worker builds `|distance|` for Symmetric, so the sign is encoding only
 * and must not flip because a drag passed through zero.
 */
export type ExtrudeDragMode = "oneSided" | "symmetricAxis" | "symmetricTotal";

/** The basis a grab (or a mid-drag mode change) takes from the current depth. */
export function extrudeDragBasis(mode: ExtrudeDragMode, depth: number, input: number): DragBasis {
  switch (mode) {
    case "oneSided":
      return { start: depth, grab: input };
    case "symmetricAxis":
      return { start: Math.abs(depth) / 2, grab: input };
    case "symmetricTotal":
      return { start: Math.abs(depth), grab: input };
  }
}

/** One extrude drag frame: the depth to store, and the basis the next frame reads. */
export function extrudeDragFrame(
  mode: ExtrudeDragMode,
  basis: DragBasis,
  input: number,
  signHint: 1 | -1,
): { depth: number; basis: DragBasis } {
  if (mode === "oneSided") return { depth: basis.start + (input - basis.grab), basis };
  const extent = flooredDrag(basis, input, 0);
  const span = mode === "symmetricAxis" ? 2 * extent.value : extent.value;
  // `signHint * 0` would store −0, which reads as a sign it cannot carry.
  return { depth: span === 0 ? 0 : signHint * span, basis: extent.basis };
}
