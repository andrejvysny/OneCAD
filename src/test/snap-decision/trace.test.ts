/*
 * Snap decision trace: shape + emission policy (SNAP P0 gate).
 *
 * The trace is the whole point of P0 — if it is emitted per pointer frame it
 * violates the logger's drag-frequency policy and drowns the lane; if it is
 * emitted too rarely the phase-to-phase behaviour changes are unreviewable. The
 * rule is: emit on a CHANGED accepted set, a click, a latch reset, or a
 * projection failure — never on an identical repeat.
 */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { __resetLogForTests, logSnapshot, type LogEvent } from "@/debug/log";
import {
  __resetSnapTraceForTests,
  latestSnapTrace,
  mintTraceId,
  projectionFailureTrace,
  recordSnapTrace,
  traceCandidates,
} from "@/tools/sketch/snapTrace";
import { emptyLatch, isotropicMetric, type SnapCandidate, type SnapDecisionTrace } from "@/tools/sketch/snapTypes";

function candidate(id: string, scorePx: number): SnapCandidate {
  return {
    id,
    source: "geometryPoint",
    kind: "endpoint",
    projection: { kind: "point", point: { x: 1, y: 2 } },
    previewPoint: { x: 1, y: 2 },
    claims: ["point"],
    errorPx: scorePx + 1,
    semanticBiasPx: -1,
    scorePx,
    label: "Endpoint",
    guides: [],
    refs: [{ entityId: "l1", position: "Start" }],
    relationIntents: [],
  };
}

function trace(acceptedIds: string[], origin: "hover" | "click" = "hover"): SnapDecisionTrace {
  return {
    traceId: mintTraceId(),
    interactionEpoch: 3,
    sessionGeneration: 7,
    origin,
    raw: { x: 1.2, y: 2.2 },
    client: { x: 400, y: 300 },
    projection: "orthographic",
    metric: isotropicMetric(1),
    gridStep: 1,
    uCellPx: 1,
    vCellPx: 1,
    pointRadiusPx: 8,
    gridRadiusPx: 0.35,
    sources: { grid: true, guide: true },
    candidates: traceCandidates(acceptedIds.map((id, i) => candidate(id, i))),
    priorLatch: emptyLatch(),
    acceptedIds,
    rejected: [{ candidateId: "curve:c1", reason: "claim-conflict" }],
    point: { x: 1, y: 2 },
    altHeld: false,
  };
}

const snapEvents = (): LogEvent[] => logSnapshot().filter((e) => e.tag === "snap");

describe("snap trace", () => {
  beforeEach(() => {
    __resetSnapTraceForTests();
    __resetLogForTests({ enabled: true, console: false });
  });
  afterEach(() => {
    __resetLogForTests({ enabled: false });
    __resetSnapTraceForTests();
  });

  it("mints strictly increasing trace ids", () => {
    const a = mintTraceId();
    const b = mintTraceId();
    expect(b).toBeGreaterThan(a);
  });

  it("retains the latest trace even when it does not log", () => {
    recordSnapTrace(trace(["point:l1:Start:endpoint"]));
    const repeat = trace(["point:l1:Start:endpoint"]);
    recordSnapTrace(repeat);
    expect(latestSnapTrace()?.traceId).toBe(repeat.traceId);
    expect(snapEvents()).toHaveLength(1); // the repeat did NOT log
  });

  it("logs when the accepted set changes", () => {
    recordSnapTrace(trace(["a"]));
    recordSnapTrace(trace(["a"]));
    recordSnapTrace(trace(["a", "b"]));
    recordSnapTrace(trace(["a", "b"]));
    expect(snapEvents()).toHaveLength(2);
  });

  it("always logs a click, even with an unchanged set", () => {
    recordSnapTrace(trace(["a"]));
    recordSnapTrace(trace(["a"], "click"));
    const events = snapEvents();
    expect(events).toHaveLength(2);
    expect(events[1].msg).toContain("click");
  });

  it("always logs a latch reset", () => {
    recordSnapTrace(trace(["a"]));
    recordSnapTrace(trace(["a"]), { latchReset: true });
    expect(snapEvents()).toHaveLength(2);
    expect(snapEvents()[1].ctx?.latchReset).toBe(true);
  });

  it("always logs a projection failure", () => {
    const t = projectionFailureTrace({
      interactionEpoch: 1,
      sessionGeneration: 1,
      origin: "hover",
      client: { x: 10, y: 10 },
      altHeld: false,
      priorLatch: emptyLatch(),
    });
    recordSnapTrace(t);
    recordSnapTrace(
      projectionFailureTrace({
        interactionEpoch: 1,
        sessionGeneration: 1,
        origin: "hover",
        client: { x: 11, y: 10 },
        altHeld: false,
        priorLatch: emptyLatch(),
      }),
    );
    const events = snapEvents();
    expect(events).toHaveLength(2);
    expect(events[0].msg).toContain("projection failed");
    expect(t.metric).toBeNull();
    expect(t.point).toBeNull();
  });

  it("carries rejection reasons and refs, but no raw geometry payloads", () => {
    recordSnapTrace(trace(["a"]));
    const ctx = snapEvents()[0].ctx as Record<string, unknown>;
    expect(ctx.rejected).toEqual(["curve:c1:claim-conflict"]);
    // Rows are FLAT strings so the logger's depth cap cannot swallow the refs
    // and claims — the two fields the trace exists to carry.
    const rows = ctx.candidates as string[];
    expect(rows[0]).toContain("[l1.Start]");
    expect(rows[0]).toContain("{point}");
    for (const row of rows) expect(typeof row).toBe("string");
    // The retained trace keeps the structured form for `?vpdebug`.
    expect(latestSnapTrace()?.candidates[0].refs).toEqual(["l1.Start"]);
  });

  it("rounds pixel figures so a trace diff is readable", () => {
    const t = trace(["a"]);
    expect(t.candidates[0].scorePx).toBe(0);
    const rows = traceCandidates([candidate("x", 1 / 3)]);
    expect(rows[0].scorePx).toBe(0.333);
  });
});
