/*
 * The overlay halo's whole job is to be the token the value arrow is NOT.
 *
 * `palette.hoverAccent()` (the arrow's fill) and `palette.selected3d()` (the
 * selected-face fill it is drawn on top of) both read `--color-accent` — the
 * same value — so the arrow is invisible on the very face it operates on unless
 * something separates them. That something is the halo, and it only works if it
 * stays far from the accent in BOTH themes: it inverts (white on the light
 * canvas, near-black on the dark one) exactly so it never tracks the surface
 * underneath it.
 *
 * jsdom runs no Tailwind pipeline, so these read palette's per-theme fallback
 * table — which is hand-kept in step with tokens.css, and is the thing this
 * would catch drifting.
 */
import { describe, it, expect, afterEach } from "vitest";
import { palette, resetPaletteCache } from "./palette";

function setTheme(theme: "light" | "dark"): void {
  document.documentElement.dataset.theme = theme;
  resetPaletteCache();
}

/** Rough perceptual distance, enough to tell "same blue" from "opposite tone". */
function luminance(hex: number): number {
  const r = ((hex >> 16) & 0xff) / 255;
  const g = ((hex >> 8) & 0xff) / 255;
  const b = (hex & 0xff) / 255;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

afterEach(() => setTheme("light"));

describe("overlay halo", () => {
  it("is never the accent the arrow and the selected face share", () => {
    for (const theme of ["light", "dark"] as const) {
      setTheme(theme);
      expect(palette.hoverAccent().getHex()).toBe(palette.selected3d().getHex());
      expect(palette.overlayHalo().getHex()).not.toBe(palette.hoverAccent().getHex());
    }
  });

  it("sits on the opposite side of the accent's luminance in each theme", () => {
    setTheme("light");
    const lightHalo = palette.overlayHalo().getHex();
    expect(luminance(lightHalo)).toBeGreaterThan(luminance(palette.hoverAccent().getHex()) + 0.3);

    setTheme("dark");
    const darkHalo = palette.overlayHalo().getHex();
    expect(luminance(darkHalo)).toBeLessThan(luminance(palette.hoverAccent().getHex()) - 0.3);

    // Non-vacuity: the token really does invert, rather than one theme's value
    // happening to satisfy both bounds.
    expect(darkHalo).not.toBe(lightHalo);
  });
});
