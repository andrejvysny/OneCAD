/*
 * constraintAuthoring — PURE builder tests: `evaluateApplicability` output →
 * authored SketchConstraint (geometric) / Dimension seed (dimensional). Targets are
 * built through the REAL adapters (toConstraintTarget + evaluateApplicability) so
 * the fixtures can't drift from production.
 */
import { describe, it, expect } from "vitest";
import type { ConstraintPosition, SketchConstraintType, SketchEntity } from "@/ipc/types";
import { toConstraintTarget, type SketchConstraintTarget } from "./constraintTarget";
import { evaluateApplicability, type ApplicableConstraint } from "./constraintApplicability";
import { buildAppliedConstraint, buildAppliedDimension } from "./constraintAuthoring";

const entities: SketchEntity[] = [
  { id: "p1", type: "Point", p0: [0, 0] },
  { id: "p2", type: "Point", p0: [10, 0] },
  { id: "lh", type: "Line", p0: [0, 0], p1: [10, 0] }, // horizontal
  { id: "lv", type: "Line", p0: [0, 0], p1: [0, 10] }, // vertical (⟂ lh)
  { id: "axis", type: "Line", p0: [5, -10], p1: [5, 10] },
  { id: "c1", type: "Circle", center: [0, 0], radius: 5 },
  { id: "c2", type: "Circle", center: [20, 0], radius: 3 },
  { id: "arc1", type: "Arc", center: [40, 0], radius: 2, start: [42, 0], end: [40, 2] },
];

function T(entityId: string, point?: ConstraintPosition): SketchConstraintTarget {
  const t = toConstraintTarget({ entityId, point }, entities);
  if (!t) throw new Error(`fixture bug: ${entityId}`);
  return t;
}
function pick(targets: SketchConstraintTarget[], type: SketchConstraintType) {
  const a = evaluateApplicability(targets, entities).find((r) => r.type === type);
  if (!a) throw new Error(`applicability did not offer ${type}`);
  return a;
}

describe("buildAppliedConstraint — geometric", () => {
  it("Parallel (two lines) → entities in order, no positions", () => {
    expect(buildAppliedConstraint(pick([T("lh"), T("lv")], "Parallel"), "cX")).toEqual({
      id: "cX",
      type: "Parallel",
      entities: ["lh", "lv"],
    });
  });

  it("Perpendicular (two lines)", () => {
    expect(buildAppliedConstraint(pick([T("lh"), T("lv")], "Perpendicular"), "cX")).toMatchObject({
      type: "Perpendicular",
      entities: ["lh", "lv"],
    });
  });

  it("Horizontal (one line) → single entity ref", () => {
    expect(buildAppliedConstraint(pick([T("lh")], "Horizontal"), "cX")).toEqual({
      id: "cX",
      type: "Horizontal",
      entities: ["lh"],
    });
  });

  it("Coincident (two points) → entities + positions", () => {
    expect(buildAppliedConstraint(pick([T("p1"), T("p2")], "Coincident"), "cX")).toEqual({
      id: "cX",
      type: "Coincident",
      entities: ["p1", "p2"],
      positions: ["Start", "Start"],
    });
  });

  it("Fixed (one point) → entity + position", () => {
    expect(buildAppliedConstraint(pick([T("p1")], "Fixed"), "cX")).toEqual({
      id: "cX",
      type: "Fixed",
      entities: ["p1"],
      positions: ["Start"],
    });
  });

  it("Concentric (two circles)", () => {
    expect(buildAppliedConstraint(pick([T("c1"), T("c2")], "Concentric"), "cX")).toEqual({
      id: "cX",
      type: "Concentric",
      entities: ["c1", "c2"],
    });
  });

  it("OnCurve normalizes point-first even when the CURVE is selected first", () => {
    // Curve-first input ⇒ applicability targets [curve, point]; builder flips it.
    expect(buildAppliedConstraint(pick([T("lh"), T("p2")], "OnCurve"), "cX")).toEqual({
      id: "cX",
      type: "OnCurve",
      entities: ["p2", "lh"],
      positions: ["Start"],
    });
  });

  it("Symmetric (2 points + axis line) → [point1, point2, axis] + two positions", () => {
    expect(buildAppliedConstraint(pick([T("p1"), T("p2"), T("axis")], "Symmetric"), "cX")).toEqual({
      id: "cX",
      type: "Symmetric",
      entities: ["p1", "p2", "axis"],
      positions: ["Start", "Start"],
    });
  });

  it("returns null for a dimensional applicable (routed through the chip)", () => {
    expect(buildAppliedConstraint(pick([T("c1")], "Radius"), "cX")).toBeNull();
  });

  it("Tangent (line + circle) → entities in order, no positions", () => {
    expect(buildAppliedConstraint(pick([T("lh"), T("c1")], "Tangent"), "cX")).toEqual({
      id: "cX",
      type: "Tangent",
      entities: ["lh", "c1"],
    });
  });

  it("Tangent (circle + arc)", () => {
    expect(buildAppliedConstraint(pick([T("c1"), T("arc1")], "Tangent"), "cX")).toEqual({
      id: "cX",
      type: "Tangent",
      entities: ["c1", "arc1"],
    });
  });

  it("Equal (two lines) → entities in order, no positions", () => {
    expect(buildAppliedConstraint(pick([T("lh"), T("lv")], "Equal"), "cX")).toEqual({
      id: "cX",
      type: "Equal",
      entities: ["lh", "lv"],
    });
  });

  it("Equal (two circles)", () => {
    expect(buildAppliedConstraint(pick([T("c1"), T("c2")], "Equal"), "cX")).toEqual({
      id: "cX",
      type: "Equal",
      entities: ["c1", "c2"],
    });
  });

  it("Midpoint (free point + line) → [point, line] entities + point's position", () => {
    expect(buildAppliedConstraint(pick([T("p1"), T("axis")], "Midpoint"), "cX")).toEqual({
      id: "cX",
      type: "Midpoint",
      entities: ["p1", "axis"],
      positions: ["Start"],
    });
  });

  it("Midpoint builder defensively normalizes point-first even if targets ever arrived line-first", () => {
    // evaluateApplicability already orders Midpoint targets [point, line] (unlike
    // OnCurve), so this constructs the reversed shape directly to prove the
    // builder's OWN `find`-based normalization (mirrors the OnCurve case) rather
    // than merely re-testing evaluateApplicability's ordering.
    const reversed: ApplicableConstraint = { type: "Midpoint", targets: [T("axis"), T("p2")], dimensional: false };
    expect(buildAppliedConstraint(reversed, "cX")).toEqual({
      id: "cX",
      type: "Midpoint",
      entities: ["p2", "axis"],
      positions: ["Start"],
    });
  });

  it("Midpoint (circle Center as point + a line)", () => {
    expect(buildAppliedConstraint(pick([T("c1", "Center"), T("axis")], "Midpoint"), "cX")).toEqual({
      id: "cX",
      type: "Midpoint",
      entities: ["c1", "axis"],
      positions: ["Center"],
    });
  });
});

