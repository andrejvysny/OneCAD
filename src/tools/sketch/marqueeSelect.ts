/*
 * Marquee (box) selection for sketch mode — PURE geometry (W2-D).
 *
 * Two CAD-standard modes, chosen by the drag DIRECTION:
 *   - WINDOW   (left → right, solid border): an entity is hit only when it lies
 *     ENTIRELY inside the rect (full containment).
 *   - CROSSING (right → left, dashed border): a window hit OR any point where the
 *     entity's curve crosses the rect BOUNDARY.
 *
 * LEGACY DIVERGENCE (deliberate). The oracle UI *promised* these semantics —
 * `OneCAD-CPP/src/ui/viewport/ViewportRendering.cpp:468-473` picks the rubber-band
 * colour + dash pattern off `crossing = (end.x < start.x)`, "L->R = blue (window),
 * R->L = green (crossing)". But the selection itself never honoured it:
 * `Sketch::findInRect` (`OneCAD-CPP/src/core/sketch/Sketch.cpp:1978-2058`) builds a
 * per-entity bounding box and keeps it on `bounds.intersects(rect)` — a bbox-CROSSING
 * test, run identically in BOTH directions. So the legacy window drag selected
 * anything whose bbox merely grazed the rect, and a circle whose bbox overlapped was
 * picked even when the curve itself was nowhere near. We implement the PROMISED
 * semantics, and against real curves rather than bboxes.
 *
 * Consequences of testing real curves (all intentional):
 *   - a rect drawn wholly INSIDE a circle's interior selects nothing: it contains no
 *     part of the curve and crosses it nowhere (the legacy bbox test selected it),
 *   - an arc is gated on its SWEEP: only the axis extrema that actually lie on the
 *     arc count toward containment (`arcQuadrantPoints`), so a window drag around a
 *     90° arc does not have to cover the whole circle,
 *   - a bare `Point` entity has no curve, so CROSSING falls back to containment for
 *     it (a point is either in the rect or it is not).
 *
 * Coordinates are sketch-PLANE (u,v); the caller maps screen → plane (see
 * SketchController.planeMarqueeRect and its AABB approximation note).
 */
import type { SketchEntity } from "@/ipc/types";
import { arcQuadrantPoints, entityIntersections } from "./snapEngine";

export type MarqueeMode = "window" | "crossing";

/** An axis-aligned plane-space selection rect (already normalized: min ≤ max). */
export interface MarqueeRect {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/** Inclusive containment slack — a curve that lands exactly on the border counts. */
const EPS = 1e-9;

/** Normalize two opposite corners into a `MarqueeRect`. */
export function marqueeRectFrom(ax: number, ay: number, bx: number, by: number): MarqueeRect {
  return {
    minX: Math.min(ax, bx),
    minY: Math.min(ay, by),
    maxX: Math.max(ax, bx),
    maxY: Math.max(ay, by),
  };
}

/**
 * Mode from the drag direction in SCREEN x (never plane x — the plane basis can be
 * mirrored/rotated relative to the screen, and the user's mental model is "which way
 * did I drag"). Rightward (or straight down/up) ⇒ window; leftward ⇒ crossing.
 */
export function marqueeModeFor(downX: number, currentX: number): MarqueeMode {
  return currentX < downX ? "crossing" : "window";
}

/** Point-in-rect, inclusive of the border. */
export function rectContains(rect: MarqueeRect, p: readonly [number, number]): boolean {
  return (
    p[0] >= rect.minX - EPS &&
    p[0] <= rect.maxX + EPS &&
    p[1] >= rect.minY - EPS &&
    p[1] <= rect.maxY + EPS
  );
}

/**
 * The points that must ALL be inside the rect for a WINDOW hit — i.e. a tight
 * witness set for the entity's extent:
 *   - Point  → its position,
 *   - Line   → both endpoints,
 *   - Circle → the two opposite corners of its `center ± radius` bbox,
 *   - Arc    → its endpoints plus the axis extrema that lie ON the sweep.
 * `null` ⇒ the entity is missing the fields it needs and can never be selected.
 */
function containmentPoints(e: SketchEntity): Array<[number, number]> | null {
  switch (e.type) {
    case "Point":
      return e.p0 ? [e.p0] : null;
    case "Line":
      return e.p0 && e.p1 ? [e.p0, e.p1] : null;
    case "Circle": {
      if (!e.center || e.radius === undefined) return null;
      const [cx, cy] = e.center;
      const r = e.radius;
      return [
        [cx - r, cy - r],
        [cx + r, cy + r],
      ];
    }
    case "Arc": {
      if (!e.center || e.radius === undefined || !e.start || !e.end) return null;
      const pts: Array<[number, number]> = [e.start, e.end];
      for (const q of arcQuadrantPoints(e.center, e.radius, e.start, e.end)) pts.push([q.x, q.y]);
      return pts;
    }
    default:
      return null;
  }
}

/** WINDOW test: the entity lies entirely inside the rect. */
export function windowContains(e: SketchEntity, rect: MarqueeRect): boolean {
  const pts = containmentPoints(e);
  if (pts === null || pts.length === 0) return false;
  return pts.every((p) => rectContains(rect, p));
}

/**
 * The rect's four sides as synthetic `Line` entities, so the crossing test can reuse
 * `entityIntersections` (line×line / line×circle / line×arc, already sweep-gated and
 * segment-parameter-gated) instead of re-deriving clipping maths. Ids are
 * internal-only and never leave this module.
 */
function rectEdges(rect: MarqueeRect): SketchEntity[] {
  const { minX, minY, maxX, maxY } = rect;
  const corners: Array<[number, number]> = [
    [minX, minY],
    [maxX, minY],
    [maxX, maxY],
    [minX, maxY],
  ];
  return corners.map((p0, i) => ({
    id: `__marquee_edge_${i}`,
    type: "Line" as const,
    p0,
    p1: corners[(i + 1) % 4],
  }));
}

/** CROSSING test: a window hit, or the curve meets the rect boundary anywhere. */
export function crossingTouches(e: SketchEntity, rect: MarqueeRect, edges: SketchEntity[]): boolean {
  if (windowContains(e, rect)) return true;
  return edges.some((edge) => entityIntersections(e, edge).length > 0);
}

/**
 * Entity ids selected by a marquee. Order follows `entities` (stable), duplicates
 * are impossible since each entity is tested once.
 *
 * A degenerate (zero-width AND zero-height) rect resolves to nothing except a
 * `Point` sitting exactly on it: its synthesized edges are zero-length segments,
 * which `segSegIntersection` / `segCircleIntersections` both reject.
 */
export function marqueeHits(
  entities: readonly SketchEntity[],
  rect: MarqueeRect,
  mode: MarqueeMode,
): string[] {
  const edges = mode === "crossing" ? rectEdges(rect) : null;
  const out: string[] = [];
  for (const e of entities) {
    const hit = edges ? crossingTouches(e, rect, edges) : windowContains(e, rect);
    if (hit) out.push(e.id);
  }
  return out;
}
