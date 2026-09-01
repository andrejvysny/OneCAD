/*
 * The line under a value input while an `=` expression is being typed: what it
 * resolves to, or why it does not — plus the millimetre nudge under a display
 * unit that is not millimetres.
 *
 * Presentational only. The evaluation, the debounce and the unit rules all live
 * in `exprPreview.ts`; this decides nothing, it only renders the verdict.
 */
import type { ExpressionDimension } from "@/ipc/types";
import type { LengthUnitId } from "@/units/lengthUnits";
import type { ExprPreview } from "./exprPreview";
import { EXPR_MM_HINT, formatExprValue } from "./exprPreview";

export interface ExprPreviewLineProps {
  /** The expression text the field currently holds (without its `=`). */
  exprText: string;
  /** The newest evaluation, or `null` while one is in flight. */
  preview: ExprPreview | null;
  /** The field's own domain — what a bare number in the expression means. */
  site: ExpressionDimension;
  unit: LengthUnitId;
  /** Show the "bare numbers inside `=` are millimetres" nudge. */
  showMmHint: boolean;
}

const CHIP = "rounded-sm border border-border bg-surface px-1 shadow-ctrl";

export function ExprPreviewLine({
  exprText,
  preview,
  site,
  unit,
  showMmHint,
}: ExprPreviewLineProps) {
  // An answer computed for OTHER text is not this text's answer: while one is
  // in flight the line waits rather than showing the previous number, which
  // would be a wrong reading rather than a stale one.
  const current = preview !== null && preview.text === exprText ? preview : null;
  return (
    <span className="absolute left-0 top-full z-10 mt-0.5 flex flex-col items-start gap-0.5 whitespace-nowrap">
      <span
        data-testid="dimension-expr-preview"
        role="status"
        className={`${CHIP} ${current?.error !== undefined ? "text-traffic-close" : "text-ink-3"}`}
      >
        {current === null
          ? "…"
          : current.error !== undefined
            ? current.error
            : `= ${formatExprValue(current, site, unit)}`}
      </span>
      {showMmHint && (
        <span data-testid="dimension-expr-hint" className={`${CHIP} text-ink-5`}>
          {EXPR_MM_HINT}
        </span>
      )}
    </span>
  );
}
