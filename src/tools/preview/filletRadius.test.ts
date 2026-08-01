import { describe, it, expect } from "vitest";
import { radiusFromDrag, formatMm, radiusFromValueText, DEFAULT_FILLET_RADIUS } from "./filletRadius";

describe("radiusFromDrag", () => {
  it("grows the radius 1:1 with world units when dragging up", () => {
    // dy = downY - currentY, so an upward drag is positive.
    expect(radiusFromDrag(2, 10, { worldPerPx: 0.5 })).toBeCloseTo(7, 9);
  });

  it("shrinks the radius when dragging down", () => {
    expect(radiusFromDrag(5, -6, { worldPerPx: 0.5 })).toBeCloseTo(2, 9);
  });

  it("clamps to the minimum radius", () => {
    expect(radiusFromDrag(2, -100, { worldPerPx: 0.5, min: 0.1 })).toBe(0.1);
  });

  it("applies the sensitivity gain", () => {
    expect(radiusFromDrag(0, 10, { worldPerPx: 1, sensitivity: 2 })).toBeCloseTo(20, 9);
  });
});

describe("formatMm", () => {
  // W2-A: the chip now shares the sketch-dimension formatter (≤3dp, trailing
  // zeros trimmed) instead of a private toFixed(1) — one number, one rendering.
  it("formats a value with the shared length formatter", () => {
    expect(formatMm(2)).toBe("2 mm");
    expect(formatMm(83.25)).toBe("83.25 mm");
    expect(formatMm(100)).toBe("100 mm");
    expect(formatMm(12.3456)).toBe("12.346 mm");
  });
});

describe("radiusFromValueText (fillet re-edit seed)", () => {
  it("parses a fillet feature's display text back to a radius", () => {
    expect(radiusFromValueText("2.0 mm")).toBe(2);
    expect(radiusFromValueText("12.5 mm")).toBe(12.5);
  });

  it("falls back to the default for non-numeric / non-positive text", () => {
    expect(radiusFromValueText("")).toBe(DEFAULT_FILLET_RADIUS);
    expect(radiusFromValueText("—")).toBe(DEFAULT_FILLET_RADIUS);
    expect(radiusFromValueText("0 mm")).toBe(DEFAULT_FILLET_RADIUS);
    expect(radiusFromValueText("bad", 7)).toBe(7);
  });

  /*
   * W2-A GUARD. `radiusFromValueText` seeds the value a parametric RE-EDIT
   * opens with, and it is fed TWO different producers:
   *   - the RUST-composed feature `valueText` ("2.0 mm", dto.rs
   *     feature_value_text) — unchanged by this wave, pinned above;
   *   - this module's own `formatMm`, which W2-A re-pointed at `formatLength`.
   * A formatter change that broke either parse would silently seed a re-edit
   * with the WRONG radius, so both directions are asserted as identities.
   */
  it("round-trips every value formatMm can produce (re-edit seed identity)", () => {
    for (const v of [0.1, 1, 2, 12.5, 83.25, 100, 1000, 12.346]) {
      expect(radiusFromValueText(formatMm(v))).toBeCloseTo(v, 9);
    }
  });

  it("still round-trips the RUST valueText form (fixed 1dp)", () => {
    for (const v of [2, 12.5, 83.3, 100]) {
      expect(radiusFromValueText(`${v.toFixed(1)} mm`)).toBeCloseTo(v, 9);
    }
  });
});
