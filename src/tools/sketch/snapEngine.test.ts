import { describe, it, expect } from "vitest";
import {
  computeSnap,
  entitySnapPoints,
  arcContainsAngle,
  circleQuadrantPoints,
  arcQuadrantPoints,
  nearestOnSegment,
  nearestOnCircle,
  nearestOnCurve,
  segSegIntersection,
  segCircleIntersections,
  circleCircleIntersections,
  entityIntersections,
  buildSnapCache,
  SNAP_PX,
  type SnapOptions,
} from "./snapEngine";
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";

const base: SnapOptions = {
  gridStep: 10,
  pixelWorld: 1, // ⇒ 8px snap threshold = 8 world units
  enableGrid: true,
  enableGuideLines: true,
  enableGuidePoints: true,
  suppress: false,
};

const hLine: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] };
const circle: SketchEntity = { id: "e2", type: "Circle", center: [100, 100], radius: 5 };

describe("entitySnapPoints", () => {
  it("yields endpoints + midpoint for a line", () => {
    const pts = entitySnapPoints(hLine);
    expect(pts).toContainEqual({ point: { x: 0, y: 0 }, kind: "endpoint" });
    expect(pts).toContainEqual({ point: { x: 40, y: 0 }, kind: "endpoint" });
    expect(pts).toContainEqual({ point: { x: 20, y: 0 }, kind: "midpoint" });
  });
  it("yields the center for a circle", () => {
    expect(entitySnapPoints(circle)).toContainEqual({ point: { x: 100, y: 100 }, kind: "center" });
  });
});

describe("computeSnap priority", () => {
  it("snaps to a nearby endpoint (beats grid)", () => {
    const r = computeSnap({ x: 41, y: 1 }, [hLine], base);
    expect(r.kind).toBe("endpoint");
    expect(r.point).toEqual({ x: 40, y: 0 });
    expect(r.label).toBe("Endpoint");
  });

  it("snaps to a midpoint", () => {
    const r = computeSnap({ x: 21, y: 1 }, [hLine], base);
    expect(r.kind).toBe("midpoint");
    expect(r.point).toEqual({ x: 20, y: 0 });
  });

  it("prefers endpoint over midpoint on a tie", () => {
    // A degenerate zero-length line: endpoint and midpoint coincide at (5,5).
    const dot: SketchEntity = { id: "d", type: "Line", p0: [5, 5], p1: [5, 5] };
    const r = computeSnap({ x: 5, y: 5 }, [dot], base);
    expect(r.kind).toBe("endpoint");
  });

  it("falls to grid when no geometry is near", () => {
    // 1.41px from the node at (10,10) — inside the cell-relative 3.5px reach.
    const r = computeSnap({ x: 11, y: 9 }, [circle], base);
    expect(r.kind).toBe("grid");
    expect(r.point).toEqual({ x: 10, y: 10 });
  });

  it("grid capture is CELL-RELATIVE: a mid-cell cursor is not captured (SNAP P2)", () => {
    // With gridStep 10 at 1px/unit the cell is 10px across, so grid reaches
    // 0.35 x 10 = 3.5px. (25,25) is the cell centre — 7.07 away — and is left
    // for cursor numeric rounding to answer, which is the whole point of the
    // cell-relative reach replacing `gridRequireProximity`.
    const r = computeSnap({ x: 25, y: 25 }, [circle], base);
    expect(r.kind).toBe("none");
    expect(r.point).toEqual({ x: 25, y: 25 });
  });

  it("grid still captures inside its own cell-relative reach", () => {
    // 2.83px from the node at (30,30) — inside 3.5px.
    const r = computeSnap({ x: 28, y: 28 }, [circle], base);
    expect(r.kind).toBe("grid");
    expect(r.point).toEqual({ x: 30, y: 30 });
  });

  it("emits an H/V alignment guide from a recent point", () => {
    // Deliberately OFF the grid: a reference at a multiple of `gridStep` would
    // put a grid node on the same coordinate, and a grid node is a full-point
    // candidate that legitimately outranks a one-axis guide (SNAP P2).
    const r = computeSnap({ x: 12.2, y: 63 }, [], { ...base, recentPoints: [{ x: 12, y: 0 }] });
    expect(r.kind).toBe("alignV");
    expect(r.point.x).toBe(12);
    expect(r.guides).toEqual([{ orientation: "vertical", value: 12, ref: { x: 12, y: 0 } }]);
  });

  it("snaps both axes but does NOT label 'Aligned' when x and y match two unrelated points", () => {
    // (10,0) supplies the x-match and (0,20) supplies the y-match — two
    // disconnected points, not one coincidental hit. The cursor still snaps
    // to their intersection (10,20) and both guide lines still draw, but the
    // label must not claim a single-point alignment that doesn't exist.
    const r = computeSnap({ x: 10.1, y: 20.1 }, [], {
      ...base,
      enableGrid: false,
      recentPoints: [{ x: 10, y: 0 }, { x: 0, y: 20 }],
    });
    expect(r.kind).not.toBe("alignHV");
    expect(r.label).not.toBe("Aligned");
    expect(r.point).toEqual({ x: 10, y: 20 });
    expect(r.guides).toHaveLength(2);
  });

  it("emits both guides (aligned) when x and y both match the SAME point", () => {
    const r = computeSnap({ x: 10.1, y: 20.1 }, [], {
      ...base,
      enableGrid: false,
      recentPoints: [{ x: 10, y: 20 }],
    });
    expect(r.kind).toBe("alignHV");
    expect(r.label).toBe("Aligned");
    expect(r.point).toEqual({ x: 10, y: 20 });
    expect(r.guides).toHaveLength(2);
  });

  it("Alt suppresses all snapping (raw point)", () => {
    const r = computeSnap({ x: 41, y: 1 }, [hLine], { ...base, suppress: true });
    expect(r.snapped).toBe(false);
    expect(r.kind).toBe("none");
    expect(r.point).toEqual({ x: 41, y: 1 });
  });

  it("returns raw when everything is disabled and nothing is near", () => {
    const r = computeSnap({ x: 3, y: 4 }, [], {
      ...base,
      enableGrid: false,
      enableGuideLines: false,
      enableGuidePoints: false,
      enableQuadrant: false,
      enableIntersection: false,
      enableOnCurve: false,
    });
    expect(r.snapped).toBe(false);
    expect(r.point).toEqual({ x: 3, y: 4 });
  });
});

