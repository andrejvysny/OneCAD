/**
 * The one place a commit result is turned into a verdict (roadmap WP0.3).
 *
 * Before this existed, every consumer family asked the same question its own way,
 * and all of them asked it wrong: `changedBodies.length === 0 && removedBodies
 * .length === 0 → failed`. Counting bodies cannot distinguish
 *
 *   - a regen that failed,
 *   - a no-op (nothing to rebuild),
 *   - a step left in `NeedsRepair` (document state, NOT a failure),
 *   - a delete-only outcome, and
 *   - a metadata-only outcome,
 *
 * so the last two were reported as failures and rolled back, and the third was
 * reported as SUCCESS. `ApplyOperationResult.terminal` carries the discriminant
 * the clients already know; this module is what consumers read instead of
 * re-deriving it.
 */

import type { ApplyOperationResult, OperationDiagnostic, RegenTerminal } from "./types";

/** What the caller should do about a commit. */
export type RegenOutcome =
  /** Geometry published. `result` is safe to apply and select from. */
  | { kind: "published"; result: ApplyOperationResult }
  /** Nothing to rebuild. Not an error, and nothing to select. */
  | { kind: "noop"; result: ApplyOperationResult }
  /**
   * The op's refs could not be resolved. First-class document state — the caller
   * shows the repair affordance and must NOT roll the record back, because the
   * record is exactly what repair operates on.
   */
  | { kind: "needsRepair"; result: ApplyOperationResult }
  /** The regen failed. `reason` is always non-empty so a caller never has to invent one. */
  | { kind: "failed"; reason: string; diagnostics?: OperationDiagnostic[] }
  /** No terminal arrived in time. Treated as a failure by callers, kept distinct for hints. */
  | { kind: "timeout"; reason: string };

const NO_RESULT = "the operation returned no result";
const NO_REASON = "the operation failed without a reason";
const TIMED_OUT = "the operation did not complete in time";

/**
 * Classifies one commit result.
 *
 * A resolved `errorMessage` is never a success, whatever the terminal says — that
 * is the invariant the whole work package exists for, so it is checked first.
 *
 * When `terminal` is absent the historical body-count inference runs instead, so
 * legacy fixtures and any client that has not populated the discriminant behave
 * exactly as they did before. Production results always carry it; see
 * `ApplyOperationResult.terminal`.
 */
export function classifyRegen(res: ApplyOperationResult | null | undefined): RegenOutcome {
  if (!res) return { kind: "failed", reason: NO_RESULT };
  if (res.errorMessage) {
    return { kind: "failed", reason: res.errorMessage, diagnostics: res.diagnostics };
  }
  const terminal: RegenTerminal | undefined = res.terminal;
  if (terminal === undefined) return legacyInference(res);
  switch (terminal) {
    case "published":
      return { kind: "published", result: res };
    case "noop":
      return { kind: "noop", result: res };
    case "needsRepair":
      return { kind: "needsRepair", result: res };
    case "timeout":
      return { kind: "timeout", reason: TIMED_OUT };
    case "failed":
      return { kind: "failed", reason: NO_REASON, diagnostics: res.diagnostics };
  }
}

/** True when the record survived — the caller keeps it and must not roll it back. */
export function keepsRecord(outcome: RegenOutcome): boolean {
  return outcome.kind === "published" || outcome.kind === "noop" || outcome.kind === "needsRepair";
}

/** The reason to show, or `null` when there is nothing to report. */
export function failureReason(outcome: RegenOutcome): string | null {
  return outcome.kind === "failed" || outcome.kind === "timeout" ? outcome.reason : null;
}

/**
 * The pre-discriminant behavior, kept verbatim for results that carry no
 * `terminal`. It cannot tell a delete-only or metadata-only success from a
 * failure — which is why it is the fallback and not the rule.
 */
function legacyInference(res: ApplyOperationResult): RegenOutcome {
  const empty = res.changedBodies.length === 0 && res.removedBodies.length === 0;
  return empty ? { kind: "failed", reason: "no body changed" } : { kind: "published", result: res };
}
