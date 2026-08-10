import { describe, it, expect } from "vitest";
import { chainRefAngleDeg, dimFrame, type DimFrame } from "./liveDimFrames";
import type { Point2 } from "@/viewport/engine/sketchBasis";

const P = (x: number, y: number): Point2 => ({ x, y });
const dist = (a: Point2, b: Point2): number => Math.hypot(a.x - b.x, a.y - b.y);

function must(f: DimFrame | null): DimFrame {
  if (!f) throw new Error("expected a frame");
  return f;
}

/** Every frame in the table, keyed by a readable phase name. */
const FRAMES: Array<[string, DimFrame]> = [
  ["line", must(dimFrame("line", [P(10, 5)]))],
  ["line (chained — active vertex is the LAST anchor)", must(dimFrame("line", [P(0, 0), P(10, 5)]))],
  ["rect", must(dimFrame("rect", [P(-2, 3)]))],
  ["centerRect", must(dimFrame("centerRect", [P(-2, 3)]))],
  ["circle", must(dimFrame("circle", [P(4, 4)]))],
  ["arc phase 1 (radius)", must(dimFrame("arc", [P(1, 1)]))],
  ["arc phase 2 (sweep)", must(dimFrame("arc", [P(1, 1), P(21, 1)]))],
  ["arc3p phase 1 (chord)", must(dimFrame("arc3p", [P(3, -4)]))],
  ["arc3p phase 2 (radius)", must(dimFrame("arc3p", [P(-20, -10), P(20, 30)]))],
  ["ellipse phase 1 (major)", must(dimFrame("ellipse", [P(0, 0)]))],
  ["ellipse phase 2 (minor)", must(dimFrame("ellipse", [P(0, 0), P(30, 10)]))],
  ["polygon", must(dimFrame("polygon", [P(2, -2)], 5))],
  ["slot phase 1 (centreline)", must(dimFrame("slot", [P(-5, -5)]))],
  ["slot phase 2 (width)", must(dimFrame("slot", [P(-5, -5), P(25, 5)]))],
  [
    "line arc mode (tangent-arc radius)",
    must(dimFrame("line", [P(0, 0), P(10, 5)], undefined, { arcMode: true, tangent: P(1, 0) })),
  ],
  [
    "line arc mode (oblique tangent)",
    must(dimFrame("line", [P(-4, 7)], undefined, { arcMode: true, tangent: P(0.6, 0.8) })),
  ],
];

const PROBES: Point2[] = [
  P(37.5, 12.25),
  P(-18, 44),
  P(-9.5, -31),
  P(60, -7),
  P(0.001, 0.001),
  P(1e4, -1e4),
];

describe("dimFrame — rebuild(measure(p), p) ≈ p for EVERY frame", () => {
  for (const [name, frame] of FRAMES) {
    it(name, () => {
      for (const p of PROBES) {
        const back = frame.rebuild(frame.measure(p), p);
        expect(back.x).toBeCloseTo(p.x, 9);
        expect(back.y).toBeCloseTo(p.y, 9);
      }
    });
  }
});

