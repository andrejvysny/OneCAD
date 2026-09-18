import { describe, it, expect } from "vitest";
import { makeBoxMesh, makeCylinderMesh, encodeMesh1 } from "@/ipc/mockMeshes";
import { parseMeshPayload, type BodyMeshView } from "@/viewport/mesh/parseMeshPayload";
import {
  edgeMidAndTangent,
  edgeOutward,
  edgeOutwardAt,
  edgePolylinePoint,
  averageOutward,
  type Vec3,
} from "./edgeDirection";

const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const ORIGIN: Vec3 = [0, 0, 0];

// Box is centred at the origin, so "dot(outward, mid - centre)" reduces to
// "dot(outward, mid)" throughout — kept explicit below for readability.
const boxView = parseMeshPayload(makeBoxMesh());

describe("edgeOutward — box (T1 bisector)", () => {
  it("e:0 (bottom-front, between (0,-1,0) and (0,0,-1)) is EXACTLY (0, -√½, -√½) — signed", () => {
    const r = edgeOutward(boxView, 0)!;
    expect(r.source).toBe("bisector");
    expect(r.outward[0]).toBeCloseTo(0, 6);
    expect(r.outward[1]).toBeCloseTo(-Math.SQRT1_2, 6);
    expect(r.outward[2]).toBeCloseTo(-Math.SQRT1_2, 6);
    expect(r.mid).toEqual([0, -30, -15]);
  });

  it("all 12 box edges resolve via bisector, always pointing away from the body", () => {
    for (let i = 0; i < 12; i++) {
      const r = edgeOutward(boxView, i);
      expect(r).not.toBeNull();
      expect(r!.source).toBe("bisector");
      expect(dot(r!.outward, sub(r!.mid, ORIGIN))).toBeGreaterThan(0);
    }
  });

  it("falls back to bbox when NORMALS is stripped, still positive vs. centre", () => {
    const stripped: BodyMeshView = { ...boxView, normals: null, hasNormals: false };
    const r = edgeOutward(stripped, 0)!;
    expect(r.source).toBe("bbox");
    expect(dot(r.outward, sub(r.mid, ORIGIN))).toBeGreaterThan(0);
  });

  it("an edgeless view yields no midpoint/tangent and no outward direction", () => {
    const edgeless = parseMeshPayload(
      encodeMesh1({
        positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
        normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
        faces: [{ triangles: [[0, 1, 2]], id: "f:0" }],
      }),
    );
    expect(edgeMidAndTangent(edgeless, 0)).toBeNull();
    expect(edgeOutward(edgeless, 0)).toBeNull();
  });
});

describe("averageOutward", () => {
  it("two ADJACENT edges (sharing face f:5) average non-null, between their bisectors", () => {
    const a = edgeOutward(boxView, 0)!.outward; // shares f:3, f:5
    const b = edgeOutward(boxView, 1)!.outward; // shares f:0, f:5
    const avg = averageOutward([a, b]);
    expect(avg).not.toBeNull();
    expect(dot(avg!, a)).toBeGreaterThan(0);
    expect(dot(avg!, b)).toBeGreaterThan(0);
  });

  it("two OPPOSITE parallel edges (e:0 vs. top-back e:6) cancel to null by construction", () => {
    const a = edgeOutward(boxView, 0)!.outward;
    const b = edgeOutward(boxView, 6)!.outward;
    expect(averageOutward([a, b])).toBeNull();
  });
});

describe("edgeOutward — cylinder cap-rim edge (honest T1→T2 degradation)", () => {
  const cylView = parseMeshPayload(makeCylinderMesh());

  it("side face's smooth normals fail the plane test (<2 kept) → bbox, still convex-positive", () => {
    const r = edgeOutward(cylView, 0)!; // e:0 = top circle rim
    expect(r.source).toBe("bbox");
    const center: Vec3 = [
      (cylView.bboxMin[0] + cylView.bboxMax[0]) / 2,
      (cylView.bboxMin[1] + cylView.bboxMax[1]) / 2,
      (cylView.bboxMin[2] + cylView.bboxMax[2]) / 2,
    ];
    expect(dot(r.outward, sub(r.mid, center))).toBeGreaterThan(0);
  });
});

