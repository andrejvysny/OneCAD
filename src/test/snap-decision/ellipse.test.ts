/*
 * Ellipse snap geometry (SNAP P5).
 *
 * The ellipse used to be a documented SEAM: no intersections, no extrema, only
 * its centre and a nearest-point-on-curve. That is defensible for a legacy port
 * and indefensible once an ellipse is a first-class sketch entity — you cannot
 * snap to where a line crosses it, or to the end of its major axis.
 *
 * The hard part is tangency. A tangent contact is a DOUBLE root: `g(t)` touches
 * zero without changing sign, so a sign-change scan misses it entirely, and a
 * naive dedupe reports the two coincident roots as two separate crossings.
 */
import { describe, expect, it } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import {
  buildSnapCache,
  entityAabb,
  entityIntersections,
  entityQuadrantPoints,
  lastBroadPhaseStats,
  segEllipseIntersections,
} from "@/tools/sketch/snapCandidates";
import { ellipseParams, ellipsePointAt } from "@/tools/sketch/ellipseMath";
import { computeSnapDecision, type SnapOptions } from "@/tools/sketch/snapEngine";
import { permutations, SCENARIO_BUILDERS, scenario } from "./scenarios";

const { ellipse, line, circle, arc } = SCENARIO_BUILDERS;

/** On-curve residual: 0 exactly ON the ellipse, signed off it. */
function residual(e: SketchEntity, p: { x: number; y: number }): number {
  const q = ellipseParams(e);
  if (!q) return NaN;
  const cos = Math.cos(q.rotation);
  const sin = Math.sin(q.rotation);
  const dx = p.x - q.center[0];
  const dy = p.y - q.center[1];
  const u = dx * cos + dy * sin;
  const v = -dx * sin + dy * cos;
  return (u * u) / (q.majorR * q.majorR) + (v * v) / (q.minorR * q.minorR) - 1;
}

const sortPts = (ps: { x: number; y: number }[]): { x: number; y: number }[] =>
  [...ps].sort((a, b) => a.x - b.x || a.y - b.y);

describe("ellipse extrema", () => {
  it("yields the four AXIS extrema of an axis-aligned ellipse", () => {
    const e = ellipse("e1", [0, 0], 20, 10, 0);
    const qs = entityQuadrantPoints(e).map((q) => q.point);
    expect(qs).toHaveLength(4);
    expect(sortPts(qs).map((p) => [round(p.x), round(p.y)])).toEqual([
      [-20, 0],
      [0, -10],
      [0, 10],
      [20, 0],
    ]);
  });

  it("rotates with the ellipse", () => {
    const e = ellipse("e1", [3, 4], 20, 10, Math.PI / 6);
    for (const q of entityQuadrantPoints(e)) {
      // Every extremum is ON the curve, whatever the rotation.
      expect(Math.abs(residual(e, q.point))).toBeLessThan(1e-9);
    }
  });

  it("gives each extremum a STABLE index-based id and no point address", () => {
    const e = ellipse("e1", [0, 0], 20, 10, 0);
    const qs = entityQuadrantPoints(e);
    expect(qs.map((q) => q.id)).toEqual([
      "quadrant:e1:0",
      "quadrant:e1:1",
      "quadrant:e1:2",
      "quadrant:e1:3",
    ]);
    // DERIVED: no point entity lives there, so nothing can be constrained to it.
    for (const q of qs) expect(q.ref).toBeUndefined();
  });

  it("carries an OnCurve relation against the ellipse (A7)", () => {
    // Was placement-only (`derived-point-not-persistable`): an extremum has no
    // independent point address, but it IS on the curve, and leaving it
    // unconstrained let it silently float free of a later edit — the same
    // defect the quadrant fix (A7) closes for circle/arc.
    const s = scenario("ellipse-extrema");
    const d = computeSnapDecision(s.raw, s.entities, opts()).decision;
    expect(d.primaryKind).toBe("quadrant");
    expect(d.rejected.some((r) => r.reason === "derived-point-not-persistable")).toBe(false);
    const quadrant = d.accepted.find((c) => c.kind === "quadrant")!;
    expect(quadrant.relationIntents).toEqual([{ kind: "OnCurve", refs: [], curveIds: ["e1"] }]);
  });
});

