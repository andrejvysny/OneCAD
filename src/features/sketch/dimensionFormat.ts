/*
 * Shared dimension-value display formatting (U6) — every place that shows a
 * constraint's numeric value (DimensionInput's text field, badgeLayout's
 * glyph, ConstraintList's value column) goes through this so they render the
 * same string for the same number.
 *
 * W2-A: the implementation moved to `@/units/format` so ONE formatter serves
 * sketch dimensions, model-tool chips, and the measure overlay alike. This file
 * stays as a delegating shim purely to keep the existing import sites stable —
 * new code should import `formatLength` directly.
 */
export { formatLength as formatDimensionValue } from "@/units/format";
