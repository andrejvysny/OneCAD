import { describe, expect, it } from "vitest";
import type { SketchConstraint, SketchEntity } from "@/ipc/types";
import { hasAuthoredLength, layoutDimensionLines, unconstrainedEdgeLabel } from "./dimensionLineLayout";

// A 20×10 rectangle, CCW from origin.
const bottom: SketchEntity = { id: "bottom", type: "Line", p0: [0, 0], p1: [20, 0] };
const right: SketchEntity = { id: "right", type: "Line", p0: [20, 0], p1: [20, 10] };
const top: SketchEntity = { id: "top", type: "Line", p0: [20, 10], p1: [0, 10] };
const left: SketchEntity = { id: "left", type: "Line", p0: [0, 10], p1: [0, 0] };
const rect = [bottom, right, top, left];

const distanceOn = (entityId: string, value = 20): SketchConstraint => ({
  id: `dist-${entityId}`,
  type: "Distance",
  entities: [entityId],
  value,
});

describe("layoutDimensionLines", () => {
  it("emits nothing when no edge carries a length constraint", () => {
    expect(layoutDimensionLines(rect, [])).toEqual([]);
  });

  it("emits nothing for a constraint with no value", () => {
    const c: SketchConstraint = { id: "c1", type: "Distance", entities: ["top"] };
    expect(layoutDimensionLines(rect, [c])).toEqual([]);
  });

  it("skips a Radius/Angle constraint (not a length-of-edge type)", () => {
    const c: SketchConstraint = { id: "c1", type: "Radius", entities: ["top"], value: 5 };
    expect(layoutDimensionLines(rect, [c])).toEqual([]);
  });

  it("skips a constraint on a non-Line entity", () => {
    const circle: SketchEntity = { id: "c", type: "Circle", center: [0, 0], radius: 5 };
    const c: SketchConstraint = { id: "c1", type: "Distance", entities: ["c"], value: 5 };
    expect(layoutDimensionLines([circle], [c])).toEqual([]);
  });

  it("one dimension line per qualifying edge, deduped when an entity has multiple length constraints", () => {
    const dims = layoutDimensionLines(rect, [distanceOn("top"), distanceOn("top", 25)]);
    expect(dims).toHaveLength(1);
    expect(dims[0].id).toBe("dist-top"); // the FIRST constraint wins
  });

  it("HorizontalDistance and VerticalDistance both qualify", () => {
    const dims = layoutDimensionLines(rect, [
      { id: "h", type: "HorizontalDistance", entities: ["bottom"], value: 20 },
      { id: "v", type: "VerticalDistance", entities: ["right"], value: 10 },
    ]);
    expect(dims.map((d) => d.id).sort()).toEqual(["h", "v"]);
  });

  it("offsets the baseline AWAY from the sketch centroid", () => {
    // The rectangle's centroid sits at (10,5). The top edge (y=10) must offset
    // to y > 10, the bottom edge (y=0) must offset to y < 0 — never inward.
    const dims = layoutDimensionLines(rect, [distanceOn("top"), distanceOn("bottom")]);
    const topDim = dims.find((d) => d.id === "dist-top")!;
    const bottomDim = dims.find((d) => d.id === "dist-bottom")!;
    expect(topDim.baseline[0].y).toBeGreaterThan(10);
    expect(topDim.baseline[1].y).toBeGreaterThan(10);
    expect(bottomDim.baseline[0].y).toBeLessThan(0);
    expect(bottomDim.baseline[1].y).toBeLessThan(0);
  });

  it("ticks run from the real edge endpoints out to the baseline", () => {
    const dims = layoutDimensionLines(rect, [distanceOn("bottom")]);
    const [tick0, tick1] = dims[0].ticks;
    expect(tick0[0]).toEqual({ x: 0, y: 0 });
    expect(tick1[0]).toEqual({ x: 20, y: 0 });
    expect(tick0[1]).toEqual(dims[0].baseline[0]);
    expect(tick1[1]).toEqual(dims[0].baseline[1]);
  });

  it("skips a degenerate (zero-length) edge", () => {
    const dot: SketchEntity = { id: "dot", type: "Line", p0: [5, 5], p1: [5, 5] };
    expect(layoutDimensionLines([dot], [distanceOn("dot")])).toEqual([]);
  });

  it("draws two arrowhead strokes at each end of the baseline", () => {
    const dims = layoutDimensionLines(rect, [distanceOn("bottom")]);
    expect(dims[0].arrows).toHaveLength(4);
    // Every arrow stroke starts at one of the two baseline endpoints.
    const [base0, base1] = dims[0].baseline;
    for (const [start] of dims[0].arrows) {
      expect([base0, base1]).toContainEqual(start);
    }
  });

  it("an authored dimension line carries the constraint's value", () => {
    const dims = layoutDimensionLines(rect, [distanceOn("bottom", 20)]);
    expect(dims[0].id).toBe("dist-bottom");
    expect(dims[0].value).toBe(20);
  });
});

/*
 * The passive-measurement case (Sketcher UX cleanup, Track B3) — a merely
 * SELECTED, unconstrained edge. `layoutDimensionLines` no longer emits
 * anything for it (authored-only); `SelectionDimensionLabels` composes these
 * two pure helpers instead: `hasAuthoredLength` to dedupe against an already-
 * constrained edge, `unconstrainedEdgeLabel` for the lightweight midpoint +
 * value with NO offset/tick/arrow geometry.
 */
describe("hasAuthoredLength", () => {
  it("is true for an entity with a defined-value length constraint", () => {
    expect(hasAuthoredLength("bottom", [distanceOn("bottom")])).toBe(true);
  });

  it("is false when the constraint has no value, is the wrong type, or is on another entity", () => {
    expect(hasAuthoredLength("bottom", [{ id: "c1", type: "Distance", entities: ["bottom"] }])).toBe(
      false,
    );
    expect(hasAuthoredLength("bottom", [{ id: "c1", type: "Radius", entities: ["bottom"], value: 5 }])).toBe(
      false,
    );
    expect(hasAuthoredLength("bottom", [distanceOn("top")])).toBe(false);
  });

  it("HorizontalDistance and VerticalDistance both count as authored length", () => {
    expect(
      hasAuthoredLength("bottom", [{ id: "h", type: "HorizontalDistance", entities: ["bottom"], value: 20 }]),
    ).toBe(true);
    expect(
      hasAuthoredLength("right", [{ id: "v", type: "VerticalDistance", entities: ["right"], value: 10 }]),
    ).toBe(true);
  });
});

describe("unconstrainedEdgeLabel", () => {
  it("gives a Line entity's midpoint + live length, no witness geometry", () => {
    const label = unconstrainedEdgeLabel(bottom); // (0,0)→(20,0)
    expect(label).toEqual({ id: "sel-bottom", at: { x: 10, y: 0 }, value: 20 });
  });

  it("is null for a non-Line entity", () => {
    const circle: SketchEntity = { id: "c", type: "Circle", center: [0, 0], radius: 5 };
    expect(unconstrainedEdgeLabel(circle)).toBeNull();
  });

  it("is null for a Line missing an endpoint", () => {
    const broken: SketchEntity = { id: "broken", type: "Line", p0: [0, 0] };
    expect(unconstrainedEdgeLabel(broken)).toBeNull();
  });
});
