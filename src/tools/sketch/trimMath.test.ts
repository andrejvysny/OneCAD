/*
 * trimMath (PURE): parametric trim over line / arc / circle targets. Exercises the
 * crossing collection (extent gating, dedup, endpoint exclusion, tangency), the
 * removal-interval location (clicked side, ties, 3+ crossings), piece filtering
 * (minSize discard), and per-kind piece geometry (line sub-lines, arc sub-arcs
 * preserving CCW incl. a sweep that crosses 0 rad, circle → single complement arc).
 * A few intersection points are cross-checked against snapEngine's point-returning
 * helpers so the two implementations are pinned to agree.
 */
import { describe, it, expect } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import type { DraftEntity } from "./toolMachine";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { trimPieces, trimPreview, entityToDraft } from "./trimMath";
import {
  segCircleIntersections,
  circleCircleIntersections,
  arcContainsAngle,
} from "./snapEngine";

const SMALL = 1e-6;
const near = (p: Point2 | undefined, x: number, y: number): void => {
  expect(p).toBeDefined();
  expect(p!.x).toBeCloseTo(x, 6);
  expect(p!.y).toBeCloseTo(y, 6);
};
const vline = (id: string, x: number, y0 = -50, y1 = 50): SketchEntity => ({
  id,
  type: "Line",
  p0: [x, y0],
  p1: [x, y1],
});
const click = (x: number, y: number): Point2 => ({ x, y });

