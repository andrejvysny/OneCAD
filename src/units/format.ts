/*
 * Units display layer (W2-A, unit-aware since WP-C2) — the ONE place a length
 * or an area becomes text, and the ONE place typed text becomes a length.
 *
 * SCOPE, deliberately narrow: this is a DISPLAY concern only. The document, the
 * wire, and every stored value stay in **millimetres**. `settingsStore.displayUnit`
 * changes only how a number READS and how a BARE typed number is INTERPRETED —
 * everything that leaves this module toward the document is millimetres, always.
 * That asymmetry is the whole point of the feature, and it is what the
 * "marshalling independence" tests pin: creating the same dimension with
 * displayUnit = "in" must put the identical mm number on the wire.
 *
 * BARE INPUT IS READ IN THE DISPLAY UNIT. Displaying inches and typing `2` means
 * 2 in (50.8 mm), not 2 mm — the professional CAD convention. A suffix always
 * wins over the preference (`2mm` is 2 mm even while displaying inches), so a
 * user who knows what they want can always say it.
 *
 * Why the asymmetry between `formatLength` and `formatArea`:
 *   - `formatLength` returns a BARE number string. It is the formatter behind
 *     the editable `DimensionInput` field, whose unit lives in a separate
 *     `suffix` span — putting the unit inside the value would make the field
 *     round-trip its own suffix on every keystroke.
 *   - `formatArea` returns the value WITH its unit. An area is never editable,
 *     so it has no bare-number twin to keep in step.
 * Callers that need a length WITH its unit use `formatLengthWithUnit`.
 *
 * `formatMillimetres` / `MM_SUFFIX` are the ESCAPE HATCH for text that is not a
 * display string: a value composed into (or parsed back out of) a document
 * `valueText`, where the mm domain is part of the data. Anything reaching those
 * must be preference-INDEPENDENT or a unit switch would silently rewrite a
 * stored number — see `tools/preview/filletRadius.ts`.
 */
import { settingsStore } from "@/stores/settingsStore";
import {
  DEFAULT_LENGTH_UNIT,
  LENGTH_UNITS,
  type LengthUnitId,
} from "@/units/lengthUnits";

/** Unit the DOCUMENT (and therefore the wire, and every stored value) uses. */
export const MM_SUFFIX = "mm";

/** The unit lengths are currently DISPLAYED in. */
export function currentLengthUnit(): LengthUnitId {
  return settingsStore.getState().displayUnit ?? DEFAULT_LENGTH_UNIT;
}

/** Symbol for the current (or an explicitly given) display unit. */
export function lengthSuffix(unit: LengthUnitId = currentLengthUnit()): string {
  return LENGTH_UNITS[unit].symbol;
}

/** Area symbol — the square of `lengthSuffix`. */
export function areaSuffix(unit: LengthUnitId = currentLengthUnit()): string {
  return `${LENGTH_UNITS[unit].symbol}²`;
}

/*
 * LIVE BINDINGS, not constants.
 *
 * `LENGTH_SUFFIX` / `AREA_SUFFIX` predate the unit preference and are read by
 * modules this work package does not own. Keeping them as `mm` literals while
 * `formatLength` converted the VALUE would render "0.984 mm" for a 25 mm edge —
 * a wrong reading is far worse than a suffix that lags one render behind. So
 * they are ES live bindings kept in step with the store below: every importer
 * that reads them at CALL time (all of them do — they are interpolated inside
 * render functions, never captured at module init) sees the current unit.
 *
 * They are NOT reactive: a component must still subscribe to `displayUnit` to
 * re-render. Everything under this WP's ownership does (DimensionInput,
 * StatusBar); prefer `lengthSuffix()` + a store subscription in new code.
 */
export let LENGTH_SUFFIX: string = lengthSuffix(DEFAULT_LENGTH_UNIT);
export let AREA_SUFFIX: string = areaSuffix(DEFAULT_LENGTH_UNIT);

function syncSuffixes(unit: LengthUnitId): void {
  LENGTH_SUFFIX = lengthSuffix(unit);
  AREA_SUFFIX = areaSuffix(unit);
}
syncSuffixes(currentLengthUnit());
settingsStore.subscribe((s) => syncSuffixes(s.displayUnit ?? DEFAULT_LENGTH_UNIT));

/** Millimetres → the display unit's number. */
export function mmToDisplay(mm: number, unit: LengthUnitId = currentLengthUnit()): number {
  return mm / LENGTH_UNITS[unit].mmPer;
}

/** A display-unit number → millimetres (exact for `in`: 25.4 by definition). */
export function displayToMm(value: number, unit: LengthUnitId = currentLengthUnit()): number {
  return value * LENGTH_UNITS[unit].mmPer;
}

