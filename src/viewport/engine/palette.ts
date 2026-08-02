/*
 * Viewport color palette.
 *
 * The engine never hard-codes colors: it reads the design tokens (CSS custom
 * properties emitted by Tailwind `@theme`) ONCE via getComputedStyle and caches
 * THREE.Color instances. tokens.css stays the single source of truth.
 *
 * In non-browser contexts (vitest/jsdom, where Tailwind's `@theme` is not
 * processed) the custom properties resolve to empty strings, so each token
 * falls back to an rgb() mirror of the token value. These fallbacks are rgb(),
 * never `#` hex literals, so the tokens-only hex gate still passes; the browser
 * always overrides them with the real token via getComputedStyle.
 */
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
  | "--color-body-edge";

// rgb() mirrors of the token values in tokens.css (non-browser fallback only).
const FALLBACK: Record<TokenName, string> = {
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
};

let cache: Map<TokenName, THREE.Color> | null = null;

function readToken(name: TokenName): string {
  if (typeof document !== "undefined" && typeof getComputedStyle === "function") {
    const value = getComputedStyle(document.documentElement)
      .getPropertyValue(name)
      .trim();
    if (value) return value;
  }
  return FALLBACK[name];
}

function tokenColor(name: TokenName): THREE.Color {
  if (!cache) cache = new Map();
  let color = cache.get(name);
  if (!color) {
    color = new THREE.Color(readToken(name));
    cache.set(name, color);
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
  /** Body edge lines. */
  bodyEdge: () => tokenColor("--color-body-edge"),
  /** Neutral gray for reference/overlay layers — deliberately NOT the body fill, which is lighter. */
  referenceNeutral: () => tokenColor("--color-ink-5"),
  /** Hover accent (face + edge highlight). */
  hoverAccent: () => tokenColor("--color-accent"),
  /** Selected face tint. */
  selectedTint: () => tokenColor("--color-sel-bg"),
  /** Selected edge / outline color. */
  selectedEdge: () => tokenColor("--color-sel-text"),

  // ── Sketch entity colors, by constraint state (F-WP6) ──
  /** Under-constrained sketch geometry (the working accent). */
  sketchUnder: () => tokenColor("--color-accent"),
  /** Fully-constrained sketch geometry. */
  sketchFull: () => tokenColor("--color-ink"),
  /** Selected sketch geometry. */
  sketchSelected: () => tokenColor("--color-sel-text"),
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

/** Test / theme-change seam: drop the cache so colors re-read from the DOM. */
export function resetPaletteCache(): void {
  cache = null;
}
