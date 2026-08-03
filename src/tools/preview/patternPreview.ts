/*
 * Pattern / mirror preview geometry — PURE math, framework-free (no THREE).
 *
 * The Level-1 preview for the three "duplicate a body" tools (LinearPattern,
 * CircularPattern, MirrorBody) is a set of translucent CLONES of the source
 * body's existing mesh — cheap and honest (no re-modelling on the frontend; the
 * exact fused body arrives from the backend regen on commit). This module owns
 * the transform math that positions each clone; the engine `GhostLayer` builds
 * THREE matrices from the `GhostTransform` descriptors and instances the geometry.
 *
 * Everything is authored in WORLD coordinates (Z-up, right-handed) as plain
 * number-triples so it unit-tests with no WebGL, mirroring depthProjection /
 * lathePreview.
 */
import { normalize, type Vec3 } from "./depthProjection";

/** The three world axes the V1 axis-chip picker offers (Rust `direction` Vec3). */
export type WorldAxis = "X" | "Y" | "Z";
/** The three world mirror planes the V1 plane-chip picker offers. */
export type WorldPlane = "XY" | "XZ" | "YZ";

/** Unit direction for a named world axis. */
export const WORLD_AXIS: Record<WorldAxis, Vec3> = {
  X: [1, 0, 0],
  Y: [0, 1, 0],
  Z: [0, 0, 1],
};

/** Outward normal of a named world plane (through the origin). */
export const WORLD_PLANE_NORMAL: Record<WorldPlane, Vec3> = {
  XY: [0, 0, 1],
  XZ: [0, 1, 0],
  YZ: [1, 0, 0],
};

/**
 * One clone placement, consumed by the engine `GhostLayer` (which turns it into a
 * THREE.Matrix4). Kept THREE-free so the math is unit-testable.
 */
export type GhostTransform =
  | { kind: "translate"; offset: Vec3 }
  | { kind: "rotate"; origin: Vec3; axis: Vec3; angleRad: number }
  | { kind: "mirror"; point: Vec3; normal: Vec3 }
  /**
   * An ALREADY-COMPOSED rigid placement (WP-B W1 `TransformBody`). The three
   * descriptors above each name exactly ONE primitive; a placement is
   * `T ∘ R(pivot)`, which is neither, and folding a two-descriptor sequence
   * would put composition order — the normative part of SCHEMA §7.3 — inside
   * the render layer. It is carried as the matrix instead, built ONCE by
   * {@link placementMatrix} and shared verbatim with the mock lane's mesh
   * rewrite, which is what makes ghost and committed geometry agree exactly.
   */
  | { kind: "rawMatrix"; m: Mat4Rows };

/** A 4×4 matrix in ROW-major order — the argument order `THREE.Matrix4.set` takes. */
export type Mat4Rows = readonly [
  number, number, number, number,
  number, number, number, number,
  number, number, number, number,
  number, number, number, number,
];

const DEG2RAD = Math.PI / 180;

// ── Point transforms (the tested primitives) ─────────────────────────────────

/** Translate a point by `offset`. */
export function translatePoint(p: Vec3, offset: Vec3): Vec3 {
  return [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]];
}

/**
 * Rotate `p` about the line through `origin` along unit-ish `axisDir` by
 * `angleRad` (right-handed / Rodrigues). `axisDir` is normalized internally.
 */
export function rotatePointAboutAxis(p: Vec3, origin: Vec3, axisDir: Vec3, angleRad: number): Vec3 {
  const [kx, ky, kz] = normalize(axisDir);
  const vx = p[0] - origin[0];
  const vy = p[1] - origin[1];
  const vz = p[2] - origin[2];
  const c = Math.cos(angleRad);
  const s = Math.sin(angleRad);
  // k × v
  const cx = ky * vz - kz * vy;
  const cy = kz * vx - kx * vz;
  const cz = kx * vy - ky * vx;
  // k · v
  const d = kx * vx + ky * vy + kz * vz;
  const rx = vx * c + cx * s + kx * d * (1 - c);
  const ry = vy * c + cy * s + ky * d * (1 - c);
  const rz = vz * c + cz * s + kz * d * (1 - c);
  return [origin[0] + rx, origin[1] + ry, origin[2] + rz];
}

/**
 * Reflect `p` across the plane through `point` with normal `normal`
 * (Householder): p' = p − 2·((p−point)·n̂)·n̂.
 */
export function reflectPoint(p: Vec3, point: Vec3, normal: Vec3): Vec3 {
  const [nx, ny, nz] = normalize(normal);
  const dx = p[0] - point[0];
  const dy = p[1] - point[1];
  const dz = p[2] - point[2];
  const d = dx * nx + dy * ny + dz * nz;
  return [p[0] - 2 * d * nx, p[1] - 2 * d * ny, p[2] - 2 * d * nz];
}

// ── Instance placement math ──────────────────────────────────────────────────

/**
 * Linear-pattern instance offsets INCLUDING the original at index 0: instance `k`
 * sits `k · spacing` along the unit `direction`. Length `max(1, count)`.
 */
export function linearOffsets(direction: Vec3, spacing: number, count: number): Vec3[] {
  const n = Math.max(1, Math.floor(count));
  const u = normalize(direction);
  const out: Vec3[] = [];
  for (let k = 0; k < n; k++) out.push([u[0] * spacing * k, u[1] * spacing * k, u[2] * spacing * k]);
  return out;
}

/**
 * Per-instance angle (DEGREES) INCLUDING the original (0°) at index 0. A full
 * 360° sweep divides by `count` so the last instance does not overlap the first;
 * a partial sweep divides by `count − 1` so the instances span the given angle.
 */
