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
import { formatDimensionValue } from "./dimensionFormat";

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
  const [text, setText] = useState(() => formatDimensionValue(value));
  const [isError, setIsError] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  const errorTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setText(formatDimensionValue(value));
  }, [value]);

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
    const n = Number.parseFloat(text);
    if (!Number.isFinite(n)) {
      if (kind) {
        flashError();
        return false;
      }
      // No kind ⇒ legacy finite-only behavior, unchanged (model-tool chips).
      setText(formatDimensionValue(value));
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
    if (formatDimensionValue(n) !== formatDimensionValue(value)) onCommit(n);
    else setText(formatDimensionValue(value));
    return true;
  };

  return (
    <span
      className={`pointer-events-auto inline-flex items-center gap-0.5 rounded-sm border bg-white px-1 font-mono text-[11px] text-sel-text shadow-ctrl ${
        isError ? "border-traffic-close" : "border-accent"
      }`}
    >
      <input
        ref={ref}
        aria-label="Dimension value"
        aria-invalid={isError}
        className={`w-9 bg-transparent text-right outline-none ${isError ? "text-traffic-close" : ""}`}
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
              setText(formatDimensionValue(value));
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
      {suffix && <span className="text-ink-5">{suffix}</span>}
    </span>
  );
}
