/*
 * solveResult — the ONE place a solver result is reconciled into entities.
 *
 * A `SketchUpsertResult` reports movement on TWO independent channels:
 *   `solvedPositions` — point coordinates that moved (Line.Start, Arc.Center, …)
 *   `solvedCurves`    — curve PARAMETERS that changed (radius, arc angles)
 *
 * They are not redundant. A `Radius` dimension on a circle moves no point at all
 * — the whole change rides `solvedCurves` — so a publication path that applies
 * only positions renders the OLD radius while the backend holds the new one.
 * That divergence is invisible until the next edit rebases on the stale
 * geometry, or until a quadrant snap offers a point that is not on the circle
 * the user can see.
 *
 * Before this module the two calls were composed by hand at ten sites and four
 * of them applied only positions. `solveResult.test.ts` pins that no production
 * file outside this one imports both halves, so a new publication path cannot
 * quietly reintroduce the split.
 */
import { applySolvedCurves, applySolvedPositions } from "@/ipc/sketchWireMap";
import type { SketchEntity, SketchUpsertResult } from "@/ipc/types";

/** The two solve channels, as any result-shaped object carries them. */
export type SolvePublication = Pick<SketchUpsertResult, "solvedPositions" | "solvedCurves">;

/**
 * Apply BOTH solve channels to `entities`, positions first.
 *
 * Order is load-bearing: `applySolvedCurves` rebuilds an arc's endpoint
 * coordinates from `(center, radius, angle)`, so it must see the centre the
 * solve moved, not the pre-solve one.
 *
 * Pure. Returns the SAME array reference when nothing moved, which is what
 * keeps the snap cache's reference-equality invalidation cheap and stops React
 * from re-rendering on an identity solve.
 */
export function applySketchSolveResult(
  entities: SketchEntity[],
  result: SolvePublication | null | undefined,
): SketchEntity[] {
  if (!result) return entities;
  return applySolvedCurves(
    applySolvedPositions(entities, result.solvedPositions ?? {}),
    result.solvedCurves ?? {},
  );
}
