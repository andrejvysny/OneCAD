/*
 * Sketch hit-test (PURE) — resolve a plane-space point to a sketch pick.
 *
 * Generalizes the two-tier resolution that lived inside `pickDimensionTarget`
 * into the shared `SketchSel` currency (entityId + optional named point):
 *   1. named point handles (entity endpoints / centers via `entityPoints`) win
 *      over body hits — a click on a vertex selects the vertex, not the curve;
 *   2. otherwise the entity whose curve the point lands on within `tolWorld`
 *      (`nearestOnCurve`, which already angle-gates arcs to their sweep).
 * Nearest wins within each tier; the higher tier (points) wins outright when any
 * candidate is in range. Returns null when nothing is within tolerance.
 *
 * `pickDimensionTarget` delegates here so the two share one resolution rule.
 * Point handles come from `entityPoints` (which carry the entityId + a real
 * ConstraintPosition) rather than snapEngine's `entitySnapPoints` (which carry
 * neither and also emit midpoints) — that keeps the dimension tool's behavior
 * identical and yields a well-typed `SketchSel.point`.
 */
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import type { SketchSel } from "@/stores/sketchSelectionStore";
import { nearestOnCurve } from "./snapEngine";
import { entityPoints } from "./autoConstrain";

export function hitTestSketch(
  raw: Point2,
  entities: SketchEntity[],
  tolWorld: number,
): SketchSel | null {
  // 1. Named point handles (higher tier — vertices win over the body).
  let bestPt: { d: number; sel: SketchSel } | null = null;
  for (const e of entities) {
    for (const p of entityPoints(e)) {
      const d = Math.hypot(raw.x - p.coord[0], raw.y - p.coord[1]);
      if (d <= tolWorld && (!bestPt || d < bestPt.d)) {
        bestPt = { d, sel: { entityId: p.entityId, point: p.position } };
      }
    }
  }
  if (bestPt) return bestPt.sel;

  // 2. Entity body under the cursor (nearestOnCurve is null for non-curves and
  //    angle-gates arcs, so only real Line/Circle/Arc bodies survive).
  let bestBody: { d: number; sel: SketchSel } | null = null;
  for (const e of entities) {
    const near = nearestOnCurve(raw, e);
    if (!near) continue;
    const d = Math.hypot(raw.x - near.x, raw.y - near.y);
    if (d > tolWorld) continue;
    if (!bestBody || d < bestBody.d) bestBody = { d, sel: { entityId: e.id } };
  }
  return bestBody?.sel ?? null;
}
