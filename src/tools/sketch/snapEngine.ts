/*
 * Frontend snap detection (PURE, NEW_SPEC §14 — the frontend owns snapping).
 *
 * Given the raw pointer position in plane (u,v) coords plus the current sketch
 * entities, returns the snapped point, the indicator kind (for the in-canvas
 * marker + hint chip), and any H/V alignment guide lines to draw.
 *
 * PRIORITY LADDER (highest wins). Ported from OneCAD-CPP `SnapManager` — the C++
 * `SnapType` enum orders snaps by priority (lower enum = higher priority) and
 * `SnapResult::operator<` sorts candidates by TYPE first, then distance. We mirror
 * that ordering with a per-kind `tier`; within a tier the nearest candidate wins.
 * All point-like snaps are gated to ~8px on screen (the pixel-space analogue of the
 * C++ 2mm sketch-coord snap radius — see the mapping note below):
 *
 *   1. Alt held           → no snap (raw point)                        [suppress]
 *   2. endpoint (tier 0)   line/arc endpoints                          [nearest]
 *   3. midpoint (tier 1)   line/arc midpoints
 *   4. center   (tier 2)   circle/arc/ellipse centers
 *   5. quadrant (tier 3)   circle/arc 0/90/180/270° points (arc: in-extent only)
 *   6. intersection (t4)   line-line / line-circle / circle-circle crossings
 *   7. onCurve  (tier 5)   nearest point ON a line/circle/arc          [lowest point tier]
 *   8. polar                a ray leaving the gesture anchor at 0/45/90/135°, or
 *                           parallel/perpendicular to the chain's direction
 *   9. H/V alignment guide  align to a reference point's x and/or y     [within 8px]
 *  10. grid                 round to the nearest grid step
 *  11. none                 raw point
 *
 * Polar sits directly ABOVE the H/V guides but wins only OUTRIGHT — a tie goes
 * to H/V, so the 0°/90° rays, which restate an alignment guide through the same
 * anchor, never rename a snap users already know. It is inert without an anchor,
 * so nothing about an idle cursor or a gesture's first click changes.
 *
 * 2mm↔8px mapping: the C++ app snaps in sketch millimetres (radius 2mm); the
 * frontend snaps in SCREEN pixels (8px by default, `snapRadius` preference) so
 * the target stays constant on screen at any zoom. `threshold = snapPx *
 * pixelWorld` converts that into world units at the cursor, so both models gate
 * on the same on-screen distance.
 *
 * Everything is pure so priority + guide math is unit-tested; the engine renders
 * the returned indicator/guides and the tool machines consume `point`.
 */
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { ellipseParams, nearestOnEllipse } from "./ellipseMath";
import type {
  GuideLine,
  SnapCandidate,
  SnapDecision,
  SnapKind,
  SnapSource,
} from "./snapTypes";

/*
 * `SnapKind` and `GuideLine` now live in `snapTypes.ts` — the composable model
 * needs them and must not depend on this module. Re-exported so the ~20
 * existing importers keep working unchanged.
 */
export type { GuideLine, SnapKind } from "./snapTypes";

export interface SnapResult {
  point: Point2;
  kind: SnapKind;
  /** Hint chip text (null when nothing to hint). */
  label: string | null;
  guides: GuideLine[];
  snapped: boolean;
}

