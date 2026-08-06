/*
 * Shared visual tessellation policy for frontend-generated geometry.
 *
 * The real worker owns production CAD tessellation. This policy keeps mock
 * bodies and L1 previews visually credible without exposing a low, fixed
 * polygon count as an accidental rendering contract.
 */

/** Largest allowed circular chord error in sketch/model millimetres. */
export const VISUAL_CHORD_ERROR = 0.02;

/** Never span more than this angle, even for a very small radius. */
export const VISUAL_MAX_ANGLE_DEG = 5;

const MIN_SWEEP_SEGMENTS = 2;
const TAU = Math.PI * 2;

/**
 * Segment count for a circular sweep. It satisfies both the chord-error bound
 * and the angular bound, which keeps small radii smooth and grows resolution
 * for large silhouettes.
 */
export function visualSegmentsForSweep(angleDeg: number, radius: number): number {
  const sweepDeg = Number.isFinite(angleDeg) ? Math.min(360, Math.abs(angleDeg)) : 0;
  if (sweepDeg === 0) return MIN_SWEEP_SEGMENTS;

  const safeRadius = Number.isFinite(radius) ? Math.max(0, Math.abs(radius)) : 0;
  const angularStep = (VISUAL_MAX_ANGLE_DEG * Math.PI) / 180;
  const chordStep =
    safeRadius > VISUAL_CHORD_ERROR
      ? 2 * Math.acos(Math.max(-1, Math.min(1, 1 - VISUAL_CHORD_ERROR / safeRadius)))
      : TAU;
  const step = Math.min(angularStep, chordStep);
  return Math.max(MIN_SWEEP_SEGMENTS, Math.ceil(((sweepDeg * Math.PI) / 180) / step));
}

/** Segment count for a full visual circle. */
export function visualSegmentsForClosedCurve(radius: number): number {
  return visualSegmentsForSweep(360, radius);
}

/** Sagitta of one segment, useful for policy-level tests and diagnostics. */
export function chordErrorForSweep(angleDeg: number, radius: number, segments: number): number {
  const sweepRad = ((Number.isFinite(angleDeg) ? Math.min(360, Math.abs(angleDeg)) : 0) * Math.PI) / 180;
  const safeRadius = Number.isFinite(radius) ? Math.max(0, Math.abs(radius)) : 0;
  const safeSegments = Math.max(1, Math.floor(segments));
  return safeRadius * (1 - Math.cos(sweepRad / (2 * safeSegments)));
}
