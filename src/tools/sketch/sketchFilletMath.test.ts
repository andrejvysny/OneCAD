/*
 * sketchFilletMath — pure data tests for the 2D corner fillet (WP-C T2b).
 *
 * Every expected number is derived from the closed form in the module header
 * (d = r/tan(θ/2), |C−P| = r/sin(θ/2)) and INDEPENDENTLY cross-checked by the
 * two invariants a fillet must satisfy no matter how it was computed:
 *   - |C − T| == r for both tangent points, and
 *   - (C − T) ⟂ the line T sits on.
 * `expectTangency` asserts exactly that, so a sign/branch slip cannot pass by
 * matching a hand-typed constant.
 */
import { describe, it, expect } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import { filletGeometry, findFilletCorner, sharedCorner, type FilletGeometry } from "./sketchFilletMath";

type XY = [number, number];

const line = (id: string, p0: XY, p1: XY, construction?: boolean): SketchEntity => ({
  id,
  type: "Line",
  p0,
  p1,
  ...(construction ? { construction: true } : {}),
});

const TOL = { tol: 1e-6 };

/** |C−T| == r and (C−T) ⟂ (far − P) for BOTH legs — the definition of tangency. */
function expectTangency(geom: FilletGeometry, u: XY, v: XY): void {
  for (const [t, dir] of [
    [geom.tangentA, u],
    [geom.tangentB, v],
  ] as [XY, XY][]) {
    const rx = geom.center[0] - t[0];
    const ry = geom.center[1] - t[1];
    expect(Math.hypot(rx, ry)).toBeCloseTo(geom.radius, 9);
    expect(rx * dir[0] + ry * dir[1]).toBeCloseTo(0, 9); // radius ⟂ leg
  }
}

describe("sharedCorner", () => {
  it("finds the meeting endpoints of an L (Start↔Start)", () => {
    const c = sharedCorner(line("a", [0, 0], [100, 0]), line("b", [0, 0], [0, 100]), 1e-6);
    expect(c).toEqual({ aEnd: "Start", bEnd: "Start", corner: [0, 0] });
  });

  it("finds an End↔Start weld", () => {
    const c = sharedCorner(line("a", [100, 0], [0, 0]), line("b", [0, 0], [0, 100]), 1e-6);
    expect(c).toEqual({ aEnd: "End", bEnd: "Start", corner: [0, 0] });
  });

  it("returns null for lines that do not meet", () => {
    expect(sharedCorner(line("a", [0, 0], [100, 0]), line("b", [5, 5], [5, 90]), 1e-6)).toBeNull();
  });

  it("returns null for a non-line (an arc is never a fillet leg in V1)", () => {
    const arc: SketchEntity = { id: "c", type: "Arc", center: [0, 0], radius: 10, start: [10, 0], end: [0, 10] };
    expect(sharedCorner(line("a", [0, 0], [100, 0]), arc, 1e-6)).toBeNull();
  });
});

