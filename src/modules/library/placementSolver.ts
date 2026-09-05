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
 *
 * A `profile` (WP-C) component's convention is separate again: its local +Z
 * is the extrusion axis the worker prisms along, and the identity frame
 * (no attachment picked) seats at the canonical end cap's `z = 0` plane — the
 * face `ExtractPrismProfile` bakes and `PlaceComponent` reads back — so an
 * unmated drop stands the stick up from that cap, matching a `generator`
 * component's identity seat.
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

// ── attachment frames (spec §2.1 `[attachments].<key>.frame`; WP-F1.1) ───────
//
// A component seats from a NAMED LOCAL BASIS, not from its model origin. The
// frame is authored in `component.toml`, frozen into the record's
// `mate.selfFrame` at placement, and honored identically here and in the
// worker's `ComponentMateSolver.cpp` (that file is a 1:1 port of this one —
// keep both sides' math and both sides' tests in lockstep).

/**
 * A component-local seating basis: `origin` is the point that lands on the
 * target's seat point, `z` the direction aligned to the target axis/normal,
 * `x` the roll reference about `z`. Right-handed (`y = z × x`, derived).
 * Assumed already orthonormal — Rust's manifest parse normalizes it.
 */
export interface MateSelfFrame {
  origin: Vec3;
  z: Vec3;
  x: Vec3;
}

/** Row-major 3×3, `m[row][col]`. */
type Mat3 = readonly [Vec3, Vec3, Vec3];

