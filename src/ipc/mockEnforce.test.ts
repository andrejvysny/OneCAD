import { describe, it, expect } from "vitest";
import { enforceDriving } from "./mockEnforce";
import type { SketchConstraint, SketchEntity } from "./types";

/*
 * `enforceDriving` is DELTA-driven: the third argument is the PREVIOUS upsert's
 * constraint list. Every case below therefore states which constraints are
 * CREATED (absent from prev), EDITED (present at a different value) and
 * UNCHANGED — that classification, not the constraint set itself, is what
 * decides drive / accept / refuse (mockEnforce.ts decision table).
 */

const line = (
  id: string,
  p0: [number, number],
  p1: [number, number],
  extra?: Partial<SketchEntity>,
): SketchEntity => ({ id, type: "Line", p0, p1, ...extra });

const circle = (id: string, center: [number, number], radius: number, extra?: Partial<SketchEntity>): SketchEntity => ({
  id,
  type: "Circle",
  center,
  radius,
  ...extra,
});

/** No previous upsert — every constraint is CREATED. */
const NONE: SketchConstraint[] = [];

/** `constraints` as they stood at the previous upsert, with `id` at `value` —
 *  i.e. exactly what makes that one constraint an EDIT and nothing else a delta. */
const editedFrom = (constraints: SketchConstraint[], id: string, value: number): SketchConstraint[] =>
  constraints.map((c) => (c.id === id ? { ...c, value } : c));

describe("enforceDriving — Radius / Diameter on a circle", () => {
  it("drives a CREATED Radius constraint onto the circle, center unchanged", () => {
    const entities = [circle("c1", [10, 10], 12.5)];
    const constraints: SketchConstraint[] = [{ id: "r1", type: "Radius", entities: ["c1"], value: 20 }];
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.refusedIds).toEqual([]);
    expect(out.entities.find((e) => e.id === "c1")?.radius).toBe(20);
    expect(out.entities.find((e) => e.id === "c1")?.center).toEqual([10, 10]);
    expect(out.solvedCurves).toEqual({ c1: { radius: 20 } });
    expect(out.solvedPositions).toEqual({});
  });

  it("drives an EDITED Diameter at half the value as radius (the bug case: 25 -> 30 -> r=15)", () => {
    const entities = [circle("c1", [0, 0], 12.5)]; // drawn at diameter 25
    const constraints: SketchConstraint[] = [{ id: "d1", type: "Diameter", entities: ["c1"], value: 30 }];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "d1", 25));
    expect(out.refusedIds).toEqual([]);
    expect(out.entities.find((e) => e.id === "c1")?.radius).toBe(15);
    expect(out.solvedCurves).toEqual({ c1: { radius: 15 } });
  });

  it("is a no-op (identity, same array reference) when the value already matches", () => {
    const entities = [circle("c1", [0, 0], 12.5)];
    const constraints: SketchConstraint[] = [{ id: "d1", type: "Diameter", entities: ["c1"], value: 25 }];
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.entities).toBe(entities);
    expect(out.solvedCurves).toEqual({});
    expect(out.refusedIds).toEqual([]);
  });

  it("is idempotent: re-running the SAME upsert on its own output changes nothing further", () => {
    const entities = [circle("c1", [0, 0], 12.5)];
    const constraints: SketchConstraint[] = [{ id: "d1", type: "Diameter", entities: ["c1"], value: 30 }];
    const prev = editedFrom(constraints, "d1", 25);
    const first = enforceDriving(entities, constraints, prev);
    const second = enforceDriving(first.entities, constraints, prev);
    expect(second.entities).toBe(first.entities);
    expect(second.solvedCurves).toEqual({});
    expect(second.refusedIds).toEqual([]);
  });

  it("refuses an EDITED Radius on a referenceLocked circle (guard: not drivable)", () => {
    const entities = [circle("c1", [0, 0], 12.5, { referenceLocked: true })];
    const constraints: SketchConstraint[] = [{ id: "r1", type: "Radius", entities: ["c1"], value: 20 }];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "r1", 12.5));
    expect(out.refusedIds).toEqual(["r1"]);
    expect(out.entities.find((e) => e.id === "c1")?.radius).toBe(12.5); // untouched
    expect(out.solvedCurves).toEqual({});
  });

  it("ACCEPTS a CREATED Radius on a referenceLocked circle silently, geometry unmoved", () => {
    const entities = [circle("c1", [0, 0], 12.5, { referenceLocked: true })];
    const constraints: SketchConstraint[] = [{ id: "r1", type: "Radius", entities: ["c1"], value: 20 }];
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.refusedIds).toEqual([]);
    expect(out.entities).toBe(entities);
  });

  it("does NOT refuse a locked circle's EDITED Radius when the new value already matches", () => {
    const entities = [circle("c1", [0, 0], 12.5, { referenceLocked: true })];
    const constraints: SketchConstraint[] = [{ id: "r1", type: "Radius", entities: ["c1"], value: 12.5 }];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "r1", 20));
    expect(out.refusedIds).toEqual([]);
  });

  it("a weld on the circle's CENTER does not block a Radius drive (no point moves)", () => {
    const entities = [circle("c1", [0, 0], 12.5), { id: "p1", type: "Point", p0: [0, 0] } as SketchEntity];
    const constraints: SketchConstraint[] = [
      { id: "r1", type: "Radius", entities: ["c1"], value: 20 },
      { id: "co1", type: "Coincident", entities: ["c1", "p1"], positions: ["Center", "Start"] },
    ];
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.refusedIds).toEqual([]);
    expect(out.entities.find((e) => e.id === "c1")?.radius).toBe(20);
  });
});