export function circularAnglesDeg(totalDeg: number, count: number): number[] {
  const n = Math.max(1, Math.floor(count));
  if (n === 1) return [0];
  const full = Math.abs(Math.abs(totalDeg) - 360) < 1e-6 || Math.abs(totalDeg) >= 360;
  const step = full ? totalDeg / n : totalDeg / (n - 1);
  const out: number[] = [];
  for (let k = 0; k < n; k++) out.push(step * k);
  return out;
}

/**
 * The SCHEMA §7.3 `TransformBody` placement as a row-major 4×4.
 *
 * NORMATIVE evaluation order: `X' = T ∘ R(center, axis, angleDeg) · X` — rotate
 * about the frozen pivot FIRST, then translate. Expanded that is
 * `X' = translate + center + R·(X − center)`, so the matrix's linear part is the
 * Rodrigues rotation `R` and its translation column is
 * `translate + center − R·center`.
 *
 * `axis` need not be unit length (it is normalized here, matching the Rust
 * `TransformRotation.axis` contract); a zero `angleDeg` yields the identity
 * rotation regardless of `axis`, so an axis-less pure translation is expressible.
 */
export function placementMatrix(
  translate: Vec3,
  center: Vec3,
  axis: Vec3,
  angleDeg: number,
): Mat4Rows {
  const r = rotationRows(axis, angleDeg * DEG2RAD);
  const tx = translate[0] + center[0] - (r[0] * center[0] + r[1] * center[1] + r[2] * center[2]);
  const ty = translate[1] + center[1] - (r[3] * center[0] + r[4] * center[1] + r[5] * center[2]);
  const tz = translate[2] + center[2] - (r[6] * center[0] + r[7] * center[1] + r[8] * center[2]);
  return [
    r[0], r[1], r[2], tx,
    r[3], r[4], r[5], ty,
    r[6], r[7], r[8], tz,
    0, 0, 0, 1,
  ];
}

/** Rodrigues rotation as a row-major 3×3 (`[r00,r01,r02, r10,…]`). */
function rotationRows(axis: Vec3, angleRad: number): number[] {
  if (angleRad === 0) return [1, 0, 0, 0, 1, 0, 0, 0, 1];
  const [x, y, z] = normalize(axis);
  const c = Math.cos(angleRad);
  const s = Math.sin(angleRad);
  const t = 1 - c;
  return [
    t * x * x + c, t * x * y - s * z, t * x * z + s * y,
    t * x * y + s * z, t * y * y + c, t * y * z - s * x,
    t * x * z - s * y, t * y * z + s * x, t * z * z + c,
  ];
}

/** Apply a {@link placementMatrix} to a world POINT (translation included). */
export function applyPlacementToPoint(m: Mat4Rows, p: Vec3): Vec3 {
  return [
    m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
    m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
    m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
  ];
}

/**
 * Apply a {@link placementMatrix} to a normal / direction — the LINEAR part only.
 * A rigid placement's linear part is a pure rotation (orthonormal, det +1), so
 * the inverse-transpose a general transform would need reduces to the rotation
 * itself and unit length is preserved exactly.
 */
export function applyPlacementToNormal(m: Mat4Rows, n: Vec3): Vec3 {
  return [
    m[0] * n[0] + m[1] * n[1] + m[2] * n[2],
    m[4] * n[0] + m[5] * n[1] + m[6] * n[2],
    m[8] * n[0] + m[9] * n[1] + m[10] * n[2],
  ];
}

// ── Ghost transforms (the clones only — the original body is already onscreen) ─

/** Translucent clone placements for a linear pattern (instances 1..count−1). */
export function linearGhostTransforms(direction: Vec3, spacing: number, count: number): GhostTransform[] {
  return linearOffsets(direction, spacing, count)
    .slice(1)
    .map((offset) => ({ kind: "translate", offset }));
}

/** Translucent clone placements for a circular pattern (instances 1..count−1). */
export function circularGhostTransforms(
  axisOrigin: Vec3,
  axisDirection: Vec3,
  totalDeg: number,
  count: number,
): GhostTransform[] {
  return circularAnglesDeg(totalDeg, count)
    .slice(1)
    .map((deg) => ({ kind: "rotate", origin: axisOrigin, axis: axisDirection, angleRad: deg * DEG2RAD }));
}

/** The single mirrored clone placement for a mirror-body op. */
export function mirrorGhostTransforms(planePoint: Vec3, planeNormal: Vec3): GhostTransform[] {
  return [{ kind: "mirror", point: planePoint, normal: planeNormal }];
}

// ── Chip defaults + clamps ───────────────────────────────────────────────────

export const DEFAULT_PATTERN_COUNT = 3;
export const MIN_PATTERN_COUNT = 2;
export const MAX_PATTERN_COUNT = 12;
export const DEFAULT_LINEAR_SPACING = 20;
export const DEFAULT_CIRCULAR_ANGLE = 360;

/** Clamp a pattern instance count to the chip range [2, 12] (integer). */
export function clampPatternCount(count: number): number {
  if (!Number.isFinite(count)) return DEFAULT_PATTERN_COUNT;
  return Math.max(MIN_PATTERN_COUNT, Math.min(MAX_PATTERN_COUNT, Math.round(count)));
}

/** Parse a pattern feature's display text ("×4") back to an instance count. */
export function countFromValueText(text: string, fallback = DEFAULT_PATTERN_COUNT): number {
  const m = /(\d+)/.exec(text);
  return m ? clampPatternCount(Number.parseInt(m[1], 10)) : fallback;
}
