/*
 * liveDimParse — reading and writing ONE live-dimension chip's text (PURE).
 *
 * Extracted from `LiveDimField.tsx` so the CONTROLLER can read the same number
 * the field does. Two callers, one parser, by construction:
 *   - the chip, on Enter / Tab / blur, where a refusal flashes the field;
 *   - `SketchController`, on every keystroke, where a parseable value becomes a
 *     PROVISIONAL lock so the rubber band previews the typed number while it is
 *     being typed (UX review S7). A refusal there is silent — the user is
 *     mid-number, and "3." is not a mistake.
 *
 * No React, no store: it takes the chip's own facts and the display unit.
 */
import { formatChipLength, formatUnitless, lengthSuffix, parseLength } from "@/units/format";
import type { LengthUnitId } from "@/units/lengthUnits";
import { foldSigned180 } from "./liveDimension";
import type { DimDomain } from "./liveDimension";

/** Polygon side count bounds — below 3 there is no polygon, and past 12 the ring
 *  reads as a circle while costing a constraint per side. */
export const MIN_SIDES = 3;
export const MAX_SIDES = 12;

/** The chip facts a parse/format needs — the intersection of `ToolDimension`
 *  and the store's `LiveDimChipField`, so both can be passed directly. */
export interface ParsableChip {
  domain: DimDomain;
  /** Authors a driving constraint? Also what separates a chained leg's TURN
   *  angle from an arc's sweep angle — see {@link turnFromCornerAngle}. */
  drives: boolean;
  /** mm · DEGREES · integer count, per `domain`. */
  value: number;
}

/** A parse outcome. `value: null` = nothing lockable typed (an empty field still
 *  Tabs on); `ok: false` = typed something the chip refuses, which flashes. */
export type ParsedChip = { ok: true; value: number | null } | { ok: false };

/**
 * A chained leg's angle field STORES (and drives geometry/authoring with) the
 * SIGNED turn away from continuing straight (`liveDimFrames.ts`'s
 * `segmentFrame` — 0° = straight, ±180° = doubled back) because that is what
 * `liveDimConstraints.ts` needs to match the committed `Angle` constraint
 * bit-for-bit. But the dashed arc preview (`angleArcPreview.ts`'s
 * `arcPreviewSweep`) necessarily shows the VISUAL CORNER angle between the
 * two rays meeting at the vertex — 180° for a straight line, 90° for a
 * square corner — which is a DIFFERENT number (`corner = 180 − |turn|`).
 * Showing the raw turn value next to that arc reads as wrong (a 71° chip
 * next to a dashed sweep that visibly spans ~109°). These two functions
 * convert ONLY at the display/input boundary — the chip's `value` itself, and
 * everything downstream of it (geometry, authoring), stays the turn value.
 *
 * The corner angle is unsigned (it can't tell CW from CCW), so recovering a
 * turn from a freshly-typed corner needs a sign from somewhere — the CURRENT
 * (pre-edit) chip value's own sign, i.e. whichever side the gesture is
 * already on. Same "zero reads positive" convention as `liveDimFrames.ts`'s
 * `side()`.
 */
export function cornerAngleOf(turnDeg: number): number {
  return 180 - Math.abs(turnDeg);
}

export function turnFromCornerAngle(cornerDeg: number, currentTurnDeg: number): number {
  const sign = currentTurnDeg < 0 ? -1 : 1;
  return foldSigned180(sign * (180 - cornerDeg));
}

/**
 * Read the field's text in its own domain. Lengths accept a unit suffix and emit
 * MILLIMETRES; angles and counts are unit-blind (a degree is a degree in an inch
 * session). `Number` rather than `parseFloat` on purpose — `parseFloat("25abc")`
 * silently commits 25, and a measurement field must refuse what it only partly
 * understood.
 */
export function parseChipValue(
  chip: ParsableChip,
  text: string,
  unit: LengthUnitId,
): ParsedChip {
  const raw = text.trim();
  if (raw === "") return { ok: true, value: null };
  if (chip.domain === "length") {
    const mm = parseLength(raw, unit);
    return mm !== null && mm > 0 ? { ok: true, value: mm } : { ok: false };
  }
  const n = Number(raw);
  if (!Number.isFinite(n)) return { ok: false };
  // A count is clamped, not refused: "20 sides" is a legible intent, "20abc" is not.
  if (chip.domain === "count") {
    return { ok: true, value: Math.min(MAX_SIDES, Math.max(MIN_SIDES, Math.round(n))) };
  }
  // The one angle with a RANGE is the arc's sweep, where 0 and 360 both mean "no
  // arc". It is identifiable as the angle field that authors nothing — a
  // chained leg types the VISUAL CORNER angle (see `turnFromCornerAngle`).
  if (!chip.drives) return n > 0 && n < 360 ? { ok: true, value: n } : { ok: false };
  return { ok: true, value: turnFromCornerAngle(n, chip.value) };
}

/**
 * A live chip's DISPLAY text — fixed precision, never trimmed.
 *
 * `formatLength` trims trailing zeros, which is right for a settled dimension
 * field and wrong for a pair of chips tracking the cursor together: the 2026-09-14
 * review caught a rectangle reading `W 30 mm` beside `H 16.225 mm` at the same
 * moment (S15). A live pair must share one precision, so millimetres are pinned
 * at 2 decimals (0.01 mm is finer than any reachable zoom quantum) and every
 * other unit keeps its own declared precision, fixed.
 */
export function formatChipValue(chip: ParsableChip, unit: LengthUnitId): string {
  if (chip.domain === "length") return formatChipLength(chip.value, unit);
  if (chip.domain === "count") return String(Math.round(chip.value));
  if (chip.domain === "angle" && chip.drives) return formatUnitless(cornerAngleOf(chip.value));
  return formatUnitless(chip.value);
}

/** The chip's own unit suffix (`mm` / `in` / `°` / none). */
export function chipSuffix(chip: ParsableChip, unit: LengthUnitId): string {
  if (chip.domain === "length") return lengthSuffix(unit);
  return chip.domain === "angle" ? "°" : "";
}

// Re-exported so the field imports ONE module.
export { formatChipLength };
