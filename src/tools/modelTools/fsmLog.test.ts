import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { withPhaseLog } from "./fsmLog";
import { extrudeInit, extrudeStep } from "./modelToolMachine";
import { __resetLogForTests, logSnapshot } from "@/debug/log";

describe("withPhaseLog", () => {
  beforeEach(() => {
    __resetLogForTests();
  });
  afterEach(() => {
    __resetLogForTests({ enabled: false });
  });

  it("passes the reducer's result through unchanged", () => {
    const wrapped = withPhaseLog("extrude", extrudeStep);
    const raw = extrudeStep(extrudeInit(), { kind: "arm" });
    const via = wrapped(extrudeInit(), { kind: "arm" });
    expect(via).toEqual(raw);
  });

  it("logs one `fsm` debug event per PHASE CHANGE, with the causing event", () => {
    const step = withPhaseLog("extrude", extrudeStep);
    let s = extrudeInit();
    s = step(s, { kind: "arm" }).state; // idle → armed
    s = step(s, { kind: "grab" }).state; // armed → dragging

    const events = logSnapshot();
    expect(events).toHaveLength(2);
    expect(events[0]).toMatchObject({ level: "debug", tag: "fsm" });
    expect(events[0].ctx).toMatchObject({
      tool: "extrude",
      event: "arm",
      from: "idle",
      to: "armed",
    });
    expect(events[1].ctx).toMatchObject({ event: "grab", from: "armed", to: "dragging" });
    expect(typeof (events[1].ctx as { effect?: unknown }).effect).toBe("string");
  });

  it("is SILENT for same-phase steps — this is what keeps a drag off the log lane", () => {
    const step = withPhaseLog("extrude", extrudeStep);
    let s = step(extrudeInit(), { kind: "arm" }).state;
    s = step(s, { kind: "grab" }).state;
    expect(logSnapshot()).toHaveLength(2);

    // 200 drag frames, all in `dragging` — not one extra event.
    for (let i = 0; i < 200; i++) s = step(s, { kind: "drag", depth: i }).state;
    expect(s.phase).toBe("dragging");
    expect(logSnapshot()).toHaveLength(2);
  });

  it("emits nothing at all when the gate is closed (production / vitest default)", () => {
    __resetLogForTests({ enabled: false });
    const step = withPhaseLog("extrude", extrudeStep);
    const out = step(extrudeInit(), { kind: "arm" });
    expect(out.state.phase).toBe("armed");
    expect(logSnapshot()).toEqual([]);
  });
});
