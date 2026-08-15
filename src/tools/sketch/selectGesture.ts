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
import { pointKeyOf } from "./sketchTopology";

/** The gesture target kinds SCHEMA §7.4 defines. */
export type GestureKind = "point" | "arcEnd" | "radius" | "entityBody";

/**
 * The handle names a gesture can address. A strict subset of `ConstraintPosition`
 * — `Midpoint` is a CONSTRAINT target, never a minted wire point, so it can never
 * be grabbed. Narrowing here is what keeps the wire's `role` union honest.
 */
export type GestureRole = "Start" | "End" | "Center";

function gestureRole(position: ConstraintPosition): GestureRole | undefined {
  return position === "Start" || position === "End" || position === "Center"
    ? position
    : undefined;
}

/**
 * Kind for a NAMED point handle, or null when that handle is not draggable.
 *
 * One rule decides the two entries that are NOT plain point drags:
 *   - Arc Center → `entityBody`. A `point` drag pins every OTHER point, which for
 *     an arc includes its own endpoints — that over-determines radius+angles and
 *     a welded arc simply refuses to move. Translating an arc has to carry its
 *     endpoints, which is exactly what `entityBody` does.
 *   - Arc Start/End → `arcEnd`. These were NOT draggable before SP-2 (the comment
 *     said "derived from center + angles"); they are real solver points, and the
 *     `curves` channel is what finally carries the reshape back to the document.
 */
export function handleKind(
  type: SketchEntityType,
  position: ConstraintPosition | undefined,
): GestureKind | null {
  if (!position) return null;
  switch (type) {
    case "Line":
      return position === "Start" || position === "End" ? "point" : null;
    case "Circle":
      return position === "Center" ? "point" : null;
    case "Ellipse":
      // Its only minted wire point; the radii/rotation are scalars the solver
      // does not register, so there is nothing else to grab.
      return position === "Center" ? "point" : null;
    case "Arc":
      if (position === "Center") return "entityBody";
      return position === "Start" || position === "End" ? "arcEnd" : null;
    case "Point":
      return position === "Start" ? "point" : null;
    default:
      return null;
  }
}

/**
 * Kind for a CURVE-BODY pick (a hit with no named point), or null when the body
 * is not draggable: round curve = resize, straight curve = translate.
 *
 * An Ellipse body is null on purpose — it is not registered with the solver
 * (SCHEMA §7.4), so no gesture can move its parameters; it stays click-select.
 */
export function bodyKind(type: SketchEntityType): GestureKind | null {
  switch (type) {
    case "Line":
      return "entityBody"; // no centre to grab — the body IS the translate handle
    case "Circle":
    case "Arc":
      return "radius";
    default:
      return null;
  }
}

/**
 * A named point handle is draggable iff some gesture kind owns it.
 * Kept as the predicate name the select lane and its tests already use.
 */
export function isDraggableHandle(
  type: SketchEntityType,
  position: ConstraintPosition | undefined,
): boolean {
  return handleKind(type, position) !== null;
}

/** The gesture point ref for `beginGesture`: `"<entityId>.<Position>"` (null for a body).
 *  Minted through `sketchTopology.pointKeyOf` so this ref, the id-map keys and the
 *  solver's `positions` keys cannot drift apart (SKETCH-V2 P0.5). */
export function pointRef(sel: SketchSel): string | null {
  return sel.point ? pointKeyOf(sel.entityId, sel.point) : null;
}

/** A resolved drag intent: the picked target + everything `beginGesture` needs. */
export interface DragIntent {
  sel: SketchSel;
  kind: GestureKind;
  /** `"<entityId>.<Position>"` for `point`; the bare entity id for every other kind. */
  pointRef: string;
  /** Which handle was grabbed (`arcEnd` needs it; the backend ignores it elsewhere). */
  role?: GestureRole;
  /** Pointer-down plane position — the backend's per-kind offset baseline. */
  grab: [number, number];
}

/**
 * Classify a pointerdown hit into a drag intent, or null when the caller should
 * fall back to click-select (a miss, or a target no gesture kind owns).
 *
 * A hit WITH a named point resolves through `handleKind`; a body hit (no point)
 * resolves through `bodyKind` — that second rung is what makes a whole line
 * draggable and a circle's ring resizable, neither of which existed before SP-2.
 */
export function dragIntent(
  hit: SketchSel | null,
  entities: SketchEntity[],
  grab: [number, number],
): DragIntent | null {
  if (!hit) return null;
  const e = entities.find((x) => x.id === hit.entityId);
  if (!e) return null;
  if (hit.point) {
    const kind = handleKind(e.type, hit.point);
    if (!kind) return null;
    const ref = kind === "point" ? pointRef(hit) : hit.entityId;
    const role = gestureRole(hit.point);
    return ref ? { sel: hit, kind, pointRef: ref, ...(role ? { role } : {}), grab } : null;
  }
  const kind = bodyKind(e.type);
  return kind ? { sel: hit, kind, pointRef: hit.entityId, grab } : null;
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