export interface SnapOptions {
  /** World units between grid snap lines. */
  gridStep: number;
  /** World units per screen pixel at the cursor (sizes the pixel threshold). */
  pixelWorld: number;
  /**
   * Point-snap reach in SCREEN pixels — the user's `snapRadius` preference,
   * resolved through `snapRadius.ts`. Omitted ⇒ {@link SNAP_PX}, the reach every
   * build before the preference existed hard-coded, so leaving it out is
   * byte-identical to the old behavior.
   */
  snapPx?: number;
  enableGrid: boolean;
  /**
   * Gate the grid tier by proximity (within the pixel threshold) instead of
   * its default unconditional nearest-round. Set while a draw gesture is
   * armed AND dimension-round is on: the two tiers answer different
   * questions for the SAME point (grid quantizes x/y, dimension-round
   * quantizes length), so grid only gets to answer when the raw cursor is
   * actually near an intersection — otherwise length-rounding (applied
   * downstream, off `kind: "none"`) is the more useful fallback.
   */
  gridRequireProximity?: boolean;
  enableGuideLines: boolean;
  enableGuidePoints: boolean;
  /** Circle/arc 0/90/180/270° quadrant snaps (default on). */
  enableQuadrant?: boolean;
  /** Entity-entity intersection snaps (default on). */
  enableIntersection?: boolean;
  /** Nearest-point-on-curve snaps (default on). */
  enableOnCurve?: boolean;
  /**
   * Polar tracking (default OFF here — the controller opts in from the pref).
   * Snaps onto a ray leaving `polarAnchor` at 0/45/90/135°, plus the
   * parallel/perpendicular pair off `polarRefDir`. INERT without an anchor: no
   * anchor means no gesture in progress, and a fan centred on nothing would
   * drag a free cursor onto an arbitrary ray.
   */
  enablePolar?: boolean;
  /** Where the polar fan is centred — the gesture's LAST placed anchor. */
  polarAnchor?: Point2 | null;
  /**
   * The direction the chain is currently travelling in, which adds the
   * Parallel/Perpendicular pair to the fan. A direction that duplicates one of
   * the fixed rays (mod π — a line has no sense) is dropped rather than
   * offering the same line under two names.
   */
  polarRefDir?: Point2 | null;
  /** Alt held ⇒ raw point, no snap. */
  suppress: boolean;
  /** Extra reference points for H/V alignment (e.g. the current chain anchor). */
  recentPoints?: Point2[];
  /**
   * Precomputed entity-derived candidates from `buildSnapCache(entities)`. When
   * present, `computeSnap` skips re-deriving guide/quadrant/intersection points
   * from `entities` (the O(n²) all-pairs intersection scan in particular) and
   * threshold-filters the cached lists instead — per-category, so the enable*
   * toggles still gate exactly as they do without a cache. `onCurve` is cursor-
   * dependent and always recomputed live, cache or not. Omitting this leaves
   * behavior byte-identical to computing everything fresh every call.
   */
  cache?: SnapCandidateCache;
}

type CachedPointKind = "endpoint" | "midpoint" | "center" | "quadrant";

interface CachedPointCandidate {
  point: Point2;
  kind: CachedPointKind;
}

/** Entity-derived snap candidates, precomputable once per sketch edit (the
 *  entity arrays are replaced immutably on every commit — see `buildSnapCache`).
 *  Everything here depends only on `entities`, never on the cursor. */
export interface SnapCandidateCache {
  /** Guide points (endpoint/midpoint/center) and quadrant points, kinds kept
   *  distinct so per-category enable* toggles still gate correctly. */
  points: CachedPointCandidate[];
  /** All-pairs entity-entity intersection points (the O(n²) part). */
  intersections: Point2[];
  /** Entity-derived reference points for H/V alignment guides (recentPoints,
   *  being cursor/chain-dependent, are never part of the cache). */
  refs: Point2[];
}

/** DEFAULT point-snap reach in screen pixels (the pixel-space analogue of the C++
 *  2mm sketch-coord radius) — used when `SnapOptions.snapPx` is omitted. The live
 *  value is the user's `snapRadius` preference (`snapRadius.ts`, whose "m" IS this
 *  number); the controller resolves it and passes it here and into its own
 *  hit/dimension tolerances, so one reach still governs every point-like pick. */
export const SNAP_PX = 8;
const EPS = 1e-9;
const TWO_PI = Math.PI * 2;

const dist = (a: Point2, b: Point2): number => Math.hypot(a.x - b.x, a.y - b.y);

interface PointCandidate {
  point: Point2;
  kind: "endpoint" | "midpoint" | "center";
}

/** All snappable geometry points of an entity (endpoints, midpoint, center). */
export function entitySnapPoints(e: SketchEntity): PointCandidate[] {
  const out: PointCandidate[] = [];
  if (e.type === "Point" && e.p0) out.push({ point: xy(e.p0), kind: "endpoint" });
  if (e.type === "Line" && e.p0 && e.p1) {
    out.push({ point: xy(e.p0), kind: "endpoint" });
    out.push({ point: xy(e.p1), kind: "endpoint" });
    out.push({ point: { x: (e.p0[0] + e.p1[0]) / 2, y: (e.p0[1] + e.p1[1]) / 2 }, kind: "midpoint" });
  }
  // An Ellipse offers its CENTRE only (like a Circle). Its quadrant points are the
  // axis endpoints, which the legacy SnapManager never emitted for an ellipse —
  // adding them would be an invention, so they stay out (documented seam).
  if ((e.type === "Circle" || e.type === "Ellipse") && e.center)
    out.push({ point: xy(e.center), kind: "center" });
  if (e.type === "Arc") {
    if (e.center) out.push({ point: xy(e.center), kind: "center" });
    if (e.start) out.push({ point: xy(e.start), kind: "endpoint" });
    if (e.end) out.push({ point: xy(e.end), kind: "endpoint" });
  }
  return out;
}

