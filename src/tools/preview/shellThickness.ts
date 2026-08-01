/*
 * Shell thickness interaction math (PURE, framework-free).
 *
 * Shell mirrors the fillet-radius interaction: a vertical pointer drag on the
 * armed body adjusts the wall thickness (up-drag grows it), with an editable mm
 * chip. There is NO cheap-and-honest L1 mesh for a shell (hollowing needs OCCT),
 * so the tool is chip + status-hint driven — the exact shelled body arrives from
 * the backend on commit. The drag mapping is shared with fillet
 * (`radiusFromDrag`); this module owns the thickness defaults + the re-edit parse.
 */
import { LENGTH_SUFFIX, formatLength } from "@/units/format";

export const DEFAULT_SHELL_THICKNESS = 2;

/** Minimum shell thickness (world units); a zero-thickness shell is a no-op. */
export const MIN_SHELL_THICKNESS = 0.1;

/**
 * Format a thickness for the mono chip. Shares the W2-A length formatter with
 * fillet's `formatMm`, so both chips render a number identically (`2`, not
 * `2.0`). `thicknessFromValueText` still parses this AND the RUST-composed
 * `valueText` ("2.0 mm") that seeds a re-edit — both round-trips are pinned.
 */
export function formatThickness(value: number): string {
  return `${formatLength(value)} ${LENGTH_SUFFIX}`;
}

/**
 * Parse a shell feature's display text ("2.0 mm") back to a thickness (re-edit
 * seed; mirrors fillet's `radiusFromValueText`). Non-numeric / non-positive text
 * falls back to the default thickness.
 */
export function thicknessFromValueText(text: string, fallback = DEFAULT_SHELL_THICKNESS): number {
  const n = Number.parseFloat(text);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}
