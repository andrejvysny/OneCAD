/*
 * alignSolve — the face-to-face ALIGN solve (WP-B W2.5).
 *
 * Two claims are pinned here and nothing else can pin them:
 *
 *   FLUSH IS EXACT. The whole feature is one equation, so every case below
 *   re-applies the solved placement and checks the two things it promised —
 *   the moving centre lands ON the destination centre, and the moving normal
 *   lands ANTI-PARALLEL to the destination normal — to 1e-12. Asserting the
 *   returned numbers instead would only prove the function returns what it
 *   computed.
 *
 *   NO CONFIGURATION PRODUCES NaN. The three degenerate ones (parallel,
 *   anti-parallel, zero-length) are each negative-checked, not merely exercised:
 *   a solve that returns NaN would sail through a `toBeCloseTo` on a value that
 *   is also NaN, so `Number.isFinite` is asserted explicitly.
 *
 * The planarity refusal is checked against a REAL tessellated cylinder from
 * `mockMeshes` rather than a hand-built curved face — that is the same encoder
 * the mock lane feeds the whole app through, so a face the e2e lane could click
 * is the face being refused.
 */
import { describe, it, expect } from "vitest";
import {
  alignPlacement,
  composePlacements,
  faceFrame,
  inversePlacement,
  minimalRotation,
  stablePerpendicular,
  transformFrame,
  PLANARITY_TOL_RAD,
  type PlanarFaceFrame,
} from "./alignSolve";
import {
  applyPlacementToNormal,
  applyPlacementToPoint,
  placementMatrix,
} from "./patternPreview";
import type { Vec3 } from "./depthProjection";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { encodeMesh1, makeBoxMesh, makeCylinderMesh, BOX_FACES } from "@/ipc/mockMeshes";

const view = (blob: ArrayBuffer) => parseMeshPayload(blob);
const boxView = () => view(makeBoxMesh()); // 80×60×30 centred on the origin
const unit = (v: Vec3): Vec3 => {
  const l = Math.hypot(...v);
  return [v[0] / l, v[1] / l, v[2] / l];
};
const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

/** Ordinal of a mock-box face by its id (`f:0`…`f:5`). */
const boxFace = (id: string): number => BOX_FACES.findIndex((f) => f.id === id);

/**
 * Apply a solved placement to `moving` and return the resulting frame. This is
 * the SAME `placementMatrix` the ghost, the mock's `transformMesh1` and the
 * worker's `TransformBody` all evaluate, so "flush" here means flush on screen.
 */
function applySolution(
  s: NonNullable<ReturnType<typeof alignPlacement>>,
  frame: PlanarFaceFrame,
): PlanarFaceFrame {
  const m = placementMatrix(s.translate, s.rotate.center, s.rotate.axis, s.rotate.angleDeg);
  return { center: applyPlacementToPoint(m, frame.center), normal: applyPlacementToNormal(m, frame.normal) };
}

/** The one assertion the whole module exists to satisfy. */
function expectFlush(moving: PlanarFaceFrame, dest: PlanarFaceFrame, pivot: Vec3): void {
  const solved = alignPlacement(moving, dest, pivot);
  expect(solved).not.toBeNull();
  expect(solved!.translate.every(Number.isFinite)).toBe(true);
  expect(Number.isFinite(solved!.rotate.angleDeg)).toBe(true);
  expect(solved!.rotate.axis.every(Number.isFinite)).toBe(true);
  const landed = applySolution(solved!, moving);
  for (let a = 0; a < 3; a++) {
    expect(landed.center[a]).toBeCloseTo(dest.center[a], 12);
    expect(landed.normal[a]).toBeCloseTo(-dest.normal[a], 12);
  }
  // The pivot is an INPUT, never re-derived — SCHEMA §7.3 freezes it.
  expect(solved!.rotate.center).toEqual(pivot);
}