// ── new-parity geometry (M6c): quadrant / intersection / onCurve ──────────────

describe("arcContainsAngle (CCW sweep start→end)", () => {
  // Quarter arc from +X (0°) CCW to +Y (90°), centered at origin.
  const c: [number, number] = [0, 0];
  const start: [number, number] = [10, 0];
  const end: [number, number] = [0, 10];
  it("contains an angle inside the sweep", () => {
    expect(arcContainsAngle(c, start, end, Math.PI / 4)).toBe(true);
  });
  it("excludes an angle outside the sweep", () => {
    expect(arcContainsAngle(c, start, end, Math.PI)).toBe(false); // 180°
    expect(arcContainsAngle(c, start, end, -Math.PI / 4)).toBe(false); // 315°
  });
  it("includes the endpoints", () => {
    expect(arcContainsAngle(c, start, end, 0)).toBe(true);
    expect(arcContainsAngle(c, start, end, Math.PI / 2)).toBe(true);
  });
});

describe("quadrant points", () => {
  it("circle yields the four cardinal points", () => {
    const q = circleQuadrantPoints([0, 0], 5);
    expect(q).toHaveLength(4);
    expect(q[0].x).toBeCloseTo(5);
    expect(q[0].y).toBeCloseTo(0);
    expect(q[1].x).toBeCloseTo(0);
    expect(q[1].y).toBeCloseTo(5);
    expect(q[2].x).toBeCloseTo(-5);
    expect(q[3].y).toBeCloseTo(-5);
  });
  it("arc yields only the quadrants inside its extent", () => {
    // Quarter arc 0°→90° contains only the 0° and 90° quadrant points.
    const q = arcQuadrantPoints([0, 0], 10, [10, 0], [0, 10]);
    expect(q).toHaveLength(2);
  });
});

