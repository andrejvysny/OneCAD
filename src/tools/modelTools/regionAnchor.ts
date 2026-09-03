/*
 * regionAnchor — the pick-time centroid a picked `SketchRegion`'s fill commits as
 * `profile.regionAnchor` (SCHEMA §7.3 "Region anchor", kernel-hardening WP-B).
 *
 * Pure and deterministic: given the SAME `previewTriangles` fill, the SAME
 * triangle wins every time — the largest-area triangle, ties broken by keeping
 * whichever triangle iterates first (`>` never `>=`). Absent a fill (or an empty
 * one), there is no anchor to author; callers omit the field rather than send a
 * degenerate one.
 */
import type { SketchRegion } from "@/ipc/types";

/** 2× the signed area of the (u,v) triangle `p0,p1,p2` (cross of two edges). */
function twiceSignedArea(
  p0: readonly [number, number],
  p1: readonly [number, number],
  p2: readonly [number, number],
): number {
  return (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p1[1] - p0[1]) * (p2[0] - p0[0]);
}

/**
 * The centroid of the largest-area triangle in `region.previewTriangles`, in the
 * SAME (u,v) plane coordinates the fill is authored in. `undefined` when the
 * region carries no fill or the fill has no complete triangle.
 */
export function regionAnchorOf(region: SketchRegion): [number, number] | undefined {
  const tris = region.previewTriangles;
  if (!tris) return undefined;
  const { positions, indices } = tris;
  let bestArea = 0;
  let best: [number, number] | undefined;
  for (let i = 0; i + 2 < indices.length; i += 3) {
    const i0 = indices[i];
    const i1 = indices[i + 1];
    const i2 = indices[i + 2];
    const p0: [number, number] = [positions[2 * i0], positions[2 * i0 + 1]];
    const p1: [number, number] = [positions[2 * i1], positions[2 * i1 + 1]];
    const p2: [number, number] = [positions[2 * i2], positions[2 * i2 + 1]];
    const area = Math.abs(twiceSignedArea(p0, p1, p2)) / 2;
    // Strict `>` only — a tie keeps the FIRST triangle, deterministic regardless
    // of any later triangle matching or exceeding it by float noise.
    if (area > bestArea || best === undefined) {
      bestArea = area;
      best = [(p0[0] + p1[0] + p2[0]) / 3, (p0[1] + p1[1] + p2[1]) / 3];
    }
  }
  return best;
}
