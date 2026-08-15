/*
 * Settings store (F-WP3) — snap / show preferences behind the corner-cluster
 * snap popover. Persisted to localStorage under a versioned key so choices
 * survive reloads (prototype 1c snap popover; defaults from its winInit()).
 */
import { createStore, useStore } from "zustand";
import { persist } from "zustand/middleware";
import { coerceTheme, DEFAULT_THEME, type ThemePref } from "@/theme/themes";
import { coerceSnapRadius, DEFAULT_SNAP_RADIUS, type SnapRadiusId } from "@/tools/sketch/snapRadius";
import {
  coerceLengthUnit,
  DEFAULT_LENGTH_UNIT,
  type LengthUnitId,
} from "@/units/lengthUnits";
import type { DevicePref } from "@/viewport/engine/navInput";
import { coerceRenderMode, DEFAULT_RENDER_MODE, type RenderModeId } from "@/viewport/engine/renderModes";

export interface SnapSettings {
  grid: boolean;
  sketchGuideLines: boolean;
  sketchGuidePoints: boolean;
  /** Circle/arc 0/90/180/270° quadrant snaps (M6c parity, default on). */
  quadrant: boolean;
  /** Entity-entity intersection snaps (M6c parity, default on). */
  intersection: boolean;
  /** Nearest-point-on-curve snaps (M6c parity, default on). */
  onCurve: boolean;
  /**
   * Round a CURSOR-placed dimension to the zoom-adaptive quantum while drawing
   * (SP-1). Lives under `snapTo` because it is a snap tier: it REPLACES the grid
   * tier during a draw gesture (grid quantizes x/y independently, a draw gesture
   * means length + angle), and every geometry tier above it still wins. Default
   * on — a drawn length landing on a round number is what makes cursor-drawn
   * geometry usable without typing.
   */
  dimensionRound: boolean;
  /**
   * Snap the cursor onto a ray leaving the chain's last anchor at 0/45/90/135°,
   * or parallel/perpendicular to the direction the chain is travelling in
   * (SP-5.5). Lives under `snapTo` because it is a snap tier: it sits between
   * the geometry ladder and the H/V alignment guides, and loses every tie to
   * them. Default on — it is inert until a gesture has placed an anchor, so it
   * can never move a first click.
   */
  polarTracking: boolean;
  /**
   * RESERVED — not read by any snap path (3D-geometry snapping is unscheduled).
   * Kept so an existing persisted blob merges unchanged and a rollback to a
   * build that still rendered the row is loss-free. Removing the field is a
   * persist-shape change and needs a `version` bump.
   */
  guidePoints3d: boolean;
  /** RESERVED — see `guidePoints3d`. */
  distantEdges: boolean;
}

export interface ShowSettings {
  guidePoints: boolean;
  snappingHints: boolean;
  /**
   * Show the live dimension CHIPS during a draw gesture (SP-1). Default on.
   * Only the chip overlay reads this — the zoom-adaptive rounding it displays is
   * a separate concern with its own pref (`snapTo.dimensionRound`), so turning
   * the chips off never silently changes where geometry lands.
   */
  liveDimensions: boolean;
  /**
   * Show the constraint badges (H/V/Coincident/dimensional chips) on
   * committed sketch geometry. Default on. Purely a display toggle — turning
   * it off never removes a constraint, only its on-canvas glyph.
   */
  constraintChips: boolean;
}

export interface NavigationSettings {
  /**
   * How wheel events are routed. "auto" runs the best-effort trackpad/mouse
   * heuristic; the explicit values pin it when the heuristic guesses wrong.
   * Only the PREFERENCE is persisted — the live detection is runtime state on
   * viewportStore, because persisting a heuristic result across sessions would
   * reintroduce the stale-classification bug it exists to avoid.
   */
  inputDevice: DevicePref;
}

export type SnapKey = keyof SnapSettings;
export type ShowKey = keyof ShowSettings;