describe("filletGeometry — right angle (the exact r/tan45 = r case)", () => {
  const a = line("a", [0, 0], [100, 0]); // u = +x
  const b = line("b", [0, 0], [0, 100]); // v = +y

  it("places the tangent points at exactly r along each leg", () => {
    const res = filletGeometry(a, b, 10, TOL);
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    // θ = 90° ⇒ d = r/tan(45°) = r = 10. (`Math.tan(π/4)` is 1 − 1 ulp, so the
    // tangent distance lands 2e-15 off — matched to 12 decimals, not by identity.)
    expect(res.geom.tangentA[0]).toBeCloseTo(10, 12);
    expect(res.geom.tangentA[1]).toBeCloseTo(0, 12);
    expect(res.geom.tangentB[0]).toBeCloseTo(0, 12);
    expect(res.geom.tangentB[1]).toBeCloseTo(10, 12);
    // |C−P| = r/sin(45°) = 10√2 along the (1,1)/√2 bisector ⇒ C = (10, 10).
    expect(res.geom.center[0]).toBeCloseTo(10, 12);
    expect(res.geom.center[1]).toBeCloseTo(10, 12);
    expect(res.geom.sweep).toBeCloseTo(Math.PI / 2, 12);
    expectTangency(res.geom, [1, 0], [0, 1]);
  });

  it("trims both legs back to their tangent points, keeping the far ends", () => {
    const res = filletGeometry(a, b, 10, TOL);
    if (!res.ok) throw new Error("expected ok");
    const [ta, tb, arc] = res.geom.drafts;
    expect(ta.type).toBe("Line");
    expect(ta.p0!.x).toBeCloseTo(10, 12);
    expect(ta.p0!.y).toBeCloseTo(0, 12);
    expect(ta.p1).toEqual({ x: 100, y: 0 }); // the FAR end is carried verbatim
    expect(tb!.p0!.x).toBeCloseTo(0, 12);
    expect(tb!.p0!.y).toBeCloseTo(10, 12);
    expect(tb!.p1).toEqual({ x: 0, y: 100 });
    expect(arc).toMatchObject({ type: "Arc", radius: 10 });
  });

  it("winds CCW the SHORT way: u×v > 0 ⇒ arc runs T_b → T_a", () => {
    const res = filletGeometry(a, b, 10, TOL);
    if (!res.ok) throw new Error("expected ok");
    const arc = res.geom.drafts[2];
    // cross(+x, +y) = +1 ⇒ start at T_b = (0,10), end at T_a = (10,0).
    expect(arc.start!.x).toBeCloseTo(0, 9);
    expect(arc.start!.y).toBeCloseTo(10, 9);
    expect(arc.end!.x).toBeCloseTo(10, 9);
    expect(arc.end!.y).toBeCloseTo(0, 9);
    expect(res.geom.arcEndA).toBe("End");
    expect(res.geom.arcEndB).toBe("Start");
  });

  it("pins the OPPOSITE orientation too: u×v < 0 ⇒ arc runs T_a → T_b", () => {
    // Same corner, legs swapped: u = +y, v = +x ⇒ cross = −1.
    const res = filletGeometry(line("a", [0, 0], [0, 100]), line("b", [0, 0], [100, 0]), 10, TOL);
    if (!res.ok) throw new Error("expected ok");
    const arc = res.geom.drafts[2];
    // The SAME arc on screen — start (0,10) → end (10,0) — reached the other way.
    expect(arc.start!.x).toBeCloseTo(0, 9);
    expect(arc.start!.y).toBeCloseTo(10, 9);
    expect(arc.end!.x).toBeCloseTo(10, 9);
    expect(arc.end!.y).toBeCloseTo(0, 9);
    expect(res.geom.arcEndA).toBe("Start");
    expect(res.geom.arcEndB).toBe("End");
  });

  it("handles an End↔End corner (both legs pointing INTO the corner)", () => {
    const res = filletGeometry(line("a", [100, 0], [0, 0]), line("b", [0, 100], [0, 0]), 10, TOL);
    if (!res.ok) throw new Error("expected ok");
    expect(res.geom.aEnd).toBe("End");
    expect(res.geom.bEnd).toBe("End");
    const [ta, tb] = res.geom.drafts;
    expect(ta.p0).toEqual({ x: 100, y: 0 });
    expect(ta.p1!.x).toBeCloseTo(10, 12);
    expect(tb!.p1!.y).toBeCloseTo(10, 12);
    expectTangency(res.geom, [1, 0], [0, 1]);
  });
});

describe("filletGeometry — acute and obtuse corners", () => {
  const R = 10;

  it("60°: d = r·√3 and |C−P| = 2r", () => {
    const v: XY = [Math.cos(Math.PI / 3) * 100, Math.sin(Math.PI / 3) * 100];
    const res = filletGeometry(line("a", [0, 0], [100, 0]), line("b", [0, 0], v), R, TOL);
    if (!res.ok) throw new Error("expected ok");
    const d = R / Math.tan(Math.PI / 6); // 17.320508…
    expect(d).toBeCloseTo(17.320508075688775, 9);
    expect(res.geom.tangentA[0]).toBeCloseTo(d, 9);
    expect(res.geom.tangentA[1]).toBeCloseTo(0, 9);
    // |C−P| = r/sin(30°) = 2r = 20, on the 30° bisector ⇒ (17.3205…, 10).
    expect(Math.hypot(...res.geom.center)).toBeCloseTo(20, 9);
    expect(res.geom.center[1]).toBeCloseTo(10, 9);
    expect(res.geom.sweep).toBeCloseTo(Math.PI - Math.PI / 3, 9);
    expectTangency(res.geom, [1, 0], [Math.cos(Math.PI / 3), Math.sin(Math.PI / 3)]);
  });

  it("120°: d = r/√3 and |C−P| = 2r/√3", () => {
    const v: XY = [Math.cos((2 * Math.PI) / 3) * 100, Math.sin((2 * Math.PI) / 3) * 100];
    const res = filletGeometry(line("a", [0, 0], [100, 0]), line("b", [0, 0], v), R, TOL);
    if (!res.ok) throw new Error("expected ok");
    const d = R / Math.tan(Math.PI / 3); // 5.7735…
    expect(d).toBeCloseTo(5.773502691896258, 9);
    expect(res.geom.tangentA[0]).toBeCloseTo(d, 9);
    expect(Math.hypot(...res.geom.center)).toBeCloseTo(R / Math.sin(Math.PI / 3), 9);
    expect(res.geom.center[1]).toBeCloseTo(10, 9); // the centre is always r off each leg
    expect(res.geom.sweep).toBeCloseTo(Math.PI / 3, 9);
    expectTangency(res.geom, [1, 0], [Math.cos((2 * Math.PI) / 3), Math.sin((2 * Math.PI) / 3)]);
  });
});

