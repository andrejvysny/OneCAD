/*
 * snapCandidates — GENERATION half of the snap decision engine (PURE).
 *
 * Owns: the geometry primitives (ported from OneCAD-CPP `SnapManager`), the
 * per-source candidate generators, stable candidate IDs, the entity-derived
 * cache, and the document PROVENANCE (`FrontendPointRef`) each candidate
 * carries so an accepted placement can later author a constraint against the
 * point that produced it rather than against a re-derived coordinate.
 *
 * It does NOT decide anything. Scoring, compatibility, hysteresis and the final
 * point live in `snapArbitration.ts`; `snapEngine.ts` is the facade.
 *
 * ── Screen space, through a 2×2 metric ───────────────────────────────────────
 * Every distance here is CSS pixels measured through `PlaneScreenMetric`, and
 * every nearest-point projection MINIMIZES that pixel distance, not the plane
 * one. Under an oblique camera the two disagree: the plane-nearest point on a
 * circle is not the screen-nearest point on its projected ellipse, so a
 * plane-space solve followed by a pixel-space conversion picks the wrong point
 * on the curve — visibly, at the reach a snap operates over.
 *
 * ── Stable IDs ───────────────────────────────────────────────────────────────
 * A candidate's id must be a function of the GEOMETRY, never of array order or
 * of whether the cache was warm: the hysteresis latch and every tie-break key
 * off it. Floating-point coordinates therefore never appear in an id except as
 * canonical integer grid indices and canonical fixed angles.
 */
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import {
  ellipseParams,
  ellipsePointAt,
  nearestOnEllipse,
  type EllipseParams,
} from "./ellipseMath";
import { canonicalRole, pointKeyOf } from "./sketchTopology";
import {
  metricNorm,
  metricTensor,
  tensorDot,
  SEMANTIC_BIAS_PX,
  ORIGIN_CORE_PX,
  CANDIDATE_DEDUPE_TOL,
  type FrontendPointRef,
  type GuideLine,
  type MetricTensor,
  type PlaneScreenMetric,
  type SnapCandidate,
  type SnapClaim,
  type SnapKind,
  type SnapRelationIntent,
  type SnapSource,
} from "./snapTypes";

const EPS = 1e-9;
const TWO_PI = Math.PI * 2;

const dist = (a: Point2, b: Point2): number => Math.hypot(a.x - b.x, a.y - b.y);
const xy = (p: [number, number]): Point2 => ({ x: p[0], y: p[1] });

// ── Geometry primitives (ported verbatim from SnapManager.cpp) ───────────────

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

/** Circle quadrant points (0/90/180/270°), in canonical index order. */
export function circleQuadrantPoints(center: [number, number], radius: number): Point2[] {
  return QUADRANTS.map((a) => ({ x: center[0] + radius * Math.cos(a), y: center[1] + radius * Math.sin(a) }));
}

/** Arc quadrant points — only those inside the arc's angular extent. The INDEX
 *  is the canonical quadrant index (0..3), not the position in the output, so
 *  an id stays stable as the arc's sweep changes. */
export function arcQuadrantPointsIndexed(
  center: [number, number],
  radius: number,
  start: [number, number],
  end: [number, number],
): Array<{ index: number; point: Point2 }> {
  const out: Array<{ index: number; point: Point2 }> = [];
  QUADRANTS.forEach((a, index) => {
    if (arcContainsAngle(center, start, end, a)) {
      out.push({ index, point: { x: center[0] + radius * Math.cos(a), y: center[1] + radius * Math.sin(a) } });
    }
  });
  return out;
}

/** Arc quadrant points (legacy shape, no indices). */
export function arcQuadrantPoints(
  center: [number, number],
  radius: number,
  start: [number, number],
  end: [number, number],
): Point2[] {
  return arcQuadrantPointsIndexed(center, radius, start, end).map((q) => q.point);
}

/** Nearest point on a bounded line segment (clamped to [0,1]), plane-Euclidean. */
export function nearestOnSegment(p: Point2, a: [number, number], b: [number, number]): Point2 {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const len2 = dx * dx + dy * dy;
  if (len2 < 1e-12) return { x: a[0], y: a[1] };
  let t = ((p.x - a[0]) * dx + (p.y - a[1]) * dy) / len2;
  t = Math.max(0, Math.min(1, t));
  return { x: a[0] + t * dx, y: a[1] + t * dy };
}

/** Nearest point on a full circle, plane-Euclidean. */
export function nearestOnCircle(p: Point2, center: [number, number], radius: number): Point2 {
  const dx = p.x - center[0];
  const dy = p.y - center[1];
  const d = Math.hypot(dx, dy);
  if (d < 1e-12) return { x: center[0] + radius, y: center[1] };
  return { x: center[0] + (radius * dx) / d, y: center[1] + (radius * dy) / d };
}

/** Segment-segment intersection (both parameters within [0,1]); null if parallel
 *  or the crossing lies off either segment. Verbatim from
 *  `SnapManager::lineLineIntersection`. */
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

/** Segment-circle intersection points (segment parameter within [0,1]).
 *  Verbatim from `SnapManager::lineCircleIntersection`. */
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

/** Circle-circle intersection points (0, 1, or 2). Verbatim from
 *  `SnapManager::circleCircleIntersection`. */
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

export interface CurveInfo {
  kind: "line" | "circle" | "arc";
  a?: [number, number];
  b?: [number, number];
  center?: [number, number];
  radius?: number;
  start?: [number, number];
  end?: [number, number];
}

