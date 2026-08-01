/*
 * mockFaceGeometry — the mock lane's analytic per-face answer (SKETCH-ON-FACE W2).
 *
 * These are STRUCTURE + basis pins, not geometric truth: the numbers here exist
 * so the mock lane cannot silently drift from the rendered box (`mockMeshes`'
 * shared corner/face tables) or from the backend's in-plane axis rule. Real
 * numeric truth for a projected face boundary lives in cargo/ctest.
 */
import { describe, expect, it } from "vitest";
import {
  lookupMockFace,
  mockElementHash,
  mockProjectedContent,
  planeFromPointNormal,
  worldToPlaneUv,
} from "./mockFaceGeometry";
import { BOX_SIZE, boxCorners } from "./mockMeshes";
import { detectRegions, solveDof } from "./mockSketch";

describe("planeFromPointNormal — the ported in-plane axis rule", () => {
  it("a +Z normal reproduces the named XY basis EXACTLY (the +X-seed fallback)", () => {
    // The lock-tested Rust rule: `z × n` vanishes for n = +Z, so it falls back to
    // `n × +X`. The operand ORDER is what makes this the XY basis rather than its
    // mirror — a sketch on a flat top face must be oriented like a sketch on XY.
    expect(planeFromPointNormal([0, 0, 15], [0, 0, 1])).toEqual({
      kind: "custom",
      origin: [0, 0, 15],
      xAxis: [0, 1, 0],
      yAxis: [-1, 0, 0],
      normal: [0, 0, 1],
    });
  });

  it("any other normal gets a HORIZONTAL x (level in world), y completing the frame", () => {
    const p = planeFromPointNormal([40, 0, 0], [1, 0, 0]);
    expect(p.xAxis).toEqual([0, 1, 0]);
    expect(p.xAxis[2]).toBe(0); // level
    expect(p.yAxis).toEqual([0, 0, 1]);
  });

  it("a degenerate normal falls back to XY rather than producing NaNs", () => {
    const p = planeFromPointNormal([1, 2, 3], [0, 0, 0]);
    expect(p.normal).toEqual([0, 0, 1]);
    expect(p.xAxis).toEqual([0, 1, 0]);
  });
});

describe("lookupMockFace — the mock box", () => {
  it("f:4 (top) pins its plane + corner UVs to the SHARED mockMeshes tables", () => {
    const found = lookupMockFace("body1", undefined, "f:4");
    expect(found.kind).toBe("planar");
    if (found.kind !== "planar") throw new Error("expected a planar face");

    // Origin = the top face centroid of the 80×60×30 box → +Z half-height.
    expect(found.geometry.plane).toEqual({
      kind: "custom",
      origin: [0, 0, BOX_SIZE[2] / 2],
      xAxis: [0, 1, 0],
      yAxis: [-1, 0, 0],
      normal: [0, 0, 1],
    });

    // The four corners in the plane's own (u,v) — derived from the corner table
    // makeBoxMesh emits vertices from, so a change to the box size or winding
    // moves BOTH and this stays honest.
    expect(found.geometry.boundary).toEqual({
      kind: "polygon",
      corners: [
        [-30, 40],
        [-30, -40],
        [30, -40],
        [30, 40],
      ],
    });

    // …and those UVs are exactly the shared corner table projected into the frame.
    const c = boxCorners(BOX_SIZE);
    expect(worldToPlaneUv(found.geometry.plane, c["001"])).toEqual([-30, 40]);
    expect(worldToPlaneUv(found.geometry.plane, c["111"])).toEqual([30, -40]);
  });

  it("every box face resolves planar, each with its own outward normal", () => {
    const normals = ["f:0", "f:1", "f:2", "f:3", "f:4", "f:5"].map((id) => {
      const found = lookupMockFace("body1", undefined, id);
      if (found.kind !== "planar") throw new Error(`${id} should be planar`);
      return found.geometry.plane.normal;
    });
    expect(normals).toEqual([
      [1, 0, 0],
      [-1, 0, 0],
      [0, 1, 0],
      [0, -1, 0],
      [0, 0, 1],
      [0, 0, -1],
    ]);
  });

  it("resolves a PROMOTED elementId by reversing the mock element hash", () => {
    // The mock mints `el_${hash(bodyId#topoKey)}`; a pick that has already been
    // promoted arrives with no topoKey, so the rung has to invert.
    const viaKey = lookupMockFace("body1", undefined, "f:4");
    if (viaKey.kind !== "planar") throw new Error("expected planar");
    // Re-derived through the public seam rather than hardcoding the hex.
    const elementId = `el_${mockElementHash("body1#f:4")}`;
    const viaId = lookupMockFace("body1", elementId, undefined);
    expect(viaId.kind).toBe("planar");
    if (viaId.kind !== "planar") throw new Error("expected planar");
    expect(viaId.faceId).toBe("f:4");
    expect(viaId.geometry.plane).toEqual(viaKey.geometry.plane);
  });
});