describe("faceFrame — area-weighted planar frame", () => {
  it("gives each mock-box face its exact centre and outward normal", () => {
    const v = boxView();
    const expected: Record<string, { center: Vec3; normal: Vec3 }> = {
      "f:0": { center: [40, 0, 0], normal: [1, 0, 0] },
      "f:1": { center: [-40, 0, 0], normal: [-1, 0, 0] },
      "f:2": { center: [0, 30, 0], normal: [0, 1, 0] },
      "f:3": { center: [0, -30, 0], normal: [0, -1, 0] },
      "f:4": { center: [0, 0, 15], normal: [0, 0, 1] },
      "f:5": { center: [0, 0, -15], normal: [0, 0, -1] },
    };
    for (const [id, want] of Object.entries(expected)) {
      const frame = faceFrame(v, boxFace(id));
      expect(frame, id).not.toBeNull();
      for (let a = 0; a < 3; a++) {
        expect(frame!.center[a]).toBeCloseTo(want.center[a], 4);
        expect(frame!.normal[a]).toBeCloseTo(want.normal[a], 6);
      }
    }
  });

  it("weights the centroid by AREA, not by triangle count", () => {
    // One 2×2 square split into a big triangle and a small one plus a sliver —
    // an unweighted mean of the three triangle centroids would land elsewhere.
    const positions = [0, 0, 0, 2, 0, 0, 2, 2, 0, 0, 2, 0];
    const blob = encodeMesh1({
      positions,
      faces: [{ triangles: [[0, 1, 2], [0, 2, 3]], id: "f:0" }],
    });
    const frame = faceFrame(view(blob), 0)!;
    expect(frame.center[0]).toBeCloseTo(1, 9);
    expect(frame.center[1]).toBeCloseTo(1, 9);
    expect(frame.center[2]).toBeCloseTo(0, 9);
    expect(frame.normal).toEqual([0, 0, 1]);
  });

  it("REFUSES a curved face — the tessellated cylinder wall", () => {
    const v = view(makeCylinderMesh());
    expect(faceFrame(v, 0)).toBeNull(); // f:0 — the 24-segment side wall
    // …while both PLANAR caps of the same body resolve, so the refusal is about
    // curvature and not about the body.
    const top = faceFrame(v, 1)!;
    expect(top.normal[2]).toBeCloseTo(1, 9);
    expect(top.center[2]).toBeCloseTo(30, 6);
    const bottom = faceFrame(v, 2)!;
    expect(bottom.normal[2]).toBeCloseTo(-1, 9);
  });

  it("refuses a face whose facets tilt past the tolerance, and accepts one inside it", () => {
    // Two triangles meeting at a shallow crease of `tilt` radians about the
    // shared edge; the deviation from the mean is tilt/2 each way.
    const roof = (tilt: number): ArrayBuffer =>
      encodeMesh1({
        positions: [
          -1, -1, 0, -1, 1, 0, 0, -1, 0, 0, 1, 0,
          1, -1, -Math.tan(tilt), 1, 1, -Math.tan(tilt),
        ],
        faces: [{ triangles: [[0, 2, 3], [0, 3, 1], [2, 4, 5], [2, 5, 3]], id: "f:0" }],
      });
    expect(faceFrame(view(roof(PLANARITY_TOL_RAD * 4)), 0)).toBeNull();
    expect(faceFrame(view(roof(PLANARITY_TOL_RAD / 4)), 0)).not.toBeNull();
  });

  it("refuses an out-of-range or empty face rather than reading past the buffer", () => {
    const v = boxView();
    expect(faceFrame(v, -1)).toBeNull();
    expect(faceFrame(v, v.faceCount)).toBeNull();
    expect(faceFrame(v, 1.5)).toBeNull();
  });

  it("refuses a zero-area face (every triangle degenerate)", () => {
    const blob = encodeMesh1({
      positions: [0, 0, 0, 1, 0, 0, 2, 0, 0],
      faces: [{ triangles: [[0, 1, 2]], id: "f:0" }],
    });
    expect(faceFrame(view(blob), 0)).toBeNull();
  });
});

