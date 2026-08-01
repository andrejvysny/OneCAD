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
  formatThickness,
  thicknessFromValueText,
} from "./shellThickness";

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
