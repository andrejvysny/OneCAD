/*
 * ViewportMetrics — the immutable size/scale snapshot (VP-HARDENING VP03).
 *
 * The identity contract is the load-bearing part: callers decide whether to
 * touch the renderer at all by comparing `next === prev`, so a no-op input must
 * return the SAME object, not an equal one.
 */
import { describe, it, expect } from "vitest";
import { nextMetrics } from "./ViewportMetrics";
import { MAX_DPR } from "./dpr";

const base = { cssWidth: 800, cssHeight: 600, rawDpr: 1, projectionRevision: 0 };

describe("nextMetrics", () => {
  it("returns the SAME object when nothing differs", () => {
    const first = nextMetrics(null, base);
    expect(nextMetrics(first, { ...base })).toBe(first);
  });

  it("bumps the revision on any change, and only on a change", () => {
    const first = nextMetrics(null, base);
    expect(first.revision).toBe(1);

    const wider = nextMetrics(first, { ...base, cssWidth: 801 });
    expect(wider.revision).toBe(2);
    const taller = nextMetrics(wider, { ...base, cssWidth: 801, cssHeight: 601 });
    expect(taller.revision).toBe(3);
    const scaled = nextMetrics(taller, { ...base, cssWidth: 801, cssHeight: 601, rawDpr: 2 });
    expect(scaled.revision).toBe(4);
    const projected = nextMetrics(scaled, {
      cssWidth: 801,
      cssHeight: 601,
      rawDpr: 2,
      projectionRevision: 1,
    });
    expect(projected.revision).toBe(5);
    expect(nextMetrics(projected, { ...projected, rawDpr: 2 })).toBe(projected);
  });

  it("caps the DPR at MAX_DPR and sizes the buffer with the CAPPED value", () => {
    const m = nextMetrics(null, { ...base, rawDpr: 3 });
    expect(MAX_DPR).toBe(2);
    expect(m.dpr).toBe(2);
    expect(m.bufferWidth).toBe(1600);
    expect(m.bufferHeight).toBe(1200);
    // …and the CSS side is untouched by the ratio, which is the whole point.
    expect(m.cssWidth).toBe(800);
    expect(m.cssHeight).toBe(600);
  });

  it("keeps a fractional DPR below the cap, rounding only the buffer", () => {
    const m = nextMetrics(null, { ...base, cssWidth: 801, rawDpr: 1.5 });
    expect(m.dpr).toBe(1.5);
    expect(m.bufferWidth).toBe(1201); // floor(801 * 1.5) = 1201, as WebGLRenderer.setSize does
    expect(m.bufferHeight).toBe(900);
  });

  it("a raw DPR at or above the cap collapses to ONE snapshot", () => {
    const capped = nextMetrics(null, { ...base, rawDpr: 2 });
    expect(nextMetrics(capped, { ...base, rawDpr: 4 })).toBe(capped);
  });

  it("clamps zero, negative and non-finite sizes to 1", () => {
    const zero = nextMetrics(null, { ...base, cssWidth: 0, cssHeight: 0 });
    expect([zero.cssWidth, zero.cssHeight]).toEqual([1, 1]);
    expect([zero.bufferWidth, zero.bufferHeight]).toEqual([1, 1]);

    const bad = nextMetrics(null, { ...base, cssWidth: Number.NaN, cssHeight: -5 });
    expect([bad.cssWidth, bad.cssHeight]).toEqual([1, 1]);

    const noDpr = nextMetrics(null, { ...base, rawDpr: 0 });
    expect(noDpr.dpr).toBe(1);
  });

  it("is frozen — a snapshot cannot be mutated behind its readers' backs", () => {
    const m = nextMetrics(null, base);
    expect(Object.isFrozen(m)).toBe(true);
  });
});