describe("nearest-point-on-curve", () => {
  it("segment: clamps the projection to the endpoints", () => {
    expect(nearestOnSegment({ x: 5, y: 3 }, [0, 0], [10, 0])).toEqual({ x: 5, y: 0 });
    expect(nearestOnSegment({ x: -4, y: 2 }, [0, 0], [10, 0])).toEqual({ x: 0, y: 0 }); // clamp low
    expect(nearestOnSegment({ x: 40, y: 2 }, [0, 0], [10, 0])).toEqual({ x: 10, y: 0 }); // clamp high
  });
  it("circle: projects radially", () => {
    const p = nearestOnCircle({ x: 20, y: 0 }, [0, 0], 5);
    expect(p.x).toBeCloseTo(5);
    expect(p.y).toBeCloseTo(0);
  });
  it("circle: degenerate point-at-center returns a point on the circle", () => {
    const p = nearestOnCircle({ x: 0, y: 0 }, [0, 0], 5);
    expect(Math.hypot(p.x, p.y)).toBeCloseTo(5);
  });
  it("arc: outside the extent snaps to the nearest endpoint", () => {
    const arc: SketchEntity = { id: "a", type: "Arc", center: [0, 0], radius: 10, start: [10, 0], end: [0, 10] };
    // Point off toward −X is outside the 0..90° sweep ⇒ nearest endpoint (0,10) or (10,0).
    const p = nearestOnCurve({ x: -5, y: 8 }, arc)!;
    expect([p.x, p.y]).toEqual([0, 10]);
  });
  it("arc: inside the extent lands on the arc", () => {
    const arc: SketchEntity = { id: "a", type: "Arc", center: [0, 0], radius: 10, start: [10, 0], end: [0, 10] };
    const p = nearestOnCurve({ x: 8, y: 8 }, arc)!;
    expect(Math.hypot(p.x, p.y)).toBeCloseTo(10);
  });
});

describe("segSegIntersection", () => {
  it("crossing segments intersect at the crossing point", () => {
    expect(segSegIntersection([0, 0], [10, 0], [5, -5], [5, 5])).toEqual({ x: 5, y: 0 });
  });
  it("parallel lines return null", () => {
    expect(segSegIntersection([0, 0], [10, 0], [0, 5], [10, 5])).toBeNull();
  });
  it("crossing beyond the segment bounds returns null", () => {
    // Infinite lines cross at (5,0) but the second segment stops short at y=1.
    expect(segSegIntersection([0, 0], [10, 0], [5, 2], [5, 1])).toBeNull();
  });
});

describe("segCircleIntersections", () => {
  it("secant segment yields two points", () => {
    const hits = segCircleIntersections([-10, 0], [10, 0], [0, 0], 5);
    expect(hits).toHaveLength(2);
    expect(hits.map((h) => h.x).sort((a, b) => a - b)).toEqual([-5, 5]);
  });
  it("tangent segment yields one point", () => {
    const hits = segCircleIntersections([-10, 5], [10, 5], [0, 0], 5);
    expect(hits).toHaveLength(1);
    expect(hits[0].y).toBeCloseTo(5);
  });
  it("miss yields none", () => {
    expect(segCircleIntersections([-10, 20], [10, 20], [0, 0], 5)).toHaveLength(0);
  });
});

describe("circleCircleIntersections (degenerate cases)", () => {
  it("two intersecting circles yield two points", () => {
    expect(circleCircleIntersections([0, 0], 5, [6, 0], 5)).toHaveLength(2);
  });
  it("externally tangent circles yield one point", () => {
    const hits = circleCircleIntersections([0, 0], 5, [10, 0], 5);
    expect(hits).toHaveLength(1);
    expect(hits[0].x).toBeCloseTo(5);
    expect(hits[0].y).toBeCloseTo(0);
  });
  it("disjoint circles yield none", () => {
    expect(circleCircleIntersections([0, 0], 5, [20, 0], 5)).toHaveLength(0);
  });
  it("one circle contained in another yields none", () => {
    expect(circleCircleIntersections([0, 0], 10, [1, 0], 2)).toHaveLength(0);
  });
  it("concentric circles yield none", () => {
    expect(circleCircleIntersections([0, 0], 5, [0, 0], 3)).toHaveLength(0);
  });
});