describe("line ∩ ellipse", () => {
  const e = ellipseParams(ellipse("e1", [0, 0], 20, 10, 0))!;

  it("SECANT: two roots, both on the curve", () => {
    const hits = segEllipseIntersections([-30, 5], [30, 5], e);
    expect(hits).toHaveLength(2);
    for (const p of hits) expect(Math.abs(residual(ellipse("e1", [0, 0], 20, 10, 0), p))).toBeLessThan(1e-9);
  });

  it("TANGENT: ONE root, not a duplicated pair", () => {
    const hits = segEllipseIntersections([-30, 10], [30, 10], e);
    expect(hits).toHaveLength(1);
    expect(hits[0].x).toBeCloseTo(0, 6);
    expect(hits[0].y).toBeCloseTo(10, 9);
  });

  it("NEAR-tangent from outside: no root", () => {
    expect(segEllipseIntersections([-30, 10.001], [30, 10.001], e)).toHaveLength(0);
  });

  it("NEAR-tangent from inside: two very close roots, both real", () => {
    const hits = segEllipseIntersections([-30, 9.999], [30, 9.999], e);
    expect(hits).toHaveLength(2);
    expect(Math.abs(hits[0].x - hits[1].x)).toBeGreaterThan(0);
  });

  it("DISJOINT: no roots", () => {
    expect(segEllipseIntersections([-30, 40], [30, 40], e)).toHaveLength(0);
  });

  it("CONTAINED: a segment entirely inside the ellipse has no roots", () => {
    expect(segEllipseIntersections([-2, 0], [2, 0], e)).toHaveLength(0);
  });

  it("clips to the SEGMENT, not the infinite line", () => {
    // The line y=0 crosses at ±20, but this segment only reaches +5.
    expect(segEllipseIntersections([-5, 0], [5, 0], e)).toHaveLength(0);
    expect(segEllipseIntersections([0, 0], [30, 0], e)).toHaveLength(1);
  });

  it("finds the near-zero root without cancellation", () => {
    // A chord starting almost exactly on the curve: the naive quadratic form
    // loses this root to catastrophic cancellation.
    const hits = segEllipseIntersections([20 - 1e-9, 0], [-30, 0], e);
    expect(hits.length).toBeGreaterThanOrEqual(1);
  });

  it("is order-independent through `entityIntersections`", () => {
    const el = ellipse("e1", [0, 0], 20, 10, 0);
    const l = line("l1", [-30, 5], [30, 5]);
    expect(sortPts(entityIntersections(el, l))).toEqual(sortPts(entityIntersections(l, el)));
  });
});

describe("ellipse ∩ circle / arc / ellipse", () => {
  it("a concentric circle BETWEEN the radii meets the ellipse four times", () => {
    const e = ellipse("e1", [0, 0], 20, 10, 0);
    const c = circle("c1", [0, 0], 14);
    const hits = entityIntersections(e, c);
    expect(hits).toHaveLength(4);
    for (const p of hits) {
      expect(Math.abs(residual(e, p))).toBeLessThan(1e-6);
      expect(Math.hypot(p.x, p.y)).toBeCloseTo(14, 6);
    }
  });

  it("a concentric circle OUTSIDE both radii meets it not at all", () => {
    expect(entityIntersections(ellipse("e1", [0, 0], 20, 10, 0), circle("c1", [0, 0], 30))).toEqual(
      [],
    );
  });

  it("a circle TANGENT at the major-axis end reports one contact, not two", () => {
    // Circle centred on the +x axis, touching the ellipse exactly at (20,0).
    const e = ellipse("e1", [0, 0], 20, 10, 0);
    const c = circle("c1", [25, 0], 5);
    const hits = entityIntersections(e, c);
    expect(hits).toHaveLength(1);
    expect(hits[0].x).toBeCloseTo(20, 4);
    expect(hits[0].y).toBeCloseTo(0, 4);
  });

  it("an ARC only counts inside its own sweep", () => {
    const e = ellipse("e1", [0, 0], 20, 10, 0);
    const full = entityIntersections(e, circle("c1", [0, 0], 14));
    // The same circle as a QUARTER arc keeps only the +x+y contact.
    const quarter = entityIntersections(e, arc("a1", [0, 0], 14, [14, 0], [0, 14]));
    expect(full).toHaveLength(4);
    expect(quarter).toHaveLength(1);
    expect(quarter[0].x).toBeGreaterThan(0);
    expect(quarter[0].y).toBeGreaterThan(0);
  });

  it("two ellipses crossing at four points", () => {
    const a = ellipse("e1", [0, 0], 20, 10, 0);
    const b = ellipse("e2", [0, 0], 10, 20, 0); // the same ellipse rotated 90°
    const hits = entityIntersections(a, b);
    expect(hits).toHaveLength(4);
    for (const p of hits) {
      expect(Math.abs(residual(a, p))).toBeLessThan(1e-6);
      expect(Math.abs(residual(b, p))).toBeLessThan(1e-6);
    }
  });

  it("two disjoint ellipses meet nowhere", () => {
    expect(
      entityIntersections(ellipse("e1", [0, 0], 20, 10, 0), ellipse("e2", [200, 0], 20, 10, 0)),
    ).toEqual([]);
  });

  it("roots are ordered stably, so a candidate id means the same point", () => {
    const a = ellipse("e1", [0, 0], 20, 10, 0);
    const c = circle("c1", [0, 0], 14);
    const first = entityIntersections(a, c);
    const again = entityIntersections(a, c);
    expect(again).toEqual(first);
  });
});

