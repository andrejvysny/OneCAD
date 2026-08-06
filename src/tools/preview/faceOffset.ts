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
import type { OffsetDistanceType } from "@/ipc/types";
import { MM_SUFFIX, formatMillimetres } from "@/units/format";

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
