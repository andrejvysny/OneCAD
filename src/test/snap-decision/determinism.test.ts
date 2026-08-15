/*
 * Snap determinism table (SNAP P0 gate).
 *
 * Two properties the engine must have in EVERY phase, asserted against the
 * public `computeSnap` facade so they survive the P2 internals rewrite:
 *
 *   1. CACHE EQUIVALENCE — precomputing entity-derived candidates must not
 *      change any answer. The cache exists purely to move the O(n²) scan off
 *      the pointer path.
 *   2. REPEATABILITY — the same input produces the same output, with no
 *      dependence on hidden state carried between calls.
 *
 * Entity-ORDER independence is asserted from P2 onward, where the arbitration
 * rewrite is what actually guarantees it; see `arbitration.test.ts`.
 */
import { describe, expect, it } from "vitest";
import { buildSnapCache, computeSnap, type SnapOptions } from "@/tools/sketch/snapEngine";
import { SCENARIO_BUILDERS, SNAP_SCENARIOS } from "./scenarios";

function optsFor(s: (typeof SNAP_SCENARIOS)[number], extra: Partial<SnapOptions> = {}): SnapOptions {
  return {
    gridStep: 1,
    pixelWorld: 1,
    snapPx: 8,
    enableGrid: true,
    enableGuideLines: true,
    enableGuidePoints: true,
    enableQuadrant: true,
    enableIntersection: true,
    enableOnCurve: true,
    enablePolar: s.anchor !== undefined,
    polarAnchor: s.anchor ?? null,
    polarRefDir: s.refDir ?? null,
    suppress: false,
    recentPoints: s.anchor ? [s.anchor] : [],
    ...extra,
  };
}

describe("snap determinism", () => {
  for (const s of SNAP_SCENARIOS) {
    it(`cache and non-cache agree — ${s.name}`, () => {
      const cache = buildSnapCache(s.entities);
      const plain = computeSnap(s.raw, s.entities, optsFor(s));
      const cached = computeSnap(s.raw, s.entities, optsFor(s, { cache }));
      expect(cached).toEqual(plain);
    });

    it(`repeats byte-identically — ${s.name}`, () => {
      const a = computeSnap(s.raw, s.entities, optsFor(s));
      const b = computeSnap(s.raw, s.entities, optsFor(s));
      expect(b).toEqual(a);
    });
  }

  it("Alt suppression is unconditional", () => {
    for (const s of SNAP_SCENARIOS) {
      const r = computeSnap(s.raw, s.entities, optsFor(s, { suppress: true }));
      expect(r.snapped, s.name).toBe(false);
      expect(r.point, s.name).toEqual(s.raw);
    }
  });

  it("every source toggle is independently honoured", () => {
    // Turning a source OFF may only ever REMOVE its own kinds from the answer,
    // never introduce a kind from a source that is also off. Peeled one rung at
    // a time down to "nothing left to snap to".
    const entities = [SCENARIO_BUILDERS.circle("c1", [0, 0], 10)];
    const raw = { x: 10.2, y: 0.4 };
    const base: SnapOptions = {
      gridStep: 1,
      pixelWorld: 1,
      snapPx: 8,
      enableGrid: true,
      enableGuideLines: true,
      enableGuidePoints: true,
      enableQuadrant: true,
      enableIntersection: true,
      enableOnCurve: true,
      suppress: false,
    };
    const at = (o: Partial<SnapOptions>): string =>
      computeSnap(raw, entities, { ...base, ...o }).kind;

    expect(at({})).toBe("quadrant");
    expect(at({ enableQuadrant: false })).toBe("onCurve");
    expect(at({ enableQuadrant: false, enableOnCurve: false })).toBe("alignH");
    expect(
      at({ enableQuadrant: false, enableOnCurve: false, enableGuideLines: false }),
    ).toBe("grid");
    expect(
      computeSnap(raw, entities, {
        ...base,
        enableQuadrant: false,
        enableOnCurve: false,
        enableGuideLines: false,
        enableGuidePoints: false,
        enableIntersection: false,
        enableGrid: false,
      }).snapped,
    ).toBe(false);
  });
});
