/*
 * Face-offset interaction math (PURE, framework-free) — SCHEMA §7.3 `op.offsetFace`.
 *
 * The OffsetFace tool moves selected face(s) along their own surface normals and
 * lets the kernel extend/trim the neighbours. Three separate directions matter and
 * are deliberately NOT the same vector:
 *
 *   • the DRAG AXIS — one arrow, one number line, so a multi-face selection still
 *     has a single handle to pull. It is the MEAN of the operative normals, and it
 *     is REFUSED outright when they diverge too far (see `offsetAxisFor`): a mean
 *     over two opposing faces points nowhere, and an arrow pointing nowhere is
 *     worse than no arrow (the fillet doctrine — degrade to the screen-space drag).
 *   • the GHOST OFFSETS — one per face, along THAT face's own normal. This is what
 *     makes the L1 ghost honest for a chained selection where the faces disagree.
 *   • the kernel's signed `d` — derived per face from `distanceType` against
 *     CURRENT geometry, every regen, by the worker. Nothing here computes it.
 *
 * Nothing in this module clamps. An out-of-domain value is refused at the
 * boundary (the reducer / the op builder), never silently corrected — a clamped
 * value desynchronizes the stored param, the preview the user approved and the
 * geometry the next regen builds.
 */
import type { ClassifyFrame, OffsetDistanceType } from "@/ipc/types";
import { MM_SUFFIX, formatMillimetres } from "@/units/format";
import { handlePointAt, type LinearHandlePath, type ValueWitness } from "./handleProjection";

export type Vec3 = [number, number, number];

/** A face's planar frame, as `alignSolve.faceFrame` reports it. */
export interface FaceFrame {
  center: Vec3;
  normal: Vec3;
}

/** Seed offset (mm) for a fresh `Offset` arm — the DEFAULT_SHELL_THICKNESS role. */
export const DEFAULT_OFFSET_DISTANCE = 2;

/**
 * Below this magnitude an `Offset` is a geometric no-op (SCHEMA §7.3: `|d| ≤ tol`
 * is an identity success). The tool REFUSES to confirm there rather than writing
 * a record that rebuilds to nothing — but it never rewrites the number.
 */
export const OFFSET_MIN_MAGNITUDE = 1e-3;

/**
 * How far the operative normals may diverge before the mean stops being an honest
 * drag axis (radians). 45° is generous for a tangent chain around a fillet and
 * still refuses the two-opposing-faces case the mean cannot represent at all.
 */
export const OFFSET_AXIS_MAX_DIVERGENCE_RAD = Math.PI / 4;

const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: Vec3, b: Vec3): Vec3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
const len = (a: Vec3): number => Math.hypot(a[0], a[1], a[2]);

function unit(v: Vec3): Vec3 | null {
  const l = len(v);
  if (!Number.isFinite(l) || l < 1e-9) return null;
  return [v[0] / l, v[1] / l, v[2] / l];
}

/**
 * The ONE drag axis for an operative face set: the unit mean of their normals, or
 * `null` when there is none worth trusting.
 *
 * Refuses an empty set, a non-finite frame, a set whose normals cancel, and any
 * set whose worst normal deviates from the mean by more than `tolRad`. The caller
 * treats `null` as "no arrow, no 3D ghost" and falls back to the screen-space
 * value drag — a degraded gesture, never a fabricated direction.
 *
 * `atan2(|n×m|, n·m)` rather than `acos(n·m)`: exact near 0 and near π, where
 * `acos` loses every digit it has (the `alignSolve.maxDeviationRad` precedent).
 */
export function offsetAxisFor(
  frames: readonly FaceFrame[],
  tolRad: number = OFFSET_AXIS_MAX_DIVERGENCE_RAD,
): Vec3 | null {
  if (frames.length === 0) return null;
  const sum: Vec3 = [0, 0, 0];
  const normals: Vec3[] = [];
  for (const f of frames) {
    const n = unit(f.normal);
    if (!n) return null;
    normals.push(n);
    sum[0] += n[0];
    sum[1] += n[1];
    sum[2] += n[2];
  }
  const mean = unit(sum);
  if (!mean) return null; // normals cancel — no direction at all
  for (const n of normals) {
    if (Math.atan2(len(cross(n, mean)), dot(n, mean)) > tolRad) return null;
  }
  return mean;
}

/**
 * The world point the arrow + chip anchor to: the plain mean of the operative
 * face centres. `null` for an empty set or a non-finite centre — the caller then
 * has nothing to anchor and must not invent the origin, which for an off-origin
 * body would put the handle inside unrelated geometry.
 */
