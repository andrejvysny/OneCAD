/*
 * Sketch constraint-state → display text (chrome bar + inspector). One source so
 * the DOF wording stays consistent as the solver state updates (F-WP6).
 */
import type { SketchMeta, SketchStatus } from "@/stores/documentStore";

/**
 * Four real tones, not two. "under" is the expected state for most of a
 * sketch's life (normal, not a warning) — collapsing it into the same "warn"
 * bucket as "over"/"error" is exactly what made a fresh rectangle look broken
 * (UX audit). Reserve amber/red for states that actually need attention.
 */
export type StatusTone = "under" | "ok" | "over" | "error";

export function hasCurrentSketchEvaluation(
  sketch: SketchMeta | undefined,
): sketch is SketchMeta & { dof: number; status: SketchStatus; solveGeometryToken: string } {
  if (!sketch || typeof sketch.dof !== "number" || sketch.status === undefined) return false;
  const { geometryToken, solveGeometryToken } = sketch;
  return typeof geometryToken === "string" &&
    geometryToken.length > 0 &&
    typeof solveGeometryToken === "string" &&
    solveGeometryToken.length > 0 &&
    solveGeometryToken === geometryToken;
}

/** Short pill: "Under-constrained · DOF 3" / "Fully constrained · DOF 0" / … */
export function sketchStatusText(status: SketchStatus, dof: number): { label: string; tone: StatusTone } {
  switch (status) {
    case "ok":
      return { label: `Fully constrained · DOF ${dof}`, tone: "ok" };
    case "over":
      return { label: `Over-constrained · DOF ${dof}`, tone: "over" };
    case "error":
      return { label: `Conflicting · DOF ${dof}`, tone: "error" };
    default:
      // DOF 0 means no freedom remains, even while the solver status label lags
      // behind (a transient "under" between the last solve and the next status
      // flip) — never contradict the DOF count on the label.
      if (dof === 0) return { label: `Fully constrained · DOF 0`, tone: "ok" };
      return { label: `Under-constrained · DOF ${dof}`, tone: "under" };
  }
}

/** The one place tone maps to a text-color class — every display site (chrome
 *  bar, Inspector, status bar) must go through this instead of hand-rolling
 *  its own ternary, or the 4-state semantics drift back apart per-call-site. */
export function sketchStatusToneClass(tone: StatusTone): string {
  switch (tone) {
    case "ok":
      return "text-dof-ok";
    case "over":
      return "text-warn";
    case "error":
      return "text-traffic-close";
    default:
      return "text-dof-neutral";
  }
}

/** Whether this tone needs a "pay attention" treatment (bordered/tinted card)
 *  vs. the plain neutral one under/ok share. */
export function sketchStatusIsAlert(tone: StatusTone): boolean {
  return tone === "over" || tone === "error";
}

/**
 * Empty-sketch card copy (design item 12 / audit A11a). A sketch with zero
 * entities has nothing to be "fully defined" about — the solver's own
 * status/dof for a blank sketch (typically "ok"/0) reads through
 * `sketchStatusText`/`sketchStatusSentence` as "Fully constrained · DOF 0 —
 * Sketch is fully defined.", the exact false claim of completeness the audit
 * caught. A distinct function rather than an extra `sketchStatusText`
 * parameter: entity count is a different axis than solver status/dof, not a
 * variant reading of it, and callers decide which axis wins BEFORE calling
 * either — this module stays a pure string lookup, with no notion of
 * "session" or "entity" of its own.
 */
export function emptySketchCard(): { label: string; tone: StatusTone; sentence: string } {
  return { label: "Empty sketch", tone: "under", sentence: "Draw geometry to begin." };
}

/**
 * A sketch whose ONLY entities are projected host-face references (a fresh
 * sketch-on-face before the user draws anything). The solver reports those as
 * fully constrained, which is true and useless: nothing the user drew exists yet,
 * so the card says what the geometry IS instead of claiming completeness
 * (UX review 2026-09-11, "constraint messaging is not context-specific").
 */
export function projectedOnlySketchCard(projectedCount: number): {
  label: string;
  tone: StatusTone;
  sentence: string;
} {
  return {
    label: "Projected geometry only",
    tone: "under",
    sentence: `${projectedCount} projected reference ${projectedCount === 1 ? "edge" : "edges"} · draw geometry to begin.`,
  };
}

/** Inspector card body sentence. */
export function sketchStatusSentence(status: SketchStatus, dof: number): string {
  // DOF is not a redundant-constraint count — "over-constrained by N" claims a
  // number the solver never reports. State the fact, not a fabricated count.
  if (status === "over") return "Sketch is over-constrained. Remove or change a conflicting constraint.";
  if (status === "error") return "Conflicting constraints. Remove one to resolve.";
  if (status === "ok" || dof === 0) return "Sketch is fully defined.";
  return `${dof} degrees of freedom remain. Add distance or coincident constraints to fully define.`;
}
