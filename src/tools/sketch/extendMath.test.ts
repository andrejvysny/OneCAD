/*
 * extendMath (PURE): grow a line/arc end out to the first entity its unbounded
 * continuation reaches. Exercises end selection from the hover, the forward and
 * backward candidate windows, the `withinExtent` gate on the OTHER entity, the
 * nearest-crossing pick, the `minSize` floor, the arc wrap guard (an extension
 * may never close the arc onto its own far endpoint), the closed/unmodelled
 * curve opt-outs, and construction carry-through. One crossing is cross-checked
 * against snapEngine's point-returning intersector so the two implementations
 * are pinned to agree (the discipline trimMath.test.ts sets).
 */
import { describe, it, expect } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { extendPieces, extendPreview } from "./extendMath";
import { entityIntersections } from "./snapEngine";

const SMALL = 1e-6;
const near = (p: Point2 | undefined, x: number, y: number): void => {
  expect(p).toBeDefined();
  expect(p!.x).toBeCloseTo(x, 6);
  expect(p!.y).toBeCloseTo(y, 6);
};
const hover = (x: number, y: number): Point2 => ({ x, y });
/** A vertical line segment at `x`, spanning [y0,y1]. */
const vline = (id: string, x: number, y0 = -50, y1 = 50): SketchEntity => ({
  id,
  type: "Line",
  p0: [x, y0],
  p1: [x, y1],
});

// ── Line target ────────────────────────────────────────────────────────────────
describe("extendPieces — line", () => {
  const target: SketchEntity = { id: "t", type: "Line", p0: [0, 0], p1: [10, 0] };

  it("a hover on the FAR half grows the end forward to the crossing", () => {
    const r = extendPieces(target, hover(9, 0), [vline("a", 20)], { minSize: SMALL });
    expect(r).not.toBeNull();
    expect(r!.end).toBe("end");
    near(r!.target, 20, 0);
    expect(r!.grew).toBeCloseTo(10, 9);
    // The replacement runs from the UNTOUCHED endpoint to the new tip.
    near(r!.extended.p0, 0, 0);
    near(r!.extended.p1, 20, 0);
    // The addition is only the gained span.
    near(r!.addition.p0, 10, 0);
    near(r!.addition.p1, 20, 0);
  });

  it("a hover on the NEAR half grows the start backward instead", () => {
    const r = extendPieces(target, hover(1, 0), [vline("a", -5)], { minSize: SMALL });
    expect(r!.end).toBe("start");
    near(r!.target, -5, 0);
    expect(r!.grew).toBeCloseTo(5, 9);
    near(r!.extended.p0, -5, 0);
    near(r!.extended.p1, 10, 0);
    near(r!.addition.p0, -5, 0);
    near(r!.addition.p1, 0, 0);
  });

  it("the same entity extends BOTH ways — the hover, not the geometry, decides", () => {
    const others = [vline("a", -5), vline("b", 20)];
    expect(extendPieces(target, hover(9, 0), others, { minSize: SMALL })!.end).toBe("end");
    expect(extendPieces(target, hover(1, 0), others, { minSize: SMALL })!.end).toBe("start");
  });

  it("the NEAREST forward crossing wins (repeat-extend walks outward)", () => {
    const r = extendPieces(target, hover(9, 0), [vline("far", 30), vline("near", 20)], {
      minSize: SMALL,
    });
    near(r!.target, 20, 0);
  });

  it("the nearest BACKWARD crossing wins too", () => {
    const r = extendPieces(target, hover(1, 0), [vline("far", -30), vline("near", -5)], {
      minSize: SMALL,
    });
    near(r!.target, -5, 0);
  });

  it("addition and extended meet EXACTLY at the tip and the old endpoint", () => {
    const r = extendPieces(target, hover(9, 0), [vline("a", 20)], { minSize: SMALL })!;
    expect(r.addition.p1).toEqual(r.extended.p1); // the new tip, bit-identical
    expect(r.addition.p0).toEqual({ x: target.p1![0], y: target.p1![1] }); // the old end
  });

  it("REJECTS a crossing outside the other entity's own extent", () => {
    // The infinite line through the target meets x=20 at (20,0), but that segment
    // only spans y∈[10,50] — the extension would reach empty space.
    expect(extendPieces(target, hover(9, 0), [vline("a", 20, 10, 50)], { minSize: SMALL })).toBeNull();
    // The same segment lowered to straddle y=0 DOES qualify.
    const r = extendPieces(target, hover(9, 0), [vline("a", 20, -1, 1)], { minSize: SMALL });
    near(r!.target, 20, 0);
  });

  it("returns null with no crossing at all (empty / parallel / behind-only)", () => {
    expect(extendPieces(target, hover(9, 0), [], { minSize: SMALL })).toBeNull();
    const parallel: SketchEntity = { id: "p", type: "Line", p0: [0, 5], p1: [30, 5] };
    expect(extendPieces(target, hover(9, 0), [parallel], { minSize: SMALL })).toBeNull();
    // Growing the END, a crossing BEHIND the start is not a candidate.
    expect(extendPieces(target, hover(9, 0), [vline("a", -5)], { minSize: SMALL })).toBeNull();
  });

  it("a crossing ON the target's own endpoint is not somewhere to grow to", () => {
    expect(extendPieces(target, hover(9, 0), [vline("a", 10)], { minSize: SMALL })).toBeNull();
    expect(extendPieces(target, hover(1, 0), [vline("a", 0)], { minSize: SMALL })).toBeNull();
  });

  it("growth below minSize is refused (an invisible extension is not an edit)", () => {
    const others = [vline("a", 10.5)];
    expect(extendPieces(target, hover(9, 0), others, { minSize: 4 })).toBeNull();
    expect(extendPieces(target, hover(9, 0), others, { minSize: 0.1 })!.grew).toBeCloseTo(0.5, 9);
  });

  it("carries `construction` onto BOTH drafts", () => {
    const cons: SketchEntity = { ...target, construction: true };
    const r = extendPieces(cons, hover(9, 0), [vline("a", 20)], { minSize: SMALL })!;
    expect(r.extended.construction).toBe(true);
    expect(r.addition.construction).toBe(true);
    // …and never invents one on ordinary geometry.
    const plain = extendPieces(target, hover(9, 0), [vline("a", 20)], { minSize: SMALL })!;
    expect(plain.extended.construction).toBeUndefined();
  });

  it("cross-check: the tip IS the intersection snapEngine reports for the grown entity", () => {
    const other = vline("a", 20, -5, 5);
    const r = extendPieces(target, hover(9, 0), [other], { minSize: SMALL })!;
    const grown: SketchEntity = {
      id: "grown",
      type: "Line",
      p0: [r.extended.p0!.x, r.extended.p0!.y],
      p1: [r.extended.p1!.x, r.extended.p1!.y],
    };
    const pts = entityIntersections(grown, other);
    expect(pts).toHaveLength(1);
    expect(pts[0].x).toBeCloseTo(r.target.x, 9);
    expect(pts[0].y).toBeCloseTo(r.target.y, 9);
  });
});