describe("stablePerpendicular / minimalRotation — the NaN-free degenerate band", () => {
  it("is perpendicular, unit, and DETERMINISTIC for every world axis", () => {
    for (const n of [[1, 0, 0], [0, 1, 0], [0, 0, 1], [-1, 0, 0], [0, 0, -1]] as Vec3[]) {
      const p = stablePerpendicular(n);
      expect(Math.hypot(...p)).toBeCloseTo(1, 12);
      expect(dot(p, n)).toBeCloseTo(0, 12);
      expect(stablePerpendicular(n)).toEqual(p); // same input ⇒ same axis, always
    }
  });

  it("picks the basis axis LEAST aligned with the normal (never a near-degenerate cross)", () => {
    const n = unit([0.9, 0.3, 0.31]);
    const p = stablePerpendicular(n);
    expect(dot(p, n)).toBeCloseTo(0, 12);
    expect(p.every(Number.isFinite)).toBe(true);
  });

  it("returns 0° for an already-aligned pair and a REAL axis with it", () => {
    const r = minimalRotation([0, 0, 1], [0, 0, 1]);
    expect(r.angleDeg).toBe(0);
    expect(r.axis.every(Number.isFinite)).toBe(true);
    expect(Math.hypot(...r.axis)).toBeCloseTo(1, 12);
  });

  it("returns exactly 180° about a stable perpendicular for an opposed pair", () => {
    const r = minimalRotation([0, 0, 1], [0, 0, -1]);
    expect(r.angleDeg).toBe(180);
    expect(dot(r.axis, [0, 0, 1])).toBeCloseTo(0, 12);
    // The cross product is the ZERO vector here — the branch that would divide
    // by |cross| is exactly the one that must not run.
    expect(r.axis.every(Number.isFinite)).toBe(true);
  });

  it("agrees with the closed form on a generic pair", () => {
    const r = minimalRotation([1, 0, 0], unit([1, 1, 0]));
    expect(r.angleDeg).toBeCloseTo(45, 12);
    expect(r.axis).toEqual([0, 0, 1]);
  });
});

