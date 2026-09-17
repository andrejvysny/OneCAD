/*
 * Model-tool gesture ownership — the PURE half (no THREE, no stores).
 *
 * A value drag (extrude depth, edge-op size, shell thickness, face offset,
 * revolve angle, placement gizmo) is owned by ONE pointer from the grab until
 * it ends. It ends one of three ways, and the controller keeps them apart:
 * release (keep the value), cancel (put the pre-grab value back) and abandon
 * (tool teardown: write nothing). This module decides only the questions those
 * endings share: whose event is this, has the release already happened, and may
 * a cancel still restore.
 */

export type ModelGestureKind = "extrude" | "fillet" | "revolve" | "shell" | "offsetFace" | "transform";

export interface ModelGesture<S> {
  readonly kind: ModelGestureKind;
  /**
   * The owning pointer, or null when the grab event carried none (a synthetic
   * `MouseEvent`, or a test hook with no event at all). Null means "unknown",
   * which matches every pointer rather than none.
   */
  readonly pointerId: number | null;
  /** The owning pointer's type ("mouse" | "pen" | "touch"), or null when unknown. */
  readonly pointerType: string | null;
  /** The controller's arm generation at the grab: a newer arm owns the value. */
  readonly armGen: number;
  /** Everything a cancel needs to put back, captured before the grab changed it. */
  readonly snapshot: S;
  /** Set once the gesture has ended, so a late event for it cannot act again. */
  released: boolean;
}

export interface GestureOwner {
  pointerId: number | null;
  pointerType: string | null;
}

export function beginModelGesture<K extends ModelGestureKind, S>(
  kind: K,
  owner: GestureOwner,
  armGen: number,
  snapshot: S,
): ModelGesture<S> & { readonly kind: K } {
  return { kind, pointerId: owner.pointerId, pointerType: owner.pointerType, armGen, snapshot, released: false };
}

/** jsdom's `MouseEvent` has no `pointerId`; its `PointerEvent` defaults to 0. */
export function normalizePointerId(e: { pointerId?: unknown }): number | null {
  const id = e.pointerId;
  return typeof id === "number" && Number.isFinite(id) ? id : null;
}

/** The pointer a grab event names; an empty or missing type is unknown (null). */
export function gestureOwnerOf(e: { pointerId?: unknown; pointerType?: unknown } | null): GestureOwner {
  if (!e) return { pointerId: null, pointerType: null };
  const type = e.pointerType;
  return {
    pointerId: normalizePointerId(e),
    pointerType: typeof type === "string" && type !== "" ? type : null,
  };
}

export function pointerMatches(gesturePointerId: number | null, eventPointerId: number | null): boolean {
  return gesturePointerId === null || eventPointerId === null || gesturePointerId === eventPointerId;
}

/**
 * A mouse or pen sample whose primary bit is already clear while a gesture is
 * live: the pointerup was lost (window switch, OS gesture, a chorded release that
 * fires pointermove instead, a pen lifted into hover). Touch never hovers, and a
 * synthetic event has no pointer type, so neither may end a gesture here.
 */
export function isMissedRelease(e: { pointerType?: unknown; buttons?: unknown }): boolean {
  return (
    (e.pointerType === "mouse" || e.pointerType === "pen") &&
    typeof e.buttons === "number" &&
    (e.buttons & 1) === 0
  );
}

/**
 * A new PRIMARY contact of the owner's own pointer type, under a different id,
 * while the gesture is live: the owner's contact is already gone (its pointerup
 * never reached us), because a still-touching finger would make this one
 * non-primary. Needs both ids and a known type — anything less is not evidence.
 */
export function supersedesLostContact(
  g: ModelGesture<unknown> | null,
  e: { pointerId?: unknown; pointerType?: unknown; isPrimary?: unknown },
): boolean {
  if (!g || g.released || e.isPrimary !== true) return false;
  const next = gestureOwnerOf(e);
  if (g.pointerType === null || next.pointerType !== g.pointerType) return false;
  return g.pointerId !== null && next.pointerId !== null && next.pointerId !== g.pointerId;
}

/**
 * Whether an unexpected end signal (pointercancel, capture loss) for
 * `eventPointerId` ends `g`: only a live gesture, and only its own pointer's.
 */
export function cancelsLiveGesture(g: ModelGesture<unknown> | null, eventPointerId: number | null): boolean {
  return g !== null && !g.released && pointerMatches(g.pointerId, eventPointerId);
}

/**
 * A cancel restores only the arm that grabbed, and only while that arm's value is
 * still the dragged one. Anything else (a type flip re-armed the lane, the FSM
 * left its dragging phase) means the snapshot describes a state that is gone.
 */
export function restoreAllowed(g: ModelGesture<unknown>, armGen: number, stillDragging: boolean): boolean {
  return g.armGen === armGen && stillDragging;
}
