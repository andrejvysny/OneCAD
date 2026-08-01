/*
 * Units display layer (W2-A) — the ONE place a length or an area becomes text,
 * and the ONE place typed text becomes a length.
 *
 * SCOPE, deliberately narrow: this is a DISPLAY concern only. The document, the
 * wire, and every stored value stay in **millimetres**; there is no unit
 * preference and nothing here is persisted. `parseLength` accepts a unit suffix
 * as a typing convenience and converts it BACK to mm immediately, so the value
 * that leaves this module is always mm.
 *
 * Why the asymmetry between `formatLength` and `formatArea`:
 *   - `formatLength` returns a BARE number string. It is the formatter behind
 *     the editable `DimensionInput` field, whose unit lives in a separate
 *     `suffix` span — putting the unit inside the value would make the field
 *     round-trip its own suffix on every keystroke.
 *   - `formatArea` returns the value WITH `mm²`. An area is never editable, so
 *     it has no bare-number twin to keep in step.
 * Callers that need a length WITH its unit compose `LENGTH_SUFFIX` themselves
 * (see `tools/preview/filletRadius.ts formatMm`).
 */

/** Unit the document (and therefore every bare value) is expressed in. */
export const LENGTH_SUFFIX = "mm";

/** Unit for a face area — the square of `LENGTH_SUFFIX`. */
export const AREA_SUFFIX = "mm²";

/**
 * Format a length (mm) for display: fixed to 3 decimals, then trailing zeros
 * (and a bare trailing decimal point) are stripped.
 *
 * `toFixed(3).replace(/\.?0+$/,"")` looks equivalent but corrupts whole numbers —
 * `"100".replace(/\.?0+$/,"")` eats the trailing zeros of "100" itself, leaving
 * "1". Stripping only inside a capture group anchored AFTER the decimal point is
 * what keeps the integer part intact (the regression the "100" test pins).
 */
export function formatLength(mm: number): string {
  if (!Number.isFinite(mm)) return String(mm);
  return mm
    .toFixed(3)
    .replace(/(\.\d*?)0+$/, "$1")
    .replace(/\.$/, "");
}

/** Format a face area (mm²) for display, unit included (`"800 mm²"`). */
export function formatArea(mm2: number): string {
  return `${formatLength(mm2)} ${AREA_SUFFIX}`;
}

/** Recognised input suffixes → millimetres. Bare input is already mm. */
const UNIT_TO_MM: Readonly<Record<string, number>> = {
  mm: 1,
  cm: 10,
  m: 1000,
  in: 25.4,
};

/**
 * A signed decimal (optionally in exponent form) followed by an OPTIONAL unit.
 * The exponent is part of the number group, so `"1e3"` parses as 1000 rather
 * than as 1 with an unknown `e3` unit — while `"1em"` still fails (the exponent
 * needs digits), which is the behaviour a typo should get.
 */
const LENGTH_RE = /^([+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)\s*([a-zA-Z]*)$/;

/**
 * Parse a typed length into MILLIMETRES. Accepts a bare number (`"25"` ⇒ 25 mm)
 * and a suffixed one (`"25mm"`, `"2.5 cm"`, `"0.5m"`, `"1 in"`), case- and
 * whitespace-insensitive.
 *
 * Returns `null` — never a silent 0 or a partial read — for anything else,
 * including an unknown unit (`"25 furlongs"`) and trailing junk (`"25abc"`).
 * `Number.parseFloat` would happily return 25 for both; a measurement UI must
 * refuse a value it did not fully understand rather than commit a guess.
 */
export function parseLength(text: string): number | null {
  const m = LENGTH_RE.exec(text.trim());
  if (!m) return null;
  const n = Number.parseFloat(m[1]);
  if (!Number.isFinite(n)) return null;
  const unit = m[2].toLowerCase();
  if (unit === "") return n;
  const factor = UNIT_TO_MM[unit];
  return factor === undefined ? null : n * factor;
}
