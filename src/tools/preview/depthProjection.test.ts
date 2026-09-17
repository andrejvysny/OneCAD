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

  it("projects the ray origin onto the axis when the ray is parallel", () => {
    // Ray runs down the axis from z=10 ⇒ closest param = the origin projection.
    const depth = axisDepthFromRay([0, 0, 10], [0, 0, -1], ORIGIN, Z);
    expect(depth).toBeCloseTo(10, 9);
  });

  it("works for a non-origin axis point", () => {
    const depth = axisDepthFromRay([5, 0, 7], [-1, 0, 0], [0, 0, 2], Z);
    expect(depth).toBeCloseTo(5, 9); // 7 measured from z=2
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
