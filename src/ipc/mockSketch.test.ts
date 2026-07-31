import { describe, it, expect } from "vitest";
import {
  planeFor,
  solveDof,
  freeDegrees,
  detectRegions,
  orderedClosedLoop,
  mockRegionId,
  constraintFreedom,
} from "./mockSketch";
import type { SketchConstraint, SketchEntity } from "./types";

const rect: SketchEntity[] = [
  { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
  { id: "e2", type: "Line", p0: [40, 0], p1: [40, 20] },
  { id: "e3", type: "Line", p0: [40, 20], p1: [0, 20] },
  { id: "e4", type: "Line", p0: [0, 20], p1: [0, 0] },
];

describe("planeFor — exact SCHEMA §7.3 bases", () => {
  it("XY is the non-standard basis", () => {
    expect(planeFor("XY")).toMatchObject({ kind: "XY", xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] });
  });
  it("XZ / YZ bases", () => {
    expect(planeFor("XZ")).toMatchObject({ xAxis: [0, 1, 0], yAxis: [0, 0, 1], normal: [1, 0, 0] });
    expect(planeFor("YZ")).toMatchObject({ xAxis: [-1, 0, 0], yAxis: [0, 0, 1], normal: [0, 1, 0] });
  });
});

describe("solveDof — naive mock heuristic", () => {
  it("empty sketch ⇒ dof 0, fully constrained", () => {
    expect(solveDof([], [])).toEqual({ dof: 0, status: "FullyConstrained" });
  });
  it("a single line has 4 free dof", () => {
    expect(freeDegrees([rect[0]])).toBe(4);
    expect(solveDof([rect[0]], [])).toEqual({ dof: 4, status: "UnderConstrained" });
  });
  it("a Horizontal constraint removes one dof", () => {
    const cs: SketchConstraint[] = [{ id: "c1", type: "Horizontal", entities: ["e1"] }];
    expect(solveDof([rect[0]], cs)).toEqual({ dof: 3, status: "UnderConstrained" });
  });
  it("more constraints than dof ⇒ over-constrained", () => {
    const cs: SketchConstraint[] = Array.from({ length: 3 }, (_, i) => ({
      id: `c${i}`,
      type: "Fixed",
      entities: ["e1"],
    }));
    expect(solveDof([rect[0]], cs).status).toBe("OverConstrained");
  });
});

describe("constraintFreedom — worker DOF table parity", () => {
  it("Concentric removes 2 dof (pairing, like Coincident)", () => {
    expect(constraintFreedom({ id: "c1", type: "Concentric", entities: ["e1", "e2"] })).toBe(2);
  });
  it("Midpoint removes 2 dof (pairing, like Coincident)", () => {
    expect(constraintFreedom({ id: "c1", type: "Midpoint", entities: ["e1", "e2"] })).toBe(2);
  });
});

