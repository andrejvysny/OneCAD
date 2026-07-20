/*
 * regionPick pure hit-test: point-in-triangle (incl. degenerate + edge hits) and
 * region resolution from plane (u,v) over `previewTriangles`.
 */
import { describe, it, expect } from "vitest";
import { pointInTriangle, regionAtPoint } from "./regionPick";
import type { SketchRegion } from "@/ipc/types";

const tri = (px: number, py: number): boolean =>
  pointInTriangle(px, py, 0, 0, 10, 0, 0, 10);

describe("pointInTriangle", () => {
  it("is true for an interior point", () => {
    expect(tri(2, 2)).toBe(true);
  });

  it("is true on an edge (inclusive)", () => {
    expect(tri(5, 0)).toBe(true); // on the a→b edge
    expect(tri(5, 5)).toBe(true); // on the hypotenuse
    expect(tri(0, 0)).toBe(true); // exactly a vertex
  });

  it("is false just outside an edge", () => {
    expect(tri(6, 5)).toBe(false); // past the hypotenuse
    expect(tri(-0.1, 5)).toBe(false);
  });

  it("is false for a degenerate (zero-area) triangle", () => {
    // All three points collinear → no interior, any query point is outside.
    expect(pointInTriangle(1, 1, 0, 0, 2, 2, 4, 4)).toBe(false);
    expect(pointInTriangle(0, 0, 0, 0, 2, 2, 4, 4)).toBe(false);
  });

  it("is winding-independent (CW triangle still contains its interior)", () => {
    expect(pointInTriangle(2, 2, 0, 0, 0, 10, 10, 0)).toBe(true);
  });
});

// Two disjoint square regions (each two triangles), plane (u,v). r0 near origin,
// r1 far off. Fan layout (centroid at index 0) so profileFromRegion would also accept.
const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};
const R1: SketchRegion = {
  regionId: "r1",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [100, 100, 140, 100, 140, 140, 100, 140], indices: [0, 1, 2, 0, 2, 3] },
};

describe("regionAtPoint", () => {
  it("resolves the region under a plane point", () => {
    expect(regionAtPoint([R0, R1], 5, 3)).toBe("r0");
    expect(regionAtPoint([R0, R1], 130, 110)).toBe("r1");
  });

  it("returns null when the point is over no region", () => {
    expect(regionAtPoint([R0, R1], 60, 60)).toBeNull();
    expect(regionAtPoint([R0, R1], -5, -5)).toBeNull();
  });

  it("resolves a point exactly on a shared/boundary edge (inclusive, first hit wins)", () => {
    expect(regionAtPoint([R0, R1], 0, 10)).toBe("r0"); // on r0's left edge
    expect(regionAtPoint([R0, R1], 120, 120)).toBe("r1"); // on r1's diagonal
  });

  it("skips regions without a usable triangulation", () => {
    const empty: SketchRegion = { regionId: "empty", outerLoop: [], holes: [], previewTriangles: undefined };
    expect(regionAtPoint([empty, R0], 5, 3)).toBe("r0");
  });
});
