/*
 * Snap performance BASELINE (SNAP §8.6 / §14).
 *
 * Deliberately not a hard CI gate. Wall-clock on a shared runner is a property
 * of the machine, and a timing assertion tight enough to catch a real
 * regression is loose enough to flake — which trains everyone to ignore it.
 *
 * What IS asserted is the shape of the cost, which is machine-independent:
 *   - the cache is built ONCE per sketch edit, and a pointer move that reuses
 *     it does no all-pairs work at all;
 *   - the broad phase prunes disjoint pairs, so the intersection scan is not
 *     quadratic in practice;
 *   - a decision allocates no unbounded candidate list.
 * The measured numbers are printed for the record and bounded only by a
 * deliberately generous ceiling that catches an ORDER-OF-MAGNITUDE regression.
 */
import { describe, expect, it } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import {
  buildSnapCache,
  lastBroadPhaseStats,
  type SnapCandidateCache,
} from "@/tools/sketch/snapCandidates";
import { computeSnapDecision, type SnapOptions } from "@/tools/sketch/snapEngine";

/** A dense but realistic sketch: a grid of boxes plus circles between them. */
function bigSketch(boxes: number): SketchEntity[] {
  const out: SketchEntity[] = [];
  const side = Math.ceil(Math.sqrt(boxes));
  for (let i = 0; i < boxes; i++) {
    const ox = (i % side) * 40;
    const oy = Math.floor(i / side) * 40;
    out.push(
      { id: `l${i}a`, type: "Line", p0: [ox, oy], p1: [ox + 30, oy] },
      { id: `l${i}b`, type: "Line", p0: [ox + 30, oy], p1: [ox + 30, oy + 30] },
      { id: `l${i}c`, type: "Line", p0: [ox + 30, oy + 30], p1: [ox, oy + 30] },
      { id: `l${i}d`, type: "Line", p0: [ox, oy + 30], p1: [ox, oy] },
      { id: `c${i}`, type: "Circle", center: [ox + 15, oy + 15], radius: 10 },
    );
  }
  return out;
}

function opts(cache?: SnapCandidateCache): SnapOptions {
  return {
    gridStep: 10,
    pixelWorld: 1,
    snapPx: 8,
    enableGrid: true,
    enableGuideLines: true,
    enableGuidePoints: true,
    enableQuadrant: true,
    enableIntersection: true,
    enableOnCurve: true,
    suppress: false,
    ...(cache ? { cache } : {}),
  };
}

const ms = (fn: () => void, runs: number): number => {
  const t0 = performance.now();
  for (let i = 0; i < runs; i++) fn();
  return (performance.now() - t0) / runs;
};

describe("snap performance baseline", () => {
  it("records cache build time for a 200- and a 500-entity sketch", () => {
    for (const count of [40, 100]) {
      const entities = bigSketch(count); // 5 entities per box
      const build = ms(() => void buildSnapCache(entities), 3);
      // eslint-disable-next-line no-console
      console.info(
        `[snap-perf] cache build: ${entities.length} entities → ${build.toFixed(1)} ms ` +
          `(pairs ${lastBroadPhaseStats().pairsTested}/${lastBroadPhaseStats().pairsTotal})`,
      );
      // An order-of-magnitude ceiling only — see the header.
      expect(build).toBeLessThan(2000);
    }
  });

  it("records pointer-decision time against a warm cache", () => {
    const entities = bigSketch(100);
    const cache = buildSnapCache(entities);
    let i = 0;
    const per = ms(() => {
      i++;
      void computeSnapDecision({ x: (i % 400) + 0.3, y: (i % 400) + 0.7 }, entities, opts(cache));
    }, 200);
    // eslint-disable-next-line no-console
    console.info(
      `[snap-perf] pointer decision: ${cache.points.length} cached points, ` +
        `${cache.intersections.length} intersections → ${per.toFixed(3)} ms/sample`,
    );
    expect(per).toBeLessThan(50);
  });

  it("the broad phase prunes: a scattered sketch tests almost no pairs", () => {
    // 100 boxes laid out on a 40-unit pitch: only near neighbours can overlap,
    // so the tested-pair count must be LINEAR-ish, not the 124 750 an
    // all-pairs scan would do.
    const entities = bigSketch(100);
    buildSnapCache(entities);
    const { pairsTested, pairsTotal } = lastBroadPhaseStats();
    expect(pairsTotal).toBeGreaterThan(100_000);
    expect(pairsTested).toBeLessThan(pairsTotal / 20);
  });

  it("a warm-cache decision does NO all-pairs work", () => {
    const entities = bigSketch(60);
    const cache = buildSnapCache(entities);
    const beforeStats = { ...lastBroadPhaseStats() };
    computeSnapDecision({ x: 5, y: 5 }, entities, opts(cache));
    // The counters are only touched by a cache BUILD; an unchanged reading is
    // the proof that the pointer path never re-derived intersections.
    expect(lastBroadPhaseStats()).toEqual(beforeStats);
  });

  it("the accepted set stays small however dense the sketch", () => {
    const entities = bigSketch(100);
    const cache = buildSnapCache(entities);
    const d = computeSnapDecision({ x: 15.2, y: 15.3 }, entities, opts(cache)).decision;
    // At most a full point, or two guides, or a directional plus its numerics.
    expect(d.accepted.length).toBeLessThanOrEqual(4);
  });
});