/**
 * Fix to `decimals`, then strip trailing zeros (and a bare trailing point).
 *
 * `toFixed(n).replace(/\.?0+$/,"")` looks equivalent but corrupts whole numbers —
 * `"100".replace(/\.?0+$/,"")` eats the trailing zeros of "100" itself, leaving
 * "1". Stripping only inside a capture group anchored AFTER the decimal point is
 * what keeps the integer part intact (the regression the "100" test pins).
 */
function fixedTrimmed(n: number, decimals: number): string {
  return n
    .toFixed(decimals)
    .replace(/(\.\d*?)0+$/, "$1")
    .replace(/\.$/, "");
}

/**
 * Format a length (given in mm) as a BARE number in the DISPLAY unit, trailing
 * zeros trimmed. No suffix — see the module header.
 */
export function formatLength(mm: number, unit: LengthUnitId = currentLengthUnit()): string {
  if (!Number.isFinite(mm)) return String(mm);
  return fixedTrimmed(mmToDisplay(mm, unit), LENGTH_UNITS[unit].decimals);
}

/** Format a length (mm) as `"<value> <unit>"` in the display unit. */
export function formatLengthWithUnit(
  mm: number,
  unit: LengthUnitId = currentLengthUnit(),
): string {
  return `${formatLength(mm, unit)} ${lengthSuffix(unit)}`;
}

/**
 * Format a number with NO unit domain attached, at the document's 3-decimal
 * precision, trailing zeros trimmed. The formatter for quantities the display
 * unit has no business touching — degrees, most obviously.
 */
export function formatUnitless(n: number): string {
  if (!Number.isFinite(n)) return String(n);
  return fixedTrimmed(n, LENGTH_UNITS.mm.decimals);
}

/**
 * Format a length (mm) as a BARE number in MILLIMETRES, independent of the
 * display preference. For document text (`valueText`) and anything re-parsed as
 * mm — NOT for something a user reads as a measurement.
 */
export function formatMillimetres(mm: number): string {
  return formatUnitless(mm);
}

/**
 * Format a face area (given in mm²) for display, unit included (`"800 mm²"`).
 *
 * The conversion factor is `mmPer` SQUARED — an area is not a length, and
 * dividing by 25.4 once would under-report an inch² by a factor of 25.4. That
 * is exactly the bug this function exists to make impossible.
 */
export function formatArea(mm2: number, unit: LengthUnitId = currentLengthUnit()): string {
  if (!Number.isFinite(mm2)) return String(mm2);
  const f = LENGTH_UNITS[unit].mmPer;
  return `${fixedTrimmed(mm2 / (f * f), LENGTH_UNITS[unit].decimals)} ${areaSuffix(unit)}`;
}

/**
 * One axis of the status-bar cursor read-out: a fixed-width, fixed-decimals
 * number in the display unit. NOT trimmed — the column must not jitter as the
 * pointer moves, which is the one place trailing zeros earn their keep.
 */
export function formatCursorAxis(mm: number, unit: LengthUnitId = currentLengthUnit()): string {
  if (!Number.isFinite(mm)) return String(mm).padStart(6);
  return mmToDisplay(mm, unit).toFixed(LENGTH_UNITS[unit].cursorDecimals).padStart(6);
}

/** Does `s` name a length unit? Used to tell a length chip from an angle chip. */
export function isLengthSuffix(s: string): boolean {
  return Object.prototype.hasOwnProperty.call(LENGTH_UNITS, s.trim().toLowerCase());
}

/**
 * A signed decimal (optionally in exponent form) followed by an OPTIONAL unit.
 * The exponent is part of the number group, so `"1e3"` parses as 1000 rather
 * than as 1 with an unknown `e3` unit — while `"1em"` still fails (the exponent
 * needs digits), which is the behaviour a typo should get.
 */
const LENGTH_RE = /^([+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)\s*([a-zA-Z]*)$/;

/**
 * Parse a typed length into MILLIMETRES. Accepts a suffixed value (`"25mm"`,
 * `"2.5 cm"`, `"0.5m"`, `"1 in"`), case- and whitespace-insensitive, where the
 * SUFFIX decides the unit; and a bare number (`"25"`), which is read in the
 * current DISPLAY unit (`unit`) — displaying inches, `2` means 2 in.
 *
 * Returns `null` — never a silent 0 or a partial read — for anything else,
 * including an unknown unit (`"25 furlongs"`) and trailing junk (`"25abc"`).
 * `Number.parseFloat` would happily return 25 for both; a measurement UI must
 * refuse a value it did not fully understand rather than commit a guess.
 */
export function parseLength(
  text: string,
  unit: LengthUnitId = currentLengthUnit(),
): number | null {
  const m = LENGTH_RE.exec(text.trim());
  if (!m) return null;
  const n = Number.parseFloat(m[1]);
  if (!Number.isFinite(n)) return null;
  const typed = m[2].toLowerCase();
  if (typed === "") return displayToMm(n, unit);
  if (!Object.prototype.hasOwnProperty.call(LENGTH_UNITS, typed)) return null;
  return displayToMm(n, typed as LengthUnitId);
}
