/*
 * alignSolve — face-to-face ALIGN as a one-shot placement solve (WP-B W2.5).
 *
 * This is NOT a mate. Nothing persistent is created: the two picked faces are
 * read once, a single rigid motion is computed from them, and that motion is
 * written straight into the armed `TransformBody` record (SCHEMA §7.3
 * `T ∘ R(center, axis, angleDeg)`). Re-running an upstream edit will not
 * re-solve it — a mate would, and V1 has no constraint graph to hang one on, so
 * pretending otherwise would be the dishonest option.
 *
 * Two pure entry points, both PLAIN number triples so they unit-test with no
 * THREE and no WebGL (the `depthProjection` / `transformDrag` precedent):
 *
 *   faceFrame      — a picked face's area-weighted centre + normal, or `null`
 *                    when the face is not planar;
 *   alignPlacement — the motion that lands one frame FLUSH on another.
 *
 * PLANARITY IS REFUSED, NOT APPROXIMATED. A curved face has no single normal, so
 * "the tangent plane at whichever triangle you happened to click" would place
 * the body somewhere the user cannot predict and cannot see was a guess. The
 * check below rejects it outright — the same honesty `edgeDirection`'s bisector
 * tier applies to a curved adjacent face, and the same reason: a deterministic
 * refusal beats a silent wrong answer.
 *
 * NO PATH RETURNS NaN. Every degenerate configuration — a zero-area face, a
 * zero-length normal, two normals that are already anti-parallel, and two
 * normals that point the SAME way (a 180° flip, where the rotation axis is
 * genuinely undetermined) — has an explicit, deterministic branch. The 180° case
 * is the sharp one: `cross(n, -n)` is the zero vector, so the axis is chosen
 * from the normal's own smallest component instead, which is stable, repeatable,
 * and never degenerate.
 */
import type { BodyMeshView } from "@/viewport/mesh/parseMeshPayload";
import type { Vec3 } from "./depthProjection";
import {
  applyPlacementToNormal,
  applyPlacementToPoint,
  placementMatrix,
  type Mat4Rows,
} from "./patternPreview";

/** A planar face reduced to the only two things an align cares about. */
export interface PlanarFaceFrame {
  /** Area-weighted centroid of the face's triangles (world). */
  center: Vec3;
  /** Unit outward normal (world), area-weighted over the same triangles. */
  normal: Vec3;
}

/** The rigid motion an align writes into the armed placement record. */
export interface AlignSolution {
  translate: Vec3;
  rotate: { center: Vec3; axis: Vec3; angleDeg: number };
}

/**
 * How far a triangle normal may deviate from the face's mean normal before the
 * face is refused as curved (radians). 1e-3 rad ≈ 0.057°: far above the float
 * noise of a genuinely planar tessellation (which measures 0 to ~1e-8 even on a
 * long thin facet) and far below the per-facet turn of any real curved surface —
 * a 24-segment cylinder turns 15° per facet, four orders of magnitude over.
 */
export const PLANARITY_TOL_RAD = 1e-3;

/** Below this |sin∠| two normals count as parallel and the axis is chosen by hand. */
const MIN_AXIS_SIN = 1e-12;

const RAD2DEG = 180 / Math.PI;

const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const len = (a: Vec3): number => Math.hypot(a[0], a[1], a[2]);
const cross = (a: Vec3, b: Vec3): Vec3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];

function unit(a: Vec3): Vec3 | null {
  const l = len(a);
  if (!Number.isFinite(l) || l < 1e-12) return null;
  return [a[0] / l, a[1] / l, a[2] / l];
}

const finite = (a: Vec3): boolean => a.every((c) => Number.isFinite(c));

// ── faceFrame ───────────────────────────────────────────────────────────────

interface FaceAccum {
  /** Σ (2·area_i · centroid_i) — divided by `area2` to give the centroid. */
  centroidSum: Vec3;
  /** Σ cross_i, which IS Σ (2·area_i · n̂_i) — the area-weighted normal. */
  normalSum: Vec3;
  /** Σ 2·area_i. */
  area2: number;
  /** Per-triangle unit normals, for the planarity pass. */
  unitNormals: Vec3[];
}

/** Vertex `vi` of the mesh as a triple. */
function vertexAt(view: BodyMeshView, vi: number): Vec3 {
  const o = vi * 3;
  return [view.positions[o], view.positions[o + 1], view.positions[o + 2]];
}