function sub(a: Vec3, b: Vec3): Vec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}

/*
 * H8 — the representative attachment POINT on one edge's display polyline
 * (docs/design/astra/modeling-handle-attachment.md §5 "Fillet / Chamfer", step 2).
 * A chain is never averaged: one edge, one point on it.
 */
describe("edgePolylinePoint", () => {
  /** A quarter-circle rim sampled at 0°, 45°, 90°, radius 20, at z = 20. */
  const quarterView = (): BodyMeshView =>
    parseMeshPayload(
      encodeMesh1({
        positions: [0, 0, 20, 20, 0, 20, 0, 20, 20],
        normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
        faces: [{ triangles: [[0, 1, 2]], id: "f:0" }],
        edges: [
          {
            id: "e:0",
            points: [
              [20, 0, 20],
              [20 * Math.SQRT1_2, 20 * Math.SQRT1_2, 20],
              [0, 20, 20],
            ],
          },
          { id: "e:1", points: [[0, 0, 0], [10, 0, 0]] },
        ],
      }),
    );

  it("with no anchor is the polyline's ARC-LENGTH midpoint, not the chord midpoint", () => {
    // §6 "Four-edge tangent chain": the first representative sits at 45°,
    // E = (14.1421, 14.1421, 20). The chord midpoint of the whole edge would be
    // (10, 10, 20) — a point that is not on the rim at all.
    const p = edgePolylinePoint(quarterView(), 0, null)!;
    expect(p[0]).toBeCloseTo(14.142136, 6);
    expect(p[1]).toBeCloseTo(14.142136, 6);
    expect(p[2]).toBe(20);
  });

  it("with an anchor is the closest point ON the polyline to it", () => {
    const view = quarterView();
    const near = edgePolylinePoint(view, 0, [30, 1, 20])!;
    expect(near[0]).toBeCloseTo(20, 6);
    expect(near[1]).toBeCloseTo(0, 6);
    // Interior of a segment, not one of its sampled vertices.
    const mid = edgePolylinePoint(view, 1, [5, 9, 0])!;
    expect(mid).toEqual([5, 0, 0]);
  });

  it("is the chord midpoint for the straight two-point edges a box ships", () => {
    expect(edgePolylinePoint(boxView, 0, null)).toEqual(edgeMidAndTangent(boxView, 0)!.mid);
  });

  it("refuses an edgeless view and an out-of-range ordinal", () => {
    expect(edgePolylinePoint(quarterView(), 9, null)).toBeNull();
    const stripped: BodyMeshView = { ...boxView, hasEdges: false };
    expect(edgePolylinePoint(stripped, 0, null)).toBeNull();
  });
});

describe("edgeOutwardAt — the tiers resolved at an arbitrary point of the edge", () => {
  it("agrees with edgeOutward at the edge's own midpoint", () => {
    const whole = edgeOutward(boxView, 0)!;
    const at = edgeOutwardAt(boxView, whole.mid)!;
    expect(at.source).toBe(whole.source);
    expect(at.outward[0]).toBeCloseTo(whole.outward[0], 12);
    expect(at.outward[1]).toBeCloseTo(whole.outward[1], 12);
    expect(at.outward[2]).toBeCloseTo(whole.outward[2], 12);
  });

  it("degrades to the bbox tier off any face plane, and refuses the bbox centre", () => {
    const stripped: BodyMeshView = { ...boxView, normals: null, hasNormals: false };
    expect(edgeOutwardAt(stripped, [0, -30, -15])!.source).toBe("bbox");
    expect(edgeOutwardAt(stripped, [0, 0, 0])).toBeNull();
  });
});
