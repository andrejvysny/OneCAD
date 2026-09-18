/*
 * Shell thickness interaction math (PURE, framework-free).
 *
 * Shell mirrors the fillet-radius interaction: a vertical pointer drag on the
 * armed body adjusts the wall thickness (up-drag grows it), with an editable mm
 * chip. There is NO cheap-and-honest L1 mesh for a shell (hollowing needs OCCT),
 * so the tool is chip + status-hint driven — the exact shelled body arrives from
 * the backend on commit. The drag mapping is shared with fillet
 * (`handleProjection.sampleMapping` + `flooredDrag`); this module owns the thickness defaults
 * + the re-edit parse.
 */
import { MM_SUFFIX, formatMillimetres } from "@/units/format";
import { handlePointAt, type LinearHandlePath, type ValueWitness } from "./handleProjection";
import type { Vec3 } from "./depthProjection";

export const DEFAULT_SHELL_THICKNESS = 2;

/** Minimum shell thickness (world units); a zero-thickness shell is a no-op. */
export const MIN_SHELL_THICKNESS = 0.1;

/**
 * Format a thickness as document text. Shares the W2-A formatter with fillet's
 * `formatMm`, so both render a number identically (`2`, not `2.0`).
 * `thicknessFromValueText` still parses this AND the RUST-composed `valueText`
 * ("2.0 mm") that seeds a re-edit — both round-trips are pinned.
 *
 * MILLIMETRE-FIXED ON PURPOSE (WP-C2) — see `filletRadius.ts formatMm` for why
 * a `valueText` producer must not follow the display-unit preference.
 */
export function formatThickness(value: number): string {
  return `${formatMillimetres(value)} ${MM_SUFFIX}`;
}

/**
 * Parse a shell feature's display text ("2.0 mm") back to a thickness (re-edit
 * seed; mirrors fillet's `radiusFromValueText`). Non-numeric / non-positive text
 * falls back to the default thickness.
 */
