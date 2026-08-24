/*
 * Snap arbitration: scoring, composition, hysteresis, determinism (SNAP P2).
 *
 * These are the cases the legacy single-winner ladder could not express or got
 * wrong. Each one names the defect it closes.
 */
import { describe, expect, it } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import { computeSnapDecision, type SnapOptions } from "@/tools/sketch/snapEngine";
import { buildSnapCache } from "@/tools/sketch/snapCandidates";
import {
  emptyLatch,
  releaseRadiusPx,
  type SnapDecision,
  type SnapLatch,
} from "@/tools/sketch/snapTypes";
import { dimFrame } from "@/tools/sketch/liveDimFrames";
import { permutations, SCENARIO_BUILDERS, SNAP_SCENARIOS, scenario } from "./scenarios";

/** Isotropic 1px/unit, everything on, no numeric — the spatial-only baseline. */
function opts(over: Partial<SnapOptions> = {}): SnapOptions {
  return {
    gridStep: 10,
    pixelWorld: 1,
    snapPx: 8,
    enableGrid: false,
    enableGuideLines: true,
    enableGuidePoints: true,
    enableQuadrant: true,
    enableIntersection: true,
    enableOnCurve: true,
    suppress: false,
    ...over,
  };
}

const decide = (
  raw: { x: number; y: number },
  entities: SketchEntity[],
  over: Partial<SnapOptions> = {},
): SnapDecision => computeSnapDecision(raw, entities, opts(over)).decision;

const kinds = (d: SnapDecision): string[] => d.accepted.map((c) => c.kind).sort();

describe("distance beats the type ladder", () => {
  it("a NEAR midpoint beats a FAR endpoint (the ladder's core defect)", () => {
    const s = scenario("endpoint-vs-midpoint-near");
    const d = decide(s.raw, s.entities);
    // Legacy: endpoint is tier 0, so l2's endpoint 4.5px away won outright.
    expect(d.primaryKind).toBe("midpoint");
  });

  it("an equal-distance endpoint still beats a midpoint, by bias alone", () => {
    const s = scenario("endpoint-vs-midpoint-equal");
    expect(decide(s.raw, s.entities).primaryKind).toBe("endpoint");
  });

  it("the origin captures absolutely inside its 2px core", () => {
    const s = scenario("origin-core-capture");
    expect(decide(s.raw, s.entities).primaryKind).toBe("origin");
  });

  it("…and is an ordinary candidate outside it", () => {
    const s = scenario("origin-outside-core-loses-to-endpoint");
    expect(decide(s.raw, s.entities).primaryKind).toBe("endpoint");
  });

  it("an equally close NAMED point beats on-curve (the +0.75 penalty)", () => {
    const s = scenario("on-curve-loses-to-equally-close-named-point");
    expect(decide(s.raw, s.entities).primaryKind).toBe("endpoint");
  });

  it("no semantic bias exceeds 2px in magnitude", () => {
    for (const s of SNAP_SCENARIOS) {
      for (const c of decide(s.raw, s.entities).accepted) {
        expect(Math.abs(c.semanticBiasPx), `${s.name}/${c.id}`).toBeLessThanOrEqual(2);
      }
    }
  });
});

