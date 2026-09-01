/*
 * The `=` expression lane behind a value input — the live preview, the two
 * authoring guardrails, and the text a resolved expression reads as.
 *
 * Split out of `DimensionInput` because none of it is about the input element:
 * it is a debounced call to `CadClient.evaluateExpression` plus the formatting
 * rules that call's answer is rendered with. Both clients serve that call
 * honestly (the mock through the shared TS port of the engine), so this lane
 * behaves identically with and without a backend.
 *
 * THE UNIT RULE, which is the whole reason the preview exists: a bare number
 * INSIDE an expression is millimetres (degrees at an angle site) — the site's
 * canonical unit, matching `regen::variables::substitute_variables`, with no
 * display-unit guesswork on the path to the document. A bare number typed on
 * its OWN is still read in the display unit, as it always was. Those two rules
 * disagree, so the preview always echoes what the expression actually resolved
 * to, and {@link isPlainLiteral} keeps `=2` out of the expression lane
 * entirely.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { createClient } from "@/ipc/client";
import type { EvaluatedExpression, ExpressionDimension } from "@/ipc/types";
import {
  formatLength,
  formatMillimetres,
  formatUnitless,
  lengthSuffix,
  MM_SUFFIX,
} from "@/units/format";
import type { LengthUnitId } from "@/units/lengthUnits";

/**
 * How long the field waits before evaluating what is being typed.
 *
 * Short enough that the preview feels attached to the keystroke, long enough
 * that a burst of typing is one call rather than one per character. Enter and
 * blur never wait for it — they flush (`evaluateNow`).
 */
export const EXPR_PREVIEW_DEBOUNCE_MS = 80;

/** One evaluation, tagged with the exact text it is FOR, so a late answer can
 *  never be rendered against text it was not computed from. */
export interface ExprPreview extends EvaluatedExpression {
  text: string;
}

/** The expression text of an `=` entry (leading `=` stripped, both sides
 *  trimmed), or `null` when the text is not an `=` entry at all — or is a bare
 *  `=` with nothing after it.
 *
 *  Deliberately NOT a validity check: which expressions are legal is the
 *  evaluator's to say, and a second grammar here would drift from it. This is
 *  only the `=` syntax that opens the lane. */
export function parseExprInput(text: string): string | null {
  const trimmed = text.trim();
  if (!trimmed.startsWith("=")) return null;
  const rest = trimmed.slice(1).trim();
  return rest === "" ? null : rest;
}

/**
 * A PURE numeric literal — no operator, no reference, no unit suffix.
 *
 * Guardrail: `=2` is not an expression anybody wants recorded, and recording it
 * would make the same two characters mean 2 mm while `2` on its own means 2 in.
 * A pure literal after `=` therefore commits through the PLAIN path (display
 * unit) with the `=` dropped, so the field has exactly one rule for a lone
 * number.
 */
export function isPlainLiteral(text: string): boolean {
  return /^[+-]?(\d+\.?\d*|\.\d+)$/.test(text.trim());
}

/**
 * The dimension a resolved value READS as.
 *
 * A `"scalar"` result carries no dimension of its own and coerces into the call
 * site verbatim (the engine's own site rule — the number is already in the
 * site's canonical unit), so `=w*2` over a plain `w` is millimetres in a length
 * field. Rendering it bare would hide exactly the unit the guardrails exist to
 * make explicit.
 */
export function readingDimension(
  p: EvaluatedExpression,
  site: ExpressionDimension,
): ExpressionDimension {
  return p.dimension === "scalar" ? site : p.dimension;
}

/**
 * What the preview line says for a successful evaluation.
 *
 * A LENGTH always leads with millimetres, because millimetres are what a bare
 * number inside the expression meant; under any other display unit the
 * equivalent follows in parentheses (`2 mm (0.079 in)`) so the field still
 * reads in the unit the user chose. An angle is degrees, and a genuine scalar
 * is bare.
 */
export function formatExprValue(
  p: EvaluatedExpression,
  site: ExpressionDimension,
  unit: LengthUnitId,
): string {
  switch (readingDimension(p, site)) {
    case "angle":
      return `${formatUnitless(p.value)}°`;
    case "scalar":
      return formatUnitless(p.value);
    default: {
      const mm = `${formatMillimetres(p.value)} ${MM_SUFFIX}`;
      return unit === MM_SUFFIX ? mm : `${mm} (${formatLength(p.value, unit)} ${lengthSuffix(unit)})`;
    }
  }
}

/** The nudge shown while authoring a length expression under a display unit
 *  that is NOT millimetres — the one place the two unit rules diverge. */
export const EXPR_MM_HINT = "bare numbers inside = are millimetres; use in/cm or a variable";

export interface ExprPreviewLane {
  /** The evaluation of the CURRENT text, or `null` while one is pending. */
  preview: ExprPreview | null;
  /**
   * Evaluate `text` now, bypassing the debounce — Enter and blur cannot wait
   * for a timer. Resolves with the same shape the debounce publishes, and
   * publishes it too when `text` is still what the field holds.
   */
  evaluateNow(text: string): Promise<ExprPreview>;
}

/**
 * Debounced live evaluation of `exprText` at `site`.
 *
 * `exprText` is the expression WITHOUT its `=`; `null` means the field is not
 * in expression mode and the lane is idle (no calls, no preview).
 *
 * Answers are memoized per text for the field's lifetime, which is what makes a
 * commit synchronous the moment the debounce has already landed. The table
 * cannot move underneath an open field — the user is typing in it — so the
 * memo cannot serve a value the document has since disagreed with.
 */
export function useExprPreview(
  exprText: string | null,
  site: ExpressionDimension,
): ExprPreviewLane {
  const [preview, setPreview] = useState<ExprPreview | null>(null);
  const cache = useRef(new Map<string, ExprPreview>());
  const latest = useRef<string | null>(exprText);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  // The site is half of the verdict, so a memo from another site is not this
  // field's answer. A field does not change site mid-life; the guard is here so
  // the memo can never answer for one it did not compute.
  const lastSite = useRef(site);
  if (lastSite.current !== site) {
    lastSite.current = site;
    cache.current.clear();
  }

  const evaluateNow = useCallback(
    async (text: string): Promise<ExprPreview> => {
      latest.current = text;
      const hit = cache.current.get(text);
      if (hit !== undefined) {
        if (alive.current) setPreview(hit);
        return hit;
      }
      let result: ExprPreview;
      try {
        result = { ...(await createClient().evaluateExpression(text, site)), text };
      } catch (e) {
        // Errors-as-values: a backend that cannot answer (no document open)
        // becomes a preview that says so, never a throw into a keystroke.
        result = {
          text,
          value: 0,
          dimension: site,
          error: e instanceof Error ? e.message : String(e),
        };
      }
      cache.current.set(text, result);
      if (alive.current && latest.current === text) setPreview(result);
      return result;
    },
    [site],
  );

  useEffect(() => {
    latest.current = exprText;
    if (exprText === null) {
      setPreview(null);
      return;
    }
    const hit = cache.current.get(exprText);
    if (hit !== undefined) {
      setPreview(hit);
      return;
    }
    // Clear first: showing the PREVIOUS text's number next to new text is a
    // wrong reading, not a stale one.
    setPreview(null);
    const timer = setTimeout(() => void evaluateNow(exprText), EXPR_PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [exprText, evaluateNow]);

  return { preview, evaluateNow };
}
