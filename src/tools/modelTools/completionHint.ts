/*
 * completionVerb — the status-bar verb shown when an extrude/revolve commit
 * succeeds (WP-U9). Pure and unit-tested so `finishExtrude`/`finishRevolve`
 * never diverge on wording between the two ops or across boolean modes.
 */
import type { BooleanMode } from "./modelToolMachine";

/**
 * `op` only feeds the defensive fallback (an unmapped `BooleanMode` future
 * value) — every known mode maps identically for extrude and revolve.
 */
export function completionVerb(
  op: "extrude" | "revolve",
  mode: BooleanMode,
  bodyCount: number,
): string {
  const plural = bodyCount > 1;
  switch (mode) {
    case "Cut":
      return plural ? `Cut created — ${bodyCount} bodies` : "Cut created";
    case "Add":
      return plural ? `Joined ${bodyCount} bodies` : "Joined";
    case "Intersect":
      return plural ? `Intersected — ${bodyCount} bodies` : "Intersected";
    case "NewBody":
      return plural ? `${bodyCount} bodies created` : "New body created";
    default:
      return op === "extrude" ? "Extruded" : "Revolved";
  }
}