describe("entityIntersections", () => {
  const hLine: SketchEntity = { id: "l1", type: "Line", p0: [0, 0], p1: [10, 0] };
  const vLine: SketchEntity = { id: "l2", type: "Line", p0: [5, -5], p1: [5, 5] };
  const circle: SketchEntity = { id: "c1", type: "Circle", center: [0, 0], radius: 5 };
  it("line-line crossing", () => {
    expect(entityIntersections(hLine, vLine)).toEqual([{ x: 5, y: 0 }]);
  });
  it("line-circle secant", () => {
    const hits = entityIntersections({ id: "l", type: "Line", p0: [-10, 0], p1: [10, 0] }, circle);
    expect(hits).toHaveLength(2);
  });
  it("filters crossings outside an arc's extent", () => {
    // A line through the origin along +X hits the full circle at ±5, but the
    // quarter arc 0..90° only contains the +5 crossing.
    const arc: SketchEntity = { id: "a", type: "Arc", center: [0, 0], radius: 5, start: [5, 0], end: [0, 5] };
    const hits = entityIntersections({ id: "l", type: "Line", p0: [-10, 0.0], p1: [10, 0.0] }, arc);
    expect(hits).toHaveLength(1);
    expect(hits[0].x).toBeCloseTo(5);
  });
});

describe("computeSnap — extended priority ladder", () => {
  const circle: SketchEntity = { id: "c", type: "Circle", center: [0, 0], radius: 20 };
  const hLine: SketchEntity = { id: "l1", type: "Line", p0: [-30, 5], p1: [30, 5] };
  const vLine: SketchEntity = { id: "l2", type: "Line", p0: [5, -30], p1: [5, 30] };

  it("snaps to a quadrant point (beats grid)", () => {
    // Right quadrant of the circle is (20,0); cursor just off it.
    const r = computeSnap({ x: 21, y: 1 }, [circle], base);
    expect(r.kind).toBe("quadrant");
    expect(r.point.x).toBeCloseTo(20);
    expect(r.point.y).toBeCloseTo(0);
    expect(r.label).toBe("Quadrant");
  });

  it("center beats a co-near quadrant", () => {
    const small: SketchEntity = { id: "c", type: "Circle", center: [4, 4], radius: 3 };
    // Cursor near the center; the quadrant at (7,4) is also within 8px.
    const r = computeSnap({ x: 5, y: 4 }, [small], { ...base, enableGrid: false });
    expect(r.kind).toBe("center");
  });

  it("the ORIGIN outranks a circle centre that sits on it (SNAP P2)", () => {
    // Same point, two names. Inside the 2px capture core the origin's bias
    // (-2.0) beats the centre's (-0.75), and the origin is the more useful
    // answer: it is what lets the commit author the `Fixed` anchor that removes
    // the sketch's two translation DOF.
    const atOrigin: SketchEntity = { id: "c", type: "Circle", center: [0, 0], radius: 3 };
    const r = computeSnap({ x: 1, y: 0 }, [atOrigin], { ...base, enableGrid: false });
    expect(r.kind).toBe("origin");
    expect(r.point).toEqual({ x: 0, y: 0 });
  });

  it("snaps to a line-line intersection", () => {
    // Long lines that cross at (5,5) but whose endpoints/midpoints are all far
    // from the cursor, so the tier-4 intersection is the only nearby point snap.
    const longH: SketchEntity = { id: "lh", type: "Line", p0: [-100, 5], p1: [50, 5] };
    const longV: SketchEntity = { id: "lv", type: "Line", p0: [5, -100], p1: [5, 50] };
    const r = computeSnap({ x: 6, y: 6 }, [longH, longV], base);
    expect(r.kind).toBe("intersection");
    expect(r.point).toEqual({ x: 5, y: 5 });
  });

  it("onCurve is the lowest point tier (loses to a midpoint)", () => {
    // On the horizontal line near its midpoint: midpoint (0,5) within 8px wins.
    const r = computeSnap({ x: 1, y: 6 }, [hLine], base);
    expect(r.kind).toBe("midpoint");
  });

  it("snaps onto a curve when no better point is near", () => {
    // Far from any endpoint/midpoint/quadrant but on the line body.
    const r = computeSnap({ x: 15, y: 6 }, [hLine], { ...base, enableGrid: false });
    expect(r.kind).toBe("onCurve");
    expect(r.point).toEqual({ x: 15, y: 5 });
  });

  it("respects the quadrant toggle", () => {
    const r = computeSnap({ x: 21, y: 1 }, [circle], { ...base, enableQuadrant: false, enableOnCurve: false });
    expect(r.kind).not.toBe("quadrant");
  });

  it("respects the intersection toggle", () => {
    const r = computeSnap({ x: 6, y: 6 }, [hLine, vLine], {
      ...base,
      enableIntersection: false,
      enableOnCurve: false,
      enableGuidePoints: false,
    });
    expect(r.kind).not.toBe("intersection");
  });

  it("respects the onCurve toggle", () => {
    const r = computeSnap({ x: 15, y: 6 }, [hLine], { ...base, enableOnCurve: false, enableGrid: false });
    expect(r.kind).not.toBe("onCurve");
  });
});

