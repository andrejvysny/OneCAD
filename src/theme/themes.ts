/*
 * Theme registry — the single descriptor table appearance is driven from.
 *
 * A theme is DATA, not a switch statement, for the same reason render modes are
 * (see renderModes.ts): the chrome control, the persisted setting and the
 * viewport palette all read this table, so they cannot disagree about what a
 * theme is called or which ids exist.
 *
 * PREFERENCE vs RESOLVED is the load-bearing distinction. `ThemePref` is what
 * the user chose and what gets persisted; `ResolvedTheme` is what the DOM is
 * actually wearing right now. "system" resolves against the OS at runtime and
 * is deliberately NEVER stored in resolved form — persisting a resolved value
 * would make the app show last session's OS appearance until something poked
 * it, the same stale-classification bug settingsStore's navigation section
 * documents for the input-device heuristic.
 */

/** What the user picked. Persisted. */
export type ThemePref = "light" | "dark" | "system";

/** What the DOM is wearing. Runtime only — derived, never stored. */
export type ResolvedTheme = "light" | "dark";

export interface ThemeDef {
  id: ThemePref;
  label: string;
}

export const THEMES: Record<ThemePref, ThemeDef> = {
  light: { id: "light", label: "Light" },
  dark: { id: "dark", label: "Dark" },
  system: { id: "system", label: "System" },
};

/** Control order for the appearance toggle (also the canonical id enumeration). */
export const THEME_ORDER: readonly ThemePref[] = ["light", "dark", "system"];

/**
 * Follow the OS by default: a CAD app that ignores the desktop appearance on
 * first launch reads as broken, and any explicit choice overrides this forever
 * after (it is persisted, this default is not).
 */
export const DEFAULT_THEME: ThemePref = "system";

/**
 * Narrow an untrusted value (persisted setting, hand-edited localStorage, a
 * rolled-back build that wrote an id this registry no longer has) to a known
 * preference, falling back to the default rather than throwing — an unknown
 * theme must never leave the app with no appearance at all.
 */
export function coerceTheme(v: unknown): ThemePref {
  if (typeof v === "string" && Object.prototype.hasOwnProperty.call(THEMES, v)) {
    return v as ThemePref;
  }
  return DEFAULT_THEME;
}

/** Collapse a preference plus the current OS appearance into what to render. */
export function resolveTheme(pref: ThemePref, systemDark: boolean): ResolvedTheme {
  if (pref === "system") return systemDark ? "dark" : "light";
  return pref;
}