describe("enforceDriving — Radius / Diameter on an ARC is never driven", () => {
  const arc = (): SketchEntity => ({
    id: "a1",
    type: "Arc",
    center: [0, 0],
    radius: 10,
    start: [10, 0],
    end: [0, 10],
  });

  it("REFUSES an EDITED arc Radius instead of rewriting radius and detaching start/end", () => {
    const entities = [arc()];
    const constraints: SketchConstraint[] = [{ id: "r1", type: "Radius", entities: ["a1"], value: 20 }];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "r1", 10));
    expect(out.refusedIds).toEqual(["r1"]);
    const a1 = out.entities.find((e) => e.id === "a1")!;
    expect(a1.radius).toBe(10); // untouched
    expect(a1.start).toEqual([10, 0]); // stored coords still ON the stroke
    expect(a1.end).toEqual([0, 10]);
    expect(out.solvedCurves).toEqual({});
  });

  it("ACCEPTS a CREATED arc Radius silently, arc untouched", () => {
    const entities = [arc()];
    const constraints: SketchConstraint[] = [{ id: "r1", type: "Radius", entities: ["a1"], value: 20 }];
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.refusedIds).toEqual([]);
    expect(out.entities).toBe(entities);
    expect(out.solvedCurves).toEqual({});
  });
});

