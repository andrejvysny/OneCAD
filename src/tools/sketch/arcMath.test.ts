/*
 * arcMath — the two arc constructions and the primitives they share.
 *
 * The properties that matter are stated as properties: an arc through three
 * points passes through all three AND contains the through point in its sweep;
 * a tangent arc leaves its anchor along the given tangent and its `exitTangent`
 * is the direction the chain leaves the far end in (so chaining one arc off
 * another is smooth by construction).
 */
import { describe, it, expect } from "vitest";
import {
  arcThroughPoints,
  circumcenter,
  entityEndTangent,
  perpDistanceTo,
  tangentArcTo,
  unitDir,
  type ArcThrough,
  type TangentArc,
} from "./arcMath";
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";

const P = (x: number, y: number): Point2 => ({ x, y });
const dist = (a: Point2, b: Point2): number => Math.hypot(a.x - b.x, a.y - b.y);
const TWO_PI = Math.PI * 2;
const norm2Pi = (a: number): number => ((a % TWO_PI) + TWO_PI) % TWO_PI;
const angleAt = (c: Point2, p: Point2): number => Math.atan2(p.y - c.y, p.x - c.x);

function mustArc(a: ArcThrough | null): ArcThrough {
  if (!a) throw new Error("expected an arc");
  return a;
}

function mustTangentArc(a: TangentArc | null): TangentArc {
  if (!a) throw new Error("expected a tangent arc");
  return a;
}

describe("unitDir", () => {
  it("normalizes", () => {
    expect(unitDir(P(1, 1), P(4, 5))).toEqual({ x: 0.6, y: 0.8 });
  });

  it("is null for coincident points — no direction to invent", () => {
    expect(unitDir(P(3, 3), P(3, 3))).toBeNull();
  });
});

describe("perpDistanceTo", () => {
  it("measures to the INFINITE line, not the segment", () => {
    expect(perpDistanceTo(P(0, 0), P(10, 0), P(50, -7))).toBeCloseTo(7, 12);
    expect(perpDistanceTo(P(0, 0), P(10, 0), P(3, 4))).toBeCloseTo(4, 12);
  });

  it("is 0 on the line and 0 for a degenerate line", () => {
    expect(perpDistanceTo(P(0, 0), P(10, 10), P(5, 5))).toBeCloseTo(0, 12);
    expect(perpDistanceTo(P(2, 2), P(2, 2), P(9, 9))).toBe(0);
  });
});

describe("circumcenter", () => {
  it("finds the centre of the circle through three points", () => {
    // Right triangle on a circle of radius 5 centred at (0, 0).
    expect(circumcenter(P(5, 0), P(0, 5), P(-5, 0))).toEqual({ x: 0, y: 0 });
  });

  it("is independent of the point ORDER", () => {
    const a = P(-3, 1);
    const b = P(4, 7);
    const c = P(9, -2);
    const ref = circumcenter(a, b, c)!;
    for (const [x, y, z] of [
      [b, c, a],
      [c, a, b],
      [c, b, a],
    ] as const) {
      const got = circumcenter(x, y, z)!;
      expect(got.x).toBeCloseTo(ref.x, 9);
      expect(got.y).toBeCloseTo(ref.y, 9);
    }
  });

  it("is equidistant from all three points", () => {
    const a = P(-3, 1);
    const b = P(4, 7);
    const c = P(9, -2);
    const ctr = circumcenter(a, b, c)!;
    expect(dist(ctr, b)).toBeCloseTo(dist(ctr, a), 9);
    expect(dist(ctr, c)).toBeCloseTo(dist(ctr, a), 9);
  });

  it("is null for collinear points (including duplicates)", () => {
    expect(circumcenter(P(0, 0), P(5, 5), P(9, 9))).toBeNull();
    expect(circumcenter(P(0, 0), P(5, 5), P(5, 5))).toBeNull();
    expect(circumcenter(P(0, 0), P(0, 0), P(1, 7))).toBeNull();
  });
});

