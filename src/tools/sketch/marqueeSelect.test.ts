/*
 * marqueeSelect — window (full containment) vs crossing (touch) box selection.
 *
 * The matrix below is the whole contract: both modes × Line / Circle / Arc / Point,
 * plus the three behaviours that distinguish this from the oracle's bbox-both-ways
 * `Sketch::findInRect` — arc SWEEP gating, a half-covered line (window fails,
 * crossing passes), and a rect drawn wholly INSIDE a circle selecting nothing.
 */
import { describe, it, expect } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import { marqueeHits, marqueeModeFor, marqueeRectFrom, rectContains } from "./marqueeSelect";

const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({
  id,
  type: "Line",
  p0,
  p1,
});
const circle = (id: string, center: [number, number], radius: number): SketchEntity => ({
  id,
  type: "Circle",
  center,
  radius,
});
const arc = (
  id: string,
  center: [number, number],
  radius: number,
  start: [number, number],
  end: [number, number],
): SketchEntity => ({ id, type: "Arc", center, radius, start, end });
const point = (id: string, p0: [number, number]): SketchEntity => ({ id, type: "Point", p0 });

/** Point on a circle at `deg` (CCW from +X). */
const at = (cx: number, cy: number, r: number, deg: number): [number, number] => [
  cx + r * Math.cos((deg * Math.PI) / 180),
  cy + r * Math.sin((deg * Math.PI) / 180),
];

const R = (ax: number, ay: number, bx: number, by: number) => marqueeRectFrom(ax, ay, bx, by);

describe("marqueeModeFor", () => {
  it("rightward is window, leftward is crossing, straight down is window", () => {
    expect(marqueeModeFor(100, 200)).toBe("window");
    expect(marqueeModeFor(200, 100)).toBe("crossing");
    expect(marqueeModeFor(100, 100)).toBe("window");
  });
});

describe("marqueeRectFrom / rectContains", () => {
  it("normalizes opposite corners in any order", () => {
    expect(R(10, 20, -5, 4)).toEqual({ minX: -5, minY: 4, maxX: 10, maxY: 20 });
  });

  it("is inclusive of the border", () => {
    const rect = R(0, 0, 10, 10);
    expect(rectContains(rect, [0, 0])).toBe(true);
    expect(rectContains(rect, [10, 10])).toBe(true);
    expect(rectContains(rect, [10.0001, 5])).toBe(false);
  });
});

describe("marqueeHits — WINDOW (full containment)", () => {
  it("selects a line only when BOTH endpoints are inside", () => {
    const l = line("l1", [0, 0], [10, 0]);
    expect(marqueeHits([l], R(-1, -1, 11, 1), "window")).toEqual(["l1"]);
    expect(marqueeHits([l], R(-1, -1, 5, 1), "window")).toEqual([]);
  });

  it("selects a circle only when its whole center±radius bbox is inside", () => {
    const c = circle("c1", [50, 50], 10);
    expect(marqueeHits([c], R(39, 39, 61, 61), "window")).toEqual(["c1"]);
    expect(marqueeHits([c], R(41, 41, 61, 61), "window")).toEqual([]); // clips the left edge
  });

  it("selects a point by containment", () => {
    const p = point("p1", [5, 5]);
    expect(marqueeHits([p], R(0, 0, 10, 10), "window")).toEqual(["p1"]);
    expect(marqueeHits([p], R(6, 6, 10, 10), "window")).toEqual([]);
  });

  it("selects an arc from endpoints + the extrema ON its sweep", () => {
    // Quarter arc 0°→90°, r=10: extent is exactly [0,10]².
    const a = arc("a1", [0, 0], 10, at(0, 0, 10, 0), at(0, 0, 10, 90));
    expect(marqueeHits([a], R(-1, -1, 11, 11), "window")).toEqual(["a1"]);
    expect(marqueeHits([a], R(-1, -1, 9, 11), "window")).toEqual([]); // clips the 0° extremum
  });

  it("SWEEP GATING: extrema outside the sweep don't count (arc ≠ its full circle)", () => {
    // 45°→135°, r=10 about the origin. Only the 90° extremum (0,10) is on the sweep,
    // so the real extent is x∈[-7.08,7.08], y∈[7.07,10] — far smaller than the
    // circle's [-10,10]². A rect covering only that extent must select the ARC and
    // NOT a circle of the same center + radius.
    const a = arc("a1", [0, 0], 10, at(0, 0, 10, 45), at(0, 0, 10, 135));
    const c = circle("c1", [0, 0], 10);
    const rect = R(-8, 7, 8, 11);
    expect(marqueeHits([a], rect, "window")).toEqual(["a1"]);
    expect(marqueeHits([c], rect, "window")).toEqual([]);
  });

  it("returns ids in entity order, skipping non-contained ones", () => {
    const entities = [line("l1", [0, 0], [4, 4]), line("l2", [90, 90], [95, 95]), point("p1", [2, 2])];
    expect(marqueeHits(entities, R(-1, -1, 5, 5), "window")).toEqual(["l1", "p1"]);
  });

  it("ignores entities missing the fields their type needs", () => {
    const broken: SketchEntity[] = [
      { id: "bad-line", type: "Line", p0: [1, 1] },
      { id: "bad-circle", type: "Circle", center: [1, 1] },
      { id: "bad-arc", type: "Arc", center: [1, 1], radius: 1 },
      { id: "bad-point", type: "Point" },
    ];
    expect(marqueeHits(broken, R(-100, -100, 100, 100), "window")).toEqual([]);
    expect(marqueeHits(broken, R(-100, -100, 100, 100), "crossing")).toEqual([]);
  });
});

