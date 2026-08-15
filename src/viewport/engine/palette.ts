/*
 * Viewport color palette.
 *
 * The engine never hard-codes colors: it reads the design tokens (CSS custom
 * properties emitted by Tailwind `@theme`) via getComputedStyle and caches
 * THREE.Color instances. tokens.css stays the single source of truth.
 *
 * THEME CHANGES: the cache is keyed to nothing — it is simply dropped by
 * `resetPaletteCache()` when the theme flips, after which every getter re-reads
 * the DOM and returns NEW THREE.Color instances. Anything holding a color from
 * before the reset is holding an orphan; that is why every layer implements
 * `refreshColors()` (see engine/README.md) instead of trusting its constructor
 * arguments to stay live.
 *
 * In non-browser contexts (vitest/jsdom, where Tailwind's `@theme` is not
 * processed) the custom properties resolve to empty strings, so each token
 * falls back to an rgb() mirror of the token value. The fallback table is
 * per-theme and selected from `data-theme` on <html>: without that, a dark
 * theme expressed only in CSS would be entirely invisible to the unit suite,
 * and no `refreshColors()` implementation could be tested at all. Fallbacks are
 * rgb(), never `#` hex literals, so the tokens-only hex gate still passes; the
 * browser always overrides them with the real token via getComputedStyle.
 */
import type { ResolvedTheme } from "@/theme/themes";
import * as THREE from "three";

export type TokenName =
  | "--color-border"
  | "--color-border-strong"
  | "--color-canvas"
  | "--color-canvas-sketch"
  | "--color-ink"
  | "--color-ink-5"
  | "--color-sketch-reference"
  | "--color-accent"
  | "--color-sel-bg"
  | "--color-sel-text"
  | "--color-warn"
  | "--color-axis-x"
  | "--color-axis-y"
  | "--color-axis-z"
  | "--color-plane-xy"
  | "--color-plane-xz"
  | "--color-plane-yz"
  | "--color-traffic-close"
  | "--color-body-fill"
  | "--color-body-edge"
  | "--color-body-edge-wire"
  | "--color-sketch-under"
  | "--color-sketch-done"
  | "--color-sketch-sel"
  | "--color-sketch-angle-ref"
  | "--color-sketch-vertex"
  | "--color-sketch-midpoint"
  | "--color-sketch-centroid"
  | "--color-viewport-hover"
  | "--color-overlay-halo";