export interface SettingsState {
  snapTo: SnapSettings;
  show: ShowSettings;
  /**
   * Experimental: use the WebGPU renderer when the platform supports it. Default
   * false — WebGL is the tested path. Gates the WebGPU code path in renderer.ts.
   */
  experimentalWebGpu: boolean;
  navigation: NavigationSettings;
  /**
   * Viewport body render mode (shaded / shaded+edges / wireframe). Moved off
   * viewportStore (session-only) so it survives reloads; coerced against the
   * render-mode registry both on migrate AND on every hydration (see `merge`
   * below) since a hand-edited or rolled-back localStorage blob can carry an
   * unknown id even at the current version.
   */
  displayMode: RenderModeId;
  /**
   * Appearance PREFERENCE — "system" stays "system" here. The resolved
   * light/dark value is derived at runtime by themeController and deliberately
   * never persisted: storing it would show last session's OS appearance until
   * something poked it, the same stale-classification trap `navigation`
   * documents above. Coerced on every hydration, not just on migrate (see
   * `merge`), since a hand-edited blob can carry an unknown id at any version.
   */
  theme: ThemePref;
  /**
   * Unit every DISPLAYED length is rendered in, and the unit a BARE typed
   * number is read in. Purely presentational: the document, the wire and every
   * persisted coordinate stay in millimetres, so switching this can never
   * change a model — only how its numbers read. Coerced on every hydration, not
   * just on migrate (see `merge`), same as displayMode and theme.
   */
  displayUnit: LengthUnitId;
  /**
   * On-screen reach of every point-like pick — the snap ladder's threshold AND
   * the controller's hit/dimension tolerances (they are one number by design:
   * two reaches would let a click select something the snap marker never
   * offered). TOP-LEVEL, not inside `snapTo`, because `snapTo` is exclusively
   * booleans the popover renders as switches — a numeric id in there would make
   * the switch-row pin test unrepresentable. Coerced on every hydration, not
   * just on migrate (see `merge`), same as displayMode / theme / displayUnit.
   */
  snapRadius: SnapRadiusId;
  setSnap(key: SnapKey, value: boolean): void;
  setShow(key: ShowKey, value: boolean): void;
  setExperimentalWebGpu(value: boolean): void;
  setInputDevice(value: DevicePref): void;
  setDisplayMode(mode: RenderModeId): void;
  setTheme(theme: ThemePref): void;
  setDisplayUnit(unit: LengthUnitId): void;
  setSnapRadius(radius: SnapRadiusId): void;
}

/** Versioned localStorage key (bump `version` on a breaking shape change). */
const STORAGE_KEY = "onecad.settings";

