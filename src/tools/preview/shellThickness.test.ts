/*
 * W2-A GUARD for the shell re-edit seed (mirrors filletRadius.test.ts).
 *
 * `thicknessFromValueText` is what a parametric shell RE-EDIT opens with, and it
 * parses two independently-produced strings: the RUST-composed feature
 * `valueText` ("2.0 mm", dto.rs feature_value_text — untouched by this wave) and
 * this module's own `formatThickness`, which W2-A re-pointed at the shared
 * `formatLength`. Breaking either parse would seed a re-edit with a silently
 * wrong thickness, so both are asserted as identities.
 */
import { describe, it, expect } from "vitest";
import {
  DEFAULT_SHELL_THICKNESS,
  MIN_SHELL_THICKNESS,
  cylindricalWallPath,
  cylindricalWallRadiusAt,
  formatThickness,
  planarWallPath,
  shellThicknessWitness,
  thicknessFromValueText,
} from "./shellThickness";
import { handlePointAt } from "./handleProjection";

describe("formatThickness", () => {
  it("formats with the shared length formatter (trailing zeros trimmed)", () => {
    expect(formatThickness(2)).toBe("2 mm");
    expect(formatThickness(0.1)).toBe("0.1 mm");
    expect(formatThickness(83.25)).toBe("83.25 mm");
    expect(formatThickness(100)).toBe("100 mm");
  });
});

describe("thicknessFromValueText (shell re-edit seed)", () => {
  it("parses a shell feature's display text back to a thickness", () => {
    expect(thicknessFromValueText("2.0 mm")).toBe(2);
    expect(thicknessFromValueText("12.5 mm")).toBe(12.5);
  });

  it("falls back to the default for non-numeric / non-positive text", () => {
    expect(thicknessFromValueText("")).toBe(DEFAULT_SHELL_THICKNESS);
    expect(thicknessFromValueText("—")).toBe(DEFAULT_SHELL_THICKNESS);
    expect(thicknessFromValueText("0 mm")).toBe(DEFAULT_SHELL_THICKNESS);
    expect(thicknessFromValueText("bad", 7)).toBe(7);
  });

  it("round-trips every value formatThickness can produce", () => {
    for (const v of [MIN_SHELL_THICKNESS, 1, 2, 12.5, 83.25, 100, 1000]) {
      expect(thicknessFromValueText(formatThickness(v))).toBeCloseTo(v, 9);
    }
  });

  it("still round-trips the RUST valueText form (fixed 1dp)", () => {
    for (const v of [2, 12.5, 83.3, 100]) {
      expect(thicknessFromValueText(`${v.toFixed(1)} mm`)).toBeCloseTo(v, 9);
    }
  });
});

/*
 * H9 — the retained-wall thickness attachment
 * (docs/design/astra/modeling-handle-attachment.md §5 "Shell", §6 test vectors).
 */
describe("planarWallPath — H(t) = E − t·n", () => {
  it("walks INTO the material: the open-box vector E=(40,20,40), n=+X, t=2", () => {
    const path = planarWallPath([40, 20, 40], [1, 0, 0])!;
    expect(path.q0Mm).toBe(0);
    expect(path.point0Mm).toEqual([40, 20, 40]);
    expect(path.dPointDValue).toEqual([-1, 0, 0]);
    expect(handlePointAt(path, 2)).toEqual([38, 20, 40]);
  });

  it("normalizes the wall normal, so the gain is per millimetre of thickness", () => {
    const path = planarWallPath([0, 0, 0], [0, 0, 3])!;
    expect(path.dPointDValue).toEqual([0, 0, -1]);
    expect(handlePointAt(path, 2)[2]).toBeCloseTo(-2, 12);
  });

  it("refuses a degenerate or non-finite frame rather than producing NaN", () => {
    expect(planarWallPath([0, 0, 0], [0, 0, 0])).toBeNull();
    expect(planarWallPath([Number.NaN, 0, 0], [1, 0, 0])).toBeNull();
    expect(planarWallPath([0, 0, 0], [Number.POSITIVE_INFINITY, 0, 0])).toBeNull();
  });
});