// ── Geometry primitives (PURE, ported verbatim from SnapManager.cpp) ──────────

/** Angle of a point relative to a center, in [0, 2π). */
function angleOf(center: [number, number], p: [number, number]): number {
  const a = Math.atan2(p[1] - center[1], p[0] - center[0]);
  return a < 0 ? a + TWO_PI : a;
}

/** True if `angle` (radians) lies on the CCW arc sweep start→end (frontend arcs
 *  sweep CCW from `start` to `end`, matching arcTool). */
export function arcContainsAngle(
  center: [number, number],
  start: [number, number],
  end: [number, number],
  angle: number,
): boolean {
  const a0 = angleOf(center, start);
  const a1 = angleOf(center, end);
  let sweep = a1 - a0;
  if (sweep < 0) sweep += TWO_PI;
  if (sweep < EPS) sweep = TWO_PI; // full turn (start==end) ⇒ whole circle
  let rel = ((angle % TWO_PI) + TWO_PI) % TWO_PI - a0;
  if (rel < 0) rel += TWO_PI;
  return rel <= sweep + 1e-9;
}

const QUADRANTS = [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2];

/** Circle quadrant points (0/90/180/270°). */
export function circleQuadrantPoints(center: [number, number], radius: number): Point2[] {
  return QUADRANTS.map((a) => ({ x: center[0] + radius * Math.cos(a), y: center[1] + radius * Math.sin(a) }));
}

/** Arc quadrant points — only those inside the arc's angular extent. */
export function arcQuadrantPoints(
  center: [number, number],
  radius: number,
  start: [number, number],
  end: [number, number],
): Point2[] {
  const out: Point2[] = [];
  for (const a of QUADRANTS) {
    if (arcContainsAngle(center, start, end, a)) {
      out.push({ x: center[0] + radius * Math.cos(a), y: center[1] + radius * Math.sin(a) });
    }
  }
  return out;
}

/** Nearest point on a bounded line segment (clamped to [0,1]). */
export function nearestOnSegment(p: Point2, a: [number, number], b: [number, number]): Point2 {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const len2 = dx * dx + dy * dy;
  if (len2 < 1e-12) return { x: a[0], y: a[1] };
  let t = ((p.x - a[0]) * dx + (p.y - a[1]) * dy) / len2;
  t = Math.max(0, Math.min(1, t));
  return { x: a[0] + t * dx, y: a[1] + t * dy };
}

/** Nearest point on a full circle. */
export function nearestOnCircle(p: Point2, center: [number, number], radius: number): Point2 {
  const dx = p.x - center[0];
  const dy = p.y - center[1];
  const d = Math.hypot(dx, dy);
  if (d < 1e-12) return { x: center[0] + radius, y: center[1] };
  return { x: center[0] + (radius * dx) / d, y: center[1] + (radius * dy) / d };
}

/**
 * Segment-segment intersection (both parameters within [0,1]); null if parallel
 * or the crossing lies off either segment. Verbatim from
 * `SnapManager::lineLineIntersection`.
 */
export function segSegIntersection(
  p1: [number, number],
  p2: [number, number],
  p3: [number, number],
  p4: [number, number],
): Point2 | null {
  const d1x = p2[0] - p1[0];
  const d1y = p2[1] - p1[1];
  const d2x = p4[0] - p3[0];
  const d2y = p4[1] - p3[1];
  const cross = d1x * d2y - d1y * d2x;
  if (Math.abs(cross) < 1e-12) return null; // parallel / collinear
  const dx = p3[0] - p1[0];
  const dy = p3[1] - p1[1];
  const t = (dx * d2y - dy * d2x) / cross;
  const u = (dx * d1y - dy * d1x) / cross;
  if (t < 0 || t > 1 || u < 0 || u > 1) return null;
  return { x: p1[0] + t * d1x, y: p1[1] + t * d1y };
}

/**
 * Segment-circle intersection points (segment parameter within [0,1]). Verbatim
 * from `SnapManager::lineCircleIntersection`.
 */
