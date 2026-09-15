/*
 * displayMode persistence (moved off viewportStore into settingsStore so it
 * survives reloads). Covers the migrate path (pre-v4 blob, no key at all) AND
 * the merge path (a same-version blob with a garbage value must still coerce —
 * migrate only runs on a version MISMATCH, so a hand-edited/rolled-back v4
 * blob relies entirely on `merge` to recover).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  DEFAULT_AUTO_CONSTRAIN,
  mergeBooleanRecord,
  settingsStore,
  SNAP_DEFAULTS,
  SHOW_DEFAULTS,
} from "./settingsStore";
import { DEFAULT_THEME } from "@/theme/themes";
import {
  DEFAULT_SNAP_RADIUS,
  SNAP_RADIUS_ORDER,
  SNAP_RADIUS_PX,
} from "@/tools/sketch/snapRadius";
import { DEFAULT_LENGTH_UNIT, LENGTH_UNIT_ORDER } from "@/units/lengthUnits";
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

  it("a v6 blob with an unknown displayMode coerces to default via merge", async () => {
    seed(6, { displayMode: "garbage" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().displayMode).toBe(DEFAULT_RENDER_MODE);
  });

  it("a valid persisted mode survives hydration unchanged", async () => {
    seed(6, { displayMode: "wireframe" });
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

  it("a v6 blob with an unknown theme coerces to default via merge", async () => {
    seed(6, { theme: "solarized" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().theme).toBe(DEFAULT_THEME);
  });

  it("a valid persisted preference survives hydration unchanged", async () => {
    seed(6, { theme: "dark" });
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

/*
 * displayUnit (WP-C2). Same two recovery paths as displayMode and theme —
 * `migrate` for a blob authored before the key existed, `merge` for a
 * same-version blob carrying a value the registry does not know.
 */
describe("settingsStore displayUnit", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
    settingsStore.setState({ displayUnit: DEFAULT_LENGTH_UNIT });
  });

  it("a v5 blob (predates displayUnit entirely) migrates to millimetres", async () => {
    seed(5, { theme: "dark", displayMode: "wireframe" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().displayUnit).toBe(DEFAULT_LENGTH_UNIT);
    expect(settingsStore.getState().displayUnit).toBe("mm");
    // The migration must not disturb the settings that already existed.
    expect(settingsStore.getState().theme).toBe("dark");
    expect(settingsStore.getState().displayMode).toBe("wireframe");
  });

  it("a v6 blob with an unknown displayUnit coerces to mm via merge", async () => {
    seed(6, { displayUnit: "furlongs" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().displayUnit).toBe("mm");
  });

  it("a non-string displayUnit coerces too", async () => {
    seed(6, { displayUnit: 42 });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().displayUnit).toBe("mm");
  });

  it("every registry unit survives hydration unchanged", async () => {
    for (const unit of LENGTH_UNIT_ORDER) {
      localStorage.clear();
      seed(6, { displayUnit: unit });
      await settingsStore.persist.rehydrate();
      expect(settingsStore.getState().displayUnit).toBe(unit);
    }
  });

  it("setDisplayUnit updates the store and writes localStorage", () => {
    settingsStore.getState().setDisplayUnit("in");
    expect(settingsStore.getState().displayUnit).toBe("in");

    const raw = localStorage.getItem(STORAGE_KEY);
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!).state.displayUnit).toBe("in");
    expect(JSON.parse(raw!).version).toBe(13);
  });
});

/*
 * Live dimensions (SP-1). Both prefs default ON, so a blob authored before they
 * existed must come back ON too — a silently disabled feature for every existing
 * user is exactly what the backfill migrate exists to prevent.
 */
describe("settingsStore live dimensions", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
  });

  it("a v6 blob (predates both keys) migrates with rounding + chips ON", async () => {
    seed(6, {
      snapTo: { grid: false, onCurve: true },
      show: { guidePoints: true, snappingHints: false },
      displayUnit: "in",
    });
    await settingsStore.persist.rehydrate();
    const s = settingsStore.getState();
    expect(s.snapTo.dimensionRound).toBe(true);
    expect(s.show.liveDimensions).toBe(true);
    // The migration must not disturb the settings that already existed.
    expect(s.snapTo.grid).toBe(false);
    expect(s.show.snappingHints).toBe(false);
    expect(s.displayUnit).toBe("in");
  });

  it("a v7 blob's explicit OFF survives hydration (never re-defaulted)", async () => {
    seed(7, {
      snapTo: { grid: true, dimensionRound: false },
      show: { guidePoints: true, snappingHints: true, liveDimensions: false },
    });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().snapTo.dimensionRound).toBe(false);
    expect(settingsStore.getState().show.liveDimensions).toBe(false);
  });
});