/** Accumulate the face's triangles. Degenerate (zero-area) facets are skipped. */
function accumulateFace(view: BodyMeshView, firstTri: number, triCount: number): FaceAccum {
  const acc: FaceAccum = { centroidSum: [0, 0, 0], normalSum: [0, 0, 0], area2: 0, unitNormals: [] };
  for (let t = firstTri; t < firstTri + triCount; t++) {
    const p0 = vertexAt(view, view.indices[t * 3]);
    const p1 = vertexAt(view, view.indices[t * 3 + 1]);
    const p2 = vertexAt(view, view.indices[t * 3 + 2]);
    const c = cross(sub(p1, p0), sub(p2, p0));
    const n = unit(c);
    if (!n) continue; // zero-area sliver: no centroid weight and no direction
    const a2 = len(c);
    for (let k = 0; k < 3; k++) {
      acc.centroidSum[k] += (a2 * (p0[k] + p1[k] + p2[k])) / 3;
      acc.normalSum[k] += c[k];
    }
    acc.area2 += a2;
    acc.unitNormals.push(n);
  }
  return acc;
}

/** The largest angle (radians) between any of `normals` and `mean`. */
function maxDeviationRad(normals: readonly Vec3[], mean: Vec3): number {
  let worst = 0;
  for (const n of normals) {
    // atan2(|n×m|, n·m) rather than acos(n·m): exact near 0 and near π, where
    // acos loses every digit it has.
    const a = Math.atan2(len(cross(n, mean)), dot(n, mean));
    if (a > worst) worst = a;
  }
  return worst;
}

/**
 * A face's planar frame, or `null` when it has none.
 *
 * Refuses: an out-of-range ordinal, an empty / zero-area face, a face whose
 * triangle normals cancel (no mean direction at all), and any face whose
 * triangles deviate from that mean by more than `tolRad` — i.e. a curved face.
 *
 * `faceOrdinal` is the FACE_RANGES index, not a TopoKey: callers resolve the
 * picked id through the registry's `faceIndex.ordinalForId` first.
 */
export function faceFrame(
  view: BodyMeshView,
  faceOrdinal: number,
  tolRad: number = PLANARITY_TOL_RAD,
): PlanarFaceFrame | null {
  if (!Number.isInteger(faceOrdinal) || faceOrdinal < 0 || faceOrdinal >= view.faceCount) return null;
  const firstTri = view.faceRanges[faceOrdinal * 2];
  const triCount = view.faceRanges[faceOrdinal * 2 + 1];
  if (triCount === 0) return null;

  const acc = accumulateFace(view, firstTri, triCount);
  if (acc.area2 <= 0 || acc.unitNormals.length === 0) return null;
  const normal = unit(acc.normalSum);
  if (!normal) return null;
  if (maxDeviationRad(acc.unitNormals, normal) > tolRad) return null;

  const center: Vec3 = [
    acc.centroidSum[0] / acc.area2,
    acc.centroidSum[1] / acc.area2,
    acc.centroidSum[2] / acc.area2,
  ];
  return finite(center) ? { center, normal } : null;
}

// ── alignPlacement ──────────────────────────────────────────────────────────

/**
 * A deterministic unit vector perpendicular to `n` (assumed unit).
 *
 * Built from the basis axis LEAST aligned with `n` — smallest |component|, ties
 * broken by the lowest index — so `|n × e| ≥ √(2/3) ≈ 0.816` always and the
 * normalisation can never divide by ~0. Deterministic matters here: this is the
 * axis of a 180° flip, and the same two faces must produce the same placement
 * every time or a re-pick would silently spin the body the other way round.
 */
export function stablePerpendicular(n: Vec3): Vec3 {
  let i: 0 | 1 | 2 = 0;
  if (Math.abs(n[1]) < Math.abs(n[i])) i = 1;
  if (Math.abs(n[2]) < Math.abs(n[i])) i = 2;
  const e: Vec3 = [0, 0, 0];
  e[i] = 1;
  // Unreachable for a unit `n` by the bound above; the fallback keeps the
  // return type honest rather than asserting.
  return unit(cross(n, e)) ?? [1, 0, 0];
}

/**
 * The MINIMAL rotation taking unit `from` onto unit `to`, as axis + degrees.
 *
 * Three branches, no NaN in any of them:
 *   already aligned  → 0° (axis is unused but still a real unit vector);
 *   opposed          → exactly 180° about {@link stablePerpendicular};
 *   otherwise        → `atan2(|from×to|, from·to)` about the normalised cross,
 *                      which stays exact at both ends of the range.
 */
export function minimalRotation(from: Vec3, to: Vec3): { axis: Vec3; angleDeg: number } {
  const c = cross(from, to);
  const s = len(c);
  if (s <= MIN_AXIS_SIN) {
    return { axis: stablePerpendicular(from), angleDeg: dot(from, to) >= 0 ? 0 : 180 };
  }
  return { axis: [c[0] / s, c[1] / s, c[2] / s], angleDeg: Math.atan2(s, dot(from, to)) * RAD2DEG };
}