export function segCircleIntersections(
  a: [number, number],
  b: [number, number],
  center: [number, number],
  radius: number,
): Point2[] {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const fx = a[0] - center[0];
  const fy = a[1] - center[1];
  const qa = dx * dx + dy * dy;
  const qb = 2 * (fx * dx + fy * dy);
  const qc = fx * fx + fy * fy - radius * radius;
  let disc = qb * qb - 4 * qa * qc;
  if (disc < 0 || qa < 1e-12) return [];
  disc = Math.sqrt(disc);
  const t1 = (-qb - disc) / (2 * qa);
  const t2 = (-qb + disc) / (2 * qa);
  const out: Point2[] = [];
  if (t1 >= 0 && t1 <= 1) out.push({ x: a[0] + t1 * dx, y: a[1] + t1 * dy });
  if (t2 >= 0 && t2 <= 1 && Math.abs(t2 - t1) > 1e-12) out.push({ x: a[0] + t2 * dx, y: a[1] + t2 * dy });
  return out;
}

/**
 * Circle-circle intersection points (0, 1, or 2). Verbatim from
 * `SnapManager::circleCircleIntersection` (tangent ⇒ 1 point, disjoint /
 * contained / concentric ⇒ 0).
 */
export function circleCircleIntersections(
  c1: [number, number],
  r1: number,
  c2: [number, number],
  r2: number,
): Point2[] {
  const dx = c2[0] - c1[0];
  const dy = c2[1] - c1[1];
  const d = Math.hypot(dx, dy);
  if (d > r1 + r2 + 1e-12 || d < Math.abs(r1 - r2) - 1e-12 || d < 1e-12) return [];
  const a = (r1 * r1 - r2 * r2 + d * d) / (2 * d);
  let h2 = r1 * r1 - a * a;
  if (h2 < 0) h2 = 0;
  const h = Math.sqrt(h2);
  const px = c1[0] + (a * dx) / d;
  const py = c1[1] + (a * dy) / d;
  const rx = -dy / d;
  const ry = dx / d;
  const out: Point2[] = [{ x: px + h * rx, y: py + h * ry }];
  if (h > 1e-12) out.push({ x: px - h * rx, y: py - h * ry });
  return out;
}

interface CurveInfo {
  kind: "line" | "circle" | "arc";
  a?: [number, number];
  b?: [number, number];
  center?: [number, number];
  radius?: number;
  start?: [number, number];
  end?: [number, number];
}

function curveOf(e: SketchEntity): CurveInfo | null {
  if (e.type === "Line" && e.p0 && e.p1) return { kind: "line", a: e.p0, b: e.p1 };
  if (e.type === "Circle" && e.center && e.radius !== undefined)
    return { kind: "circle", center: e.center, radius: e.radius };
  if (e.type === "Arc" && e.center && e.radius !== undefined && e.start && e.end)
    return { kind: "arc", center: e.center, radius: e.radius, start: e.start, end: e.end };
  return null;
}

/** Angle-gate a candidate on an arc curve; lines/circles always pass. */
function onCurveAngleOk(c: CurveInfo, p: Point2): boolean {
  if (c.kind !== "arc") return true;
  return arcContainsAngle(c.center!, c.start!, c.end!, Math.atan2(p.y - c.center![1], p.x - c.center![0]));
}

/** All intersection points between two entities (line/circle/arc). */
export function entityIntersections(e1: SketchEntity, e2: SketchEntity): Point2[] {
  const c1 = curveOf(e1);
  const c2 = curveOf(e2);
  if (!c1 || !c2) return [];

  const raw: Point2[] = [];
  const seg = (c: CurveInfo): [[number, number], [number, number]] => [c.a!, c.b!];

  if (c1.kind === "line" && c2.kind === "line") {
    const hit = segSegIntersection(...seg(c1), ...seg(c2));
    if (hit) raw.push(hit);
  } else if (c1.kind === "line" && (c2.kind === "circle" || c2.kind === "arc")) {
    raw.push(...segCircleIntersections(c1.a!, c1.b!, c2.center!, c2.radius!));
  } else if ((c1.kind === "circle" || c1.kind === "arc") && c2.kind === "line") {
    raw.push(...segCircleIntersections(c2.a!, c2.b!, c1.center!, c1.radius!));
  } else {
    // circle/arc × circle/arc
    raw.push(...circleCircleIntersections(c1.center!, c1.radius!, c2.center!, c2.radius!));
  }

  // Keep only points that lie within BOTH entities' angular extents (arcs).
  return raw.filter((p) => onCurveAngleOk(c1, p) && onCurveAngleOk(c2, p));
}