export function thicknessFromValueText(text: string, fallback = DEFAULT_SHELL_THICKNESS): number {
  const n = Number.parseFloat(text);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

/*
 * H9 — the RETAINED-WALL thickness attachment (derivation §5 "Shell").
 *
 * Shell removes faces and thins what is left, so the quantity the user drags is
 * the thickness of a RETAINED wall — not a depth along the removed lid's own
 * normal, which describes the cavity, not the wall. The handle therefore rides
 * the wall's inward offset:
 *
 *     planar       H(t) = E − t·n            dH/dt = −n
 *     cylindrical  H(t) = C + (R − σt)·r̂     dH/dt = −σ·r̂
 *
 * with `E` the rim point where a removed face met the wall, `n` the wall's
 * outward normal, and σ = +1 for a `pin` (material inside the cylinder) or −1
 * for a `hole` (material outside it). The two σ branches are NOT
 * interchangeable: a 6 mm bore shelled 2 mm leaves its wall at radius 8, and
 * using material-outward for both would run the drag backwards (§7 "Inner
 * cylinder sign inversion").
 *
 * WHAT THE SEGMENT MAY CLAIM. It spans the reference wall and its inward offset
 * PLANE — a `targetConstruction`. It certifies neither the shelled wall the
 * kernel will build nor its trimmed extent, and a mesh match is never a BRep
 * proof: a 40 mm cube at fine LOD carries a 0.069282 mm adjacency tolerance, so
 * an unrelated face 0.05 mm away passes the same distance test (§7 "Mesh
 * adjacency falsely identifies a wall"). The attachment is display-only and
 * never becomes a persistent topology reference.
 *
 * Every entry point REFUSES rather than approximating: `null` for a degenerate
 * or non-finite frame, so no path can be built that produces NaN on screen.
 */

/** Which side of a classified cylinder the material is on (SCHEMA §7.5). */
export type WallSidedness = "pin" | "hole";

const finite3 = (v: Vec3): boolean => v.every((c) => Number.isFinite(c));

function unit(v: Vec3): Vec3 | null {
  if (!finite3(v)) return null;
  const l = Math.hypot(v[0], v[1], v[2]);
  if (!(l > 0)) return null;
  return [v[0] / l, v[1] / l, v[2] / l];
}

/** `−x` without producing −0, which reads as a sign a direction cannot carry. */
const neg = (x: number): number => 0 - x;

/**
 * `H(t) = E − t·n` for a planar retained wall: rim point `E`, outward normal
 * `n`. `n` need not arrive normalized — the path's derivative is per millimetre
 * of THICKNESS, so it always is on the way out.
 */
export function planarWallPath(rimPointMm: Vec3, outward: Vec3): LinearHandlePath | null {
  if (!finite3(rimPointMm)) return null;
  const n = unit(outward);
  if (!n) return null;
  return {
    q0Mm: 0,
    point0Mm: [rimPointMm[0], rimPointMm[1], rimPointMm[2]],
    dPointDValue: [neg(n[0]), neg(n[1]), neg(n[2])],
  };
}

/**
 * `H(t) = C + (R − σt)·r̂` for a classified cylindrical retained wall.
 *
 * The rim point supplies the RADIAL DIRECTION only; the path is seated at the
 * classified radius, so a faceted polyline vertex sitting inside the true
 * cylinder does not shorten the construction. `null` when the radial anchor
 * cannot be resolved (a rim point on the axis), when the axis is degenerate, or
 * when the radius is not a positive finite number.
 */
export function cylindricalWallPath(
  axisOriginMm: Vec3,
  axisDirection: Vec3,
  rimPointMm: Vec3,
  radiusMm: number,
  sidedness: WallSidedness,
): LinearHandlePath | null {
  if (!finite3(axisOriginMm) || !finite3(rimPointMm)) return null;
  if (!Number.isFinite(radiusMm) || radiusMm <= 0) return null;
  const z = unit(axisDirection);
  if (!z) return null;
  const d: Vec3 = [
    rimPointMm[0] - axisOriginMm[0],
    rimPointMm[1] - axisOriginMm[1],
    rimPointMm[2] - axisOriginMm[2],
  ];
  const along = d[0] * z[0] + d[1] * z[1] + d[2] * z[2];
  const centre: Vec3 = [
    axisOriginMm[0] + z[0] * along,
    axisOriginMm[1] + z[1] * along,
    axisOriginMm[2] + z[2] * along,
  ];
  const radial = unit([rimPointMm[0] - centre[0], rimPointMm[1] - centre[1], rimPointMm[2] - centre[2]]);
  if (!radial) return null; // the rim point sits ON the axis: no radial direction
  const sigma = sidedness === "pin" ? 1 : -1;
  return {
    q0Mm: 0,
    point0Mm: [
      centre[0] + radial[0] * radiusMm,
      centre[1] + radial[1] * radiusMm,
      centre[2] + radial[2] * radiusMm,
    ],
    dPointDValue: [neg(sigma * radial[0]), neg(sigma * radial[1]), neg(sigma * radial[2])],
  };
}

/**
 * `R − σt` — the radius the remaining cylindrical wall would measure.
 *
 * Non-positive means the construction has eaten the cylinder, which cannot be
 * DEPICTED as an inner wall (§5). The caller keeps a labelled parameter control
 * there and lets the existing preview/domain validation refuse the operation —
 * this function clamps nothing.
 */
export function cylindricalWallRadiusAt(
  radiusMm: number,
  sidedness: WallSidedness,
  thicknessMm: number,
): number {
  return radiusMm - (sidedness === "pin" ? 1 : -1) * thicknessMm;
}

/**
 * The segment from the reference wall to its inward offset, and the ONLY claim
 * it may make: a TARGET construction, never a measured reference. A successful
 * prepare does not establish that a thickness the user has merely asked for has
 * already been built (derivation §2.4).
 */
export function shellThicknessWitness(path: LinearHandlePath, thicknessMm: number): ValueWitness {
  return {
    meaning: "targetConstruction",
    label: "Thickness target t — construction",
    fromMm: path.point0Mm,
    toMm: handlePointAt(path, thicknessMm),
  };
}