describe("dimFrame — phase table", () => {
  it("has no frame before the first anchor, or for the point tool", () => {
    expect(dimFrame("line", [])).toBeNull();
    expect(dimFrame("point", [P(0, 0)])).toBeNull();
    expect(dimFrame("nope", [P(0, 0)])).toBeNull();
  });

  it("has no frame past a tool's last phase", () => {
    expect(dimFrame("rect", [P(0, 0), P(1, 1)])).toBeNull();
    expect(dimFrame("circle", [P(0, 0), P(1, 1)])).toBeNull();
    expect(dimFrame("arc", [P(0, 0), P(1, 1), P(2, 2)])).toBeNull();
    expect(dimFrame("arc3p", [P(0, 0), P(1, 1), P(2, 2)])).toBeNull();
    expect(dimFrame("slot", [P(0, 0), P(1, 1), P(2, 2)])).toBeNull();
  });

  it("exposes the documented Tab order per phase", () => {
    const order = (f: DimFrame | null): string[] => must(f).fields.map((s) => s.field);
    // A FIRST leg has no previous segment to angle against — length only.
    expect(order(dimFrame("line", [P(0, 0)]))).toEqual(["length"]);
    expect(order(dimFrame("line", [P(0, 0), P(5, 5)]))).toEqual(["length", "angle"]);
    expect(order(dimFrame("rect", [P(0, 0)]))).toEqual(["width", "height"]);
    expect(order(dimFrame("circle", [P(0, 0)]))).toEqual(["diameter", "radius"]);
    // Radius first: an ARMED digit types the radius, Tab reaches the side count.
    expect(order(dimFrame("polygon", [P(0, 0)]))).toEqual(["radius", "sides"]);
    expect(order(dimFrame("slot", [P(0, 0)]))).toEqual(["length"]);
    expect(order(dimFrame("slot", [P(0, 0), P(10, 0)]))).toEqual(["width"]);
  });

  it("marks only the fields with a wire kind as driving", () => {
    const drives = (f: DimFrame | null): boolean[] => must(f).fields.map((s) => s.drives);
    expect(drives(dimFrame("ellipse", [P(0, 0)]))).toEqual([false]);
    expect(drives(dimFrame("ellipse", [P(0, 0), P(10, 0)]))).toEqual([false]);
    expect(drives(dimFrame("arc", [P(0, 0), P(10, 0)]))).toEqual([false]); // sweep
    expect(drives(dimFrame("arc", [P(0, 0)]))).toEqual([true]); // radius
    expect(drives(dimFrame("polygon", [P(0, 0)]))).toEqual([true, false]); // R, n
    // The 3-point arc's CHORD is a leg with no previous segment — length only,
    // and not an entity, so it cannot author anything either.
    expect(drives(dimFrame("arc3p", [P(0, 0)]))).toEqual([false]);
    expect(drives(dimFrame("arc3p", [P(0, 0), P(10, 0)]))).toEqual([true]); // radius
  });

  it("segmentFrame's new `drives` param DEFAULTS to true — line/slot unchanged", () => {
    const drives = (f: DimFrame | null): boolean[] => must(f).fields.map((s) => s.drives);
    expect(drives(dimFrame("line", [P(0, 0)]))).toEqual([true]);
    expect(drives(dimFrame("line", [P(0, 0), P(5, 5)]))).toEqual([true, true]);
    expect(drives(dimFrame("slot", [P(0, 0)]))).toEqual([true]);
  });
});

describe("line / slot centreline frame", () => {
  const a = P(10, 20);
  const frame = must(dimFrame("line", [a]));

  it("measures a length and an ABSOLUTE angle to plane +U", () => {
    expect(frame.measure(P(40, 20))).toEqual({ length: 30, angle: 0 });
    const up = frame.measure(P(10, 50));
    expect(up.length).toBeCloseTo(30, 9);
    expect(up.angle).toBeCloseTo(90, 9);
    const back = frame.measure(P(-20, 20));
    expect(back.angle).toBeCloseTo(180, 9);
    const down = frame.measure(P(10, -10));
    expect(down.angle).toBeCloseTo(270, 9); // folded into [0, 360)
  });

  it("a locked length keeps the cursor's direction", () => {
    const p = frame.rebuild({ length: 50 }, P(10 + 3, 20 + 4));
    expect(dist(a, p)).toBeCloseTo(50, 9);
    expect(p).toEqual({ x: 10 + 50 * 0.6, y: 20 + 50 * 0.8 });
  });

  it("a locked angle keeps the cursor's length", () => {
    const p = frame.rebuild({ angle: 90 }, P(40, 20));
    expect(dist(a, p)).toBeCloseTo(30, 9);
    expect(p.x).toBeCloseTo(10, 9);
    expect(p.y).toBeCloseTo(50, 9);
  });

  it("a zero-extent gesture is finite (angle 0, length 0)", () => {
    expect(frame.measure(a)).toEqual({ length: 0, angle: 0 });
    expect(frame.rebuild(frame.measure(a), a)).toEqual(a);
  });

  it("the slot's centreline phase is the same frame", () => {
    const slot = must(dimFrame("slot", [a]));
    expect(slot.measure(P(40, 20))).toEqual({ length: 30, angle: 0 });
  });
});

