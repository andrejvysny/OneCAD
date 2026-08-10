/*
 * Angle arc preview (PURE) — the sampled polyline for the dashed arc glyph
 * shown next to a live angle chip, tying the chip to which line it is being
 * measured against.
 *
 * `fromDeg`/`toDeg` are ABSOLUTE headings (plane +U = 0°); the sweep is
 * sampled in a straight numeric line from one to the other, never re-folded.
 * Callers already hand it the SHORTER arc — `liveDimFrames.ts`'s
 * `chainRefAngleDeg` + `foldSigned180` guarantee `toDeg - fromDeg` never
 * exceeds 180° in magnitude — so a direct linear sweep IS the shorter arc; a
 * second fold here would be redundant, not corrective.
 */
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { foldSigned180 } from "./liveDimension";

const ARC_PREVIEW_SEGMENTS = 32;
const DEG = Math.PI / 180;

/**
 * The arc's sweep endpoints for a chained leg — NOT the same baseline as the
 * chip's own signed value.
 *
 * The chip (`liveDimFrames.ts`'s `segmentFrame`) measures `relativeAngleDeg`
 * against the reference segment's INCOMING direction (`refAngleDeg` = the
 * direction FROM the anchor before it TO the vertex), because that is what
 * `liveDimConstraints.ts` authors the `Angle` constraint against — the chip
 * and the committed value must never disagree.
 *
 * The ARC, though, is meant to show the actual visual CORNER angle at the
 * vertex — between the OUTGOING ray of the old segment (the half of it a
 * person actually sees extending from the vertex, `refAngleDeg + 180`) and
 * the new leg. Those two baselines are 180° apart, so reusing the chip's own
 * shortest-arc fold would pick the short way round the WRONG ray — the long,
 * visually-backwards side of the actual corner.
 */
export function arcPreviewSweep(
  refAngleDeg: number,
  relativeAngleDeg: number,
): { fromDeg: number; toDeg: number } {
  const refRayDeg = refAngleDeg + 180;
  const newRayDeg = refAngleDeg + relativeAngleDeg;
  const sweepDeg = foldSigned180(newRayDeg - refRayDeg);
  return { fromDeg: refRayDeg, toDeg: refRayDeg + sweepDeg };
}

/** Sampled points from `fromDeg` to `toDeg` (both absolute, plane +U = 0°) on
 *  a circle of `radius` centred at `center`. Degenerate (zero-radius) input
 *  yields a run of coincident points rather than throwing — the caller drops
 *  anything under its own polyline-length floor, same as every other draft. */
export function angleArcPoints(center: Point2, radius: number, fromDeg: number, toDeg: number): Point2[] {
  const out: Point2[] = [];
  for (let i = 0; i <= ARC_PREVIEW_SEGMENTS; i++) {
    const deg = fromDeg + ((toDeg - fromDeg) * i) / ARC_PREVIEW_SEGMENTS;
    const th = deg * DEG;
    out.push({ x: center.x + radius * Math.cos(th), y: center.y + radius * Math.sin(th) });
  }
  return out;
}