/**
 * Nearest point ON an entity's curve (segment / circle / arc / ellipse).
 *
 * The Ellipse branch is deliberately OUTSIDE `curveOf`: that structure feeds
 * `entityIntersections` (and, by the same shape, `trimMath`'s own `curveOf`), and
 * neither has closed-form ellipse intersection math. Keeping the ellipse out of
 * `CurveInfo` means it contributes no intersection snaps and no parametric trim —
 * both legacy-correct (the oracle's `TrimTool.cpp:298-299` whole-DELETES an
 * ellipse) — while still being hoverable/pickable through `nearestOnCurve`, which
 * is what `hitTestSketch` resolves bodies with.
 */
export function nearestOnCurve(p: Point2, e: SketchEntity): Point2 | null {
  if (e.type === "Ellipse") {
    const params = ellipseParams(e);
    return params ? nearestOnEllipse(p, params) : null;
  }
  const c = curveOf(e);
  if (!c) return null;
  if (c.kind === "line") return nearestOnSegment(p, c.a!, c.b!);
  if (c.kind === "circle") return nearestOnCircle(p, c.center!, c.radius!);
  // arc: nearest on the full circle, snapping to the closer endpoint when outside.
  const onCircle = nearestOnCircle(p, c.center!, c.radius!);
  if (onCurveAngleOk(c, onCircle)) return onCircle;
  const ds = dist(p, xy(c.start!));
  const de = dist(p, xy(c.end!));
  return ds <= de ? xy(c.start!) : xy(c.end!);
}

// ── Ranked candidate ladder ───────────────────────────────────────────────────

const KIND_LABEL: Record<string, string> = {
  origin: "Origin",
  endpoint: "Endpoint",
  midpoint: "Midpoint",
  center: "Center",
  quadrant: "Quadrant",
  intersection: "Intersection",
  onCurve: "On Curve",
};

// Tier = C++ SnapType priority (lower = higher priority). Endpoint > Midpoint >
// Center > Quadrant > Intersection > OnCurve, then guides, then grid.
const KIND_TIER: Record<string, number> = {
  endpoint: 0,
  midpoint: 1,
  center: 2,
  // The origin sits with QUADRANT: below the three snaps that name a specific
  // relationship to drawn geometry (endpoint / midpoint / center), above the
  // two derived ones (intersection / onCurve). Ranking it top would let it steal
  // a snap from an endpoint the cursor is visibly closer to, purely because the
  // sketch happens to be drawn near (0,0); ranking it bottom would lose it to
  // any stray curve passing the origin, which is the case the audit is about.
  origin: 3,
  quadrant: 3,
  intersection: 4,
  onCurve: 5,
};

interface Ranked {
  point: Point2;
  kind: Exclude<SnapKind, "none" | "grid" | "alignH" | "alignV" | "alignHV" | "polar">;
  d: number;
  tier: number;
}