export function offsetAnchorFor(frames: readonly FaceFrame[]): Vec3 | null {
  if (frames.length === 0) return null;
  let x = 0;
  let y = 0;
  let z = 0;
  for (const f of frames) {
    if (!f.center.every((c) => Number.isFinite(c))) return null;
    x += f.center[0];
    y += f.center[1];
    z += f.center[2];
  }
  const n = frames.length;
  return [x / n, y / n, z / n];
}

/**
 * Per-face L1 ghost translations: each face moves along ITS OWN normal by
 * `distance`, which is what an offset does and what a mean-axis translation would
 * misreport for a tangent chain. Frames whose normal is degenerate are dropped
 * (they have no direction to move along) rather than translated by zero, so the
 * caller can see the ghost is incomplete.
 *
 * This shows WHERE THE FACES GO, not the resulting solid — the same contract the
 * extrude L1 prism has. The exact re-closed body is the kernel's L2 candidate.
 */
export function ghostOffsets(frames: readonly FaceFrame[], distance: number): Vec3[] {
  if (!Number.isFinite(distance)) return [];
  const out: Vec3[] = [];
  for (const f of frames) {
    const n = unit(f.normal);
    if (!n) continue;
    out.push([n[0] * distance, n[1] * distance, n[2] * distance]);
  }
  return out;
}

/** A feature row's value text + inline-edit seed, for one offset. */
export interface OffsetFaceValue {
  valueText: string;
  primaryValue: number;
  primaryValueKind: "length" | "diameter";
}

/**
 * The history row's value for an offset — a VERBATIM mirror of Rust `dto.rs
 * feature_value`'s `KnownOperation::OffsetFace` arm, because the mock lane must
 * render exactly what the real projection renders or the mock stays green while
 * the Tauri lane shows something else.
 *
 * MILLIMETRE-FIXED (the `formatMm` rule): this is a `valueText`, and
 * {@link distanceFromValueText} reads it back with `parseFloat`, i.e. as mm.
 * Anything a user reads as a MEASUREMENT goes through the display formatter
 * instead.
 */
export function offsetFaceValue(
  distance: number,
  distanceType: OffsetDistanceType,
): OffsetFaceValue {
  if (distanceType === "Diameter") {
    return { valueText: `Ø${distance.toFixed(1)}`, primaryValue: distance, primaryValueKind: "diameter" };
  }
  if (distanceType === "Radius") {
    return { valueText: `R${distance.toFixed(1)}`, primaryValue: distance, primaryValueKind: "length" };
  }
  return {
    valueText: `${distance.toFixed(1)} ${MM_SUFFIX}`,
    primaryValue: distance,
    primaryValueKind: "length",
  };
}

/**
 * Parse an offset feature's display text back to a distance (re-edit seed).
 *
 * Handles all three shapes `offsetFaceValue` emits — `"2.5 mm"`, `"R6.0"`,
 * `"Ø12.0"` — plus a NEGATIVE offset (`"-2.5 mm"`), which is a legal and common
 * `Offset` value: this is the one dimension in the app whose sign is meaningful,
 * so the fillet/shell "non-positive ⇒ fallback" rule would silently flip a
 * shrinking offset into a growing one. Only a non-numeric string falls back.
 */
export function distanceFromValueText(text: string, fallback = DEFAULT_OFFSET_DISTANCE): number {
  // Strip a leading Ø / R prefix; `parseFloat` handles the trailing " mm".
  const n = Number.parseFloat(text.replace(/^[ØR]/, ""));
  return Number.isFinite(n) ? n : fallback;
}

/** Formatted magnitude for a status hint (`2 mm`, never `2.0 mm`). */
export function formatOffsetMm(value: number): string {
  return `${formatMillimetres(value)} ${MM_SUFFIX}`;
}