describe("alignPlacement — flush, verified by re-applying the answer", () => {
  const pivot: Vec3 = [0, 0, 0];

  it("axis-aligned: +X face onto a −X face facing it", () => {
    const moving: PlanarFaceFrame = { center: [40, 0, 0], normal: [1, 0, 0] };
    const dest: PlanarFaceFrame = { center: [90, 0, 0], normal: [-1, 0, 0] };
    const solved = alignPlacement(moving, dest, pivot)!;
    // Already anti-parallel ⇒ a PURE translation, no spurious rotation.
    expect(solved.rotate.angleDeg).toBe(0);
    expect(solved.translate).toEqual([50, 0, 0]);
    expectFlush(moving, dest, pivot);
  });

  it("axis-aligned quarter turn: +X face onto a −Y face (the e2e case, by hand)", () => {
    // body1's +X face (centre (40,0,0)) onto the import box's −Y face at
    // (110,−20,0). R is +90° about Z; R·(40,0,0) = (0,40,0); so the translate is
    // (110,−20,0) − (0,40,0) = (110,−60,0).
    const moving: PlanarFaceFrame = { center: [40, 0, 0], normal: [1, 0, 0] };
    const dest: PlanarFaceFrame = { center: [110, -20, 0], normal: [0, -1, 0] };
    const solved = alignPlacement(moving, dest, pivot)!;
    expect(solved.rotate.axis[2]).toBeCloseTo(1, 12); // +Z; ±0 on the other two
    expect(solved.rotate.axis[0]).toBeCloseTo(0, 12);
    expect(solved.rotate.axis[1]).toBeCloseTo(0, 12);
    expect(solved.rotate.angleDeg).toBeCloseTo(90, 12);
    expect(solved.translate[0]).toBeCloseTo(110, 12);
    expect(solved.translate[1]).toBeCloseTo(-60, 12);
    expect(solved.translate[2]).toBeCloseTo(0, 12);
    expectFlush(moving, dest, pivot);
  });

  it("tilted 30°: an arbitrary destination normal still lands flush", () => {
    const dest: PlanarFaceFrame = {
      center: [12.5, -7, 3.25],
      normal: unit([Math.cos(Math.PI / 6), Math.sin(Math.PI / 6), 0]),
    };
    const moving: PlanarFaceFrame = { center: [40, 0, 15], normal: [0, 0, 1] };
    expectFlush(moving, dest, pivot);
    expectFlush(moving, dest, [3, -4, 5]); // …and about a pivot that is not the origin
  });

  it("ANTI-PARALLEL already: identity rotation, translation only", () => {
    const moving: PlanarFaceFrame = { center: [0, 0, 15], normal: [0, 0, 1] };
    const dest: PlanarFaceFrame = { center: [5, 5, 40], normal: [0, 0, -1] };
    const solved = alignPlacement(moving, dest, [1, 2, 3])!;
    expect(solved.rotate.angleDeg).toBe(0);
    expect(solved.translate).toEqual([5, 5, 25]);
    expectFlush(moving, dest, [1, 2, 3]);
  });

  it("SAME direction (a 180° flip): stable axis, no NaN, still flush", () => {
    const moving: PlanarFaceFrame = { center: [0, 0, 15], normal: [0, 0, 1] };
    const dest: PlanarFaceFrame = { center: [0, 0, 60], normal: [0, 0, 1] };
    const solved = alignPlacement(moving, dest, pivot)!;
    expect(solved.rotate.angleDeg).toBe(180);
    expect(solved.rotate.axis.every(Number.isFinite)).toBe(true);
    expect(dot(solved.rotate.axis, [0, 0, 1])).toBeCloseTo(0, 12);
    expectFlush(moving, dest, pivot);
    // Determinism: the same two faces must place the body the same way twice.
    expect(alignPlacement(moving, dest, pivot)).toEqual(solved);
  });

  it("solves from REAL mock-box faces (registry frames, not hand-written ones)", () => {
    const v = boxView();
    const moving = faceFrame(v, boxFace("f:4"))!; // +Z, centre (0,0,15)
    const dest = faceFrame(v, boxFace("f:3"))!; // −Y, centre (0,−30,0)
    expectFlush(moving, dest, [0, 0, 0]);
  });

  it("refuses non-finite input instead of returning NaN", () => {
    const good: PlanarFaceFrame = { center: [0, 0, 0], normal: [0, 0, 1] };
    expect(alignPlacement({ center: [NaN, 0, 0], normal: [0, 0, 1] }, good, [0, 0, 0])).toBeNull();
    expect(alignPlacement(good, { center: [0, Infinity, 0], normal: [1, 0, 0] }, [0, 0, 0])).toBeNull();
    expect(alignPlacement(good, good, [0, 0, NaN])).toBeNull();
    // A zero-length normal has no direction to rotate onto.
    expect(alignPlacement({ center: [0, 0, 0], normal: [0, 0, 0] }, good, [0, 0, 0])).toBeNull();
    expect(alignPlacement(good, { center: [0, 0, 0], normal: [0, 0, 0] }, [0, 0, 0])).toBeNull();
  });
});

