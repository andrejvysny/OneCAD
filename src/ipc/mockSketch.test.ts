import { describe, it, expect } from "vitest";
import {
  planeFor,
  solveDof,
  solveSketch,
  freeDegrees,
  closedCurveOf,
  orderedClosedLoop,
  mockRegionId,
  constraintFreedom,
  mockEntityStates,
} from "./mockSketch";
import { detectRegions } from "./mockRegions";
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

describe("solveSketch — solveDof + deterministic conflict precedence", () => {
  it("clean sketch: same dof/status as solveDof, empty conflicting", () => {
    const cs: SketchConstraint[] = [{ id: "c1", type: "Horizontal", entities: ["e1"] }];
    expect(solveSketch([rect[0]], cs)).toEqual({ dof: 3, status: "UnderConstrained", conflicting: [] });
  });

  it("Conflicting OUTRANKS the dof-derived OverConstrained status", () => {
    // Two Distance constraints on the same points with different values: an
    // OverConstrained-by-count sketch that is ALSO genuinely conflicting.
    const cs: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 },
      { id: "d2", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 999 },
    ];
    const result = solveSketch([rect[0]], cs);
    expect(result.status).toBe("Conflicting");
    expect(result.conflicting.sort()).toEqual(["d1", "d2"]);
  });

  it("Conflicting can fire even when the dof arithmetic alone reads UnderConstrained", () => {
    // Horizontal + Vertical on one line: only 2 dof removed by the coarse count
    // (4 free − 2 = 2, still "UnderConstrained" by count alone) but a genuine
    // geometric contradiction — Conflicting must win regardless of dof sign.
    const cs: SketchConstraint[] = [
      { id: "h1", type: "Horizontal", entities: ["e1"] },
      { id: "v1", type: "Vertical", entities: ["e1"] },
    ];
    const result = solveSketch([rect[0]], cs);
    expect(result.status).toBe("Conflicting");
    expect(result.conflicting.sort()).toEqual(["h1", "v1"]);
    expect(result.dof).toBe(solveDof([rect[0]], cs).dof); // dof arithmetic itself is untouched
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

describe("closedCurveOf — visual tessellation parity", () => {
  it("uses the shared high-quality sampling density for circles and ellipses", () => {
    const circle = closedCurveOf({ id: "c", type: "Circle", center: [0, 0], radius: 5 })!;
    const ellipse = closedCurveOf({
      id: "e",
      type: "Ellipse",
      center: [0, 0],
      majorR: 10,
      minorR: 4,
      rotation: 0,
    })!;
    expect(circle.polygon).toHaveLength(72);
    expect(ellipse.polygon).toHaveLength(72);
  });

  it("raises the polygon resolution for a large round profile", () => {
    const small = closedCurveOf({ id: "small", type: "Circle", center: [0, 0], radius: 5 })!;
    const large = closedCurveOf({ id: "large", type: "Circle", center: [0, 0], radius: 100 })!;
    expect(large.polygon.length).toBeGreaterThan(small.polygon.length);
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
    // The visual polygon slightly under-approximates the disc; a filled hole would be 800.
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

  it("subtracts EVERY hole from a multi-hole enclosing cell", () => {
    const regions = detectRegions([
      ...rect,
      { id: "c1", type: "Circle", center: [10, 10], radius: 3 },
      { id: "c2", type: "Circle", center: [30, 10], radius: 3 },
    ]);
    expect(regions).toHaveLength(3);
    const enclosing = regions.find((region) => region.holes.length > 0)!;
    expect(enclosing.holes).toEqual([["c1"], ["c2"]]);
    expect(enclosing.previewTriangles!.holesSubtracted).toBe(2);
    const tris = enclosing.previewTriangles!;
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
    // 800 − 2·π·3² ≈ 743.5 (visual polygon holes slightly under-approximate the discs).
    expect(area).toBeGreaterThan(741);
    expect(area).toBeLessThan(746);
  });
});

// Concave loop: the old centroid fan was exact only for star-shaped polygons —
// on an L-shape it filled (and hit-tested) the notch OUTSIDE the boundary, so
// the fill visibly disagreed with the drawn edges.
describe("detectRegions — concave loop fill stays inside the boundary", () => {
  const L: SketchEntity[] = [
    { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
    { id: "e2", type: "Line", p0: [40, 0], p1: [40, 20] },
    { id: "e3", type: "Line", p0: [40, 20], p1: [20, 20] },
    { id: "e4", type: "Line", p0: [20, 20], p1: [20, 40] },
    { id: "e5", type: "Line", p0: [20, 40], p1: [0, 40] },
    { id: "e6", type: "Line", p0: [0, 40], p1: [0, 0] },
  ];

  it("fill area equals the polygon area (no spill, no overlap)", () => {
    const regions = detectRegions(L);
    expect(regions).toHaveLength(1);
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
    expect(area).toBeCloseTo(1200, 6); // 40·40 − 20·20 notch
  });

  it("a point in the notch hit-tests to NO region", async () => {
    const { regionAtPoint } = await import("@/tools/preview/regionPick");
    const regions = detectRegions(L);
    expect(regionAtPoint(regions, 30, 30)).toBeNull(); // notch interior
    expect(regionAtPoint(regions, 10, 10)).toBe(regions[0].regionId); // material
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
    // The visual polygon slightly under-approximates π·10·4 ≈ 125.7.
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

/*
 * `mockEntityStates` — the MOCK lane's per-entity map (SCHEMA §7.4).
 *
 * The point of these tests is what the map does NOT say. This lane has no
 * Jacobian, so almost every entity's per-entity state is genuinely unknown, and
 * the honest encoding of unknown is ABSENCE from the map. Anything richer would
 * be the mock diagnosing itself and every consumer believing it.
 */
describe("mockEntityStates — the honest mock map (SCHEMA §7.4)", () => {
  const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({
    id,
    type: "Line",
    p0,
    p1,
  });

  it("says NOTHING about ordinary user geometry", () => {
    expect(mockEntityStates([line("e1", [0, 0], [10, 0])], [])).toEqual({});
  });

  it("NEVER derives fullyConstrained from dof === 0", () => {
    // A rectangle with enough coarse constraint arity to zero the heuristic:
    // the whole-sketch count says "defined", which says nothing about any one
    // entity, and a map built from it would be a fabricated diagnosis.
    const constraints: SketchConstraint[] = [
      { id: "c1", type: "Fixed", entities: ["e1"], positions: ["Start"] },
      { id: "c2", type: "Fixed", entities: ["e1"], positions: ["End"] },
      { id: "c3", type: "Fixed", entities: ["e2"], positions: ["Start"] },
      { id: "c4", type: "Fixed", entities: ["e2"], positions: ["End"] },
      { id: "c5", type: "Fixed", entities: ["e3"], positions: ["Start"] },
      { id: "c6", type: "Fixed", entities: ["e3"], positions: ["End"] },
      { id: "c7", type: "Fixed", entities: ["e4"], positions: ["Start"] },
      { id: "c8", type: "Fixed", entities: ["e4"], positions: ["End"] },
    ];
    expect(solveDof(rect, constraints).dof).toBe(0); // guard: the premise holds
    expect(mockEntityStates(rect, constraints)).toEqual({});
  });

  it("reports fullyConstrained for referenceLocked geometry only", () => {
    const entities: SketchEntity[] = [
      { ...line("locked", [0, 0], [10, 0]), referenceLocked: true },
      line("mine", [0, 5], [10, 5]),
    ];
    expect(mockEntityStates(entities, [])).toEqual({ locked: "fullyConstrained" });
  });

  it("projects a PROVABLE conflict onto every entity the clashing constraints name", () => {
    const entities = [line("e1", [0, 0], [40, 0])];
    // R1: two Distances on the same target with different values.
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 },
      { id: "d2", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 120 },
    ];
    expect(mockEntityStates(entities, constraints)).toEqual({ e1: "conflicting" });
  });

  it("OVER-attributes across both operands, exactly as §7.4 specifies", () => {
    const entities = [line("a", [0, 0], [10, 0]), line("b", [0, 5], [10, 5])];
    // R3: Parallel + Perpendicular on the same pair names both lines.
    const constraints: SketchConstraint[] = [
      { id: "p", type: "Parallel", entities: ["a", "b"] },
      { id: "q", type: "Perpendicular", entities: ["a", "b"] },
    ];
    expect(mockEntityStates(entities, constraints)).toEqual({ a: "conflicting", b: "conflicting" });
  });

  it("conflicting OUTRANKS fullyConstrained on the same entity", () => {
    const entities: SketchEntity[] = [
      { ...line("locked", [0, 0], [10, 0]), referenceLocked: true },
      line("b", [0, 5], [10, 5]),
    ];
    const constraints: SketchConstraint[] = [
      { id: "p", type: "Parallel", entities: ["locked", "b"] },
      { id: "q", type: "Perpendicular", entities: ["locked", "b"] },
    ];
    expect(mockEntityStates(entities, constraints)).toEqual({ locked: "conflicting", b: "conflicting" });
  });

  it("names no entity that is not in the sketch", () => {
    const constraints: SketchConstraint[] = [
      { id: "p", type: "Parallel", entities: ["a", "ghost"] },
      { id: "q", type: "Perpendicular", entities: ["a", "ghost"] },
    ];
    expect(mockEntityStates([line("a", [0, 0], [10, 0])], constraints)).toEqual({ a: "conflicting" });
  });
});
