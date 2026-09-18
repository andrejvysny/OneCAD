/*
 * faceEdges — the mesh-derived face → boundary-edge expansion behind C7 (arming
 * Fillet / Chamfer from a face selection). The mock box's twelve edges and six
 * faces are an exact analytic answer to check against: `BOX_FACES` lists each
 * face's four corners and `BOX_EDGE_PAIRS` lists each edge's two, so the
 * expected set is derived here from the same tables the mesh is rendered from
 * rather than written out by hand.
 */
import { describe, it, expect } from "vitest";
import { chordalDeflection, faceBoundaryEdgeOrdinals, retainedRimNeighbours } from "./faceEdges";
import { parseMeshPayload } from "./parseMeshPayload";
import { BOX_FACES, BOX_EDGE_PAIRS, encodeMesh1, makeBoxMesh } from "@/ipc/mockMeshes";

const box = () => parseMeshPayload(makeBoxMesh());

/** The edges whose BOTH corners are corners of this face — the exact answer. */
function analyticEdges(faceOrdinal: number): number[] {
  const corners = new Set<string>(BOX_FACES[faceOrdinal].corners);
  return BOX_EDGE_PAIRS.flatMap(([a, b], i) =>
    corners.has(a) && corners.has(b) ? [i] : [],
  );
}

describe("faceBoundaryEdgeOrdinals — the mock box", () => {
  it("finds exactly the four bounding edges of every face", () => {
    const view = box();
    for (let f = 0; f < BOX_FACES.length; f++) {
      const expected = analyticEdges(f);
      expect(expected).toHaveLength(4); // guard: the analytic answer is real
      expect(faceBoundaryEdgeOrdinals(view, f)).toEqual(expected);
    }
  });

  it("names the +X face's edges explicitly", () => {
    // f:0 is the +X face; e:1/e:5 run along its bottom and top, e:9/e:10 up its
    // two sides. The eight edges on the far side of the box are 60–80 mm away.
    expect(faceBoundaryEdgeOrdinals(box(), 0)).toEqual([1, 5, 9, 10]);
  });

  it("is empty for an out-of-range face and for a mesh with no edges", () => {
    const view = box();
    expect(faceBoundaryEdgeOrdinals(view, -1)).toEqual([]);
    expect(faceBoundaryEdgeOrdinals(view, view.faceCount)).toEqual([]);
    const noEdges = parseMeshPayload(
      encodeMesh1({
        positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
        faces: [{ triangles: [[0, 1, 2]], id: "f:0" }],
      }),
    );
    expect(faceBoundaryEdgeOrdinals(noEdges, 0)).toEqual([]);
  });

  it("rejects an edge that only TOUCHES the face at one corner", () => {
    // e:0 runs 000→100: its 100 end is a corner of f:0, its 000 end is 80 mm off.
    expect(faceBoundaryEdgeOrdinals(box(), 0)).not.toContain(0);
  });
});

/*
 * H9 — the retained-wall rim search Shell attaches its thickness handle to
 * (docs/design/astra/modeling-handle-attachment.md §5 "Shell").
 */
describe("retainedRimNeighbours — the walls left standing around a removed face", () => {
  /** A 40 mm cube spanning [0,40]³, the derivation's open-box case. */
  const openBox = (lod = 0) => parseMeshPayload(makeBoxMesh(40, 40, 40, lod, [20, 20, 20]));

  it("finds one side wall per top rim edge, and never the floor", () => {
    // f:4 is the +Z lid; its four rim edges are e:4..e:7, each shared with ONE
    // side wall. The floor f:5 is 40 mm away and is not a rim neighbour at all.
    expect(retainedRimNeighbours(openBox(), [4])).toEqual([
      { faceOrdinal: 0, edgeOrdinal: 5 },
      { faceOrdinal: 1, edgeOrdinal: 7 },
      { faceOrdinal: 2, edgeOrdinal: 6 },
      { faceOrdinal: 3, edgeOrdinal: 4 },
    ]);
    expect(retainedRimNeighbours(openBox(), [4]).map((n) => n.faceOrdinal)).not.toContain(5);
  });

  it("never reports the removed faces themselves", () => {
    for (const n of retainedRimNeighbours(openBox(), [4, 5])) {
      expect([4, 5]).not.toContain(n.faceOrdinal);
    }
  });

  /*
   * THE COUNTEREXAMPLE (§7 "Mesh adjacency falsely identifies a wall"). A 40 mm
   * cube at fine LOD has an adjacency tolerance of 2δ = 0.069282 mm, so a face
   * 0.05 mm off passes the same distance test as the real wall. A unique mesh
   * match is not a BRep proof, and TWO matches are not a choice — the edge is
   * discarded rather than resolved by picking one.
   */
  function twoWalls(includeImpostor: boolean) {
    const lid = [0, 0, 40, 40, 0, 40, 40, 40, 40, 0, 40, 40];
    const wall = [40, 0, 40, 40, 40, 40, 40, 40, 0, 40, 0, 0];
    const impostor = wall.map((v, i) => (i % 3 === 0 ? v + 0.05 : v));
    const floor = [0, 0, 0, 40, 0, 0, 40, 40, 0, 0, 40, 0];
    const positions = [...lid, ...wall, ...(includeImpostor ? impostor : []), ...floor];
    const quad = (base: number) => [
      [base, base + 1, base + 2] as [number, number, number],
      [base, base + 2, base + 3] as [number, number, number],
    ];
    const faces = [0, 4, ...(includeImpostor ? [8, 12] : [8])].map((base, i) => ({
      triangles: quad(base),
      id: `f:${i}`,
    }));
    return parseMeshPayload(
      encodeMesh1({
        positions,
        faces,
        // The lid's +X rim edge: it bounds the real wall exactly, and the
        // impostor within the tessellation tolerance.
        edges: [{ id: "e:0", points: [[40, 0, 40], [40, 40, 40]] }],
        lod: 2,
      }),
    );
  }

  it("resolves the rim when exactly one retained face shares the edge", () => {
    expect(retainedRimNeighbours(twoWalls(false), [0])).toEqual([
      { faceOrdinal: 1, edgeOrdinal: 0 },
    ]);
  });

  it("refuses an edge two retained faces match inside the mesh tolerance", () => {
    expect(retainedRimNeighbours(twoWalls(true), [0])).toEqual([]);
  });

  it("is empty for a mesh with no edges and for an out-of-range removed face", () => {
    const noEdges = parseMeshPayload(
      encodeMesh1({
        positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
        faces: [{ triangles: [[0, 1, 2]], id: "f:0" }],
      }),
    );
    expect(retainedRimNeighbours(noEdges, [0])).toEqual([]);
    expect(retainedRimNeighbours(openBox(), [99])).toEqual([]);
    expect(retainedRimNeighbours(openBox(), [])).toEqual([]);
  });
});

describe("chordalDeflection — mirrors the worker's tessellation policy", () => {
  it("is relative to the body, clamped at both ends, and finer per LOD", () => {
    expect(chordalDeflection(0, 100)).toBeCloseTo(0.5, 9); // 1% of 100, at the coarse cap
    expect(chordalDeflection(1, 20)).toBeCloseTo(0.05, 9); // 0.25% of 20, inside the band
    expect(chordalDeflection(2, 20)).toBeCloseTo(0.01, 9); // 0.05% of 20
    expect(chordalDeflection(2, 0)).toBeCloseTo(0.0001, 9); // floor, never zero
    expect(chordalDeflection(0, 1e6)).toBeCloseTo(0.5, 9); // cap
  });
});
