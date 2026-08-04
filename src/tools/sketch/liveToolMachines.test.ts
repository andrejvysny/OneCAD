import { describe, it, expect } from "vitest";
import { LIVE_TOOL_MACHINES, projectEvent, withLiveDims } from "./liveToolMachines";
import { dimFrame } from "./liveDimFrames";
import { TOOL_MACHINES, type ToolContext, type ToolEvent, type ToolMachine, type ToolStep } from "./toolMachine";
import type { Point2 } from "@/viewport/engine/sketchBasis";

const P = (x: number, y: number): Point2 => ({ x, y });

type Script = Array<["click" | "move", Point2] | ["esc"] | ["sides", number]>;

function toEvent(e: Script[number]): ToolEvent {
  if (e[0] === "esc") return { kind: "esc" };
  if (e[0] === "sides") return { kind: "sides", n: e[1] as number };
  return { kind: e[0], pt: e[1] as Point2 };
}

function run(m: ToolMachine, script: Script, ctx?: ToolContext): ToolStep[] {
  let state = m.init();
  const steps: ToolStep[] = [];
  for (const e of script) {
    const step = m.step(state, toEvent(e), ctx);
    state = step.state;
    steps.push(step);
  }
  return steps;
}

const dist = (a: Point2, b: Point2): number => Math.hypot(a.x - b.x, a.y - b.y);

/** Pointer sequences that carry every tool through all of its phases. */
const SCRIPTS: Record<string, Script> = {
  line: [["click", P(0, 0)], ["move", P(40, 0)], ["click", P(40, 0)], ["click", P(40, 20)], ["esc"]],
  rect: [["click", P(0, 0)], ["move", P(80, 40)], ["click", P(80, 40)]],
  centerRect: [["click", P(0, 0)], ["move", P(40, 20)], ["click", P(40, 20)]],
  circle: [["click", P(0, 0)], ["move", P(0, 25)], ["click", P(0, 25)]],
  ellipse: [["click", P(0, 0)], ["move", P(40, 0)], ["click", P(40, 0)], ["move", P(20, 15)], ["click", P(20, 15)]],
  arc: [["click", P(0, 0)], ["click", P(20, 0)], ["move", P(0, 20)], ["click", P(0, 20)]],
  polygon: [["click", P(0, 0)], ["sides", 5], ["move", P(30, 0)], ["click", P(30, 0)]],
  slot: [["click", P(0, 0)], ["click", P(40, 0)], ["move", P(20, 10)], ["click", P(20, 10)]],
  point: [["click", P(5, 5)], ["move", P(6, 6)], ["click", P(7, 7)]],
};

describe("LIVE_TOOL_MACHINES — transparency with no locks and no quantum", () => {
  it("wraps every raw machine, one for one", () => {
    expect(Object.keys(LIVE_TOOL_MACHINES).sort()).toEqual(Object.keys(TOOL_MACHINES).sort());
    for (const [id, m] of Object.entries(LIVE_TOOL_MACHINES)) expect(m.id).toBe(id);
  });

  for (const [id, script] of Object.entries(SCRIPTS)) {
    it(`${id}: state / preview / committed / committedConstraints match the raw machine`, () => {
      const raw = run(TOOL_MACHINES[id], script);
      const live = run(LIVE_TOOL_MACHINES[id], script);
      expect(live).toHaveLength(raw.length);
      for (let i = 0; i < raw.length; i++) {
        expect(live[i].state).toEqual(raw[i].state);
        expect(live[i].preview).toEqual(raw[i].preview);
        expect(live[i].committed).toEqual(raw[i].committed);
        expect(live[i].committedConstraints).toEqual(raw[i].committedConstraints);
        expect(live[i].done).toEqual(raw[i].done);
      }
    });
  }

  it("passes the point through by IDENTITY, not through a measure/rebuild trip", () => {
    const frame = dimFrame("line", [P(0, 0)]);
    const ev: ToolEvent = { kind: "click", pt: P(1 / 3, Math.PI) };
    expect(projectEvent(frame, ev)).toBe(ev);
    expect(projectEvent(frame, ev, {}, null)).toBe(ev);
  });

  it("still honours ctx.minSize (the raw machines' own contract)", () => {
    const live = withLiveDims(TOOL_MACHINES.line);
    const steps = run(live, [["click", P(0, 0)], ["click", P(0.5, 0)]], { minSize: 4 });
    expect(steps[1].committed).toBeUndefined();
    expect(steps[1].done).toBe(true); // sub-minSize click ends the chain
  });
});

