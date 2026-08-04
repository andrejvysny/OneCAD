import { describe, it, expect } from "vitest";
import type { SketchRegion } from "@/ipc/types";
import {
  profileFromRegion,
  profileBounds,
  prismLocal,
  unitPrismGeometry,
} from "./prismPreview";

/** A 40×40 square region centred at (10, 10), fan-triangulated like mockSketch. */
function squareRegion(): SketchRegion {
  const ring: [number, number][] = [
    [-10, -10],
    [30, -10],
    [30, 30],
    [-10, 30],
  ];
  const positions = [10, 10, ...ring.flat()]; // centroid first, then ring
  const indices: number[] = [];
  for (let i = 0; i < ring.length; i++) indices.push(0, 1 + i, 1 + ((i + 1) % ring.length));
  return { regionId: "r_test", outerLoop: ["e1"], holes: [], previewTriangles: { positions, indices } };
}

describe("profileFromRegion", () => {
  it("extracts the boundary ring (excluding the centroid) + the cap fan", () => {
    const p = profileFromRegion(squareRegion())!;
    expect(p).not.toBeNull();
    expect(p.ring).toEqual([
      [-10, -10],
      [30, -10],
      [30, 30],
      [-10, 30],
    ]);
    // The cap keeps the full fan (centroid at index 0).
    expect(p.cap.positions.slice(0, 2)).toEqual([10, 10]);
  });

  it("returns null when there is no usable triangulation", () => {
    expect(profileFromRegion({ regionId: "r", outerLoop: [], holes: [] })).toBeNull();
  });

  // REAL-worker lane layout (SolverLane ear_clip): polygon vertices in loop
  // order, NO centroid hub, ear-clip indices. The old fan-convention reader
  // dropped vertex 0 here and produced the twisted-prism preview defect.
  it("extracts the full ring from an ear-clip (hub-less) triangulation", () => {
    const region: SketchRegion = {
      regionId: "r_real",
      outerLoop: ["e1"],
      holes: [],
      previewTriangles: {
        positions: [0, 0, 40, 0, 40, 20, 0, 20],
        indices: [0, 1, 2, 0, 2, 3],
      },
    };
    const p = profileFromRegion(region)!;
    expect(p).not.toBeNull();
    expect(p.ring).toHaveLength(4); // ALL four corners — nothing dropped
    expect(p.ring).toEqual(
      expect.arrayContaining([
        [0, 0],
        [40, 0],
        [40, 20],
        [0, 20],
      ]),
    );
    const b = profileBounds(p);
    expect(b.centroidU).toBeCloseTo(20);
    expect(b.centroidV).toBeCloseTo(10);
  });

  it("orients a clockwise-wound triangulation's ring CCW", () => {
    const region: SketchRegion = {
      regionId: "r_cw",
      outerLoop: ["e1"],
      holes: [],
      previewTriangles: {
        positions: [0, 0, 40, 0, 40, 20, 0, 20],
        indices: [2, 1, 0, 3, 2, 0], // CW winding
      },
    };
    const ring = profileFromRegion(region)!.ring;
    let area = 0;
    for (let i = 0; i < ring.length; i++) {
      const [x0, y0] = ring[i];
      const [x1, y1] = ring[(i + 1) % ring.length];
      area += x0 * y1 - x1 * y0;
    }
    expect(area).toBeGreaterThan(0); // CCW as prismLocal assumes
  });

  it("rejects undeclared extra boundary loops", () => {
    // A big quad plus a disjoint sliver triangle is not one declared planar cell.
    const region: SketchRegion = {
      regionId: "r_multi",
      outerLoop: ["e1"],
      holes: [],
      previewTriangles: {
        positions: [0, 0, 40, 0, 40, 40, 0, 40, 100, 100, 101, 100, 100, 101],
        indices: [0, 1, 2, 0, 2, 3, 4, 5, 6],
      },
    };
    expect(profileFromRegion(region)).toBeNull();
  });
});

describe("profileBounds", () => {
  it("computes (u,v) bounds + centroid", () => {
    const b = profileBounds(profileFromRegion(squareRegion())!);
    expect(b).toEqual({ minU: -10, maxU: 30, minV: -10, maxV: 30, centroidU: 10, centroidV: 10 });
  });
});