// ── perf: precomputed cache must be byte-identical to the live path ───────────
// (snap-engine perf work — buildSnapCache(entities) precomputes the entity-derived
// candidates once per sketch edit; SketchController passes it through opts.cache.
// The only contract that matters is: with or without a cache, computeSnap returns
// the exact same SnapResult for the exact same inputs.)

describe("computeSnap — cache equivalence (buildSnapCache)", () => {
  const line1: SketchEntity = { id: "l1", type: "Line", p0: [-30, 5], p1: [30, 5] };
  const line2: SketchEntity = { id: "l2", type: "Line", p0: [5, -30], p1: [5, 30] };
  const circ: SketchEntity = { id: "c1", type: "Circle", center: [0, 0], radius: 20 };
  const smallCirc: SketchEntity = { id: "c2", type: "Circle", center: [0, 0], radius: 3 };
  const arc: SketchEntity = { id: "a1", type: "Arc", center: [0, 0], radius: 10, start: [10, 0], end: [0, 10] };
  const dot: SketchEntity = { id: "d1", type: "Line", p0: [5, 5], p1: [5, 5] };

  const entitySets: Record<string, SketchEntity[]> = {
    empty: [],
    singleLine: [line1],
    twoCrossingLines: [line1, line2],
    circleAndLine: [circ, line1],
    circleAndArc: [circ, arc],
    mixedWithDegenerate: [line1, line2, circ, smallCirc, arc, dot],
  };

  const rawPoints: Point2[] = [
    { x: 41, y: 1 },
    { x: 21, y: 1 },
    { x: 5, y: 5 },
    { x: 6, y: 6 },
    { x: 15, y: 6 },
    { x: 1, y: 0 },
    { x: 3, y: 4 },
    { x: 0, y: 0 },
    { x: 10.2, y: 60 },
  ];

  const toggleCombos: Array<Partial<SnapOptions>> = [
    {},
    { enableGuidePoints: false },
    { enableQuadrant: false },
    { enableIntersection: false },
    { enableOnCurve: false },
    { enableGuideLines: false, enableGrid: false },
    { enableGuidePoints: false, enableQuadrant: false, enableIntersection: false, enableOnCurve: false },
  ];

  for (const [name, entities] of Object.entries(entitySets)) {
    it(`matches computeSnap without a cache — entities: ${name}`, () => {
      const cache = buildSnapCache(entities);
      for (const raw of rawPoints) {
        for (const toggles of toggleCombos) {
          const opts: SnapOptions = {
            ...base,
            ...toggles,
            recentPoints: [{ x: 10, y: 0 }, { x: 0, y: 20 }],
          };
          const withoutCache = computeSnap(raw, entities, opts);
          const withCache = computeSnap(raw, entities, { ...opts, cache });
          expect(withCache).toEqual(withoutCache);
        }
      }
    });
  }
});

