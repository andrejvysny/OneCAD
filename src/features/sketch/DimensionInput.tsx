/*
 * DimensionInput — the editable mono chip on a dimensional constraint badge
 * (Distance/Radius/Angle/…). Commits on Enter/blur → sketchUpsert with the new
 * value (design acknowledges a full dimension DIALOG lands later; this is the
 * inline chip).
 *
 * The Dimension TOOL reuses this chip with `commitOnBlur={false}` + `onCancel`:
 * while placing a dimension, a canvas click must NOT blur-commit the seeded value
 * (a second line click upgrades a length into an angle), and Esc must cancel the
 * whole pick, not just reset the text.
 */
import { useEffect, useRef, useState } from "react";
import type { SketchConstraintType } from "@/ipc/types";
import { useSettingsStore } from "@/stores/settingsStore";
import {
  formatLength,
  formatUnitless,
  isLengthSuffix,
  lengthSuffix,
  parseLength,
} from "@/units/format";

export interface DimensionInputProps {
  value: number;
  suffix?: string;
  onCommit(value: number): void;
  /**
   * Enter contract for the armed model-tool cluster (MODEL-HARDEN Wave 1): when
   * provided, a valid Enter applies the typed value (via `onCommit`) and THEN
   * confirms the op, instead of just blurring. The input stops propagation so the
   * controller's capture-phase Enter handler never double-fires (single confirm).
   */
  onConfirm?: () => void;
  /** Commit the current text when the input loses focus (default true). */
  commitOnBlur?: boolean;
  /** Esc handler — when provided, Esc calls this instead of resetting the text. */
  onCancel?: () => void;
  /** Focus + select the field on mount (the dimension tool opens ready to type). */
  autoFocus?: boolean;
  /**
   * Constraint kind driving the badge (ConstraintBadgeLayer only). When present,
   * enables constraint-specific range validation on commit (below). Absent ⇒ the
   * legacy finite-only check (model-tool chips have no constraint kind).
   */
  kind?: SketchConstraintType;
}

const ERROR_FLASH_MS = 400;

/** Constraint-specific commit-value validity. Caller has already checked finite. */
function isValidForKind(kind: SketchConstraintType, n: number): boolean {
  switch (kind) {
    case "Distance":
    case "Radius":
    case "Diameter":
      return n > 0;
    case "Angle":
      return n > 0 && n < 360;
    default:
      // HorizontalDistance/VerticalDistance are signed — any finite value is legal.
      return true;
  }
}