/*
 * Polar tracking + snap radius (SP-5). polarTracking backfills ON (fresh-install
 * parity), and snapRadius backfills to "m" = 8px — the reach every build before
 * v8 hard-coded, so a migrated blob must snap EXACTLY as it did before the knob
 * existed. Same two recovery paths as the other registry-backed settings:
 * `migrate` for a blob authored before the key, `merge` for a same-version blob
 * carrying a value the registry does not know.
 */
describe("settingsStore polar tracking + snap radius", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
    settingsStore.setState({ snapRadius: DEFAULT_SNAP_RADIUS });
  });

  it("a v7 blob (predates both keys) migrates with polar ON and the 8px radius", async () => {
    seed(7, {
      snapTo: { grid: false, dimensionRound: true },
      show: { guidePoints: true, snappingHints: false, liveDimensions: true },
      displayUnit: "in",
    });
    await settingsStore.persist.rehydrate();
    const s = settingsStore.getState();
    expect(s.snapTo.polarTracking).toBe(true);
    expect(s.snapRadius).toBe(DEFAULT_SNAP_RADIUS);
    expect(SNAP_RADIUS_PX[s.snapRadius]).toBe(8);
    // The migration must not disturb the settings that already existed.
    expect(s.snapTo.grid).toBe(false);
    expect(s.snapTo.dimensionRound).toBe(true);
    expect(s.show.snappingHints).toBe(false);
    expect(s.displayUnit).toBe("in");
  });

  it("a v8 blob with an unknown snapRadius coerces to the default via merge", async () => {
    seed(8, { snapRadius: "xl" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().snapRadius).toBe(DEFAULT_SNAP_RADIUS);
  });

  it("a NUMERIC snapRadius (the px value, not the id) coerces too", async () => {
    seed(8, { snapRadius: 12 });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().snapRadius).toBe(DEFAULT_SNAP_RADIUS);
  });

  it("every registry radius survives hydration unchanged", async () => {
    for (const id of SNAP_RADIUS_ORDER) {
      localStorage.clear();
      seed(8, { snapRadius: id });
      await settingsStore.persist.rehydrate();
      expect(settingsStore.getState().snapRadius).toBe(id);
    }
  });

  it("a v8 blob's explicit polar OFF survives hydration (never re-defaulted)", async () => {
    seed(8, { snapTo: { grid: true, polarTracking: false } });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().snapTo.polarTracking).toBe(false);
  });

  it("setSnapRadius updates the store and writes localStorage", () => {
    settingsStore.getState().setSnapRadius("s");
    expect(settingsStore.getState().snapRadius).toBe("s");

    const raw = localStorage.getItem(STORAGE_KEY);
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!).state.snapRadius).toBe("s");
  });
});

/*
 * Coincident-badge visibility (Sketcher UX cleanup, Track B2). Unlike every
 * other `show.*` backfill above, this is a behavior CHANGE for existing
 * users, not fresh-install parity — Coincident badges used to always render.
 * A pre-v10 blob therefore backfills OFF, the new default, same as a fresh
 * install would get.
 */
describe("settingsStore coincident badges", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
  });

  it("a v9 blob (predates the key) migrates to OFF", async () => {
    seed(9, {
      show: { guidePoints: true, snappingHints: true, constraintChips: true },
      displayUnit: "in",
    });
    await settingsStore.persist.rehydrate();
    const s = settingsStore.getState();
    expect(s.show.coincidentBadges).toBe(false);
    // The migration must not disturb the settings that already existed.
    expect(s.show.constraintChips).toBe(true);
    expect(s.displayUnit).toBe("in");
  });

  it("a v10 blob's explicit ON survives hydration (never re-defaulted)", async () => {
    seed(10, { show: { guidePoints: true, constraintChips: true, coincidentBadges: true } });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().show.coincidentBadges).toBe(true);
  });

  it("setShow updates the store and writes localStorage", () => {
    settingsStore.getState().setShow("coincidentBadges", true);
    expect(settingsStore.getState().show.coincidentBadges).toBe(true);

    const raw = localStorage.getItem(STORAGE_KEY);
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw!).state.show.coincidentBadges).toBe(true);
  });
});