describe("composition", () => {
  it("two guides are accepted TOGETHER, one claiming x and one claiming y", () => {
    const s = scenario("two-guides-compose");
    const d = decide(s.raw, s.entities);
    expect(d.accepted).toHaveLength(2);
    expect(d.accepted.map((c) => [...c.claims]).flat().sort()).toEqual(["x", "y"]);
    expect(d.point).toEqual({ x: 25, y: 12 });
    expect(d.guides).toHaveLength(2);
    // Two UNRELATED references: naming that "Aligned" would claim a 2D
    // coincidence that does not exist.
    expect(d.primaryKind).not.toBe("alignHV");
    expect(d.label).toBe("Vertical · Horizontal");
  });

  it("two guides through the SAME point are 'Aligned'", () => {
    const d = decide({ x: 10.1, y: 20.1 }, [], { recentPoints: [{ x: 10, y: 20 }] });
    expect(d.primaryKind).toBe("alignHV");
    expect(d.label).toBe("Aligned");
    expect(d.point).toEqual({ x: 10, y: 20 });
  });

  it("a polar ray composes with a rounded LENGTH", () => {
    const anchors = [{ x: 0, y: 0 }];
    const d = decide(
      { x: 17.6, y: 17.72 },
      [],
      {
        enablePolar: true,
        polarAnchor: anchors[0],
        anchors: [{ point: anchors[0], key: "g0:0" }],
        frame: dimFrame("line", anchors),
        toolId: "line",
        toolAnchors: anchors,
        quantum: { length: 0.5, angle: 1 },
      },
    );
    const sources = d.accepted.map((c) => c.source).sort();
    expect(sources).toEqual(["numeric", "polar"]);
    expect(d.point.x).toBeCloseTo(d.point.y, 6); // on the 45° ray
    const len = Math.hypot(d.point.x, d.point.y);
    expect(len / 0.5).toBeCloseTo(Math.round(len / 0.5), 6); // …at a round length
  });

  it("a FULL POINT suppresses every numeric candidate", () => {
    const anchors = [{ x: 0, y: 0 }];
    const line = SCENARIO_BUILDERS.line("l1", [30, 0.2], [40, 0.2]);
    const d = decide({ x: 30.1, y: 0.3 }, [line], {
      frame: dimFrame("line", anchors),
      toolId: "line",
      toolAnchors: anchors,
      quantum: { length: 0.5, angle: 1 },
    });
    expect(d.primaryKind).toBe("endpoint");
    expect(d.accepted.every((c) => c.source !== "numeric")).toBe(true);
    expect(d.point).toEqual({ x: 30, y: 0.2 });
    expect(d.rejected.some((r) => r.reason === "claim-conflict")).toBe(true);
  });

  it("a TYPED lock outranks a geometry candidate that would move its field", () => {
    const anchors = [{ x: 0, y: 0 }];
    const line = SCENARIO_BUILDERS.line("l1", [30, 0.2], [40, 0.2]);
    const d = decide({ x: 30.1, y: 0.3 }, [line], {
      frame: dimFrame("line", anchors),
      toolId: "line",
      toolAnchors: anchors,
      locks: { length: 25 },
    });
    expect(Math.hypot(d.point.x, d.point.y)).toBeCloseTo(25, 9);
    expect(d.rejected.some((r) => r.reason === "higher-authority")).toBe(true);
  });

  it("a rectangle's x guide fixes the WIDTH and leaves the height to round", () => {
    const anchors = [{ x: 0, y: 0 }];
    const d = decide({ x: 25.2, y: 13.3 }, [], {
      recentPoints: [{ x: 25, y: 90 }],
      frame: dimFrame("rect", anchors),
      toolId: "rect",
      toolAnchors: anchors,
      quantum: { length: 1, angle: 1 },
    });
    expect(d.point.x).toBe(25); // the guide pinned it
    expect(d.point.y).toBe(13); // …and the free extent rounded
  });

  it("a guide + rounded length with NO real intersection rejects the numeric", () => {
    const anchors = [{ x: 0, y: 0 }];
    // Guide at x = 60, but the rounded length is ~20 — the circle of radius 20
    // about the anchor never reaches x = 60.
    const d = decide({ x: 19.9, y: 3 }, [], {
      recentPoints: [{ x: 60, y: 0 }],
      frame: dimFrame("line", anchors),
      toolId: "line",
      toolAnchors: anchors,
      quantum: { length: 5, angle: 1 },
    });
    // The impossible pair simply never forms; the decision is still valid.
    expect(Number.isFinite(d.point.x)).toBe(true);
    expect(Number.isFinite(d.point.y)).toBe(true);
  });
});

describe("grid is cell-relative", () => {
  it("captures near a node and NOT mid-cell", () => {
    // Away from the origin, whose 2px capture core would answer instead.
    const near = decide({ x: 20.4, y: 20.4 }, [], { enableGrid: true, gridStep: 10 });
    expect(near.primaryKind).toBe("grid");
    const mid = decide({ x: 25, y: 25 }, [], { enableGrid: true, gridStep: 10 });
    expect(mid.primaryKind).not.toBe("grid");
  });

  it("is inert when the projected cell is sub-pixel", () => {
    // 0.05 units at 1px/unit = 0.05px — a grid nobody can see or aim at.
    const d = decide({ x: 20.4, y: 20.4 }, [], { enableGrid: true, gridStep: 0.05 });
    expect(d.primaryKind).not.toBe("grid");
  });
});