describe("buildAppliedDimension — dimensional seed + deferred build", () => {
  it("Radius seeds the circle radius; build authors a Radius constraint", () => {
    const dim = buildAppliedDimension(pick([T("c1")], "Radius"), entities)!;
    expect(dim.value).toBe(5);
    expect(dim.suffix).toBe("mm");
    expect(dim.anchor).toEqual({ x: 0, y: 0 });
    expect(dim.build(8, "cX")).toEqual({ id: "cX", type: "Radius", entities: ["c1"], value: 8 });
  });

  it("Diameter seeds 2·radius", () => {
    const dim = buildAppliedDimension(pick([T("c1")], "Diameter"), entities)!;
    expect(dim.value).toBe(10);
    expect(dim.build(12, "cX")).toMatchObject({ type: "Diameter", entities: ["c1"], value: 12 });
  });

  it("Angle seeds the between-lines degrees; build authors an Angle constraint", () => {
    const dim = buildAppliedDimension(pick([T("lh"), T("lv")], "Angle"), entities)!;
    expect(dim.value).toBeCloseTo(90, 6);
    expect(dim.suffix).toBe("°");
    expect(dim.build(45, "cX")).toEqual({ id: "cX", type: "Angle", entities: ["lh", "lv"], value: 45 });
  });

  it("Distance (point↔point) seeds the length + positions", () => {
    const dim = buildAppliedDimension(pick([T("p1"), T("p2")], "Distance"), entities)!;
    expect(dim.value).toBe(10);
    expect(dim.build(25, "cX")).toEqual({
      id: "cX",
      type: "Distance",
      entities: ["p1", "p2"],
      positions: ["Start", "Start"],
      value: 25,
    });
  });

  it("Distance (point↔line) is perpendicular + normalized point-first (line slot omits its position)", () => {
    const dim = buildAppliedDimension(pick([T("p2"), T("lv")], "Distance"), entities)!;
    expect(dim.value).toBeCloseTo(10, 6); // p2 (10,0) ⟂ to the x=0 line
    expect(dim.build(4, "cX")).toEqual({
      id: "cX",
      type: "Distance",
      entities: ["p2", "lv"],
      positions: ["Start"],
      value: 4,
    });
  });

  it("HorizontalDistance is signed (x2 − x1)", () => {
    const dim = buildAppliedDimension(pick([T("p1"), T("p2")], "HorizontalDistance"), entities)!;
    expect(dim.value).toBe(10);
    expect(dim.build(10, "cX")).toMatchObject({ type: "HorizontalDistance", positions: ["Start", "Start"] });
  });

  it("returns null for a geometric applicable", () => {
    expect(buildAppliedDimension(pick([T("lh"), T("lv")], "Parallel"), entities)).toBeNull();
  });
});