describe("metamorphic invariance", () => {
  const SCALES = [1e-3, 1, 1e3, 1e6];

  it("survives translation, rotation and scale", () => {
    for (const s of SCALES) {
      const e = ellipse("e1", [3 * s, -4 * s], 20 * s, 10 * s, Math.PI / 5);
      // A line through the centre along the MAJOR axis meets it at both ends.
      const dir = { x: Math.cos(Math.PI / 5), y: Math.sin(Math.PI / 5) };
      const l = line(
        "l1",
        [3 * s - 30 * s * dir.x, -4 * s - 30 * s * dir.y],
        [3 * s + 30 * s * dir.x, -4 * s + 30 * s * dir.y],
      );
      const hits = entityIntersections(e, l);
      expect(hits, `scale ${s}`).toHaveLength(2);
      for (const p of hits) {
        // Relative residual — an absolute epsilon is meaningless across 9
        // decades of coordinate magnitude.
        expect(Math.abs(residual(e, p)), `scale ${s}`).toBeLessThan(1e-6);
      }
    }
  });

  it("extrema stay on the curve at every scale and rotation", () => {
    for (const s of SCALES) {
      for (const rot of [0, 0.3, Math.PI / 2, 2.7]) {
        const e = ellipse("e1", [s, -s], 20 * s, 7 * s, rot);
        for (const q of entityQuadrantPoints(e)) {
          expect(Math.abs(residual(e, q.point)), `scale ${s} rot ${rot}`).toBeLessThan(1e-9);
        }
      }
    }
  });
});

describe("broad phase", () => {
  it("SKIPS a pair whose bounding boxes do not overlap", () => {
    const s = scenario("disjoint-pair-broad-phase");
    buildSnapCache(s.entities);
    const stats = lastBroadPhaseStats();
    expect(stats.pairsTotal).toBe(1);
    expect(stats.pairsTested).toBe(0);
  });

  it("TESTS a pair whose boxes do overlap", () => {
    buildSnapCache([line("l1", [-30, 5], [30, 5]), ellipse("e1", [0, 0], 20, 10, 0)]);
    expect(lastBroadPhaseStats().pairsTested).toBe(1);
  });

  it("prunes most pairs in a scattered sketch", () => {
    const entities: SketchEntity[] = [];
    for (let i = 0; i < 20; i++) entities.push(circle(`c${i}`, [i * 100, 0], 5));
    buildSnapCache(entities);
    const stats = lastBroadPhaseStats();
    expect(stats.pairsTotal).toBe(190);
    expect(stats.pairsTested).toBe(0);
  });

  it("computes EXACT bounds for a rotated ellipse", () => {
    const e = ellipse("e1", [0, 0], 20, 10, Math.PI / 4);
    const box = entityAabb(e)!;
    // Support function along ±x/±y: √((a·cosθ)² + (b·sinθ)²).
    const ex = Math.hypot(20 * Math.cos(Math.PI / 4), 10 * Math.sin(Math.PI / 4));
    expect(box.maxX).toBeCloseTo(ex, 9);
    // …and it really bounds the curve.
    const p = ellipseParams(e)!;
    for (let i = 0; i < 64; i++) {
      const [x, y] = ellipsePointAt(p, (i / 64) * Math.PI * 2);
      expect(x).toBeLessThanOrEqual(box.maxX + 1e-9);
      expect(x).toBeGreaterThanOrEqual(box.minX - 1e-9);
      expect(y).toBeLessThanOrEqual(box.maxY + 1e-9);
      expect(y).toBeGreaterThanOrEqual(box.minY - 1e-9);
    }
  });
});

describe("ellipse determinism", () => {
  const cases = ["ellipse-extrema", "ellipse-line-secant", "ellipse-line-tangent", "ellipse-circle-four-roots"];

  it("entity order cannot change the decision", () => {
    for (const name of cases) {
      const s = scenario(name);
      const base = computeSnapDecision(s.raw, s.entities, opts()).decision;
      for (const perm of permutations(s.entities, 8)) {
        const d = computeSnapDecision(s.raw, perm as SketchEntity[], opts()).decision;
        expect(d.point, name).toEqual(base.point);
        expect(d.primaryId, name).toEqual(base.primaryId);
      }
    }
  });

  it("cache and non-cache agree", () => {
    for (const name of cases) {
      const s = scenario(name);
      const cache = buildSnapCache(s.entities);
      const plain = computeSnapDecision(s.raw, s.entities, opts()).decision;
      const cached = computeSnapDecision(s.raw, s.entities, opts({ cache })).decision;
      expect(cached.point, name).toEqual(plain.point);
      expect(cached.accepted.map((c) => c.id), name).toEqual(plain.accepted.map((c) => c.id));
    }
  });
});

function opts(over: Partial<SnapOptions> = {}): SnapOptions {
  return {
    gridStep: 10,
    pixelWorld: 1,
    snapPx: 8,
    enableGrid: false,
    enableGuideLines: false,
    enableGuidePoints: true,
    enableQuadrant: true,
    enableIntersection: true,
    enableOnCurve: true,
    suppress: false,
    ...over,
  };
}

/** Rounds AND normalizes -0 to 0 — `cos(π/2)` is -0 and `toEqual` distinguishes them. */
const round = (v: number): number => (Math.round(v * 1e9) / 1e9) + 0;
