import { describe, it, expect } from "vitest";
import {
  formatMm,
  radiusFromValueText,
  DEFAULT_FILLET_RADIUS,
  clampToEdgeOpRange,
  edgeParameterPath,
  edgeParameterWitness,
  type EdgeOpRangeGuard,
} from "./filletRadius";
import { handlePointAt } from "./handleProjection";
import type { Vec3 } from "./depthProjection";

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

/*
 * H8 — the Fillet/Chamfer PARAMETER construction
 * (docs/design/astra/modeling-handle-attachment.md §5, §7).
 *
 * A generic geometric fillet attachment is not available: for a certified
 * convex 90° corner the true blend midpoint moves −(√2−1)·q·b, and that sign
 * REVERSES on the concave corner. So the handle is the parameter point
 * H(q) = E + q·b and the witness says so — it measures the construction's
 * length, not a blend contact.
 */
describe("edgeParameterPath", () => {
  const E: Vec3 = [40, 40, 20];
  const B: Vec3 = [Math.SQRT1_2, Math.SQRT1_2, 0];

  it("is H(q) = E + q·b, anchored at q = 0", () => {
    const path = edgeParameterPath(E, B)!;
    expect(path.q0Mm).toBe(0);
    expect(path.point0Mm).toEqual(E);
    expect(path.dPointDValue[0]).toBeCloseTo(Math.SQRT1_2, 12);
    expect(path.dPointDValue[1]).toBeCloseTo(Math.SQRT1_2, 12);
    expect(path.dPointDValue[2]).toBeCloseTo(0, 12);
  });

  it("§6 convex box edge: q = 2 mm puts the handle at (41.414214, 41.414214, 20)", () => {
    const h = handlePointAt(edgeParameterPath(E, B)!, 2);
    expect(h[0]).toBeCloseTo(41.414214, 6);
    expect(h[1]).toBeCloseTo(41.414214, 6);
    expect(h[2]).toBeCloseTo(20, 12);
  });

  it("is NOT the blend midpoint: that moves −0.828427·b, the opposite way", () => {
    const h = handlePointAt(edgeParameterPath(E, B)!, 2);
    const blendMidpoint = 39.414214; // E − (√2−1)·q·b, the real contact at r = 2
    expect(h[0]).not.toBeCloseTo(blendMidpoint, 3);
    // …and the parameter handle travels +2 mm along b, the full construction.
    expect(Math.hypot(h[0] - E[0], h[1] - E[1], h[2] - E[2])).toBeCloseTo(2, 12);
  });

  it("§6 four-edge tangent chain: the 45° representative reaches (15.5563, 15.5563, 20)", () => {
    const anchor: Vec3 = [20 * Math.SQRT1_2, 20 * Math.SQRT1_2, 20];
    const h = handlePointAt(edgeParameterPath(anchor, [Math.SQRT1_2, Math.SQRT1_2, 0])!, 2);
    expect(h[0]).toBeCloseTo(15.556349, 6);
    expect(h[1]).toBeCloseTo(15.556349, 6);
    expect(h[2]).toBeCloseTo(20, 12);
  });

  it("normalizes a non-unit outward, and refuses a degenerate one", () => {
    const path = edgeParameterPath(E, [0, 0, 7])!;
    expect(path.dPointDValue).toEqual([0, 0, 1]);
    expect(edgeParameterPath(E, [0, 0, 0])).toBeNull();
    expect(edgeParameterPath(E, [NaN, 0, 0])).toBeNull();
    expect(edgeParameterPath([NaN, 0, 0], [0, 0, 1])).toBeNull();
  });
});

describe("edgeParameterWitness", () => {
  const path = edgeParameterPath([40, 40, 20], [Math.SQRT1_2, Math.SQRT1_2, 0])!;

  it("is a PARAMETER construction spanning E → H(q), never a measured reference", () => {
    const w = edgeParameterWitness("Fillet", path, 2, 1);
    expect(w.meaning).toBe("parameterConstruction");
    expect(w.label).toBe("Radius parameter");
    expect(w.fromMm).toEqual([40, 40, 20]);
    expect(w.toMm[0]).toBeCloseTo(41.414214, 6);
  });

  it("names the Chamfer quantity by its own noun", () => {
    expect(edgeParameterWitness("Chamfer", path, 2, 1).label).toBe("Chamfer distance parameter");
  });

  it("says how many edges share the parameter when the closure grew", () => {
    expect(edgeParameterWitness("Fillet", path, 2, 4).label).toBe("Radius parameter · shared by 4 edges");
    expect(edgeParameterWitness("Chamfer", path, 2, 2).label).toBe(
      "Chamfer distance parameter · shared by 2 edges",
    );
  });
});
