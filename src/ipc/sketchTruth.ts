import type { SketchTruth } from "./types";
import { frontendConflictingIds, type SketchIdMap } from "./sketchWireMap";

export function frontendSketchTruth(map: SketchIdMap | undefined, truth: SketchTruth): SketchTruth {
  const id = (backendId: string): string | undefined =>
    map ? frontendConflictingIds(map, [backendId])[0] : backendId;
  const reasons: NonNullable<SketchTruth["reasons"]> = {};
  for (const [backendId, reason] of Object.entries(truth.reasons ?? {})) {
    const frontendId = id(backendId);
    if (frontendId) reasons[frontendId] = reason;
  }
  return {
    reasons,
    unassessed: (truth.unassessed ?? []).flatMap((backendId) => {
      const frontendId = id(backendId);
      return frontendId ? [frontendId] : [];
    }),
    violations: (truth.violations ?? []).flatMap((violation) => {
      const frontendId = id(violation.constraintId);
      return frontendId ? [{ ...violation, constraintId: frontendId }] : [];
    }),
    normalizedMaxResidual: truth.normalizedMaxResidual,
    diagnostics: truth.diagnostics ?? [],
  };
}