/*
 * Deep hydration of the NESTED boolean sections (SNAP P1).
 *
 * zustand's default `merge` is a shallow spread: a persisted `snapTo` object
 * REPLACED the defaults wholesale, so every key the blob omitted read
 * `undefined` — falsy — and the corresponding snap source came up OFF. That is
 * silent, permanent, and indistinguishable from the user having turned it off.
 */
describe("settingsStore deep hydration", () => {
  beforeEach(() => {
    localStorage.clear();
  });
  afterEach(() => {
    localStorage.clear();
    settingsStore.setState({
      snapTo: { ...SNAP_DEFAULTS },
      show: { ...SHOW_DEFAULTS },
      autoConstrainMode: DEFAULT_AUTO_CONSTRAIN,
    });
  });

  it("a SAME-version partial snapTo keeps the defaults for absent keys", async () => {
    seed(10, { snapTo: { grid: false } });
    await settingsStore.persist.rehydrate();
    const s = settingsStore.getState();
    expect(s.snapTo.grid).toBe(false); // the explicit choice survives
    expect(s.snapTo.quadrant).toBe(true); // …and the rest are not silently off
    expect(s.snapTo.onCurve).toBe(true);
    expect(s.snapTo.polarTracking).toBe(true);
    expect(s.snapTo.dimensionRound).toBe(true);
  });

  it("a SAME-version partial show keeps the defaults for absent keys", async () => {
    seed(10, { show: { snappingHints: false } });
    await settingsStore.persist.rehydrate();
    const s = settingsStore.getState();
    expect(s.show.snappingHints).toBe(false);
    expect(s.show.guidePoints).toBe(true);
    expect(s.show.liveDimensions).toBe(true);
    expect(s.show.constraintChips).toBe(true);
    expect(s.show.coincidentBadges).toBe(false); // an off-by-default stays off
  });

  it("null / wrong-typed sections fall back to the defaults wholesale", async () => {
    seed(10, { snapTo: null, show: "nope" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().snapTo).toEqual(SNAP_DEFAULTS);
    expect(settingsStore.getState().show).toEqual(SHOW_DEFAULTS);
  });

  it("a wrong-typed VALUE falls back rather than being coerced", async () => {
    // `"false"` is a truthy string: coercing it would turn a switch the user
    // deliberately turned off back ON.
    seed(10, { snapTo: { grid: "false", quadrant: 0, onCurve: 1 } });
    await settingsStore.persist.rehydrate();
    const s = settingsStore.getState();
    expect(s.snapTo.grid).toBe(true);
    expect(s.snapTo.quadrant).toBe(true);
    expect(s.snapTo.onCurve).toBe(true);
  });

  it("unknown nested keys are ignored", async () => {
    seed(10, { snapTo: { grid: true, notARealSetting: true } });
    await settingsStore.persist.rehydrate();
    expect(Object.keys(settingsStore.getState().snapTo).sort()).toEqual(
      Object.keys(SNAP_DEFAULTS).sort(),
    );
  });

  it("a corrupt navigation section falls back without losing the rest", async () => {
    seed(10, { navigation: 42, snapTo: { grid: false } });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().navigation.inputDevice).toBe("auto");
    expect(settingsStore.getState().snapTo.grid).toBe(false);
  });

  it("a pre-v11 blob migrates to the STANDARD auto-constrain mode", async () => {
    // Not a fresh-install-parity backfill by accident: `standard` IS what every
    // pre-v11 build did (H/V + Perpendicular always inferred). The mode is new;
    // the behaviour a migrated user gets is unchanged.
    seed(10, { snapTo: { grid: true } });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().autoConstrainMode).toBe("standard");
  });

  it("an unknown persisted auto-constrain mode coerces to standard", async () => {
    seed(11, { autoConstrainMode: "aggressive" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().autoConstrainMode).toBe(DEFAULT_AUTO_CONSTRAIN);
  });

  it("an explicit auto-constrain mode survives hydration", async () => {
    seed(11, { autoConstrainMode: "off" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().autoConstrainMode).toBe("off");
  });

  it("mergeBooleanRecord preserves an explicit false and every default", () => {
    expect(mergeBooleanRecord(SNAP_DEFAULTS, { grid: false })).toEqual({
      ...SNAP_DEFAULTS,
      grid: false,
    });
    expect(mergeBooleanRecord(SNAP_DEFAULTS, undefined)).toEqual(SNAP_DEFAULTS);
    expect(mergeBooleanRecord(SNAP_DEFAULTS, [])).toEqual(SNAP_DEFAULTS);
  });
});

/*
 * The assistant gate (v11 → v12). Not a fresh-install-parity backfill and not a
 * coercion: the assistant is opt-in for EVERY user, so a pre-v12 blob lands off
 * regardless of what it carried.
 */
describe("settingsStore assistantEnabled", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
    settingsStore.setState({ assistantEnabled: false });
  });

  it("defaults to off", () => {
    expect(settingsStore.getInitialState().assistantEnabled).toBe(false);
  });

  it("a v11 blob (predates the key) migrates to v12 with the gate off", async () => {
    seed(11, { autoConstrainMode: "off" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().assistantEnabled).toBe(false);
    // The rest of the blob is untouched by the bump.
    expect(settingsStore.getState().autoConstrainMode).toBe("off");
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!).version).toBe(13);
  });

  it("a v11 blob that somehow carries a true still lands off", async () => {
    seed(11, { assistantEnabled: true });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().assistantEnabled).toBe(false);
  });

  it("an explicit opt-in at the current version survives hydration", async () => {
    seed(13, { assistantEnabled: true });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().assistantEnabled).toBe(true);
  });
});

