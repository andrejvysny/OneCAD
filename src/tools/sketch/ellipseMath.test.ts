import { describe, it, expect } from "vitest";
import {
  ELLIPSE_SEGMENTS,
  ellipseExtents,
  ellipseParams,
  ellipsePointAt,
  nearestOnEllipse,
  normalizeEllipse,
  sampleEllipse,
  wrapTwoPi,
} from "./ellipseMath";

const TWO_PI = Math.PI * 2;

describe("wrapTwoPi", () => {
  it("folds any angle into [0, 2π)", () => {
    expect(wrapTwoPi(0)).toBe(0);
    expect(wrapTwoPi(Math.PI)).toBeCloseTo(Math.PI, 12);
    expect(wrapTwoPi(-Math.PI / 2)).toBeCloseTo((3 * Math.PI) / 2, 12);
    expect(wrapTwoPi(TWO_PI)).toBeCloseTo(0, 12);
    expect(wrapTwoPi(3 * TWO_PI + 1)).toBeCloseTo(1, 12);
    expect(wrapTwoPi(-3 * TWO_PI - 1)).toBeCloseTo(TWO_PI - 1, 12);
  });
});

describe("normalizeEllipse (legacy major≥minor rule)", () => {
  // Oracle: OneCAD-CPP/src/core/sketch/tools/EllipseTool.cpp:88-99.
  it("leaves an already-normalized triple alone (rotation only folded)", () => {
    expect(normalizeEllipse(10, 4, 0.3)).toEqual({ majorR: 10, minorR: 4, rotation: 0.3 });
  });

  it("swaps the radii and adds π/2 when minor exceeds major", () => {
    const out = normalizeEllipse(4, 10, 0);
    expect(out.majorR).toBe(10);
    expect(out.minorR).toBe(4);
    expect(out.rotation).toBeCloseTo(Math.PI / 2, 12);
  });

  it("folds the swapped rotation back into [0, 2π)", () => {
    // 3π/2 + π/2 = 2π ⇒ 0
    const out = normalizeEllipse(1, 2, (3 * Math.PI) / 2);
    expect(out.rotation).toBeCloseTo(0, 12);
  });

  it("folds a NEGATIVE rotation even without a swap (single canonical domain)", () => {
    const out = normalizeEllipse(10, 4, -Math.PI / 4);
    expect(out.majorR).toBe(10);
    expect(out.rotation).toBeCloseTo(TWO_PI - Math.PI / 4, 12);
  });

  it("is idempotent — re-normalizing a normalized triple changes nothing", () => {
    const once = normalizeEllipse(3, 9, -2.5);
    const twice = normalizeEllipse(once.majorR, once.minorR, once.rotation);
    expect(twice).toEqual(once);
  });

  it("describes the SAME curve after a swap (the point set is unchanged)", () => {
    const raw = { center: [5, -2] as [number, number], majorR: 4, minorR: 10, rotation: 0.7 };
    const n = normalizeEllipse(raw.majorR, raw.minorR, raw.rotation);
    const normalized = { center: raw.center, ...n };
    // The swapped labelling puts the OLD minor direction on the new major axis.
    const rawEnd = ellipsePointAt(raw, Math.PI / 2); // old minor-axis end
    const newEnd = ellipsePointAt(normalized, 0); // new major-axis end
    expect(newEnd[0]).toBeCloseTo(rawEnd[0], 10);
    expect(newEnd[1]).toBeCloseTo(rawEnd[1], 10);
  });
});

describe("ellipseParams", () => {
  it("resolves a complete Ellipse and defaults a missing rotation to 0", () => {
    expect(ellipseParams({ type: "Ellipse", center: [1, 2], majorR: 5, minorR: 3 })).toEqual({
      center: [1, 2],
      majorR: 5,
      minorR: 3,
      rotation: 0,
    });
  });
  it("returns null for a non-Ellipse or an incomplete one", () => {
    expect(ellipseParams({ type: "Circle", center: [0, 0], majorR: 1, minorR: 1 })).toBeNull();
    expect(ellipseParams({ type: "Ellipse", center: [0, 0], majorR: 1 })).toBeNull();
    expect(ellipseParams({ type: "Ellipse", majorR: 1, minorR: 1 })).toBeNull();
  });
});

describe("ellipsePointAt", () => {
  const e = { center: [10, 20] as [number, number], majorR: 6, minorR: 2, rotation: 0 };

  it("puts t=0 at the +major end and t=π/2 at the +minor end (unrotated)", () => {
    expect(ellipsePointAt(e, 0)).toEqual([16, 20]);
    const q = ellipsePointAt(e, Math.PI / 2);
    expect(q[0]).toBeCloseTo(10, 12);
    expect(q[1]).toBeCloseTo(22, 12);
  });

  it("rotates the axes by `rotation`", () => {
    const r = { ...e, rotation: Math.PI / 2 };
    const p = ellipsePointAt(r, 0);
    expect(p[0]).toBeCloseTo(10, 12);
    expect(p[1]).toBeCloseTo(26, 12); // major now points along +Y
  });

  it("satisfies the implicit ellipse equation in local coords", () => {
    const r = { center: [0, 0] as [number, number], majorR: 7, minorR: 3, rotation: 0.9 };
    for (const t of [0, 0.4, 1.7, 3.3, 5.9]) {
      const [x, y] = ellipsePointAt(r, t);
      const lx = x * Math.cos(r.rotation) + y * Math.sin(r.rotation);
      const ly = -x * Math.sin(r.rotation) + y * Math.cos(r.rotation);
      expect((lx / r.majorR) ** 2 + (ly / r.minorR) ** 2).toBeCloseTo(1, 10);
    }
  });
});