describe("chainRefAngleDeg", () => {
  it("is undefined with fewer than two anchors — no reference exists yet", () => {
    expect(chainRefAngleDeg([])).toBeUndefined();
    expect(chainRefAngleDeg([P(0, 0)])).toBeUndefined();
  });

  it("is the ABSOLUTE heading of the LAST committed leg (its final two anchors)", () => {
    expect(chainRefAngleDeg([P(0, 0), P(10, 0)])).toBeCloseTo(0, 9);
    expect(chainRefAngleDeg([P(0, 0), P(0, 10)])).toBeCloseTo(90, 9);
    // A third anchor moves the reference to the LATEST leg, not the first one.
    expect(chainRefAngleDeg([P(0, 0), P(0, 10), P(20, 10)])).toBeCloseTo(0, 9);
  });
});

describe("line frame, CHAINED (a reference segment exists) — signed shortest-arc angle", () => {
  // Previous leg ran P(0,0) → P(0,10): an absolute heading of 90°. The ACTIVE
  // leg leaves the last anchor, P(0,10).
  const anchors = [P(0, 0), P(0, 10)];
  const a = anchors[1];
  const frame = must(dimFrame("line", anchors));

  it("measures the SIGNED turn away from the reference, not the absolute heading", () => {
    // Cursor straight up from `a`: absolute heading 90° = the SAME direction
    // as the reference itself ⇒ zero turn.
    expect(frame.measure(P(a.x, a.y + 30)).angle).toBeCloseTo(0, 9);
    // Cursor to the right: absolute heading 0°, a −90° turn from the 90° ref.
    expect(frame.measure(P(a.x + 30, a.y)).angle).toBeCloseTo(-90, 9);
    // Cursor to the left: absolute heading 180°, a +90° turn from the 90° ref.
    expect(frame.measure(P(a.x - 30, a.y)).angle).toBeCloseTo(90, 9);
  });

  it("never reports a reflex angle — always the shorter arc, signed", () => {
    // Cursor almost back the way the reference came from: absolute heading
    // ~270°+ε, i.e. a huge "the long way round" delta from 90° (≈180°+ε) —
    // must fold to the SHORT way instead of the reflex ~180°+ε value.
    const cursor = P(a.x + 30 * Math.cos((269 * Math.PI) / 180), a.y + 30 * Math.sin((269 * Math.PI) / 180));
    const angle = frame.measure(cursor).angle!;
    expect(angle).toBeDefined();
    expect(Math.abs(angle)).toBeLessThanOrEqual(180);
  });

  it("rebuild(measure(p), p) round-trips exactly across the ±180° seam", () => {
    for (const deg of [0, 1, 89, 90, 91, 179, 180, -1, -90, -179, -180]) {
      const th = (deg * Math.PI) / 180;
      const p = P(a.x + 30 * Math.cos(th), a.y + 30 * Math.sin(th));
      const rebuilt = frame.rebuild(frame.measure(p), p);
      expect(rebuilt.x).toBeCloseTo(p.x, 6);
      expect(rebuilt.y).toBeCloseTo(p.y, 6);
    }
  });

  it("a locked signed angle turns the SAME direction the number reads", () => {
    // −90° from a 90°-absolute reference points straight along world +X.
    const p = frame.rebuild({ angle: -90 }, P(a.x + 5, a.y + 5));
    expect(p.x).toBeGreaterThan(a.x);
    expect(p.y).toBeCloseTo(a.y, 6);
  });
});