// ── W3 P3: Ellipse — centre snap + nearest-on-curve (no quadrant/intersection) ─

describe("Ellipse snap surface", () => {
  const ell: SketchEntity = {
    id: "el1",
    type: "Ellipse",
    center: [0, 0],
    majorR: 20,
    minorR: 5,
    rotation: 0,
  };

  it("offers its CENTRE as a `center` candidate, like a circle", () => {
    expect(entitySnapPoints(ell)).toEqual([{ point: { x: 0, y: 0 }, kind: "center" }]);
  });

  it("snaps the cursor to the centre at tier `center`", () => {
    const res = computeSnap({ x: 0.4, y: -0.3 }, [ell], {
      gridStep: 10,
      pixelWorld: 1,
      enableGrid: true,
      enableGuideLines: true,
      enableGuidePoints: true,
      suppress: false,
    });
    // The ellipse is centred ON the sketch origin, and inside the 2px capture
    // core the origin outranks a coincident centre — same point, better name.
    expect(res.kind).toBe("origin");
    expect(res.point).toEqual({ x: 0, y: 0 });
  });

  it("nearestOnCurve lands ON the ellipse (not on its bounding circle)", () => {
    const p = nearestOnCurve({ x: 0, y: 40 }, ell)!;
    expect(p.x).toBeCloseTo(0, 6);
    expect(p.y).toBeCloseTo(5, 6); // the minor-axis end, NOT y = 20
  });

  it("nearestOnCurve returns null for an incomplete ellipse", () => {
    expect(nearestOnCurve({ x: 0, y: 0 }, { id: "x", type: "Ellipse", center: [0, 0] })).toBeNull();
  });

  it("DOES contribute intersection snaps now (SNAP P5 closed the seam)", () => {
    // The line through the centre meets the ellipse at both major-axis ends.
    const l: SketchEntity = { id: "l1", type: "Line", p0: [-50, 0], p1: [50, 0] };
    const hits = entityIntersections(ell, l).map((h) => [round9(h.x), round9(h.y)]);
    expect(hits).toHaveLength(2);
    expect(hits).toContainEqual([-ell.majorR!, 0]);
    expect(hits).toContainEqual([ell.majorR!, 0]);
    // Order-independent: the same two points either way round.
    expect(entityIntersections(l, ell)).toHaveLength(2);
  });

  it("contributes its four AXIS EXTREMA as quadrant candidates (SNAP P5)", () => {
    const cache = buildSnapCache([ell]);
    const kinds = cache.points.map((c) => c.kind).sort();
    expect(kinds).toEqual(["center", "quadrant", "quadrant", "quadrant", "quadrant"]);
    const centre = cache.points.find((c) => c.kind === "center");
    expect(centre?.point).toEqual({ x: 0, y: 0 });
    expect(centre?.ref).toEqual({ entityId: ell.id, position: "Center" });
    // Extrema are DERIVED — no point address, so they can author nothing.
    for (const q of cache.points.filter((c) => c.kind === "quadrant")) {
      expect(q.ref).toBeUndefined();
      expect(q.ownerCurveId).toBe(ell.id);
    }
    expect(cache.intersections).toEqual([]);
  });
});

const round9 = (v: number): number => Math.round(v * 1e9) / 1e9;

// ── SP-5.5: polar tracking ────────────────────────────────────────────────────

