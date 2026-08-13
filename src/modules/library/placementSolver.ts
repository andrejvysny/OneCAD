/*
 * placementSolver — pure candidate-transform math for the placement gesture
 * (spec §5.3/§5.4; Component Library WP-1.5). No viewport/DOM/CadClient
 * dependency, so it is unit-testable with plain object literals.
 *
 * COMPONENT-LOCAL CONVENTION (matches the worker's v1 generator stub —
 * `worker/src/ops/ComponentOp.cpp::build_m6_shcs`): origin at the seating
 * plane, head above in local +Z, shank below in local -Z. So:
 *   - `shankAxis` attachments align local `(0,0,1)` to the target's axis
 *     direction (the shank then points down the hole in local -Z).
 *   - `headSeat` (plane) attachments align local `(0,0,1)` to the target
 *     face's OUTWARD normal — the head then sits in the +normal half-space
 *     (free space) and the shank crosses into the target's material.
 * A future non-fastener generator with a different local convention will
 * need its own solver branch; nothing here is fastener-agnostic yet, which
 * is honest for a P1 library whose only real geometry IS the M6 SHCS stub.
 */
import type { ClassifyFrame, ClassifyResult } from "@/ipc/types";

export type Vec3 = readonly [number, number, number];

export interface AxisAngle {
  axis: [number, number, number];
  angleDeg: number;
}

export interface CandidatePlacement {
  translate: [number, number, number];
  rotate: { center: [number, number, number]; axis: [number, number, number]; angleDeg: number };
}

const EPS = 1e-9;

function length(v: Vec3): number {
  return Math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
}

function normalize(v: Vec3): Vec3 {
  const len = length(v);
  if (len < EPS) throw new Error("placementSolver: cannot normalize a near-zero vector");
  return [v[0] / len, v[1] / len, v[2] / len];
}

function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

function cross(a: Vec3, b: Vec3): Vec3 {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}