describe("withLiveDims — chip descriptors", () => {
  it("emits nothing before the first anchor", () => {
    const steps = run(LIVE_TOOL_MACHINES.line, [["move", P(10, 10)]]);
    expect(steps[0].dims).toBeUndefined();
  });

  it("emits the phase's fields once the gesture is armed", () => {
    const steps = run(LIVE_TOOL_MACHINES.line, [["click", P(0, 0)], ["move", P(30, 40)]]);
    expect(steps[1].dims?.map((d) => d.field)).toEqual(["length", "angle"]);
    expect(steps[1].dims?.[0].value).toBeCloseTo(50, 9);
    expect(steps[1].dims?.[0].locked).toBe(false);
  });

  it("follows the tool through its phases", () => {
    const steps = run(LIVE_TOOL_MACHINES.arc, [
      ["click", P(0, 0)],
      ["move", P(20, 0)],
      ["click", P(20, 0)],
      ["move", P(0, 20)],
    ]);
    expect(steps[1].dims?.map((d) => d.field)).toEqual(["radius"]);
    expect(steps[3].dims?.map((d) => d.field)).toEqual(["angle"]);
    expect(steps[3].dims?.[0].value).toBeCloseTo(90, 9);
  });

  it("emits nothing for the point tool, which has no dimensions", () => {
    for (const step of run(LIVE_TOOL_MACHINES.point, SCRIPTS.point)) {
      expect(step.dims).toBeUndefined();
    }
  });

  it("clears the chips when the gesture is done", () => {
    const steps = run(LIVE_TOOL_MACHINES.rect, SCRIPTS.rect);
    expect(steps[1].dims?.map((d) => d.field)).toEqual(["width", "height"]);
    expect(steps[2].done).toBe(true);
    expect(steps[2].dims).toBeUndefined();
  });

  it("reports a locked field as locked, carrying the typed value", () => {
    const ctx: ToolContext = { locks: { length: 50 } };
    const steps = run(LIVE_TOOL_MACHINES.line, [["click", P(0, 0)], ["move", P(18, 24)]], ctx);
    const length = steps[1].dims?.find((d) => d.field === "length");
    expect(length).toMatchObject({ locked: true, value: 50 });
    expect(steps[1].dims?.find((d) => d.field === "angle")).toMatchObject({ locked: false });
  });

  it("reports the polygon side count as non-driving alongside its radius", () => {
    const steps = run(LIVE_TOOL_MACHINES.polygon, [["click", P(0, 0)], ["sides", 5], ["move", P(30, 0)]]);
    expect(steps[2].dims).toEqual([
      expect.objectContaining({ field: "radius", value: 30, drives: true }),
      expect.objectContaining({ field: "sides", value: 5, drives: false }),
    ]);
  });
});