function collectPointCandidates(
  raw: Point2,
  entities: SketchEntity[],
  opts: SnapOptions,
  threshold: number,
): Ranked[] {
  const out: Ranked[] = [];
  const consider = (point: Point2, kind: Ranked["kind"]): void => {
    const d = dist(raw, point);
    if (d <= threshold) out.push({ point, kind, d, tier: KIND_TIER[kind] });
  };

  // The origin exists in every sketch and needs no entity to hang off, but it is
  // still a POINT snap: it follows the same guide-point preference as endpoint /
  // midpoint / center, so "turn point snapping off" means all of them.
  if (opts.enableGuidePoints) consider({ x: 0, y: 0 }, "origin");

  const cache = opts.cache;
  if (cache) {
    // endpoint / midpoint / center (gated by sketch-guide-points).
    if (opts.enableGuidePoints) {
      for (const c of cache.points) if (c.kind !== "quadrant") consider(c.point, c.kind);
    }
    // quadrant.
    if (opts.enableQuadrant ?? true) {
      for (const c of cache.points) if (c.kind === "quadrant") consider(c.point, "quadrant");
    }
    // intersection (all pairs, precomputed; only crossings near the cursor survive).
    if (opts.enableIntersection ?? true) {
      for (const p of cache.intersections) consider(p, "intersection");
    }
  } else {
    // endpoint / midpoint / center (gated by sketch-guide-points).
    if (opts.enableGuidePoints) {
      for (const e of entities) for (const c of entitySnapPoints(e)) consider(c.point, c.kind);
    }
    // quadrant.
    if (opts.enableQuadrant ?? true) {
      for (const e of entities) {
        if (e.type === "Circle" && e.center && e.radius !== undefined) {
          for (const q of circleQuadrantPoints(e.center, e.radius)) consider(q, "quadrant");
        } else if (e.type === "Arc" && e.center && e.radius !== undefined && e.start && e.end) {
          for (const q of arcQuadrantPoints(e.center, e.radius, e.start, e.end)) consider(q, "quadrant");
        }
      }
    }
    // intersection (all pairs; only crossings near the cursor survive the threshold).
    if (opts.enableIntersection ?? true) {
      for (let i = 0; i < entities.length; i++) {
        for (let j = i + 1; j < entities.length; j++) {
          for (const p of entityIntersections(entities[i], entities[j])) consider(p, "intersection");
        }
      }
    }
  }
  // onCurve (lowest point tier) — cursor-dependent, always live regardless of cache.
  if (opts.enableOnCurve ?? true) {
    for (const e of entities) {
      const p = nearestOnCurve(raw, e);
      if (p) consider(p, "onCurve");
    }
  }
  return out;
}

/**
 * Precompute the entity-derived snap candidates once per sketch edit — guide
 * points, quadrant points, and the O(n²) all-pairs intersection scan. Pass the
 * result as `opts.cache` to `computeSnap` on every subsequent pointer move
 * until `entities` changes (session entity arrays are replaced immutably on
 * every commit, so reference equality is a valid invalidation key — see
 * `SketchController.snapAt`). `onCurve` is intentionally excluded: it depends
 * on the live cursor position and is recomputed every call either way.
 */
export function buildSnapCache(entities: SketchEntity[]): SnapCandidateCache {
  const points: CachedPointCandidate[] = [];
  const refs: Point2[] = [];
  for (const e of entities) {
    for (const c of entitySnapPoints(e)) {
      points.push(c);
      refs.push(c.point);
    }
  }
  for (const e of entities) {
    if (e.type === "Circle" && e.center && e.radius !== undefined) {
      for (const q of circleQuadrantPoints(e.center, e.radius)) points.push({ point: q, kind: "quadrant" });
    } else if (e.type === "Arc" && e.center && e.radius !== undefined && e.start && e.end) {
      for (const q of arcQuadrantPoints(e.center, e.radius, e.start, e.end)) points.push({ point: q, kind: "quadrant" });
    }
  }
  const intersections: Point2[] = [];
  for (let i = 0; i < entities.length; i++) {
    for (let j = i + 1; j < entities.length; j++) {
      intersections.push(...entityIntersections(entities[i], entities[j]));
    }
  }
  return { points, intersections, refs };
}

// ── Guide tiers (polar + H/V), ranked by how far they MOVE the point ─────────

/** A guide-tier candidate. `d` is the displacement the snap would apply — the
 *  one metric the polar and H/V tiers are compared on. */
interface GuideHit {
  point: Point2;
  kind: SnapKind;
  label: string;
  guides: GuideLine[];
  d: number;
}

const POLAR_RAYS: { angle: number; label: string }[] = [
  { angle: 0, label: "0°" },
  { angle: Math.PI / 4, label: "45°" },
  { angle: Math.PI / 2, label: "90°" },
  { angle: (3 * Math.PI) / 4, label: "135°" },
];

/**
 * The candidate polar LINES through the anchor: the four fixed rays, plus the
 * chain direction and its perpendicular when one is known.
 *
 * Deduped MOD π, because a line has no sense: `dir` and `−dir` are one candidate,
 * so two directions collide when their cross product vanishes, not when they are
 * equal. The fixed rays are listed first, so a chain running at exactly 45° is
 * reported as "45°" rather than "Parallel" — one line, one name, deterministically.
 */