describe("enforceDriving — Distance on a single line's own two endpoints", () => {
  it("scales the line about its midpoint along its current direction", () => {
    const entities = [line("e1", [0, 0], [10, 0])]; // length 10, midpoint (5,0)
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 20 },
    ];
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.refusedIds).toEqual([]);
    const e1 = out.entities.find((e) => e.id === "e1")!;
    expect(e1.p0).toEqual([-5, 0]);
    expect(e1.p1).toEqual([15, 0]);
    expect(out.solvedPositions).toEqual({ "e1.Start": [-5, 0], "e1.End": [15, 0] });
  });

  it("preserves direction for a non-axis-aligned line", () => {
    const entities = [line("e1", [0, 0], [6, 8])]; // length 10, dir (0.6, 0.8)
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["End", "Start"], value: 20 },
    ];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "d1", 10));
    const e1 = out.entities.find((e) => e.id === "e1")!;
    expect(e1.p0![0]).toBeCloseTo(-3, 9);
    expect(e1.p0![1]).toBeCloseTo(-4, 9);
    expect(e1.p1![0]).toBeCloseTo(9, 9);
    expect(e1.p1![1]).toBeCloseTo(12, 9);
  });

  it("refuses an EDIT when a weld (Coincident) references the line — mock cannot propagate", () => {
    const entities = [line("e1", [0, 0], [10, 0]), line("e2", [10, 0], [10, 10])];
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 20 },
      { id: "co1", type: "Coincident", entities: ["e1", "e2"], positions: ["End", "Start"] },
    ];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "d1", 10));
    expect(out.refusedIds).toEqual(["d1"]);
    const e1 = out.entities.find((e) => e.id === "e1")!;
    expect(e1.p0).toEqual([0, 0]); // untouched
    expect(e1.p1).toEqual([10, 0]);
  });

  it("refuses an EDIT when the line is referenceLocked", () => {
    const entities = [line("e1", [0, 0], [10, 0], { referenceLocked: true })];
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 20 },
    ];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "d1", 10));
    expect(out.refusedIds).toEqual(["d1"]);
  });

  it("refuses an EDIT of an unsupported shape (Distance between two DIFFERENT lines)", () => {
    const entities = [line("e1", [0, 0], [10, 0]), line("e2", [0, 20], [10, 20])];
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e2"], positions: ["Start", "Start"], value: 999 },
    ];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "d1", 20));
    expect(out.refusedIds).toEqual(["d1"]);
    // Neither line moved — this module only ever drives the narrow same-line shape.
    expect(out.entities).toBe(entities);
  });

  it("ACCEPTS a CREATED unsupported shape silently (geometry unmoved, no refusal)", () => {
    const entities = [line("e1", [0, 0], [10, 0]), line("e2", [0, 20], [10, 20])];
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e2"], positions: ["Start", "Start"], value: 999 },
    ];
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.refusedIds).toEqual([]);
    expect(out.entities).toBe(entities);
  });

  it("does NOT refuse an EDITED unsupported shape when the new value already matches geometry", () => {
    const entities = [line("e1", [0, 0], [10, 0]), line("e2", [0, 20], [10, 20])];
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e2"], positions: ["Start", "Start"], value: 20 }, // matches |[0,0]-[0,20]|
    ];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "d1", 999));
    expect(out.refusedIds).toEqual([]);
  });

  it("refuses an EDIT on a degenerate (zero-length) line — no direction to scale along", () => {
    const entities = [line("e1", [3, 3], [3, 3])];
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 20 },
    ];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "d1", 5));
    expect(out.refusedIds).toEqual(["d1"]);
    expect(out.entities).toBe(entities);
  });
});

describe("enforceDriving — a rectangle's welded edge (blocker B1)", () => {
  /** A closed axis-aligned rectangle: four lines, corner Coincidents — so EVERY
   *  edge is welded and none of them is drivable by this module. */
  const rect = (): { entities: SketchEntity[]; constraints: SketchConstraint[] } => {
    const entities = [
      line("l0", [0, 0], [40, 0]),
      line("l1", [40, 0], [40, 20]),
      line("l2", [40, 20], [0, 20]),
      line("l3", [0, 20], [0, 0]),
    ];
    const constraints: SketchConstraint[] = [
      { id: "co0", type: "Coincident", entities: ["l0", "l1"], positions: ["End", "Start"] },
      { id: "co1", type: "Coincident", entities: ["l1", "l2"], positions: ["End", "Start"] },
      { id: "co2", type: "Coincident", entities: ["l2", "l3"], positions: ["End", "Start"] },
      { id: "co3", type: "Coincident", entities: ["l3", "l0"], positions: ["End", "Start"] },
    ];
    return { entities, constraints };
  };

  it("ACCEPTS a newly authored Distance on a rect edge — never refused, never driven", () => {
    const { entities, constraints } = rect();
    const dim: SketchConstraint = {
      id: "d1",
      type: "Distance",
      entities: ["l0", "l0"],
      positions: ["Start", "End"],
      value: 90, // the edge actually measures 40
    };
    const out = enforceDriving(entities, [...constraints, dim], constraints);
    expect(out.refusedIds).toEqual([]);
    expect(out.entities).toBe(entities); // geometry unmoved: the real solver's job
    expect(out.solvedPositions).toEqual({});
  });

  it("REFUSES an EDIT of that same rect-edge Distance (the label would lie)", () => {
    const { entities, constraints } = rect();
    const dim: SketchConstraint = {
      id: "d1",
      type: "Distance",
      entities: ["l0", "l0"],
      positions: ["Start", "End"],
      value: 90,
    };
    const withDim = [...constraints, dim];
    const out = enforceDriving(entities, withDim, editedFrom(withDim, "d1", 40));
    expect(out.refusedIds).toEqual(["d1"]);
    expect(out.entities).toBe(entities);
  });

  it("does not refuse the rect's own auto-inferred Horizontal/Vertical batch", () => {
    const { entities, constraints } = rect();
    const inferred: SketchConstraint[] = [
      ...constraints,
      { id: "h0", type: "Horizontal", entities: ["l0"] },
      { id: "v1", type: "Vertical", entities: ["l1"] },
    ];
    const out = enforceDriving(entities, inferred, NONE);
    expect(out.refusedIds).toEqual([]);
    expect(out.entities).toBe(entities);
  });
});