/** Rodrigues: the rotation matrix for (`axis`, `angleDeg`). */
function matFromAxisAngle(axis: Vec3, angleDeg: number): Mat3 {
  const [x, y, z] = normalize(axis);
  const rad = (angleDeg * Math.PI) / 180;
  const c = Math.cos(rad);
  const s = Math.sin(rad);
  const t = 1 - c;
  return [
    [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
    [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
    [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
  ];
}

function matMultiply(a: Mat3, b: Mat3): Mat3 {
  const row = (r: number): Vec3 => [
    a[r][0] * b[0][0] + a[r][1] * b[1][0] + a[r][2] * b[2][0],
    a[r][0] * b[0][1] + a[r][1] * b[1][1] + a[r][2] * b[2][1],
    a[r][0] * b[0][2] + a[r][1] * b[1][2] + a[r][2] * b[2][2],
  ];
  return [row(0), row(1), row(2)];
}

function matTranspose(m: Mat3): Mat3 {
  return [
    [m[0][0], m[1][0], m[2][0]],
    [m[0][1], m[1][1], m[2][1]],
    [m[0][2], m[1][2], m[2][2]],
  ];
}

function matApply(m: Mat3, v: Vec3): Vec3 {
  return [
    m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
    m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
    m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
  ];
}

/**
 * The inverse of `matFromAxisAngle` — the composed rotation has to be
 * re-expressed as axis-angle because `FrozenPlacement.rotate` is axis-angle
 * and the wire reads nothing else.
 */
function axisAngleFromMat(m: Mat3): AxisAngle {
  const trace = m[0][0] + m[1][1] + m[2][2];
  const c = Math.max(-1, Math.min(1, (trace - 1) / 2));
  if (c > 1 - EPS) return { axis: [0, 0, 1], angleDeg: 0 };
  if (c < -1 + EPS) {
    // θ = π: the skew part vanishes, so recover the axis from `R + I`, whose
    // columns are all 2·u·uᵢ — take the longest to stay conditioned.
    let best: Vec3 = [0, 0, 1];
    let bestLen = 0;
    for (let col = 0; col < 3; col++) {
      const v: Vec3 = [
        m[0][col] + (col === 0 ? 1 : 0),
        m[1][col] + (col === 1 ? 1 : 0),
        m[2][col] + (col === 2 ? 1 : 0),
      ];
      const len = length(v);
      if (len > bestLen) {
        bestLen = len;
        best = v;
      }
    }
    const axis = bestLen < EPS ? ([0, 0, 1] as Vec3) : normalize(best);
    return { axis: [axis[0], axis[1], axis[2]], angleDeg: 180 };
  }
  const s = Math.sqrt(1 - c * c);
  const raw: Vec3 = [
    (m[2][1] - m[1][2]) / (2 * s),
    (m[0][2] - m[2][0]) / (2 * s),
    (m[1][0] - m[0][1]) / (2 * s),
  ];
  const axis = normalize(raw);
  return { axis: [axis[0], axis[1], axis[2]], angleDeg: (Math.acos(c) * 180) / Math.PI };
}

/**
 * `M = S ∘ F⁻¹` expressed back as a `CandidatePlacement`: rotation
 * `R_s · R_fᵀ`, translation `T_s − (R_s · R_fᵀ) · origin_f`.
 *
 * `S` is the seat transform (`seat`, `seatRot`) the snap solved; `F` maps
 * component-local identity onto the attachment frame (rotation columns
 * `[x, z×x, z]`, translation `origin`). Composing the inverse is what makes
 * the ATTACHMENT POINT, rather than the model origin, land on the seat.
 */
function composeWithSelfFrame(seat: Vec3, seatRot: AxisAngle, f: MateSelfFrame): CandidatePlacement {
  const z = normalize(f.z);
  const d = dot(z, f.x);
  const xRel: Vec3 = [f.x[0] - z[0] * d, f.x[1] - z[1] * d, f.x[2] - z[2] * d];
  // Rust normalizes before freezing; this only guards a hand-written record.
  const x = length(xRel) < EPS ? arbitraryPerpendicular(z) : normalize(xRel);
  const y = cross(z, x);
  const rF: Mat3 = [
    [x[0], y[0], z[0]],
    [x[1], y[1], z[1]],
    [x[2], y[2], z[2]],
  ];
  const rM = matMultiply(matFromAxisAngle(seatRot.axis, seatRot.angleDeg), matTranspose(rF));
  const shifted = matApply(rM, f.origin);
  const { axis, angleDeg } = axisAngleFromMat(rM);
  return {
    translate: [seat[0] - shifted[0], seat[1] - shifted[1], seat[2] - shifted[2]],
    rotate: { center: [0, 0, 0], axis, angleDeg },
  };
}

/**
 * The seat placement, composed with `selfFrame` when one is present. Absent
 * returns the un-composed placement VERBATIM — not a multiply-by-identity —
 * so a component with no authored frame keeps its exact pre-WP-F1.1 numbers.
 */
function seated(
  seat: Vec3,
  seatRot: AxisAngle,
  selfFrame: MateSelfFrame | null | undefined,
): CandidatePlacement {
  if (!selfFrame) {
    return {
      translate: [seat[0], seat[1], seat[2]],
      rotate: { center: [0, 0, 0], axis: seatRot.axis, angleDeg: seatRot.angleDeg },
    };
  }
  return composeWithSelfFrame(seat, seatRot, selfFrame);
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
 * Where a camera ray meets the world ground plane `z = 0` — the free-space
 * drop point (spec §5.4 steps 1/6). `null` when the ray cannot reach that
 * plane: parallel to it (`|dir.z|` vanishes) or pointing away from it, where
 * the only honest answer is "no point", and the gesture keeps the ghost where
 * it already was rather than teleporting it to a fabricated one.
 *
 * Z=0 rather than an arbitrary depth because world is Z-up here (viewport
 * README's hard invariant): the ground plane IS the XY plane a part is
 * modelled on, so a free-space drop lands where the grid is drawn.
 */
export function groundPlanePoint(origin: Vec3, direction: Vec3): [number, number, number] | null {
  const dz = direction[2];
  if (!Number.isFinite(dz) || Math.abs(dz) < EPS) return null;
  const t = -origin[2] / dz;
  if (!Number.isFinite(t) || t <= 0) return null;
  return [origin[0] + direction[0] * t, origin[1] + direction[1] * t, origin[2] + direction[2] * t];
}

/**
 * The candidate placement for one already-matched (attachment, classify)
 * pair — spec §5.3's transform rules. `pickWorldPos` is the raw hover hit
 * point (used to pick a seat point along an infinite axis / a point on an
 * infinite plane; `ClassifyFrame` carries no face/edge BOUNDS, so "seated at
 * near end" is approximated as "seated under the cursor" rather than a true
 * nearest-endpoint solve — an honest simplification, not a claim of exact
 * end-detection).
 *
 * `selfFrame` is the matched attachment's own local basis (WP-F1.1). Omit it
 * (or pass `null`) for a component that declares none — that is the identity
 * frame, i.e. the component seats at its model origin, which is exactly the
 * pre-WP-F1.1 behavior and takes the pre-WP-F1.1 arithmetic.
 */
export function solveCandidatePlacement(
  snapKind: MateSnapKind,
  frame: ClassifyFrame,
  pickWorldPos: Vec3,
  flipped: boolean,
  selfFrame?: MateSelfFrame | null,
): CandidatePlacement {
  if (snapKind === "coincident") {
    if (!frame.normal) throw new Error("placementSolver: coincident snap requires a plane normal");
    const normal = normalize(frame.normal);
    const direction: Vec3 = flipped ? [-normal[0], -normal[1], -normal[2]] : normal;
    // WP-I: project the pick onto the target plane — parity with the worker's
    // coincident seat. Identity at pick time (the cursor is already on the
    // plane), but keeps the seat correct once `frame.origin` moves without a
    // fresh classify (e.g. a replayed/edited mate).
    const rel: Vec3 = [
      pickWorldPos[0] - frame.origin[0],
      pickWorldPos[1] - frame.origin[1],
      pickWorldPos[2] - frame.origin[2],
    ];
    const off = dot(rel, normal);
    const seat: Vec3 = [
      pickWorldPos[0] - off * normal[0],
      pickWorldPos[1] - off * normal[1],
      pickWorldPos[2] - off * normal[2],
    ];
    return seated(seat, rotationFromLocalZTo(direction), selfFrame);
  }
  if (!frame.axis) throw new Error(`placementSolver: ${snapKind} snap requires an axis`);
  const axisDir = normalize(frame.axis);
  const direction: Vec3 = flipped ? [-axisDir[0], -axisDir[1], -axisDir[2]] : axisDir;
  const seat =
    snapKind === "concentricAndCoincident"
      ? frame.origin
      : projectPointOntoLine(pickWorldPos, frame.origin, axisDir);
  return seated(seat, rotationFromLocalZTo(direction), selfFrame);
}
