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
 * REUSED from it verbatim: the `displayUnit` subscription (which is what
 * re-renders every chip on a unit switch, without ever re-committing) and the
 * 400 ms error flash. The mm-vs-other field width rule is NOT reused — see
 * `widthCh` below.
 *
 * Reading and formatting the chip's text lives in `tools/sketch/liveDimParse.ts`,
 * NOT here: `SketchController` parses every keystroke through the same function
 * to preview the typed value on the rubber band (UX review S7), and two parsers
 * for one field would eventually disagree about what the user typed.
 */
import { useEffect, useRef, useState } from "react";
import { useSettingsStore } from "@/stores/settingsStore";
import {
  chipSuffix,
  formatChipValue,
  parseChipValue,
  type ParsedChip,
} from "@/tools/sketch/liveDimParse";
import type { LiveDimChipField } from "@/stores/liveDimStore";

const ERROR_FLASH_MS = 400;

/** Floor for the content-driven field width, in `ch` (see `widthCh` below). */
const MIN_FIELD_CH = 4;

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
    const p: ParsedChip = parseChipValue(chip, text, unit);
    if (p.ok) return p.value;
    flashError();
    return "invalid";
  };

  const shown = focused ? text : formatChipValue(chip, unit);
  const suffix = chipSuffix(chip, unit);
  // Width FOLLOWS THE CONTENT (audit A11c / plan item 14). The old rule was a
  // fixed 36px for mm and 56px otherwise, which clipped its own text the
  // moment an mm value reached 5-6 characters ("12.75", "-137.5") — the
  // tangent-arc radius chip does that routinely mid-gesture. `ch` is the right
  // unit here because the field is `font-mono`, so one `ch` IS one glyph; the
  // floor keeps a short/empty value from collapsing the chip, and the +1
  // leaves room for the caret past the last character.
  const widthCh = `${Math.max(MIN_FIELD_CH, shown.length + 1)}ch`;
  // A chained leg's angle sits INSIDE the dashed arc preview (its anchor is
  // moved there — `SketchController.syncAngleReference`) as plain annotation
  // text matching the arc's own color, not a separate floating pill chip —
  // still a real input underneath (focus/type/Tab/lock all work identically).
  const isCornerAngleLabel = chip.field === "angle" && chip.drives;

  return (
    <span
      className={
        isCornerAngleLabel
          ? `pointer-events-auto inline-flex items-center font-mono text-[11px] ${
              isError ? "text-traffic-close" : focused || chip.locked ? "text-accent" : "text-sketch-angle-ref"
            }`
          : `pointer-events-auto inline-flex items-center gap-0.5 rounded-full border bg-surface px-2 font-mono text-[11px] shadow-popover ${
              isError
                ? "border-traffic-close text-traffic-close"
                : focused || chip.locked
                  ? "border-accent text-ink-2"
                  : "border-border text-ink-3"
            }`
      }
    >
      {!isCornerAngleLabel && <span className="text-ink-5">{chip.label}</span>}
      <input
        ref={ref}
        aria-label={`Live ${chip.field}`}
        aria-invalid={isError}
        data-testid={`live-dim-${chip.field}`}
        style={{ width: widthCh }}
        className="bg-transparent text-right outline-none"
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