describe("rect / centerRect frames", () => {
  const a = P(10, 10);
  const rect = must(dimFrame("rect", [a]));
  const centered = must(dimFrame("centerRect", [a]));

  it("rect measures corner-to-corner extents", () => {
    expect(rect.measure(P(90, 50))).toEqual({ width: 80, height: 40 });
  });

  it("centerRect measures FULL extents (twice the half-extent)", () => {
    expect(centered.measure(P(50, 30))).toEqual({ width: 80, height: 40 });
  });

  it("preserves the quadrant in all four directions", () => {
    for (const [dx, dy] of [[1, 1], [-1, 1], [-1, -1], [1, -1]]) {
      const cursor = P(a.x + dx * 12, a.y + dy * 7);
      const p = rect.rebuild({ width: 80, height: 40 }, cursor);
      expect(p).toEqual({ x: a.x + dx * 80, y: a.y + dy * 40 });
      const q = centered.rebuild({ width: 80, height: 40 }, cursor);
      expect(q).toEqual({ x: a.x + dx * 40, y: a.y + dy * 20 });
    }
  });

  it("locking width leaves height on the cursor", () => {
    const p = rect.rebuild({ width: 80, height: 7 }, P(a.x - 3, a.y - 7));
    expect(p).toEqual({ x: a.x - 80, y: a.y - 7 });
  });

  it("a zero-extent cursor picks the POSITIVE side rather than collapsing", () => {
    expect(rect.rebuild({ width: 80, height: 40 }, a)).toEqual({ x: 90, y: 50 });
  });

  it("width and height are NOT forced into a shared overlay cluster", () => {
    // They sit at genuinely different screen anchors (top edge vs side edge) —
    // collapsing them into one cluster would throw that away and stack them
    // near the box centre instead (the reported "chips randomly placed" bug).
    const width = rect.fields.find((f) => f.field === "width")!;
    const height = rect.fields.find((f) => f.field === "height")!;
    expect(width.clusterId).toBeUndefined();
    expect(height.clusterId).toBeUndefined();
  });
});

describe("circle frame — Ø and R are two views of one number", () => {
  const ctr = P(5, 5);
  const frame = must(dimFrame("circle", [ctr]));

  it("measures both", () => {
    expect(frame.measure(P(5, 15))).toEqual({ diameter: 20, radius: 10 });
  });

  it("both rebuilds land on the same circle", () => {
    const cursor = P(5 + 3, 5 + 4);
    expect(dist(ctr, frame.rebuild({ radius: 10 }, cursor))).toBeCloseTo(10, 9);
    expect(dist(ctr, frame.rebuild({ diameter: 20 }, cursor))).toBeCloseTo(10, 9);
  });

  it("radius wins when both survive (rounding keeps BOTH chips whole)", () => {
    expect(dist(ctr, frame.rebuild({ radius: 10, diameter: 999 }, P(6, 5)))).toBeCloseTo(10, 9);
  });

  it("falls back to plane +U when the cursor sits on the centre", () => {
    expect(frame.rebuild({ radius: 10 }, ctr)).toEqual({ x: 15, y: 5 });
  });

  it("diameter and radius share one overlay cluster (their anchors coincide)", () => {
    const diameter = frame.fields.find((f) => f.field === "diameter")!;
    const radius = frame.fields.find((f) => f.field === "radius")!;
    expect(diameter.clusterId).toBeDefined();
    expect(diameter.clusterId).toBe(radius.clusterId);
  });
});