describe("detectRegions", () => {
  it("finds one region for a closed rectangle", () => {
    const regions = detectRegions(rect);
    expect(regions).toHaveLength(1);
    expect(regions[0].outerLoop.sort()).toEqual(["e1", "e2", "e3", "e4"]);
    expect(regions[0].previewTriangles!.indices.length).toBeGreaterThan(0);
  });

  it("finds a region for a circle", () => {
    const regions = detectRegions([{ id: "c", type: "Circle", center: [0, 0], radius: 5 }]);
    expect(regions).toHaveLength(1);
    expect(regions[0].outerLoop).toEqual(["c"]);
  });

  it("returns no region for an open chain", () => {
    const open: SketchEntity[] = [
      { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
      { id: "e2", type: "Line", p0: [40, 0], p1: [40, 20] },
      { id: "e3", type: "Line", p0: [40, 20], p1: [10, 20] },
    ];
    expect(detectRegions(open)).toHaveLength(0);
  });

  it("ignores construction geometry", () => {
    const withConstruction = rect.map((e, i) => (i === 0 ? { ...e, construction: true } : e));
    expect(detectRegions(withConstruction)).toHaveLength(0);
  });
});

describe("orderedClosedLoop + mockRegionId", () => {
  it("walks the rectangle into a 4-segment cycle", () => {
    const loop = orderedClosedLoop(rect);
    expect(loop).not.toBeNull();
    expect(loop!.ids).toHaveLength(4);
    expect(loop!.points).toHaveLength(4);
  });
  it("region id is deterministic & order-independent", () => {
    expect(mockRegionId(["e1", "e2", "e3"])).toBe(mockRegionId(["e3", "e1", "e2"]));
    expect(mockRegionId(["e1"])).toMatch(/^r_[0-9a-f]{8}$/);
  });
});

// Nesting: each bounded planar cell is selectable. A contained circle is both a
// disc region and the hole boundary of the surrounding annulus.
describe("detectRegions — nested circle creates two cells", () => {
  const inner: SketchEntity = { id: "c", type: "Circle", center: [20, 10], radius: 4 };

  it("publishes the inner disc and surrounding annulus independently", () => {
    const regions = detectRegions([...rect, inner]);
    expect(regions).toHaveLength(2);
    expect(regions[0].outerLoop).toEqual(["c"]);
    expect(regions[0].holes).toEqual([]);
    expect(regions[1].outerLoop).toEqual(["e1", "e2", "e3", "e4"]);
    expect(regions[1].holes).toEqual([["c"]]);
    expect(regions[1].regionId).not.toBe(detectRegions(rect)[0].regionId);
  });

  it("subtracts the hole from the fill (area 800 − π·4² ≈ 749.7)", () => {
    const annulus = detectRegions([...rect, inner]).find((region) => region.holes.length > 0)!;
    const tris = annulus.previewTriangles!;
    let area = 0;
    for (let i = 0; i + 2 < tris.indices.length; i += 3) {
      const [a, b, c] = [tris.indices[i], tris.indices[i + 1], tris.indices[i + 2]];
      const ax = tris.positions[a * 2];
      const ay = tris.positions[a * 2 + 1];
      const bx = tris.positions[b * 2];
      const by = tris.positions[b * 2 + 1];
      const cx = tris.positions[c * 2];
      const cy = tris.positions[c * 2 + 1];
      area += Math.abs(((bx - ax) * (cy - ay) - (by - ay) * (cx - ax)) / 2);
    }
    // 32-gon slightly under-approximates the disc; a FILLED hole would be 800.
    expect(area).toBeGreaterThan(745);
    expect(area).toBeLessThan(755);
  });

  it("keeps a SEPARATED circle as its own region (e2e/multiregion.spec.ts)", () => {
    const outside: SketchEntity = { id: "c", type: "Circle", center: [200, 200], radius: 5 };
    const regions = detectRegions([...rect, outside]);
    expect(regions).toHaveLength(2);
    expect(regions[0].outerLoop).toEqual(["c"]); // circles first, then the loop
    expect(regions[1].holes).toEqual([]);
  });

  it("keeps a circle that CROSSES the loop as its own region", () => {
    // Centre inside, but the disc pokes through the boundary ⇒ invalid nesting.
    const crossing: SketchEntity = { id: "c", type: "Circle", center: [20, 10], radius: 15 };
    const regions = detectRegions([...rect, crossing]);
    expect(regions).toHaveLength(2);
    expect(regions[1].holes).toEqual([]);
  });

  it("omits an enclosing cell when the mock cannot subtract every hole", () => {
    const regions = detectRegions([
      ...rect,
      { id: "c1", type: "Circle", center: [10, 10], radius: 3 },
      { id: "c2", type: "Circle", center: [30, 10], radius: 3 },
    ]);
    expect(regions).toHaveLength(2);
    expect(regions.every((region) => region.holes.length === 0)).toBe(true);
  });
});

// The mock's fill has to satisfy the SAME topology contract as the worker's:
// the only single-use edges are the outer and hole boundaries, so the extrude
// preview recovers both rings and grows an inner wall.
describe("detectRegions hole fill → profileFromRegion", () => {
  it("yields one outer ring and one hole ring", async () => {
    const { profileFromRegion } = await import("@/tools/preview/prismPreview");
    const region = detectRegions([
      ...rect,
      { id: "c", type: "Circle", center: [20, 10], radius: 4 },
    ]).find((candidate) => candidate.holes.length > 0)!;
    const p = profileFromRegion(region)!;
    expect(p).not.toBeNull();
    expect(p.holes).toHaveLength(1);
    // The outer ring keeps the rectangle's silhouette: extremes at the corners.
    const us = p.ring.map(([u]) => u);
    const vs = p.ring.map(([, v]) => v);
    expect(Math.min(...us)).toBeCloseTo(0, 9);
    expect(Math.max(...us)).toBeCloseTo(40, 9);
    expect(Math.min(...vs)).toBeCloseTo(0, 9);
    expect(Math.max(...vs)).toBeCloseTo(20, 9);
    // The hole ring stays within the circle's bounds.
    for (const [u, v] of p.holes[0]) {
      expect(Math.hypot(u - 20, v - 10)).toBeLessThanOrEqual(4 + 1e-9);
    }
  });
});

// ── W3 P3: Ellipse parity — 5 DOF, own region, nests as a hole like a circle ──

describe("Ellipse — mock solver + region parity", () => {
  const ell = (
    id: string,
    center: [number, number],
    majorR: number,
    minorR: number,
    rotation = 0,
  ): SketchEntity => ({ id, type: "Ellipse", center, majorR, minorR, rotation });

  it("contributes 5 free DOF (centre 2 + majorR/minorR/rotation 3)", () => {
    expect(freeDegrees([ell("el1", [0, 0], 10, 4)])).toBe(5);
    expect(solveDof([ell("el1", [0, 0], 10, 4)], [])).toEqual({ dof: 5, status: "UnderConstrained" });
  });

  it("is its own region, with a fill of area ≈ πab", () => {
    const regions = detectRegions([ell("el1", [0, 0], 10, 4, 0.7)]);
    expect(regions).toHaveLength(1);
    expect(regions[0].outerLoop).toEqual(["el1"]);
    expect(regions[0].holes).toEqual([]);
    const tris = regions[0].previewTriangles!;
    let area = 0;
    for (let i = 0; i + 2 < tris.indices.length; i += 3) {
      const [a, b, c] = [tris.indices[i], tris.indices[i + 1], tris.indices[i + 2]];
      area += Math.abs(
        ((tris.positions[b * 2] - tris.positions[a * 2]) *
          (tris.positions[c * 2 + 1] - tris.positions[a * 2 + 1]) -
          (tris.positions[b * 2 + 1] - tris.positions[a * 2 + 1]) *
            (tris.positions[c * 2] - tris.positions[a * 2])) /
          2,
      );
    }
    // 32-gon slightly under-approximates π·10·4 ≈ 125.7 (same as the circle case).
    expect(area).toBeGreaterThan(120);
    expect(area).toBeLessThan(126);
  });

  it("is EXCLUDED from region detection when it is construction geometry", () => {
    expect(detectRegions([{ ...ell("el1", [0, 0], 10, 4), construction: true }])).toEqual([]);
  });

  it("nested inside a closed loop yields the disc cell AND the loop-with-hole cell", () => {
    const regions = detectRegions([...rect, ell("el1", [20, 10], 8, 3)]);
    expect(regions).toHaveLength(2);
    expect(regions[0].outerLoop).toEqual(["el1"]);
    expect(regions[1].outerLoop).toEqual(["e1", "e2", "e3", "e4"]);
    expect(regions[1].holes).toEqual([["el1"]]);
  });

  it("stays its own region when it CROSSES the loop (semi-major reaches out)", () => {
    // Centre inside, but a = 15 pokes through the rect's 10-unit half-height.
    const regions = detectRegions([...rect, ell("el1", [20, 10], 15, 2)]);
    expect(regions).toHaveLength(2);
    expect(regions[1].holes).toEqual([]);
  });

  it("never closes a line loop by itself (no endpoints to walk)", () => {
    expect(orderedClosedLoop([ell("el1", [0, 0], 10, 4)])).toBeNull();
  });

  it("does not change the circle path — a nested circle still yields the same fill", () => {
    const regions = detectRegions([...rect, { id: "c", type: "Circle", center: [20, 10], radius: 4 }]);
    expect(regions).toHaveLength(2);
    expect(regions[1].holes).toEqual([["c"]]);
  });
});