// ── Arc target ─────────────────────────────────────────────────────────────────
describe("extendPieces — arc", () => {
  /** Quarter arc, centre (0,0) r=10, CCW 0° → 90°. */
  const arc: SketchEntity = {
    id: "t",
    type: "Arc",
    center: [0, 0],
    radius: 10,
    start: [10, 0],
    end: [0, 10],
  };
  const at = (deg: number): Point2 => ({
    x: 10 * Math.cos((deg * Math.PI) / 180),
    y: 10 * Math.sin((deg * Math.PI) / 180),
  });

  it("grows the END counter-clockwise to the first crossing past it", () => {
    // A segment along y=0 to the LEFT: of the two circle crossings only (−10,0)
    // lies on it, so the arc grows CCW from 90° to 180°.
    const other: SketchEntity = { id: "o", type: "Line", p0: [-20, 0], p1: [-5, 0] };
    const r = extendPieces(arc, hover(at(80).x, at(80).y), [other], { minSize: SMALL });
    expect(r!.end).toBe("end");
    near(r!.target, -10, 0);
    expect(r!.grew).toBeCloseTo(10 * (Math.PI / 2), 9); // a further quarter turn
    // The replacement keeps the untouched start and runs CCW to the new tip.
    near(r!.extended.start, 10, 0);
    near(r!.extended.end, -10, 0);
    expect(r!.extended.radius).toBe(10);
    // The addition is the gained quarter, CCW from the OLD end.
    near(r!.addition.start, 0, 10);
    near(r!.addition.end, -10, 0);
  });

  it("grows the START clockwise instead when the hover is near it", () => {
    const other: SketchEntity = { id: "o", type: "Line", p0: [0, -20], p1: [0, -5] };
    const r = extendPieces(arc, hover(at(10).x, at(10).y), [other], { minSize: SMALL });
    expect(r!.end).toBe("start");
    near(r!.target, 0, -10);
    expect(r!.grew).toBeCloseTo(10 * (Math.PI / 2), 9);
    near(r!.extended.start, 0, -10); // the new tip is now the CCW start
    near(r!.extended.end, 0, 10); // the untouched end
    near(r!.addition.start, 0, -10);
    near(r!.addition.end, 10, 0); // …up to the OLD start
  });

  it("WRAP GUARD: never grows the long way round onto its own far endpoint", () => {
    // The only crossing is at (0,10) — the arc's OWN end. Growing the start CW to
    // it means a complete turn, not an extension.
    const other: SketchEntity = { id: "o", type: "Line", p0: [-5, 10], p1: [5, 10] };
    expect(extendPieces(arc, hover(at(10).x, at(10).y), [other], { minSize: SMALL })).toBeNull();
    // Mirror case: growing the END forward, a crossing at the arc's own START is
    // behind the growth window, never a full turn ahead of it.
    const other2: SketchEntity = { id: "o2", type: "Line", p0: [10, -5], p1: [10, 5] };
    expect(extendPieces(arc, hover(at(80).x, at(80).y), [other2], { minSize: SMALL })).toBeNull();
  });

  it("…but a near-full extension that stops SHORT of the wrap is allowed", () => {
    // Crossing at 350° — 260° of growth, just inside the 270° of free circle.
    const x = 10 * Math.cos((350 * Math.PI) / 180);
    const other: SketchEntity = { id: "o", type: "Line", p0: [x, -5], p1: [x, 0] };
    const r = extendPieces(arc, hover(at(80).x, at(80).y), [other], { minSize: SMALL });
    expect(r!.end).toBe("end");
    near(r!.target, at(350).x, at(350).y);
    expect(r!.grew).toBeCloseTo(10 * ((260 * Math.PI) / 180), 6);
  });

  it("gates arc candidates by the other entity's extent, and floors tiny growth", () => {
    // The infinite line through this segment meets the circle at (±10,0), but the
    // segment itself stops short of both — the arc would grow into empty space.
    const short: SketchEntity = { id: "o", type: "Line", p0: [-9, 0], p1: [-5, 0] };
    expect(extendPieces(arc, hover(at(80).x, at(80).y), [short], { minSize: SMALL })).toBeNull();
    // 91° is a 0.17mm growth: real, but below a 4mm floor.
    const p = at(91);
    const tiny: SketchEntity = { id: "o2", type: "Line", p0: [0, 0], p1: [p.x * 2, p.y * 2] };
    expect(extendPieces(arc, hover(at(80).x, at(80).y), [tiny], { minSize: 4 })).toBeNull();
    expect(extendPieces(arc, hover(at(80).x, at(80).y), [tiny], { minSize: SMALL })).not.toBeNull();
  });

  it("carries `construction` onto both arc drafts", () => {
    const other: SketchEntity = { id: "o", type: "Line", p0: [-20, 0], p1: [-5, 0] };
    const r = extendPieces({ ...arc, construction: true }, hover(at(80).x, at(80).y), [other], {
      minSize: SMALL,
    })!;
    expect(r.extended.construction).toBe(true);
    expect(r.addition.construction).toBe(true);
  });
});