export function curveOf(e: SketchEntity): CurveInfo | null {
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

// ── Metric-aware nearest projections (SNAP §5.3.1) ───────────────────────────

/** Squared SCREEN distance from `raw` to `p`, through the metric tensor. */
function screenDist2(A: MetricTensor, raw: Point2, p: Point2): number {
  const dx = p.x - raw.x;
  const dy = p.y - raw.y;
  return tensorDot(A, dx, dy, dx, dy);
}

/**
 * Nearest point on the line `origin + t·dir` in SCREEN metric.
 *
 * `t = (dirᵀ A (raw − origin)) / (dirᵀ A dir)`; a degenerate denominator means
 * the direction projects to nothing on screen (edge-on), so there is no
 * meaningful nearest point and the caller must reject the candidate.
 */
export function nearestOnRayMetric(
  A: MetricTensor,
  raw: Point2,
  origin: Point2,
  dir: Point2,
  clampToUnit: boolean,
): Point2 | null {
  const dd = tensorDot(A, dir.x, dir.y, dir.x, dir.y);
  if (dd <= 1e-18) return null;
  const wx = raw.x - origin.x;
  const wy = raw.y - origin.y;
  let t = tensorDot(A, dir.x, dir.y, wx, wy) / dd;
  if (clampToUnit) t = Math.max(0, Math.min(1, t));
  return { x: origin.x + t * dir.x, y: origin.y + t * dir.y };
}

/** Uniform seed samples for the parametric curve minimizations. */
const CURVE_SEED_SAMPLES = 64;
/** Safeguarded Newton iterations; the bracket also stops at 1e-12 wide. */
const CURVE_REFINE_ITERATIONS = 24;

/**
 * Minimize `f(t) = (curve(t) − raw)ᵀ A (curve(t) − raw)` over `[t0, t1]`.
 *
 * Seed from uniform samples, bracket the best interval, then refine with a
 * safeguarded Newton step (on `f'`) that falls back to bisection whenever the
 * Newton step leaves the bracket. Derivatives are numeric on purpose: this runs
 * for circles, arcs AND ellipses, and one shared routine beats three
 * hand-differentiated ones that can disagree.
 */
export function minimizeOnCurve(
  at: (t: number) => Point2,
  A: MetricTensor,
  raw: Point2,
  t0: number,
  t1: number,
  samples = CURVE_SEED_SAMPLES,
): { t: number; point: Point2 } {
  const f = (t: number): number => screenDist2(A, raw, at(t));
  const span = t1 - t0;
  const h = span / samples;
  let bestT = t0;
  let bestF = Infinity;
  for (let i = 0; i <= samples; i++) {
    const t = t0 + i * h;
    const v = f(t);
    if (v < bestF) {
      bestF = v;
      bestT = t;
    }
  }
  let lo = Math.max(t0, bestT - h);
  let hi = Math.min(t1, bestT + h);
  // Derivative of f, by central difference on a bracket-relative step.
  const df = (t: number): number => {
    const d = Math.max((hi - lo) * 1e-6, 1e-12);
    return (f(t + d) - f(t - d)) / (2 * d);
  };
  let dLo = df(lo);
  for (let k = 0; k < CURVE_REFINE_ITERATIONS && hi - lo > 1e-12; k++) {
    const mid = (lo + hi) / 2;
    const dMid = df(mid);
    // Newton on f' using a secant slope over the bracket.
    const dHi = df(hi);
    const slope = (dHi - dLo) / Math.max(hi - lo, 1e-18);
    let next = Math.abs(slope) > 1e-18 ? mid - dMid / slope : mid;
    if (!(next > lo && next < hi) || !Number.isFinite(next)) next = mid; // bisect
    const dNext = df(next);
    if (dLo * dNext <= 0) {
      hi = next;
    } else {
      lo = next;
      dLo = dNext;
    }
  }
  const t = (lo + hi) / 2;
  return { t, point: at(t) };
}

/** Nearest point ON an entity's curve, in SCREEN metric. */
export function nearestOnCurveMetric(
  A: MetricTensor,
  raw: Point2,
  e: SketchEntity,
): Point2 | null {
  if (e.type === "Ellipse") {
    const params = ellipseParams(e);
    if (!params) return null;
    return minimizeOnCurve((t) => xy(ellipsePointAt(params, t)), A, raw, 0, TWO_PI).point;
  }
  const c = curveOf(e);
  if (!c) return null;
  if (c.kind === "line") {
    const a = xy(c.a!);
    const dir = { x: c.b![0] - c.a![0], y: c.b![1] - c.a![1] };
    if (Math.hypot(dir.x, dir.y) < 1e-12) return a;
    return nearestOnRayMetric(A, raw, a, dir, true);
  }
  const center = c.center!;
  const radius = c.radius!;
  const at = (t: number): Point2 => ({
    x: center[0] + radius * Math.cos(t),
    y: center[1] + radius * Math.sin(t),
  });
  if (c.kind === "circle") return minimizeOnCurve(at, A, raw, 0, TWO_PI).point;
  // Arc: minimize over the SWEEP only, and include both endpoints explicitly —
  // the constrained minimum can sit exactly at an end.
  const a0 = angleOf(center, c.start!);
  let a1 = angleOf(center, c.end!);
  if (a1 <= a0 + EPS) a1 += TWO_PI;
  const inner = minimizeOnCurve(at, A, raw, a0, a1);
  const candidates: Point2[] = [inner.point, xy(c.start!), xy(c.end!)];
  let best = candidates[0];
  let bestD = screenDist2(A, raw, best);
  for (const p of candidates.slice(1)) {
    const d = screenDist2(A, raw, p);
    if (d < bestD) {
      bestD = d;
      best = p;
    }
  }
  return best;
}

/**
 * Nearest point ON an entity's curve, plane-Euclidean.
 *
 * Kept for the HIT-TEST lane (`sketchHitTest`), which asks a different question
 * — "is the cursor within `pickTol` plane units of this curve?" — and has no
 * camera metric in scope. Snap arbitration must use `nearestOnCurveMetric`.
 */
export function nearestOnCurve(p: Point2, e: SketchEntity): Point2 | null {
  if (e.type === "Ellipse") {
    const params = ellipseParams(e);
    if (!params) return null;
    return nearestOnEllipse(p, params);
  }
  const c = curveOf(e);
  if (!c) return null;
  if (c.kind === "line") return nearestOnSegment(p, c.a!, c.b!);
  if (c.kind === "circle") return nearestOnCircle(p, c.center!, c.radius!);
  const onCircle = nearestOnCircle(p, c.center!, c.radius!);
  if (onCurveAngleOk(c, onCircle)) return onCircle;
  const ds = dist(p, xy(c.start!));
  const de = dist(p, xy(c.end!));
  return ds <= de ? xy(c.start!) : xy(c.end!);
}

/* The plane-Euclidean ellipse nearest point stays `ellipseMath.nearestOnEllipse`
 * — a 72-sample scan plus a 40-step ternary search, which is tighter than the
 * shared safeguarded-Newton minimizer and is already the pinned hit-test path.
 * `minimizeOnCurve` is used only for the METRIC form, where the objective is
 * anisotropic and no closed alternative exists. */

// ── Entity-derived candidate cache ───────────────────────────────────────────

export type CachedPointKind = "endpoint" | "midpoint" | "center" | "quadrant";

export interface CachedPointCandidate {
  point: Point2;
  kind: CachedPointKind;
  /** Stable candidate id (see the module header). */
  id: string;
  /** The document point this candidate IS, when it has an address. A midpoint
   *  and a quadrant are DERIVED — no independent point exists — so they carry
   *  none and can never author a point constraint. */
  ref?: FrontendPointRef;
  /** For a midpoint: the line it is the midpoint OF. */
  ownerLineId?: string;
  /** For a quadrant: the curve it is an extremum of. */
  ownerCurveId?: string;
}

export interface CachedIntersection {
  point: Point2;
  id: string;
  /** Sorted source entity pair — the curves an OnCurve intent would name. */
  curveIds: [string, string];
}

/** A reference point for H/V alignment guides, with its address. */
export interface CachedRef {
  point: Point2;
  key: string;
  ref?: FrontendPointRef;
}

/** Entity-derived snap candidates, precomputable once per sketch edit. */
export interface SnapCandidateCache {
  points: CachedPointCandidate[];
  intersections: CachedIntersection[];
  refs: CachedRef[];
}

/** The addressable + derived snap points of an entity, with provenance. */
export function entityCandidatePoints(e: SketchEntity): CachedPointCandidate[] {
  const out: CachedPointCandidate[] = [];
  const push = (
    point: Point2,
    kind: CachedPointKind,
    position?: "Start" | "End" | "Center",
    ownerLineId?: string,
  ): void => {
    // Canonicalize the role so `Point.Center` and `Point.Start` cannot mint two
    // ids for one coordinate (sketchTopology's blessed alias).
    const role = position ? canonicalRole(e.type, position) : null;
    out.push({
      point,
      kind,
      id: role
        ? `point:${pointKeyOf(e.id, role)}:${kind}`
        : `point:${e.id}:${kind}`,
      ...(role ? { ref: { entityId: e.id, position: role } } : {}),
      ...(ownerLineId ? { ownerLineId } : {}),
    });
  };
  if (e.type === "Point" && e.p0) push(xy(e.p0), "endpoint", "Start");
  if (e.type === "Line" && e.p0 && e.p1) {
    push(xy(e.p0), "endpoint", "Start");
    push(xy(e.p1), "endpoint", "End");
    // DERIVED: a line's midpoint has no independent address (`sketchTopology`
    // marks the slot `derived`), so it carries the owning LINE instead — which
    // is exactly what a `Midpoint(point, line)` constraint needs.
    push({ x: (e.p0[0] + e.p1[0]) / 2, y: (e.p0[1] + e.p1[1]) / 2 }, "midpoint", undefined, e.id);
  }
  if ((e.type === "Circle" || e.type === "Ellipse") && e.center) push(xy(e.center), "center", "Center");
  if (e.type === "Arc") {
    if (e.center) push(xy(e.center), "center", "Center");
    if (e.start) push(xy(e.start), "endpoint", "Start");
    if (e.end) push(xy(e.end), "endpoint", "End");
  }
  return out;
}

/** Legacy shape kept for `autoConstrain`-style callers. */
export function entitySnapPoints(e: SketchEntity): Array<{ point: Point2; kind: "endpoint" | "midpoint" | "center" }> {
  return entityCandidatePoints(e)
    .filter((c) => c.kind !== "quadrant")
    .map((c) => ({ point: c.point, kind: c.kind as "endpoint" | "midpoint" | "center" }));
}

/** Quadrant (extremum) candidates of one entity, with canonical indices. */
export function entityQuadrantPoints(e: SketchEntity): CachedPointCandidate[] {
  const mk = (index: number, point: Point2): CachedPointCandidate => ({
    point,
    kind: "quadrant",
    id: `quadrant:${e.id}:${index}`,
    ownerCurveId: e.id,
  });
  if (e.type === "Circle" && e.center && e.radius !== undefined) {
    return circleQuadrantPoints(e.center, e.radius).map((p, i) => mk(i, p));
  }
  if (e.type === "Arc" && e.center && e.radius !== undefined && e.start && e.end) {
    return arcQuadrantPointsIndexed(e.center, e.radius, e.start, e.end).map((q) => mk(q.index, q.point));
  }
  if (e.type === "Ellipse") {
    // P5: the four AXIS extrema, at eccentric parameters 0, π/2, π, 3π/2.
    const params = ellipseParams(e);
    if (!params) return [];
    return [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2].map((t, i) => mk(i, xy(ellipsePointAt(params, t))));
  }
  return [];
}

/** All intersection points between two entities (line/circle/arc/ellipse). */
export function entityIntersections(e1: SketchEntity, e2: SketchEntity): Point2[] {
  const ell1 = e1.type === "Ellipse" ? ellipseParams(e1) : null;
  const ell2 = e2.type === "Ellipse" ? ellipseParams(e2) : null;
  if (ell1 || ell2) return ellipseIntersections(e1, e2, ell1, ell2);

  const c1 = curveOf(e1);
  const c2 = curveOf(e2);
  if (!c1 || !c2) return [];

  const raw: Point2[] = [];
  if (c1.kind === "line" && c2.kind === "line") {
    const hit = segSegIntersection(c1.a!, c1.b!, c2.a!, c2.b!);
    if (hit) raw.push(hit);
  } else if (c1.kind === "line") {
    raw.push(...segCircleIntersections(c1.a!, c1.b!, c2.center!, c2.radius!));
  } else if (c2.kind === "line") {
    raw.push(...segCircleIntersections(c2.a!, c2.b!, c1.center!, c1.radius!));
  } else {
    raw.push(...circleCircleIntersections(c1.center!, c1.radius!, c2.center!, c2.radius!));
  }
  return raw.filter((p) => onCurveAngleOk(c1, p) && onCurveAngleOk(c2, p));
}

// ── Ellipse intersections (P5) ───────────────────────────────────────────────

/** Root-isolation samples per ellipse pair, before refinement. */
const ELLIPSE_ISOLATION_SAMPLES = 512;

/** Implicit value of a curve at `p`: zero ON the curve, signed off it. */
type Implicit = (p: Point2) => number;

/** `((x−cx)/a)² + ((y−cy)/b)² − 1` in the ellipse's own rotated frame. */
function ellipseImplicit(e: EllipseParams): Implicit {
  const cos = Math.cos(e.rotation);
  const sin = Math.sin(e.rotation);
  return (p) => {
    const dx = p.x - e.center[0];
    const dy = p.y - e.center[1];
    const u = dx * cos + dy * sin;
    const v = -dx * sin + dy * cos;
    return (u * u) / (e.majorR * e.majorR) + (v * v) / (e.minorR * e.minorR) - 1;
  };
}

/** Characteristic size of an ellipse — the tolerance scale for its implicit. */
const ellipseScale = (e: EllipseParams): number => Math.max(e.majorR, e.minorR, 1);

/**
 * Line ∩ ellipse, in the ellipse's LOCAL frame.
 *
 * With the segment `A + tD` transformed into local coordinates, the implicit
 * equation becomes a plain quadratic in `t`. A small NEGATIVE discriminant is
 * clamped to zero when it is within a scale-aware tolerance — that is the
 * tangent case, where exact arithmetic would give a double root and floating
 * point gives a tiny negative.
 */
export function segEllipseIntersections(
  a: [number, number],
  b: [number, number],
  e: EllipseParams,
): Point2[] {
  const cos = Math.cos(e.rotation);
  const sin = Math.sin(e.rotation);
  const toLocal = (p: [number, number]): [number, number] => {
    const dx = p[0] - e.center[0];
    const dy = p[1] - e.center[1];
    return [dx * cos + dy * sin, -dx * sin + dy * cos];
  };
  const [ax, ay] = toLocal(a);
  const [bx, by] = toLocal(b);
  const dx = bx - ax;
  const dy = by - ay;
  const ia = e.majorR * e.majorR;
  const ib = e.minorR * e.minorR;
  if (ia <= 0 || ib <= 0) return [];
  const qa = (dx * dx) / ia + (dy * dy) / ib;
  const qb = 2 * ((ax * dx) / ia + (ay * dy) / ib);
  const qc = (ax * ax) / ia + (ay * ay) / ib - 1;
  if (qa < 1e-18) return [];
  let disc = qb * qb - 4 * qa * qc;
  const discTol = 1e-12 * Math.max(1, qb * qb);
  if (disc < 0 && disc > -discTol) disc = 0; // tangent
  if (disc < 0) return [];
  const sq = Math.sqrt(disc);
  // Stable quadratic: use the sign-matched root first, then Vieta for the other,
  // so catastrophic cancellation cannot lose the near-zero root.
  const q = -0.5 * (qb + Math.sign(qb || 1) * sq);
  const roots = [q / qa, qc === 0 ? 0 : qc / q].filter((t) => Number.isFinite(t));
  const out: Point2[] = [];
  const scale = ellipseScale(e);
  for (const t of roots) {
    if (t < -1e-12 || t > 1 + 1e-12) continue;
    const tc = Math.min(1, Math.max(0, t));
    const p = { x: a[0] + tc * (b[0] - a[0]), y: a[1] + tc * (b[1] - a[1]) };
    if (out.some((o) => dist(o, p) <= 1e-9 * scale)) continue; // the tangent double root
    out.push(p);
  }
  return out;
}

/**
 * Ellipse ∩ (circle | arc | ellipse): parameterize the FIRST ellipse and solve
 * `g(t) = implicitB(ellipseA(t)) = 0`.
 *
 * Two isolation mechanisms, because one is not enough:
 *   - sign changes of `g` bracket the TRANSVERSAL roots;
 *   - sign changes of `g'` bracket the STATIONARY points, which is where a
 *     TANGENCY lives — `g` touches zero there without changing sign, so a
 *     sign-change scan alone silently misses every tangent contact.
 * A stationary point is accepted only when `|g|` is inside a scale-aware
 * tolerance, so a near-miss is not reported as an intersection.
 */
export function ellipseImplicitRoots(
  ea: EllipseParams,
  implicitB: Implicit,
  scale: number,
  samples = ELLIPSE_ISOLATION_SAMPLES,
): Point2[] {
  const at = (t: number): Point2 => xy(ellipsePointAt(ea, t));
  const g = (t: number): number => implicitB(at(t));
  const step = TWO_PI / samples;
  const gs: number[] = new Array(samples + 1);
  for (let i = 0; i <= samples; i++) gs[i] = g(i * step);
  const implicitTol = 1e-10 * Math.max(1, scale * scale);

  const roots: number[] = [];
  const refine = (lo: number, hi: number): number | null => {
    let a = lo;
    let b = hi;
    let ga = g(a);
    const gb = g(b);
    if (ga === 0) return a;
    if (gb === 0) return b;
    if (ga * gb > 0) return null;
    for (let k = 0; k < 80 && b - a > 1e-13; k++) {
      const m = 0.5 * (a + b);
      const gm = g(m);
      if (gm === 0) return m;
      if (ga * gm < 0) {
        b = m;
      } else {
        a = m;
        ga = gm;
      }
    }
    return 0.5 * (a + b);
  };

  for (let i = 0; i < samples; i++) {
    const t0 = i * step;
    const t1 = t0 + step;
    if (gs[i] === 0) {
      roots.push(t0);
      continue;
    }
    if (gs[i] * gs[i + 1] < 0) {
      const r = refine(t0, t1);
      if (r !== null) roots.push(r);
    }
  }

  // Stationary points: bracket sign changes of the numeric derivative.
  const dg = (t: number): number => (g(t + step * 1e-3) - g(t - step * 1e-3)) / (2 * step * 1e-3);
  let prev = dg(0);
  for (let i = 1; i <= samples; i++) {
    const t = i * step;
    const cur = dg(t);
    if (prev * cur < 0) {
      // Bisect the DERIVATIVE to locate the extremum, then test |g| there.
      let a = t - step;
      let b = t;
      let da = prev;
      for (let k = 0; k < 60 && b - a > 1e-13; k++) {
        const m = 0.5 * (a + b);
        const dm = dg(m);
        if (da * dm < 0) {
          b = m;
        } else {
          a = m;
          da = dm;
        }
      }
      const tm = 0.5 * (a + b);
      if (Math.abs(g(tm)) <= implicitTol) roots.push(tm);
    }
    prev = cur;
  }

  // Sort by the FIRST curve's parameter, then dedupe in plane space, so root
  // indices — and therefore candidate ids — are stable.
  roots.sort((p, q) => p - q);
  const out: Point2[] = [];
  const tol = 1e-7 * Math.max(1, scale);
  for (const t of roots) {
    const p = at(t);
    if (out.some((o) => dist(o, p) <= tol)) continue;
    out.push(p);
  }
  return out;
}

function ellipseIntersections(
  e1: SketchEntity,
  e2: SketchEntity,
  ell1: EllipseParams | null,
  ell2: EllipseParams | null,
): Point2[] {
  // Ellipse × line — the closed-form quadratic.
  if (ell1 && e2.type === "Line" && e2.p0 && e2.p1) return segEllipseIntersections(e2.p0, e2.p1, ell1);
  if (ell2 && e1.type === "Line" && e1.p0 && e1.p1) return segEllipseIntersections(e1.p0, e1.p1, ell2);

  if (ell1 && ell2) {
    const scale = Math.max(ellipseScale(ell1), ellipseScale(ell2));
    return ellipseImplicitRoots(ell1, ellipseImplicit(ell2), scale);
  }
  // Ellipse × circle/arc — the circle's implicit is `|p − c|² − r²`.
  const ell = ell1 ?? ell2;
  const other = ell1 ? e2 : e1;
  const c = other ? curveOf(other) : null;
  if (!ell || !c || (c.kind !== "circle" && c.kind !== "arc")) return [];
  const center = c.center!;
  const radius = c.radius!;
  const scale = Math.max(ellipseScale(ell), radius, 1);
  const implicit: Implicit = (p) =>
    (p.x - center[0]) * (p.x - center[0]) + (p.y - center[1]) * (p.y - center[1]) - radius * radius;
  const roots = ellipseImplicitRoots(ell, implicit, scale);
  // An ARC only counts inside its own sweep.
  return c.kind === "arc" ? roots.filter((p) => onCurveAngleOk(c, p)) : roots;
}

// ── Broad phase (P5) ─────────────────────────────────────────────────────────

export interface Aabb {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/** Exact axis-aligned bounds of an entity, or null when it is degenerate. */
export function entityAabb(e: SketchEntity): Aabb | null {
  if (e.type === "Line" && e.p0 && e.p1) {
    return {
      minX: Math.min(e.p0[0], e.p1[0]),
      maxX: Math.max(e.p0[0], e.p1[0]),
      minY: Math.min(e.p0[1], e.p1[1]),
      maxY: Math.max(e.p0[1], e.p1[1]),
    };
  }
  if ((e.type === "Circle" || e.type === "Arc") && e.center && e.radius !== undefined) {
    // Conservative for an arc (its own extent is tighter) — a broad phase may
    // over-report, never under-report.
    return {
      minX: e.center[0] - e.radius,
      maxX: e.center[0] + e.radius,
      minY: e.center[1] - e.radius,
      maxY: e.center[1] + e.radius,
    };
  }
  if (e.type === "Ellipse") {
    const p = ellipseParams(e);
    if (!p) return null;
    const cos = Math.cos(p.rotation);
    const sin = Math.sin(p.rotation);
    const ex = Math.hypot(p.majorR * cos, p.minorR * sin);
    const ey = Math.hypot(p.majorR * sin, p.minorR * cos);
    return {
      minX: p.center[0] - ex,
      maxX: p.center[0] + ex,
      minY: p.center[1] - ey,
      maxY: p.center[1] + ey,
    };
  }
  return null;
}

/** Broad-phase counters, for the perf tests. Reset by `buildSnapCache`. */
export interface BroadPhaseStats {
  pairsTotal: number;
  pairsTested: number;
}

let lastBroadPhase: BroadPhaseStats = { pairsTotal: 0, pairsTested: 0 };

/** Stats from the most recent `buildSnapCache` (test/telemetry only). */
export function lastBroadPhaseStats(): BroadPhaseStats {
  return lastBroadPhase;
}

/**
 * Sweep-and-prune over exact AABBs: only pairs whose bounds overlap reach the
 * exact intersection solver.
 *
 * The unconditional all-pairs scan it replaces cost an ellipse-pair root
 * isolation (512 samples + refinement) for every pair in the sketch, including
 * pairs on opposite sides of the drawing.
 */
function intersectionPairs(entities: SketchEntity[]): Array<[SketchEntity, SketchEntity]> {
  const boxed = entities
    .map((e) => ({ e, box: entityAabb(e) }))
    .filter((b): b is { e: SketchEntity; box: Aabb } => b.box !== null)
    .sort((a, b) => a.box.minX - b.box.minX || (a.e.id < b.e.id ? -1 : a.e.id > b.e.id ? 1 : 0));
  const out: Array<[SketchEntity, SketchEntity]> = [];
  const active: Array<{ e: SketchEntity; box: Aabb }> = [];
  let total = 0;
  for (const cur of boxed) {
    // Retire boxes that ended before this one starts.
    for (let i = active.length - 1; i >= 0; i--) {
      if (active[i].box.maxX < cur.box.minX) active.splice(i, 1);
    }
    for (const other of active) {
      total++;
      if (other.box.maxY < cur.box.minY || cur.box.maxY < other.box.minY) continue;
      out.push([other.e, cur.e]);
    }
    active.push(cur);
  }
  const n = entities.length;
  lastBroadPhase = { pairsTotal: (n * (n - 1)) / 2, pairsTested: out.length };
  void total;
  return out;
}

/**
 * Precompute the entity-derived snap candidates once per sketch edit — guide
 * points, quadrant points, alignment references and the (broad-phased)
 * intersection scan.
 *
 * `onCurve` is intentionally excluded: it depends on the live cursor AND on the
 * camera metric, so it is recomputed every call either way.
 */
export function buildSnapCache(entities: SketchEntity[]): SnapCandidateCache {
  const points: CachedPointCandidate[] = [];
  const refs: CachedRef[] = [];
  for (const e of entities) {
    for (const c of entityCandidatePoints(e)) {
      points.push(c);
      refs.push({
        point: c.point,
        key: c.ref ? pointKeyOf(c.ref.entityId, c.ref.position) : c.id,
        ...(c.ref ? { ref: c.ref } : {}),
      });
    }
  }
  for (const e of entities) points.push(...entityQuadrantPoints(e));

  const intersections: CachedIntersection[] = [];
  for (const [a, b] of intersectionPairs(entities)) {
    const hits = entityIntersections(a, b);
    if (hits.length === 0) continue;
    const pair: [string, string] = a.id < b.id ? [a.id, b.id] : [b.id, a.id];
    hits.forEach((point, rootIndex) => {
      intersections.push({ point, id: `intersection:${pair[0]}:${pair[1]}:${rootIndex}`, curveIds: pair });
    });
  }
  return { points, intersections, refs };
}

// ── Candidate generation ─────────────────────────────────────────────────────

/** Which sources are switched on for this sample. */
export interface SnapSourceToggles {
  guidePoints: boolean;
  guideLines: boolean;
  quadrant: boolean;
  intersection: boolean;
  onCurve: boolean;
  polar: boolean;
  grid: boolean;
}

/** One gesture anchor offered as an alignment reference. `key` must not embed a
 *  raw coordinate — it is the anchor INDEX plus the interaction epoch, so an id
 *  stays stable within a gesture and never collides across gestures. */
export interface GestureAnchor {
  point: Point2;
  key: string;
  ref?: FrontendPointRef;
}

export interface CandidateContext {
  raw: Point2;
  metric: PlaneScreenMetric;
  entities: SketchEntity[];
  cache?: SnapCandidateCache;
  gridStep: number;
  /** The user's point reach; also the guide/polar/on-curve reach. */
  acquirePx: number;
  /** The widest distance anything can still matter at (the release radius). */
  reachPx: number;
  /** Grid's own, cell-relative reach. */
  gridReachPx: number;
  sources: SnapSourceToggles;
  polarAnchor?: Point2 | null;
  polarRefDir?: Point2 | null;
  /** The entity the parallel/perpendicular rays are relative to, when known —
   *  what makes those two rays PERSISTABLE rather than placement-only. */
  polarRefLineId?: string | null;
  anchors?: readonly GestureAnchor[];
}

const originCandidateId = "origin:0:0";

function biasFor(kind: SnapKind, errorPx: number): number {
  switch (kind) {
    case "origin":
      return errorPx <= ORIGIN_CORE_PX ? SEMANTIC_BIAS_PX.originCore : SEMANTIC_BIAS_PX.origin;
    case "endpoint":
      return SEMANTIC_BIAS_PX.endpoint;
    case "midpoint":
      return SEMANTIC_BIAS_PX.midpoint;
    case "center":
      return SEMANTIC_BIAS_PX.center;
    case "intersection":
      return SEMANTIC_BIAS_PX.intersection;
    case "quadrant":
      return SEMANTIC_BIAS_PX.quadrant;
    case "onCurve":
      return SEMANTIC_BIAS_PX.onCurve;
    default:
      return 0;
  }
}

const KIND_LABEL: Partial<Record<SnapKind, string>> = {
  origin: "Origin",
  endpoint: "Endpoint",
  midpoint: "Midpoint",
  center: "Center",
  quadrant: "Quadrant",
  intersection: "Intersection",
  onCurve: "On Curve",
  grid: "Grid",
};

/** Build a full-point candidate (the common shape for every geometry snap). */
function pointCandidate(args: {
  id: string;
  kind: SnapKind;
  source: SnapSource;
  point: Point2;
  errorPx: number;
  refs?: readonly FrontendPointRef[];
  relationIntents?: readonly SnapRelationIntent[];
  label?: string;
}): SnapCandidate {
  const bias = biasFor(args.kind, args.errorPx);
  return {
    id: args.id,
    source: args.source,
    kind: args.kind,
    projection: { kind: "point", point: args.point },
    previewPoint: args.point,
    claims: POINT_CLAIM,
    errorPx: args.errorPx,
    semanticBiasPx: bias,
    scorePx: args.errorPx + bias,
    label: args.label ?? KIND_LABEL[args.kind] ?? "",
    guides: [],
    refs: args.refs ?? [],
    relationIntents: args.relationIntents ?? [],
  };
}

const POINT_CLAIM: readonly SnapClaim[] = ["point"];

/**
 * Generate every SPATIAL candidate within reach.
 *
 * Numeric candidates are NOT here — they are phase-local and come from the
 * live-dimension frame (`snapArbitration.numericCandidates`).
 */
export function generateCandidates(ctx: CandidateContext): SnapCandidate[] {
  const A = metricTensor(ctx.metric);
  const out: SnapCandidate[] = [];
  const errPx = (p: Point2): number => metricNorm(ctx.metric, p.x - ctx.raw.x, p.y - ctx.raw.y);
  const within = (p: Point2): number | null => {
    const d = errPx(p);
    return Number.isFinite(d) && d <= ctx.reachPx ? d : null;
  };

  // ── geometry points ──
  if (ctx.sources.guidePoints) {
    const originErr = within({ x: 0, y: 0 });
    if (originErr !== null) {
      out.push(
        pointCandidate({
          id: originCandidateId,
          kind: "origin",
          source: "geometryPoint",
          point: { x: 0, y: 0 },
          errorPx: originErr,
          relationIntents: [{ kind: "FixedOrigin", refs: [] }],
        }),
      );
    }
  }

  const cachedPoints = ctx.cache?.points ?? cachedPointsOf(ctx.entities);
  for (const c of cachedPoints) {
    const isQuadrant = c.kind === "quadrant";
    if (isQuadrant ? !ctx.sources.quadrant : !ctx.sources.guidePoints) continue;
    const d = within(c.point);
    if (d === null) continue;
    out.push(
      pointCandidate({
        id: c.id,
        kind: c.kind,
        source: "geometryPoint",
        point: c.point,
        errorPx: d,
        refs: c.ref ? [c.ref] : [],
        relationIntents: relationForCachedPoint(c),
      }),
    );
  }

  // ── intersections ──
  if (ctx.sources.intersection) {
    const inters = ctx.cache?.intersections ?? cachedIntersectionsOf(ctx.entities);
    for (const c of inters) {
      const d = within(c.point);
      if (d === null) continue;
      out.push(
        pointCandidate({
          id: c.id,
          kind: "intersection",
          source: "geometryPoint",
          point: c.point,
          errorPx: d,
          relationIntents: [{ kind: "OnCurve", refs: [], curveIds: c.curveIds }],
        }),
      );
    }
  }

  // ── on-curve (cursor-dependent, never cached) ──
  if (ctx.sources.onCurve) {
    for (const e of ctx.entities) {
      const p = nearestOnCurveMetric(A, ctx.raw, e);
      if (!p) continue;
      const d = within(p);
      if (d === null) continue;
      out.push(
        pointCandidate({
          id: `curve:${e.id}`,
          kind: "onCurve",
          source: "curve",
          point: p,
          errorPx: d,
          relationIntents: [{ kind: "OnCurve", refs: [], curveIds: [e.id] }],
        }),
      );
    }
  }

  // ── grid ──
  if (ctx.sources.grid && ctx.gridStep > 0 && ctx.gridReachPx > 0) {
    const iu = Math.round(ctx.raw.x / ctx.gridStep);
    const iv = Math.round(ctx.raw.y / ctx.gridStep);
    const point = { x: iu * ctx.gridStep, y: iv * ctx.gridStep };
    const d = errPx(point);
    if (Number.isFinite(d) && d <= ctx.gridReachPx) {
      out.push(
        pointCandidate({
          id: `grid:${iu}:${iv}:${canonicalNumber(ctx.gridStep)}`,
          kind: "grid",
          source: "grid",
          point,
          errorPx: d,
        }),
      );
    }
  }

  // ── H/V alignment guides ──
  if (ctx.sources.guideLines) out.push(...guideCandidates(ctx));

  // ── polar rays ──
  if (ctx.sources.polar && ctx.polarAnchor) out.push(...polarCandidates(ctx, A));

  // Stable order: the arbitration sorts by (score, id), but a deterministic
  // generation order makes the trace readable and the tests diffable.
  out.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return out;
}

function relationForCachedPoint(c: CachedPointCandidate): readonly SnapRelationIntent[] {
  if (c.kind === "midpoint" && c.ownerLineId) {
    return [{ kind: "Midpoint", refs: [], lineIds: [c.ownerLineId] }];
  }
  if (c.ref) return [{ kind: "Coincident", refs: [c.ref] }];
  // A QUADRANT (circle/arc/ellipse extremum) is a derived coordinate with no
  // point address of its own. Authoring `OnCurve` against the owning curve
  // would pin the point to the curve but NOT to the extremum, which is a
  // different — and wrong — relation, so it stays placement-only (SNAP §9.5).
  return [];
}

function cachedPointsOf(entities: SketchEntity[]): CachedPointCandidate[] {
  const out: CachedPointCandidate[] = [];
  for (const e of entities) {
    out.push(...entityCandidatePoints(e));
    out.push(...entityQuadrantPoints(e));
  }
  return out;
}

function cachedIntersectionsOf(entities: SketchEntity[]): CachedIntersection[] {
  const out: CachedIntersection[] = [];
  for (const [a, b] of intersectionPairs(entities)) {
    const hits = entityIntersections(a, b);
    const pair: [string, string] = a.id < b.id ? [a.id, b.id] : [b.id, a.id];
    hits.forEach((point, rootIndex) => {
      out.push({ point, id: `intersection:${pair[0]}:${pair[1]}:${rootIndex}`, curveIds: pair });
    });
  }
  return out;
}

/**
 * Alignment guides: the nearest reference point on each AXIS independently.
 *
 * Two candidates, not one: an x guide claims only `x` and a y guide only `y`,
 * which is what lets them COMPOSE. The legacy engine collapsed them into a
 * single result and therefore had to choose one axis to report.
 */
function guideCandidates(ctx: CandidateContext): SnapCandidate[] {
  const refs: CachedRef[] = [
    ...(ctx.anchors ?? []).map((a) => ({ point: a.point, key: a.key, ...(a.ref ? { ref: a.ref } : {}) })),
    ...(ctx.cache?.refs ?? entityRefsOf(ctx.entities)),
  ];
  let bestX: { r: CachedRef; d: number } | null = null;
  let bestY: { r: CachedRef; d: number } | null = null;
  for (const r of refs) {
    // Axis miss measured in SCREEN pixels: |Δx| alone would make the guide's
    // reach depend on zoom.
    const dx = metricNorm(ctx.metric, r.point.x - ctx.raw.x, 0);
    const dy = metricNorm(ctx.metric, 0, r.point.y - ctx.raw.y);
    if (dx <= ctx.reachPx && (bestX === null || dx < bestX.d - EPS)) bestX = { r, d: dx };
    if (dy <= ctx.reachPx && (bestY === null || dy < bestY.d - EPS)) bestY = { r, d: dy };
  }
  const out: SnapCandidate[] = [];
  if (bestX) {
    const guide: GuideLine = { orientation: "vertical", value: bestX.r.point.x, ref: bestX.r.point };
    out.push({
      id: `guide:x:${bestX.r.key}`,
      source: "guide",
      kind: "alignV",
      projection: { kind: "axisX", x: bestX.r.point.x },
      previewPoint: { x: bestX.r.point.x, y: ctx.raw.y },
      claims: ["x"],
      errorPx: bestX.d,
      semanticBiasPx: 0,
      scorePx: bestX.d,
      label: "Vertical",
      guides: [guide],
      refs: bestX.r.ref ? [bestX.r.ref] : [],
      // An x guide means "these two points share an x" — a VERTICAL alignment
      // of the point PAIR, which is `VerticalPoints`, not the line-only
      // `Vertical` (SNAP §9.4).
      relationIntents: bestX.r.ref ? [{ kind: "VerticalPoints", refs: [bestX.r.ref] }] : [],
    });
  }
  if (bestY) {
    const guide: GuideLine = { orientation: "horizontal", value: bestY.r.point.y, ref: bestY.r.point };
    out.push({
      id: `guide:y:${bestY.r.key}`,
      source: "guide",
      kind: "alignH",
      projection: { kind: "axisY", y: bestY.r.point.y },
      previewPoint: { x: ctx.raw.x, y: bestY.r.point.y },
      claims: ["y"],
      errorPx: bestY.d,
      semanticBiasPx: 0,
      scorePx: bestY.d,
      label: "Horizontal",
      guides: [guide],
      refs: bestY.r.ref ? [bestY.r.ref] : [],
      relationIntents: bestY.r.ref ? [{ kind: "HorizontalPoints", refs: [bestY.r.ref] }] : [],
    });
  }
  return out;
}

function entityRefsOf(entities: SketchEntity[]): CachedRef[] {
  const out: CachedRef[] = [];
  for (const e of entities) {
    for (const c of entityCandidatePoints(e)) {
      out.push({
        point: c.point,
        key: c.ref ? pointKeyOf(c.ref.entityId, c.ref.position) : c.id,
        ...(c.ref ? { ref: c.ref } : {}),
      });
    }
  }
  return out;
}

const POLAR_RAYS: { angle: number; deg: number; label: string }[] = [
  { angle: 0, deg: 0, label: "0°" },
  { angle: Math.PI / 4, deg: 45, label: "45°" },
  { angle: Math.PI / 2, deg: 90, label: "90°" },
  { angle: (3 * Math.PI) / 4, deg: 135, label: "135°" },
];

/**
 * Polar rays leaving the gesture anchor.
 *
 * Deduped MOD π (a line has no sense), fixed rays first so a chain running at
 * exactly 45° is named "45°" rather than "Parallel". A ray claims only `angle`,
 * so it composes with a rounded LENGTH — the composition the legacy single-slot
 * ladder could not represent at all.
 */
function polarCandidates(ctx: CandidateContext, A: MetricTensor): SnapCandidate[] {
  const anchor = ctx.polarAnchor!;
  const anchorKey = anchorKeyOf(ctx);
  const rays: Array<{ dir: Point2; id: string; label: string; intents: SnapRelationIntent[] }> =
    POLAR_RAYS.map((r) => ({
      dir: { x: Math.cos(r.angle), y: Math.sin(r.angle) },
      id: `polar:${anchorKey}:fixed:${r.deg}`,
      label: r.label,
      // A 0°/90° ray states the new segment is Horizontal/Vertical — a line-only
      // relation the tool authors on the committed entity. 45°/135° state
      // nothing any constraint kind can express, so they are placement-only.
      intents:
        r.deg === 0
          ? [{ kind: "HorizontalPoints", refs: [] }]
          : r.deg === 90
            ? [{ kind: "VerticalPoints", refs: [] }]
            : [],
    }));
  const refDir = ctx.polarRefDir;
  const len = refDir ? Math.hypot(refDir.x, refDir.y) : 0;
  if (refDir && len >= 1e-12) {
    const u = { x: refDir.x / len, y: refDir.y / len };
    const isNew = (d: Point2): boolean =>
      rays.every((o) => Math.abs(o.dir.x * d.y - o.dir.y * d.x) > 1e-9);
    const refId = ctx.polarRefLineId ?? "chain";
    if (isNew(u)) {
      rays.push({
        dir: u,
        id: `polar:${anchorKey}:parallel:${refId}`,
        label: "Parallel",
        intents: ctx.polarRefLineId ? [{ kind: "Parallel", refs: [], lineIds: [ctx.polarRefLineId] }] : [],
      });
    }
    const perp = { x: -u.y, y: u.x };
    if (isNew(perp)) {
      rays.push({
        dir: perp,
        id: `polar:${anchorKey}:perpendicular:${refId}`,
        label: "Perpendicular",
        intents: ctx.polarRefLineId
          ? [{ kind: "Perpendicular", refs: [], lineIds: [ctx.polarRefLineId] }]
          : [],
      });
    }
  }

  const out: SnapCandidate[] = [];
  for (const ray of rays) {
    const p = nearestOnRayMetric(A, ctx.raw, anchor, ray.dir, false);
    if (!p) continue; // the ray projects to nothing on screen
    // Cost is the OFF-RAY miss only: the snap slides the cursor ALONG the ray,
    // so travel along it is free by construction.
    const d = metricNorm(ctx.metric, p.x - ctx.raw.x, p.y - ctx.raw.y);
    if (!Number.isFinite(d) || d > ctx.reachPx) continue;
    out.push({
      id: ray.id,
      source: "polar",
      kind: "polar",
      projection: { kind: "ray", origin: anchor, dir: ray.dir },
      previewPoint: p,
      claims: ["angle"],
      errorPx: d,
      semanticBiasPx: 0,
      scorePx: d,
      label: ray.label,
      guides: [{ orientation: "polar", origin: anchor, dir: ray.dir }],
      refs: [],
      relationIntents: ray.intents,
    });
  }
  return out;
}

/** The polar fan's identity: the anchor's own key when the gesture supplies one,
 *  else its INDEX in the anchor list — never its decimal coordinates. */
function anchorKeyOf(ctx: CandidateContext): string {
  const anchors = ctx.anchors ?? [];
  const last = anchors.length > 0 ? anchors[anchors.length - 1] : null;
  return last ? last.key : "a0";
}

/** A float rendered so it can safely appear in an id (fixed precision, no
 *  exponent drift between equal values written differently). */
export function canonicalNumber(v: number): string {
  if (!Number.isFinite(v)) return "nan";
  const r = Math.round(v * 1e9) / 1e9;
  return String(r);
}

/** Drop duplicate candidates that resolve to the same point AND kind — two
 *  entities sharing a welded endpoint produce two ids for one coordinate. The
 *  LOWER id survives so the choice is a function of the geometry. */
export function dedupeCandidates(candidates: SnapCandidate[]): SnapCandidate[] {
  const out: SnapCandidate[] = [];
  for (const c of candidates) {
    const dup = out.find(
      (o) =>
        o.kind === c.kind &&
        o.projection.kind === c.projection.kind &&
        dist(o.previewPoint, c.previewPoint) <= CANDIDATE_DEDUPE_TOL,
    );
    if (!dup) {
      out.push(c);
      continue;
    }
    if (c.id < dup.id) out[out.indexOf(dup)] = c;
  }
  return out;
}