/*
 * H10 — WHERE AN OFFSET'S VALUE ATTACHES
 * (docs/design/astra/modeling-handle-attachment.md §5 "Offset: …").
 *
 * `Total`, `Radius` and `Diameter` used to be typed-only, on the stated grounds
 * that an absolute value has "no zero to drag from". That is the misframing the
 * derivation names in §1: an absolute dimension needs a starting VALUE, not a
 * meaningful zero, and SCHEMA §7.3 already fixes the reference the worker reads
 * it against — `d = σ(distance − R)` for `Radius`, `d = σ(distance/2 − R)` for
 * `Diameter`, with `σ = sign(n_out·r̂)`.
 *
 * So every type gets a path, and each one is the derivative of the point that
 * actually moves:
 *
 *     Offset   (plane)     H(q) = P + q·n              dH/dq = n
 *     Offset   (cylinder)  H(q) = C + (R0 + σq)·r̂      dH/dq = σ·r̂
 *     Radius               H(R) = C + R·r̂              dH/dR = r̂
 *     Diameter             H(D) = C + (D/2)·r̂          dH/dD = r̂/2
 *     Total                H(T) = B + T·n, B = P − t0·n dH/dT = n
 *
 * TWO CONSEQUENCES THAT ARE EASY TO GET BACKWARDS.
 *
 *  - **Increasing Radius moves the wall OUTWARD for an inner wall too.** A 6 mm
 *    bore opened to 8 mm moves `+2·r̂`, while its material-outward normal is
 *    `−r̂`; driving the handle by the material normal would invert that drag
 *    (§7 "Inner cylinder sign inversion"). `sidedness` explains the kernel's
 *    SIGNED offset — it does not define which way a radius grows.
 *  - **`dPointDValue` carries the semantic gain and must not be normalized.**
 *    Diameter's `r̂/2` is what makes `Cq = g·s` half the radial conditioning, so
 *    a Ø drag hands over to the screen proxy earlier than the same Radius drag
 *    does (0.1598 / 0.1602 radial ⇒ 0.0799 / 0.0801 scalar ⇒ proxy / world).
 *
 * Every entry point REFUSES rather than approximating: `null` for a degenerate
 * or non-finite frame, so no path can be built that produces NaN on screen.
 */

/** Which side of a classified cylinder the material is on (SCHEMA §7.5). */
export type CylinderSidedness = NonNullable<ClassifyFrame["sidedness"]>;

const finite3 = (v: Vec3): boolean => v.every((c) => Number.isFinite(c));

/**
 * Unit vector, or `null` for a non-finite or zero-length input.
 *
 * Strict `l > 0` rather than the legacy {@link unit}'s `1e-9` floor: an ABSOLUTE
 * floor breaks similarity and unit invariance (derivation §3) — shrink a model
 * by λ and the same direction stops resolving.
 */
function direction(v: Vec3): Vec3 | null {
  if (!finite3(v)) return null;
  const l = Math.hypot(v[0], v[1], v[2]);
  if (!(l > 0)) return null;
  return [v[0] / l, v[1] / l, v[2] / l];
}

/** `σ·v` without producing −0, which reads as a sign a direction cannot carry. */
const signed = (v: Vec3, sigma: 1 | -1): Vec3 =>
  sigma > 0 ? [v[0], v[1], v[2]] : [0 - v[0], 0 - v[1], 0 - v[2]];

const sigmaOf = (sidedness: CylinderSidedness): 1 | -1 => (sidedness === "pin" ? 1 : -1);

/** The axis station and unit radial direction a cylindrical attachment needs. */
export interface RadialFrame {
  /** The point on the axis at the sample's own station along it. */
  readonly centreMm: Vec3;
  /** Unit radial direction, axis → sample. */
  readonly radial: Vec3;
}

/**
 * `C = O + z[z·(P−O)]` and `r̂ = unit(P−C)` for a classified cylinder axis
 * `(O, z)` and a supported surface sample `P`.
 *
 * `null` for a degenerate axis or a sample that sits ON it — the derivation's
 * "radial anchor unresolved" class (§4). There is no radial direction there, so
 * the caller tries another real surface sample or falls back to the proxy;
 * normalizing a near-zero vector would fabricate one.
 */
export function radialFrameAt(
  axisOriginMm: Vec3,
  axisDirection: Vec3,
  sampleMm: Vec3,
): RadialFrame | null {
  if (!finite3(axisOriginMm) || !finite3(sampleMm)) return null;
  const z = direction(axisDirection);
  if (!z) return null;
  const d: Vec3 = [
    sampleMm[0] - axisOriginMm[0],
    sampleMm[1] - axisOriginMm[1],
    sampleMm[2] - axisOriginMm[2],
  ];
  const along = d[0] * z[0] + d[1] * z[1] + d[2] * z[2];
  const centreMm: Vec3 = [
    axisOriginMm[0] + z[0] * along,
    axisOriginMm[1] + z[1] * along,
    axisOriginMm[2] + z[2] * along,
  ];
  const radial = direction([
    sampleMm[0] - centreMm[0],
    sampleMm[1] - centreMm[1],
    sampleMm[2] - centreMm[2],
  ]);
  return radial ? { centreMm, radial } : null;
}