/** Any unit vector perpendicular to `v` (v must already be unit length). */
function arbitraryPerpendicular(v: Vec3): Vec3 {
  const ref: Vec3 = Math.abs(v[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
  return normalize(cross(v, ref));
}

/**
 * The minimal rotation taking world `(0,0,1)` to unit direction `to`
 * (`from` is always local +Z here, so this is not a general two-vector
 * solve). Degenerate cases: `to ≈ +Z` is the identity; `to ≈ -Z` has no
 * unique axis (any perpendicular works) so an arbitrary one is picked —
 * still a geometrically correct 180° flip, just not a specific one.
 */
export function rotationFromLocalZTo(to: Vec3): AxisAngle {
  const target = normalize(to);
  const z: Vec3 = [0, 0, 1];
  const c = Math.max(-1, Math.min(1, dot(z, target)));
  if (c > 1 - EPS) return { axis: [0, 0, 1], angleDeg: 0 };
  if (c < -1 + EPS) {
    const perp = arbitraryPerpendicular(z);
    return { axis: [perp[0], perp[1], perp[2]], angleDeg: 180 };
  }
  const axis = normalize(cross(z, target));
  const angleDeg = (Math.acos(c) * 180) / Math.PI;
  return { axis: [axis[0], axis[1], axis[2]], angleDeg };
}

/** Nearest point on the infinite line `(origin, direction)` to `point`. */
function projectPointOntoLine(point: Vec3, origin: Vec3, direction: Vec3): Vec3 {
  const dir = normalize(direction);
  const rel: Vec3 = [point[0] - origin[0], point[1] - origin[1], point[2] - origin[2]];
  const t = dot(rel, dir);
  return [origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t];
}

/**
 * The snap kind an attachment/classify pairing resolves to (spec §5.3's
 * table), or `null` when the hovered target does not satisfy any accepted
 * geometry kind. `curveType === "circle"` is the "hole rim" concentric +
 * coincident row; a plain circular edge with no seatable frame (radius-less)
 * cannot snap at all.
 */
export type MateSnapKind = "concentric" | "coincident" | "concentricAndCoincident";

export function classifySnapKind(classify: ClassifyResult): MateSnapKind | null {
  if (!classify.frame) return null;
  if (classify.kind === "face" && classify.surfaceType === "cylinder") return "concentric";
  if (classify.kind === "face" && classify.surfaceType === "plane") return "coincident";
  if (classify.kind === "edge" && classify.curveType === "circle") {
    return "concentricAndCoincident";
  }
  return null;
}

/** Geometry kind an `accepts` entry names, matched against `classifySnapKind`'s result. */
const ACCEPTS_FOR_SNAP_KIND: Record<MateSnapKind, readonly string[]> = {
  concentric: ["cylinder", "hole"],
  coincident: ["plane"],
  concentricAndCoincident: ["cylinder", "hole", "circularEdge"],
};

/** Whether `accepts` (one attachment's declared list) admits `snapKind`. */
export function attachmentAccepts(accepts: readonly string[], snapKind: MateSnapKind): boolean {
  return ACCEPTS_FOR_SNAP_KIND[snapKind].some((k) => accepts.includes(k));
}

/**
 * The nominal diameter a metric thread designation names, in mm — `"M6"` → 6,
 * `"M2.5"` → 2.5. `null` for anything that is not an `M<number>` designation
 * (an inch series, a free-text key), which simply opts that entry out of
 * auto-sizing rather than guessing at it.
 */
export function threadNominalDiameterMm(designation: string): number | null {
  const m = /^M(\d+(?:\.\d+)?)$/.exec(designation.trim());
  if (!m) return null;
  const d = Number(m[1]);
  return Number.isFinite(d) && d > 0 ? d : null;
}

/**
 * Auto-size (spec §5.3's hole row, §5.4 step 3): the largest declared size
 * whose NOMINAL diameter still fits `holeDiameterMm` — "nearest smaller
 * standard size", the rule a fastener actually has to obey (an M8 screw does
 * not go through a 6.6 mm clearance hole).
 *
 * `null` when nothing fits (a hole smaller than the smallest declared size) or
 * when no entry is a metric designation — the caller then leaves the size
 * alone rather than substituting one, which is the same refusal discipline the
 * generator applies to an unknown size.
 */
export function nearestSmallerThread(
  holeDiameterMm: number,
  domain: readonly string[],
): string | null {
  if (!Number.isFinite(holeDiameterMm) || holeDiameterMm <= 0) return null;
  let best: { designation: string; diameter: number } | null = null;
  for (const designation of domain) {
    const diameter = threadNominalDiameterMm(designation);
    if (diameter === null || diameter > holeDiameterMm) continue;
    if (!best || diameter > best.diameter) best = { designation, diameter };
  }
  return best?.designation ?? null;
}

/**
 * The candidate placement for one already-matched (attachment, classify)
 * pair — spec §5.3's transform rules. `pickWorldPos` is the raw hover hit
 * point (used to pick a seat point along an infinite axis / a point on an
 * infinite plane; `ClassifyFrame` carries no face/edge BOUNDS, so "seated at
 * near end" is approximated as "seated under the cursor" rather than a true
 * nearest-endpoint solve — an honest simplification, not a claim of exact
 * end-detection).
 */
export function solveCandidatePlacement(
  snapKind: MateSnapKind,
  frame: ClassifyFrame,
  pickWorldPos: Vec3,
  flipped: boolean,
): CandidatePlacement {
  const center: [number, number, number] = [0, 0, 0];
  if (snapKind === "coincident") {
    if (!frame.normal) throw new Error("placementSolver: coincident snap requires a plane normal");
    const normal = normalize(frame.normal);
    const direction: Vec3 = flipped ? [-normal[0], -normal[1], -normal[2]] : normal;
    const { axis, angleDeg } = rotationFromLocalZTo(direction);
    return { translate: [...pickWorldPos], rotate: { center, axis, angleDeg } };
  }
  if (!frame.axis) throw new Error(`placementSolver: ${snapKind} snap requires an axis`);
  const axisDir = normalize(frame.axis);
  const direction: Vec3 = flipped ? [-axisDir[0], -axisDir[1], -axisDir[2]] : axisDir;
  const { axis, angleDeg } = rotationFromLocalZTo(direction);
  const seat =
    snapKind === "concentricAndCoincident"
      ? frame.origin
      : projectPointOntoLine(pickWorldPos, frame.origin, axisDir);
  return { translate: [...seat], rotate: { center, axis, angleDeg } };
}
