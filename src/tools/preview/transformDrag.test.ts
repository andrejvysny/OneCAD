/*
 * `transformDrag` — the gizmo's pointer projections (WP-B W2).
 *
 * Two things are pinned here, and the second is the reason this module exists at
 * all.
 *
 * EXACTNESS: every case below is set up so the right answer is a round number
 * derivable by hand (a ray fired square-on at a known point), because "the body
 * moved roughly the right way" is indistinguishable on screen from a projection
 * that is subtly wrong in a way that only shows up at an odd camera angle.
 *
 * DEGENERACY IS REFUSED, NOT APPROXIMATED. Each projection divides by a quantity
 * that vanishes as the view lines up with the handle. The failure there is not a
 * small error — it is ±∞ or NaN reaching the placement and teleporting the body
 * on one frame. Every guard is therefore NEGATIVE-CHECKED: the test asserts
 * `null`, and asserts the near-degenerate neighbour still WORKS, so a guard that
 * silently swallowed every drag would fail too.
 */
import { describe, it, expect } from "vitest";
import {
  MIN_VIEW_SIN,
  accumulateAngleDeg,
  angleDeltaDeg,
  axisDragDelta,
  planeDragDelta,
  planeDragPoint,
  ringAngleDeg,
  ringBasis,
  snapRotateDeg,
  snapTranslate,
  snapTranslateVec,
  type PointerRay,
} from "./transformDrag";
import type { Vec3 } from "./depthProjection";

const X: Vec3 = [1, 0, 0];
const Y: Vec3 = [0, 1, 0];
const Z: Vec3 = [0, 0, 1];
const ORIGIN: Vec3 = [0, 0, 0];

const ray = (origin: Vec3, dir: Vec3): PointerRay => ({ origin, dir });

/** Every guard returns exactly `null` — never NaN, never Infinity. */
const isClean = (v: number | Vec3 | null): boolean => {
  if (v === null) return true;
  return typeof v === "number" ? Number.isFinite(v) : v.every((c) => Number.isFinite(c));
};

describe("axisDragDelta", () => {
  it("reports the closest-approach point's signed offset along the axis", () => {
    // Fired straight down −Z at x = 5: the nearest point on the X axis is x = 5.
    expect(axisDragDelta(ray([5, 0, 10], [0, 0, -1]), ORIGIN, X)).toBeCloseTo(5, 12);
    expect(axisDragDelta(ray([-12, 4, 10], [0, 0, -1]), ORIGIN, X)).toBeCloseTo(-12, 12);
  });

  it("is measured FROM the pivot, so a moved gizmo reports a relative offset", () => {
    expect(axisDragDelta(ray([50, 0, 10], [0, 0, -1]), [30, 0, 0], X)).toBeCloseTo(20, 12);
  });

  it("ignores the ray's distance from the axis (only the along-axis position counts)", () => {
    const near = axisDragDelta(ray([7, 0, 3], [0, 0, -1]), ORIGIN, X);
    const far = axisDragDelta(ray([7, 900, 3], [0, 0, -1]), ORIGIN, X);
    expect(near).toBeCloseTo(7, 12);
    expect(far).toBeCloseTo(7, 12);
  });

  it("does not require a unit ray direction", () => {
    expect(axisDragDelta(ray([5, 0, 10], [0, 0, -37]), ORIGIN, X)).toBeCloseTo(5, 12);
  });

  it("REFUSES a ray looking down the axis, and still accepts its neighbour", () => {
    // Exactly parallel: the closest-approach parameter is unbounded.
    expect(axisDragDelta(ray([-20, 0, 0], X), ORIGIN, X)).toBeNull();
    // Just inside the band (sin ≈ 0.02) — also refused…
    expect(axisDragDelta(ray([-20, 0, 0], [1, 0.02, 0]), ORIGIN, X)).toBeNull();
    // …but comfortably outside it (sin ≈ 0.2) the projection is live again, so
    // the guard cannot be passing by simply rejecting everything.
    expect(axisDragDelta(ray([-20, 0, 0], [1, 0.2, 0]), ORIGIN, X)).not.toBeNull();
  });

  it("never returns NaN for a degenerate or non-finite input", () => {
    expect(axisDragDelta(ray([0, 0, 10], [0, 0, 0]), ORIGIN, X)).toBeNull();
    expect(axisDragDelta(ray([0, 0, 10], [0, 0, -1]), ORIGIN, [0, 0, 0])).toBeNull();
    expect(axisDragDelta(ray([NaN, 0, 10], [0, 0, -1]), ORIGIN, X)).toBeNull();
    expect(axisDragDelta(ray([0, 0, 10], [NaN, 0, -1]), ORIGIN, X)).toBeNull();
    expect(axisDragDelta(ray([0, 0, 10], [0, 0, -1]), [NaN, 0, 0], X)).toBeNull();
    expect(isClean(axisDragDelta(ray([0, 0, 1e12], [0, 0, -1]), ORIGIN, X))).toBe(true);
  });
});