describe("arc frames", () => {
  const ctr = P(0, 0);

  it("phase 1 pins a radius along the cursor ray", () => {
    const frame = must(dimFrame("arc", [ctr]));
    expect(frame.measure(P(0, 25))).toEqual({ radius: 25 });
    expect(dist(ctr, frame.rebuild({ radius: 40 }, P(0, 25)))).toBeCloseTo(40, 9);
  });

  it("phase 2 measures the CCW sweep from the placed start", () => {
    const frame = must(dimFrame("arc", [ctr, P(20, 0)]));
    expect(frame.measure(P(0, 20)).angle).toBeCloseTo(90, 9);
    expect(frame.measure(P(-20, 0)).angle).toBeCloseTo(180, 9);
    // CCW, so a cursor clockwise of the start reads the LONG way round.
    expect(frame.measure(P(0, -20)).angle).toBeCloseTo(270, 9);
    expect(frame.measure(P(20, 0)).angle).toBeCloseTo(0, 9);
  });

  it("phase 2 rebuild lands on the sweep, keeping the cursor's radius", () => {
    const frame = must(dimFrame("arc", [ctr, P(20, 0)]));
    const p = frame.rebuild({ angle: 90 }, P(50, 10));
    expect(Math.atan2(p.y, p.x) * (180 / Math.PI)).toBeCloseTo(90, 9);
    expect(dist(ctr, p)).toBeCloseTo(dist(ctr, P(50, 10)), 9);
  });

  it("a start point off the +U axis shifts the sweep origin", () => {
    const frame = must(dimFrame("arc", [ctr, P(0, 20)]));
    expect(frame.measure(P(-20, 0)).angle).toBeCloseTo(90, 9);
  });
});

describe("arc3p frames", () => {
  const S = P(0, 0);
  const E = P(10, 0);
  const frame = must(dimFrame("arc3p", [S, E]));
  /** The radius the frame's own measure reports for a cursor — the arc's radius. */
  const radiusAt = (p: Point2): number => frame.measure(p).radius!;

  it("phase 1 is the CHORD: the same length + angle numbers as a line", () => {
    const chord = must(dimFrame("arc3p", [P(10, 20)]));
    expect(chord.measure(P(40, 20))).toEqual({ length: 30, angle: 0 });
  });

  it("phase 2 measures the radius of the arc through both endpoints and the cursor", () => {
    expect(radiusAt(P(5, 5))).toBeCloseTo(5, 9); // semicircle on the chord
    expect(radiusAt(P(5, -5))).toBeCloseTo(5, 9); // …either side, same number
    expect(radiusAt(P(5, 2.5))).toBeCloseTo(6.25, 9);
  });

  it("a cursor ON the chord has no arc — the radius reads 0", () => {
    expect(radiusAt(P(5, 0))).toBe(0);
    expect(radiusAt(P(50, 0))).toBe(0);
  });

  it("a locked radius keeps the cursor's position ALONG the chord and its side", () => {
    for (const cursor of [P(3, 4), P(3, -4), P(-2, 2), P(11, -9)]) {
      const p = frame.rebuild({ radius: 9 }, cursor);
      expect(p.x).toBeCloseTo(cursor.x, 9); // u preserved (chord runs +U here)
      expect(radiusAt(p)).toBeCloseTo(9, 6);
      expect(Math.sign(p.y)).toBe(Math.sign(cursor.y));
    }
  });

  it("PROPERTY: a typed radius is the radius that commits, across the whole sweep", () => {
    for (const r of [5.01, 6, 9, 25, 200, 5000]) {
      for (const cursor of [P(1, 1), P(5, -0.5), P(8, 30), P(-3, -2), P(4.9, 12)]) {
        expect(radiusAt(frame.rebuild({ radius: r }, cursor))).toBeCloseTo(r, 5);
      }
    }
  });

  it("clamps a cursor past the circle's extent, so the typed radius still holds", () => {
    // The r=9 circle on this chord spans u ∈ [−4, 14]; the cursor is well past it.
    const p = frame.rebuild({ radius: 9 }, P(60, 3));
    expect(p.x).toBeCloseTo(14, 9);
    expect(radiusAt(p)).toBeCloseTo(9, 6);
  });

  it("a LARGER radius flattens the bulge toward the chord", () => {
    const flatter = frame.rebuild({ radius: 20 }, P(5, 1));
    expect(radiusAt(flatter)).toBeCloseTo(20, 6);
    expect(flatter.y).toBeGreaterThan(0);
    expect(flatter.y).toBeLessThan(1);
  });

  it("keeps a MAJOR arc major (the cursor's side of the centre height)", () => {
    const major = frame.rebuild({ radius: 8 }, P(5, 9));
    expect(radiusAt(major)).toBeCloseTo(8, 6);
    // Past the centre height ⇒ still the far half of the circle, not the near one.
    expect(major.y).toBeGreaterThan(8);
  });

  it("CLAMPS a radius below half the chord — the smallest arc that still spans it", () => {
    const clamped = frame.rebuild({ radius: 1 }, P(5, 3));
    expect(radiusAt(clamped)).toBeCloseTo(5, 6);
    expect(clamped).toEqual({ x: 5, y: 5 });
  });

  it("a degenerate chord stays finite (no NaN)", () => {
    const degenerate = must(dimFrame("arc3p", [P(2, 2), P(2, 2)]));
    const p = degenerate.rebuild({ radius: 7 }, P(9, 4));
    expect(Number.isFinite(p.x)).toBe(true);
    expect(Number.isFinite(p.y)).toBe(true);
  });

  it("anchors the R chip between the chord midpoint and the cursor", () => {
    expect(frame.fields[0].anchor(P(5, 20))).toEqual({ x: 5, y: 10 });
  });
});

