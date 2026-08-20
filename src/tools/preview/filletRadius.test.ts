import { describe, it, expect } from "vitest";
import {
  radiusFromDrag,
  formatMm,
  radiusFromValueText,
  DEFAULT_FILLET_RADIUS,
  screenDragAxis,
  signedValueFromDrag,
  SCREEN_UP_AXIS,
  clampToEdgeOpRange,
  type EdgeOpRangeGuard,
  type ScreenAxis,
  type RadiusDragOpts,
} from "./filletRadius";

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

// ── FILLET-CHAMFER-UNIFY W0 additions below (additive only) ────────────────

describe("screenDragAxis", () => {
  it("returns a unit vector from mid toward the outward point", () => {
    const axis = screenDragAxis({ x: 100, y: 100 }, { x: 100, y: 70 })!; // 30px up
    expect(Math.hypot(axis.x, axis.y)).toBeCloseTo(1, 9);
    expect(axis).toEqual({ x: 0, y: -1 });
  });

  it("removes the tangent component: a drag purely along the projected tangent is inert", () => {
    const mid: ScreenAxis = { x: 0, y: 0 };
    const out: ScreenAxis = { x: 10, y: 10 }; // outward at 45°, not parallel to the tangent
    const tan: ScreenAxis = { x: 10, y: 0 }; // tangent along +x
    const axis = screenDragAxis(mid, out, tan);
    expect(axis).not.toBeNull();
    const opts = { worldPerPx: 0.5 };
    // A drag along (mid -> tan) is, by construction, perpendicular to `axis`
    // (its tangent-aligned component was removed when deriving the axis) —
    // so it must not move the value at all.
    const delta = signedValueFromDrag(5, tan.x - mid.x, tan.y - mid.y, axis!, opts) - 5;
    expect(delta).toBeCloseTo(0, 9);
  });

  it("pixel floor: a remaining vector shorter than minPx is null (no fabricated axis)", () => {
    expect(screenDragAxis({ x: 0, y: 0 }, { x: 2, y: 0 })).toBeNull(); // 2px < default 3px
    expect(screenDragAxis({ x: 0, y: 0 }, { x: 5, y: 0 }, undefined, 10)).toBeNull(); // custom floor
  });

  it("works with no tangent argument (undefined path)", () => {
    const axis = screenDragAxis({ x: 0, y: 0 }, { x: 0, y: -10 });
    expect(axis).toEqual({ x: 0, y: -1 });
  });
});

describe("signedValueFromDrag + SCREEN_UP_AXIS — golden-pins radiusFromDrag", () => {
  // `dy` here is the RAW screen delta signedValueFromDrag takes (clientY −
  // downY); radiusFromDrag's own convention is up-positive dyPixels = -dy.
  // Reuses the exact numeric cases from the `radiusFromDrag` describe above.
  const grid: Array<{ start: number; dy: number; opts: RadiusDragOpts }> = [
    { start: 2, dy: -10, opts: { worldPerPx: 0.5 } }, // up 10px
    { start: 5, dy: 6, opts: { worldPerPx: 0.5 } }, // down 6px
    { start: 0, dy: -10, opts: { worldPerPx: 1, sensitivity: 2 } },
    { start: 10, dy: -3, opts: { worldPerPx: 2, sensitivity: 0.5 } },
  ];

  it("equals radiusFromDrag's UNCLAMPED value for a grid of positive-result drags", () => {
    for (const { start, dy, opts } of grid) {
      const gain = opts.sensitivity ?? 1;
      const unclamped = start + -dy * opts.worldPerPx * gain;
      const min = opts.min ?? 0.1;
      expect(unclamped).toBeGreaterThan(min); // grid never engages the clamp
      expect(radiusFromDrag(start, -dy, opts)).toBeCloseTo(Math.max(min, unclamped), 9);
      const signed = signedValueFromDrag(start, 0, dy, SCREEN_UP_AXIS, opts);
      expect(signed).toBeCloseTo(unclamped, 9);
      expect(signed).toBeCloseTo(radiusFromDrag(start, -dy, opts), 9);
    }
  });

  it("returns a negative result UNCLAMPED where radiusFromDrag would floor it", () => {
    const opts = { worldPerPx: 0.5, min: 0.1 };
    const dy = 100; // moved down 100px
    const signed = signedValueFromDrag(2, 0, dy, SCREEN_UP_AXIS, opts);
    expect(signed).toBeCloseTo(-48, 9);
    expect(radiusFromDrag(2, -dy, opts)).toBe(0.1); // same unclamped value, but floored
  });
});

