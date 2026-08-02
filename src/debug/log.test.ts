import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  __resetLogForTests,
  log,
  logDebug,
  logEnabled,
  logError,
  logInfo,
  logSnapshot,
  logWarn,
  onLogEvent,
  type LogEvent,
} from "./log";
import { trace, traceWarn } from "./trace";

/** The window surface the logger installs when the gate is open. */
interface LogWindow {
  __logs?: LogEvent[];
  __logsDump?: () => string;
  __logsClear?: () => void;
}
const w = window as unknown as LogWindow;

describe("log gate", () => {
  // NOT enabled here on purpose — this asserts the vitest exclusion itself.
  it("is OFF under vitest, so ~30 test files never console-flood", () => {
    expect(logEnabled()).toBe(false);
  });

  it("a disabled logger records nothing and installs no window surface", () => {
    const seen: LogEvent[] = [];
    const off = onLogEvent((e) => seen.push(e));
    logInfo("sketch", "should be dropped");
    off();
    expect(seen).toEqual([]);
    expect(logSnapshot()).toEqual([]);
    expect(w.__logsDump).toBeUndefined();
  });

  it("__resetLogForTests re-opens the gate and installs the window surface", () => {
    __resetLogForTests();
    expect(logEnabled()).toBe(true);
    expect(typeof w.__logsDump).toBe("function");
  });
});

describe("log emission", () => {
  beforeEach(() => {
    __resetLogForTests();
  });
  afterEach(() => {
    __resetLogForTests({ enabled: false });
  });

  it("stamps lane/level/tag/msg and a monotonically increasing seq", () => {
    logDebug("ipc", "first");
    logInfo("ipc", "second");
    const events = logSnapshot();
    expect(events).toHaveLength(2);
    expect(events[0]).toMatchObject({ lane: "fe", level: "debug", tag: "ipc", msg: "first", seq: 0 });
    expect(events[1]).toMatchObject({ level: "info", seq: 1 });
    expect(events[1].tMono).toBeGreaterThanOrEqual(events[0].tMono);
    expect(events[0].ts).toBeGreaterThan(0);
  });

  it("carries every level through the right helper", () => {
    logDebug("vp", "d");
    logInfo("vp", "i");
    logWarn("vp", "w");
    logError("vp", "e");
    log("warn", "vp", "runtime-chosen");
    expect(logSnapshot().map((e) => e.level)).toEqual(["debug", "info", "warn", "error", "warn"]);
  });

  it("omits ctx entirely when the caller passed none", () => {
    logInfo("vp", "no ctx");
    expect("ctx" in logSnapshot()[0]).toBe(false);
  });

  it("notifies subscribers and survives a throwing one", () => {
    const seen: LogEvent[] = [];
    const offBad = onLogEvent(() => {
      throw new Error("subscriber blew up");
    });
    const offGood = onLogEvent((e) => seen.push(e));
    expect(() => logInfo("vp", "still delivered")).not.toThrow();
    expect(seen.map((e) => e.msg)).toEqual(["still delivered"]);
    offBad();
    offGood();
    logInfo("vp", "after unsubscribe");
    expect(seen).toHaveLength(1);
  });

  it("mirrors to the console only when explicitly opted in", () => {
    const spy = vi.spyOn(console, "info").mockImplementation(() => {});
    logInfo("vp", "silent");
    expect(spy).not.toHaveBeenCalled();
    __resetLogForTests({ console: true });
    logInfo("vp", "loud", { a: 1 });
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy.mock.calls[0][0]).toContain("[vp] loud");
    spy.mockRestore();
  });
});

