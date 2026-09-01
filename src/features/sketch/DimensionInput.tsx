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
 *
 * With `onCommitExpr` the field also runs the `=` EXPRESSION lane: the text is
 * evaluated live by the backend (`exprPreview.ts`) and the resolved value is
 * shown under the field before anything is committed. Validity is the
 * evaluator's verdict, never a second grammar here — see `exprPreview.ts` for
 * the unit rule and the two authoring guardrails.
 */
import { useEffect, useRef, useState } from "react";
import type { ExpressionDimension, SketchConstraintType } from "@/ipc/types";
import { useSettingsStore } from "@/stores/settingsStore";
import {
  formatLength,
  formatUnitless,
  isLengthSuffix,
  lengthSuffix,
  parseLength,
} from "@/units/format";
import { isPlainLiteral, parseExprInput, useExprPreview } from "./exprPreview";
import { ExprPreviewLine } from "./ExprPreviewLine";

export { parseExprInput } from "./exprPreview";

export interface DimensionInputProps {
  value: number;
  suffix?: string;
  onCommit(value: number): void;
  /**
   * OPT-IN expression binding. When provided, the field also accepts `=<expr>`
   * — any arithmetic over document variables and unit-suffixed literals
   * (`=w*2 + 5mm`) — and commits through this callback instead of
   * {@link DimensionInputProps.onCommit}. `null` means "the user typed a plain
   * number over a binding": clear it. `value` is the number the expression
   * currently resolves to, in the DOCUMENT domain (mm / degrees).
   *
   * Opt-in rather than universal because MOST consumers of this chip are sketch
   * constraint badges, whose values are solver dimensions with no `Scalar` and no
   * `expr` behind them. Accepting an expression there would show a binding the
   * backend has nowhere to record. With this prop absent, `=x` is rejected
   * exactly as any other unparseable text is.
   */
  onCommitExpr?(expr: string | null, value: number): void;
  /** The binding this field currently holds (BACKEND-AUTHORITATIVE — the
   *  projection's `primaryExpr`), rendered as `=<expr>` while at rest. */
  expr?: string;
  /**
   * Enter contract for the armed model-tool cluster (MODEL-HARDEN Wave 1): when
   * provided, a valid Enter applies the typed value (via `onCommit`) and THEN
   * confirms the op, instead of just blurring. The input stops propagation so the
   * controller's capture-phase Enter handler never double-fires (single confirm).
   */
  onConfirm?: () => void;
  /**
   * Live preview for the ARMED model-tool cluster (U3, contract column
   * `livePreviewOnEdit`): called on EVERY parseable change with the
   * document-domain value (mm / degrees), so the viewport follows the keystroke.
   * Blur must never be the only update mechanism.
   *
   * Invalid or partial text ("2.", "25abc") emits nothing — it leaves the field
   * editable and the last valid preview standing, and never clamps.
   *
   * The controller is what coalesces these into kernel requests; this is only the
   * value edge (see `previewThrottle.ts`).
   */
  onPreview?: (value: number) => void;
  /**
   * Seed text for a type-to-enter arm (U3): the character the user typed on the
   * canvas, which REPLACES the formatted value rather than appending to it.
   * Subsequent characters edit normally, because the field now has focus.
   */
  initialText?: string;
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
  onCommitExpr,
  expr,
  onConfirm,
  onPreview,
  initialText,
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

  /*
   * A BOUND field reads as its binding, not its number: the number is derived,
   * and showing it would leave the user editing a value the variable overwrites
   * on the next regen. The resolved number rides the title attribute + the
   * suffix slot instead (see the read-only chip in `HistoryList`).
   *
   * A type-to-enter SEED outranks the binding: the user typed a digit on the
   * canvas, which is an unbind gesture, so the field must show that digit and
   * not the `=name` it is about to replace.
   */
  const [text, setText] = useState(
    () => initialText ?? (expr ? `=${expr}` : formatValue(value)),
  );
  const [isError, setIsError] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  const errorTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  /*
   * The `=` EXPRESSION lane. `exprText` is what the field currently holds after
   * the `=`, or null when the field is not in expression mode — which is also
   * how the lane stays completely idle for the sketch-badge chips that never
   * opted in (`onCommitExpr` absent).
   *
   * The SITE is the field's own domain, and it is what turns `=45deg` typed
   * into a length field into a loud refusal rather than a silent 45 mm.
   */
  const site: ExpressionDimension = isAngle ? "angle" : "length";
  const typedExpr = onCommitExpr ? parseExprInput(text) : null;
  const exprText = typedExpr !== null && !isPlainLiteral(typedExpr) ? typedExpr : null;
  const { preview, evaluateNow } = useExprPreview(exprText, site);
  // A LENGTH expression under any other display unit is the one place the two
  // unit rules diverge (bare-inside-`=` is mm, bare-alone is the display unit),
  // so the field says so where it is being typed.
  const showMmHint = exprText !== null && site === "length" && unit !== "mm";