describe("enforceDriving — HorizontalDistance / VerticalDistance on the same line", () => {
  it("HorizontalDistance sets the X delta, keeps the midpoint and Y delta", () => {
    const entities = [line("e1", [0, 0], [10, 4])]; // dx=10, dy=4, midpoint (5,2)
    const constraints: SketchConstraint[] = [
      { id: "hd1", type: "HorizontalDistance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 20 },
    ];
    const out = enforceDriving(entities, constraints, NONE);
    const e1 = out.entities.find((e) => e.id === "e1")!;
    expect(e1.p0).toEqual([-5, 0]);
    expect(e1.p1).toEqual([15, 4]);
    expect(e1.p1![1] - e1.p0![1]).toBe(4); // Y delta preserved
    expect((e1.p0![0] + e1.p1![0]) / 2).toBe(5); // midpoint X preserved
  });

  it("VerticalDistance sets the Y delta, keeps the midpoint and X delta", () => {
    const entities = [line("e1", [0, 0], [4, 10])]; // dx=4, dy=10, midpoint (2,5)
    const constraints: SketchConstraint[] = [
      { id: "vd1", type: "VerticalDistance", entities: ["e1", "e1"], positions: ["Start", "End"], value: -20 },
    ];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "vd1", 10));
    const e1 = out.entities.find((e) => e.id === "e1")!;
    // signed: End.y - Start.y === -20
    expect(e1.p1![1] - e1.p0![1]).toBe(-20);
    expect(e1.p1![0] - e1.p0![0]).toBe(4); // X delta preserved
    expect((e1.p0![1] + e1.p1![1]) / 2).toBe(5); // midpoint Y preserved
  });
});

describe("enforceDriving — Horizontal / Vertical geometric projection", () => {
  it("projects a newly created Horizontal onto the axis through the midpoint (bug: 0.4 deg forever)", () => {
    const entities = [line("e1", [0, 0], [40, 0.28])]; // ~0.4deg tilt
    const constraints: SketchConstraint[] = [{ id: "h1", type: "Horizontal", entities: ["e1"] }];
    const out = enforceDriving(entities, constraints, NONE);
    const e1 = out.entities.find((e) => e.id === "e1")!;
    expect(e1.p0![1]).toBe(e1.p1![1]); // exactly horizontal now
    expect(e1.p0![1]).toBeCloseTo(0.14, 9); // midpoint Y preserved
  });

  it("projects a newly created Vertical onto the axis through the midpoint", () => {
    const entities = [line("e1", [0, 0], [0.3, 40])];
    const constraints: SketchConstraint[] = [{ id: "v1", type: "Vertical", entities: ["e1"] }];
    const out = enforceDriving(entities, constraints, NONE);
    const e1 = out.entities.find((e) => e.id === "e1")!;
    expect(e1.p0![0]).toBe(e1.p1![0]);
    expect(e1.p0![0]).toBeCloseTo(0.15, 9);
  });

  it("is a no-op (same array reference) on an already-exact Horizontal line", () => {
    const entities = [line("e1", [0, 0], [40, 0])];
    const constraints: SketchConstraint[] = [{ id: "h1", type: "Horizontal", entities: ["e1"] }];
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.entities).toBe(entities);
    expect(out.solvedPositions).toEqual({});
  });

  it("does NOT re-project an UNCHANGED Horizontal whose line a drag has since tilted", () => {
    const entities = [line("e1", [0, 0], [40, 3])]; // dragged out of true
    const constraints: SketchConstraint[] = [{ id: "h1", type: "Horizontal", entities: ["e1"] }];
    const out = enforceDriving(entities, constraints, constraints); // present before AND now
    expect(out.entities).toBe(entities);
    expect(out.solvedPositions).toEqual({});
    expect(out.refusedIds).toEqual([]);
  });

  it("leaves a welded, tilted line untouched (guard) — never a refusal (no numeric label to lie)", () => {
    const entities = [line("e1", [0, 0], [40, 0.28]), line("e2", [40, 0.28], [40, 20])];
    const constraints: SketchConstraint[] = [
      { id: "h1", type: "Horizontal", entities: ["e1"] },
      { id: "co1", type: "Coincident", entities: ["e1", "e2"], positions: ["End", "Start"] },
    ];
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.refusedIds).toEqual([]);
    const e1 = out.entities.find((e) => e.id === "e1")!;
    expect(e1.p1).toEqual([40, 0.28]); // untouched
  });
});