describe("cylindricalWallPath — H(t) = C + (R − σt)·r̂", () => {
  it("offsets a PIN wall inward: R=10, axis Z, E=(10,0,40), t=2 ⇒ (8,0,40)", () => {
    const path = cylindricalWallPath([0, 0, 0], [0, 0, 1], [10, 0, 40], 10, "pin")!;
    expect(path.point0Mm[0]).toBeCloseTo(10, 12);
    expect(path.point0Mm[1]).toBeCloseTo(0, 12);
    expect(path.point0Mm[2]).toBeCloseTo(40, 12);
    expect(path.dPointDValue[0]).toBeCloseTo(-1, 12);
    const h = handlePointAt(path, 2);
    expect(h[0]).toBeCloseTo(8, 12);
    expect(h[1]).toBeCloseTo(0, 12);
    expect(h[2]).toBeCloseTo(40, 12);
  });

  /*
   * A BORE's remaining wall is at LARGER radius: the material is outside the
   * cylinder, so σ = −1 and the inward offset runs away from the axis. Using the
   * material-outward direction for both would invert this drag (§7).
   */
  it("offsets a HOLE wall away from the axis: σ = −1", () => {
    const path = cylindricalWallPath([0, 0, 0], [0, 0, 1], [6, 0, 0], 6, "hole")!;
    expect(path.dPointDValue[0]).toBeCloseTo(1, 12);
    expect(handlePointAt(path, 2)[0]).toBeCloseTo(8, 12);
  });

  /*
   * The rim point only supplies the RADIAL direction: the radius is the
   * classified one, so a faceted polyline vertex inside the true cylinder does
   * not shrink the construction.
   */
  it("seats the path at the CLASSIFIED radius, not at the rim point's own", () => {
    const path = cylindricalWallPath([0, 0, 0], [0, 0, 1], [9.9, 0, 40], 10, "pin")!;
    expect(path.point0Mm[0]).toBeCloseTo(10, 12);
  });

  it("re-projects the axis: an axis point off the rim's plane still centres C", () => {
    const path = cylindricalWallPath([0, 0, -100], [0, 0, 2], [10, 0, 40], 10, "pin")!;
    expect(path.point0Mm[2]).toBeCloseTo(40, 12);
  });

  it("refuses an unresolvable radial anchor, a degenerate axis and a bad radius", () => {
    expect(cylindricalWallPath([0, 0, 0], [0, 0, 1], [0, 0, 40], 10, "pin")).toBeNull();
    expect(cylindricalWallPath([0, 0, 0], [0, 0, 0], [10, 0, 40], 10, "pin")).toBeNull();
    expect(cylindricalWallPath([0, 0, 0], [0, 0, 1], [10, 0, 40], 0, "pin")).toBeNull();
    expect(cylindricalWallPath([0, 0, 0], [0, 0, 1], [10, 0, 40], Number.NaN, "pin")).toBeNull();
  });
});

describe("cylindricalWallRadiusAt — what the inner wall would measure", () => {
  it("shrinks a pin and grows a hole", () => {
    expect(cylindricalWallRadiusAt(10, "pin", 2)).toBeCloseTo(8, 12);
    expect(cylindricalWallRadiusAt(6, "hole", 2)).toBeCloseTo(8, 12);
  });

  /*
   * A non-positive predicted radius cannot be DEPICTED as an inner wall (§5):
   * the caller keeps a labelled parameter control and lets the existing
   * preview/domain validation refuse the operation.
   */
  it("goes non-positive once a pin is shelled past its own radius", () => {
    expect(cylindricalWallRadiusAt(10, "pin", 10)).toBeLessThanOrEqual(0);
    expect(cylindricalWallRadiusAt(10, "pin", 12)).toBeLessThan(0);
  });
});

describe("shellThicknessWitness", () => {
  it("claims a TARGET construction and never a measured reference", () => {
    const path = planarWallPath([40, 20, 40], [1, 0, 0])!;
    const witness = shellThicknessWitness(path, 2);
    expect(witness.meaning).toBe("targetConstruction");
    expect(witness.meaning).not.toBe("measuredReference");
    expect(witness.label).toBe("Thickness target t — construction");
    expect(witness.fromMm).toEqual([40, 20, 40]);
    expect(witness.toMm).toEqual([38, 20, 40]);
  });
});
