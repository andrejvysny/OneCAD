import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { viewportStore, AUTO_DISMISS_MS } from "./viewportStore";

const hint = () => viewportStore.getState().statusHint;
const set = (message: string | null, opts?: { severity?: "info" | "error"; sticky?: boolean }) =>
  viewportStore.getState().setStatusHint(message, opts);

describe("viewportStore.setStatusHint", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    set(null);
  });
  afterEach(() => {
    set(null); // cancel any armed timer
    vi.useRealTimers();
  });

  it("defaults to a non-sticky info hint that auto-dismisses after AUTO_DISMISS_MS", () => {
    set("Extruded");
    expect(hint()).toEqual({ message: "Extruded", severity: "info", sticky: false });

    vi.advanceTimersByTime(AUTO_DISMISS_MS - 1);
    expect(hint()).not.toBeNull();
    vi.advanceTimersByTime(1);
    expect(hint()).toBeNull();
  });

  it("keeps a sticky hint past the auto-dismiss window", () => {
    set("Select a plane", { sticky: true });
    vi.advanceTimersByTime(AUTO_DISMISS_MS * 3);
    expect(hint()).toEqual({ message: "Select a plane", severity: "info", sticky: true });
  });

  it("stores error severity", () => {
    set("Extrude failed: boom", { severity: "error", sticky: true });
    expect(hint()).toEqual({ message: "Extrude failed: boom", severity: "error", sticky: true });
  });

  it("latest-wins: a newer hint re-arms the timer and a stale timer never clears it early", () => {
    set("first");
    vi.advanceTimersByTime(AUTO_DISMISS_MS - 100); // first is nearly expired
    set("second"); // supersedes → cancels first's timer, arms its own

    // The moment first's ORIGINAL deadline would have fired: second must survive.
    vi.advanceTimersByTime(100);
    expect(hint()?.message).toBe("second");

    // second lives out its own full window, then clears.
    vi.advanceTimersByTime(AUTO_DISMISS_MS - 100);
    expect(hint()).toBeNull();
  });

  it("explicit null clears and cancels the pending timer", () => {
    set("transient");
    set(null);
    expect(hint()).toBeNull();
    // A leftover timer would try to clear again (harmless) — assert nothing throws / re-fires.
    vi.advanceTimersByTime(AUTO_DISMISS_MS * 2);
    expect(hint()).toBeNull();
  });

  it("a sticky hint replacing a non-sticky one cancels the pending auto-dismiss", () => {
    set("transient");
    set("prompt", { sticky: true });
    vi.advanceTimersByTime(AUTO_DISMISS_MS * 2);
    expect(hint()?.message).toBe("prompt"); // the earlier non-sticky timer did not fire
  });
});
