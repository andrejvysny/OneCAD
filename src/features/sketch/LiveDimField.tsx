/*
 * LiveDimField — ONE live dimension chip during a draw gesture.
 *
 * NOT `DimensionInput`, for three reasons that are all load-bearing:
 *   1. `DimensionInput` owns `aria-label="Dimension value"`; two of these on
 *      screen next to a constraint badge would make every existing locator
 *      ambiguous. This one is `aria-label="Live <field>"`.
 *   2. `DimensionInput` re-derives its text from `value` on every change. A live
 *      dimension's value updates at rAF frequency, so that effect would wipe the
 *      user's half-typed number several times a second.
 *   3. Blur here LOCKS without committing (a viewport click during editing), and
 *      Tab moves to the next field — neither concept exists there.
 *
 * REUSED from it verbatim: `parseLength`/`formatLength`/`formatUnitless`, the
 * `displayUnit` subscription (which is what re-renders every chip on a unit
 * switch, without ever re-committing), the 400 ms error flash, and the
 * mm-vs-other field width rule.
 */
import { useEffect, useRef, useState } from "react";
import { useSettingsStore } from "@/stores/settingsStore";
import type { LengthUnitId } from "@/units/lengthUnits";
import { formatLength, formatUnitless, lengthSuffix, parseLength } from "@/units/format";
import { norm360 } from "@/tools/sketch/liveDimension";
import type { LiveDimChipField } from "@/stores/liveDimStore";

const ERROR_FLASH_MS = 400;

/** Polygon side count bounds — below 3 there is no polygon, and past 12 the ring
 *  reads as a circle while costing a constraint per side. */
const MIN_SIDES = 3;
const MAX_SIDES = 12;

export interface LiveDimFieldProps {
  chip: LiveDimChipField;
  focused: boolean;
  /** The store's raw text — rendered only while focused (see reason 2 above). */
  text: string;
  onText(text: string): void;
  onFocus(): void;
  onTab(back: boolean, value: number | null): void;
  onEnter(value: number | null): void;
  onEscape(): void;
  onBlur(value: number | null): void;
}

/** A parse outcome. `value: null` = nothing lockable typed (an empty field still
 *  Tabs on); `ok: false` = typed something the chip refuses, which flashes. */
type Parsed = { ok: true; value: number | null } | { ok: false };

/**
 * Read the field's text in its own domain. Lengths accept a unit suffix and emit
 * MILLIMETRES; angles and counts are unit-blind (a degree is a degree in an inch
 * session). `Number` rather than `parseFloat` on purpose — `parseFloat("25abc")`
 * silently commits 25, and a measurement field must refuse what it only partly
 * understood.
 */
function parseChip(chip: LiveDimChipField, text: string, unit: LengthUnitId): Parsed {
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
  // arc". It is identifiable as the angle field that authors nothing — a line or
  // slot heading is absolute and folds freely into [0, 360).
  if (!chip.drives) return n > 0 && n < 360 ? { ok: true, value: n } : { ok: false };
  return { ok: true, value: norm360(n) };
}

function formatChip(chip: LiveDimChipField, unit: LengthUnitId): string {
  if (chip.domain === "length") return formatLength(chip.value, unit);
  if (chip.domain === "count") return String(Math.round(chip.value));
  return formatUnitless(chip.value);
}

function suffixFor(chip: LiveDimChipField, unit: LengthUnitId): string {
  if (chip.domain === "length") return lengthSuffix(unit);
  return chip.domain === "angle" ? "°" : "";
}

export function LiveDimField({
  chip,
  focused,
  text,
  onText,
  onFocus,
  onTab,
  onEnter,
  onEscape,
  onBlur,
}: LiveDimFieldProps) {
  // Subscribing here is what re-renders every open chip on a unit switch. It is
  // a pure re-display: no handler fires, so the mm the gesture holds is untouched.
  const unit = useSettingsStore((s) => s.displayUnit);
  const [isError, setIsError] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  const errorTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (errorTimer.current) clearTimeout(errorTimer.current);
    };
  }, []);

  // The FSM owns focus, so the DOM follows it rather than the other way round:
  // a digit typed over the viewport opens this chip with the character already
  // in the store, and the caret has to land after it.
  useEffect(() => {
    if (!focused) return;
    const el = ref.current;
    if (!el) return;
    el.focus();
    const end = el.value.length;
    el.setSelectionRange(end, end);
  }, [focused]);

  const flashError = (): void => {
    setIsError(true);
    if (errorTimer.current) clearTimeout(errorTimer.current);
    errorTimer.current = setTimeout(() => setIsError(false), ERROR_FLASH_MS);
  };

  /** Parsed value, or null when the text is refused (flashing on the way out). */
  const read = (): number | null | "invalid" => {
    const p = parseChip(chip, text, unit);
    if (p.ok) return p.value;
    flashError();
    return "invalid";
  };

  const shown = focused ? text : formatChip(chip, unit);
  const suffix = suffixFor(chip, unit);
  // Only a LENGTH in a non-mm unit renders enough decimals to need the wide
  // field ("0.0394 in" for 1 mm); mm stays prototype-exact at 36px.
  const wide = chip.domain === "length" && unit !== "mm";

  return (
    <span
      className={`pointer-events-auto inline-flex items-center gap-0.5 rounded-full border bg-surface px-2 font-mono text-[11px] shadow-popover ${
        isError
          ? "border-traffic-close text-traffic-close"
          : focused || chip.locked
            ? "border-accent text-ink-2"
            : "border-border text-ink-3"
      }`}
    >
      <span className="text-ink-5">{chip.label}</span>
      <input
        ref={ref}
        aria-label={`Live ${chip.field}`}
        aria-invalid={isError}
        data-testid={`live-dim-${chip.field}`}
        className={`${wide ? "w-14" : "w-9"} bg-transparent text-right outline-none`}
        value={shown}
        inputMode="decimal"
        onChange={(e) => onText(e.target.value)}
        // The DOM focus event fires for our OWN programmatic focus above too —
        // reporting it back would reset the store text and eat the character
        // that opened the chip, so only a focus the FSM does not know about
        // (a click straight onto the chip) is relayed.
        onFocus={() => {
          if (!focused) onFocus();
        }}
        onBlur={() => {
          const v = read();
          onBlur(v === "invalid" ? null : v);
        }}
        onKeyDown={(e) => {
          // ALWAYS: this component sits under the controller's capture-phase
          // window listener, which must never see a key aimed at this field.
          e.stopPropagation();
          if (e.key === "Enter") {
            const v = read();
            if (v !== "invalid") onEnter(v);
            return;
          }
          if (e.key === "Tab") {
            e.preventDefault(); // Tab cycles the chip set, not the DOM tab order.
            const v = read();
            if (v !== "invalid") onTab(e.shiftKey, v);
            return;
          }
          if (e.key === "Escape") {
            onEscape();
            ref.current?.blur();
          }
        }}
      />
      {suffix && <span className="text-ink-5">{suffix}</span>}
      {!chip.drives && (
        <span
          aria-hidden="true"
          title="Sets the geometry — no constraint is created"
          className="text-ink-5"
        >
          •
        </span>
      )}
    </span>
  );
}