// ── Line target ────────────────────────────────────────────────────────────────
describe("trimPieces — line", () => {
  const target: SketchEntity = { id: "t", type: "Line", p0: [0, 0], p1: [30, 0] };

  it("crossed by 2 lines → middle removed, 2 pieces", () => {
    const r = trimPieces(target, click(15, 0), [vline("a", 10), vline("b", 20)], { minSize: SMALL });
    expect(r).not.toBeNull();
    expect(r!.pieces).toHaveLength(2);
    near(r!.pieces[0].p0, 0, 0);
    near(r!.pieces[0].p1, 10, 0);
    near(r!.pieces[1].p0, 20, 0);
    near(r!.pieces[1].p1, 30, 0);
    // doomed = the removed middle span.
    near(r!.doomed.p0, 10, 0);
    near(r!.doomed.p1, 20, 0);
  });

  it("crossed by 1 line → the CLICKED side is removed (1 piece each way)", () => {
    const left = trimPieces(target, click(5, 0), [vline("a", 10)], { minSize: SMALL });
    expect(left!.pieces).toHaveLength(1);
    near(left!.pieces[0].p0, 10, 0); // left span gone, right survives
    near(left!.pieces[0].p1, 30, 0);

    const right = trimPieces(target, click(20, 0), [vline("a", 10)], { minSize: SMALL });
    expect(right!.pieces).toHaveLength(1);
    near(right!.pieces[0].p0, 0, 0); // right span gone, left survives
    near(right!.pieces[0].p1, 10, 0);
  });

  it("no crossings → null (empty and parallel others)", () => {
    expect(trimPieces(target, click(15, 0), [], { minSize: SMALL })).toBeNull();
    const parallel: SketchEntity = { id: "p", type: "Line", p0: [0, 5], p1: [30, 5] };
    expect(trimPieces(target, click(15, 0), [parallel], { minSize: SMALL })).toBeNull();
  });

  it("a crossing exactly at the target endpoint is excluded (⇒ null)", () => {
    // A vertical line through the start endpoint (t≈0) must not count as a cut.
    expect(trimPieces(target, click(15, 0), [vline("end0", 0)], { minSize: SMALL })).toBeNull();
    expect(trimPieces(target, click(15, 0), [vline("end1", 30)], { minSize: SMALL })).toBeNull();
  });

  it("two entities through the SAME point dedupe to one crossing", () => {
    // Both cross at x=15; a third at x=25. Deduped ⇒ crossings {0.5, 0.833}.
    const others = [vline("a", 15, -50, 50), vline("b", 15, -3, 3), vline("c", 25)];
    const r = trimPieces(target, click(20, 0), others, { minSize: SMALL });
    // Click at t=0.667 between 0.5 and 0.833 ⇒ middle removed, exactly 2 pieces
    // (no spurious zero-length fragment from the duplicated crossing).
    expect(r!.pieces).toHaveLength(2);
    near(r!.pieces[0].p1, 15, 0);
    near(r!.pieces[1].p0, 25, 0);
  });

  it("tangent circle-line counts as one crossing", () => {
    // Circle tangent to the target line at (0,0)… shift so the tangent is interior.
    const circle: SketchEntity = { id: "c", type: "Circle", center: [15, 10], radius: 10 };
    // Cross-check: the tangent touch is a single point at (15,0).
    const pts = segCircleIntersections([0, 0], [30, 0], [15, 10], 10);
    expect(pts).toHaveLength(1);
    near(pts[0], 15, 0);
    const r = trimPieces(target, click(5, 0), [circle], { minSize: SMALL });
    expect(r!.pieces).toHaveLength(1);
    near(r!.pieces[0].p0, 15, 0); // left removed, right survives
    near(r!.pieces[0].p1, 30, 0);
  });

  it("a surviving piece shorter than minSize is discarded (doomed swallows it)", () => {
    // Crossings at x=1 (t≈0.033) and x=20 (t≈0.667); click mid ⇒ remove [0.033,0.667].
    const r = trimPieces(target, click(10, 0), [vline("a", 1), vline("b", 20)], { minSize: 4 });
    // Left piece length 1 < 4 → discarded; only the right piece survives.
    expect(r!.pieces).toHaveLength(1);
    near(r!.pieces[0].p0, 20, 0);
    near(r!.pieces[0].p1, 30, 0);
    // The doomed span extends to 0 (the discarded left piece merged in).
    near(r!.doomed.p0, 0, 0);
    near(r!.doomed.p1, 20, 0);
  });

  it("3+ crossings pick the interval the click falls in", () => {
    const t2: SketchEntity = { id: "t", type: "Line", p0: [0, 0], p1: [40, 0] };
    const others = [vline("a", 10), vline("b", 20), vline("c", 30)];
    const r = trimPieces(t2, click(25, 0), others, { minSize: SMALL }); // t=0.625 ∈ [0.5,0.75]
    expect(r!.pieces).toHaveLength(2);
    near(r!.pieces[0].p0, 0, 0);
    near(r!.pieces[0].p1, 20, 0);
    near(r!.pieces[1].p0, 30, 0);
    near(r!.pieces[1].p1, 40, 0);
    near(r!.doomed.p0, 20, 0);
    near(r!.doomed.p1, 30, 0);
  });

  it("a click within ε of a crossing picks the exact cursor side", () => {
    const other = [vline("a", 15)]; // crossing t=0.5
    const l = trimPieces(target, click(15 - 1e-4, 0), other, { minSize: SMALL });
    near(l!.pieces[0].p0, 15, 0); // just-left ⇒ left removed, right survives
    near(l!.pieces[0].p1, 30, 0);
    const r = trimPieces(target, click(15 + 1e-4, 0), other, { minSize: SMALL });
    near(r!.pieces[0].p0, 0, 0); // just-right ⇒ right removed, left survives
    near(r!.pieces[0].p1, 15, 0);
  });
});