describe("sampleEllipse", () => {
  it("returns `segments` points and does NOT repeat the first", () => {
    const pts = sampleEllipse({ center: [0, 0], majorR: 5, minorR: 2, rotation: 0 });
    expect(pts).toHaveLength(ELLIPSE_SEGMENTS);
    expect(pts[0]).toEqual([5, 0]);
    expect(pts[pts.length - 1]).not.toEqual(pts[0]);
  });

  it("encloses (approximately) the analytic area πab", () => {
    const e = { center: [3, -4] as [number, number], majorR: 8, minorR: 3, rotation: 1.1 };
    const pts = sampleEllipse(e, 256);
    let a2 = 0;
    for (let i = 0; i < pts.length; i++) {
      const [x0, y0] = pts[i];
      const [x1, y1] = pts[(i + 1) % pts.length];
      a2 += x0 * y1 - x1 * y0;
    }
    expect(Math.abs(a2 / 2)).toBeCloseTo(Math.PI * 8 * 3, 1);
  });
});

describe("ellipseExtents", () => {
  it("reduces to (a, b) at rotation 0 and swaps at π/2", () => {
    expect(ellipseExtents({ center: [0, 0], majorR: 9, minorR: 4, rotation: 0 })).toEqual({ ex: 9, ey: 4 });
    const q = ellipseExtents({ center: [0, 0], majorR: 9, minorR: 4, rotation: Math.PI / 2 });
    expect(q.ex).toBeCloseTo(4, 12);
    expect(q.ey).toBeCloseTo(9, 12);
  });

  it("bounds every sampled point (tight support extents)", () => {
    const e = { center: [2, 1] as [number, number], majorR: 10, minorR: 3, rotation: 0.63 };
    const { ex, ey } = ellipseExtents(e);
    let maxX = 0;
    let maxY = 0;
    for (const [x, y] of sampleEllipse(e, 512)) {
      maxX = Math.max(maxX, Math.abs(x - 2));
      maxY = Math.max(maxY, Math.abs(y - 1));
    }
    expect(maxX).toBeLessThanOrEqual(ex + 1e-9);
    expect(maxY).toBeLessThanOrEqual(ey + 1e-9);
    // …and is actually attained (not a loose box).
    expect(maxX).toBeCloseTo(ex, 3);
    expect(maxY).toBeCloseTo(ey, 3);
  });

  it("degenerates to a circle's bbox when the radii are equal", () => {
    const { ex, ey } = ellipseExtents({ center: [0, 0], majorR: 5, minorR: 5, rotation: 1.234 });
    expect(ex).toBeCloseTo(5, 12);
    expect(ey).toBeCloseTo(5, 12);
  });
});

describe("nearestOnEllipse", () => {
  it("lands on the major-axis end for a point far out along +X (unrotated)", () => {
    const e = { center: [0, 0] as [number, number], majorR: 6, minorR: 2, rotation: 0 };
    const p = nearestOnEllipse({ x: 100, y: 0 }, e);
    expect(p.x).toBeCloseTo(6, 6);
    expect(p.y).toBeCloseTo(0, 6);
  });

  it("matches a circle's exact nearest point when the radii are equal", () => {
    const e = { center: [1, 2] as [number, number], majorR: 4, minorR: 4, rotation: 0.4 };
    const p = nearestOnEllipse({ x: 11, y: 2 }, e);
    expect(p.x).toBeCloseTo(5, 6);
    expect(p.y).toBeCloseTo(2, 6);
  });

  it("returns a point ON the curve (satisfies the implicit equation)", () => {
    const e = { center: [-3, 7] as [number, number], majorR: 9, minorR: 2.5, rotation: 2.2 };
    for (const probe of [
      { x: 0, y: 0 },
      { x: -3, y: 7 },
      { x: 20, y: -5 },
      { x: -10, y: 9 },
    ]) {
      const p = nearestOnEllipse(probe, e);
      const dx = p.x - e.center[0];
      const dy = p.y - e.center[1];
      const lx = dx * Math.cos(e.rotation) + dy * Math.sin(e.rotation);
      const ly = -dx * Math.sin(e.rotation) + dy * Math.cos(e.rotation);
      expect((lx / e.majorR) ** 2 + (ly / e.minorR) ** 2).toBeCloseTo(1, 8);
    }
  });

  it("beats every coarse sample — the refinement actually refines", () => {
    const e = { center: [0, 0] as [number, number], majorR: 100, minorR: 20, rotation: 0.37 };
    const probe = { x: 41, y: 63 };
    const best = nearestOnEllipse(probe, e);
    const bestD = Math.hypot(best.x - probe.x, best.y - probe.y);
    for (const [x, y] of sampleEllipse(e)) {
      expect(bestD).toBeLessThanOrEqual(Math.hypot(x - probe.x, y - probe.y) + 1e-9);
    }
  });
});