describe("prismLocal", () => {
  it("puts the bottom cap at z=0 and the top cap at z=depth", () => {
    const prism = prismLocal(profileFromRegion(squareRegion())!, 5);
    let minZ = Infinity;
    let maxZ = -Infinity;
    for (let i = 2; i < prism.positions.length; i += 3) {
      minZ = Math.min(minZ, prism.positions[i]);
      maxZ = Math.max(maxZ, prism.positions[i]);
    }
    expect(minZ).toBe(0);
    expect(maxZ).toBe(5);
  });

  it("emits bottom, top and side faces plus boundary edges", () => {
    const prism = prismLocal(profileFromRegion(squareRegion())!, 3);
    expect(prism.faces).toHaveLength(3); // bottom, top, sides
    // A 4-vertex ring ⇒ 4 side quads = 8 triangles.
    expect(prism.faces[2].triangles).toHaveLength(8);
    // Top + bottom loop + verticals (ring ≤ 12 ⇒ one per vertex).
    expect(prism.edges.length).toBe(2 + 4);
  });
});

describe("unitPrismGeometry", () => {
  it("builds a unit-depth (z ∈ [0,1]) merged indexed geometry", () => {
    const g = unitPrismGeometry(profileFromRegion(squareRegion())!);
    expect(g.positions).toBeInstanceOf(Float32Array);
    expect(g.indices).toBeInstanceOf(Uint32Array);
    expect(g.indices.length % 3).toBe(0);
    let maxZ = -Infinity;
    for (let i = 2; i < g.positions.length; i += 3) maxZ = Math.max(maxZ, g.positions[i]);
    expect(maxZ).toBe(1); // caller scales z by the live depth
  });
});

// ── Holes ───────────────────────────────────────────────────────────────────
//
// The kernel subtracts a region's holes (FaceBuilder adds each innerLoop as a
// hole wire), so the preview must too. The producer of the triangulation
// (worker `loop/PolygonFill.cpp`, mock `loopTriangles`) merges holes with
// BRIDGES that stay interior, leaving the outer and hole boundaries as the only
// single-use edges — which is what the ring derivation below recovers.

/**
 * A 40×20 rectangle with a 10×10 square hole, triangulated as a bridged loop
 * exactly the way the worker emits it: vertices 0-3 outer (CCW), 4-7 hole (CW
 * after normalization), and a bridge from outer[0] to hole[0] that is used by
 * two triangles. Built here by stitching the two rings so the topology — not a
 * hand-written index list — is what the assertions exercise.
 */
function rectWithSquareHoleRegion(): SketchRegion {
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
  // Stitch outer[i]..outer[i+1]..hole[i] and outer[i+1]..hole[i+1]..hole[i]:
  // every connecting edge is shared, both rings stay single-use.
  const indices: number[] = [];
  for (let i = 0; i < 4; i++) {
    const o0 = i;
    const o1 = (i + 1) % 4;
    const h0 = 4 + i;
    const h1 = 4 + ((i + 1) % 4);
    indices.push(o0, o1, h0);
    indices.push(o1, h1, h0);
  }
  return {
    regionId: "r_hole",
    outerLoop: ["e1"],
    holes: [["e2"]],
    previewTriangles: { positions, indices, holesSubtracted: 1 },
  };
}

describe("profileFromRegion with holes", () => {
  it("rejects a fill that did not subtract every declared hole", () => {
    const region = rectWithSquareHoleRegion();
    region.previewTriangles = { ...region.previewTriangles!, holesSubtracted: 0 };
    expect(profileFromRegion(region)).toBeNull();
  });

  it("rejects hole metadata without a matching boundary loop", () => {
    const region = squareRegion();
    region.holes = [["e2"]];
    region.previewTriangles = { ...region.previewTriangles!, holesSubtracted: 1 };
    expect(profileFromRegion(region)).toBeNull();
  });

  it("recovers the outer ring AND the hole ring from triangulation topology", () => {
    const p = profileFromRegion(rectWithSquareHoleRegion())!;
    expect(p).not.toBeNull();
    // Largest-|area| loop is the outer boundary.
    expect(p.ring).toHaveLength(4);
    expect(new Set(p.ring.map(String))).toEqual(
      new Set([
        [0, 0],
        [40, 0],
        [40, 20],
        [0, 20],
      ].map(String)),
    );
    expect(p.holes).toHaveLength(1);
    expect(p.holes[0]).toHaveLength(4);
    expect(new Set(p.holes[0].map(String))).toEqual(
      new Set([
        [15, 5],
        [25, 5],
        [25, 15],
        [15, 15],
      ].map(String)),
    );
  });

  it("orients the outer ring CCW and the hole ring CW", () => {
    const p = profileFromRegion(rectWithSquareHoleRegion())!;
    const area = (r: [number, number][]): number => {
      let a = 0;
      for (let i = 0; i < r.length; i++) {
        const [x0, y0] = r[i];
        const [x1, y1] = r[(i + 1) % r.length];
        a += x0 * y1 - x1 * y0;
      }
      return a / 2;
    };
    expect(area(p.ring)).toBeGreaterThan(0); // CCW — prismLocal assumes it
    expect(area(p.holes[0])).toBeLessThan(0); // CW — flips the wall winding
  });

  it("reports no holes for a hole-free region (mock fan layout unchanged)", () => {
    expect(profileFromRegion(squareRegion())!.holes).toEqual([]);
  });
});