export const settingsStore = createStore<SettingsState>()(
  persist(
    (set) => ({
      snapTo: {
        grid: true,
        sketchGuideLines: true,
        sketchGuidePoints: true,
        quadrant: true,
        intersection: true,
        onCurve: true,
        dimensionRound: true,
        polarTracking: true,
        guidePoints3d: true,
        distantEdges: false,
      },
      show: {
        guidePoints: true,
        snappingHints: true,
        liveDimensions: true,
        constraintChips: true,
      },
      experimentalWebGpu: false,
      navigation: { inputDevice: "auto" },
      displayMode: DEFAULT_RENDER_MODE,
      theme: DEFAULT_THEME,
      displayUnit: DEFAULT_LENGTH_UNIT,
      snapRadius: DEFAULT_SNAP_RADIUS,
      setSnap(key, value) {
        set((s) => ({ snapTo: { ...s.snapTo, [key]: value } }));
      },
      setShow(key, value) {
        set((s) => ({ show: { ...s.show, [key]: value } }));
      },
      setExperimentalWebGpu(value) {
        set({ experimentalWebGpu: value });
      },
      setInputDevice(value) {
        set((s) => ({ navigation: { ...s.navigation, inputDevice: value } }));
      },
      setDisplayMode(mode) {
        set({ displayMode: mode });
      },
      setTheme(theme) {
        set({ theme });
      },
      setDisplayUnit(unit) {
        set({ displayUnit: unit });
      },
      setSnapRadius(radius) {
        set({ snapRadius: radius });
      },
    }),
    {
      name: STORAGE_KEY,
      version: 9,
      // v1 → v2 added the M6c snap types (quadrant / intersection / onCurve).
      // A v1 blob has no keys for them; backfill the on-by-default values so an
      // existing user's popover shows them enabled (parity with a fresh install).
      // v2 → v3 added the navigation section.
      // v3 → v4 moved displayMode here from viewportStore (session-only before);
      // a pre-v4 blob has no key for it at all, so coerce(undefined) → default.
      // v4 → v5 added the appearance preference; a pre-v5 blob has no key, so
      // coerce(undefined) → "system" (follow the OS, matching a fresh install).
      // v5 → v6 added the display unit; a pre-v6 blob has no key, so
      // coerce(undefined) → "mm", which is what every such blob was authored in.
      // v6 → v7 added the live-dimension prefs (SP-1). A pre-v7 blob has neither
      // key; backfill both ON so an existing user gets the same behaviour as a
      // fresh install rather than a silently disabled feature.
      // v7 → v8 added polar tracking + the snap radius (SP-5). polarTracking
      // backfills ON (fresh-install parity; it is inert until a gesture has an
      // anchor, so it cannot surprise anyone on a first click), and snapRadius
      // coerce(undefined) → "m" = 8px, exactly the reach every pre-v8 build
      // hard-coded — so a migrated blob snaps identically until it is changed.
      // v8 → v9 added the constraint-chip visibility toggle. A pre-v9 blob has
      // no key; backfill ON (fresh-install parity — the badges were always on).
      migrate: (persisted, version) => {
        const s = persisted as Partial<SettingsState>;
        if (s && version < 2) {
          s.snapTo = {
            quadrant: true,
            intersection: true,
            onCurve: true,
            ...(s.snapTo as Partial<SnapSettings>),
          } as SnapSettings;
        }
        if (s && version < 3) {
          s.navigation = { inputDevice: "auto", ...(s.navigation as Partial<NavigationSettings>) };
        }
        if (s && version < 4) {
          s.displayMode = coerceRenderMode((s as Partial<SettingsState>).displayMode);
        }
        if (s && version < 5) {
          s.theme = coerceTheme((s as Partial<SettingsState>).theme);
        }
        if (s && version < 6) {
          s.displayUnit = coerceLengthUnit((s as Partial<SettingsState>).displayUnit);
        }
        if (s && version < 7) {
          s.snapTo = { dimensionRound: true, ...(s.snapTo as Partial<SnapSettings>) } as SnapSettings;
          s.show = { liveDimensions: true, ...(s.show as Partial<ShowSettings>) } as ShowSettings;
        }
        if (s && version < 8) {
          s.snapTo = { polarTracking: true, ...(s.snapTo as Partial<SnapSettings>) } as SnapSettings;
          s.snapRadius = coerceSnapRadius((s as Partial<SettingsState>).snapRadius);
        }
        if (s && version < 9) {
          s.show = { constraintChips: true, ...(s.show as Partial<ShowSettings>) } as ShowSettings;
        }
        return s as unknown as SettingsState;
      },
      // `migrate` only runs when the persisted blob's version differs from the
      // current one. A SAME-version blob can still carry a garbage displayMode,
      // theme, displayUnit or snapRadius (hand-edited localStorage, or a rolled-back build
      // that wrote an id a newer registry no longer has) — coerce them here
      // too, on every hydration, not just across a version bump. Mirrors
      // zustand's default shallow-merge shape so nothing else about persist's
      // merge changes.
      merge: (persisted, current) => {
        const merged = { ...current, ...(persisted as Partial<SettingsState>) };
        merged.displayMode = coerceRenderMode(merged.displayMode);
        merged.theme = coerceTheme(merged.theme);
        merged.displayUnit = coerceLengthUnit(merged.displayUnit);
        merged.snapRadius = coerceSnapRadius(merged.snapRadius);
        return merged;
      },
    },
  ),
);

/** Typed selector hook over the vanilla store. */
export function useSettingsStore<T>(selector: (s: SettingsState) => T): T {
  return useStore(settingsStore, selector);
}
