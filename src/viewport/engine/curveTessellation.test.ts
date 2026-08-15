/*
 * Adaptive curve tessellation (SNAP §10.8).
 *
 * The budget is a SCREEN sagitta, so the assertions are geometric: for the
 * chosen segment count, the actual chord-to-arc distance at the projected
 * radius must land under 0.35 CSS px — and the count must not be higher than it
 * needs to be, or a small arc pays for a large one's resolution.
 */
import { describe, expect, it } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import {
  MAX_SAGITTA_PX,
  MAX_SEGMENTS,
  MIN_ELLIPSE_SEGMENTS,
  MIN_SEGMENTS,
  needsRetessellation,
  segmentsForArc,
  segmentsForEntity,
} from "./curveTessellation";
import { entityPolyline } from "./SketchObject";

/** The real chord error for `n` segments over `sweep` at projected radius `r`. */
const sagitta = (rPx: number, sweep: number, n: number): number =>
  rPx * (1 - Math.cos(sweep / n / 2));

describe("segmentsForArc", () => {
  const RADII = [1, 6, 20, 60, 200, 1000, 4000, 20_000];

  it("meets the sagitta budget at every projected radius", () => {
    for (const r of RADII) {
      const n = segmentsForArc(r, Math.PI * 2);
      if (n >= MAX_SEGMENTS) continue; // capped — checked separately below
      expect(sagitta(r, Math.PI * 2, n), `r=${r}`).toBeLessThanOrEqual(MAX_SAGITTA_PX + 1e-9);
    }
  });

  it("does not over-tessellate: one segment fewer would MISS the budget", () => {
    for (const r of RADII) {
      const n = segmentsForArc(r, Math.PI * 2);
      if (n <= MIN_SEGMENTS || n >= MAX_SEGMENTS) continue;
      expect(sagitta(r, Math.PI * 2, n - 1), `r=${r}`).toBeGreaterThan(MAX_SAGITTA_PX);
    }
  });

  it("scales with the SWEEP, not just the radius", () => {
    const full = segmentsForArc(500, Math.PI * 2);
    const quarter = segmentsForArc(500, Math.PI / 2);
    expect(quarter).toBeLessThan(full);
    expect(quarter * 4).toBeGreaterThanOrEqual(full - 4);
  });

  it("floors at MIN_SEGMENTS for a tiny curve", () => {
    expect(segmentsForArc(0.5, Math.PI * 2)).toBe(MIN_SEGMENTS);
    expect(segmentsForArc(6, Math.PI * 2)).toBeGreaterThanOrEqual(MIN_SEGMENTS);
  });

  it("caps at MAX_SEGMENTS for an enormous one", () => {
    expect(segmentsForArc(1e9, Math.PI * 2)).toBe(MAX_SEGMENTS);
  });

  it("survives degenerate input instead of returning NaN", () => {
    expect(segmentsForArc(NaN, Math.PI)).toBe(MIN_SEGMENTS);
    expect(segmentsForArc(10, 0)).toBe(MIN_SEGMENTS);
    expect(segmentsForArc(-5, Math.PI)).toBe(MIN_SEGMENTS);
  });
});

describe("segmentsForEntity", () => {
  const circle: SketchEntity = { id: "c1", type: "Circle", center: [0, 0], radius: 50 };
  const arc: SketchEntity = {
    id: "a1",
    type: "Arc",
    center: [0, 0],
    radius: 50,
    start: [50, 0],
    end: [0, 50],
  };
  const ellipse: SketchEntity = {
    id: "e1",
    type: "Ellipse",
    center: [0, 0],
    majorR: 50,
    minorR: 10,
    rotation: 0,
  };

  it("grows with zoom", () => {
    const near = segmentsForEntity(circle, 20);
    const far = segmentsForEntity(circle, 0.5);
    expect(near).toBeGreaterThan(far);
  });

  it("a quarter arc needs about a quarter of its circle's segments", () => {
    const full = segmentsForEntity(circle, 20);
    const quarter = segmentsForEntity(arc, 20);
    expect(quarter).toBeLessThan(full);
    expect(quarter).toBeGreaterThan(full / 4 - 3);
  });

  it("an ellipse floors higher than a circle (uniform-t sampling is uneven)", () => {
    expect(segmentsForEntity(ellipse, 0.01)).toBe(MIN_ELLIPSE_SEGMENTS);
  });

  it("returns 0 for entities with no sweep", () => {
    expect(segmentsForEntity({ id: "l1", type: "Line", p0: [0, 0], p1: [1, 1] }, 10)).toBe(0);
    expect(segmentsForEntity({ id: "p1", type: "Point", p0: [0, 0] }, 10)).toBe(0);
  });
});

describe("needsRetessellation", () => {
  it("ignores a small drift so a zoom does not churn every curve per frame", () => {
    expect(needsRetessellation(100, 110)).toBe(false);
    expect(needsRetessellation(100, 120)).toBe(false);
  });

  it("fires once the count moves by a quarter", () => {
    expect(needsRetessellation(100, 125)).toBe(true);
    expect(needsRetessellation(100, 70)).toBe(true);
  });

  it("always builds the first time", () => {
    expect(needsRetessellation(0, 32)).toBe(true);
    expect(needsRetessellation(0, 0)).toBe(false);
  });
});

describe("entityPolyline honours the segment count", () => {
  const circle: SketchEntity = { id: "c1", type: "Circle", center: [0, 0], radius: 50 };

  it("emits n+1 points for a closed circle (the ring meets itself)", () => {
    const flat = entityPolyline(circle, 16);
    expect(flat.length / 3).toBe(17);
  });

  it("keeps the historical fixed count when no budget is supplied", () => {
    expect(entityPolyline(circle).length / 3).toBe(65);
  });

  it("ignores a nonsensical count rather than emitting a degenerate strip", () => {
    expect(entityPolyline(circle, 1).length / 3).toBe(65);
    expect(entityPolyline(circle, 0).length / 3).toBe(65);
  });

  it("every emitted vertex is ON the circle at any count", () => {
    for (const n of [8, 16, 64, 256]) {
      const flat = entityPolyline(circle, n);
      for (let i = 0; i < flat.length; i += 3) {
        expect(Math.hypot(flat[i], flat[i + 1])).toBeCloseTo(50, 9);
      }
    }
  });
});