describe("hysteresis", () => {
  // Two endpoints 6 units apart on the x axis. Guides are OFF throughout: every
  // reference here shares y = 0, so a zero-miss y guide would answer instead and
  // the retention rule under test would never be exercised.
  const line = SCENARIO_BUILDERS.line("l1", [0, 0], [20, 0]);
  const other = SCENARIO_BUILDERS.line("l2", [26, 0], [26, 20]);
  const entities = [line, other];
  const END = "point:l1.End:endpoint";
  const OTHER = "point:l2.Start:endpoint";
  const bare = { enableGuideLines: false };

  const latchOn = (id: string): SnapLatch => ({
    primaryId: id,
    acceptedIds: [id],
    challengerId: null,
    challengerFrames: 0,
  });

  it("retains a latched target inside the release radius", () => {
    // l1 ALONE: nothing else may be in reach, or the retention rule is not what
    // the assertion measures. 10px from l1's End — outside acquire (8), inside
    // release (12).
    const held = computeSnapDecision(
      { x: 30, y: 0 },
      [line],
      opts({ ...bare, latch: latchOn(END) }),
    );
    expect(held.decision.primaryId).toBe(END);
    // With NO latch the same sample finds nothing.
    const cold = computeSnapDecision(
      { x: 30, y: 0 },
      [line],
      opts({ ...bare, latch: emptyLatch() }),
    );
    expect(cold.decision.primaryId).toBeNull();
  });

  it("drops a latched target once it leaves the release radius", () => {
    const d = computeSnapDecision(
      { x: 40, y: 0 },
      [line],
      opts({ ...bare, latch: latchOn(END) }),
    ).decision;
    expect(d.primaryId).toBeNull();
    expect(d.rejected.some((r) => r.reason === "outside-release-radius")).toBe(true);
  });

  it("release radii are 8 / 12 / 17 for S / M / L", () => {
    expect(releaseRadiusPx(5)).toBe(8);
    expect(releaseRadiusPx(8)).toBe(12);
    expect(releaseRadiusPx(12)).toBe(17);
  });

  it("a challenger needs a 2px advantage for TWO consecutive frames", () => {
    let latch = latchOn(END);
    const at = (x: number): SnapDecision => {
      const out = computeSnapDecision({ x, y: 0 }, entities, opts({ ...bare, latch }));
      latch = out.latch;
      return out.decision;
    };
    // At x = 25.6 the other endpoint is 5.2px cheaper — well past the 2px bar.
    const first = at(25.6);
    expect(first.primaryId, "one frame is not enough to switch").toBe(END);
    expect(latch.challengerId).toBe(OTHER);
    expect(latch.challengerFrames).toBe(1);
    const second = at(25.6);
    expect(second.primaryId, "the second consecutive frame switches").toBe(OTHER);
  });

  it("a marginal challenger never switches, however long it leads", () => {
    let latch = latchOn(END);
    const at = (x: number): SnapDecision => {
      const out = computeSnapDecision({ x, y: 0 }, entities, opts({ ...bare, latch }));
      latch = out.latch;
      return out.decision;
    };
    // Midway: the other endpoint is only ~1px cheaper — under the 2px bar.
    for (let i = 0; i < 5; i++) expect(at(23.5).primaryId).toBe(END);
    expect(latch.challengerId).toBeNull();
  });

  it("a challenger's streak RESETS when it stops leading", () => {
    let latch = latchOn(END);
    const at = (x: number): void => {
      latch = computeSnapDecision({ x, y: 0 }, entities, opts({ ...bare, latch })).latch;
    };
    at(25.6); // challenger frame 1
    expect(latch.challengerFrames).toBe(1);
    at(20.1); // back on the retained target — the streak is broken
    expect(latch.challengerFrames).toBe(0);
    at(25.6); // …so this is frame 1 again, not frame 2
    expect(latch.challengerFrames).toBe(1);
  });
});

describe("determinism", () => {
  it("entity ORDER cannot change the decision", () => {
    for (const s of SNAP_SCENARIOS) {
      if (s.entities.length < 2) continue;
      const base = decide(s.raw, s.entities, { enableGrid: true });
      for (const perm of permutations(s.entities, 12)) {
        const d = decide(s.raw, perm as SketchEntity[], { enableGrid: true });
        expect(d.point, `${s.name}`).toEqual(base.point);
        expect(d.primaryId, `${s.name}`).toEqual(base.primaryId);
        expect(kinds(d), `${s.name}`).toEqual(kinds(base));
      }
    }
  });

  it("cache and non-cache produce identical candidates", () => {
    for (const s of SNAP_SCENARIOS) {
      const cache = buildSnapCache(s.entities);
      const plain = decide(s.raw, s.entities, { enableGrid: true });
      const cached = decide(s.raw, s.entities, { enableGrid: true, cache });
      expect(cached.point, s.name).toEqual(plain.point);
      expect(
        cached.accepted.map((c) => c.id),
        s.name,
      ).toEqual(plain.accepted.map((c) => c.id));
    }
  });

  it("candidate ids embed no raw coordinates", () => {
    for (const s of SNAP_SCENARIOS) {
      for (const c of decide(s.raw, s.entities, { enableGrid: true }).accepted) {
        // Ids may carry INTEGER grid indices and canonical angles; a long
        // decimal is the signature of a coordinate having leaked in.
        expect(c.id, `${s.name}/${c.id}`).not.toMatch(/\d+\.\d{3,}/);
      }
    }
  });

  it("a dense junction binds by geometry, not by array order", () => {
    const s = scenario("dense-junction-determinism");
    const ids = new Set<string>();
    for (const perm of permutations(s.entities, 12)) {
      ids.add(decide(s.raw, perm as SketchEntity[]).primaryId ?? "");
    }
    expect(ids.size).toBe(1);
  });
});

