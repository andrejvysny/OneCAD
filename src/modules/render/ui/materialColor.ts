/*
 * `Color3` (linear-light floats, `model/material.ts`) ↔ what a browser control
 * speaks.
 *
 * TWO CONVERSIONS, both necessary and both easy to get wrong:
 *
 *  TRANSFER FUNCTION. `base_color` is LINEAR light; a CSS colour and an
 *  `<input type="color">` value are sRGB-ENCODED. Writing the linear float
 *  straight into a byte would show a swatch visibly darker than the material it
 *  names (0.5 linear is ~0.74 sRGB, not 0.5), and reading a picked colour back
 *  without decoding would store a value the renderer then brightens a second
 *  time. `srgbEncode`/`srgbDecode` are the IEC 61966-2-1 pair.
 *
 *  NO HEX LITERALS. `tokens.css` is the sole source of design colours and the
 *  repo greps `src` for `#rrggbb`-shaped strings. Every string here is BUILT at
 *  runtime from numbers, and the neutral fallback comes from `OPENPBR_DEFAULTS`
 *  — material colour is document data, not a design token, so it must not reach
 *  for one either.
 */
import { OPENPBR_DEFAULTS, type Color3 } from "../model/material";

const clamp01 = (v: number): number => (Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0);

/** Linear → sRGB (IEC 61966-2-1). */
export function srgbEncode(linear: number): number {
  const v = clamp01(linear);
  return v <= 0.0031308 ? v * 12.92 : 1.055 * Math.pow(v, 1 / 2.4) - 0.055;
}

/** sRGB → linear, the exact inverse of {@link srgbEncode}. */
export function srgbDecode(encoded: number): number {
  const v = clamp01(encoded);
  return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
}

const toByte = (linear: number): number => Math.round(srgbEncode(linear) * 255);

/** A swatch's `style.background`. `rgb()` rather than a hex string, same
 *  reasoning `viewport/engine/palette.ts` gives for its own fallbacks. */
export function colorToCss(c: Color3 | undefined): string {
  const [r, g, b] = c ?? OPENPBR_DEFAULTS.base.base_color;
  return `rgb(${toByte(r)}, ${toByte(g)}, ${toByte(b)})`;
}

/** `<input type="color">` only speaks a 6-digit hex string; this builds one. */
export function colorToInputValue(c: Color3 | undefined): string {
  const [r, g, b] = c ?? OPENPBR_DEFAULTS.base.base_color;
  const hex = (v: number) => toByte(v).toString(16).padStart(2, "0");
  return `#${hex(r)}${hex(g)}${hex(b)}`;
}

/** The inverse: a picked `#rrggbb` back to linear-light floats. */
export function inputValueToColor(value: string): Color3 {
  const raw = value.trim().replace("#", "");
  const full =
    raw.length === 3
      ? raw
          .split("")
          .map((ch) => ch + ch)
          .join("")
      : raw;
  const n = Number.parseInt(full, 16);
  if (full.length !== 6 || !Number.isFinite(n)) {
    // An unparseable value is the control's problem, not the document's: keep
    // the spec default rather than writing NaN into a material.
    const d = OPENPBR_DEFAULTS.base.base_color;
    return [d[0], d[1], d[2]];
  }
  return [
    srgbDecode(((n >> 16) & 255) / 255),
    srgbDecode(((n >> 8) & 255) / 255),
    srgbDecode((n & 255) / 255),
  ];
}