describe("marqueeHits — CROSSING (touch)", () => {
  it("a half-covered line fails WINDOW but passes CROSSING", () => {
    const l = line("l1", [0, 0], [10, 0]);
    const rect = R(-1, -1, 5, 1); // the rect's right edge (x=5) cuts the line
    expect(marqueeHits([l], rect, "window")).toEqual([]);
    expect(marqueeHits([l], rect, "crossing")).toEqual(["l1"]);
  });

  it("still selects a fully-contained entity (crossing ⊇ window)", () => {
    const l = line("l1", [0, 0], [10, 0]);
    expect(marqueeHits([l], R(-1, -1, 11, 1), "crossing")).toEqual(["l1"]);
  });

  it("selects a circle the rect boundary cuts", () => {
    const c = circle("c1", [50, 50], 10);
    // [40,40] is 14.1 from the center (outside), [50,50] is the center (inside) ⇒ the
    // rect boundary must cross the curve.
    expect(marqueeHits([c], R(40, 40, 50, 50), "crossing")).toEqual(["c1"]);
  });

  it("a rect drawn WHOLLY INSIDE a circle selects nothing (no curve touched)", () => {
    const c = circle("c1", [50, 50], 10);
    const rect = R(45, 45, 55, 55); // farthest corner is 7.07 < r ⇒ entirely interior
    expect(marqueeHits([c], rect, "window")).toEqual([]);
    expect(marqueeHits([c], rect, "crossing")).toEqual([]);
  });

  it("selects an arc the rect boundary cuts, sweep-gated", () => {
    const a = arc("a1", [0, 0], 10, at(0, 0, 10, 0), at(0, 0, 10, 90)); // first quadrant
    // x=8 edge meets the arc at (8,6), angle 36.9° — on the sweep.
    expect(marqueeHits([a], R(0, 0, 8, 8), "window")).toEqual([]);
    expect(marqueeHits([a], R(0, 0, 8, 8), "crossing")).toEqual(["a1"]);
    // The mirrored rect meets the same CIRCLE at (-8,6) but that angle (143°) is off
    // the sweep, so the arc is not touched.
    expect(marqueeHits([a], R(-8, 0, 0, 8), "crossing")).toEqual([]);
    expect(marqueeHits([circle("c1", [0, 0], 10)], R(-8, 0, 0, 8), "crossing")).toEqual(["c1"]);
  });

  it("handles a bare Point by containment (it has no curve to cross)", () => {
    const p = point("p1", [5, 5]);
    expect(marqueeHits([p], R(0, 0, 10, 10), "crossing")).toEqual(["p1"]);
    expect(marqueeHits([p], R(0, 0, 4, 4), "crossing")).toEqual([]);
  });
});

describe("marqueeHits — degenerate rect", () => {
  const entities = [line("l1", [0, 0], [10, 0]), circle("c1", [5, 0], 3), point("p1", [5, 5])];

  it("a zero-size rect selects only a point sitting exactly on it", () => {
    expect(marqueeHits(entities, R(5, 5, 5, 5), "window")).toEqual(["p1"]);
    expect(marqueeHits(entities, R(5, 5, 5, 5), "crossing")).toEqual(["p1"]);
  });

  it("a zero-size rect ON a curve selects nothing (its edges are zero-length)", () => {
    expect(marqueeHits(entities, R(5, 0, 5, 0), "window")).toEqual([]);
    expect(marqueeHits(entities, R(5, 0, 5, 0), "crossing")).toEqual([]);
  });

  it("a zero-size rect in empty space selects nothing", () => {
    expect(marqueeHits(entities, R(-40, -40, -40, -40), "crossing")).toEqual([]);
  });
});