describe("prismLocal with holes", () => {
  it("adds an inner wall per hole, with normals pointing INTO the hole", () => {
    const p = profileFromRegion(rectWithSquareHoleRegion())!;
    const prism = prismLocal(p, 7);
    // 4 outer + 4 hole ring vertices ⇒ 8 side quads ⇒ 16 triangles.
    expect(prism.faces[2].triangles).toHaveLength(16);

    // The hole wall's normal at a vertex must point toward the hole centre
    // (20,10) — i.e. away from the material — while the outer wall's points away
    // from the profile centre.
    const idx = (u: number, v: number, w: number): number => {
      for (let i = 0; i < prism.positions.length / 3; i++) {
        if (
          Math.abs(prism.positions[i * 3] - u) < 1e-9 &&
          Math.abs(prism.positions[i * 3 + 1] - v) < 1e-9 &&
          Math.abs(prism.positions[i * 3 + 2] - w) < 1e-9 &&
          Math.abs(prism.normals[i * 3 + 2]) < 1e-9 // a side vertex, not a cap
        ) {
          return i;
        }
      }
      return -1;
    };
    const holeCorner = idx(15, 5, 0); // bottom-left of the hole
    expect(holeCorner).toBeGreaterThanOrEqual(0);
    // Corner (15,5) relative to hole centre (20,10) is (-5,-5); pointing INTO
    // the hole means the normal is +u,+v.
    expect(prism.normals[holeCorner * 3]).toBeGreaterThan(0);
    expect(prism.normals[holeCorner * 3 + 1]).toBeGreaterThan(0);

    const outerCorner = idx(0, 0, 0);
    expect(outerCorner).toBeGreaterThanOrEqual(0);
    // Corner (0,0) relative to the profile centre (20,10) is (-20,-10); pointing
    // OUT of the material means the normal is -u,-v.
    expect(prism.normals[outerCorner * 3]).toBeLessThan(0);
    expect(prism.normals[outerCorner * 3 + 1]).toBeLessThan(0);
  });

  it("keeps the caps hole-free — cap area is outer minus hole", () => {
    const p = profileFromRegion(rectWithSquareHoleRegion())!;
    const prism = prismLocal(p, 1);
    // Cap triangles come straight from the producer's triangulation, so their
    // summed area is already 800 - 100 = 700. A filled hole would read 800.
    let area = 0;
    for (const [a, b, c] of prism.faces[1].triangles) {
      const ax = prism.positions[a * 3];
      const ay = prism.positions[a * 3 + 1];
      const bx = prism.positions[b * 3];
      const by = prism.positions[b * 3 + 1];
      const cx = prism.positions[c * 3];
      const cy = prism.positions[c * 3 + 1];
      area += Math.abs(((bx - ax) * (cy - ay) - (by - ay) * (cx - ax)) / 2);
    }
    expect(area).toBeCloseTo(700, 9);
  });

  it("emits edge loops for the hole boundary as well as the outer boundary", () => {
    const p = profileFromRegion(rectWithSquareHoleRegion())!;
    const prism = prismLocal(p, 2);
    // Per ring: bottom loop + top loop + one vertical per vertex (ring ≤ 12).
    expect(prism.edges.length).toBe((2 + 4) * 2);
  });
});