describe("enforceDriving — Angle (never driven)", () => {
  it("does not touch geometry — Angle has no provable driving rule", () => {
    const entities = [line("a", [0, 0], [10, 0]), line("b", [0, 0], [0, 20])];
    const constraints: SketchConstraint[] = [{ id: "ang1", type: "Angle", entities: ["a", "b"], value: 90 }]; // matches
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.entities).toBe(entities);
    expect(out.refusedIds).toEqual([]);
  });

  it("refuses a mismatched Angle EDIT instead of silently accepting the wrong label", () => {
    const entities = [line("a", [0, 0], [10, 0]), line("b", [0, 0], [0, 20])]; // actual angle 90
    const constraints: SketchConstraint[] = [{ id: "ang1", type: "Angle", entities: ["a", "b"], value: 45 }];
    const out = enforceDriving(entities, constraints, editedFrom(constraints, "ang1", 90));
    expect(out.refusedIds).toEqual(["ang1"]);
    expect(out.entities).toBe(entities); // never touched
  });

  it("ACCEPTS a mismatched Angle CREATION silently (the real solver's job)", () => {
    const entities = [line("a", [0, 0], [10, 0]), line("b", [0, 0], [0, 20])];
    const constraints: SketchConstraint[] = [{ id: "ang1", type: "Angle", entities: ["a", "b"], value: 45 }];
    const out = enforceDriving(entities, constraints, NONE);
    expect(out.refusedIds).toEqual([]);
    expect(out.entities).toBe(entities);
  });
});