function polarDirections(refDir?: Point2 | null): { dir: Point2; label: string }[] {
  const out = POLAR_RAYS.map((r) => ({
    dir: { x: Math.cos(r.angle), y: Math.sin(r.angle) },
    label: r.label,
  }));
  const len = refDir ? Math.hypot(refDir.x, refDir.y) : 0;
  if (!refDir || len < 1e-12) return out;
  const u = { x: refDir.x / len, y: refDir.y / len };
  const isNew = (d: Point2): boolean => out.every((o) => Math.abs(o.dir.x * d.y - o.dir.y * d.x) > 1e-9);
  if (isNew(u)) out.push({ dir: u, label: "Parallel" });
  const perp = { x: -u.y, y: u.x };
  if (isNew(perp)) out.push({ dir: perp, label: "Perpendicular" });
  return out;
}

/** Nearest polar ray within `threshold`, by PERPENDICULAR distance (the snap
 *  slides the cursor ALONG the ray, so only the off-ray miss is a cost). */
function polarSnap(raw: Point2, opts: SnapOptions, threshold: number): GuideHit | null {
  const anchor = opts.polarAnchor;
  if (!opts.enablePolar || !anchor) return null;
  const wx = raw.x - anchor.x;
  const wy = raw.y - anchor.y;
  let best: GuideHit | null = null;
  for (const { dir, label } of polarDirections(opts.polarRefDir)) {
    const off = Math.abs(dir.x * wy - dir.y * wx); // |dir × w|; dir is a unit vector
    if (off > threshold || (best && off >= best.d - EPS)) continue;
    const along = wx * dir.x + wy * dir.y;
    best = {
      point: { x: anchor.x + along * dir.x, y: anchor.y + along * dir.y },
      kind: "polar",
      label,
      guides: [{ orientation: "polar", origin: anchor, dir }],
      d: off,
    };
  }
  return best;
}

/** H/V alignment onto a reference point's x and/or y, within `threshold`. */
function guideSnap(
  raw: Point2,
  entities: SketchEntity[],
  opts: SnapOptions,
  threshold: number,
): GuideHit | null {
  if (!opts.enableGuideLines) return null;
  const refs = referencePoints(entities, opts.recentPoints, opts.cache);
  let vGuide: number | null = null; // constant x
  let hGuide: number | null = null; // constant y
  let vBest = threshold;
  let hBest = threshold;
  let vPt: Point2 | null = null; // the reference point that produced vGuide
  let hPt: Point2 | null = null; // the reference point that produced hGuide
  for (const r of refs) {
    const dx = Math.abs(raw.x - r.x);
    if (dx <= vBest) {
      vBest = dx;
      vGuide = r.x;
      vPt = r;
    }
    const dy = Math.abs(raw.y - r.y);
    if (dy <= hBest) {
      hBest = dy;
      hGuide = r.y;
      hPt = r;
    }
  }
  if (vGuide === null && hGuide === null) return null;
  const guides: GuideLine[] = [];
  // `vPt`/`hPt` are set in the same branch as `vGuide`/`hGuide` above, so
  // non-null here is guaranteed, not merely hoped for.
  if (vGuide !== null) guides.push({ orientation: "vertical", value: vGuide, ref: vPt! });
  if (hGuide !== null) guides.push({ orientation: "horizontal", value: hGuide, ref: hPt! });
  const both = vGuide !== null && hGuide !== null;
  // "Aligned" means the cursor sits on BOTH axes of the SAME reference point —
  // a true 2D coincidence. When vGuide/hGuide instead come from two unrelated
  // points that just happen to each land within threshold on one axis, calling
  // that "Aligned" is misleading (the cursor isn't aligned to anything single);
  // report only the axis this tier is surest about instead.
  const samePoint =
    both && vPt !== null && hPt !== null && Math.abs(vPt.x - hPt.x) < EPS && Math.abs(vPt.y - hPt.y) < EPS;
  const kind: SnapKind = samePoint ? "alignHV" : vBest <= hBest ? "alignV" : "alignH";
  const label = samePoint ? "Aligned" : vBest <= hBest ? "Vertical" : "Horizontal";
  return {
    point: { x: vGuide ?? raw.x, y: hGuide ?? raw.y },
    kind,
    label,
    guides,
    // Ranked by the SMALLER axis miss, not the combined displacement: it is the
    // axis this tier is surest about, and it keeps a polar ray that merely
    // RESTATES an alignment (0°/90° through the same anchor) at a tie, which
    // polar loses. Anything polar wins is a line H/V could not have offered.
    d: Math.min(vGuide !== null ? vBest : Infinity, hGuide !== null ? hBest : Infinity),
  };
}

