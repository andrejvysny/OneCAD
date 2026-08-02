/*
 * Theme controller: DOM stamping, OS following, and — the part that matters for
 * cost — that listeners fire ONLY when the resolved theme actually changes.
 * Each notification triggers a PMREM environment rebuild in the viewport, so a
 * spurious fire is real GPU work.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { settingsStore } from "@/stores/settingsStore";
import { DEFAULT_THEME } from "./themes";
import {
  startThemeController,
  getResolvedTheme,
  subscribeResolvedTheme,
  __resetThemeControllerForTests,
} from "./themeController";

/** Controllable stand-in for the OS appearance. */
function installMatchMedia(dark: boolean) {
  const handlers = new Set<() => void>();
  const mql = {
    matches: dark,
    media: "(prefers-color-scheme: dark)",
    onchange: null,
    addEventListener: (_: string, cb: () => void) => handlers.add(cb),
    removeEventListener: (_: string, cb: () => void) => handlers.delete(cb),
    addListener: (cb: () => void) => handlers.add(cb),
    removeListener: (cb: () => void) => handlers.delete(cb),
    dispatchEvent: () => true,
  };
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    configurable: true,
    value: () => mql as unknown as MediaQueryList,
  });
  return {
    /** Flip the simulated OS appearance and notify, like a real media change. */
    setDark(next: boolean) {
      mql.matches = next;
      for (const cb of handlers) cb();
    },
    listenerCount: () => handlers.size,
  };
}

const root = () => document.documentElement;

describe("themeController", () => {
  beforeEach(() => {
    __resetThemeControllerForTests();
    settingsStore.setState({ theme: DEFAULT_THEME });
    delete root().dataset.theme;
    root().style.colorScheme = "";
  });

  afterEach(() => {
    __resetThemeControllerForTests();
    settingsStore.setState({ theme: DEFAULT_THEME });
  });

  it("stamps data-theme and color-scheme on <html> at start", () => {
    installMatchMedia(false);
    startThemeController();
    expect(root().dataset.theme).toBe("light");
    expect(root().style.colorScheme).toBe("light");
  });

  it('"system" follows a dark OS', () => {
    installMatchMedia(true);
    startThemeController();
    expect(root().dataset.theme).toBe("dark");
    expect(getResolvedTheme()).toBe("dark");
  });

  it("an explicit preference overrides the OS", () => {
    installMatchMedia(true);
    settingsStore.setState({ theme: "light" });
    startThemeController();
    expect(root().dataset.theme).toBe("light");
  });

  it("follows a live OS change while on system", () => {
    const os = installMatchMedia(false);
    startThemeController();
    expect(root().dataset.theme).toBe("light");

    os.setDark(true);
    expect(root().dataset.theme).toBe("dark");
    expect(root().style.colorScheme).toBe("dark");
  });

  it("ignores OS changes once the preference is explicit", () => {
    const os = installMatchMedia(false);
    settingsStore.setState({ theme: "light" });
    startThemeController();

    const seen = vi.fn();
    subscribeResolvedTheme(seen);

    os.setDark(true);
    expect(root().dataset.theme).toBe("light");
    expect(seen).not.toHaveBeenCalled();
  });

  it("notifies subscribers when the preference changes the resolved theme", () => {
    installMatchMedia(false);
    startThemeController();

    const seen = vi.fn();
    subscribeResolvedTheme(seen);

    settingsStore.getState().setTheme("dark");
    expect(seen).toHaveBeenCalledTimes(1);
    expect(seen).toHaveBeenCalledWith("dark");
  });

  it("does NOT notify when a preference change resolves to the same theme", () => {
    // OS is dark, preference goes system → dark: resolved value never moves.
    installMatchMedia(true);
    startThemeController();
    expect(getResolvedTheme()).toBe("dark");

    const seen = vi.fn();
    subscribeResolvedTheme(seen);

    settingsStore.getState().setTheme("dark");
    expect(getResolvedTheme()).toBe("dark");
    expect(seen).not.toHaveBeenCalled();
  });

  it("does NOT notify on unrelated settings writes", () => {
    installMatchMedia(false);
    startThemeController();

    const seen = vi.fn();
    subscribeResolvedTheme(seen);

    settingsStore.getState().setSnap("grid", false);
    settingsStore.getState().setDisplayMode("wireframe");
    expect(seen).not.toHaveBeenCalled();
  });

  it("unsubscribe stops delivery", () => {
    installMatchMedia(false);
    startThemeController();

    const seen = vi.fn();
    const off = subscribeResolvedTheme(seen);
    off();

    settingsStore.getState().setTheme("dark");
    expect(seen).not.toHaveBeenCalled();
  });

  it("start is idempotent — a second call does not double-subscribe", () => {
    const os = installMatchMedia(false);
    startThemeController();
    startThemeController();
    expect(os.listenerCount()).toBe(1);

    const seen = vi.fn();
    subscribeResolvedTheme(seen);
    settingsStore.getState().setTheme("dark");
    expect(seen).toHaveBeenCalledTimes(1);
  });

  it("stop() detaches both the store and the media listener", () => {
    const os = installMatchMedia(false);
    const stop = startThemeController();
    stop();
    expect(os.listenerCount()).toBe(0);

    const seen = vi.fn();
    subscribeResolvedTheme(seen);
    settingsStore.getState().setTheme("dark");
    os.setDark(true);
    expect(seen).not.toHaveBeenCalled();
  });

  it("getResolvedTheme derives on demand before start", () => {
    installMatchMedia(true);
    // No startThemeController() — nothing has stamped the DOM yet.
    expect(root().dataset.theme).toBeUndefined();
    expect(getResolvedTheme()).toBe("dark");
  });

  it("falls back to light when the platform has no matchMedia", () => {
    Object.defineProperty(window, "matchMedia", {
      writable: true,
      configurable: true,
      value: undefined,
    });
    startThemeController();
    expect(root().dataset.theme).toBe("light");
  });
});
