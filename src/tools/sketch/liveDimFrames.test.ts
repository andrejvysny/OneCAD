import { describe, it, expect } from "vitest";
import { dimFrame, type DimFrame } from "./liveDimFrames";
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
  ["ellipse phase 1 (major)", must(dimFrame("ellipse", [P(0, 0)]))],
  ["ellipse phase 2 (minor)", must(dimFrame("ellipse", [P(0, 0), P(30, 10)]))],
  ["polygon", must(dimFrame("polygon", [P(2, -2)], 5))],
  ["slot phase 1 (centreline)", must(dimFrame("slot", [P(-5, -5)]))],
  ["slot phase 2 (width)", must(dimFrame("slot", [P(-5, -5), P(25, 5)]))],
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
    expect(dimFrame("slot", [P(0, 0), P(1, 1), P(2, 2)])).toBeNull();
  });

  it("exposes the documented Tab order per phase", () => {
    const order = (f: DimFrame | null): string[] => must(f).fields.map((s) => s.field);
    expect(order(dimFrame("line", [P(0, 0)]))).toEqual(["length", "angle"]);
    expect(order(dimFrame("rect", [P(0, 0)]))).toEqual(["width", "height"]);
    expect(order(dimFrame("circle", [P(0, 0)]))).toEqual(["diameter", "radius"]);
    // Radius first: an ARMED digit types the radius, Tab reaches the side count.
    expect(order(dimFrame("polygon", [P(0, 0)]))).toEqual(["radius", "sides"]);
    expect(order(dimFrame("slot", [P(0, 0), P(10, 0)]))).toEqual(["width"]);
  });

  it("marks only the fields with a wire kind as driving", () => {
    const drives = (f: DimFrame | null): boolean[] => must(f).fields.map((s) => s.drives);
    expect(drives(dimFrame("ellipse", [P(0, 0)]))).toEqual([false]);
    expect(drives(dimFrame("ellipse", [P(0, 0), P(10, 0)]))).toEqual([false]);
    expect(drives(dimFrame("arc", [P(0, 0), P(10, 0)]))).toEqual([false]); // sweep
    expect(drives(dimFrame("arc", [P(0, 0)]))).toEqual([true]); // radius
    expect(drives(dimFrame("polygon", [P(0, 0)]))).toEqual([true, false]); // R, n
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
    const [length, angle] = must(dimFrame("line", [P(0, 0)])).fields;
    expect(length.anchor(P(100, 0))).toEqual({ x: 50, y: 0 });
    expect(angle.anchor(P(100, 0)).x).toBeCloseTo(30, 9);
    expect(angle.anchor(P(100, 0)).y).toBeCloseTo(0, 9);
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