export function computeSnap(
  raw: Point2,
  entities: SketchEntity[],
  opts: SnapOptions,
): SnapResult {
  if (opts.suppress) {
    return { point: raw, kind: "none", label: null, guides: [], snapped: false };
  }

  const threshold = (opts.snapPx ?? SNAP_PX) * opts.pixelWorld;

  // 2. Point-like snaps, ranked by (tier, distance) — C++ SnapResult::operator<.
  const candidates = collectPointCandidates(raw, entities, opts, threshold);
  if (candidates.length > 0) {
    let best = candidates[0];
    for (const c of candidates) {
      if (c.tier < best.tier || (c.tier === best.tier && c.d < best.d - EPS)) best = c;
    }
    return { point: best.point, kind: best.kind, label: KIND_LABEL[best.kind], guides: [], snapped: true };
  }

  // 3. Guide tiers: polar rays off the gesture anchor, then H/V alignment.
  // Polar is the more specific statement (it knows where the chain is heading)
  // so it is offered first, but it must win OUTRIGHT — `>=` would let a 0°/90°
  // ray relabel an alignment guide it lands identically on.
  const polar = polarSnap(raw, opts, threshold);
  const guide = guideSnap(raw, entities, opts, threshold);
  const hit = polar && (!guide || polar.d < guide.d - EPS) ? polar : guide;
  if (hit) {
    return { point: hit.point, kind: hit.kind, label: hit.label, guides: hit.guides, snapped: true };
  }

  // 4. Grid snap.
  if (opts.enableGrid && opts.gridStep > 0) {
    const point = {
      x: Math.round(raw.x / opts.gridStep) * opts.gridStep,
      y: Math.round(raw.y / opts.gridStep) * opts.gridStep,
    };
    const close = !opts.gridRequireProximity || Math.hypot(point.x - raw.x, point.y - raw.y) <= threshold;
    if (close) {
      return { point, kind: "grid", label: "Grid", guides: [], snapped: true };
    }
  }

  return { point: raw, kind: "none", label: null, guides: [], snapped: false };
}

/**
 * Wrap a legacy single-winner {@link SnapResult} as a {@link SnapDecision}
 * carrying exactly one accepted candidate — the P1→P2 BRIDGE.
 *
 * P1 needs the decision SHAPE (so the commit path can capture an immutable
 * intent at click time) before P2 supplies the real composable arbitration.
 * Encoding the legacy answer honestly means: one candidate, claiming the whole
 * point, with a zero semantic bias, because this engine did not rank on
 * distance-plus-bias at all. P2 deletes this function.
 */
export function legacySnapDecision(
  result: SnapResult,
  raw: Point2,
  traceId: number,
): SnapDecision {
  if (!result.snapped) {
    return {
      raw,
      point: result.point,
      accepted: [],
      rejected: [],
      primaryId: null,
      primaryKind: "none",
      label: null,
      guides: [],
      snapped: false,
      traceId,
    };
  }
  const id = `legacy:${result.kind}`;
  const candidate: SnapCandidate = {
    id,
    source: legacySource(result.kind),
    kind: result.kind,
    projection: { kind: "point", point: result.point },
    previewPoint: result.point,
    claims: ["point"],
    errorPx: 0,
    semanticBiasPx: 0,
    scorePx: 0,
    label: result.label ?? "",
    guides: result.guides,
    refs: [],
    relationIntents: [],
  };
  return {
    raw,
    point: result.point,
    accepted: [candidate],
    rejected: [],
    primaryId: id,
    primaryKind: result.kind,
    label: result.label,
    guides: result.guides,
    snapped: true,
    traceId,
  };
}

function legacySource(kind: SnapKind): SnapSource {
  switch (kind) {
    case "onCurve":
      return "curve";
    case "alignH":
    case "alignV":
    case "alignHV":
      return "guide";
    case "polar":
      return "polar";
    case "grid":
      return "grid";
    default:
      return "geometryPoint";
  }
}

// ── helpers ───────────────────────────────────────────────────────────────

function xy(p: [number, number]): Point2 {
  return { x: p[0], y: p[1] };
}

function referencePoints(entities: SketchEntity[], recent: Point2[] = [], cache?: SnapCandidateCache): Point2[] {
  if (cache) return [...recent, ...cache.refs];
  const refs: Point2[] = [...recent];
  for (const e of entities) {
    for (const c of entitySnapPoints(e)) refs.push(c.point);
  }
  return refs;
}
