import { describe, it, expect } from "vitest";
import { detectConflicts } from "./mockConflicts";
import { AUTO_KINDS_LEGACY, inferConstraints } from "@/tools/sketch/autoConstrain";
import type { SketchConstraint, SketchEntity } from "./types";

const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({
  id,
  type: "Line",
  p0,
  p1,
});

describe("detectConflicts — R1 duplicate-incompatible dimension", () => {
  const entities: SketchEntity[] = [line("e1", [0, 0], [40, 0])];

  it("fires: two Distance constraints on the same point pair, different values", () => {
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 50 },
      { id: "d2", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 120 },
    ];
    expect(detectConflicts(entities, constraints).sort()).toEqual(["d1", "d2"]);
  });

  it("fires: two Radius constraints on the same circle, different values", () => {
    const circle: SketchEntity[] = [{ id: "c1", type: "Circle", center: [0, 0], radius: 5 }];
    const constraints: SketchConstraint[] = [
      { id: "r1", type: "Radius", entities: ["c1"], value: 5 },
      { id: "r2", type: "Radius", entities: ["c1"], value: 8 },
    ];
    expect(detectConflicts(circle, constraints).sort()).toEqual(["r1", "r2"]);
  });

  it("does NOT fire: equal values (a harmless re-dimension)", () => {
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 },
      { id: "d2", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 },
    ];
    expect(detectConflicts(entities, constraints)).toEqual([]);
  });

  it("does NOT fire: different target pairs", () => {
    const two: SketchEntity[] = [line("e1", [0, 0], [40, 0]), line("e2", [0, 0], [0, 40])];
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 },
      { id: "d2", type: "Distance", entities: ["e2", "e2"], positions: ["Start", "End"], value: 999 },
    ];
    expect(detectConflicts(two, constraints)).toEqual([]);
  });

  it("does NOT fire across different dimensional kinds on the same target", () => {
    // HorizontalDistance vs VerticalDistance measure different things — not a clash.
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "HorizontalDistance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 },
      { id: "d2", type: "VerticalDistance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 0 },
    ];
    expect(detectConflicts(entities, constraints)).toEqual([]);
  });
});

describe("detectConflicts — R2 Horizontal + Vertical", () => {
  it("fires: both authored on the same line", () => {
    const entities: SketchEntity[] = [line("e1", [0, 0], [40, 0])];
    const constraints: SketchConstraint[] = [
      { id: "h1", type: "Horizontal", entities: ["e1"] },
      { id: "v1", type: "Vertical", entities: ["e1"] },
    ];
    expect(detectConflicts(entities, constraints).sort()).toEqual(["h1", "v1"]);
  });

  it("does NOT fire: Horizontal and Vertical on DIFFERENT lines", () => {
    const entities: SketchEntity[] = [line("e1", [0, 0], [40, 0]), line("e2", [0, 0], [0, 40])];
    const constraints: SketchConstraint[] = [
      { id: "h1", type: "Horizontal", entities: ["e1"] },
      { id: "v1", type: "Vertical", entities: ["e2"] },
    ];
    expect(detectConflicts(entities, constraints)).toEqual([]);
  });
});

describe("detectConflicts — R3 Parallel + Perpendicular", () => {
  const entities: SketchEntity[] = [line("e1", [0, 0], [40, 0]), line("e2", [0, 10], [40, 10])];

  it("fires: both authored on the same unordered line pair", () => {
    const constraints: SketchConstraint[] = [
      { id: "p1", type: "Parallel", entities: ["e1", "e2"] },
      { id: "x1", type: "Perpendicular", entities: ["e2", "e1"] }, // reversed order — still same pair
    ];
    expect(detectConflicts(entities, constraints).sort()).toEqual(["p1", "x1"]);
  });

  it("does NOT fire: Parallel and Perpendicular on DIFFERENT pairs", () => {
    const three: SketchEntity[] = [...entities, line("e3", [0, 20], [40, 60])];
    const constraints: SketchConstraint[] = [
      { id: "p1", type: "Parallel", entities: ["e1", "e2"] },
      { id: "x1", type: "Perpendicular", entities: ["e1", "e3"] },
    ];
    expect(detectConflicts(three, constraints)).toEqual([]);
  });
});