// ── Circle target ──────────────────────────────────────────────────────────────
describe("trimPieces — circle", () => {
  const circle: SketchEntity = { id: "t", type: "Circle", center: [0, 0], radius: 10 };

  it("with 2 crossings → ONE complementary arc (CCW, correct span)", () => {
    // A vertical secant x=0 crosses the circle at (0,±10) [90° and 270°].
    const secant: SketchEntity = { id: "s", type: "Line", p0: [0, -20], p1: [0, 20] };
    // Cross-check the two crossing points against snapEngine.
    const pts = segCircleIntersections([0, -20], [0, 20], [0, 0], 10);
    expect(pts).toHaveLength(2);

    // Click the RIGHT side (10,0) → remove the right half through 0°, keep the left.
    const r = trimPieces(circle, click(10, 0), [secant], { minSize: SMALL });
    expect(r!.pieces).toHaveLength(1);
    const arc = r!.pieces[0] as DraftEntity;
    expect(arc.type).toBe("Arc");
    near(arc.center, 0, 0);
    expect(arc.radius).toBe(10);
    // Remaining arc: CCW from (0,10) to (0,-10) through 180° (the LEFT side).
    near(arc.start, 0, 10);
    near(arc.end, 0, -10);
    const c: [number, number] = [0, 0];
    const s: [number, number] = [arc.start!.x, arc.start!.y];
    const e: [number, number] = [arc.end!.x, arc.end!.y];
    expect(arcContainsAngle(c, s, e, Math.PI)).toBe(true); // 180° → (-10,0) kept
    expect(arcContainsAngle(c, s, e, 0)).toBe(false); // 0° → (10,0) removed
    // doomed = the removed right span through 0°.
    const dS: [number, number] = [r!.doomed.start!.x, r!.doomed.start!.y];
    const dE: [number, number] = [r!.doomed.end!.x, r!.doomed.end!.y];
    expect(arcContainsAngle(c, dS, dE, 0)).toBe(true);
  });

  it("with only 1 crossing (tangent) → null", () => {
    // A line tangent at the top (0,10).
    const tangent: SketchEntity = { id: "s", type: "Line", p0: [-20, 10], p1: [20, 10] };
    const pts = segCircleIntersections([-20, 10], [20, 10], [0, 0], 10);
    expect(pts).toHaveLength(1);
    expect(trimPieces(circle, click(10, 0), [tangent], { minSize: SMALL })).toBeNull();
  });

  it("cross-check: two overlapping circles give the crossing points", () => {
    const other: SketchEntity = { id: "o", type: "Circle", center: [10, 0], radius: 10 };
    const pts = circleCircleIntersections([0, 0], 10, [10, 0], 10);
    expect(pts).toHaveLength(2);
    const r = trimPieces(circle, click(-10, 0), [other], { minSize: SMALL });
    expect(r!.pieces).toHaveLength(1); // one surviving arc
    expect(r!.pieces[0].type).toBe("Arc");
  });
});

// ── Arc target ─────────────────────────────────────────────────────────────────
describe("trimPieces — arc", () => {
  const S32 = Math.sqrt(3) / 2; // cos30 = 0.8660254…

  it("split into 2 sub-arcs (CCW preserved)", () => {
    // Upper semicircle: center (0,0) r=10, 0°→180° CCW.
    const arc: SketchEntity = { id: "t", type: "Arc", center: [0, 0], radius: 10, start: [10, 0], end: [-10, 0] };
    const x5: SketchEntity = { id: "a", type: "Line", p0: [5, -20], p1: [5, 20] }; // hits 60°
    const xm5: SketchEntity = { id: "b", type: "Line", p0: [-5, -20], p1: [-5, 20] }; // hits 120°
    const r = trimPieces(arc, click(0, 10), [x5, xm5], { minSize: SMALL }); // click top (90°)
    expect(r!.pieces).toHaveLength(2);
    expect(r!.pieces.every((p) => p.type === "Arc")).toBe(true);
    near(r!.pieces[0].start, 10, 0); // 0°
    near(r!.pieces[0].end, 5, 10 * S32); // 60°
    near(r!.pieces[1].start, -5, 10 * S32); // 120°
    near(r!.pieces[1].end, -10, 0); // 180°
    near(r!.doomed.start, 5, 10 * S32); // removed 60°→120°
    near(r!.doomed.end, -5, 10 * S32);
  });

  it("handles a sweep that crosses 0 rad", () => {
    // Arc 300°→60° CCW (sweep 120°, through 0°). One vertical secant x=10·cos30
    // hits it at 330° (s=0.25) and 30° (s=0.75).
    const arc: SketchEntity = {
      id: "t",
      type: "Arc",
      center: [0, 0],
      radius: 10,
      start: [10 * 0.5, -10 * S32], // 300° = (5, -8.66)
      end: [10 * 0.5, 10 * S32], // 60°  = (5,  8.66)
    };
    const secant: SketchEntity = { id: "s", type: "Line", p0: [10 * S32, -20], p1: [10 * S32, 20] };
    const r = trimPieces(arc, click(10, 0), [secant], { minSize: SMALL }); // click 0° (10,0)
    expect(r!.pieces).toHaveLength(2);
    // piece0: 300°→330°, piece1: 30°→60° (neither crosses 0°).
    near(r!.pieces[0].start, 5, -10 * S32); // 300°
    near(r!.pieces[0].end, 10 * S32, -5); // 330°
    near(r!.pieces[1].start, 10 * S32, 5); // 30°
    near(r!.pieces[1].end, 5, 10 * S32); // 60°
    // doomed spans 330°→30°, which DOES cross 0°.
    const c: [number, number] = [0, 0];
    const dS: [number, number] = [r!.doomed.start!.x, r!.doomed.start!.y];
    const dE: [number, number] = [r!.doomed.end!.x, r!.doomed.end!.y];
    expect(arcContainsAngle(c, dS, dE, 0)).toBe(true);
  });

  it("no qualifying crossings → null", () => {
    const arc: SketchEntity = { id: "t", type: "Arc", center: [0, 0], radius: 10, start: [10, 0], end: [-10, 0] };
    // A line that only meets the LOWER (excluded) half of the circle.
    const low: SketchEntity = { id: "s", type: "Line", p0: [-20, -8], p1: [20, -8] };
    expect(trimPieces(arc, click(0, 10), [low], { minSize: SMALL })).toBeNull();
  });
});

