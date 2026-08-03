/*
 * offsetMath — pure data tests for the 2D chain offset (WP-C T2b).
 *
 * The rectangle cases are the load-bearing ones: an axis-aligned rectangle's
 * offset is exactly an inset/outset rectangle, so every join is checkable
 * against a closed-form corner rather than against a re-derivation of the same
 * code. `expectClosed` additionally asserts the property that actually matters
 * downstream — consecutive drafts SHARE an endpoint exactly, which is what the
 * region detector's degree-2 walk needs (mockSketch.orderedClosedLoop quantises
 * at 1e-6, so a "nearly" welded chain would silently stop being a profile).
 */
import { describe, it, expect } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import type { DraftEntity } from "./toolMachine";
import { buildChain, chainSideSign, offsetChain } from "./offsetMath";

type XY = [number, number];

const line = (id: string, p0: XY, p1: XY, construction?: boolean): SketchEntity => ({
  id,
  type: "Line",
  p0,
  p1,
  ...(construction ? { construction: true } : {}),
});
const arc = (id: string, center: XY, radius: number, start: XY, end: XY): SketchEntity => ({
  id,
  type: "Arc",
  center,
  radius,
  start,
  end,
});
const circle = (id: string, center: XY, radius: number): SketchEntity => ({
  id,
  type: "Circle",
  center,
  radius,
});

const MIN = { minSize: 1e-6 };

/** CCW rectangle (0,0)→(100,0)→(100,50)→(0,50), declared in loop order. */
const RECT: SketchEntity[] = [
  line("l1", [0, 0], [100, 0]),
  line("l2", [100, 0], [100, 50]),
  line("l3", [100, 50], [0, 50]),
  line("l4", [0, 50], [0, 0]),
];

const ends = (d: DraftEntity): [XY, XY] =>
  d.type === "Line" ? [[d.p0!.x, d.p0!.y], [d.p1!.x, d.p1!.y]] : [[d.start!.x, d.start!.y], [d.end!.x, d.end!.y]];

/** Every consecutive pair (wrapping when `closed`) shares an endpoint exactly. */
function expectClosed(drafts: DraftEntity[], closed: boolean): void {
  const n = drafts.length;
  const last = closed ? n : n - 1;
  for (let i = 0; i < last; i++) {
    const a = ends(drafts[i])[1];
    const b = ends(drafts[(i + 1) % n])[0];
    expect(Math.hypot(a[0] - b[0], a[1] - b[1])).toBeLessThan(1e-9);
  }
}

/** The 4 corner points of a rectangle-shaped line chain, sorted for comparison. */
function corners(drafts: DraftEntity[]): XY[] {
  return drafts
    .map((d) => ends(d)[0])
    .sort((p, q) => p[0] - q[0] || p[1] - q[1]);
}

describe("buildChain", () => {
  it("walks a closed rectangle into one loop in traversal order", () => {
    const chain = buildChain(RECT[0], RECT, 1e-6)!;
    expect(chain.closed).toBe(true);
    expect(chain.members.map((m) => m.entity.id)).toEqual(["l1", "l2", "l3", "l4"]);
    expect(chain.members.every((m) => !m.reversed)).toBe(true);
  });

  it("finds the SAME loop from any seed", () => {
    const chain = buildChain(RECT[2], RECT, 1e-6)!;
    expect(chain.closed).toBe(true);
    expect(chain.members.map((m) => m.entity.id)).toEqual(["l3", "l4", "l1", "l2"]);
  });

  it("orients a backwards-declared member with `reversed`", () => {
    // l2 declared END-first: the walk still enters it at (100,0).
    const flipped = [RECT[0], line("l2", [100, 50], [100, 0])];
    const chain = buildChain(flipped[0], flipped, 1e-6)!;
    expect(chain.closed).toBe(false);
    expect(chain.members.map((m) => [m.entity.id, m.reversed])).toEqual([
      ["l1", false],
      ["l2", true],
    ]);
  });

  it("walks BACKWARD from the seed too (seed in the middle of an open chain)", () => {
    const chain = buildChain(RECT[1], RECT.slice(0, 3), 1e-6)!;
    expect(chain.closed).toBe(false);
    expect(chain.members.map((m) => m.entity.id)).toEqual(["l1", "l2", "l3"]);
  });

  it("stops at a BRANCH rather than guessing which way the chain runs", () => {
    const branch = [...RECT.slice(0, 2), line("spur", [100, 0], [140, -40])];
    const chain = buildChain(branch[0], branch, 1e-6)!;
    expect(chain.members.map((m) => m.entity.id)).toEqual(["l1"]);
    expect(chain.closed).toBe(false);
  });

  it("never crosses the construction boundary", () => {
    const mixed = [RECT[0], line("c1", [100, 0], [100, 50], true)];
    expect(buildChain(mixed[0], mixed, 1e-6)!.members.map((m) => m.entity.id)).toEqual(["l1"]);
    expect(buildChain(mixed[1], mixed, 1e-6)!.members.map((m) => m.entity.id)).toEqual(["c1"]);
  });

  it("treats a Circle as its own closed single-member chain", () => {
    const c = circle("c", [0, 0], 10);
    const chain = buildChain(c, [c, ...RECT], 1e-6)!;
    expect(chain).toEqual({ members: [{ entity: c, reversed: false }], closed: true });
  });

  it("refuses an entity with no offsettable curve (Ellipse / Point)", () => {
    const ell: SketchEntity = { id: "e", type: "Ellipse", center: [0, 0], majorR: 8, minorR: 4, rotation: 0 };
    expect(buildChain(ell, [ell], 1e-6)).toBeNull();
  });
});