/*
 * WP4 — the measured range guard. Five confidence rungs, five obligations. The
 * one that matters most is `coarse`: capping at `provenUpperBound` there would
 * offer a value the kernel only ever REFUSED, which is the exact class of silent
 * wrong answer the whole verb exists to remove.
 */
describe("clampToEdgeOpRange", () => {
  const guard = (over: Partial<EdgeOpRangeGuard>): EdgeOpRangeGuard => ({
    confidence: "bracketed",
    lowerBound: 0.001,
    bestKnownMax: 9.99925,
    provenUpperBound: 10,
    feasibleIntervals: [{ lower: 0.001, upper: 9.99925 }],
    ...over,
  });

  it("enforces nothing without an answer, and nothing on `none`", () => {
    expect(clampToEdgeOpRange(1000, null)).toEqual({ value: 1000, clamped: false, reason: "none" });
    expect(clampToEdgeOpRange(1000, undefined).value).toBe(1000);
    // A refusal and the mock lane both produce `none`; neither is evidence, so
    // both must behave exactly like "no answer at all".
    const none = guard({
      confidence: "none",
      lowerBound: null,
      bestKnownMax: null,
      provenUpperBound: null,
      feasibleIntervals: [],
    });
    expect(clampToEdgeOpRange(1000, none)).toEqual({ value: 1000, clamped: false, reason: "none" });
  });

  it("caps at bestKnownMax and floors at lowerBound on `bracketed`", () => {
    expect(clampToEdgeOpRange(12, guard({}))).toEqual({
      value: 9.99925,
      clamped: true,
      reason: "ceiling",
    });
    expect(clampToEdgeOpRange(2, guard({})).clamped).toBe(false);
    expect(clampToEdgeOpRange(0.0001, guard({}))).toEqual({
      value: 0.001,
      clamped: true,
      reason: "floor",
    });
    // NEVER the proven upper bound: 10 is a value the kernel refused.
    expect(clampToEdgeOpRange(12, guard({})).value).not.toBe(10);
  });

  it("raises to the floor but never caps on `lowerOnly`", () => {
    const g = guard({ confidence: "lowerOnly", provenUpperBound: null, bestKnownMax: 2.944 });
    expect(clampToEdgeOpRange(0.0001, g)).toEqual({ value: 0.001, clamped: true, reason: "floor" });
    // The ceiling is UNPROVEN — the search simply never saw a refusal — so a
    // value above the largest success is allowed through rather than forbidden.
    expect(clampToEdgeOpRange(50, g)).toEqual({ value: 50, clamped: false, reason: "none" });
  });

  it("caps at bestKnownMax, not provenUpperBound, on `coarse`", () => {
    // A truncated search: the real frontier may sit anywhere between the largest
    // success and the first refusal, so only the success may be offered.
    const g = guard({ confidence: "coarse", bestKnownMax: 4.096, provenUpperBound: 16.384 });
    const out = clampToEdgeOpRange(12, g);
    expect(out).toEqual({ value: 4.096, clamped: true, reason: "ceiling" });
    expect(out.value).not.toBe(16.384);
    expect(clampToEdgeOpRange(3, g).clamped).toBe(false);
  });

  it("honours the intervals on `nonMonotonic`", () => {
    // Feasible [1,2] and [8,9]; 4..6 was MEASURED to fail. A single ceiling at
    // bestKnownMax (9) would happily offer 5.
    const g = guard({
      confidence: "nonMonotonic",
      lowerBound: 1,
      bestKnownMax: 9,
      provenUpperBound: 10,
      feasibleIntervals: [
        { lower: 1, upper: 2 },
        { lower: 8, upper: 9 },
      ],
    });
    expect(clampToEdgeOpRange(1.5, g).clamped).toBe(false);
    expect(clampToEdgeOpRange(8.5, g).clamped).toBe(false);
    // 5 sits in the gap: pulled to the nearest PROBED endpoint, never to a
    // midpoint or an interior point of the gap — only endpoints were built.
    expect(clampToEdgeOpRange(5, g)).toEqual({ value: 2, clamped: true, reason: "interval" });
    expect(clampToEdgeOpRange(7, g)).toEqual({ value: 8, clamped: true, reason: "interval" });
    expect(clampToEdgeOpRange(20, g)).toEqual({ value: 9, clamped: true, reason: "interval" });
    expect(clampToEdgeOpRange(0.5, g)).toEqual({ value: 1, clamped: true, reason: "interval" });
  });

  it("enforces nothing when nonMonotonic carries no intervals", () => {
    const g = guard({ confidence: "nonMonotonic", feasibleIntervals: [] });
    expect(clampToEdgeOpRange(1000, g).clamped).toBe(false);
  });
});
