/*
 * faceEdges — the mesh-derived face → boundary-edge expansion behind C7 (arming
 * Fillet / Chamfer from a face selection). The mock box's twelve edges and six
 * faces are an exact analytic answer to check against: `BOX_FACES` lists each
 * face's four corners and `BOX_EDGE_PAIRS` lists each edge's two, so the
 * expected set is derived here from the same tables the mesh is rendered from
 * rather than written out by hand.
 */
import { describe, it, expect } from "vitest";
import { chordalDeflection, faceBoundaryEdgeOrdinals } from "./faceEdges";
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

describe("chordalDeflection — mirrors the worker's tessellation policy", () => {
  it("is relative to the body, clamped at both ends, and finer per LOD", () => {
    expect(chordalDeflection(0, 100)).toBeCloseTo(0.5, 9); // 1% of 100, at the coarse cap
    expect(chordalDeflection(1, 20)).toBeCloseTo(0.05, 9); // 0.25% of 20, inside the band
    expect(chordalDeflection(2, 20)).toBeCloseTo(0.01, 9); // 0.05% of 20
    expect(chordalDeflection(2, 0)).toBeCloseTo(0.0001, 9); // floor, never zero
    expect(chordalDeflection(0, 1e6)).toBeCloseTo(0.5, 9); // cap
  });
});