describe("chainSideSign — the side follows the cursor", () => {
  const chain = buildChain(RECT[0], RECT, 1e-6)!;

  it("is +1 INSIDE a CCW loop (left of travel) and −1 outside", () => {
    expect(chainSideSign(chain, { x: 50, y: 25 })).toBe(1);
    expect(chainSideSign(chain, { x: 50, y: -10 })).toBe(-1);
    expect(chainSideSign(chain, { x: 110, y: 25 })).toBe(-1);
  });

  it("is +1 INSIDE a circle (its CCW left side) and −1 outside", () => {
    const c = circle("c", [0, 0], 10);
    const circleChain = buildChain(c, [c], 1e-6)!;
    expect(chainSideSign(circleChain, { x: 0, y: 3 })).toBe(1);
    expect(chainSideSign(circleChain, { x: 0, y: 20 })).toBe(-1);
  });
});

describe("offsetChain — rectangle (line-line joins extend/trim to the corner)", () => {
  const chain = buildChain(RECT[0], RECT, 1e-6)!;

  it("+3 (inward for this CCW loop) is EXACTLY the inset rectangle", () => {
    const res = offsetChain(chain, 3, MIN);
    if (!res.ok) throw new Error(`expected ok, got ${res.reason}`);
    expect(res.entities).toHaveLength(4);
    expect(res.entities.every((e) => e.type === "Line")).toBe(true);
    expectClosed(res.entities, true);
    expect(corners(res.entities)).toEqual([
      [3, 3],
      [3, 47],
      [97, 3],
      [97, 47],
    ]);
  });

  it("−3 is EXACTLY the outset rectangle", () => {
    const res = offsetChain(chain, -3, MIN);
    if (!res.ok) throw new Error(`expected ok, got ${res.reason}`);
    expectClosed(res.entities, true);
    expect(corners(res.entities)).toEqual([
      [-3, -3],
      [-3, 53],
      [103, -3],
      [103, 53],
    ]);
  });

  it("refuses the distance at which a wall would vanish", () => {
    // Half-height is 25: at d = 25 the top and bottom offsets coincide, and the
    // side walls collapse to nothing.
    expect(offsetChain(chain, 25, { minSize: 1e-3 })).toEqual({ ok: false, reason: "segmentVanished" });
  });

  it("refuses an OVER-offset that folds the chain inside out", () => {
    // d = 30 > half-height: every join still meets, so the naive result is a
    // valid-looking 40×10 rectangle whose walls run BACKWARDS. Caught by the
    // per-segment orientation check, not by a length test.
    expect(offsetChain(chain, 30, { minSize: 1e-3 })).toEqual({ ok: false, reason: "segmentVanished" });
    expect(offsetChain(chain, 24.9, { minSize: 1e-3 }).ok).toBe(true);
  });

  it("refuses a zero distance", () => {
    expect(offsetChain(chain, 0, MIN)).toEqual({ ok: false, reason: "badDistance" });
  });
});