describe("degenerate inputs", () => {
  it("Alt suppresses every candidate and every rounding", () => {
    const anchors = [{ x: 0, y: 0 }];
    const d = decide({ x: 10.4, y: 0.1 }, [SCENARIO_BUILDERS.line("l1", [10, 0], [20, 0])], {
      suppress: true,
      frame: dimFrame("line", anchors),
      toolId: "line",
      toolAnchors: anchors,
      quantum: { length: 1, angle: 1 },
    });
    expect(d.accepted).toEqual([]);
    expect(d.point).toEqual({ x: 10.4, y: 0.1 });
    expect(d.snapped).toBe(false);
  });

  it("a numeric-only decision is not reported as a snap", () => {
    const anchors = [{ x: 0, y: 0 }];
    const d = decide({ x: 22.3, y: 0 }, [], {
      frame: dimFrame("line", anchors),
      toolId: "line",
      toolAnchors: anchors,
      quantum: { length: 1, angle: 1 },
    });
    expect(d.snapped).toBe(false);
    expect(d.label).toBeNull();
    expect(d.accepted.every((c) => c.source === "numeric")).toBe(true);
    expect(d.point.x).toBeCloseTo(22, 9);
  });

  it("every rejected candidate carries a reason", () => {
    const d = decide({ x: 3, y: 3 }, [SCENARIO_BUILDERS.line("l1", [0, 0], [20, 0])], {
      enableGrid: true,
    });
    for (const r of d.rejected) {
      expect(r.reason, r.candidateId).toBeTruthy();
      expect(r.candidateId).toBeTruthy();
    }
  });
});

// ── grid-crossing shadow (user report: "snaps to grid line NEAR the crossing") ──
//
// An alignment guide whose row/column IS a grid line composes with numeric
// rounding into an on-line point at near-zero cost, out-scoring the crossing a
// few px away. The shadow rule makes the crossing win exactly there — and
// nowhere else: off-grid guides, geometry points, and guide pairs landing ON
// the crossing are all untouched.
describe("grid-crossing shadow", () => {
  const gridOn = { enableGrid: true, gridStep: 10 } as const;

  it("the crossing beats a guide+rounding composite sitting ON its own column", () => {
    const anchors = [{ x: 0, y: 0 }];
    // Guide column x=20 IS a grid column; cursor 3px below the (20,30) crossing.
    const d = decide({ x: 20.3, y: 27 }, [], {
      ...gridOn,
      recentPoints: [{ x: 20, y: 90 }],
      frame: dimFrame("rect", anchors),
      toolId: "rect",
      toolAnchors: anchors,
      quantum: { length: 1, angle: 1 },
    });
    expect(d.primaryKind).toBe("grid");
    expect(d.point).toEqual({ x: 20, y: 30 });
    expect(d.rejected.some((r) => r.reason === "grid-crossing-shadow")).toBe(true);
  });

  it("an OFF-grid guide composite is untouched by the rule", () => {
    const anchors = [{ x: 0, y: 0 }];
    // Guide column x=22 is NOT a grid line: the composite stays the winner.
    const d = decide({ x: 22.3, y: 27 }, [], {
      ...gridOn,
      recentPoints: [{ x: 22, y: 90 }],
      frame: dimFrame("rect", anchors),
      toolId: "rect",
      toolAnchors: anchors,
      quantum: { length: 1, angle: 1 },
    });
    expect(d.point.x).toBe(22);
    expect(d.primaryKind).not.toBe("grid");
    expect(d.rejected.some((r) => r.reason === "grid-crossing-shadow")).toBe(false);
  });

  it("a geometry point on the crossing's row is exempt from the shadow", () => {
    const line = SCENARIO_BUILDERS.line("l1", [20.5, 30], [40, 30]);
    const d = decide({ x: 20.4, y: 30.1 }, [line], gridOn);
    expect(d.primaryKind).toBe("endpoint");
    expect(d.point).toEqual({ x: 20.5, y: 30 });
  });

  it("a guide PAIR landing exactly ON the crossing is kept, not shadowed", () => {
    const d = decide({ x: 20.3, y: 30.2 }, [], {
      ...gridOn,
      recentPoints: [
        { x: 20, y: 90 },
        { x: 90, y: 30 },
      ],
    });
    expect(d.point).toEqual({ x: 20, y: 30 });
    expect(d.rejected.some((r) => r.reason === "grid-crossing-shadow")).toBe(false);
  });
});