/**
 * `H(q) = P + q·n` — one supported planar face's signed offset, and the shared
 * parameter construction a multi-face closure uses at its mean axis.
 *
 * `n` need not arrive normalized; the derivative is per millimetre of the VALUE,
 * so it always is on the way out.
 */
export function planarOffsetPath(referenceMm: Vec3, outward: Vec3): LinearHandlePath | null {
  if (!finite3(referenceMm)) return null;
  const n = direction(outward);
  if (!n) return null;
  return {
    q0Mm: 0,
    point0Mm: [referenceMm[0], referenceMm[1], referenceMm[2]],
    dPointDValue: n,
  };
}

/**
 * `H(q) = C + (R0 + σq)·r̂` — one classified cylinder's SIGNED offset.
 *
 * Seated at the CLASSIFIED radius, not at the sample: a faceted polyline vertex
 * sits inside the true cylinder and would shorten the construction. σ is the
 * material side, so a positive `q` grows the solid on either side of the wall —
 * which is what a signed `Offset` means, and the one place `sidedness` does
 * belong in a derivative.
 */
export function cylindricalOffsetPath(
  axisOriginMm: Vec3,
  axisDirection: Vec3,
  sampleMm: Vec3,
  radiusMm: number,
  sidedness: CylinderSidedness,
): LinearHandlePath | null {
  if (!Number.isFinite(radiusMm) || radiusMm <= 0) return null;
  const frame = radialFrameAt(axisOriginMm, axisDirection, sampleMm);
  if (!frame) return null;
  const { centreMm: c, radial: r } = frame;
  return {
    q0Mm: 0,
    point0Mm: [c[0] + r[0] * radiusMm, c[1] + r[1] * radiusMm, c[2] + r[2] * radiusMm],
    dPointDValue: signed(r, sigmaOf(sidedness)),
  };
}

/**
 * `H(R) = C + R·r̂` / `H(D) = C + (D/2)·r̂` — the two ABSOLUTE radial dimensions.
 *
 * The derivative is `r̂` for a Radius and `r̂/2` for a Diameter, on an inner wall
 * exactly as on an outer one. It carries the ½ deliberately: that is the whole
 * difference in a Ø drag's gain and in the sensitivity it is judged by.
 */
export function radialDimensionPath(
  axisOriginMm: Vec3,
  axisDirection: Vec3,
  sampleMm: Vec3,
  kind: "Radius" | "Diameter",
): LinearHandlePath | null {
  const frame = radialFrameAt(axisOriginMm, axisDirection, sampleMm);
  if (!frame) return null;
  const gain = kind === "Diameter" ? 0.5 : 1;
  const { centreMm: c, radial: r } = frame;
  return {
    q0Mm: 0,
    point0Mm: [c[0], c[1], c[2]],
    dPointDValue: [r[0] * gain, r[1] * gain, r[2] * gain],
  };
}

/*
 * γ₃ = 3u/(1−3u) — Higham's accumulated-rounding factor for a THREE-term dot
 * product (derivation §3). The sign test below is relative to its own operands,
 * so it refuses an uncertain sign at any model scale instead of at a fixed
 * millimetre floor.
 */
const UNIT_ROUNDOFF = Number.EPSILON / 2;
const GAMMA_DOT3 = (3 * UNIT_ROUNDOFF) / (1 - 3 * UNIT_ROUNDOFF);

/**
 * `planeNormal`, flipped if necessary so it points from `planeOriginMm` TOWARD
 * `referenceMm` — the orientation a `Total` needs (derivation §5 "Offset:
 * Total": *orient n from the opposite plane toward the selected plane; certify
 * the sign*).
 *
 * `null` when that sign is not certifiable: the reference lies in the plane, or
 * the separation is smaller than the dot product's own rounding error. A guessed
 * sign there would put `B` on the wrong side and run the drag backwards.
 */
export function orientTowardReference(
  planeNormal: Vec3,
  planeOriginMm: Vec3,
  referenceMm: Vec3,
): Vec3 | null {
  if (!finite3(planeOriginMm) || !finite3(referenceMm)) return null;
  const n = direction(planeNormal);
  if (!n) return null;
  const s: Vec3 = [
    referenceMm[0] - planeOriginMm[0],
    referenceMm[1] - planeOriginMm[1],
    referenceMm[2] - planeOriginMm[2],
  ];
  const dot3 = n[0] * s[0] + n[1] * s[1] + n[2] * s[2];
  const magnitude = Math.abs(n[0] * s[0]) + Math.abs(n[1] * s[1]) + Math.abs(n[2] * s[2]);
  if (!(Math.abs(dot3) > GAMMA_DOT3 * magnitude)) return null;
  return signed(n, dot3 > 0 ? 1 : -1);
}