describe("planeDragPoint / planeDragDelta", () => {
  it("hits the plane through the pivot at the exact intersection", () => {
    const p = planeDragPoint(ray([7, -3, 10], [0, 0, -1]), ORIGIN, Z);
    expect(p).not.toBeNull();
    expect(p![0]).toBeCloseTo(7, 12);
    expect(p![1]).toBeCloseTo(-3, 12);
    expect(p![2]).toBeCloseTo(0, 12);
  });

  it("reports a delta that lies IN the plane (the normal component is exactly 0)", () => {
    const d = planeDragDelta(ray([9, 9, 40], [0, 0, -1]), [4, 1, 6], Z);
    expect(d).toEqual([5, 8, 0]);
  });

  it("follows the pivot's plane, not the world origin's", () => {
    const d = planeDragDelta(ray([0, 5, 40], [0, 0, -1]), [0, 0, 25], Z);
    // The plane sits at z = 25, so the ray stops short of the world origin's.
    expect(d![1]).toBeCloseTo(5, 12);
    expect(d![2]).toBeCloseTo(0, 12);
  });

  it("REFUSES a grazing view, and still accepts a shallow-but-usable one", () => {
    // Exactly in-plane, then inside the guard band (|cos| = 0.02).
    expect(planeDragPoint(ray([0, 0, 10], X), ORIGIN, Z)).toBeNull();
    expect(planeDragPoint(ray([0, 0, 10], [1, 0, -0.02]), ORIGIN, Z)).toBeNull();
    // Outside it the projection works — 0.2 > MIN_VIEW_SIN.
    expect(MIN_VIEW_SIN).toBeLessThan(0.2);
    expect(planeDragPoint(ray([0, 0, 10], [1, 0, -0.2]), ORIGIN, Z)).not.toBeNull();
  });

  it("REFUSES a hit behind the pointer (past the horizon the plane flips sides)", () => {
    // Pointing AWAY from the plane: the algebraic intersection is at t < 0.
    expect(planeDragPoint(ray([0, 0, 10], [0, 0, 1]), ORIGIN, Z)).toBeNull();
  });

  it("never returns NaN for a degenerate or non-finite input", () => {
    expect(planeDragPoint(ray([0, 0, 10], [0, 0, 0]), ORIGIN, Z)).toBeNull();
    expect(planeDragPoint(ray([0, 0, 10], [0, 0, -1]), ORIGIN, [0, 0, 0])).toBeNull();
    expect(planeDragPoint(ray([0, 0, NaN], [0, 0, -1]), ORIGIN, Z)).toBeNull();
    expect(planeDragDelta(ray([0, 0, 10], [0, NaN, -1]), ORIGIN, Z)).toBeNull();
  });
});

describe("ringBasis", () => {
  it("is RIGHT-HANDED: u × v = axis, so +angle is CCW about +axis", () => {
    for (const axis of [X, Y, Z]) {
      const b = ringBasis(axis)!;
      const cross: Vec3 = [
        b.u[1] * b.v[2] - b.u[2] * b.v[1],
        b.u[2] * b.v[0] - b.u[0] * b.v[2],
        b.u[0] * b.v[1] - b.u[1] * b.v[0],
      ];
      cross.forEach((c, i) => expect(c).toBeCloseTo(axis[i], 12));
    }
  });

  it("stays well-conditioned for an axis parallel to the reference vector", () => {
    // +Z is the fallback reference; the basis must not degenerate there.
    const b = ringBasis(Z)!;
    expect(Math.hypot(...b.u)).toBeCloseTo(1, 12);
    expect(Math.hypot(...b.v)).toBeCloseTo(1, 12);
  });

  it("refuses a zero / non-finite axis", () => {
    expect(ringBasis([0, 0, 0])).toBeNull();
    expect(ringBasis([NaN, 0, 1])).toBeNull();
  });
});

