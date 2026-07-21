/*
 * Direct unit coverage for the ONE canonical deg↔rad seam (BUG-2). The three
 * marshalling sites are covered indirectly via sketchWireMap tests; this file
 * pins the seam itself, including the edge values those tests skip.
 */
import { describe, expect, it } from "vitest";
import {
  degToRad,
  fromWireDimensionValue,
  isAngular,
  radToDeg,
  toWireDimensionValue,
} from "./angleUnits";
import type { SketchConstraintType } from "./types";

describe("degToRad / radToDeg", () => {
  it("converts the cardinal values exactly", () => {
    expect(degToRad(0)).toBe(0);
    expect(degToRad(180)).toBeCloseTo(Math.PI, 15);
    expect(degToRad(90)).toBeCloseTo(Math.PI / 2, 15);
    expect(radToDeg(Math.PI)).toBeCloseTo(180, 12);
    expect(radToDeg(0)).toBe(0);
  });

  it("handles negative angles", () => {
    expect(degToRad(-45)).toBeCloseTo(-Math.PI / 4, 15);
    expect(radToDeg(-Math.PI / 2)).toBeCloseTo(-90, 12);
  });

  it("handles values past a full turn (no wrapping — the seam converts, never normalizes)", () => {
    expect(degToRad(450)).toBeCloseTo(2.5 * Math.PI, 12);
    expect(radToDeg(4 * Math.PI)).toBeCloseTo(720, 10);
  });

  it("round-trips within float tolerance, including negatives and >360°", () => {
    for (const deg of [0, 0.001, 30, 90, 179.999, 360, 450, -12.5, -720]) {
      expect(radToDeg(degToRad(deg))).toBeCloseTo(deg, 10);
    }
    for (const rad of [0, 1e-9, Math.PI / 6, Math.PI, 3 * Math.PI, -2.5]) {
      expect(degToRad(radToDeg(rad))).toBeCloseTo(rad, 12);
    }
  });
});

describe("isAngular", () => {
  it("is true ONLY for Angle", () => {
    const kinds: SketchConstraintType[] = [
      "Coincident",
      "Horizontal",
      "Vertical",
      "Fixed",
      "Midpoint",
      "OnCurve",
      "Parallel",
      "Perpendicular",
      "Tangent",
      "Concentric",
      "Equal",
      "Distance",
      "HorizontalDistance",
      "VerticalDistance",
      "Angle",
      "Radius",
      "Diameter",
      "Symmetric",
    ];
    for (const k of kinds) {
      expect(isAngular(k)).toBe(k === "Angle");
    }
  });
});

describe("toWireDimensionValue / fromWireDimensionValue", () => {
  it("converts Angle deg↔rad", () => {
    expect(toWireDimensionValue("Angle", 90)).toBeCloseTo(Math.PI / 2, 15);
    expect(fromWireDimensionValue("Angle", Math.PI / 4)).toBeCloseTo(45, 12);
  });

  it("passes every non-angular dimensional kind through UNTOUCHED (mm both domains)", () => {
    for (const k of ["Distance", "Radius", "Diameter", "HorizontalDistance", "VerticalDistance"] as const) {
      expect(toWireDimensionValue(k, 12.345)).toBe(12.345);
      expect(fromWireDimensionValue(k, -7.5)).toBe(-7.5);
    }
  });

  it("wire→UI→wire round-trips an Angle exactly enough for the solver (1e-12)", () => {
    const wire = 1.0471975511965976; // 60° in radians
    expect(toWireDimensionValue("Angle", fromWireDimensionValue("Angle", wire))).toBeCloseTo(
      wire,
      12,
    );
  });
});