describe("arcThroughPoints — the CCW swap rule", () => {
  const S = P(0, 0);
  const E = P(10, 0);

  it("keeps the placed order when the through point is ALREADY in the CCW sweep", () => {
    // Below the chord: CCW from S (angle 180°) to E (angle 0°) runs through 270°.
    const arc = mustArc(arcThroughPoints(S, E, P(5, -5)));
    expect(arc.center.x).toBeCloseTo(5, 9);
    expect(arc.center.y).toBeCloseTo(0, 9);
    expect(arc.radius).toBeCloseTo(5, 9);
    expect(arc.start).toEqual(S);
    expect(arc.end).toEqual(E);
    expect(arc.sweep).toBeCloseTo(Math.PI, 9);
  });

  it("SWAPS the endpoints when the through point sits on the other arc", () => {
    const arc = mustArc(arcThroughPoints(S, E, P(5, 5)));
    expect(arc.start).toEqual(E);
    expect(arc.end).toEqual(S);
    expect(arc.sweep).toBeCloseTo(Math.PI, 9);
  });

  it("produces the MAJOR arc when the through point is the long way round", () => {
    // Circle r=10 at the origin: S at 0°, E at 90°, through at 180°.
    const arc = mustArc(arcThroughPoints(P(10, 0), P(0, 10), P(-10, 0)));
    expect(arc.start).toEqual(P(0, 10));
    expect(arc.end).toEqual(P(10, 0));
    expect(arc.sweep).toBeCloseTo((3 * Math.PI) / 2, 9);
  });

  it("PROPERTY: all three points are on the circle and the through point is inside the sweep", () => {
    const cases: Array<[Point2, Point2, Point2]> = [
      [S, E, P(5, 5)],
      [S, E, P(5, -5)],
      [S, E, P(1, 0.2)],
      [S, E, P(9, -0.1)],
      [P(-4, 3), P(12, -7), P(2, 30)],
      [P(-4, 3), P(12, -7), P(2, -30)],
      [P(1e3, 1e3), P(-1e3, 900), P(0, 1500)],
    ];
    for (const [s, e, m] of cases) {
      const arc = mustArc(arcThroughPoints(s, e, m));
      for (const p of [s, e, m]) expect(dist(arc.center, p)).toBeCloseTo(arc.radius, 6);
      const a0 = angleAt(arc.center, arc.start);
      expect(norm2Pi(angleAt(arc.center, m) - a0)).toBeLessThanOrEqual(arc.sweep + 1e-9);
      expect(arc.sweep).toBeGreaterThan(0);
      expect(arc.sweep).toBeLessThan(TWO_PI);
    }
  });

  it("is null when the three points are collinear", () => {
    expect(arcThroughPoints(S, E, P(5, 0))).toBeNull();
    expect(arcThroughPoints(S, E, P(50, 0))).toBeNull();
  });
});

describe("tangentArcTo", () => {
  const A = P(0, 0);
  const T = P(1, 0);

  it("s > 0 is the CCW case: the anchor is the START", () => {
    const arc = mustTangentArc(tangentArcTo(A, T, P(0, 10)));
    expect(arc.center).toEqual({ x: 0, y: 5 });
    expect(arc.radius).toBeCloseTo(5, 9);
    expect(arc.startsAtAnchor).toBe(true);
    expect(arc.start).toEqual(A);
    expect(arc.end).toEqual({ x: 0, y: 10 });
    // Half a turn: out along +U, back to the chord's other end heading −U.
    expect(arc.sweep).toBeCloseTo(Math.PI, 9);
    expect(arc.exitTangent.x).toBeCloseTo(-1, 9);
    expect(arc.exitTangent.y).toBeCloseTo(0, 9);
  });

  it("s < 0 is the CW case: the arc is stated REVERSED so it still runs CCW", () => {
    const arc = mustTangentArc(tangentArcTo(A, T, P(0, -10)));
    expect(arc.center).toEqual({ x: 0, y: -5 });
    expect(arc.startsAtAnchor).toBe(false);
    expect(arc.start).toEqual({ x: 0, y: -10 });
    expect(arc.end).toEqual(A);
    expect(arc.sweep).toBeCloseTo(Math.PI, 9);
    expect(arc.exitTangent.x).toBeCloseTo(-1, 9);
    expect(arc.exitTangent.y).toBeCloseTo(0, 9);
  });

  it("is null when the cursor sits on the tangent LINE (no finite circle)", () => {
    expect(tangentArcTo(A, T, P(10, 0))).toBeNull();
    expect(tangentArcTo(A, T, P(-10, 0))).toBeNull();
    expect(tangentArcTo(A, T, A)).toBeNull();
  });

  it("is null for a zero-length tangent", () => {
    expect(tangentArcTo(A, P(0, 0), P(3, 4))).toBeNull();
  });

  it("normalizes the tangent — magnitude cannot change the geometry", () => {
    const unit = mustTangentArc(tangentArcTo(A, T, P(3, 7)));
    const long = mustTangentArc(tangentArcTo(A, P(250, 0), P(3, 7)));
    expect(long.radius).toBeCloseTo(unit.radius, 9);
    expect(long.center.x).toBeCloseTo(unit.center.x, 9);
    expect(long.center.y).toBeCloseTo(unit.center.y, 9);
  });

  it("PROPERTY: leaves the anchor along the tangent and passes through the cursor", () => {
    const cases: Array<[Point2, Point2, Point2]> = [
      [P(0, 0), P(1, 0), P(3, 7)],
      [P(0, 0), P(1, 0), P(3, -7)],
      [P(-5, 2), P(0.6, 0.8), P(30, -12)],
      [P(-5, 2), P(-0.6, 0.8), P(-30, -12)],
      [P(120, -40), P(0, 1), P(90, -80)],
    ];
    for (const [anchor, t, cursor] of cases) {
      const arc = mustTangentArc(tangentArcTo(anchor, t, cursor));
      expect(dist(arc.center, anchor)).toBeCloseTo(arc.radius, 9);
      expect(dist(arc.center, cursor)).toBeCloseTo(arc.radius, 9);
      // Travel direction at the anchor === the requested tangent. CCW velocity is
      // ẑ×(P−C); when the arc is stated reversed the chain still ENTERS there, so
      // the travel direction is the reverse of the stored CCW one.
      const r = unitDir(arc.center, anchor)!;
      const ccwVel = { x: -r.y, y: r.x };
      const sign = arc.startsAtAnchor ? 1 : -1;
      const tu = unitDir(P(0, 0), t)!;
      expect(sign * ccwVel.x).toBeCloseTo(tu.x, 9);
      expect(sign * ccwVel.y).toBeCloseTo(tu.y, 9);
      expect(Math.hypot(arc.exitTangent.x, arc.exitTangent.y)).toBeCloseTo(1, 9);
      expect(arc.sweep).toBeGreaterThan(0);
      expect(arc.sweep).toBeLessThan(TWO_PI);
    }
  });

  it("PROPERTY: exitTangent chains — the next arc leaves smoothly from the same point", () => {
    const first = mustTangentArc(tangentArcTo(P(0, 0), P(1, 0), P(20, 12)));
    const second = mustTangentArc(tangentArcTo(P(20, 12), first.exitTangent, P(35, -4)));
    // The junction is on both arcs …
    expect(dist(first.center, P(20, 12))).toBeCloseTo(first.radius, 9);
    expect(dist(second.center, P(20, 12))).toBeCloseTo(second.radius, 9);
    // … and both centres lie on the SAME normal there, which is tangency.
    const n1 = unitDir(P(20, 12), first.center)!;
    const n2 = unitDir(P(20, 12), second.center)!;
    expect(Math.abs(n1.x * n2.y - n1.y * n2.x)).toBeCloseTo(0, 9);
  });
});