describe("detectConflicts — R4 Fixed contradictions", () => {
  it("fires: two Fixed points declared Coincident but pinned at different locations", () => {
    const entities: SketchEntity[] = [line("e1", [0, 0], [1, 1]), line("e2", [5, 5], [6, 6])];
    const constraints: SketchConstraint[] = [
      { id: "f1", type: "Fixed", entities: ["e1"], positions: ["Start"] },
      { id: "f2", type: "Fixed", entities: ["e2"], positions: ["Start"] },
      { id: "co1", type: "Coincident", entities: ["e1", "e2"], positions: ["Start", "Start"] },
    ];
    expect(detectConflicts(entities, constraints).sort()).toEqual(["f1", "f2"]);
  });

  it("does NOT fire: Coincident points that are ALREADY at the same location", () => {
    const entities: SketchEntity[] = [line("e1", [0, 0], [1, 1]), line("e2", [0, 0], [6, 6])];
    const constraints: SketchConstraint[] = [
      { id: "f1", type: "Fixed", entities: ["e1"], positions: ["Start"] },
      { id: "f2", type: "Fixed", entities: ["e2"], positions: ["Start"] },
      { id: "co1", type: "Coincident", entities: ["e1", "e2"], positions: ["Start", "Start"] },
    ];
    expect(detectConflicts(entities, constraints)).toEqual([]);
  });

  it("fires: a Distance between two Fixed points that disagrees with their actual separation", () => {
    const entities: SketchEntity[] = [line("e1", [0, 0], [1, 1]), line("e2", [10, 0], [11, 1])];
    const constraints: SketchConstraint[] = [
      { id: "f1", type: "Fixed", entities: ["e1"], positions: ["Start"] },
      { id: "f2", type: "Fixed", entities: ["e2"], positions: ["Start"] },
      { id: "d1", type: "Distance", entities: ["e1", "e2"], positions: ["Start", "Start"], value: 999 },
    ];
    expect(detectConflicts(entities, constraints).sort()).toEqual(["d1", "f1", "f2"]);
  });

  it("does NOT fire: a Distance between two Fixed points that AGREES with their actual separation", () => {
    const entities: SketchEntity[] = [line("e1", [0, 0], [1, 1]), line("e2", [10, 0], [11, 1])];
    const constraints: SketchConstraint[] = [
      { id: "f1", type: "Fixed", entities: ["e1"], positions: ["Start"] },
      { id: "f2", type: "Fixed", entities: ["e2"], positions: ["Start"] },
      { id: "d1", type: "Distance", entities: ["e1", "e2"], positions: ["Start", "Start"], value: 10 },
    ];
    expect(detectConflicts(entities, constraints)).toEqual([]);
  });
});

describe("detectConflicts — no false positives on autoConstrain-inferred batches", () => {
  it("a representative inferred batch (H + Coincident + Parallel + Concentric + Equal) never conflicts", () => {
    // Existing reference geometry: a 30°-ish line + a circle.
    const refLine: SketchEntity = { id: "ref-line", type: "Line", p0: [0, 50], p1: [10, 55.77] };
    const refCircle: SketchEntity = { id: "ref-circle", type: "Circle", center: [100, 100], radius: 5 };
    const existing = [refLine, refCircle];

    // Horizontal line, joined at its End by a second (Vertical) line — auto-infers
    // Horizontal, Vertical, and Coincident (the shared endpoint).
    const lineH: SketchEntity = { id: "line-h", type: "Line", p0: [0, 0], p1: [10, 0] };
    const lineJoin: SketchEntity = { id: "line-join", type: "Line", p0: [10, 0], p1: [10, 10] };
    // Same ~30° direction as refLine, well clear of it — auto-infers Parallel only
    // (angleBetweenLines to refLine folds near 0°, nowhere near the 90° perpendicular
    // window, so Perpendicular never fires for this pair).
    const lineParallel: SketchEntity = { id: "line-parallel", type: "Line", p0: [50, 0], p1: [60, 5.77] };
    // Close to (but not exactly at) refCircle's center/radius — within Concentric's
    // 2mm tolerance and Equal's 0.5mm tolerance, but NOT within the 1e-6 coincidence
    // tolerance, so Concentric fires ungated by a Coincident.
    const circleNear: SketchEntity = { id: "circle-near", type: "Circle", center: [100, 100.001], radius: 5.0002 };

    const newEntities = [lineH, lineJoin, lineParallel, circleNear];
    let nextId = 0;
    // AUTO_KINDS_LEGACY on purpose: this test guards `detectConflicts` against
    // false positives on a RICH batch, and Parallel/Concentric/Equal still reach
    // a document by MANUAL application even though V2 policy no longer infers
    // them (SKETCH-V2 P1). Using the restricted default here would quietly drop
    // three kinds of conflict coverage.
    const out = inferConstraints(newEntities, existing, {
      nextConstraintId: () => `c${nextId++}`,
      kinds: AUTO_KINDS_LEGACY,
    });

    // Sanity: the batch actually exercises every kind the test name promises.
    const kinds = new Set(out.map((c) => c.type));
    expect(kinds.has("Horizontal")).toBe(true);
    expect(kinds.has("Coincident")).toBe(true);
    expect(kinds.has("Parallel")).toBe(true);
    expect(kinds.has("Concentric")).toBe(true);
    expect(kinds.has("Equal")).toBe(true);

    expect(detectConflicts([...existing, ...newEntities], out)).toEqual([]);
  });
});
