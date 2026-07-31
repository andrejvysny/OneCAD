/*
 * entityPolyline — the one plane-local polyline builder BOTH sketch layers render
 * from (SketchObject's Line2 strips + SketchStaticLayer's LineSegments). These
 * specs pin the Ellipse branch added in W3 P3: a CLOSED sampled ring, honouring
 * the rotation, and empty for a half-formed entity (which both layers skip).
 */
import { describe, it, expect } from "vitest";
import { entityPolyline } from "./SketchObject";
import { ELLIPSE_SEGMENTS } from "@/tools/sketch/ellipseMath";

/** Flat xyz → [x, y] pairs (z is always 0 in plane-local space). */
function xy(flat: number[]): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  for (let i = 0; i < flat.length; i += 3) {
    expect(flat[i + 2]).toBe(0);
    out.push([flat[i], flat[i + 1]]);
  }
  return out;
}

describe("entityPolyline — Ellipse", () => {
  const ell = { type: "Ellipse" as const, center: [10, 20] as [number, number], majorR: 6, minorR: 2, rotation: 0 };

  it("emits a CLOSED ring of ELLIPSE_SEGMENTS + 1 points", () => {
    const pts = xy(entityPolyline(ell));
    expect(pts).toHaveLength(ELLIPSE_SEGMENTS + 1);
    expect(pts[pts.length - 1][0]).toBeCloseTo(pts[0][0], 12);
    expect(pts[pts.length - 1][1]).toBeCloseTo(pts[0][1], 12);
  });

  it("starts at the +major end and reaches the +minor end a quarter turn later", () => {
    const pts = xy(entityPolyline(ell));
    expect(pts[0][0]).toBeCloseTo(16, 12);
    expect(pts[0][1]).toBeCloseTo(20, 12);
    const quarter = pts[ELLIPSE_SEGMENTS / 4];
    expect(quarter[0]).toBeCloseTo(10, 12);
    expect(quarter[1]).toBeCloseTo(22, 12);
  });

  it("honours the rotation (major axis along +Y at π/2)", () => {
    const pts = xy(entityPolyline({ ...ell, rotation: Math.PI / 2 }));
    expect(pts[0][0]).toBeCloseTo(10, 12);
    expect(pts[0][1]).toBeCloseTo(26, 12);
  });

  it("defaults a missing rotation to 0", () => {
    const pts = xy(entityPolyline({ type: "Ellipse", center: [0, 0], majorR: 3, minorR: 1 }));
    expect(pts[0]).toEqual([3, 0]);
  });

  it("returns [] for an incomplete ellipse (both layers then skip it)", () => {
    expect(entityPolyline({ type: "Ellipse", center: [0, 0], majorR: 3 })).toEqual([]);
    expect(entityPolyline({ type: "Ellipse", majorR: 3, minorR: 1 })).toEqual([]);
  });

  it("leaves the Circle branch untouched", () => {
    const circle = entityPolyline({ type: "Circle", center: [0, 0], radius: 5 });
    expect(circle.length).toBe((64 + 1) * 3);
    expect(circle.slice(0, 3)).toEqual([5, 0, 0]);
  });
});
