/*
 * mockMeshMetrics — the mock lane's mesh-derived geometry (WP-C1).
 *
 * The point of these cases is EXACTNESS, not plausibility. For a closed,
 * outward-wound triangle mesh the divergence theorem gives the enclosed volume,
 * centroid and inertia with no sampling error at all, so every expectation below
 * is the analytic answer for the mock's own box — computed from `BOX_SIZE`
 * rather than pasted, so resizing the seed body cannot leave a stale literal
 * passing.
 *
 * The cylinder cases pin the opposite property: the mock draws a FACETED 24-gon
 * prism, and the metrics must report that prism honestly rather than the smooth
 * cylinder it stands for.
 */
import { describe, it, expect } from "vitest";
import { BOX_SIZE, makeBoxMesh, makeCylinderMesh } from "./mockMeshes";
import {
  edgeMetricsFromMesh,
  faceMetricsFromMesh,
  massPropertiesFromMesh,
} from "./mockMeshMetrics";

const [SX, SY, SZ] = BOX_SIZE; // 80 × 60 × 30, centred at the origin

describe("massPropertiesFromMesh — the box is EXACT", () => {
  const mp = massPropertiesFromMesh("body1", makeBoxMesh());

  it("echoes the body id it was asked about", () => {
    expect(mp.bodyId).toBe("body1");
  });

  it("volume is a·b·c exactly", () => {
    // POSITIVE, which is also the winding check: an inward-wound mesh would sum
    // to −144000 and this assertion is what would catch it.
    expect(mp.volume).toBeCloseTo(SX * SY * SZ, 6);
  });

  it("surface area is 2(ab + ac + bc) exactly", () => {
    expect(mp.surfaceArea).toBeCloseTo(2 * (SX * SY + SX * SZ + SY * SZ), 6);
  });

  it("centroid is the origin — the seed box is centred there", () => {
    expect(mp.centroid[0]).toBeCloseTo(0, 9);
    expect(mp.centroid[1]).toBeCloseTo(0, 9);
    expect(mp.centroid[2]).toBeCloseTo(0, 9);
  });

  it("principal moments are V(a²+b²)/12 about the CENTROID", () => {
    // An about-ORIGIN answer would coincide here (the box IS centred), so the
    // translated case below is what actually pins the parallel-axis shift.
    const V = SX * SY * SZ;
    const want = [
      (V * (SY * SY + SZ * SZ)) / 12,
      (V * (SX * SX + SZ * SZ)) / 12,
      (V * (SX * SX + SY * SY)) / 12,
    ];
    for (let k = 0; k < 3; k++) expect(mp.principalMoments[k]).toBeCloseTo(want[k], 3);
  });

  it("principal axes are the world axes, unit and right-handed", () => {
    // An axis-aligned body has an already-diagonal inertia tensor, so Jacobi
    // performs zero rotations and the axes come out in world X, Y, Z order.
    expect(mp.principalAxes[0].map(Math.round)).toEqual([1, 0, 0]);
    expect(mp.principalAxes[1].map(Math.round)).toEqual([0, 1, 0]);
    expect(mp.principalAxes[2].map(Math.round)).toEqual([0, 0, 1]);
    for (const axis of mp.principalAxes) {
      expect(Math.hypot(axis[0], axis[1], axis[2])).toBeCloseTo(1, 12);
    }
    const [a1, a2, a3] = mp.principalAxes;
    const cross = [
      a1[1] * a2[2] - a1[2] * a2[1],
      a1[2] * a2[0] - a1[0] * a2[2],
      a1[0] * a2[1] - a1[1] * a2[0],
    ];
    expect(cross[0] * a3[0] + cross[1] * a3[1] + cross[2] * a3[2]).toBeCloseTo(1, 9);
  });
});

describe("massPropertiesFromMesh — translation behaves correctly", () => {
  // A box translated well off the origin. Volume and area are invariant; the
  // centroid moves with it; and the moments must NOT (they are about the
  // centroid). Omitting the parallel-axis shift would inflate these by
  // V·|offset|², i.e. by orders of magnitude here — this is the case that pins it.
  const offset: [number, number, number] = [200, -140, 90];
  const at = massPropertiesFromMesh("b", makeBoxMesh(SX, SY, SZ, 0, offset));
  const origin = massPropertiesFromMesh("b", makeBoxMesh());

  it("volume and area are translation-invariant", () => {
    expect(at.volume).toBeCloseTo(origin.volume, 4);
    expect(at.surfaceArea).toBeCloseTo(origin.surfaceArea, 4);
  });

  it("the centroid follows the body", () => {
    for (let i = 0; i < 3; i++) expect(at.centroid[i]).toBeCloseTo(offset[i], 4);
  });

  it("the moments are UNCHANGED — they are about the centroid, not the origin", () => {
    for (let k = 0; k < 3; k++) {
      expect(at.principalMoments[k]).toBeCloseTo(origin.principalMoments[k], 0);
    }
  });
});

