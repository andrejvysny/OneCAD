import { describe, expect, it } from "vitest";

import type { SketchRegion } from "@/ipc/types";
import { regionAnchorOf } from "./regionAnchor";

/** 20×20 square, fan-triangulated into two EQUAL-area triangles (a genuine tie). */
const SQUARE: SketchRegion = {
  regionId: "square",
  outerLoop: ["e1"],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};

/**
 * A 20×20 outer square (0,0)–(20,20) minus an 4×4 inner hole (8,8)–(12,12),
 * triangulated as a ring of 8 triangles (2 per side band). `holesSubtracted: 1`
 * matches the real fill's convention. The bottom-band and right-band OUTER
 * triangles are a deliberate tie at area 80 — the bottom one appears first in
 * `indices`, so it must win.
 */
const RING: SketchRegion = {
  regionId: "ring",
  outerLoop: ["e1"],
  holes: [["e2"]],
  previewTriangles: {
    // A=0 B=1 C=2 D=3 (outer, CCW from origin) E=4 F=5 G=6 H=7 (inner hole)
    positions: [0, 0, 20, 0, 20, 20, 0, 20, 8, 8, 12, 8, 12, 12, 8, 12],
    indices: [
      0, 1, 5, 0, 5, 4, // bottom band
      1, 2, 6, 1, 6, 5, // right band
      2, 3, 7, 2, 7, 6, // top band
      3, 0, 4, 3, 4, 7, // left band
    ],
    holesSubtracted: 1,
  },
};

describe("regionAnchorOf", () => {
  it("returns undefined when the region has no fill", () => {
    expect(regionAnchorOf({ regionId: "r", outerLoop: [], holes: [] })).toBeUndefined();
  });

  it("returns undefined when the fill has no complete triangle", () => {
    const region: SketchRegion = {
      regionId: "r",
      outerLoop: [],
      holes: [],
      previewTriangles: { positions: [0, 0], indices: [] },
    };
    expect(regionAnchorOf(region)).toBeUndefined();
  });

  it("picks the centroid of the largest-area triangle, inside the region", () => {
    const anchor = regionAnchorOf(SQUARE);
    expect(anchor).toBeDefined();
    const [u, v] = anchor!;
    expect(u).toBeGreaterThan(0);
    expect(u).toBeLessThan(20);
    expect(v).toBeGreaterThan(0);
    expect(v).toBeLessThan(20);
  });

  it("breaks a tie deterministically by keeping the FIRST max-area triangle", () => {
    // Triangle 0 (0,1,2) and triangle 1 (0,2,3) both have area 200; triangle 0's
    // centroid is (13.33, 6.67), triangle 1's is (6.67, 13.33) — distinguishable.
    const anchor = regionAnchorOf(SQUARE);
    expect(anchor![0]).toBeCloseTo(40 / 3, 5);
    expect(anchor![1]).toBeCloseTo(20 / 3, 5);
  });

  it("stays deterministic across repeated calls on the identical fill", () => {
    expect(regionAnchorOf(SQUARE)).toEqual(regionAnchorOf(SQUARE));
  });

  it("anchors inside the ring's outer boundary and OUTSIDE the hole for a fill with holesSubtracted", () => {
    const anchor = regionAnchorOf(RING);
    expect(anchor).toBeDefined();
    const [u, v] = anchor!;
    // Inside the outer 20×20 square.
    expect(u).toBeGreaterThan(0);
    expect(u).toBeLessThan(20);
    expect(v).toBeGreaterThan(0);
    expect(v).toBeLessThan(20);
    // Outside the 8..12 × 8..12 hole (a simple bbox test suffices for this
    // axis-aligned fixture — the hole is a square, not an arbitrary polygon).
    const insideHole = u > 8 && u < 12 && v > 8 && v < 12;
    expect(insideHole).toBe(false);
    // Pins the deterministic tie winner: the bottom band's outer triangle
    // (0,1,5) — area 80 — appears before the right band's equal-area (1,2,6).
    expect(u).toBeCloseTo(32 / 3, 5);
    expect(v).toBeCloseTo(8 / 3, 5);
  });
});
