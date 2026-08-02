/*
 * displayMode persistence (moved off viewportStore into settingsStore so it
 * survives reloads). Covers the migrate path (pre-v4 blob, no key at all) AND
 * the merge path (a same-version blob with a garbage value must still coerce —
 * migrate only runs on a version MISMATCH, so a hand-edited/rolled-back v4
 * blob relies entirely on `merge` to recover).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { settingsStore } from "./settingsStore";
import { DEFAULT_THEME } from "@/theme/themes";
import { DEFAULT_RENDER_MODE } from "@/viewport/engine/renderModes";

const STORAGE_KEY = "onecad.settings";

function seed(version: number, state: Record<string, unknown>): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({ state, version }));
}

describe("settingsStore displayMode", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
    settingsStore.setState({ displayMode: DEFAULT_RENDER_MODE });
  });

  it("a v3 blob (predates displayMode entirely) migrates to the default mode", async () => {
    seed(3, {
      snapTo: { grid: true },
      show: { guidePoints: true, snappingHints: true },
      navigation: { inputDevice: "auto" },
    });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().displayMode).toBe(DEFAULT_RENDER_MODE);
  });

  it("a v5 blob with an unknown displayMode coerces to default via merge", async () => {
    seed(5, { displayMode: "garbage" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().displayMode).toBe(DEFAULT_RENDER_MODE);
  });

  it("a valid persisted mode survives hydration unchanged", async () => {
    seed(5, { displayMode: "wireframe" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().displayMode).toBe("wireframe");
  });

  it("setDisplayMode updates the store and writes localStorage", () => {
    settingsStore.getState().setDisplayMode("wireframe");
    expect(settingsStore.getState().displayMode).toBe("wireframe");

    const raw = localStorage.getItem(STORAGE_KEY);
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!).state.displayMode).toBe("wireframe");
  });
});

describe("settingsStore theme", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
    settingsStore.setState({ theme: DEFAULT_THEME });
  });

  it("a v4 blob (predates theme entirely) migrates to the default preference", async () => {
    seed(4, { displayMode: "wireframe", navigation: { inputDevice: "auto" } });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().theme).toBe(DEFAULT_THEME);
    // The migration must not disturb the setting that already existed.
    expect(settingsStore.getState().displayMode).toBe("wireframe");
  });

  it("a v5 blob with an unknown theme coerces to default via merge", async () => {
    seed(5, { theme: "solarized" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().theme).toBe(DEFAULT_THEME);
  });

  it("a valid persisted preference survives hydration unchanged", async () => {
    seed(5, { theme: "dark" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().theme).toBe("dark");
  });

  it('persists "system" AS "system" — never a resolved light/dark value', async () => {
    settingsStore.getState().setTheme("system");
    const raw = localStorage.getItem(STORAGE_KEY);
    expect(JSON.parse(raw!).state.theme).toBe("system");
  });

  it("setTheme updates the store and writes localStorage", () => {
    settingsStore.getState().setTheme("dark");
    expect(settingsStore.getState().theme).toBe("dark");

    const raw = localStorage.getItem(STORAGE_KEY);
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!).state.theme).toBe("dark");
  });
});
