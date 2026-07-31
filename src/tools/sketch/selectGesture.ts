/*
 * Select-tool gesture logic (PURE) — the click-vs-drag decision + shift/meta
 * selection semantics + the draggable-handle predicate, factored out of the
 * imperative SketchController so the whole select flow is unit-tested by data.
 *
 * A pointerdown resolves (via `hitTestSketch`) to a `SketchSel`:
 *   - a DRAGGABLE point handle  → arm a drag gesture (Line Start/End, Circle /
 *     Arc Center, free Point Start). Arc Start/End + radii are NOT handles — the
 *     worker derives them from center + angles, so they cannot be dragged.
 *   - anything else (a body pick or a non-draggable point pick) → click-select.
 * A drag that never passes DRAG_PX collapses back to a click (handled upstream).
 *
 * The drag lane is fire-and-forget latest-wins: each `solveDrag` carries a `seq`;
 * `shouldApplyDrag` drops a null (already-dropped by the client) or an out-of-order
 * response so a stale preview never clobbers a newer one.
 */
import type { ConstraintPosition, SketchEntity, SketchEntityType } from "@/ipc/types";
import type { SketchSel } from "@/stores/sketchSelectionStore";
import { sameSketchSel } from "@/stores/sketchSelectionStore";

/**
 * A named point handle is draggable iff the worker owns it as a movable point:
 *   - Line    → Start / End
 *   - Circle  → Center
 *   - Ellipse → Center (its only minted wire point; the radii/rotation are scalars)
 *   - Arc     → Center only (Start/End are derived from center + angles)
 *   - Point   → Start (a free point)
 * A body pick (no `position`) is never a drag handle.
 */
export function isDraggableHandle(
  type: SketchEntityType,
  position: ConstraintPosition | undefined,
): boolean {
  if (!position) return false;
  switch (type) {
    case "Line":
      return position === "Start" || position === "End";
    case "Circle":
      return position === "Center";
    case "Ellipse":
      return position === "Center";
    case "Arc":
      return position === "Center";
    case "Point":
      return position === "Start";
    default:
      return false;
  }
}

/** The gesture point ref for `beginGesture`: `"<entityId>.<Position>"` (null for a body). */
export function pointRef(sel: SketchSel): string | null {
  return sel.point ? `${sel.entityId}.${sel.point}` : null;
}

/** A resolved drag intent: the picked handle + its `beginGesture` point ref. */
export interface DragIntent {
  sel: SketchSel;
  pointRef: string;
}

/**
 * Classify a pointerdown hit: a draggable handle → a `DragIntent`; a body pick, a
 * non-draggable point pick, or a miss → null (the caller falls back to click-select).
 */
export function dragIntent(hit: SketchSel | null, entities: SketchEntity[]): DragIntent | null {
  if (!hit || !hit.point) return null;
  const e = entities.find((x) => x.id === hit.entityId);
  if (!e || !isDraggableHandle(e.type, hit.point)) return null;
  const ref = pointRef(hit);
  return ref ? { sel: hit, pointRef: ref } : null;
}

/**
 * Next selection for a click:
 *   - plain    + hit  → `[hit]`            (replace)
 *   - plain    + miss → `[]`               (clear)
 *   - additive + hit  → toggle `hit`       (Shift / Meta)
 *   - additive + miss → unchanged          (never clears)
 */
export function clickSelection(
  current: SketchSel[],
  hit: SketchSel | null,
  additive: boolean,
): SketchSel[] {
  if (!hit) return additive ? current : [];
  if (!additive) return [hit];
  const has = current.some((s) => sameSketchSel(s, hit));
  return has ? current.filter((s) => !sameSketchSel(s, hit)) : [...current, hit];
}

/**
 * Latest-wins gate for a drag-solve response: apply iff it exists (a null was
 * already dropped client-side) AND its `seq` is newer than the last one applied.
 */
export function shouldApplyDrag(lastSeq: number, res: { seq: number } | null | undefined): boolean {
  return !!res && res.seq > lastSeq;
}