describe("ringAngleDeg", () => {
  it("advances by exactly 90° for a quarter turn about +Z", () => {
    const at = (x: number, y: number): number | null =>
      ringAngleDeg(ray([x, y, 10], [0, 0, -1]), ORIGIN, Z);
    const a0 = at(10, 0)!;
    const a1 = at(0, 10)!;
    expect(angleDeltaDeg(a0, a1)).toBeCloseTo(90, 9);
  });

  it("signs the sweep by the right-hand rule about the axis", () => {
    const at = (p: Vec3): number => ringAngleDeg(ray(p, [0, 0, -1]), ORIGIN, Z)!;
    // u → v is CCW seen from +Z, which is the POSITIVE direction.
    expect(angleDeltaDeg(at([10, 0, 10]), at([0, -10, 10]))).toBeCloseTo(-90, 9);
  });

  it("is measured about the PIVOT, not the origin", () => {
    const pivot: Vec3 = [100, 100, 0];
    const a0 = ringAngleDeg(ray([110, 100, 10], [0, 0, -1]), pivot, Z)!;
    const a1 = ringAngleDeg(ray([100, 110, 10], [0, 0, -1]), pivot, Z)!;
    expect(angleDeltaDeg(a0, a1)).toBeCloseTo(90, 9);
  });

  it("refuses a grazing view, a hit ON the pivot, and non-finite input", () => {
    expect(ringAngleDeg(ray([0, 0, 10], [1, 0, 0]), ORIGIN, Z)).toBeNull();
    expect(ringAngleDeg(ray([0, 0, 10], [0, 0, -1]), ORIGIN, Z)).toBeNull(); // dead centre
    expect(ringAngleDeg(ray([0, 0, NaN], [0, 0, -1]), ORIGIN, Z)).toBeNull();
    expect(ringAngleDeg(ray([1, 0, 10], [0, 0, -1]), ORIGIN, [0, 0, 0])).toBeNull();
  });
});

describe("angleDeltaDeg / accumulateAngleDeg", () => {
  it("takes the SHORTEST way round, so the ±180 seam is not a 360° jump", () => {
    expect(angleDeltaDeg(179, -179)).toBeCloseTo(2, 12);
    expect(angleDeltaDeg(-179, 179)).toBeCloseTo(-2, 12);
    expect(angleDeltaDeg(0, 90)).toBeCloseTo(90, 12);
  });

  it("keeps climbing past 180° when the frames are accumulated", () => {
    // Four 90° steps from 0: the wrapped samples go 90, 180, -90, 0 …
    const samples = [0, 90, 180, -90, 0];
    let total = 0;
    for (let i = 1; i < samples.length; i++) {
      total = accumulateAngleDeg(total, samples[i - 1], samples[i]);
    }
    // …and the accumulated total is a full turn, not zero.
    expect(total).toBeCloseTo(360, 9);
  });

  it("treats a non-finite total or sample as no motion rather than poisoning it", () => {
    expect(accumulateAngleDeg(NaN, 0, 10)).toBe(0);
    expect(accumulateAngleDeg(30, NaN, 10)).toBe(30);
    expect(angleDeltaDeg(0, Infinity)).toBe(0);
  });
});

describe("snapping", () => {
  it("snaps a translation to 1mm, and to 0.1mm on the fine tier", () => {
    expect(snapTranslate(30.4)).toBe(30);
    expect(snapTranslate(-30.6)).toBe(-31);
    expect(snapTranslate(30.44, true)).toBe(30.4);
  });

  it("leaves no binary-fraction dust on a fine snap (0.1 × 3 must be 0.3)", () => {
    expect(snapTranslate(0.31, true)).toBe(0.3);
    expect(snapTranslate(2.71, true)).toBe(2.7);
  });

  it("snaps every component of a plane drag independently", () => {
    expect(snapTranslateVec([12.4, -7.6, 0])).toEqual([12, -8, 0]);
    expect(snapTranslateVec([12.44, -7.66, 0], true)).toEqual([12.4, -7.7, 0]);
  });

  it("snaps an angle to 15°, and to 1° on the fine tier", () => {
    expect(snapRotateDeg(43)).toBe(45);
    expect(snapRotateDeg(-38)).toBe(-45);
    expect(snapRotateDeg(43.4, true)).toBe(43);
  });

  it("maps a non-finite value to 0 rather than passing NaN to the placement", () => {
    expect(snapTranslate(NaN)).toBe(0);
    expect(snapRotateDeg(Infinity)).toBe(0);
    expect(snapTranslateVec([NaN, 1.2, Infinity])).toEqual([0, 1, 0]);
  });
});
