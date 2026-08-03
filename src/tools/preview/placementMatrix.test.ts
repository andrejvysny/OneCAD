/*
 * `placementMatrix` — the SCHEMA §7.3 `TransformBody` evaluation order.
 *
 * The composition order is NORMATIVE (`X' = T ∘ R(center, axis, angleDeg) · X`)
 * and it is the one thing about a placement that is easy to get backwards and
 * hard to see: a swapped order still produces a rigid motion, just the wrong one,
 * and only a non-origin pivot with BOTH halves nonzero separates the two. These
 * cases pin that, plus the exactness the mock lane and the ghost both depend on.
 */
import { describe, it, expect } from "vitest";
import {
  placementMatrix,
  applyPlacementToPoint,
  applyPlacementToNormal,
  WORLD_AXIS,
} from "./patternPreview";
import type { Vec3 } from "./depthProjection";

const near = (got: Vec3, want: Vec3, digits = 9): void => {
  got.forEach((v, i) => expect(v).toBeCloseTo(want[i], digits));
};

describe("placementMatrix", () => {
  it("is the identity for a zero placement (a legal no-op)", () => {
    const m = placementMatrix([0, 0, 0], [0, 0, 0], WORLD_AXIS.Z, 0);
    expect([...m]).toEqual([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
  });

  it("a pure translation ignores the pivot entirely", () => {
    const m = placementMatrix([30, 0, 0], [17, -4, 9], WORLD_AXIS.Z, 0);
    near(applyPlacementToPoint(m, [0, 0, 0]), [30, 0, 0]);
    near(applyPlacementToPoint(m, [-40, -30, -15]), [-10, -30, -15]);
  });

  it("a pure rotation about Z through the origin maps +X → +Y at 90°", () => {
    const m = placementMatrix([0, 0, 0], [0, 0, 0], WORLD_AXIS.Z, 90);
    near(applyPlacementToPoint(m, [1, 0, 0]), [0, 1, 0]);
    near(applyPlacementToPoint(m, [0, 1, 0]), [-1, 0, 0]);
    near(applyPlacementToPoint(m, [0, 0, 5]), [0, 0, 5]); // on-axis point is fixed
  });

  it("rotates about the FROZEN pivot, not the world origin", () => {
    // 180° about the vertical line through (1,1,0) sends (2,1,0) → (0,1,0).
    const m = placementMatrix([0, 0, 0], [1, 1, 0], WORLD_AXIS.Z, 180);
    near(applyPlacementToPoint(m, [2, 1, 0]), [0, 1, 0]);
    near(applyPlacementToPoint(m, [1, 1, 0]), [1, 1, 0]); // the pivot is fixed
  });

  it("evaluates T ∘ R — rotate about the pivot FIRST, then translate", () => {
    // R alone sends (2,1,0) → (0,1,0); the +10X translate then lands it at (10,1,0).
    // The reversed order (translate then rotate about the pivot) would give
    // (1,-10,0)+… — a different point, which is what makes this case decisive.
    const m = placementMatrix([10, 0, 0], [1, 1, 0], WORLD_AXIS.Z, 180);
    near(applyPlacementToPoint(m, [2, 1, 0]), [10, 1, 0]);
  });

  it("moves normals by the ROTATION only — a translation leaves them alone", () => {
    const moved = placementMatrix([30, -12, 4], [5, 5, 5], WORLD_AXIS.Z, 0);
    near(applyPlacementToNormal(moved, [0, 0, 1]), [0, 0, 1]);
    const spun = placementMatrix([30, -12, 4], [5, 5, 5], WORLD_AXIS.Z, 90);
    near(applyPlacementToNormal(spun, [1, 0, 0]), [0, 1, 0]);
  });

  it("preserves unit length under rotation about a non-world axis", () => {
    const m = placementMatrix([0, 0, 0], [0, 0, 0], [1, 1, 1], 37);
    const n = applyPlacementToNormal(m, [0, 0, 1]);
    expect(Math.hypot(n[0], n[1], n[2])).toBeCloseTo(1, 12);
  });

  it("normalizes a non-unit axis (the Rust `axis` contract)", () => {
    const unit = placementMatrix([0, 0, 0], [0, 0, 0], [0, 0, 1], 90);
    const long = placementMatrix([0, 0, 0], [0, 0, 0], [0, 0, 7], 90);
    [...long].forEach((v, i) => expect(v).toBeCloseTo(unit[i], 12));
  });
});