/**
 * The placement that lands `moving` FLUSH on `dest`: the moving face's normal
 * ends up ANTI-PARALLEL to the destination's (the two faces meet, they do not
 * point the same way) and the two face centres coincide.
 *
 * `pivot` is the record's FROZEN rotation centre — it is an input, not a
 * derived value, because SCHEMA §7.3 freezes it at first authoring and a later
 * align must compose against the same point rather than move it. The
 * translation is whatever is left over once the rotation about that pivot has
 * had its say:
 *
 *     translate = dest.center − ( R·(moving.center − pivot) + pivot )
 *
 * Returns `null` only for non-finite input (a caller that hands over a frame it
 * never checked); every geometric configuration has an answer.
 */
export function alignPlacement(
  moving: PlanarFaceFrame,
  dest: PlanarFaceFrame,
  pivot: Vec3,
): AlignSolution | null {
  if (!finite(moving.center) || !finite(dest.center) || !finite(pivot)) return null;
  const m = unit(moving.normal);
  const d = unit(dest.normal);
  if (!m || !d) return null;

  const { axis, angleDeg } = minimalRotation(m, [-d[0], -d[1], -d[2]]);
  // Rotation ONLY (zero translation) about the frozen pivot: exactly the `R`
  // half of the record's own `T ∘ R`, so the translation solved against it is
  // the record's `translate` verbatim — no second convention to keep in step.
  const rotOnly = placementMatrix([0, 0, 0], pivot, axis, angleDeg);
  const rotated = applyPlacementToPoint(rotOnly, moving.center);
  return {
    translate: [dest.center[0] - rotated[0], dest.center[1] - rotated[1], dest.center[2] - rotated[2]],
    rotate: { center: [pivot[0], pivot[1], pivot[2]], axis, angleDeg },
  };
}

// ── Frames vs. the record's INPUT geometry ──────────────────────────────────
//
// A `TransformBody` record's `translate`/`rotate` are relative to the geometry
// the record CONSUMES, not to what is on screen. On a fresh arm those are the
// same thing. On a fold / re-edit they are not: the visible body already carries
// the record's stored motion, so a frame read off the mesh registry has to be
// pulled BACK through that motion before it can be solved against — otherwise
// the align composes on top of a placement it is supposed to replace, and the
// body lands at roughly twice the intended offset.
//
// The same correction is what a GHOST of the re-edited placement needs, for the
// same reason and in the same direction: `M_live ∘ M_armed⁻¹` is the motion that
// takes the mesh as it is on screen to where the edited record would put it.

/** Row-major 4×4 product `a · b` — apply `b` first, then `a`. */
export function composePlacements(a: Mat4Rows, b: Mat4Rows): Mat4Rows {
  const out = new Array<number>(16).fill(0);
  for (let i = 0; i < 4; i++) {
    for (let j = 0; j < 4; j++) {
      let sum = 0;
      for (let k = 0; k < 4; k++) sum += a[i * 4 + k] * b[k * 4 + j];
      out[i * 4 + j] = sum;
    }
  }
  return out as unknown as Mat4Rows;
}

/**
 * The inverse of a {@link placementMatrix}, expressed as another placement about
 * the SAME centre: `T(t) ∘ R(c,a,θ)` inverts to `T(−R⁻¹t) ∘ R(c,a,−θ)`. Built
 * out of `placementMatrix` itself rather than a hand-rolled 4×4 inverse — the
 * linear part is a rotation, so there is nothing to invert numerically.
 */
export function inversePlacement(
  translate: Vec3,
  center: Vec3,
  axis: Vec3,
  angleDeg: number,
): Mat4Rows {
  const rInv = placementMatrix([0, 0, 0], [0, 0, 0], axis, -angleDeg);
  const t = applyPlacementToNormal(rInv, translate); // R⁻¹·t
  return placementMatrix([-t[0], -t[1], -t[2]], center, axis, -angleDeg);
}

/**
 * Push a face frame through a placement: the centre takes the full motion, the
 * normal only its rotation. Re-normalised so a long chain cannot drift the
 * normal off the unit sphere; `null` if the frame arrives non-finite.
 */
export function transformFrame(m: Mat4Rows, frame: PlanarFaceFrame): PlanarFaceFrame | null {
  const center = applyPlacementToPoint(m, frame.center);
  const normal = unit(applyPlacementToNormal(m, frame.normal));
  if (!normal || !finite(center)) return null;
  return { center, normal };
}