  /*
   * Re-render the SAME value in the new unit. This is a pure re-display: it
   * never calls `onCommit`, so switching units cannot dirty a document or
   * re-commit a dimension (pinned test) — the mm the constraint holds is
   * untouched.
   */
  const seeded = useRef(initialText !== undefined);
  /*
   * The value THIS field last previewed. Live preview means our own edits come
   * straight back as a new `value` prop, and re-formatting on that echo would
   * rewrite the text under the cursor — typing "25." would become "25" the
   * instant the 25 previewed, eating the decimal point.
   */
  const lastPreviewed = useRef<number | null>(null);
  /*
   * A type-to-enter seed IS a parseable change — the user typed a character and
   * the viewport must follow it, exactly as if they had typed it into the field.
   * It arrives as initial state rather than an input event, so it needs its own
   * emit; every character after it comes through `onChange` normally.
   */
  useEffect(() => {
    if (initialText === undefined || !onPreview) return;
    const n = parse(initialText);
    if (Number.isFinite(n) && (!kind || isValidForKind(kind, n))) {
      lastPreviewed.current = n;
      onPreview(n);
    }
    // Mount-only: `initialText` changes identity with the remount key, so a new
    // seed is a new component instance, not a new run of this effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const lastUnit = useRef(unit);
  const lastExpr = useRef(expr);
  useEffect(() => {
    const unitChanged = lastUnit.current !== unit;
    lastUnit.current = unit;
    // A BINDING change is a re-label for exactly the same reason a unit switch
    // is: `=height` ⇄ a number is a change of what the field DENOTES, not an
    // echo of a value this field previewed. Without this the echo guard below
    // would swallow it whenever the resolved number happened to be unchanged.
    const exprChanged = lastExpr.current !== expr;
    lastExpr.current = expr;
    // A type-to-enter seed owns the field until the user leaves it: the first
    // `onPreview` pushes `value` back down, and re-formatting it here would
    // overwrite the very characters being typed.
    if (seeded.current) {
      seeded.current = false;
      return;
    }
    // A UNIT switch always re-displays: it is a pure re-label of the same
    // document value (50.8 mm ⇄ 2 in) and has nothing to do with the echo guard.
    // Only a VALUE that came back as this field's own last preview is suppressed.
    if (
      !unitChanged &&
      !exprChanged &&
      lastPreviewed.current !== null &&
      lastPreviewed.current === value
    )
      return;
    lastPreviewed.current = null;
    setText(expr ? `=${expr}` : formatValue(value));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, unit, expr]);

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
  /**
   * Text → document-domain number, or NaN when the text is not (yet) a value.
   *
   * ANGLE chips keep the plain float parse: the UI angle domain is degrees and
   * its deg↔rad marshalling lives in `@/ipc/angleUnits`, which this module must
   * not second-guess. Every other chip is a LENGTH, so it accepts a unit suffix
   * (`25mm`, `2.5 cm`, `1 in`) or a BARE number read in the current display unit
   * — and always emits MILLIMETRES, whatever the preference. It REJECTS text it
   * only partly understood ("25abc"), where parseFloat would commit 25.
   */
  const parse = (raw: string): number => {
    if (!isAngle) return parseLength(raw, unit) ?? Number.NaN;
    // Angles are strict for the same reason lengths are: `parseFloat("12abc")`
    // is 12, so a typo would silently commit — and under live preview it would
    // also rewrite the field mid-keystroke. Only a complete number parses.
    const t = raw.trim();
    return /^[+-]?(\d+\.?\d*|\.\d+)$/.test(t) ? Number.parseFloat(t) : Number.NaN;
  };

  /**
   * Commit an `=` EXPRESSION. Returns whether the field may close.
   *
   * The committed number is the evaluator's, never the field's own parse: the
   * expression is what the document records, and its value has to be the one
   * the preview just showed. A refusal keeps the field open with the reason on
   * screen — the user asked for a binding, and a silent fall-through to the old
   * number would give them nothing.
   */
  const commitExpr = (typed: string): boolean => {
    // Re-typing the SAME binding is not an edit.
    if (typed === expr) {
      setText(`=${typed}`);
      return true;
    }
    if (preview !== null && preview.text === typed) {
      if (preview.error !== undefined) {
        flashError();
        return false;
      }
      onCommitExpr?.(typed, preview.value);
      return true;
    }
    /*
     * Enter (or blur) beat the debounce. Flush the evaluation and finish when
     * it lands, holding the field open until then — the alternative is
     * committing a number nobody has computed yet.
     */
    void evaluateNow(typed).then((r) => {
      if (r.error !== undefined) flashError();
      else onCommitExpr?.(typed, r.value);
    });
    return false;
  };

  const commit = (): boolean => {
    /*
     * The `=` lane, checked BEFORE the numeric parse so an expression can never
     * fall through to it. Only reachable when the caller opted in; otherwise
     * `=x` stays the unparseable text it has always been.
     */
    let numericText = text;
    if (onCommitExpr) {
      const typed = parseExprInput(text);
      if (typed !== null && isPlainLiteral(typed)) {
        // GUARDRAIL: a pure literal is not an expression anybody wants
        // recorded, and recording it would make `=2` mean 2 mm while `2` means
        // 2 in. It commits through the PLAIN path with the `=` dropped.
        numericText = typed;
      } else if (typed !== null) {
        return commitExpr(typed);
      } else if (text.trim().startsWith("=")) {
        // `=` with nothing after it: the user asked for a binding and named
        // none. Never silently re-commit the old number.
        flashError();
        return false;
      }
    }
    // ANGLE chips keep the plain float parse: the UI angle domain is degrees and
    // its deg↔rad marshalling lives in `@/ipc/angleUnits`, which this module must
    // not second-guess. Every other chip is a LENGTH, so it accepts a unit suffix
    // (`25mm`, `2.5 cm`, `1 in`) or a BARE number read in the current display
    // unit — and always emits MILLIMETRES, whatever the preference. It REJECTS
    // text it only partly understood ("25abc"), where parseFloat would commit 25.
    const n = parse(numericText);
    if (!Number.isFinite(n)) {
      if (kind) {
        flashError();
        return false;
      }
      // No kind ⇒ legacy finite-only behavior, unchanged (model-tool chips).
      setText(expr ? `=${expr}` : formatValue(value));
      return true;
    }
    if (kind && !isValidForKind(kind, n)) {
      flashError();
      return false;
    }
    // A plain number typed over a BOUND field clears the binding — even when the
    // number is unchanged, because unbinding IS the edit the user made.
    if (onCommitExpr && expr) {
      onCommitExpr(null, n);
      return true;
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
      /* `relative` anchors the expression preview, which hangs BELOW the chip:
         the chip itself is laid out against a badge or a history row, and
         growing it would move whatever it is attached to. */
      className={`pointer-events-auto relative inline-flex items-center gap-0.5 rounded-sm border bg-surface px-1 font-mono text-[11px] text-sel-text shadow-ctrl ${
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
        className={`${onCommitExpr ? "w-20" : isAngle || unit === "mm" ? "w-9" : "w-14"} bg-transparent text-right outline-none ${isError ? "text-traffic-close" : ""}`}
        value={text}
        /* A bindable field takes `=name`, so it must not ask for a numeric
           keypad — that would make the "=" unreachable on a touch keyboard. */
        inputMode={onCommitExpr ? "text" : "decimal"}
        onChange={(e) => {
          setText(e.target.value);
          if (!onPreview) return;
          // A half-typed BINDING never previews: the field is showing `=h`, and
          // emitting the stale number behind it would move the viewport to a
          // value the user is not looking at. (`parse` already returns NaN for
          // it; the explicit bail states the intent and survives a `parse`
          // change.)
          if (onCommitExpr && e.target.value.trim().startsWith("=")) return;
          // Every PARSEABLE change previews; partial/invalid text emits nothing
          // and leaves the FSM untouched (U3). Out-of-domain values are the
          // caller's to refuse — never clamped here.
          const n = parse(e.target.value);
          if (Number.isFinite(n) && (!kind || isValidForKind(kind, n))) {
            lastPreviewed.current = n;
            onPreview(n);
          }
        }}
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
              // Abandoning the edit abandons the preview claim with it: leaving
              // `lastPreviewed` set would let the guard swallow the next
              // legitimate echo of that same number.
              lastPreviewed.current = null;
              setText(expr ? `=${expr}` : formatValue(value));
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
      {shownSuffix && !exprText && <span className="text-ink-5">{shownSuffix}</span>}
      {/* The verdict, always: what the expression RESOLVED to, before anything
          is committed. */}
      {exprText !== null && (
        <ExprPreviewLine
          exprText={exprText}
          preview={preview}
          site={site}
          unit={unit}
          showMmHint={showMmHint}
        />
      )}
    </span>
  );
}
