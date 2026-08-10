import { describe, it, expect } from "vitest";
import { angleArcPoints, arcPreviewSweep } from "./angleArcPreview";

const dist = (a: { x: number; y: number }, b: { x: number; y: number }): number =>
  Math.hypot(a.x - b.x, a.y - b.y);

describe("angleArcPoints", () => {
  const center = { x: 5, y: -3 };

  it("starts at fromDeg and ends at toDeg, on the circle of the given radius", () => {
    const pts = angleArcPoints(center, 10, 0, 90);
    expect(pts[0].x).toBeCloseTo(center.x + 10, 9);
    expect(pts[0].y).toBeCloseTo(center.y, 9);
    expect(pts[pts.length - 1].x).toBeCloseTo(center.x, 9);
    expect(pts[pts.length - 1].y).toBeCloseTo(center.y + 10, 9);
    for (const p of pts) expect(dist(p, center)).toBeCloseTo(10, 9);
  });

  it("sweeps in the DIRECTION of toDeg − fromDeg — negative goes clockwise", () => {
    const pts = angleArcPoints(center, 10, 90, -90);
    // Midpoint of a 90 → -90 sweep is 0°, not the 180° a positive-only sweep
    // would visit — confirms the arc takes the signed, shorter direction.
    const mid = pts[Math.floor(pts.length / 2)];
    expect(mid.x).toBeCloseTo(center.x + 10, 6);
    expect(mid.y).toBeCloseTo(center.y, 6);
  });

  it("a zero sweep degenerates to a single repeated point, not a full circle", () => {
    const pts = angleArcPoints(center, 10, 45, 45);
    for (const p of pts) {
      expect(p.x).toBeCloseTo(center.x + 10 * Math.SQRT1_2, 9);
      expect(p.y).toBeCloseTo(center.y + 10 * Math.SQRT1_2, 9);
    }
  });

  it("a zero radius collapses every sample onto the centre", () => {
    const pts = angleArcPoints(center, 0, 0, 90);
    for (const p of pts) {
      expect(p.x).toBeCloseTo(center.x, 9);
      expect(p.y).toBeCloseTo(center.y, 9);
    }
  });
});

describe("arcPreviewSweep", () => {
  it("reproduces the exact reported bug: a −144.1° chip value sweeps a 35.9° corner", () => {
    // Reference segment's incoming heading 90° (straight up); the chip read
    // ∠ −144.1°. The corner between the two VISIBLE rays (the old segment's
    // outgoing ray at 270°, and the new leg at 90 + (−144.1) = −54.1° ≡
    // 305.9°) is the SMALL 35.9° gap, not the 144.1° the raw chip value
    // would sweep if reused directly as the arc's own baseline.
    const { fromDeg, toDeg } = arcPreviewSweep(90, -144.1);
    expect(fromDeg).toBeCloseTo(270, 9); // the OUTGOING ray, not 90 (incoming)
    expect(Math.abs(toDeg - fromDeg)).toBeCloseTo(35.9, 9);
  });

  it("a straight continuation (relative 0°) sweeps a 180° corner — no bend", () => {
    const { fromDeg, toDeg } = arcPreviewSweep(0, 0);
    expect(fromDeg).toBeCloseTo(180, 9);
    expect(Math.abs(toDeg - fromDeg)).toBeCloseTo(180, 9);
  });

  it("a 90° relative turn (a square corner) sweeps exactly 90°", () => {
    expect(Math.abs(arcPreviewSweep(0, 90).toDeg - arcPreviewSweep(0, 90).fromDeg)).toBeCloseTo(90, 9);
    expect(Math.abs(arcPreviewSweep(0, -90).toDeg - arcPreviewSweep(0, -90).fromDeg)).toBeCloseTo(90, 9);
  });

  it("never sweeps more than 180° — always the visually shorter corner", () => {
    for (const ref of [0, 37, -120, 200]) {
      for (const rel of [-179, -90, -1, 0, 1, 90, 179, 180]) {
        const { fromDeg, toDeg } = arcPreviewSweep(ref, rel);
        expect(Math.abs(toDeg - fromDeg)).toBeLessThanOrEqual(180 + 1e-6);
      }
    }
  });
});