describe("enforceDriving — delta scoping (regressions B2 / M1)", () => {
  it("B2: an UNCHANGED stale dimension is not refused when an unrelated constraint is added", () => {
    // `d1` says 50 while the line measures 10 (a drag left it stale, and the drag
    // lane never re-checks). Adding a Radius elsewhere must not resurrect it.
    const entities = [line("e1", [0, 0], [10, 0]), circle("c1", [50, 50], 5)];
    const stale: SketchConstraint = {
      id: "d1",
      type: "Distance",
      entities: ["e1", "e1"],
      positions: ["Start", "End"],
      value: 50,
    };
    const added: SketchConstraint = { id: "r1", type: "Radius", entities: ["c1"], value: 8 };
    const out = enforceDriving(entities, [stale, added], [stale]);
    expect(out.refusedIds).toEqual([]);
    const e1 = out.entities.find((e) => e.id === "e1")!;
    expect(e1.p0).toEqual([0, 0]); // stale dimension's line untouched
    expect(e1.p1).toEqual([10, 0]);
    expect(out.entities.find((e) => e.id === "c1")?.radius).toBe(8); // the delta still drove
    expect(out.solvedPositions).toEqual({});
  });

  it("B2: a stale dimension is not refused on ANY number of later unrelated upserts", () => {
    const entities = [line("e1", [0, 0], [10, 0])];
    const stale: SketchConstraint = {
      id: "d1",
      type: "Distance",
      entities: ["e1", "e1"],
      positions: ["Start", "End"],
      value: 50,
    };
    let constraints: SketchConstraint[] = [stale];
    let current = entities;
    for (const id of ["x1", "x2", "x3"]) {
      const next = [...constraints, { id, type: "Vertical", entities: ["nonexistent"] } as SketchConstraint];
      const out = enforceDriving(current, next, constraints);
      expect(out.refusedIds).toEqual([]);
      current = out.entities;
      constraints = next;
    }
    expect(current).toBe(entities);
  });

  it("M1: Distance + HorizontalDistance on one line do not drift across repeated upserts", () => {
    const entities = [line("e1", [0, 0], [24, 10])];
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 30 },
      { id: "hd1", type: "HorizontalDistance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 20 },
    ];
    const measure = (es: SketchEntity[]): [number, number] => {
      const e1 = es.find((e) => e.id === "e1")!;
      return [Math.hypot(e1.p1![0] - e1.p0![0], e1.p1![1] - e1.p0![1]), e1.p1![0] - e1.p0![0]];
    };

    const first = enforceDriving(entities, constraints, NONE); // both CREATED
    const settled = measure(first.entities);

    // Every later upsert re-sends the same constraint set (a draw, a selection,
    // any unrelated edit): the delta is empty, so nothing is re-driven.
    const second = enforceDriving(first.entities, constraints, constraints);
    const third = enforceDriving(second.entities, constraints, constraints);
    expect(second.entities).toBe(first.entities);
    expect(third.entities).toBe(first.entities);
    expect(measure(third.entities)).toEqual(settled);
    expect(second.solvedPositions).toEqual({});
    expect(third.solvedPositions).toEqual({});
    expect(third.refusedIds).toEqual([]);
  });

  it("re-running the very same upsert is a no-op (idempotent on its own output)", () => {
    const entities = [line("e1", [0, 0], [24, 10]), circle("c1", [0, 0], 5)];
    const constraints: SketchConstraint[] = [
      { id: "h1", type: "Horizontal", entities: ["e1"] },
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 30 },
      { id: "r1", type: "Radius", entities: ["c1"], value: 8 },
    ];
    const first = enforceDriving(entities, constraints, NONE);
    const again = enforceDriving(first.entities, constraints, NONE);
    expect(again.entities).toBe(first.entities);
    expect(again.solvedPositions).toEqual({});
    expect(again.solvedCurves).toEqual({});
    expect(again.refusedIds).toEqual([]);
  });

  it("a removed constraint is not a delta — nothing is driven or refused for it", () => {
    const entities = [line("e1", [0, 0], [10, 0])];
    const gone: SketchConstraint = {
      id: "d1",
      type: "Distance",
      entities: ["e1", "e1"],
      positions: ["Start", "End"],
      value: 50,
    };
    const out = enforceDriving(entities, [], [gone]);
    expect(out.entities).toBe(entities);
    expect(out.refusedIds).toEqual([]);
  });
});

describe("enforceDriving — ordering within one upsert", () => {
  it("applies a newly created Horizontal BEFORE the delta's Distance drive", () => {
    const entities = [line("e1", [0, 0], [10, 1])]; // tilted, length ~10.05
    const constraints: SketchConstraint[] = [
      { id: "h1", type: "Horizontal", entities: ["e1"] },
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 20 },
    ];
    const out = enforceDriving(entities, constraints, NONE);
    const e1 = out.entities.find((e) => e.id === "e1")!;
    // H first: both endpoints to y = 0.5. Then Distance along the now-horizontal
    // direction about x = 5. Driving first would have scaled along the TILT and
    // the projection would then have shortened it back below 20.
    expect(e1.p0).toEqual([-5, 0.5]);
    expect(e1.p1).toEqual([15, 0.5]);
    expect(Math.hypot(e1.p1![0] - e1.p0![0], e1.p1![1] - e1.p0![1])).toBe(20);
  });

  it("drives delta dimensional constraints in constraint-array order (last writer wins)", () => {
    const entities = [line("e1", [0, 0], [10, 0])];
    const forward: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 30 },
      { id: "hd1", type: "HorizontalDistance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 20 },
    ];
    const out = enforceDriving(entities, forward, NONE);
    const e1 = out.entities.find((e) => e.id === "e1")!;
    expect(e1.p1![0] - e1.p0![0]).toBe(20); // the LATER HorizontalDistance won

    const reversed = [forward[1], forward[0]];
    const out2 = enforceDriving(entities, reversed, NONE);
    const e1b = out2.entities.find((e) => e.id === "e1")!;
    expect(Math.hypot(e1b.p1![0] - e1b.p0![0], e1b.p1![1] - e1b.p0![1])).toBe(30); // the later Distance won
  });
});