describe("computeSnap polar tier", () => {
  const anchor: Point2 = { x: 0, y: 0 };
  // Guides + grid off unless a case is about them: the polar tier is what these
  // assert, and the two tiers below it would otherwise mask an inert polar.
  const polarBase: SnapOptions = {
    ...base,
    enableGrid: false,
    enableGuideLines: false,
    enablePolar: true,
    polarAnchor: anchor,
  };

  it("snaps onto the 45° ray off the anchor", () => {
    // (30, 26) misses y = x by 4/√2 ≈ 2.83 world units — inside the 8px reach.
    const r = computeSnap({ x: 30, y: 26 }, [], polarBase);
    expect(r.kind).toBe("polar");
    expect(r.label).toBe("45°");
    expect(r.point.x).toBeCloseTo(28, 9);
    expect(r.point.y).toBeCloseTo(28, 9);
    expect(r.snapped).toBe(true);
  });

  it("snaps BACKWARDS along a ray — a polar guide is a full line (mod π)", () => {
    const r = computeSnap({ x: -40, y: 2 }, [], polarBase);
    expect(r.kind).toBe("polar");
    expect(r.label).toBe("0°");
    expect(r.point).toEqual({ x: -40, y: 0 });
  });

  it("emits an origin+dir guide (never a constant coordinate)", () => {
    const r = computeSnap({ x: 30, y: 26 }, [], { ...polarBase, polarAnchor: { x: 1, y: 1 } });
    expect(r.guides).toHaveLength(1);
    const g = r.guides[0];
    expect(g.orientation).toBe("polar");
    if (g.orientation !== "polar") throw new Error("expected a polar guide");
    expect(g.origin).toEqual({ x: 1, y: 1 });
    expect(Math.hypot(g.dir.x, g.dir.y)).toBeCloseTo(1, 12);
  });

  it("picks the NEAREST ray when two are in reach", () => {
    // 17° out at radius 10 ⇒ 2.96 off the 0° ray and 4.66 off the 45° one, so
    // BOTH are inside the 8px reach and only ranking decides.
    const p = { x: 10 * Math.cos(0.3), y: 10 * Math.sin(0.3) };
    expect(computeSnap(p, [], polarBase).label).toBe("0°");
  });

  it("is INERT without an anchor — byte-identical to polar off", () => {
    const opts: SnapOptions = { ...base, enablePolar: true, polarAnchor: null };
    for (const raw of [{ x: 30, y: 26 }, { x: 3, y: 4 }, { x: 0.5, y: 41 }]) {
      expect(computeSnap(raw, [hLine], opts)).toEqual(computeSnap(raw, [hLine], base));
    }
  });

  it("is INERT when the pref is off, anchor or not", () => {
    const off: SnapOptions = { ...base, enablePolar: false, polarAnchor: anchor };
    for (const raw of [{ x: 30, y: 26 }, { x: 3, y: 4 }]) {
      expect(computeSnap(raw, [hLine], off)).toEqual(computeSnap(raw, [hLine], base));
    }
  });

  it("loses to the point ladder (an endpoint still wins outright)", () => {
    // (41,1) is 1.4 from the line's (40,0) endpoint AND ~1 off the 0° ray.
    const r = computeSnap({ x: 41, y: 1 }, [hLine], { ...polarBase, enableGuidePoints: true });
    expect(r.kind).toBe("endpoint");
    expect(r.point).toEqual({ x: 40, y: 0 });
  });

  it("beats an H/V guide it strictly out-measures", () => {
    // 0.5 off the 45° ray; 3 off the vertical guide through x = 33.
    const opts: SnapOptions = {
      ...polarBase,
      enableGuideLines: true,
      recentPoints: [{ x: 33, y: 90 }],
    };
    const r = computeSnap({ x: 30, y: 29.29289321881345 }, [], opts);
    expect(r.kind).toBe("polar");
    expect(r.label).toBe("45°");
  });

  it("LOSES a tie to H/V — a 90° ray never relabels an alignment guide", () => {
    // The vertical guide through the anchor and the 90° polar ray are the SAME
    // line, so both miss by 0.2. H/V owns the tie (and the wording).
    const opts: SnapOptions = { ...polarBase, enableGuideLines: true, recentPoints: [anchor] };
    const r = computeSnap({ x: 0.2, y: 60 }, [], opts);
    expect(r.kind).toBe("alignV");
    expect(r.label).toBe("Vertical");
    expect(r.guides).toEqual([{ orientation: "vertical", value: 0, ref: anchor }]);
  });

  it("labels the reference direction Parallel and its normal Perpendicular", () => {
    // A chain running at 20°: neither the ray nor its normal duplicates 0/45/90/135°.
    const refDir = { x: Math.cos(0.349), y: Math.sin(0.349) };
    const along = computeSnap({ x: 30 * refDir.x + 1 * refDir.y, y: 30 * refDir.y - 1 * refDir.x }, [], {
      ...polarBase,
      polarRefDir: refDir,
    });
    expect(along.label).toBe("Parallel");

    const perp = { x: -refDir.y, y: refDir.x };
    const across = computeSnap({ x: 30 * perp.x + 1 * perp.y, y: 30 * perp.y - 1 * perp.x }, [], {
      ...polarBase,
      polarRefDir: refDir,
    });
    expect(across.label).toBe("Perpendicular");
  });

  it("dedupes mod π — a 45° chain reports 45°, not Parallel", () => {
    const r = computeSnap({ x: 30, y: 26 }, [], { ...polarBase, polarRefDir: { x: 1, y: 1 } });
    expect(r.label).toBe("45°");
  });

  it("dedupes an ANTIPARALLEL reference too (a line has no sense)", () => {
    const r = computeSnap({ x: -40, y: 2 }, [], { ...polarBase, polarRefDir: { x: -1, y: 0 } });
    expect(r.label).toBe("0°");
  });

  it("ignores a degenerate (zero-length) reference direction", () => {
    const r = computeSnap({ x: 30, y: 26 }, [], { ...polarBase, polarRefDir: { x: 0, y: 0 } });
    expect(r.label).toBe("45°");
  });

  it("Alt (suppress) beats polar like every other tier", () => {
    const r = computeSnap({ x: 30, y: 26 }, [], { ...polarBase, suppress: true });
    expect(r.kind).toBe("none");
    expect(r.snapped).toBe(false);
  });
});

