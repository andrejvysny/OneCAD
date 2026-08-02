import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args) }));

import { __resetLogForTests, logInfo, type LogEvent } from "./log";
import { installLogForwarder } from "./logSink";

/** Pretend we are inside a Tauri webview (the only place forwarding may run). */
function withBridge(): void {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}
function withoutBridge(): void {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}

/** The events of the Nth `log_event` batch. */
function batch(n: number): LogEvent[] {
  return (invoke.mock.calls[n][1] as { events: LogEvent[] }).events;
}

let uninstall: (() => void) | null = null;

beforeEach(() => {
  vi.useFakeTimers();
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  __resetLogForTests();
  withBridge();
});

afterEach(() => {
  uninstall?.();
  uninstall = null;
  withoutBridge();
  __resetLogForTests({ enabled: false });
  vi.useRealTimers();
});

describe("installLogForwarder gating", () => {
  it("no-ops on the MOCK lane (no __TAURI_INTERNALS__) — nothing is ever invoked", async () => {
    withoutBridge();
    uninstall = installLogForwarder();
    logInfo("vp", "mock lane");
    await vi.advanceTimersByTimeAsync(1000);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("no-ops when the logger gate itself is closed", async () => {
    __resetLogForTests({ enabled: false });
    uninstall = installLogForwarder();
    await vi.advanceTimersByTimeAsync(1000);
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("batching", () => {
  it("flushes a lone event on the 250ms lazy timer, not before", async () => {
    uninstall = installLogForwarder();
    logInfo("vp", "one");
    await vi.advanceTimersByTimeAsync(249);
    expect(invoke).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke.mock.calls[0][0]).toBe("log_event");
    expect(batch(0)).toHaveLength(1);
  });

  it("flushes immediately at the 50-event threshold", async () => {
    uninstall = installLogForwarder();
    for (let i = 0; i < 50; i++) logInfo("vp", `m${i}`);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(batch(0)).toHaveLength(50);
  });

  it("forwards the DTO verbatim — camelCase fields the Rust side pins", async () => {
    uninstall = installLogForwarder();
    logInfo("worker", "lifecycle ready", { epoch: 3 });
    await vi.advanceTimersByTimeAsync(250);
    expect(batch(0)[0]).toEqual({
      ts: expect.any(Number),
      tMono: expect.any(Number),
      seq: expect.any(Number),
      lane: "fe",
      level: "info",
      tag: "worker",
      msg: "lifecycle ready",
      ctx: { epoch: 3 },
    });
  });

  it("never forwards the `sink` tag (the recursion path)", async () => {
    uninstall = installLogForwarder();
    logInfo("sink", "about myself");
    logInfo("vp", "about the viewport");
    await vi.advanceTimersByTimeAsync(250);
    expect(batch(0).map((e) => e.tag)).toEqual(["vp"]);
  });

  it("caps the pending buffer at 500, dropping the OLDEST", async () => {
    // A backend that never settles keeps `inFlight` true, so everything after the
    // first flush piles up in the buffer — exactly the case the cap exists for.
    let release: () => void = () => {};
    invoke.mockReturnValueOnce(
      new Promise<void>((resolve) => {
        release = () => resolve();
      }),
    );
    uninstall = installLogForwarder();
    for (let i = 0; i < 600; i++) logInfo("vp", `m${i}`);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(batch(0)).toHaveLength(50); // m0..m49 went out before the stall

    release();
    await vi.advanceTimersByTimeAsync(0);
    // 550 events arrived while stalled; the newest 500 survived.
    expect(invoke).toHaveBeenCalledTimes(2);
    const second = batch(1);
    expect(second).toHaveLength(500);
    expect(second[0].msg).toBe("m100");
    expect(second[499].msg).toBe("m599");
  });

  it("flushes on pagehide", async () => {
    uninstall = installLogForwarder();
    logInfo("vp", "unsaved");
    window.dispatchEvent(new Event("pagehide"));
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});

describe("failure handling", () => {
  it("disables forwarding after 3 consecutive failures, with exactly one console.warn", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    invoke.mockRejectedValue(new Error("command log_event not found"));
    uninstall = installLogForwarder();

    for (let i = 0; i < 3; i++) {
      logInfo("vp", `attempt${i}`);
      await vi.advanceTimersByTimeAsync(250);
    }
    expect(invoke).toHaveBeenCalledTimes(3);
    expect(warn).toHaveBeenCalledTimes(1);

    // Session-disabled: further events neither buffer nor invoke.
    logInfo("vp", "after");
    await vi.advanceTimersByTimeAsync(1000);
    expect(invoke).toHaveBeenCalledTimes(3);
    expect(warn).toHaveBeenCalledTimes(1);
    warn.mockRestore();
  });

  it("resets the failure count on a success, so a transient error never disables", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    uninstall = installLogForwarder();
    for (let i = 0; i < 6; i++) {
      invoke.mockImplementationOnce(() => (i % 2 === 0 ? Promise.reject(new Error("x")) : Promise.resolve()));
      logInfo("vp", `m${i}`);
      await vi.advanceTimersByTimeAsync(250);
    }
    expect(invoke).toHaveBeenCalledTimes(6);
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });
});

describe("teardown", () => {
  it("uninstalls the subscription and allows a fresh install", async () => {
    uninstall = installLogForwarder();
    uninstall();
    uninstall = null;
    logInfo("vp", "after teardown");
    await vi.advanceTimersByTimeAsync(1000);
    expect(invoke).not.toHaveBeenCalled();

    uninstall = installLogForwarder();
    logInfo("vp", "after reinstall");
    await vi.advanceTimersByTimeAsync(250);
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("a second install while one is live is a no-op (single forwarder per session)", async () => {
    uninstall = installLogForwarder();
    const second = installLogForwarder();
    logInfo("vp", "once");
    await vi.advanceTimersByTimeAsync(250);
    expect(invoke).toHaveBeenCalledTimes(1);
    second();
  });
});