export function DimensionInput({
  value,
  suffix = "",
  onCommit,
  onConfirm,
  commitOnBlur = true,
  onCancel,
  autoFocus = false,
  kind,
}: DimensionInputProps) {
  /*
   * Is this chip editing an ANGLE (degrees) rather than a LENGTH (mm)?
   *
   * Both signals are load-bearing because the two producers differ:
   *   - ConstraintBadgeLayer passes `kind="Angle"` with an EMPTY suffix (the °
   *     is already baked into the badge glyph);
   *   - the model-tool cluster + the sketch Dimension chip pass no `kind` at all
   *     and mark the domain with `suffix="°"` (revolve angle, circular pattern
   *     angle, an Angle dimension being authored).
   * Checking only one of them would silently run a degree value through the
   * length parser (or vice versa), so both are honoured.
   */
  const isAngle = kind === "Angle" || suffix === "°";
  /*
   * WP-C2. `value` is ALWAYS the document number: millimetres for a length,
   * degrees for an angle. The DISPLAY unit only changes how a length reads and
   * how a bare typed number is read back, so an angle ignores it entirely (the
   * deg↔rad marshalling lives in `@/ipc/angleUnits` and this module must not
   * second-guess it).
   *
   * Subscribing here is what makes every length chip in the app re-render on a
   * unit switch — model-tool chips, sketch dimension badges and the dimension
   * tool all render through THIS component, so none of them needs its own
   * subscription.
   */
  const unit = useSettingsStore((s) => s.displayUnit);
  const formatValue = (n: number) => (isAngle ? formatUnitless(n) : formatLength(n, unit));
  /*
   * A caller passes a LENGTH suffix ("mm", from `LENGTH_SUFFIX` or a literal)
   * to say "this chip is a length", not to pick the unit — so it is replaced by
   * the current display unit. A non-length suffix ("°", "") passes through
   * untouched, which is what keeps angle chips out of this path.
   */
  const shownSuffix = !isAngle && isLengthSuffix(suffix) ? lengthSuffix(unit) : suffix;

  const [text, setText] = useState(() => formatValue(value));
  const [isError, setIsError] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  const errorTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  /*
   * Re-render the SAME value in the new unit. This is a pure re-display: it
   * never calls `onCommit`, so switching units cannot dirty a document or
   * re-commit a dimension (pinned test) — the mm the constraint holds is
   * untouched.
   */
  useEffect(() => {
    setText(formatValue(value));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, unit]);

  useEffect(() => {
    if (autoFocus) {
      ref.current?.focus();
      ref.current?.select();
    }
  }, [autoFocus]);

  useEffect(() => {
    return () => {
      if (errorTimer.current) clearTimeout(errorTimer.current);
    };
  }, []);

  const flashError = () => {
    setIsError(true);
    if (errorTimer.current) clearTimeout(errorTimer.current);
    errorTimer.current = setTimeout(() => setIsError(false), ERROR_FLASH_MS);
  };

  /**
   * Returns whether the commit went through (or was a no-op legacy reset) —
   * callers use this to decide whether it's safe to blur the field. A kind-
   * validation reject must NOT blur: the chip stays open + focused so the
   * user can correct the value in place.
   */
  const commit = (): boolean => {
    // ANGLE chips keep the plain float parse: the UI angle domain is degrees and
    // its deg↔rad marshalling lives in `@/ipc/angleUnits`, which this module must
    // not second-guess. Every other chip is a LENGTH, so it accepts a unit suffix
    // (`25mm`, `2.5 cm`, `1 in`) or a BARE number read in the current display
    // unit — and always emits MILLIMETRES, whatever the preference. It REJECTS
    // text it only partly understood ("25abc"), where parseFloat would commit 25.
    const n = isAngle ? Number.parseFloat(text) : (parseLength(text, unit) ?? Number.NaN);
    if (!Number.isFinite(n)) {
      if (kind) {
        flashError();
        return false;
      }
      // No kind ⇒ legacy finite-only behavior, unchanged (model-tool chips).
      setText(formatValue(value));
      return true;
    }
    if (kind && !isValidForKind(kind, n)) {
      flashError();
      return false;
    }
    // Compare formatted strings, not raw floats: an untouched blur of a value
    // that displays truncated (e.g. 12.345 → "12.345" already exact, but a
    // value like 12.3456 rendering as "12.346") must not spuriously re-commit
    // the rounded display value as if the user had typed something new.
    if (formatValue(n) !== formatValue(value)) onCommit(n);
    else setText(formatValue(value));
    return true;
  };

  return (
    <span
      className={`pointer-events-auto inline-flex items-center gap-0.5 rounded-sm border bg-surface px-1 font-mono text-[11px] text-sel-text shadow-ctrl ${
        isError ? "border-traffic-close" : "border-accent"
      }`}
    >
      <input
        ref={ref}
        aria-label="Dimension value"
        aria-invalid={isError}
        /* The 36px field is prototype-exact for millimetres. Every other unit
           divides, so it systematically renders more decimals ("0.0394 in" for
           1 mm) and would scroll its leading digits out of sight — the field
           widens only when a unit that needs it is selected, leaving the mm
           default pixel-identical. */
        className={`${isAngle || unit === "mm" ? "w-9" : "w-14"} bg-transparent text-right outline-none ${isError ? "text-traffic-close" : ""}`}
        value={text}
        inputMode="decimal"
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            // Apply the typed value, THEN confirm the op (armed cluster) — else
            // just blur (legacy dimension chips / fillet / shell).
            if (commit()) {
              if (onConfirm) onConfirm();
              else ref.current?.blur();
            }
          } else if (e.key === "Escape") {
            if (onCancel) {
              onCancel();
            } else {
              setText(formatValue(value));
              ref.current?.blur();
            }
          }
          e.stopPropagation();
        }}
        onBlur={() => {
          if (commitOnBlur) {
            if (!commit()) ref.current?.focus();
          }
        }}
      />
      {shownSuffix && <span className="text-ink-5">{shownSuffix}</span>}
    </span>
  );
}