describe("lookupMockFace — refusals + limits", () => {
  it("the CYLINDER's curved side face is nonPlanar (the refusal path)", () => {
    expect(lookupMockFace("body2", undefined, "f:0")).toEqual({ kind: "nonPlanar", faceId: "f:0" });
  });

  it("the cylinder's CAPS are planar, bounded by a circle", () => {
    const top = lookupMockFace("body2", undefined, "f:1");
    if (top.kind !== "planar") throw new Error("expected the top cap to be planar");
    expect(top.geometry.plane.origin).toEqual([0, 0, 30]);
    expect(top.geometry.plane.normal).toEqual([0, 0, 1]);
    expect(top.geometry.boundary).toEqual({ kind: "circle", center: [0, 0], radius: 25 });

    const bottom = lookupMockFace("body2", undefined, "f:2");
    if (bottom.kind !== "planar") throw new Error("expected the bottom cap to be planar");
    expect(bottom.geometry.plane.normal).toEqual([0, 0, -1]);
  });

  it("a synthesized body / an unknown face id reports unknown, NOT a refusal", () => {
    // MOCK LIMIT: extruded bodies have no analytic entry; the caller keeps its
    // old fallback frame rather than refusing to sketch at all.
    expect(lookupMockFace("body_7", undefined, "f:0")).toEqual({ kind: "unknown" });
    expect(lookupMockFace("body1", undefined, "f:99")).toEqual({ kind: "unknown" });
    expect(lookupMockFace("body1", undefined, "e:3")).toEqual({ kind: "unknown" });
  });
});

describe("mockProjectedContent — the shape the REAL DTO carries", () => {
  it("a box face yields 4 LOCKED lines, no free points, and 4 Fixed pins", () => {
    const found = lookupMockFace("body1", undefined, "f:4");
    if (found.kind !== "planar") throw new Error("expected planar");
    const { entities, constraints } = mockProjectedContent(found.geometry);

    // Owned child points ride on their line (BUG-5 hydration rule) — a projected
    // rectangle must NOT hydrate as 4 lines + 4 stray dots.
    expect(entities.map((e) => e.type)).toEqual(["Line", "Line", "Line", "Line"]);
    expect(entities.every((e) => e.referenceLocked)).toBe(true);
    expect(entities.some((e) => e.construction)).toBe(false);

    // One Fixed per projected POINT, re-keyed onto its owning line + position.
    expect(constraints).toHaveLength(4);
    expect(constraints.every((c) => c.type === "Fixed")).toBe(true);
    for (const c of constraints) {
      expect(entities.some((e) => e.id === c.entities[0])).toBe(true);
      expect(c.positions?.[0]).toMatch(/^(Start|End)$/);
    }
  });

  it("the projected boundary is FULLY CONSTRAINED and bounds a region", () => {
    const found = lookupMockFace("body1", undefined, "f:4");
    if (found.kind !== "planar") throw new Error("expected planar");
    const { entities, constraints } = mockProjectedContent(found.geometry);

    // The reference-lock DOF rule: pinned locked geometry contributes 0 net DOF,
    // so a freshly opened sketch-on-face reads "fully constrained", not "DOF 8".
    expect(solveDof(entities, constraints)).toEqual({ dof: 0, status: "FullyConstrained" });

    // Locked geometry DOES bound regions (the deliberate contrast with
    // construction geometry) — otherwise the projected outline is decorative and
    // nothing can be extruded off the face.
    expect(detectRegions(entities)).toHaveLength(1);
  });

  it("a cylinder cap yields ONE locked circle pinned at its centre", () => {
    const found = lookupMockFace("body2", undefined, "f:1");
    if (found.kind !== "planar") throw new Error("expected planar");
    const { entities, constraints } = mockProjectedContent(found.geometry);
    expect(entities).toHaveLength(1);
    expect(entities[0]).toMatchObject({ type: "Circle", radius: 25, referenceLocked: true });
    expect(constraints).toHaveLength(1);
    expect(constraints[0]).toMatchObject({ type: "Fixed", entities: [entities[0].id] });
    expect(constraints[0].positions?.[0]).toBe("Center");
    expect(solveDof(entities, constraints)).toEqual({ dof: 0, status: "FullyConstrained" });
    expect(detectRegions(entities)).toHaveLength(1);
  });

  it("ids are deterministic and do not collide with the frontend's e<N>/c<N>", () => {
    const found = lookupMockFace("body1", undefined, "f:4");
    if (found.kind !== "planar") throw new Error("expected planar");
    const a = mockProjectedContent(found.geometry);
    const b = mockProjectedContent(found.geometry);
    expect(a.entities.map((e) => e.id)).toEqual(b.entities.map((e) => e.id));
    for (const e of a.entities) expect(e.id).not.toMatch(/^e\d+$/);
    for (const c of a.constraints) expect(c.id).not.toMatch(/^c\d+$/);
  });
});