describe("withLiveDims — lock projection", () => {
  it("a locked length commits EXACTLY that length, at the cursor's angle", () => {
    const ctx: ToolContext = { locks: { length: 50 } };
    // Cursor 30mm out on a 3-4-5 ray; the lock stretches it to 50 without turning.
    const steps = run(LIVE_TOOL_MACHINES.line, [["click", P(0, 0)], ["click", P(18, 24)]], ctx);
    const seg = steps[1].committed?.[0];
    expect(seg?.p1?.x).toBeCloseTo(30, 9);
    expect(seg?.p1?.y).toBeCloseTo(40, 9);
    expect(dist(P(0, 0), seg?.p1 as Point2)).toBeCloseTo(50, 9);
  });

  it("a locked angle steers without changing the length", () => {
    const ctx: ToolContext = { locks: { angle: 90 } };
    const steps = run(LIVE_TOOL_MACHINES.line, [["click", P(0, 0)], ["click", P(30, 5)]], ctx);
    const seg = steps[1].committed?.[0];
    expect(seg?.p1?.x).toBeCloseTo(0, 9);
    expect(seg?.p1?.y).toBeCloseTo(Math.hypot(30, 5), 9);
  });

  it("locks both fields at once", () => {
    const ctx: ToolContext = { locks: { length: 50, angle: 30 } };
    const steps = run(LIVE_TOOL_MACHINES.line, [["click", P(0, 0)], ["click", P(1, -80)]], ctx);
    const p1 = steps[1].committed?.[0].p1 as Point2;
    expect(p1.x).toBeCloseTo(50 * Math.cos(Math.PI / 6), 9);
    expect(p1.y).toBeCloseTo(50 * Math.sin(Math.PI / 6), 9);
  });

  it("a locked rect width keeps the height on the cursor and the quadrant intact", () => {
    const ctx: ToolContext = { locks: { width: 80 } };
    const steps = run(LIVE_TOOL_MACHINES.rect, [["click", P(0, 0)], ["click", P(-12, -33)]], ctx);
    expect(steps[1].committed?.[0]).toEqual({ type: "Line", p0: P(0, 0), p1: P(-80, 0) });
    expect(steps[1].committed?.[2].p0).toEqual(P(-80, -33));
  });

  it("a locked circle diameter beats its stale radius sibling", () => {
    const ctx: ToolContext = { locks: { diameter: 25 } };
    const steps = run(LIVE_TOOL_MACHINES.circle, [["click", P(0, 0)], ["click", P(100, 0)]], ctx);
    expect(steps[1].committed?.[0].radius).toBeCloseTo(12.5, 9);
  });

  it("a locked arc radius survives the phase change and shapes the commit", () => {
    const ctx: ToolContext = { locks: { radius: 40 } };
    const steps = run(
      LIVE_TOOL_MACHINES.arc,
      [["click", P(0, 0)], ["click", P(10, 0)], ["click", P(0, 5)]],
      ctx,
    );
    expect(steps[1].state.anchors[1]).toEqual(P(40, 0)); // phase 1 click stretched
    expect(steps[2].committed?.[0].radius).toBeCloseTo(40, 9);
  });

  it("a locked slot width is the FULL width (cap radius is half)", () => {
    const ctx: ToolContext = { locks: { width: 20 } };
    const steps = run(
      LIVE_TOOL_MACHINES.slot,
      [["click", P(0, 0)], ["click", P(40, 0)], ["click", P(20, 3)]],
      ctx,
    );
    expect(steps[2].committed?.[2].radius).toBeCloseTo(10, 9);
  });

  it("leaves the non-pointer verbs alone", () => {
    const ctx: ToolContext = { locks: { radius: 40 } };
    const steps = run(LIVE_TOOL_MACHINES.polygon, [["click", P(0, 0)], ["sides", 8]], ctx);
    expect(steps[1].state.sides).toBe(8);
  });
});

describe("withLiveDims — quantum rounding", () => {
  const quantum = { length: 0.1, angle: 1 };

  it("rounds a cursor-placed length to the quantum", () => {
    const steps = run(LIVE_TOOL_MACHINES.line, [["click", P(0, 0)], ["click", P(22.3607, 0)]], {
      quantum,
    });
    expect(dist(P(0, 0), steps[1].committed?.[0].p1 as Point2)).toBeCloseTo(22.4, 9);
  });

  it("rounds a cursor-placed angle to whole degrees", () => {
    const steps = run(LIVE_TOOL_MACHINES.line, [["click", P(0, 0)], ["move", P(10, 0.9)]], {
      quantum,
    });
    const angle = steps[1].dims?.find((d) => d.field === "angle");
    expect(angle?.value).toBeCloseTo(5, 6); // atan2 ⇒ 5.143° ⇒ 5°
  });

  it("NEVER re-rounds a locked value — the typed number wins outright", () => {
    const ctx: ToolContext = { locks: { length: 22.3607 }, quantum };
    const steps = run(LIVE_TOOL_MACHINES.line, [["click", P(0, 0)], ["click", P(5, 0)]], ctx);
    expect(steps[1].committed?.[0].p1).toEqual(P(22.3607, 0));
  });

  it("locks still apply with rounding OFF (Alt held ⇒ quantum null)", () => {
    const ctx: ToolContext = { locks: { length: 22.3607 }, quantum: null };
    const steps = run(LIVE_TOOL_MACHINES.line, [["click", P(0, 0)], ["click", P(5, 0)]], ctx);
    expect(steps[1].committed?.[0].p1).toEqual(P(22.3607, 0));
  });

  it("rounds the RADIUS, not the diameter, so both chips stay whole", () => {
    const steps = run(LIVE_TOOL_MACHINES.circle, [["click", P(0, 0)], ["click", P(12.34, 0)]], {
      quantum,
    });
    expect(steps[1].committed?.[0].radius).toBeCloseTo(12.3, 9);
  });

  it("leaves a side COUNT alone — it is a verb, not a measurement", () => {
    const steps = run(LIVE_TOOL_MACHINES.polygon, [["click", P(0, 0)], ["sides", 7], ["move", P(30, 0)]], {
      quantum,
    });
    expect(steps[2].dims?.find((d) => d.field === "sides")?.value).toBe(7);
  });
});
