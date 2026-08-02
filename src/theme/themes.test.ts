/*
 * Theme registry: table self-consistency, the preference→resolved matrix, and
 * coercion of untrusted persisted values.
 */
import { describe, it, expect } from "vitest";
import {
  THEMES,
  THEME_ORDER,
  DEFAULT_THEME,
  coerceTheme,
  resolveTheme,
  type ThemePref,
} from "./themes";

describe("theme registry", () => {
  it("every entry's key matches its id", () => {
    for (const [key, def] of Object.entries(THEMES)) {
      expect(def.id).toBe(key);
    }
  });

  it("THEME_ORDER covers every registered theme exactly once", () => {
    expect([...THEME_ORDER].sort()).toEqual(Object.keys(THEMES).sort());
  });

  it("labels are the exact strings the chrome renders", () => {
    expect(THEMES.light.label).toBe("Light");
    expect(THEMES.dark.label).toBe("Dark");
    expect(THEMES.system.label).toBe("System");
  });

  it("the default is a registered theme", () => {
    expect(THEMES[DEFAULT_THEME]).toBeDefined();
  });
});

describe("resolveTheme", () => {
  it("an explicit preference ignores the OS entirely", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("light", false)).toBe("light");
    expect(resolveTheme("dark", true)).toBe("dark");
    expect(resolveTheme("dark", false)).toBe("dark");
  });

  it('"system" follows the OS', () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });

  it("never resolves to a non-renderable value", () => {
    for (const pref of THEME_ORDER) {
      for (const systemDark of [true, false]) {
        expect(["light", "dark"]).toContain(resolveTheme(pref, systemDark));
      }
    }
  });
});

describe("coerceTheme", () => {
  it("passes every registered id through unchanged", () => {
    for (const id of THEME_ORDER) {
      expect(coerceTheme(id)).toBe(id);
    }
  });

  it("falls back to the default for junk rather than throwing", () => {
    const junk: unknown[] = [
      undefined,
      null,
      "",
      "solarized",
      "Dark", // case matters
      42,
      {},
      [],
      true,
    ];
    for (const v of junk) {
      expect(coerceTheme(v)).toBe(DEFAULT_THEME);
    }
  });

  it("does not accept inherited Object.prototype keys", () => {
    // hasOwnProperty guard: "constructor"/"toString" are truthy on a bare
    // `v in THEMES` check and would otherwise slip through as valid ids.
    expect(coerceTheme("constructor")).toBe(DEFAULT_THEME);
    expect(coerceTheme("toString")).toBe(DEFAULT_THEME);
    expect(coerceTheme("__proto__")).toBe(DEFAULT_THEME);
  });

  it("round-trips a coerced value back through resolveTheme", () => {
    const pref: ThemePref = coerceTheme("nonsense");
    expect(["light", "dark"]).toContain(resolveTheme(pref, false));
  });
});