describe("filletGeometry — refusals (loud, never a guess)", () => {
  it("refuses lines that share no endpoint", () => {
    const res = filletGeometry(line("a", [0, 0], [100, 0]), line("b", [5, 5], [5, 90]), 10, TOL);
    expect(res).toEqual({ ok: false, reason: "notACorner" });
  });

  it("refuses a straight-through (θ = π) pair", () => {
    const res = filletGeometry(line("a", [0, 0], [100, 0]), line("b", [0, 0], [-100, 0]), 10, TOL);
    expect(res).toEqual({ ok: false, reason: "collinear" });
  });

  it("refuses a doubled-back (θ = 0) pair", () => {
    const res = filletGeometry(line("a", [0, 0], [100, 0]), line("b", [0, 0], [50, 0]), 10, TOL);
    expect(res).toEqual({ ok: false, reason: "collinear" });
  });

  it("refuses r ≤ 0", () => {
    const res = filletGeometry(line("a", [0, 0], [100, 0]), line("b", [0, 0], [0, 100]), 0, TOL);
    expect(res).toEqual({ ok: false, reason: "badRadius" });
  });

  it("refuses a radius the corner cannot fit, NAMING the maximum", () => {
    // 10 × 10 right-angle corner: d = r, so the largest usable r is min(leg)·tan45 = 10.
    const a = line("a", [0, 0], [10, 0]);
    const b = line("b", [0, 0], [0, 10]);
    const res = filletGeometry(a, b, 20, TOL);
    if (res.ok || res.reason !== "radiusTooLarge") throw new Error("expected radiusTooLarge");
    expect(res.maxRadius).toBeCloseTo(10, 12);
    // Exactly at the max the tangent point would land on the far endpoint — refused.
    expect(filletGeometry(a, b, 10, TOL).ok).toBe(false);
    expect(filletGeometry(a, b, 9.99, TOL).ok).toBe(true);
  });

  it("names a SHORTER maximum on a 60° corner (tan(θ/2) < 1)", () => {
    const v: XY = [Math.cos(Math.PI / 3) * 10, Math.sin(Math.PI / 3) * 10];
    const res = filletGeometry(line("a", [0, 0], [10, 0]), line("b", [0, 0], v), 10, TOL);
    if (res.ok || res.reason !== "radiusTooLarge") throw new Error("expected radiusTooLarge");
    expect(res.maxRadius).toBeCloseTo(10 * Math.tan(Math.PI / 6), 9); // 5.7735…
  });

  it("refuses a zero-length leg", () => {
    const res = filletGeometry(line("a", [0, 0], [0, 0]), line("b", [0, 0], [0, 100]), 10, TOL);
    expect(res).toEqual({ ok: false, reason: "degenerate" });
  });
});

describe("filletGeometry — construction inheritance", () => {
  it("keeps each leg's own flag and makes the arc REAL unless both legs are construction", () => {
    const res = filletGeometry(line("a", [0, 0], [100, 0], true), line("b", [0, 0], [0, 100]), 10, TOL);
    if (!res.ok) throw new Error("expected ok");
    expect(res.geom.drafts[0].construction).toBe(true);
    expect(res.geom.drafts[1].construction).toBeUndefined();
    expect(res.geom.drafts[2].construction).toBeUndefined();
  });

  it("keeps the arc construction when BOTH legs are", () => {
    const res = filletGeometry(line("a", [0, 0], [100, 0], true), line("b", [0, 0], [0, 100], true), 10, TOL);
    if (!res.ok) throw new Error("expected ok");
    expect(res.geom.drafts[2].construction).toBe(true);
  });
});

describe("findFilletCorner", () => {
  // A closed rectangle: four corners, so the cursor decides which one is meant.
  const rect: SketchEntity[] = [
    line("l1", [0, 0], [100, 0]),
    line("l2", [100, 0], [100, 50]),
    line("l3", [100, 50], [0, 50]),
    line("l4", [0, 50], [0, 0]),
  ];
  const opts = { reach: 8, weldTol: 1e-6 };

  it("picks the corner nearest the cursor", () => {
    const near = findFilletCorner({ x: 2, y: 2 }, rect, opts);
    expect(near?.corner).toEqual([0, 0]);
    expect([near?.a.id, near?.b.id].sort()).toEqual(["l1", "l4"]);

    const far = findFilletCorner({ x: 99, y: 49 }, rect, opts);
    expect(far?.corner).toEqual([100, 50]);
    expect([far?.a.id, far?.b.id].sort()).toEqual(["l2", "l3"]);
  });

  it("returns null when no corner is within reach", () => {
    expect(findFilletCorner({ x: 50, y: 25 }, rect, opts)).toBeNull();
  });

  it("ignores a near-miss pair: `weldTol` is not the pick reach", () => {
    // Endpoints 2 units apart — well inside the 8-unit cursor reach, but NOT a
    // corner. Welding it would author geometry where the user sees a gap.
    const apart = [line("a", [0, 0], [100, 0]), line("b", [0, 2], [0, 100])];
    expect(findFilletCorner({ x: 0, y: 1 }, apart, opts)).toBeNull();
    expect(findFilletCorner({ x: 0, y: 1 }, apart, { reach: 8, weldTol: 3 })).not.toBeNull();
  });
});
