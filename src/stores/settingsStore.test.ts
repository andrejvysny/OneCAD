/*
 * displayMode persistence (moved off viewportStore into settingsStore so it
 * survives reloads). Covers the migrate path (pre-v4 blob, no key at all) AND
 * the merge path (a same-version blob with a garbage value must still coerce —
 * migrate only runs on a version MISMATCH, so a hand-edited/rolled-back v4
 * blob relies entirely on `merge` to recover).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { settingsStore } from "./settingsStore";
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

  it("a v4 blob with an unknown displayMode coerces to default via merge", async () => {
    seed(4, { displayMode: "garbage" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().displayMode).toBe(DEFAULT_RENDER_MODE);
  });

  it("a valid persisted mode survives hydration unchanged", async () => {
    seed(4, { displayMode: "wireframe" });
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
