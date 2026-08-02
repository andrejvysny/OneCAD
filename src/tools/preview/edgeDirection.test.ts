import { describe, it, expect } from "vitest";
import { makeBoxMesh, makeCylinderMesh, encodeMesh1 } from "@/ipc/mockMeshes";
import { parseMeshPayload, type BodyMeshView } from "@/viewport/mesh/parseMeshPayload";
import { edgeMidAndTangent, edgeOutward, averageOutward, type Vec3 } from "./edgeDirection";

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
