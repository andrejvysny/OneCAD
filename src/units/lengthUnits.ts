/*
 * Length-unit registry (WP-C2) — the single descriptor table the DISPLAY unit
 * is driven from.
 *
 * A unit is DATA, not a switch statement, for the same reason render modes and
 * themes are (see `viewport/engine/renderModes.ts`, `theme/themes.ts`): the
 * picker control, the persisted setting, the formatter and the parser all read
 * this table, so they cannot disagree about which ids exist, what they are
 * called, or what one of them is worth.
 *
 * LOAD-BEARING: this is a DISPLAY concern only. The document, the wire and
 * every stored value stay in MILLIMETRES — `mmPer` is the one place a display
 * number becomes a document number and back, and `mmPer.in` is exact by
 * definition (1 in ≡ 25.4 mm), so an inch round-trip is lossless.
 */

/** The display units offered. `mm` is the document unit and the default. */
export type LengthUnitId = "mm" | "cm" | "m" | "in";

export interface LengthUnitDef {
  id: LengthUnitId;
  /**
   * Rendered next to a value AND used as the picker label — they are
   * deliberately the same string: a segmented tab reading "mm" and a chip
   * suffix reading "millimetres" would be two names for one thing.
   */
  symbol: string;
  /** Millimetres in one unit. */
  mmPer: number;
  /**
   * Decimals a VALUE renders with before trailing zeros are trimmed. Scaled so
   * every unit resolves at least as finely as the frozen mm contract (3 dp ⇒
   * 0.001 mm): cm needs one more digit, m three more, in ~1.4 more (4 dp ⇒
   * 0.00254 mm, comfortably inside a CAD chip's useful range).
   */
  decimals: number;
  /**
   * Decimals for the compact status-bar cursor read-out — one digit coarser
   * than `decimals` at every unit, because that row is a live-updating glance
   * value, not something anyone types back in.
   */
  cursorDecimals: number;
}

export const LENGTH_UNITS: Record<LengthUnitId, LengthUnitDef> = {
  mm: { id: "mm", symbol: "mm", mmPer: 1, decimals: 3, cursorDecimals: 2 },
  cm: { id: "cm", symbol: "cm", mmPer: 10, decimals: 4, cursorDecimals: 3 },
  m: { id: "m", symbol: "m", mmPer: 1000, decimals: 6, cursorDecimals: 5 },
  in: { id: "in", symbol: "in", mmPer: 25.4, decimals: 4, cursorDecimals: 3 },
};

/** Control order for the units toggle (also the canonical id enumeration). */
export const LENGTH_UNIT_ORDER: readonly LengthUnitId[] = ["mm", "cm", "m", "in"];

/**
 * Millimetres. The document unit is the display default so a fresh install
 * shows exactly what it stores — any other default would make the very first
 * number a user sees a converted one.
 */
export const DEFAULT_LENGTH_UNIT: LengthUnitId = "mm";

/**
 * Narrow an untrusted value (persisted setting, hand-edited localStorage, a
 * rolled-back build that wrote an id this registry no longer has) to a known
 * unit, falling back to the default rather than throwing — an unknown unit must
 * never leave the app unable to render a length at all.
 */
export function coerceLengthUnit(v: unknown): LengthUnitId {
  if (typeof v === "string" && Object.prototype.hasOwnProperty.call(LENGTH_UNITS, v)) {
    return v as LengthUnitId;
  }
  return DEFAULT_LENGTH_UNIT;
}
