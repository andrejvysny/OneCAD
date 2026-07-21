import { describe, it, expect } from "vitest";
import { lineTool, rectTool, circleTool, arcTool, draftToEntityFields, type ToolMachine, type ToolStep } from "./toolMachine";
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
