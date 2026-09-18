import { describe, it, expect } from "vitest";
import {
  axisDepthFromRay,
  extrudeDragBasis,
  extrudeDragFrame,
  flooredDrag,
  resolveDepth,
  snapDepth,
  normalize,
  type Vec3,
} from "./depthProjection";

const Z: Vec3 = [0, 0, 1];
const ORIGIN: Vec3 = [0, 0, 0];

describe("axisDepthFromRay", () => {
  it("returns the height of a horizontal ray crossing the axis", () => {
    // Axis = +Z at origin; ray at z=3 heading -x passes over the axis at z=3.
    const depth = axisDepthFromRay([5, 0, 3], [-1, 0, 0], ORIGIN, Z);
    expect(depth).toBeCloseTo(3, 9);
  });

  it("is signed: a ray below the plane yields a negative depth (flip)", () => {
    const depth = axisDepthFromRay([5, 0, -4], [-1, 0, 0], ORIGIN, Z);
    expect(depth).toBeCloseTo(-4, 9);
  });

  it("works for a non-origin axis point", () => {
    const depth = axisDepthFromRay([5, 0, 7], [-1, 0, 0], [0, 0, 2], Z);
    expect(depth).toBeCloseTo(5, 9); // 7 measured from z=2
  });

  /*
   * H9 — THE PARALLEL BRANCH REFUSES (docs/design/astra/modeling-handle-attachment.md
   * §3, the refusal shape H8 gave `handleProjection`). A ray that cannot be
   * intersected with the axis has no depth to report, and projecting the ray
   * ORIGIN onto the axis instead is a different quantity wearing the same name:
   * it moves with the camera rather than with the pointer, so the caller jumps.
   */
  it("refuses a ray running along the axis instead of projecting its origin", () => {
    expect(axisDepthFromRay([0, 0, 10], [0, 0, -1], ORIGIN, Z)).toBeNull();
  });

  it("refuses a NEAR-parallel ray whose closest approach is not certifiable", () => {
    // sin∠ ≈ 1e-9: the denominator (≈1e-18) is far below its own rounding error.
    expect(axisDepthFromRay([0, 0, 10], [1e-9, 0, -1], ORIGIN, Z)).toBeNull();
    // …and the verdict is a property of the ANGLE, not of the vector's length.
    expect(axisDepthFromRay([0, 0, 10], [1e-14, 0, -1e-5], ORIGIN, Z)).toBeNull();
  });

  /*
   * THE GUARD IS SCALE-RELATIVE. The absolute `denom <= 1e-9` floor it replaces
   * refused this perfectly well-conditioned ray purely because its direction
   * vector is short — and then answered 3 (the ray origin's height) instead of
   * the true closest approach at 8.
   */
  it("answers a well-conditioned ray whatever its direction vector's length", () => {
    expect(axisDepthFromRay([5, 0, 3], [-1, 0, 1], ORIGIN, Z)).toBeCloseTo(8, 9);
    expect(axisDepthFromRay([5, 0, 3], [-1e-5, 0, 1e-5], ORIGIN, Z)).toBeCloseTo(8, 9);
    expect(axisDepthFromRay([5, 0, 3], [-1e-8, 0, 1e-8], ORIGIN, Z)).toBeCloseTo(8, 9);
  });
});

describe("resolveDepth", () => {
  it("negates when flipped", () => {
    expect(resolveDepth(6)).toBe(6);
    expect(resolveDepth(6, { flip: true })).toBe(-6);
  });
});

describe("snapDepth", () => {
  it("snaps to the nearest step", () => {
    expect(snapDepth(23, 5)).toBe(25);
    expect(snapDepth(22, 5)).toBe(20);
    expect(snapDepth(7, 0)).toBe(7); // no snapping when step ≤ 0
  });
});

describe("normalize", () => {
  it("returns a unit vector", () => {
    const n = normalize([0, 3, 4]);
    expect(Math.hypot(...n)).toBeCloseTo(1, 9);
  });
});

/*
 * TODO.md SESSION 37 H3 (D-S2): the grab-relative drag math the extrude, fillet
 * and shell gestures share. A lower bound REBASES instead of clamping, so a
 * pointer that overshoots it moves the value on its first reversing sample.
 */
describe("flooredDrag", () => {
  it("follows the input 1:1 above the floor and keeps its basis", () => {
    const basis = { start: 5, grab: 10 };
    const frame = flooredDrag(basis, 13, 0.1);
    expect(frame.value).toBe(8);
    expect(frame.basis).toBe(basis);
  });

  it("rebases at the floor: the first reversing sample moves the value (no wind-up)", () => {
    const overshoot = flooredDrag({ start: 2, grab: 0 }, -50, 0.1);
    expect(overshoot.value).toBe(0.1);
    const reversed = flooredDrag(overshoot.basis, -49, 0.1);
    expect(reversed.value).toBeCloseTo(1.1, 12);
  });
});

describe("extrude drag frame (D-S2 symmetric)", () => {
  it("one-sided: signed depth tracks the input 1:1 and crosses zero continuously", () => {
    const basis = extrudeDragBasis("oneSided", 3, 7);
    expect(extrudeDragFrame("oneSided", basis, 2, 1).depth).toBe(-2);
  });

  it("symmetricAxis on a NEGATIVE span: +1 of input grows the span to −22 (head moves with the pointer)", () => {
    const basis = extrudeDragBasis("symmetricAxis", -20, 4);
    expect(basis.start).toBe(10); // the physical half-span
    expect(extrudeDragFrame("symmetricAxis", basis, 5, -1).depth).toBe(-22);
  });

  it("symmetricAxis clamps the span at 0, keeps the sign, and reverses immediately", () => {
    // A NEGATIVE hint, so a `signHint * 0` would really produce −0 here.
    const clamped = extrudeDragFrame("symmetricAxis", extrudeDragBasis("symmetricAxis", -20, 0), -15, -1);
    expect(Object.is(clamped.depth, 0)).toBe(true); // never −0: the sign lives in the hint
    const reversed = extrudeDragFrame("symmetricAxis", clamped.basis, -14, -1);
    expect(reversed.depth).toBe(-2);
  });

  it("symmetricTotal (screen proxy) drives the Total at gain 1, clamped at 0 with the sign kept", () => {
    const basis = extrudeDragBasis("symmetricTotal", -20, 0);
    expect(extrudeDragFrame("symmetricTotal", basis, 3, -1).depth).toBe(-23);
    const clamped = extrudeDragFrame("symmetricTotal", basis, -30, -1);
    expect(clamped.depth).toBe(0);
    expect(extrudeDragFrame("symmetricTotal", clamped.basis, -29, -1).depth).toBe(-1);
  });
});