/**
 * `B = P − t0·n`, `H(T) = B + T·n` — the ABSOLUTE total thickness.
 *
 * `orientedNormal` must already point from the opposite plane toward `P` (see
 * {@link orientTowardReference}), and `referenceThicknessMm` is the PREPARED
 * `currentDims.thickness`, never a locally measured separation: an accepted
 * Total preparation is what establishes the unique opposite and its material
 * column (SCHEMA §7.6).
 */
export function totalThicknessPath(
  referenceMm: Vec3,
  orientedNormal: Vec3,
  referenceThicknessMm: number,
): LinearHandlePath | null {
  if (!finite3(referenceMm)) return null;
  if (!Number.isFinite(referenceThicknessMm) || referenceThicknessMm <= 0) return null;
  const n = direction(orientedNormal);
  if (!n) return null;
  return {
    q0Mm: 0,
    point0Mm: [
      referenceMm[0] - n[0] * referenceThicknessMm,
      referenceMm[1] - n[1] * referenceThicknessMm,
      referenceMm[2] - n[2] * referenceThicknessMm,
    ],
    dPointDValue: n,
  };
}

/**
 * The signed offset from the frozen reference surface — a TARGET until the
 * kernel validates it (derivation §5).
 *
 * A MULTI-face closure downgrades to a shared PARAMETER construction: `A` is the
 * mean of the reference points and lies on no face, and a V3 closure can contain
 * rebuilt blends and fixed supports whose motion roles the frontend does not
 * have. "Every closure face moves by q" is therefore a claim this may not make.
 */
export function offsetTargetWitness(
  path: LinearHandlePath,
  distanceMm: number,
  faceCount: number,
): ValueWitness {
  const shared = faceCount > 1;
  return {
    meaning: shared ? "parameterConstruction" : "targetConstruction",
    label: shared
      ? `Offset parameter d · shared by ${faceCount} faces`
      : "Offset target d — construction",
    fromMm: path.point0Mm,
    toMm: handlePointAt(path, distanceMm),
  };
}

/**
 * Radius: the centreline `C` to the radial target `H(R)`. Diameter: the
 * supporting cylinder's own diameter construction, `H(−D)` to `H(D)`.
 *
 * Both are labelled constructions. Neither depicts a MEASURED span between
 * actual walls — the frontend has not established both finite surface endpoints
 * (derivation §5 "Offset: Radius / Diameter").
 */
export function radialDimensionWitness(
  path: LinearHandlePath,
  valueMm: number,
  kind: "Radius" | "Diameter",
): ValueWitness {
  if (kind === "Diameter") {
    return {
      meaning: "targetConstruction",
      label: "ØD target — construction",
      fromMm: handlePointAt(path, -valueMm),
      toMm: handlePointAt(path, valueMm),
    };
  }
  return {
    meaning: "targetConstruction",
    label: "Radius target R — construction",
    fromMm: path.point0Mm,
    toMm: handlePointAt(path, valueMm),
  };
}

/**
 * The TWO segments a `Total` draws: the prepared reference thickness `P ↔ B`,
 * and the target `B ↔ H(T)`.
 *
 * `measuredReference` is allowed on the first ONLY when the prepare genuinely
 * established `t0` (`currentDims.thickness`); otherwise it is a construction
 * like every other unvalidated claim. The second is always a target — a
 * successful prepare does not establish that a total the user has merely asked
 * for has already been built (derivation §2.4).
 */
export function totalThicknessWitnesses(
  path: LinearHandlePath,
  totalMm: number,
  referenceThicknessMm: number,
  measured: boolean,
): readonly [ValueWitness, ValueWitness] {
  return [
    {
      meaning: measured ? "measuredReference" : "targetConstruction",
      label: measured
        ? "Reference thickness t0 — measured"
        : "Reference thickness t0 — construction",
      fromMm: handlePointAt(path, referenceThicknessMm),
      toMm: path.point0Mm,
    },
    {
      meaning: "targetConstruction",
      label: "Total target T — construction",
      fromMm: path.point0Mm,
      toMm: handlePointAt(path, totalMm),
    },
  ];
}
