/*
 * TEST-LIFE-04 (lane U part) — the idle DPR watcher.
 *
 * A stationary window dragged between displays changes nothing a
 * `ResizeObserver` can see, and render-on-demand means there is no next frame
 * to recheck in. The watcher must therefore be EVENT driven, must re-arm its
 * media query at the NEW ratio (a query pinned to the old value reports only
 * the first transition), must remove every listener on disposal, and must not
 * poll — an idle viewport schedules zero recurring work.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { watchDevicePixelRatio } from "./dprWatcher";

interface FakeQuery {
  media: string;
  listeners: Array<() => void>;
  addEventListener: (type: string, fn: () => void) => void;
  removeEventListener: (type: string, fn: () => void) => void;
}

function installFakeMatchMedia(): { queries: FakeQuery[]; live: () => FakeQuery[] } {
  const queries: FakeQuery[] = [];
  vi.stubGlobal("matchMedia", (media: string) => {
    const q: FakeQuery = {
      media,
      listeners: [],
      addEventListener: (type, fn) => {
        if (type === "change") q.listeners.push(fn);
      },
      removeEventListener: (type, fn) => {
        if (type !== "change") return;
        const i = q.listeners.indexOf(fn);
        if (i >= 0) q.listeners.splice(i, 1);
      },
    };
    queries.push(q);
    return q as unknown as MediaQueryList;
  });
  return { queries, live: () => queries.filter((q) => q.listeners.length > 0) };
}

afterEach(() => vi.unstubAllGlobals());

describe("TEST-LIFE-04 — watchDevicePixelRatio", () => {
  it("arms a resolution query at the CURRENT ratio", () => {
    const { queries } = installFakeMatchMedia();
    vi.stubGlobal("devicePixelRatio", 2);
    const dispose = watchDevicePixelRatio(() => {});
    expect(queries).toHaveLength(1);
    expect(queries[0].media).toBe("(resolution: 2dppx)");
    dispose();
  });

  it("fires onChange once and RE-ARMS on the new ratio", () => {
    const { queries, live } = installFakeMatchMedia();
    vi.stubGlobal("devicePixelRatio", 1);
    const onChange = vi.fn();
    const dispose = watchDevicePixelRatio(onChange);
    expect(queries[0].media).toBe("(resolution: 1dppx)");

    // The display changed: the 1dppx query stopped matching.
    vi.stubGlobal("devicePixelRatio", 2);
    queries[0].listeners[0]();

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith(2);
    // Exactly one live query, and it is pinned to the NEW ratio. A watcher that
    // failed to re-arm would still hold the stale 1dppx query and would never
    // hear about the trip back.
    expect(queries).toHaveLength(2);
    expect(queries[1].media).toBe("(resolution: 2dppx)");
    expect(live()).toEqual([queries[1]]);

    // …and the trip back is heard.
    vi.stubGlobal("devicePixelRatio", 1);
    queries[1].listeners[0]();
    expect(onChange).toHaveBeenCalledTimes(2);
    expect(onChange).toHaveBeenLastCalledWith(1);

    dispose();
  });

  it("the disposer removes every listener, and later events are silent", () => {
    const { queries, live } = installFakeMatchMedia();
    vi.stubGlobal("devicePixelRatio", 1);
    const onChange = vi.fn();
    const dispose = watchDevicePixelRatio(onChange);
    const armed = queries[0];

    dispose();
    expect(live()).toEqual([]);

    // A listener someone else still holds a reference to must be inert.
    vi.stubGlobal("devicePixelRatio", 2);
    armed.listeners.forEach((fn) => fn());
    expect(onChange).not.toHaveBeenCalled();
    dispose(); // idempotent
  });

  it("also listens to visualViewport resize, and unhooks it", () => {
    installFakeMatchMedia();
    vi.stubGlobal("devicePixelRatio", 1);
    const add = vi.fn();
    const remove = vi.fn();
    vi.stubGlobal("visualViewport", { addEventListener: add, removeEventListener: remove });

    const onChange = vi.fn();
    const dispose = watchDevicePixelRatio(onChange);
    expect(add).toHaveBeenCalledWith("resize", expect.any(Function));

    vi.stubGlobal("devicePixelRatio", 2);
    add.mock.calls[0][1]();
    expect(onChange).toHaveBeenCalledWith(2);

    dispose();
    expect(remove).toHaveBeenCalledWith("resize", add.mock.calls[0][1]);
  });

  it("polls NOTHING — no interval, no timeout, no rAF", () => {
    installFakeMatchMedia();
    vi.stubGlobal("devicePixelRatio", 1);
    const setInterval = vi.fn();
    const setTimeout = vi.fn();
    const requestAnimationFrame = vi.fn();
    vi.stubGlobal("setInterval", setInterval);
    vi.stubGlobal("setTimeout", setTimeout);
    vi.stubGlobal("requestAnimationFrame", requestAnimationFrame);

    const dispose = watchDevicePixelRatio(() => {});
    expect(setInterval).not.toHaveBeenCalled();
    expect(setTimeout).not.toHaveBeenCalled();
    expect(requestAnimationFrame).not.toHaveBeenCalled();
    dispose();
  });

  it("is an inert no-op where matchMedia does not exist", () => {
    vi.stubGlobal("matchMedia", undefined);
    const onChange = vi.fn();
    const dispose = watchDevicePixelRatio(onChange);
    expect(() => dispose()).not.toThrow();
    expect(onChange).not.toHaveBeenCalled();
  });
});
