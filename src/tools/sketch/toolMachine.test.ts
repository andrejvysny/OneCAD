import { describe, it, expect } from "vitest";
import {
  lineTool,
  rectTool,
  centerRectTool,
  circleTool,
  ellipseTool,
  ellipseDraft,
  arcTool,
  pointTool,
  polygonTool,
  slotTool,
  clampSides,
  draftToEntityFields,
  perpDistance,
  polygonVertices,
  resolveToolConstraints,
  DEFAULT_POLYGON_SIDES,
  type DraftEntity,
  type ToolMachine,
  type ToolStep,
} from "./toolMachine";
import type { SketchConstraint, SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";

function run(m: ToolMachine, events: Array<["click" | "move", Point2] | ["esc"]>): ToolStep[] {
  let state = m.init();
  const steps: ToolStep[] = [];
  for (const e of events) {
    const ev = e[0] === "esc" ? { kind: "esc" as const } : { kind: e[0], pt: e[1] as Point2 };
    const step = m.step(state, ev);
    state = step.state;
    steps.push(step);
  }
  return steps;
}

describe("lineTool — click-click chaining, Esc ends chain", () => {
  it("commits a segment on the second click and keeps chaining", () => {
    const steps = run(lineTool, [
      ["click", { x: 0, y: 0 }],
      ["move", { x: 40, y: 0 }],
      ["click", { x: 40, y: 0 }],
      ["click", { x: 40, y: 20 }],
    ]);
    expect(steps[0].committed).toBeUndefined();
    expect(steps[1].preview).toEqual([{ type: "Line", p0: { x: 0, y: 0 }, p1: { x: 40, y: 0 } }]);
    expect(steps[2].committed).toEqual([{ type: "Line", p0: { x: 0, y: 0 }, p1: { x: 40, y: 0 } }]);
    expect(steps[3].committed).toEqual([{ type: "Line", p0: { x: 40, y: 0 }, p1: { x: 40, y: 20 } }]);
  });

  it("Esc ends the chain (done, state reset)", () => {
    const steps = run(lineTool, [["click", { x: 0, y: 0 }], ["esc"]]);
    expect(steps[1].done).toBe(true);
    expect(steps[1].state.anchors).toEqual([]);
    expect(steps[1].preview).toEqual([]);
  });
});

describe("rectTool — 2 corner clicks → 4 lines", () => {
  it("commits four lines forming the rectangle", () => {
    const steps = run(rectTool, [["click", { x: 0, y: 0 }], ["click", { x: 40, y: 20 }]]);
    expect(steps[1].done).toBe(true);
    const c = steps[1].committed!;
    expect(c).toHaveLength(4);
    expect(c).toEqual([
      { type: "Line", p0: { x: 0, y: 0 }, p1: { x: 40, y: 0 } },
      { type: "Line", p0: { x: 40, y: 0 }, p1: { x: 40, y: 20 } },
      { type: "Line", p0: { x: 40, y: 20 }, p1: { x: 0, y: 20 } },
      { type: "Line", p0: { x: 0, y: 20 }, p1: { x: 0, y: 0 } },
    ]);
  });

  it("ignores a degenerate second corner (shared axis)", () => {
    const steps = run(rectTool, [["click", { x: 0, y: 0 }], ["click", { x: 40, y: 0 }]]);
    expect(steps[1].committed).toBeUndefined();
  });

  it("previews the rectangle while moving", () => {
    const steps = run(rectTool, [["click", { x: 0, y: 0 }], ["move", { x: 10, y: 5 }]]);
    expect(steps[1].preview).toHaveLength(4);
  });
});

describe("circleTool — center → radius", () => {
  it("commits a circle with the dragged radius", () => {
    const steps = run(circleTool, [["click", { x: 0, y: 0 }], ["click", { x: 3, y: 4 }]]);
    expect(steps[1].committed).toEqual([{ type: "Circle", center: { x: 0, y: 0 }, radius: 5 }]);
  });
});

describe("arcTool — center → start → end (center-start-end)", () => {
  it("locks the radius from center→start and projects end onto the circle", () => {
    const steps = run(arcTool, [
      ["click", { x: 0, y: 0 }],
      ["click", { x: 10, y: 0 }], // radius 10, start at angle 0
      ["click", { x: 0, y: 20 }], // end direction +Y ⇒ projected to (0,10)
    ]);
    const arc = steps[2].committed![0];
    expect(arc.type).toBe("Arc");
    expect(arc.radius).toBeCloseTo(10);
    expect(arc.start).toEqual({ x: 10, y: 0 });
    expect(arc.end!.x).toBeCloseTo(0);
    expect(arc.end!.y).toBeCloseTo(10);
  });
});

describe("ellipseTool — centre → major-axis end → minor extent (3 clicks)", () => {
  const C = { x: 0, y: 0 };

  it("previews a CIRCLE while the major axis is being aimed (oracle parity)", () => {
    const steps = run(ellipseTool, [
      ["click", C],
      ["move", { x: 30, y: 40 }],
    ]);
    expect(steps[0].preview).toEqual([]);
    expect(steps[1].preview).toEqual([{ type: "Circle", center: C, radius: 50 }]);
    expect(steps[1].committed).toBeUndefined();
  });

  it("seeds minor = major·0.5 in the preview emitted by the SECOND click", () => {
    const steps = run(ellipseTool, [
      ["click", C],
      ["click", { x: 10, y: 0 }],
    ]);
    expect(steps[1].committed).toBeUndefined();
    expect(steps[1].state.anchors).toHaveLength(2);
    expect(steps[1].preview).toEqual([
      { type: "Ellipse", center: C, majorR: 10, minorR: 5, rotation: 0 },
    ]);
  });

  it("tracks the minor as the PERPENDICULAR distance to the major axis", () => {
    const steps = run(ellipseTool, [
      ["click", C],
      ["click", { x: 20, y: 0 }], // major along +X, a = 20
      ["move", { x: 5, y: -7 }], // perpendicular distance 7 (sign-free)
    ]);
    expect(steps[2].preview).toEqual([
      { type: "Ellipse", center: C, majorR: 20, minorR: 7, rotation: 0 },
    ]);
  });

  it("commits the ellipse on the third click and resets", () => {
    const steps = run(ellipseTool, [
      ["click", C],
      ["click", { x: 0, y: 12 }], // major along +Y ⇒ rotation π/2
      ["click", { x: 4, y: 3 }], // perpendicular distance 4
    ]);
    const [committed] = steps[2].committed!;
    expect(committed.type).toBe("Ellipse");
    expect(committed.center).toEqual(C);
    expect(committed.majorR).toBe(12);
    expect(committed.minorR).toBe(4);
    expect(committed.rotation).toBeCloseTo(Math.PI / 2, 12);
    expect(steps[2].done).toBe(true);
    expect(steps[2].state.anchors).toEqual([]);
    expect(steps[2].preview).toEqual([]);
  });

  it("applies the swap-normalization LIVE — the preview equals what commits", () => {
    // Major aimed along +X with a = 10; the cursor is 25 away perpendicular, so the
    // "minor" exceeds the "major" and the axes swap (oracle EllipseTool.cpp:151-169).
    const armedSteps = run(ellipseTool, [
      ["click", C],
      ["click", { x: 10, y: 0 }],
    ]);
    const armed = armedSteps[armedSteps.length - 1].state;
    const far = { x: 3, y: 25 };
    const moved = ellipseTool.step(armed, { kind: "move", pt: far });
    const clicked = ellipseTool.step(armed, { kind: "click", pt: far });
    const preview = moved.preview[0];
    expect(preview.majorR).toBe(25);
    expect(preview.minorR).toBe(10);
    expect(preview.rotation).toBeCloseTo(Math.PI / 2, 12);
    // Byte-for-byte identical: what the user sees is exactly what lands.
    expect(clicked.committed).toEqual([preview]);
  });

  it("never authors tool constraints (the ellipse CURVE takes none)", () => {
    const steps = run(ellipseTool, [
      ["click", C],
      ["click", { x: 8, y: 0 }],
      ["click", { x: 0, y: 3 }],
    ]);
    expect(steps[2].committedConstraints).toBeUndefined();
  });

  it("Esc resets from any phase", () => {
    for (const clicks of [1, 2]) {
      let state = ellipseTool.init();
      for (let i = 0; i < clicks; i++) {
        state = ellipseTool.step(state, { kind: "click", pt: { x: 10 * i, y: 0 } }).state;
      }
      const esc = ellipseTool.step(state, { kind: "esc" });
      expect(esc.state.anchors).toEqual([]);
      expect(esc.done).toBe(true);
      expect(esc.committed).toBeUndefined();
    }
  });

  it("normalizes the rotation into [0, 2π) for a major axis aimed below +X", () => {
    const steps = run(ellipseTool, [
      ["click", C],
      ["click", { x: 10, y: -10 }], // atan2 ⇒ −π/4
      ["click", { x: 1, y: 1 }],
    ]);
    const rot = steps[2].committed![0].rotation!;
    expect(rot).toBeGreaterThanOrEqual(0);
    expect(rot).toBeLessThan(Math.PI * 2);
    expect(rot).toBeCloseTo(Math.PI * 2 - Math.PI / 4, 12);
  });
});

describe("ellipseDraft", () => {
  it("derives majorR/rotation from the centre→endpoint vector", () => {
    const d = ellipseDraft({ x: 2, y: 2 }, { x: 2, y: 8 }, 3);
    expect(d).toEqual({
      type: "Ellipse",
      center: { x: 2, y: 2 },
      majorR: 6,
      minorR: 3,
      rotation: Math.PI / 2,
    });
  });
});

describe("degeneracy guards — minSize context (C4)", () => {
  const ctx = { minSize: 4 };

  it("lineTool ends the chain on a click within minSize of the last anchor, commits a far one", () => {
    const armed = lineTool.step(lineTool.init(), { kind: "click", pt: { x: 0, y: 0 } });
    const ended = lineTool.step(armed.state, { kind: "click", pt: { x: 2, y: 0 } }, ctx);
    expect(ended.committed).toBeUndefined();
    expect(ended.done).toBe(true);
    expect(ended.state.anchors).toEqual([]); // chain ended, not still armed
    const far = lineTool.step(armed.state, { kind: "click", pt: { x: 10, y: 0 } }, ctx);
    expect(far.committed).toEqual([{ type: "Line", p0: { x: 0, y: 0 }, p1: { x: 10, y: 0 } }]);
  });

  it("rectTool ignores a corner with a sub-minSize extent on either axis", () => {
    const armed = rectTool.step(rectTool.init(), { kind: "click", pt: { x: 0, y: 0 } });
    const thin = rectTool.step(armed.state, { kind: "click", pt: { x: 2, y: 50 } }, ctx); // dx < 4
    expect(thin.committed).toBeUndefined();
    const ok = rectTool.step(armed.state, { kind: "click", pt: { x: 40, y: 20 } }, ctx);
    expect(ok.committed).toHaveLength(4);
  });

  it("circleTool ignores a radius below minSize", () => {
    const armed = circleTool.step(circleTool.init(), { kind: "click", pt: { x: 0, y: 0 } });
    const tiny = circleTool.step(armed.state, { kind: "click", pt: { x: 2, y: 0 } }, ctx); // r = 2 < 4
    expect(tiny.committed).toBeUndefined();
    const ok = circleTool.step(armed.state, { kind: "click", pt: { x: 0, y: 5 } }, ctx);
    expect(ok.committed).toEqual([{ type: "Circle", center: { x: 0, y: 0 }, radius: 5 }]);
  });

  it("ellipseTool rejects a sub-minSize major AND a sub-minSize minor", () => {
    const armed = ellipseTool.step(ellipseTool.init(), { kind: "click", pt: { x: 0, y: 0 } });
    const tinyMajor = ellipseTool.step(armed.state, { kind: "click", pt: { x: 2, y: 0 } }, ctx);
    expect(tinyMajor.state.anchors).toHaveLength(1); // still armed on the centre only

    const withMajor = ellipseTool.step(armed.state, { kind: "click", pt: { x: 20, y: 0 } }, ctx);
    expect(withMajor.state.anchors).toHaveLength(2);
    const tinyMinor = ellipseTool.step(withMajor.state, { kind: "click", pt: { x: 5, y: 1 } }, ctx);
    expect(tinyMinor.committed).toBeUndefined();
    expect(tinyMinor.state.anchors).toHaveLength(2); // stays armed for a real third click
    const ok = ellipseTool.step(withMajor.state, { kind: "click", pt: { x: 5, y: 9 } }, ctx);
    expect(ok.committed).toHaveLength(1);
  });

  it("arcTool ignores a start click within minSize of the center", () => {
    const armed = arcTool.step(arcTool.init(), { kind: "click", pt: { x: 0, y: 0 } }); // center
    const tiny = arcTool.step(armed.state, { kind: "click", pt: { x: 2, y: 0 } }, ctx); // r < 4
    expect(tiny.state.anchors).toHaveLength(1); // start rejected — only center anchored
    const ok = arcTool.step(armed.state, { kind: "click", pt: { x: 10, y: 0 } }, ctx);
    expect(ok.state.anchors).toHaveLength(2); // start accepted
  });
});

describe("lineTool — chain end / close semantics (U4)", () => {
  it("anchors accumulate the whole chain, not just the last segment", () => {
    const steps = run(lineTool, [
      ["click", { x: 0, y: 0 }],
      ["click", { x: 40, y: 0 }],
      ["click", { x: 40, y: 40 }],
    ]);
    expect(steps[2].state.anchors).toEqual([
      { x: 0, y: 0 },
      { x: 40, y: 0 },
      { x: 40, y: 40 },
    ]);
  });

  it("a 3-click open chain, then a click at the first anchor, closes the loop", () => {
    const steps = run(lineTool, [
      ["click", { x: 0, y: 0 }], // p0
      ["click", { x: 40, y: 0 }], // p1 — commits p0→p1
      ["click", { x: 40, y: 40 }], // p2 — commits p1→p2
      ["click", { x: 0, y: 0 }], // back to p0 — closes the loop
    ]);
    expect(steps[3].committed).toEqual([
      { type: "Line", p0: { x: 40, y: 40 }, p1: { x: 0, y: 0 } },
    ]);
    expect(steps[3].done).toBe(true);
    expect(steps[3].state.anchors).toEqual([]);
  });

  it("a click at the same point as the last anchor ends the chain without committing", () => {
    const ctx = { minSize: 4 };
    const armed = lineTool.step(lineTool.init(), { kind: "click", pt: { x: 0, y: 0 } }, ctx); // p0
    const afterFirstSeg = lineTool.step(armed.state, { kind: "click", pt: { x: 40, y: 0 } }, ctx); // p1 — commits p0→p1
    expect(afterFirstSeg.committed).toEqual([{ type: "Line", p0: { x: 0, y: 0 }, p1: { x: 40, y: 0 } }]);
    // Click within minSize of the just-placed last anchor — ends the chain.
    const ended = lineTool.step(afterFirstSeg.state, { kind: "click", pt: { x: 41, y: 0 } }, ctx);
    expect(ended.committed).toBeUndefined();
    expect(ended.done).toBe(true);
    expect(ended.state.anchors).toEqual([]);
  });

  it("a double-click (two pointerups at the same snapped point) ends the chain after committing the segment", () => {
    const ctx = { minSize: 4 };
    let state = lineTool.init();
    let step = lineTool.step(state, { kind: "click", pt: { x: 0, y: 0 } }, ctx); // p0
    state = step.state;
    step = lineTool.step(state, { kind: "click", pt: { x: 40, y: 0 } }, ctx); // p1 — commits p0→p1
    state = step.state;
    expect(step.committed).toEqual([{ type: "Line", p0: { x: 0, y: 0 }, p1: { x: 40, y: 0 } }]);
    // Second pointerup of the double-click lands on the same snapped point.
    step = lineTool.step(state, { kind: "click", pt: { x: 40, y: 0 } }, ctx);
    expect(step.committed).toBeUndefined();
    expect(step.done).toBe(true);
    expect(step.state.anchors).toEqual([]);
  });
});

// ── W2-B/C: point / centerRect / slot / polygon ──────────────────────────────

describe("pointTool — one click commits and re-arms", () => {
  it("commits a Point on every click and resets immediately", () => {
    const steps = run(pointTool, [
      ["move", { x: 5, y: 5 }],
      ["click", { x: 5, y: 5 }],
      ["click", { x: 20, y: 30 }],
    ]);
    expect(steps[0].preview).toEqual([]); // no rubber-band for a point
    expect(steps[0].committed).toBeUndefined();
    expect(steps[1].committed).toEqual([{ type: "Point", p0: { x: 5, y: 5 } }]);
    expect(steps[1].done).toBe(true);
    expect(steps[1].state.anchors).toEqual([]); // re-armed, not chained
    expect(steps[2].committed).toEqual([{ type: "Point", p0: { x: 20, y: 30 } }]);
  });

  it("Esc resets without committing", () => {
    const steps = run(pointTool, [["move", { x: 1, y: 1 }], ["esc"]]);
    expect(steps[1].committed).toBeUndefined();
    expect(steps[1].done).toBe(true);
    expect(steps[1].state).toEqual({ anchors: [], cursor: null });
  });
});

describe("centerRectTool — center → corner", () => {
  it("commits the 4 lines mirrored about the center", () => {
    const steps = run(centerRectTool, [["click", { x: 0, y: 0 }], ["click", { x: 40, y: 20 }]]);
    expect(steps[1].done).toBe(true);
    // Opposite corner is the reflection of the click through the centre.
    expect(steps[1].committed).toEqual([
      { type: "Line", p0: { x: -40, y: -20 }, p1: { x: 40, y: -20 } },
      { type: "Line", p0: { x: 40, y: -20 }, p1: { x: 40, y: 20 } },
      { type: "Line", p0: { x: 40, y: 20 }, p1: { x: -40, y: 20 } },
      { type: "Line", p0: { x: -40, y: 20 }, p1: { x: -40, y: -20 } },
    ]);
    // NO tool-authored constraints — standard inference only (documented V1).
    expect(steps[1].committedConstraints).toBeUndefined();
  });

  it("previews the mirrored rectangle while moving", () => {
    const steps = run(centerRectTool, [["click", { x: 0, y: 0 }], ["move", { x: 10, y: 5 }]]);
    expect(steps[1].preview).toHaveLength(4);
    expect(steps[1].preview[0].p0).toEqual({ x: -10, y: -5 });
  });

  it("rejects a corner whose HALF-extent is below minSize on either axis", () => {
    const ctx = { minSize: 4 };
    const armed = centerRectTool.step(centerRectTool.init(), { kind: "click", pt: { x: 0, y: 0 } }, ctx);
    const thin = centerRectTool.step(armed.state, { kind: "click", pt: { x: 2, y: 50 } }, ctx);
    expect(thin.committed).toBeUndefined();
    expect(thin.state.anchors).toEqual([{ x: 0, y: 0 }]); // still armed on the centre
    const ok = centerRectTool.step(armed.state, { kind: "click", pt: { x: 40, y: 20 } }, ctx);
    expect(ok.committed).toHaveLength(4);
  });

  it("Esc mid-gesture drops the centre anchor", () => {
    const armed = centerRectTool.step(centerRectTool.init(), { kind: "click", pt: { x: 0, y: 0 } });
    const escaped = centerRectTool.step(armed.state, { kind: "esc" });
    expect(escaped.done).toBe(true);
    expect(escaped.state.anchors).toEqual([]);
    expect(escaped.committed).toBeUndefined();
  });
});

describe("slotTool — centerline → width", () => {
  const ctx = { minSize: 4 };

  /** The CCW sweep midpoint of a center-start-end arc draft. */
  function sweepMidpoint(a: DraftEntity): Point2 {
    const c = a.center!;
    const r = a.radius!;
    const a0 = Math.atan2(a.start!.y - c.y, a.start!.x - c.x);
    const a1 = Math.atan2(a.end!.y - c.y, a.end!.x - c.x);
    let sweep = a1 - a0;
    while (sweep <= 0) sweep += 2 * Math.PI;
    const m = a0 + sweep / 2;
    return { x: c.x + r * Math.cos(m), y: c.y + r * Math.sin(m) };
  }

  it("commits [wall, wall, cap@p1, cap@p0] with exact geometry", () => {
    const steps = run(slotTool, [
      ["click", { x: 0, y: 0 }],
      ["click", { x: 100, y: 0 }],
      ["click", { x: 50, y: 20 }], // perpendicular distance 20 ⇒ r = 20
    ]);
    const c = steps[2].committed!;
    expect(steps[2].done).toBe(true);
    expect(c).toHaveLength(4);
    expect(c[0]).toEqual({ type: "Line", p0: { x: 0, y: 20 }, p1: { x: 100, y: 20 } });
    expect(c[1]).toEqual({ type: "Line", p0: { x: 0, y: -20 }, p1: { x: 100, y: -20 } });
    expect(c[2]).toMatchObject({ type: "Arc", center: { x: 100, y: 0 }, radius: 20 });
    expect(c[3]).toMatchObject({ type: "Arc", center: { x: 0, y: 0 }, radius: 20 });
    expect(c[2].start).toEqual({ x: 100, y: -20 });
    expect(c[2].end).toEqual({ x: 100, y: 20 });
    expect(c[3].start).toEqual({ x: 0, y: 20 });
    expect(c[3].end).toEqual({ x: 0, y: -20 });
  });

  it("each cap sweeps 180° over the OUTSIDE of the slot (6dp)", () => {
    // Vertical centerline so the check is not trivially axis-symmetric.
    const steps = run(slotTool, [
      ["click", { x: 0, y: 0 }],
      ["click", { x: 0, y: 100 }],
      ["click", { x: 30, y: 50 }], // r = 30
    ]);
    const [, , capP1, capP0] = steps[2].committed!;
    // Beyond p1 (+d) and beyond p0 (−d), not doubling back into the slot.
    const m1 = sweepMidpoint(capP1);
    expect(m1.x).toBeCloseTo(0, 6);
    expect(m1.y).toBeCloseTo(130, 6);
    const m0 = sweepMidpoint(capP0);
    expect(m0.x).toBeCloseTo(0, 6);
    expect(m0.y).toBeCloseTo(-30, 6);
    // Exactly 180° of sweep, and endpoints exactly on the wall ends.
    expect(capP1.start).toEqual({ x: 30, y: 100 });
    expect(capP1.end).toEqual({ x: -30, y: 100 });
    expect(capP0.start).toEqual({ x: -30, y: 0 });
    expect(capP0.end).toEqual({ x: 30, y: 0 });
  });

  it("welds each wall endpoint to the cap endpoint it touches + Equal ×1 (W0b)", () => {
    const steps = run(slotTool, [
      ["click", { x: 0, y: 0 }],
      ["click", { x: 100, y: 0 }],
      ["click", { x: 50, y: 20 }],
    ]);
    expect(steps[2].committedConstraints).toEqual([
      { type: "Coincident", refs: [0, 2], positions: ["End", "End"] },
      { type: "Coincident", refs: [1, 2], positions: ["End", "Start"] },
      { type: "Coincident", refs: [0, 3], positions: ["Start", "Start"] },
      { type: "Coincident", refs: [1, 3], positions: ["Start", "End"] },
      { type: "Equal", refs: [2, 3] },
    ]);
    // NO entity-level Tangents: welded to the arc endpoint, `distance(center,
    // wall) == radius` sits at its maximum and PlaneGCS calls all four redundant
    // (which renders as OverConstrained). See the slot header.
    expect(steps[2].committedConstraints!.some((c) => c.type === "Tangent")).toBe(false);
  });

  it("every weld names the endpoint the two drafts actually share", () => {
    const steps = run(slotTool, [
      ["click", { x: 0, y: 0 }],
      ["click", { x: 100, y: 0 }],
      ["click", { x: 50, y: 20 }],
    ]);
    const committed = steps[2].committed!;
    const at = (draft: DraftEntity, position: string): Point2 => {
      if (draft.type === "Line") return position === "End" ? draft.p1! : draft.p0!;
      return position === "End" ? draft.end! : draft.start!;
    };
    for (const spec of steps[2].committedConstraints!) {
      if (spec.type !== "Coincident") continue;
      const a = at(committed[spec.refs[0]], spec.positions![0]!);
      const b = at(committed[spec.refs[1]], spec.positions![1]!);
      expect(a.x).toBeCloseTo(b.x, 9);
      expect(a.y).toBeCloseTo(b.y, 9);
    }
  });

  it("rejects a sub-minSize centerline and a sub-minSize width", () => {
    const p0 = slotTool.step(slotTool.init(), { kind: "click", pt: { x: 0, y: 0 } }, ctx);
    const tinyLine = slotTool.step(p0.state, { kind: "click", pt: { x: 2, y: 0 } }, ctx);
    expect(tinyLine.state.anchors).toHaveLength(1); // centerline rejected
    const p1 = slotTool.step(p0.state, { kind: "click", pt: { x: 100, y: 0 } }, ctx);
    expect(p1.state.anchors).toHaveLength(2);
    const tinyWidth = slotTool.step(p1.state, { kind: "click", pt: { x: 50, y: 2 } }, ctx);
    expect(tinyWidth.committed).toBeUndefined();
    expect(tinyWidth.state.anchors).toHaveLength(2); // still armed on the centerline
  });

  it("previews the centerline as construction, then the full slot", () => {
    const p0 = slotTool.step(slotTool.init(), { kind: "click", pt: { x: 0, y: 0 } }, ctx);
    const rubber = slotTool.step(p0.state, { kind: "move", pt: { x: 100, y: 0 } }, ctx);
    expect(rubber.preview).toEqual([
      { type: "Line", p0: { x: 0, y: 0 }, p1: { x: 100, y: 0 }, construction: true },
    ]);
    const p1 = slotTool.step(p0.state, { kind: "click", pt: { x: 100, y: 0 } }, ctx);
    const full = slotTool.step(p1.state, { kind: "move", pt: { x: 50, y: 20 } }, ctx);
    expect(full.preview).toHaveLength(4);
    expect(full.preview.filter((d) => d.type === "Arc")).toHaveLength(2);
  });

  it("Esc mid-gesture drops both anchors", () => {
    const p0 = slotTool.step(slotTool.init(), { kind: "click", pt: { x: 0, y: 0 } }, ctx);
    const p1 = slotTool.step(p0.state, { kind: "click", pt: { x: 100, y: 0 } }, ctx);
    const escaped = slotTool.step(p1.state, { kind: "esc" }, ctx);
    expect(escaped.done).toBe(true);
    expect(escaped.state.anchors).toEqual([]);
    expect(escaped.committed).toBeUndefined();
  });

  it("perpDistance measures the offset from the infinite centerline", () => {
    expect(perpDistance({ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 300, y: -7 })).toBeCloseTo(7, 9);
    expect(perpDistance({ x: 0, y: 0 }, { x: 0, y: 0 }, { x: 3, y: 4 })).toBe(0); // degenerate
  });
});

describe("polygonTool — center → vertex, sides 3-12", () => {
  const ctx = { minSize: 4 };

  it("defaults to 6 sides and commits n lines + a construction circumcircle", () => {
    const armed = polygonTool.step(polygonTool.init(), { kind: "click", pt: { x: 0, y: 0 } }, ctx);
    expect(armed.state.sides).toBe(DEFAULT_POLYGON_SIDES);
    const done = polygonTool.step(armed.state, { kind: "click", pt: { x: 50, y: 0 } }, ctx);
    const c = done.committed!;
    expect(c).toHaveLength(7); // 6 ring lines + 1 circle
    expect(c.slice(0, 6).every((d) => d.type === "Line" && !d.construction)).toBe(true);
    expect(c[6]).toEqual({ type: "Circle", center: { x: 0, y: 0 }, radius: 50, construction: true });
    // The ring closes: line k's End is line k+1's Start.
    for (let k = 0; k < 6; k++) {
      const next = c[(k + 1) % 6];
      expect(c[k].p1!.x).toBeCloseTo(next.p0!.x, 9);
      expect(c[k].p1!.y).toBeCloseTo(next.p0!.y, 9);
    }
  });

  it("authors Coincident ×n + OnCurve ×n + Equal ×(n−1) — DOF (4n+3)−2n−n−(n−1) = 4", () => {
    const armed = polygonTool.step(polygonTool.init(), { kind: "click", pt: { x: 0, y: 0 } }, ctx);
    const done = polygonTool.step(armed.state, { kind: "click", pt: { x: 50, y: 0 } }, ctx);
    const specs = done.committedConstraints!;
    const n = 6;
    expect(specs.filter((s) => s.type === "Coincident")).toHaveLength(n);
    expect(specs.filter((s) => s.type === "OnCurve")).toHaveLength(n);
    expect(specs.filter((s) => s.type === "Equal")).toHaveLength(n - 1);
    // Coincident chain wraps: the last line's End meets the first line's Start.
    expect(specs[n - 1]).toEqual({ type: "Coincident", refs: [n - 1, 0], positions: ["End", "Start"] });
    // OnCurve points every line's Start at the circumcircle (index n, entity-level).
    for (const s of specs.filter((x) => x.type === "OnCurve")) {
      expect(s.refs[1]).toBe(n);
      expect(s.positions).toEqual(["Start", undefined]);
    }
  });

  it("a sides event re-previews at the new count and sticks across commits", () => {
    let step = polygonTool.step(polygonTool.init(), { kind: "sides", n: 5 }, ctx);
    expect(step.state.sides).toBe(5);
    step = polygonTool.step(step.state, { kind: "click", pt: { x: 0, y: 0 } }, ctx);
    const moved = polygonTool.step(step.state, { kind: "move", pt: { x: 50, y: 0 } }, ctx);
    expect(moved.preview).toHaveLength(6); // 5 lines + circumcircle
    const resized = polygonTool.step(moved.state, { kind: "sides", n: 8 }, ctx);
    expect(resized.preview).toHaveLength(9); // re-previewed at the live cursor
    const done = polygonTool.step(resized.state, { kind: "click", pt: { x: 50, y: 0 } }, ctx);
    expect(done.committed).toHaveLength(9);
    expect(done.state.sides).toBe(8); // sticky after the commit
    expect(polygonTool.step(done.state, { kind: "esc" }, ctx).state.sides).toBe(8); // and after Esc
  });

  it("clamps the side count to [3, 12]", () => {
    expect(clampSides(1)).toBe(3);
    expect(clampSides(2)).toBe(3);
    expect(clampSides(3)).toBe(3);
    expect(clampSides(12)).toBe(12);
    expect(clampSides(99)).toBe(12);
    expect(clampSides(6.4)).toBe(6);
    expect(clampSides(Number.NaN)).toBe(DEFAULT_POLYGON_SIDES);
    expect(polygonTool.step(polygonTool.init(), { kind: "sides", n: 40 }, ctx).state.sides).toBe(12);
  });

  it("rejects a vertex within minSize of the center", () => {
    const armed = polygonTool.step(polygonTool.init(), { kind: "click", pt: { x: 0, y: 0 } }, ctx);
    const tiny = polygonTool.step(armed.state, { kind: "click", pt: { x: 2, y: 0 } }, ctx);
    expect(tiny.committed).toBeUndefined();
    expect(tiny.state.anchors).toEqual([{ x: 0, y: 0 }]);
  });

  it("places the first vertex under the cursor and spaces the rest by 2π/n", () => {
    const v = polygonVertices({ x: 0, y: 0 }, { x: 0, y: 10 }, 4);
    expect(v[0].x).toBeCloseTo(0, 9);
    expect(v[0].y).toBeCloseTo(10, 9);
    expect(v[1].x).toBeCloseTo(-10, 9);
    expect(v[1].y).toBeCloseTo(0, 9);
    expect(v[2].x).toBeCloseTo(0, 9);
    expect(v[2].y).toBeCloseTo(-10, 9);
  });

  it("Esc mid-gesture drops the center anchor", () => {
    const armed = polygonTool.step(polygonTool.init(), { kind: "click", pt: { x: 0, y: 0 } }, ctx);
    const escaped = polygonTool.step(armed.state, { kind: "esc" }, ctx);
    expect(escaped.done).toBe(true);
    expect(escaped.state.anchors).toEqual([]);
  });
});

describe("sides events are ignored by every non-polygon machine", () => {
  it("leaves the state untouched", () => {
    for (const m of [lineTool, rectTool, centerRectTool, circleTool, ellipseTool, arcTool, slotTool, pointTool]) {
      const armed = m.step(m.init(), { kind: "click", pt: { x: 0, y: 0 } });
      const ignored = m.step(armed.state, { kind: "sides", n: 5 });
      expect(ignored.state).toBe(armed.state);
      expect(ignored.committed).toBeUndefined();
    }
  });
});

describe("resolveToolConstraints — index refs → real ids + inference filter", () => {
  const ent = (id: string): SketchEntity => ({ id, type: "Line", p0: [0, 0], p1: [1, 0] });
  const ids = (): (() => string) => {
    let n = 0;
    return () => `c${++n}`;
  };

  it("returns the inference untouched when a tool authored nothing", () => {
    const inferred: SketchConstraint[] = [{ id: "c1", type: "Horizontal", entities: ["e1"] }];
    expect(resolveToolConstraints(undefined, [ent("e1")], inferred, ids())).toBe(inferred);
    expect(resolveToolConstraints([], [ent("e1")], inferred, ids())).toBe(inferred);
  });

  it("resolves refs positionally against the id-assigned committed entities", () => {
    const out = resolveToolConstraints(
      [
        { type: "Tangent", refs: [0, 2] },
        { type: "Coincident", refs: [0, 1], positions: ["End", "Start"] },
        { type: "OnCurve", refs: [1, 2], positions: ["Start", undefined] },
      ],
      [ent("e7"), ent("e8"), ent("e9")],
      [],
      ids(),
    );
    expect(out).toEqual([
      { id: "c1", type: "Tangent", entities: ["e7", "e9"] },
      { id: "c2", type: "Coincident", entities: ["e7", "e8"], positions: ["End", "Start"] },
      // `undefined` marshals as "" — falsy, so resolveRef treats it as an ENTITY ref.
      { id: "c3", type: "OnCurve", entities: ["e8", "e9"], positions: ["Start", ""] },
    ]);
  });

  it("drops a spec whose ref is out of range instead of mis-binding it", () => {
    const out = resolveToolConstraints(
      [{ type: "Equal", refs: [0, 5] }, { type: "Equal", refs: [0, 1] }],
      [ent("e1"), ent("e2")],
      [],
      ids(),
    );
    expect(out).toEqual([{ id: "c1", type: "Equal", entities: ["e1", "e2"] }]);
  });

  it("keeps only inferences touching a PRE-EXISTING entity when specs are present", () => {
    const inferred: SketchConstraint[] = [
      { id: "i1", type: "Horizontal", entities: ["e1"] }, // intra-batch → dropped
      { id: "i2", type: "Coincident", entities: ["e1", "e2"] }, // intra-batch → dropped
      { id: "i3", type: "Coincident", entities: ["e2", "old1"] }, // touches existing → kept
    ];
    const out = resolveToolConstraints(
      [{ type: "Equal", refs: [0, 1] }],
      [ent("e1"), ent("e2")],
      inferred,
      ids(),
    );
    expect(out).toEqual([
      { id: "c1", type: "Equal", entities: ["e1", "e2"] },
      { id: "i3", type: "Coincident", entities: ["e2", "old1"] },
    ]);
  });
});

describe("draftToEntityFields", () => {
  it("flattens Point2 coords into [u,v] pairs", () => {
    const f = draftToEntityFields({ type: "Line", p0: { x: 1, y: 2 }, p1: { x: 3, y: 4 } });
    expect(f).toEqual({ type: "Line", p0: [1, 2], p1: [3, 4] });
  });
  it("keeps the construction flag", () => {
    const f = draftToEntityFields({ type: "Line", construction: true, p0: { x: 0, y: 0 }, p1: { x: 1, y: 1 } });
    expect(f.construction).toBe(true);
  });
});