// rgb() mirrors of the token values in tokens.css (non-browser fallback only).
// MUST be kept in step with tokens.css by hand — nothing enforces it, because
// jsdom never runs the Tailwind pipeline that would produce the real values.
const FALLBACK: Record<ResolvedTheme, Record<TokenName, string>> = {
  light: {
    "--color-border": "rgb(226, 228, 232)",
    "--color-border-strong": "rgb(216, 219, 224)",
    "--color-canvas": "rgb(234, 236, 239)",
    "--color-canvas-sketch": "rgb(244, 247, 252)",
    "--color-ink": "rgb(27, 29, 33)",
    "--color-ink-5": "rgb(138, 145, 156)",
    "--color-sketch-reference": "rgb(107, 122, 143)",
    "--color-accent": "rgb(46, 111, 224)",
    "--color-sel-bg": "rgb(225, 235, 251)",
    "--color-sel-text": "rgb(29, 79, 168)",
    "--color-warn": "rgb(178, 107, 16)",
    "--color-axis-x": "rgb(196, 92, 92)",
    "--color-axis-y": "rgb(92, 160, 92)",
    "--color-axis-z": "rgb(80, 120, 190)",
    "--color-plane-xy": "rgb(80, 120, 190)",
    "--color-plane-xz": "rgb(217, 134, 60)",
    "--color-plane-yz": "rgb(92, 160, 92)",
    // Traffic-light close (see tokens.css) — the destructive trim-ghost color.
    "--color-traffic-close": "rgb(255, 95, 87)",
    "--color-body-fill": "rgb(169, 174, 182)",
    "--color-body-edge": "rgb(58, 63, 71)",
    "--color-body-edge-wire": "rgb(17, 20, 24)",
    "--color-sketch-under": "rgb(45, 127, 249)",
    "--color-sketch-done": "rgb(31, 157, 85)",
    "--color-sketch-sel": "rgb(217, 119, 6)",
    "--color-sketch-angle-ref": "rgb(139, 92, 246)",
    "--color-sketch-vertex": "rgb(45, 127, 249)",
    "--color-sketch-midpoint": "rgb(168, 85, 247)",
    "--color-sketch-centroid": "rgb(14, 165, 164)",
    "--color-viewport-hover": "rgb(0, 172, 193)",
    "--color-overlay-halo": "rgb(251, 252, 254)",
  },
  dark: {
    "--color-border": "rgb(52, 56, 63)",
    "--color-border-strong": "rgb(67, 73, 82)",
    "--color-canvas": "rgb(35, 38, 43)",
    "--color-canvas-sketch": "rgb(27, 33, 48)",
    "--color-ink": "rgb(232, 234, 237)",
    "--color-ink-5": "rgb(127, 135, 145)",
    "--color-sketch-reference": "rgb(139, 155, 176)",
    "--color-accent": "rgb(77, 139, 240)",
    "--color-sel-bg": "rgb(29, 49, 87)",
    "--color-sel-text": "rgb(156, 194, 255)",
    "--color-warn": "rgb(224, 164, 74)",
    "--color-axis-x": "rgb(224, 122, 122)",
    "--color-axis-y": "rgb(120, 200, 120)",
    "--color-axis-z": "rgb(111, 157, 232)",
    "--color-plane-xy": "rgb(111, 157, 232)",
    "--color-plane-xz": "rgb(224, 160, 92)",
    "--color-plane-yz": "rgb(120, 200, 120)",
    // Traffic lights are macOS constants — identical in both themes.
    "--color-traffic-close": "rgb(255, 95, 87)",
    "--color-body-fill": "rgb(125, 131, 140)",
    "--color-body-edge": "rgb(34, 38, 44)",
    "--color-body-edge-wire": "rgb(207, 212, 219)",
    "--color-sketch-under": "rgb(90, 162, 255)",
    "--color-sketch-done": "rgb(67, 193, 122)",
    "--color-sketch-sel": "rgb(240, 162, 74)",
    "--color-sketch-angle-ref": "rgb(167, 139, 250)",
    "--color-sketch-vertex": "rgb(90, 162, 255)",
    "--color-sketch-midpoint": "rgb(192, 132, 252)",
    "--color-sketch-centroid": "rgb(45, 212, 197)",
    "--color-viewport-hover": "rgb(38, 198, 218)",
    "--color-overlay-halo": "rgb(17, 20, 26)",
  },
};

/**
 * Cache, keyed BY THEME.
 *
 * A single flat cache would be correct only if `resetPaletteCache()` were called
 * at exactly the right moment every time — and anything that reads a color
 * before that (an engine still constructing, a layer built during the async
 * renderer init, a module evaluated before the theme was stamped) would poison
 * it with the wrong theme's values and go undetected, because nothing throws.
 * Keying by theme makes that whole class of ordering bug unrepresentable: a read
 * taken while the document says "dark" can never hand back a light color.
 *
 * `resetPaletteCache()` still exists and is still called on a theme change,
 * because live MATERIALS must be re-read — but correctness no longer depends on
 * it having been called.
 */
const cache = new Map<ResolvedTheme, Map<TokenName, THREE.Color>>();

/** The theme the document is currently wearing (themeController's DOM stamp). */
function currentTheme(): ResolvedTheme {
  if (typeof document === "undefined" || !document.documentElement) return "light";
  return document.documentElement.dataset.theme === "dark" ? "dark" : "light";
}

function readToken(name: TokenName): string {
  if (typeof document !== "undefined" && typeof getComputedStyle === "function") {
    const value = getComputedStyle(document.documentElement)
      .getPropertyValue(name)
      .trim();
    if (value) return value;
  }
  return FALLBACK[currentTheme()][name];
}

function tokenColor(name: TokenName): THREE.Color {
  const theme = currentTheme();
  let byToken = cache.get(theme);
  if (!byToken) {
    byToken = new Map();
    cache.set(theme, byToken);
  }
  let color = byToken.get(name);
  if (!color) {
    color = new THREE.Color(readToken(name));
    byToken.set(name, color);
  }
  return color;
}

