import { describe, expect, it } from "vitest";
import { frontendSketchTruth } from "./sketchTruth";
import { createIdMap } from "./sketchWireMap";
import type { SketchTruth } from "./types";

const truth: SketchTruth = {
  reasons: { uuid: "residualViolation", foreign: "rankConflict" },
  unassessed: ["uuid", "foreign"],
  violations: [{ constraintId: "uuid", dimension: "mm", residual: 0.001,
    tolerance: 0.0001, normalized: 10, component: "constraint" }],
  normalizedMaxResidual: 10,
};

describe("typed exact truth IPC", () => {
  it("maps every constraint evidence channel through the existing ID map", () => {
    const map = createIdMap("sketch", "XY");
    map.constraint.set("dimension", "uuid");
    const result = frontendSketchTruth(map, truth);
    expect(result.reasons).toEqual({ dimension: "residualViolation" });
    expect(result.unassessed).toEqual(["dimension"]);
    expect(result.violations?.[0]).toEqual({ ...truth.violations?.[0], constraintId: "dimension" });
    expect(result.normalizedMaxResidual).toBe(10);
  });

  it("preserves UUID evidence on static reads and accepts legacy omissions", () => {
    expect(frontendSketchTruth(undefined, truth).reasons).toEqual(truth.reasons);
    expect(frontendSketchTruth(undefined, {}).violations).toEqual([]);
  });
});
