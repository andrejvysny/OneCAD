import { describe, it, expect } from "vitest";
import { hitTestSketch } from "./sketchHitTest";
import type { SketchEntity } from "@/ipc/types";

describe("hitTestSketch — tier priority", () => {
  const entities: SketchEntity[] = [{ id: "l1", type: "Line", p0: [0, 0], p1: [40, 0] }];

  it("a point handle wins over the body at the same spot", () => {
    // (0,0) is both the line's Start endpoint AND on its body.
    expect(hitTestSketch({ x: 0, y: 0 }, entities, 5)).toEqual({ entityId: "l1", point: "Start" });
  });

  it("a click on the body away from any vertex → an edge/body pick (no point)", () => {
    const sel = hitTestSketch({ x: 20, y: 1 }, entities, 5);
    expect(sel).toEqual({ entityId: "l1" });
    expect(sel?.point).toBeUndefined();
  });

  it("the nearest point handle wins within the point tier", () => {
    // Near the End [40,0] rather than Start [0,0].
    expect(hitTestSketch({ x: 39, y: 1 }, entities, 5)).toEqual({ entityId: "l1", point: "End" });
  });
});

describe("hitTestSketch — tolerance boundary (inclusive)", () => {
  // A lone Point entity has a handle but NO body, so only the point tier applies.
  const entities: SketchEntity[] = [{ id: "pt", type: "Point", p0: [0, 0] }];

  it("hits at exactly tolWorld (d <= tol)", () => {
    expect(hitTestSketch({ x: 5, y: 0 }, entities, 5)).toEqual({ entityId: "pt", point: "Start" });
  });

  it("misses just beyond tolWorld", () => {
    expect(hitTestSketch({ x: 5, y: 0 }, entities, 4.999)).toBeNull();
  });
});

describe("hitTestSketch — arc angular extent", () => {
  // Arc: center (0,0) r=10, sweeps CCW from (10,0) [0°] to (0,10) [90°] — 1st quadrant only.
  const entities: SketchEntity[] = [
    { id: "a1", type: "Arc", center: [0, 0], radius: 10, start: [10, 0], end: [0, 10] },
  ];

  it("hits the body within the arc's sweep (~45°)", () => {
    const p = { x: Math.SQRT1_2 * 10, y: Math.SQRT1_2 * 10 }; // on the arc at 45°
    expect(hitTestSketch(p, entities, 2)).toEqual({ entityId: "a1" });
  });

  it("does NOT hit the body where the circle would be but the arc is not (~180°)", () => {
    // (-10,0) is on the full circle but outside the [0°,90°] sweep; nearestOnCurve
    // snaps to the far endpoint, which is > tol away, and no vertex is near.
    expect(hitTestSketch({ x: -10, y: 0 }, entities, 2)).toBeNull();
  });

  it("still hits the arc's center handle when clicked near it", () => {
    expect(hitTestSketch({ x: 0.5, y: 0 }, entities, 2)).toEqual({ entityId: "a1", point: "Center" });
  });
});

describe("hitTestSketch — empty result", () => {
  const entities: SketchEntity[] = [
    { id: "l1", type: "Line", p0: [0, 0], p1: [40, 0] },
    { id: "c1", type: "Circle", center: [100, 0], radius: 10 },
  ];

  it("returns null when nothing is within tolerance", () => {
    expect(hitTestSketch({ x: 500, y: 500 }, entities, 5)).toBeNull();
  });

  it("returns null for an empty entity list", () => {
    expect(hitTestSketch({ x: 0, y: 0 }, [], 5)).toBeNull();
  });
});