describe("massPropertiesFromMesh — the cylinder is a FACETED prism, honestly", () => {
  const R = 25;
  const H = 60;
  const N = 24;
  const mp = massPropertiesFromMesh("body2", makeCylinderMesh(R, H, N));

  it("volume is the regular N-gon prism's, NOT π·r²·h", () => {
    const prism = 0.5 * N * R * R * Math.sin((2 * Math.PI) / N) * H;
    expect(mp.volume).toBeCloseTo(prism, 2);
    // Strictly inscribed, so strictly smaller — the mock must not silently
    // present a smooth-cylinder number it does not draw.
    expect(mp.volume).toBeLessThan(Math.PI * R * R * H);
  });

  it("surface area is the prism's lateral + two N-gon caps", () => {
    const side = 2 * R * Math.sin(Math.PI / N);
    const capArea = 0.5 * N * R * R * Math.sin((2 * Math.PI) / N);
    expect(mp.surfaceArea).toBeCloseTo(N * side * H + 2 * capArea, 2);
  });

  it("the centroid is on the axis, mid-height", () => {
    expect(mp.centroid[0]).toBeCloseTo(0, 6);
    expect(mp.centroid[1]).toBeCloseTo(0, 6);
    expect(mp.centroid[2]).toBeCloseTo(0, 6); // makeCylinderMesh straddles z = 0
  });
});

describe("faceMetricsFromMesh", () => {
  const box = makeBoxMesh();

  it("a box cap reports its exact area, plane normal and bbox centre", () => {
    // f:4 is the +Z top face (BOX_FACES).
    const top = faceMetricsFromMesh(box, "f:4");
    expect(top).not.toBeNull();
    expect(top!.area).toBeCloseTo(SX * SY, 4);
    expect(top!.normal[0]).toBeCloseTo(0, 9);
    expect(top!.normal[1]).toBeCloseTo(0, 9);
    expect(top!.normal[2]).toBeCloseTo(1, 9);
    expect(top!.center[2]).toBeCloseTo(SZ / 2, 6);
    expect(top!.planar).toBe(true);
  });

  it("each of the box's six faces is planar with a distinct axis normal", () => {
    const normals = ["f:0", "f:1", "f:2", "f:3", "f:4", "f:5"].map((id) => {
      const m = faceMetricsFromMesh(box, id);
      expect(m!.planar).toBe(true);
      return m!.normal.map(Math.round).join(",");
    });
    expect(new Set(normals).size).toBe(6);
  });

  it("adjacent box faces are PERPENDICULAR — the angle reading's real input", () => {
    const a = faceMetricsFromMesh(box, "f:4")!.normal; // +Z
    const b = faceMetricsFromMesh(box, "f:0")!.normal; // +X
    expect(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).toBeCloseTo(0, 9);
  });

  it("a CURVED face is reported non-planar", () => {
    // The cylinder's side is 24 quads with 24 different normals. Calling it
    // planar would let the measure tool treat it as a plane.
    const side = faceMetricsFromMesh(makeCylinderMesh(), "f:0");
    expect(side!.planar).toBe(false);
    // Its caps still are planar.
    expect(faceMetricsFromMesh(makeCylinderMesh(), "f:1")!.planar).toBe(true);
  });

  it("returns null for a face the mesh does not carry", () => {
    expect(faceMetricsFromMesh(box, "f:999")).toBeNull();
  });
});

describe("edgeMetricsFromMesh", () => {
  const box = makeBoxMesh();

  it("a box edge is straight and one of the three side lengths", () => {
    const e = edgeMetricsFromMesh(box, "e:0")!;
    expect(e.straight).toBe(true);
    expect([SX, SY, SZ].some((want) => Math.abs(e.length - want) < 1e-4)).toBe(true);
  });

  it("a cylinder rim polyline is NOT straight and approximates the circumference", () => {
    const rim = edgeMetricsFromMesh(makeCylinderMesh(25, 60, 24), "e:0")!;
    expect(rim.straight).toBe(false);
    // 24 chords inscribed in the circle — shorter than 2πr, by a known amount.
    expect(rim.length).toBeCloseTo(24 * 2 * 25 * Math.sin(Math.PI / 24), 3);
    expect(rim.length).toBeLessThan(2 * Math.PI * 25);
  });

  it("returns null for an edge the mesh does not carry", () => {
    expect(edgeMetricsFromMesh(box, "e:999")).toBeNull();
  });
});