// ── trimPreview + entityToDraft ─────────────────────────────────────────────────
describe("trimPreview / entityToDraft", () => {
  const target: SketchEntity = { id: "t", type: "Line", p0: [0, 0], p1: [30, 0] };

  it("returns the doomed piece when trimmable", () => {
    const doomed = trimPreview(target, click(15, 0), [vline("a", 10), vline("b", 20)], { minSize: SMALL });
    expect(doomed).not.toBeNull();
    near(doomed!.p0, 10, 0);
    near(doomed!.p1, 20, 0);
  });

  it("returns null (whole entity doomed) when there is nothing to trim", () => {
    expect(trimPreview(target, click(15, 0), [], { minSize: SMALL })).toBeNull();
  });

  it("entityToDraft mirrors an entity's fields as Point2 coords", () => {
    const d = entityToDraft(target);
    expect(d.type).toBe("Line");
    near(d.p0, 0, 0);
    near(d.p1, 30, 0);
    const circle: SketchEntity = { id: "c", type: "Circle", center: [1, 2], radius: 5, construction: true };
    const cd = entityToDraft(circle);
    expect(cd).toMatchObject({ type: "Circle", radius: 5, construction: true });
    near(cd.center, 1, 2);
  });
});

// ── W3 P3: an ellipse is never parametrically trimmed (whole-DELETE) ─────────
//
// Legacy parity: `OneCAD-CPP/src/core/sketch/tools/TrimTool.cpp:298-299` routes
// `EntityType::Ellipse` straight to `sketch.removeEntity(entityId)`. Here the
// signal is `trimPieces === null`, which the service's trim path already reads as
// "nothing to trim → whole-entity delete" (sketchService.trimEntityNow).

describe("trimPieces — Ellipse whole-delete fallback", () => {
  const ell: SketchEntity = {
    id: "el1",
    type: "Ellipse",
    center: [0, 0],
    majorR: 20,
    minorR: 5,
    rotation: 0,
  };

  it("returns null even when a line crosses it twice", () => {
    const crossing: SketchEntity = { id: "l1", type: "Line", p0: [0, -50], p1: [0, 50] };
    expect(trimPieces(ell, { x: 20, y: 0 }, [crossing], { minSize: 0.1 })).toBeNull();
    expect(trimPreview(ell, { x: 20, y: 0 }, [crossing], { minSize: 0.1 })).toBeNull();
  });

  it("an ellipse never cuts OTHER entities either (no ellipse×line crossings)", () => {
    const l: SketchEntity = { id: "l1", type: "Line", p0: [-50, 0], p1: [50, 0] };
    expect(trimPieces(l, { x: 0, y: 0 }, [ell], { minSize: 0.1 })).toBeNull();
  });

  it("entityToDraft carries the ellipse fields so the doomed ghost renders", () => {
    expect(entityToDraft(ell)).toEqual({
      type: "Ellipse",
      center: { x: 0, y: 0 },
      majorR: 20,
      minorR: 5,
      rotation: 0,
    });
  });
});