describe("entityEndTangent", () => {
  const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({
    id,
    type: "Line",
    p0,
    p1,
  });
  /** CCW quarter arc of radius 10 about the origin: start (10,0) → end (0,10). */
  const quarter: SketchEntity = {
    id: "a1",
    type: "Arc",
    center: [0, 0],
    radius: 10,
    start: [10, 0],
    end: [0, 10],
  };

  it("returns null when NOTHING ends there", () => {
    expect(entityEndTangent(P(50, 50), [line("l1", [0, 0], [10, 0])], 0.01)).toBeNull();
    expect(entityEndTangent(P(0, 0), [], 0.01)).toBeNull();
  });

  it("a line END continues FORWARD, a line START continues BACKWARD", () => {
    const l = [line("l1", [0, 0], [10, 0])];
    expect(entityEndTangent(P(10, 0), l, 0.01)).toEqual({ x: 1, y: 0 });
    expect(entityEndTangent(P(0, 0), l, 0.01)).toEqual({ x: -1, y: 0 });
  });

  it("an arc leaves along its CCW velocity at the end, and the reverse at the start", () => {
    const atEnd = entityEndTangent(P(0, 10), [quarter], 0.01)!;
    expect(atEnd.x).toBeCloseTo(-1, 9);
    expect(atEnd.y).toBeCloseTo(0, 9);
    const atStart = entityEndTangent(P(10, 0), [quarter], 0.01)!;
    expect(atStart.x).toBeCloseTo(0, 9);
    expect(atStart.y).toBeCloseTo(-1, 9);
  });

  it("returns null on TWO matches — a junction is ambiguous, never a guess", () => {
    const two = [line("l1", [0, 0], [10, 0]), line("l2", [10, 0], [10, 9])];
    expect(entityEndTangent(P(10, 0), two, 0.01)).toBeNull();
    // Both ends of ONE degenerate entity is equally ambiguous.
    expect(entityEndTangent(P(4, 4), [line("l3", [4, 4], [4, 4])], 0.01)).toBeNull();
  });

  it("honours the tolerance", () => {
    const l = [line("l1", [0, 0], [10, 0])];
    expect(entityEndTangent(P(10.05, 0), l, 0.01)).toBeNull();
    expect(entityEndTangent(P(10.05, 0), l, 0.1)).toEqual({ x: 1, y: 0 });
  });

  it("ignores closed curves and points — they have no end to leave from", () => {
    const closed: SketchEntity[] = [
      { id: "c1", type: "Circle", center: [0, 0], radius: 10 },
      { id: "e1", type: "Ellipse", center: [0, 0], majorR: 10, minorR: 4, rotation: 0 },
      { id: "p1", type: "Point", p0: [10, 0] },
    ];
    expect(entityEndTangent(P(10, 0), closed, 0.01)).toBeNull();
    // …and they do not count toward the ambiguity check either.
    expect(entityEndTangent(P(10, 0), [...closed, quarter], 0.01)).not.toBeNull();
  });
});