describe("line arc-mode frame — the tangent arc's radius (SP-4 W3)", () => {
  const A = P(10, 0);
  const T = P(1, 0); // the chain leaves the anchor along +U
  const chain = { arcMode: true, tangent: T };
  const frame = must(dimFrame("line", [P(0, 0), A], undefined, chain));
  const radiusAt = (p: Point2): number => frame.measure(p).radius!;

  it("the 4th param DEFAULTS to nothing — a plain line frame is untouched", () => {
    const plain = must(dimFrame("line", [P(0, 0), A]));
    expect(plain.fields.map((f) => f.field)).toEqual(["length", "angle"]);
    // …and arc mode WITHOUT a tangent is still the straight frame (nothing to be
    // tangent to, so the mode is inert here exactly as it is in the machine).
    const noTangent = must(dimFrame("line", [P(0, 0), A], undefined, { arcMode: true }));
    expect(noTangent.fields.map((f) => f.field)).toEqual(["length", "angle"]);
  });

  it("is ONE driving radius field, anchored between the anchor and the cursor", () => {
    expect(frame.fields).toHaveLength(1);
    expect(frame.fields[0]).toMatchObject({ field: "radius", label: "R", domain: "length", drives: true });
    expect(frame.fields[0].anchor(P(20, 10))).toEqual({ x: 15, y: 5 });
  });

  it("measures the arc that leaves the anchor along the tangent", () => {
    expect(radiusAt(P(20, 10))).toBeCloseTo(10, 9); // centre (10,10), quarter turn
    expect(radiusAt(P(20, -10))).toBeCloseTo(10, 9); // mirrored — same number
    expect(radiusAt(P(10, 20))).toBeCloseTo(10, 9); // half turn
  });

  it("a cursor ON the tangent ray has no arc — the radius reads 0", () => {
    expect(radiusAt(P(30, 0))).toBe(0);
    expect(radiusAt(P(-30, 0))).toBe(0);
  });

  /** How far the arc through `q` TURNS off the anchor, in the direction it bends.
   *  The tangent is +U here, so every centre sits at x = A.x. */
  const turnOf = (q: Point2): number => {
    const sgn = Math.sign(q.y - A.y) || 1;
    const cy = A.y + sgn * radiusAt(q);
    const d = sgn * (Math.atan2(q.y - cy, q.x - A.x) - Math.atan2(A.y - cy, 0));
    return ((d % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI);
  };

  it("a locked radius keeps the SIDE the arc bends to and how far it TURNS", () => {
    for (const cursor of [P(20, 10), P(20, -10), P(10, 20), P(4, -3)]) {
      const p = frame.rebuild({ radius: 25 }, cursor);
      expect(radiusAt(p)).toBeCloseTo(25, 6);
      // Same side of the tangent line (y = 0 here) …
      expect(Math.sign(p.y - A.y)).toBe(Math.sign(cursor.y - A.y));
      // … and the same turn: only the radius moved.
      expect(turnOf(p)).toBeCloseTo(turnOf(cursor), 6);
    }
  });

  it("PROPERTY: a typed radius is the radius that commits, all the way round", () => {
    for (const r of [0.5, 3, 12.5, 400]) {
      for (const cursor of [P(11, 1), P(20, 10), P(10, 9), P(2, -6), P(-5, 4), P(13, -0.25)]) {
        const p = frame.rebuild({ radius: r }, cursor);
        expect(radiusAt(p)).toBeCloseTo(r, 6);
        // The arc still leaves the anchor along the tangent: the anchor is one of
        // its two endpoints, so it is exactly `r` from the rebuilt centre.
        expect(Math.hypot(p.x - A.x, p.y - A.y)).toBeLessThanOrEqual(2 * r + 1e-9);
      }
    }
  });

  it("a bigger radius sweeps the cursor further out, same side, same turn direction", () => {
    const near = frame.rebuild({ radius: 10 }, P(20, 10)); // the identity case
    const far = frame.rebuild({ radius: 40 }, P(20, 10));
    expect(near).toEqual({ x: 20, y: 10 });
    expect(far.y).toBeGreaterThan(0);
    expect(far.x).toBeGreaterThan(near.x);
  });

  it("|R| — a negative typed radius never mirrors the arc across its own tangent", () => {
    const p = frame.rebuild({ radius: -12 }, P(20, 10));
    expect(p.y).toBeGreaterThan(0);
    expect(radiusAt(p)).toBeCloseTo(12, 6);
  });

  it("a degenerate cursor (on the tangent ray) hands the cursor back unchanged", () => {
    const c = P(30, 0);
    expect(frame.rebuild({ radius: 8 }, c)).toBe(c);
  });

  it("a degenerate TANGENT stays finite (no NaN)", () => {
    const dead = must(dimFrame("line", [A], undefined, { arcMode: true, tangent: P(0, 0) }));
    const c = P(9, 4);
    expect(dead.measure(c)).toEqual({ radius: 0 });
    expect(dead.rebuild({ radius: 7 }, c)).toBe(c);
  });
});

describe("ellipse frames", () => {
  it("phase 1 measures the major radius (geometry only)", () => {
    const frame = must(dimFrame("ellipse", [P(0, 0)]));
    expect(frame.measure(P(30, 40))).toEqual({ major: 50 });
  });

  it("phase 2 measures the PERPENDICULAR distance to the major axis", () => {
    const frame = must(dimFrame("ellipse", [P(0, 0), P(40, 0)]));
    expect(frame.measure(P(15, 12))).toEqual({ minor: 12 });
    expect(frame.measure(P(15, -12))).toEqual({ minor: 12 });
  });

  it("phase 2 rebuild preserves which SIDE of the axis the cursor is on", () => {
    const frame = must(dimFrame("ellipse", [P(0, 0), P(40, 0)]));
    expect(frame.rebuild({ minor: 9 }, P(15, 12))).toEqual({ x: 15, y: 9 });
    expect(frame.rebuild({ minor: 9 }, P(15, -12))).toEqual({ x: 15, y: -9 });
    // On the axis itself: no side left to preserve, so the POSITIVE normal wins.
    expect(frame.rebuild({ minor: 9 }, P(15, 0))).toEqual({ x: 15, y: 9 });
  });

  it("a degenerate major axis falls back to plane +U (finite, no NaN)", () => {
    const frame = must(dimFrame("ellipse", [P(3, 3), P(3, 3)]));
    const p = frame.rebuild({ minor: 5 }, P(10, 10));
    expect(Number.isFinite(p.x)).toBe(true);
    expect(Number.isFinite(p.y)).toBe(true);
  });
});

describe("polygon frame", () => {
  const ctr = P(0, 0);

  it("reports the side count from the machine state", () => {
    expect(must(dimFrame("polygon", [ctr], 5)).measure(P(30, 0))).toEqual({ radius: 30, sides: 5 });
    // Absent ⇒ the machine's own default, not a guess.
    expect(must(dimFrame("polygon", [ctr])).measure(P(30, 0)).sides).toBe(6);
  });

  it("the side count NEVER moves the vertex", () => {
    const frame = must(dimFrame("polygon", [ctr], 5));
    const cursor = P(30, 40);
    expect(frame.rebuild({ radius: 50, sides: 3 }, cursor)).toEqual(
      frame.rebuild({ radius: 50, sides: 12 }, cursor),
    );
  });

  it("anchors the count chip at the centre and the radius mid-ray", () => {
    const [radius, sides] = must(dimFrame("polygon", [ctr], 5)).fields;
    expect(sides.anchor(P(30, 40))).toEqual(ctr);
    expect(radius.anchor(P(30, 40))).toEqual({ x: 15, y: 20 });
  });
});

describe("slot width frame — FULL width, side preserved", () => {
  const p0 = P(0, 0);
  const p1 = P(40, 0);
  const frame = must(dimFrame("slot", [p0, p1]));

  it("measures twice the perpendicular offset", () => {
    expect(frame.measure(P(20, 6))).toEqual({ width: 12 });
    expect(frame.measure(P(20, -6))).toEqual({ width: 12 });
  });

  it("rebuild puts the cursor at half the width on its own side", () => {
    expect(frame.rebuild({ width: 20 }, P(20, 6))).toEqual({ x: 20, y: 10 });
    expect(frame.rebuild({ width: 20 }, P(20, -6))).toEqual({ x: 20, y: -10 });
  });

  it("keeps the cursor's position ALONG the centreline", () => {
    expect(frame.rebuild({ width: 20 }, P(-13, 6)).x).toBeCloseTo(-13, 9);
  });

  it("a degenerate centreline stays finite", () => {
    const degenerate = must(dimFrame("slot", [P(1, 1), P(1, 1)]));
    const p = degenerate.rebuild({ width: 8 }, P(5, 5));
    expect(Number.isFinite(p.x)).toBe(true);
    expect(Number.isFinite(p.y)).toBe(true);
  });
});

describe("chip anchors", () => {
  it("puts extents at their midpoint and angles ~30% out from the vertex", () => {
    // Chained: a counter leg exists, so the scalar frame exposes its ∠ chip.
    const [length, angle] = must(dimFrame("line", [P(0, 0), P(0, 0)])).fields;
    expect(length.anchor(P(100, 0))).toEqual({ x: 50, y: 0 });
    expect(angle.anchor(P(100, 0)).x).toBeCloseTo(30, 9);
    expect(angle.anchor(P(100, 0)).y).toBeCloseTo(0, 9);
  });

  it("the angle field carries no clusterId — the controller pins its own anchor", () => {
    // `SketchController.syncAngleReference` unconditionally overrides this
    // chip's position to the arc preview's bisector; a shared cluster would
    // let `HtmlOverlayDriver`'s screen-space push-apart drag it back off that
    // fixed spot whenever the (length-dependent) length chip lands nearby.
    const [, angle] = must(dimFrame("line", [P(0, 0), P(0, 0)])).fields;
    expect(angle.clusterId).toBeUndefined();
  });

  it("puts a rect's W on its horizontal edge and H on its vertical one", () => {
    const [w, h] = must(dimFrame("rect", [P(0, 0)])).fields;
    expect(w.anchor(P(80, 40))).toEqual({ x: 40, y: 0 });
    expect(h.anchor(P(80, 40))).toEqual({ x: 80, y: 20 });
  });

  it("co-anchors Ø and R on the centre→cursor midpoint", () => {
    const [d, r] = must(dimFrame("circle", [P(0, 0)])).fields;
    expect(d.anchor(P(20, 0))).toEqual({ x: 10, y: 0 });
    expect(r.anchor(P(20, 0))).toEqual({ x: 10, y: 0 });
  });
});