describe("ctx capping (applied AT ENTRY)", () => {
  beforeEach(() => {
    __resetLogForTests();
  });
  afterEach(() => {
    __resetLogForTests({ enabled: false });
  });

  const ctxOf = (): Record<string, unknown> => logSnapshot()[0].ctx as Record<string, unknown>;

  it("truncates a long string and says how much it dropped", () => {
    logInfo("vp", "m", { s: "x".repeat(600) });
    const s = ctxOf().s as string;
    expect(s).toHaveLength(500 + "…(+100)".length);
    expect(s.endsWith("…(+100)")).toBe(true);
  });

  it("keeps 20 array items plus a remainder marker", () => {
    logInfo("vp", "m", { xs: Array.from({ length: 25 }, (_, i) => i) });
    const xs = ctxOf().xs as unknown[];
    expect(xs).toHaveLength(21);
    expect(xs[20]).toBe("…+5 more");
  });

  it("collapses a TypedArray to its type + byteLength, never its samples", () => {
    logInfo("mesh", "m", { verts: new Float32Array(3000) });
    expect(ctxOf().verts).toEqual({ type: "Float32Array", byteLength: 12000 });
  });

  it("collapses nesting past depth 2", () => {
    logInfo("vp", "m", { a: { b: { c: { d: 1 } } } });
    const a = ctxOf().a as { b: { c: unknown } };
    expect(a.b.c).toBe("[Object]");
  });

  it("keeps an Error readable instead of serializing it to {}", () => {
    logError("vp", "m", { error: new TypeError("boom") });
    expect(ctxOf().error).toMatchObject({ name: "TypeError", message: "boom" });
  });

  it("does not recurse forever on a circular object", () => {
    const a: Record<string, unknown> = {};
    a.self = a;
    expect(() => logInfo("vp", "m", { a })).not.toThrow();
  });
});

describe("ring buffer", () => {
  beforeEach(() => {
    __resetLogForTests();
  });
  afterEach(() => {
    __resetLogForTests({ enabled: false });
  });

  it("caps at 2000 and drops the OLDEST, keeping chronological order", () => {
    for (let i = 0; i < 2050; i++) logDebug("vp", `m${i}`);
    const events = logSnapshot();
    expect(events).toHaveLength(2000);
    expect(events[0].msg).toBe("m50");
    expect(events[1999].msg).toBe("m2049");
    // seq keeps counting past the wrap — the ring drops events, it never renumbers.
    expect(events[0].seq).toBe(50);
  });
});

describe("window surface", () => {
  beforeEach(() => {
    __resetLogForTests();
  });
  afterEach(() => {
    __resetLogForTests({ enabled: false });
  });

  it("__logs reads the LIVE ring, __logsDump serializes it, __logsClear empties it", () => {
    logInfo("vp", "one");
    expect(w.__logs).toHaveLength(1);
    logInfo("vp", "two");
    expect(w.__logs).toHaveLength(2); // a getter, not a stale snapshot
    const dumped = JSON.parse(w.__logsDump!()) as LogEvent[];
    expect(dumped.map((e) => e.msg)).toEqual(["one", "two"]);
    w.__logsClear!();
    expect(logSnapshot()).toEqual([]);
  });

  it("is removed again when the gate closes", () => {
    __resetLogForTests({ enabled: false });
    expect(w.__logs).toBeUndefined();
    expect(w.__logsDump).toBeUndefined();
  });
});

describe("trace.ts back-compat shim", () => {
  beforeEach(() => {
    __resetLogForTests();
  });
  afterEach(() => {
    __resetLogForTests({ enabled: false });
  });

  it("maps trace/traceWarn onto info/warn and wraps `data` as ctx.data", () => {
    trace("extrude", "armed");
    trace("extrude", "with data", { depth: 10 });
    traceWarn("extrude", "commit failed", "reason");
    const events = logSnapshot();
    expect(events.map((e) => e.level)).toEqual(["info", "info", "warn"]);
    expect(events[0].ctx).toBeUndefined();
    expect(events[1].ctx).toEqual({ data: { depth: 10 } });
    expect(events[2]).toMatchObject({ tag: "extrude", ctx: { data: "reason" } });
  });
});