describe("offsetChain — open chains stay open", () => {
  it("offsets an L, welding only the interior join and keeping square free ends", () => {
    const l = [line("a", [0, 0], [100, 0]), line("b", [100, 0], [100, 50])];
    const res = offsetChain(buildChain(l[0], l, 1e-6)!, 3, MIN);
    if (!res.ok) throw new Error("expected ok");
    expect(res.entities).toHaveLength(2);
    expectClosed(res.entities, false);
    // Free ends are the plain perpendicular offsets; the shared corner extended.
    expect(ends(res.entities[0])[0]).toEqual([0, 3]);
    expect(ends(res.entities[0])[1]).toEqual([97, 3]);
    expect(ends(res.entities[1])[0]).toEqual([97, 3]);
    expect(ends(res.entities[1])[1]).toEqual([97, 50]);
  });
});

describe("offsetChain — arcs offset concentrically", () => {
  // CCW quarter: (10,0) → (0,10) about the origin.
  const quarter = arc("a", [0, 0], 10, [10, 0], [0, 10]);
  const chain = buildChain(quarter, [quarter], 1e-6)!;

  it("+d shrinks a CCW arc (its left side is INWARD): r − d", () => {
    const res = offsetChain(chain, 3, MIN);
    if (!res.ok) throw new Error("expected ok");
    expect(res.entities[0]).toMatchObject({ type: "Arc", radius: 7 });
    expect(ends(res.entities[0])[0][0]).toBeCloseTo(7, 12);
    expect(ends(res.entities[0])[1][1]).toBeCloseTo(7, 12);
    expect(res.entities[0].center).toEqual({ x: 0, y: 0 }); // concentric
  });

  it("−d grows it: r + d", () => {
    const res = offsetChain(chain, -3, MIN);
    if (!res.ok) throw new Error("expected ok");
    expect(res.entities[0]).toMatchObject({ type: "Arc", radius: 13 });
  });

  it("a REVERSED (CW-traversed) arc flips which sign shrinks it", () => {
    // Seeded from a line that meets the arc's END, so the arc is walked backwards.
    const lead = line("l", [0, 40], [0, 10]);
    const chain2 = buildChain(lead, [lead, quarter], 1e-6)!;
    expect(chain2.members.map((m) => [m.entity.id, m.reversed])).toEqual([
      ["l", false],
      ["a", true],
    ]);
    const res = offsetChain(chain2, 3, MIN);
    if (!res.ok) throw new Error("expected ok");
    // Travelling CW, the left side is OUTWARD ⇒ r + d.
    expect(res.entities[1]).toMatchObject({ type: "Arc", radius: 13 });
  });

  it("refuses an inward offset that collapses the arc, NAMING the maximum", () => {
    expect(offsetChain(chain, 10, { minSize: 1e-3 })).toEqual({
      ok: false,
      reason: "arcCollapsed",
      maxDistance: 10,
    });
    expect(offsetChain(chain, 9.9999, { minSize: 1e-3 })).toEqual({
      ok: false,
      reason: "arcCollapsed",
      maxDistance: 10,
    });
    expect(offsetChain(chain, 9.9, { minSize: 1e-3 }).ok).toBe(true);
  });

  it("offsets a circle concentrically with no joins", () => {
    const c = circle("c", [4, 5], 10);
    const res = offsetChain(buildChain(c, [c], 1e-6)!, -3, MIN);
    if (!res.ok) throw new Error("expected ok");
    expect(res.entities).toEqual([{ type: "Circle", center: { x: 4, y: 5 }, radius: 13 }]);
  });
});

describe("offsetChain — a tangent line↔arc join stays tangent (no gap, no kink)", () => {
  // Line (0,0)→(10,0) running into a CCW quarter arc about (10,10) that starts
  // at (10,0) with tangent (1,0) — i.e. a genuinely tangent join.
  const l = line("l", [0, 0], [10, 0]);
  const a = arc("a", [10, 10], 10, [10, 0], [20, 10]);

  it("welds both offsets at the single tangency point", () => {
    const chain = buildChain(l, [l, a], 1e-6)!;
    expect(chain.members.map((m) => [m.entity.id, m.reversed])).toEqual([
      ["l", false],
      ["a", false],
    ]);
    const res = offsetChain(chain, 3, MIN);
    if (!res.ok) throw new Error("expected ok");
    expectClosed(res.entities, false);
    expect(res.entities[1]).toMatchObject({ type: "Arc", radius: 7 });
    // The tangency is at (10, 3) for BOTH the offset line and the offset arc.
    expect(ends(res.entities[0])[1][0]).toBeCloseTo(10, 9);
    expect(ends(res.entities[0])[1][1]).toBeCloseTo(3, 9);
  });
});