/*
 * The assistant's local provider (v12 → v13). Endpoint and model are the user's
 * PROPOSAL — Rust validates them (ADR-0017) — so nothing here is checked for
 * shape beyond "is it a string": what a pre-v13 blob gets is "", which is the
 * "no provider configured" value, and which is exactly what every pre-v13 build
 * effectively had since nothing populated the registry at all.
 */
describe("settingsStore assistant provider", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
    settingsStore.setState({ assistantProviderBaseUrl: "", assistantProviderModel: "" });
  });

  it("defaults to no endpoint and no model", () => {
    expect(settingsStore.getInitialState().assistantProviderBaseUrl).toBe("");
    expect(settingsStore.getInitialState().assistantProviderModel).toBe("");
  });

  it("a v12 blob (predates both keys) migrates to v13 with both defaulted", async () => {
    seed(12, { assistantEnabled: true, autoConstrainMode: "off" });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().assistantProviderBaseUrl).toBe("");
    expect(settingsStore.getState().assistantProviderModel).toBe("");
    // The bump carries the rest of the blob through untouched.
    expect(settingsStore.getState().assistantEnabled).toBe(true);
    expect(settingsStore.getState().autoConstrainMode).toBe("off");
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!).version).toBe(13);
  });

  it("a configured endpoint and model survive hydration", async () => {
    seed(13, {
      assistantProviderBaseUrl: "http://127.0.0.1:11434/v1",
      assistantProviderModel: "qwen3:8b",
    });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().assistantProviderBaseUrl).toBe(
      "http://127.0.0.1:11434/v1",
    );
    expect(settingsStore.getState().assistantProviderModel).toBe("qwen3:8b");
  });

  it("a non-string in a same-version blob coerces to no provider", async () => {
    // `merge` runs on every hydration, not just across a bump — a hand-edited
    // blob must not reach the configure command as a number.
    seed(13, { assistantProviderBaseUrl: 11434, assistantProviderModel: null });
    await settingsStore.persist.rehydrate();
    expect(settingsStore.getState().assistantProviderBaseUrl).toBe("");
    expect(settingsStore.getState().assistantProviderModel).toBe("");
  });

  it("the setters write through to localStorage", () => {
    setProvider("http://127.0.0.1:1234/v1", "llama3");
    expect(settingsStore.getState().assistantProviderBaseUrl).toBe("http://127.0.0.1:1234/v1");
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY)!);
    expect(raw.state.assistantProviderBaseUrl).toBe("http://127.0.0.1:1234/v1");
    expect(raw.state.assistantProviderModel).toBe("llama3");
  });
});

function setProvider(baseUrl: string, model: string): void {
  settingsStore.getState().setAssistantProviderBaseUrl(baseUrl);
  settingsStore.getState().setAssistantProviderModel(model);
}