/** Named viewport colors, resolved from design tokens on first access. */
export const palette = {
  /** Grid minor lines. */
  gridMinor: () => tokenColor("--color-border"),
  /** Grid major lines. */
  gridMajor: () => tokenColor("--color-border-strong"),
  /** Renderer clear color = base viewport canvas background. */
  clear: () => tokenColor("--color-canvas"),
  /** Neutral body face material. */
  bodyNeutral: () => tokenColor("--color-body-fill"),
  /** Body edge lines (shaded+edges render mode) — near-black in BOTH themes. */
  bodyEdge: () => tokenColor("--color-body-edge"),
  /** Wireframe-mode edge lines — no shaded face behind them, so this one DOES invert (light in dark theme) for legibility. */
  bodyEdgeWire: () => tokenColor("--color-body-edge-wire"),
  /** Neutral gray for reference/overlay layers — deliberately NOT the body fill, which is lighter. */
  referenceNeutral: () => tokenColor("--color-ink-5"),
  /** Hover accent (face + edge highlight) — previews/UI hover states. */
  hoverAccent: () => tokenColor("--color-accent"),
  /** Cyan 3D-viewport hover (hovering a body/face/edge in the scene) — distinct from hoverAccent. */
  hover3d: () => tokenColor("--color-viewport-hover"),
  /**
   * Outline behind a screen-space overlay handle (the value arrow). INVERTS
   * between themes: the arrow's own fill is `--color-accent`, which is also the
   * selected-face fill, so only an opposite-tone outline separates the two.
   */
  overlayHalo: () => tokenColor("--color-overlay-halo"),
  /** Selected face tint (UI-side; panels/lists). */
  selectedTint: () => tokenColor("--color-sel-bg"),
  /**
   * Selected face/body FILL in the 3D viewport — the saturated accent blue.
   * Deliberately NOT `--color-sel-bg`: that pale list-row tint all but vanishes
   * blended over the gray body fill, so a selected face read as unselected.
   */
  selected3d: () => tokenColor("--color-accent"),
  /** Selected edge / outline color. */
  selectedEdge: () => tokenColor("--color-sel-text"),

  // ── Sketch entity colors, by constraint state (F-WP6) ──
  /** Under-constrained sketch geometry (sketch base blue). */
  sketchUnder: () => tokenColor("--color-sketch-under"),
  /** Fully-constrained sketch geometry — green. */
  sketchFull: () => tokenColor("--color-sketch-done"),
  /** Selected sketch geometry — orange. */
  sketchSelected: () => tokenColor("--color-sketch-sel"),
  /** The chain segment a live angle chip is measured against — violet. */
  sketchAngleRef: () => tokenColor("--color-sketch-angle-ref"),
  /** Construction (dashed) geometry. */
  sketchConstruction: () => tokenColor("--color-ink-5"),
  /** Host-face reference geometry (`referenceLocked`) — SOLID, recessive. */
  sketchReference: () => tokenColor("--color-sketch-reference"),
  /** Conflicting / over-constrained geometry. */
  sketchConflict: () => tokenColor("--color-warn"),
  /** Sketch plane tint quad + sketch canvas background. */
  sketchPlane: () => tokenColor("--color-canvas-sketch"),
  /** Destructive overlay (trim doomed-piece ghost). */
  destructive: () => tokenColor("--color-traffic-close"),
  /** Vertex/corner marker ring — uniform across every entity, regardless of constraint state. */
  sketchVertex: () => tokenColor("--color-sketch-vertex"),
  /** Edge midpoint dot. */
  sketchMidpoint: () => tokenColor("--color-sketch-midpoint"),
  /** Closed-loop centroid dot. */
  sketchCentroid: () => tokenColor("--color-sketch-centroid"),

  // ── Origin axis triad (always-visible XYZ at the origin) ──
  /** +X axis leg. */
  axisX: () => tokenColor("--color-axis-x"),
  /** +Y axis leg. */
  axisY: () => tokenColor("--color-axis-y"),
  /** +Z axis leg. */
  axisZ: () => tokenColor("--color-axis-z"),

  // ── Sketch plane picker (origin-plane quads, pre-sketch plane-select gizmo) ──
  /** XY plane quad. */
  planeXY: () => tokenColor("--color-plane-xy"),
  /** XZ plane quad. */
  planeXZ: () => tokenColor("--color-plane-xz"),
  /** YZ plane quad. */
  planeYZ: () => tokenColor("--color-plane-yz"),
};

/**
 * Test / theme-change seam: drop every cached color so the next read re-samples
 * the DOM. Needed when the token VALUES themselves change under a fixed theme
 * (a stylesheet hot-reload in dev, or a test rewriting tokens); a plain theme
 * flip is already handled by the per-theme keying above.
 */
export function resetPaletteCache(): void {
  cache.clear();
}
