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

// A hole is absent from the triangulation (the producer subtracts it), so the
// hit-test misses inside it for free — no hole-aware code path here. Before the
// fill subtracted holes, clicking the middle of a hole selected the region,
// which then extruded a profile the user never pointed at.
const RING: SketchRegion = {
  regionId: "r_ring",
  outerLoop: [],
  holes: [[]],
  previewTriangles: (() => {
    const outer: [number, number][] = [
      [0, 0],
      [40, 0],
      [40, 20],
      [0, 20],
    ];
    const hole: [number, number][] = [
      [15, 5],
      [25, 5],
      [25, 15],
      [15, 15],
    ];
    const positions = [...outer.flat(), ...hole.flat()];
    const indices: number[] = [];
    for (let i = 0; i < 4; i++) {
      const o0 = i;
      const o1 = (i + 1) % 4;
      indices.push(o0, o1, 4 + i);
      indices.push(o1, 4 + ((i + 1) % 4), 4 + i);
    }
    return { positions, indices };
  })(),
};

describe("regionAtPoint over a region with a hole", () => {
  it("hits the material between the outer boundary and the hole", () => {
    expect(regionAtPoint([RING], 5, 10)).toBe("r_ring"); // left of the hole
    expect(regionAtPoint([RING], 35, 10)).toBe("r_ring"); // right of the hole
    expect(regionAtPoint([RING], 20, 2)).toBe("r_ring"); // below the hole
  });

  it("MISSES inside the hole", () => {
    expect(regionAtPoint([RING], 20, 10)).toBeNull(); // dead centre of the hole
    expect(regionAtPoint([RING], 16, 6)).toBeNull();
  });

  it("still misses outside the outer boundary", () => {
    expect(regionAtPoint([RING], -5, 10)).toBeNull();
  });
});