// ── Closed / unmodelled curves ────────────────────────────────────────────────
describe("extendPieces — nothing to extend", () => {
  const crossing = vline("l1", 0);

  it("a CIRCLE has no end to grow ⇒ null", () => {
    const circle: SketchEntity = { id: "c", type: "Circle", center: [0, 0], radius: 10 };
    expect(extendPieces(circle, hover(10, 0), [crossing], { minSize: SMALL })).toBeNull();
  });

  it("an ELLIPSE is not modelled ⇒ null (same opt-out Trim takes)", () => {
    const ell: SketchEntity = {
      id: "e",
      type: "Ellipse",
      center: [0, 0],
      majorR: 20,
      minorR: 5,
      rotation: 0,
    };
    expect(extendPieces(ell, hover(20, 0), [crossing], { minSize: SMALL })).toBeNull();
    // …and an ellipse never offers a crossing to extend TO either.
    const line: SketchEntity = { id: "t", type: "Line", p0: [0, 0], p1: [10, 0] };
    expect(extendPieces(line, hover(9, 0), [ell], { minSize: SMALL })).toBeNull();
  });

  it("a zero-length line is degenerate ⇒ null", () => {
    const dot: SketchEntity = { id: "t", type: "Line", p0: [5, 5], p1: [5, 5] };
    expect(extendPieces(dot, hover(5, 5), [crossing], { minSize: SMALL })).toBeNull();
  });
});

// ── extendPreview ─────────────────────────────────────────────────────────────
describe("extendPreview", () => {
  const target: SketchEntity = { id: "t", type: "Line", p0: [0, 0], p1: [10, 0] };

  it("returns ONLY the gained span (never the whole grown entity)", () => {
    const d = extendPreview(target, hover(9, 0), [vline("a", 20)], { minSize: SMALL });
    near(d!.p0, 10, 0);
    near(d!.p1, 20, 0);
  });

  it("returns null when there is nothing to extend to", () => {
    expect(extendPreview(target, hover(9, 0), [], { minSize: SMALL })).toBeNull();
  });
});