describe("inversePlacement / transformFrame — the record's INPUT frame", () => {
  const cases: { translate: Vec3; center: Vec3; axis: Vec3; angleDeg: number }[] = [
    { translate: [30, 0, 0], center: [0, 0, 0], axis: [0, 0, 1], angleDeg: 0 },
    { translate: [7, -3, 11], center: [2, 5, -1], axis: unit([1, 2, 3]), angleDeg: 37.5 },
    { translate: [0, 0, 0], center: [4, 4, 4], axis: [0, 1, 0], angleDeg: -120 },
  ];

  it("composePlacements applies its RIGHT operand first", () => {
    const a = placementMatrix([0, 0, 0], [0, 0, 0], [0, 0, 1], 90); // rotate
    const b = placementMatrix([10, 0, 0], [0, 0, 0], [0, 0, 1], 0); // translate +X
    // b then a: (1,0,0) → (11,0,0) → (0,11,0).
    const ab = applyPlacementToPoint(composePlacements(a, b), [1, 0, 0]);
    expect(ab[0]).toBeCloseTo(0, 9);
    expect(ab[1]).toBeCloseTo(11, 9);
    // a then b: (1,0,0) → (0,1,0) → (10,1,0).
    const ba = applyPlacementToPoint(composePlacements(b, a), [1, 0, 0]);
    expect(ba[0]).toBeCloseTo(10, 9);
    expect(ba[1]).toBeCloseTo(1, 9);
    // …and a placement composed with its own inverse is the identity.
    const inv = composePlacements(a, inversePlacement([0, 0, 0], [0, 0, 0], [0, 0, 1], 90));
    const p = applyPlacementToPoint(inv, [3, -4, 5]);
    expect(p[0]).toBeCloseTo(3, 9);
    expect(p[1]).toBeCloseTo(-4, 9);
    expect(p[2]).toBeCloseTo(5, 9);
  });

  it("round-trips every point and normal back to where it started", () => {
    for (const c of cases) {
      const fwd = placementMatrix(c.translate, c.center, c.axis, c.angleDeg);
      const inv = inversePlacement(c.translate, c.center, c.axis, c.angleDeg);
      for (const p of [[1, 2, 3], [-40, 0, 15], [0, 0, 0]] as Vec3[]) {
        const back = applyPlacementToPoint(inv, applyPlacementToPoint(fwd, p));
        for (let a = 0; a < 3; a++) expect(back[a]).toBeCloseTo(p[a], 10);
      }
      const n: Vec3 = unit([1, -2, 0.5]);
      const backN = applyPlacementToNormal(inv, applyPlacementToNormal(fwd, n));
      for (let a = 0; a < 3; a++) expect(backN[a]).toBeCloseTo(n[a], 12);
    }
  });

  it("pulls a face frame back through a placement, normal included", () => {
    const fwd = placementMatrix([30, 0, 0], [0, 0, 0], [0, 0, 1], 90);
    const base: PlanarFaceFrame = { center: [40, 0, 0], normal: [1, 0, 0] };
    const moved = transformFrame(fwd, base)!;
    expect(moved.center[0]).toBeCloseTo(30, 9);
    expect(moved.center[1]).toBeCloseTo(40, 9);
    expect(moved.normal[1]).toBeCloseTo(1, 12);
    const back = transformFrame(inversePlacement([30, 0, 0], [0, 0, 0], [0, 0, 1], 90), moved)!;
    for (let a = 0; a < 3; a++) {
      expect(back.center[a]).toBeCloseTo(base.center[a], 9);
      expect(back.normal[a]).toBeCloseTo(base.normal[a], 12);
    }
  });

  it("keeps the normal on the unit sphere and refuses a non-finite frame", () => {
    const m = placementMatrix([1, 2, 3], [0, 0, 0], unit([1, 1, 1]), 200);
    const out = transformFrame(m, { center: [5, 5, 5], normal: [0, 0, 1] })!;
    expect(Math.hypot(...out.normal)).toBeCloseTo(1, 12);
    expect(transformFrame(m, { center: [NaN, 0, 0], normal: [0, 0, 1] })).toBeNull();
    expect(transformFrame(m, { center: [0, 0, 0], normal: [0, 0, 0] })).toBeNull();
  });

  /**
   * The reason this pair exists at all: on a FOLD the visible mesh already
   * carries the record's stored motion, so solving straight off the registry
   * would compose onto a placement the align is supposed to REPLACE.
   */
  it("makes a fold-time solve land flush instead of doubling the offset", () => {
    const stored = { translate: [30, 0, 0] as Vec3, center: [0, 0, 0] as Vec3, axis: [0, 0, 1] as Vec3, angleDeg: 0 };
    const baseFrame: PlanarFaceFrame = { center: [40, 0, 0], normal: [1, 0, 0] };
    const onScreen = transformFrame(
      placementMatrix(stored.translate, stored.center, stored.axis, stored.angleDeg),
      baseFrame,
    )!;
    expect(onScreen.center[0]).toBeCloseTo(70, 9);
    const dest: PlanarFaceFrame = { center: [90, 0, 0], normal: [-1, 0, 0] };

    const inv = inversePlacement(stored.translate, stored.center, stored.axis, stored.angleDeg);
    const corrected = alignPlacement(transformFrame(inv, onScreen)!, dest, stored.center)!;
    expect(corrected.translate[0]).toBeCloseTo(50, 9); // base +X face 40 → 90 ✓

    // What the naive version would have written: 20mm, which lands the face at
    // 40+20 = 60 in the record's own frame — 30mm short of the destination.
    const naive = alignPlacement(onScreen, dest, stored.center)!;
    expect(naive.translate[0]).toBeCloseTo(20, 9);
  });
});
