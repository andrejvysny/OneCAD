/*
 * Shared dimension-value display formatting (U6) — every place that shows a
 * constraint's numeric value (DimensionInput's text field, badgeLayout's
 * glyph, ConstraintList's value column) goes through this so they render the
 * same string for the same number. Up to 3 decimals, trailing zeros trimmed.
 */

/**
 * Formats a dimension value: fixed to 3 decimals, then trailing zeros (and a
 * bare trailing decimal point) are stripped. `toFixed(3).replace(/\.?0+$/,"")`
 * looks equivalent but corrupts whole numbers — "100".replace(/\.?0+$/,"")
 * eats the trailing zeros of "100" itself, leaving "1". Stripping only inside
 * a capture group anchored after the decimal point avoids touching the
 * integer part.
 */
export function formatDimensionValue(v: number): string {
  if (!Number.isFinite(v)) return String(v);
  return v
    .toFixed(3)
    .replace(/(\.\d*?)0+$/, "$1")
    .replace(/\.$/, "");
}