// ── SP-5.7: the snap radius scales every pixel-gated tier ─────────────────────

describe("computeSnap snapPx", () => {
  it("defaults to SNAP_PX (8) when omitted", () => {
    // 7.5 away: inside 8px, outside 5px.
    // Grid off: at 7.5px the node at (50,0) is 2.5px away and would legitimately
    // win, which says nothing about the point reach this case is measuring.
    const noGrid = { ...base, enableGrid: false };
    expect(computeSnap({ x: 47.5, y: 0 }, [hLine], noGrid).kind).toBe("endpoint");
    expect(computeSnap({ x: 47.5, y: 0 }, [hLine], { ...noGrid, snapPx: SNAP_PX }).kind).toBe("endpoint");
    expect(computeSnap({ x: 47.5, y: 0 }, [hLine], { ...noGrid, snapPx: 5 }).kind).not.toBe("endpoint");
  });

  it("a LARGER radius extends the point ladder's reach", () => {
    // 10 away: outside 8px, inside 12px.
    const far = { x: 50, y: 0 };
    expect(computeSnap(far, [hLine], { ...base, enableGrid: false, enableGuideLines: false }).snapped).toBe(false);
    expect(computeSnap(far, [hLine], { ...base, enableGrid: false, snapPx: 12 }).kind).toBe(
      "endpoint",
    );
  });

  it("scales the guide and polar tiers too (one reach governs all of them)", () => {
    const opts: SnapOptions = {
      ...base,
      enableGrid: false,
      enableGuidePoints: false,
      enableOnCurve: false,
      enablePolar: true,
      polarAnchor: { x: 0, y: 0 },
      recentPoints: [{ x: 100, y: 100 }],
    };
    // 10 off both the 0° ray and every reference point.
    const raw = { x: 60, y: 10 };
    expect(computeSnap(raw, [], { ...opts, snapPx: 8 }).snapped).toBe(false);
    expect(computeSnap(raw, [], { ...opts, snapPx: 12 }).kind).toBe("polar");
  });

  it("is scaled by pixelWorld exactly as the default reach is", () => {
    // snapPx 8 at pixelWorld 0.5 ⇒ a 4-world-unit reach.
    const opts: SnapOptions = { ...base, pixelWorld: 0.5, enableGrid: false, enableGuideLines: false };
    expect(computeSnap({ x: 43, y: 0 }, [hLine], opts).kind).toBe("endpoint");
    expect(computeSnap({ x: 45, y: 0 }, [hLine], opts).snapped).toBe(false);
  });
});
