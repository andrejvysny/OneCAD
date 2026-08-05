/*
 * trimMath — PURE parametric-trim geometry for the sketch Trim tool.
 *
 * Clicking a line/circle/arc removes the SPAN between its two nearest crossings
 * with other entities (the classic CAD trim), leaving the surviving pieces. This
 * module owns the math; the controller renders a doomed-piece ghost on hover and
 * the service commits the surviving pieces through one sketchUpsert.
 *
 * ── Param spaces ──────────────────────────────────────────────────────────────
 *   Line   : t ∈ [0,1] along p0→p1.
 *   Arc    : s ∈ [0,1] sweep-relative — the angle normalized within the CCW
 *            start→end sweep (handles the 0-rad crossing).
 *   Circle : θ ∈ [0,2π) absolute angle, treated as a RING (wraparound intervals).
 *
 * ── Crossings ─────────────────────────────────────────────────────────────────
 * Intersections are computed against each other entity's FULL underlying curve
 * (infinite line / full circle) and then gated to BOTH entities' extents (a line
 * to its segment, an arc to its sweep). A tangency yields ONE crossing. Crossings
 * are deduped (the same point reached via two entities collapses); crossings that
 * land on the TARGET's own endpoints are excluded (they don't cut the span).
 *
 * ── Result ────────────────────────────────────────────────────────────────────
 * The clicked param falls inside one removal interval [a,b]; the surviving pieces
 * are the complement. A surviving piece shorter than `minSize` (arc length for
 * curves) is discarded (merged into the removed span). Line → ≤2 sub-lines;
 * Arc → ≤2 sub-arcs; Circle (needs ≥2 crossings) → the ONE complementary arc.
 * A line/arc with no qualifying crossings (or a circle with <2) returns null —
 * the caller falls back to whole-entity delete.
 *
 * An ELLIPSE is never parametrically trimmed: `curveOf` does not model it, so
 * `trimPieces` returns null and the caller whole-DELETES it. That is exact legacy
 * parity — `OneCAD-CPP/src/core/sketch/tools/TrimTool.cpp:298-299` dispatches
 * `EntityType::Ellipse` straight to `sketch.removeEntity(entityId)`.
 *
 * Pure ⇒ every edge is unit-tested by data (see trimMath.test.ts). Intersection
 * math is implemented here (param-returning) rather than reusing snapEngine's
 * point-returning helpers; the tests cross-check a few points against those.
 */
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import type { DraftEntity } from "./toolMachine";

/** Plane (u,v) pair. Exported because `offsetMath` reuses the three full-curve
 *  intersection primitives below rather than growing a second copy of them. */
export type XY = [number, number];

/** Param-equality epsilon — FIXED 1e-9 in param space (line/arc t,s ∈ [0,1]) and
 *  radian space (circle θ), never scaled by length/radius. Adequate at CAD scales:
 *  on a 1000mm line 1e-9 param ≈ 1e-6mm; endpoint crossings land ~1e-12 from the
 *  bound, so exclusion stays robust. */
const EPS = 1e-9;
const TWO_PI = Math.PI * 2;

// ── Curve model ───────────────────────────────────────────────────────────────

/** The three curve kinds trim models. EXPORTED (additive) because `extendMath`
 *  works in the SAME curve model — a second copy would be free to drift, and the
 *  two tools must agree on what a crossing is or Trim/Extend become inverses in
 *  name only. */
export type Curve =
  | { kind: "line"; a: XY; b: XY }
  | { kind: "circle"; c: XY; r: number }
  | { kind: "arc"; c: XY; r: number; start: XY; end: XY };

export function curveOf(e: SketchEntity | DraftLike): Curve | null {
  if (e.type === "Line" && e.p0 && e.p1) return { kind: "line", a: e.p0, b: e.p1 };
  if (e.type === "Circle" && e.center && e.radius !== undefined)
    return { kind: "circle", c: e.center, r: e.radius };
  if (e.type === "Arc" && e.center && e.radius !== undefined && e.start && e.end)
    return { kind: "arc", c: e.center, r: e.radius, start: e.start, end: e.end };
  return null;
}

/** Entity-shaped input (array coords) — SketchEntity or its field subset. */
interface DraftLike {
  type: SketchEntity["type"];
  construction?: boolean;
  p0?: XY;
  p1?: XY;
  center?: XY;
  radius?: number;
  start?: XY;
  end?: XY;
}

// ── Angle helpers ─────────────────────────────────────────────────────────────

/** Angle of a point about a center, in [0, 2π). Exported for `extendMath`, whose
 *  arc param space is stated against this SAME [0,2π) convention. */
export function angleOf(c: XY, p: XY): number {
  const a = Math.atan2(p[1] - c[1], p[0] - c[0]);
  return a < 0 ? a + TWO_PI : a;
}

/** CCW sweep magnitude start→end in (0, 2π]. Exported alongside `angleOf`: the
 *  0-rad-crossing handling here is subtle enough that a duplicate would be a bug
 *  waiting to happen. */
export function arcSweep(c: XY, start: XY, end: XY): number {
  const a0 = angleOf(c, start);
  const a1 = angleOf(c, end);
  let sweep = a1 - a0;
  if (sweep < 0) sweep += TWO_PI;
  if (sweep < EPS) sweep = TWO_PI; // start==end ⇒ full turn
  return sweep;
}

// ── Full-curve intersection points (no extent gating — gated afterwards) ───────

/** Two infinite lines: 0 or 1 point (empty if parallel/collinear). */
export function lineLineFull(p1: XY, p2: XY, p3: XY, p4: XY): XY[] {
  const d1x = p2[0] - p1[0];
  const d1y = p2[1] - p1[1];
  const d2x = p4[0] - p3[0];
  const d2y = p4[1] - p3[1];
  const cross = d1x * d2y - d1y * d2x;
  if (Math.abs(cross) < 1e-12) return [];
  const t = ((p3[0] - p1[0]) * d2y - (p3[1] - p1[1]) * d2x) / cross;
  return [[p1[0] + t * d1x, p1[1] + t * d1y]];
}

/** Infinite line vs full circle: 0, 1 (tangent), or 2 points. */
export function lineCircleFull(a: XY, b: XY, c: XY, r: number): XY[] {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const fx = a[0] - c[0];
  const fy = a[1] - c[1];
  const qa = dx * dx + dy * dy;
  const qb = 2 * (fx * dx + fy * dy);
  const qc = fx * fx + fy * fy - r * r;
  if (qa < 1e-12) return [];
  let disc = qb * qb - 4 * qa * qc;
  if (disc < 0) return [];
  disc = Math.sqrt(disc);
  const t1 = (-qb - disc) / (2 * qa);
  const t2 = (-qb + disc) / (2 * qa);
  const out: XY[] = [[a[0] + t1 * dx, a[1] + t1 * dy]];
  if (Math.abs(t2 - t1) > 1e-12) out.push([a[0] + t2 * dx, a[1] + t2 * dy]);
  return out;
}

/** Two full circles: 0, 1 (tangent), or 2 points. */
export function circleCircleFull(c1: XY, r1: number, c2: XY, r2: number): XY[] {
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
  const out: XY[] = [[px + h * rx, py + h * ry]];
  if (h > 1e-12) out.push([px - h * rx, py - h * ry]);
  return out;
}

/**
 * All intersection points of two full curves — an ARC contributes its whole
 * circle here, which is exactly what an extend's unbounded continuation needs
 * (extents are applied afterwards, per-caller). Exported for `extendMath`.
 */
export function fullIntersections(t: Curve, o: Curve): XY[] {
  const circT = t.kind !== "line" ? t : null;
  const circO = o.kind !== "line" ? o : null;
  if (t.kind === "line" && o.kind === "line") return lineLineFull(t.a, t.b, o.a, o.b);
  if (t.kind === "line" && circO) return lineCircleFull(t.a, t.b, circO.c, circO.r);
  if (circT && o.kind === "line") return lineCircleFull(o.a, o.b, circT.c, circT.r);
  if (circT && circO) return circleCircleFull(circT.c, circT.r, circO.c, circO.r);
  return [];
}

// ── Extent gating ─────────────────────────────────────────────────────────────

/** True if `p` lies within the OTHER curve's bounded extent (endpoints inclusive).
 *  Exported: Extend gates its candidate crossings by the SAME rule — a segment
 *  you cannot see is not something you can extend TO. */
export function withinExtent(curve: Curve, p: XY): boolean {
  if (curve.kind === "line") {
    const dx = curve.b[0] - curve.a[0];
    const dy = curve.b[1] - curve.a[1];
    const len2 = dx * dx + dy * dy;
    if (len2 < 1e-18) return false;
    const u = ((p[0] - curve.a[0]) * dx + (p[1] - curve.a[1]) * dy) / len2;
    return u >= -1e-9 && u <= 1 + 1e-9;
  }
  if (curve.kind === "circle") return true;
  // arc: within the CCW sweep.
  const sweep = arcSweep(curve.c, curve.start, curve.end);
  let rel = angleOf(curve.c, p) - angleOf(curve.c, curve.start);
  if (rel < 0) rel += TWO_PI;
  return rel <= sweep + 1e-9;
}

/**
 * Parameter of `p` on the TARGET curve (line t, arc sweep-relative s, circle
 * absolute angle θ), or null if outside the target's extent. Line/arc allow a
 * small over-run so endpoint crossings are captured then excluded by the caller.
 */
function targetParam(curve: Curve, p: XY): number | null {
  if (curve.kind === "line") {
    const dx = curve.b[0] - curve.a[0];
    const dy = curve.b[1] - curve.a[1];
    const len2 = dx * dx + dy * dy;
    if (len2 < 1e-18) return null;
    const t = ((p[0] - curve.a[0]) * dx + (p[1] - curve.a[1]) * dy) / len2;
    return t >= -1e-9 && t <= 1 + 1e-9 ? t : null;
  }
  if (curve.kind === "circle") return angleOf(curve.c, p);
  const sweep = arcSweep(curve.c, curve.start, curve.end);
  let rel = angleOf(curve.c, p) - angleOf(curve.c, curve.start);
  if (rel < 0) rel += TWO_PI;
  const s = rel / sweep;
  return s >= -1e-9 && s <= 1 + 1e-9 ? s : null;
}

// ── Crossing collection ───────────────────────────────────────────────────────

/**
 * All target params where `target` crosses any `others`, gated to both extents,
 * deduped (ε), with target-endpoint crossings excluded (line/arc only). Circle
 * params are angles in [0,2π); line/arc params are in [0,1].
 */
function crossings(targetCurve: Curve, others: SketchEntity[]): number[] {
  const params: number[] = [];
  for (const o of others) {
    const oc = curveOf(o);
    if (!oc) continue;
    for (const p of fullIntersections(targetCurve, oc)) {
      const tp = targetParam(targetCurve, p);
      if (tp === null) continue;
      if (!withinExtent(oc, p)) continue;
      params.push(tp);
    }
  }
  if (targetCurve.kind === "circle") return dedupeRing(params);
  return dedupeLinear(params).filter((t) => t > EPS && t < 1 - EPS);
}

/** Sort + collapse params within ε (line/arc domain). */
function dedupeLinear(params: number[]): number[] {
  const sorted = [...params].sort((a, b) => a - b);
  const out: number[] = [];
  for (const p of sorted) {
    if (out.length === 0 || Math.abs(p - out[out.length - 1]) > EPS) out.push(p);
  }
  return out;
}

/** Sort + collapse angle params within ε, honoring the 2π wraparound. */
function dedupeRing(angles: number[]): number[] {
  const norm = angles.map((a) => ((a % TWO_PI) + TWO_PI) % TWO_PI).sort((a, b) => a - b);
  const out: number[] = [];
  for (const a of norm) {
    if (out.length === 0 || Math.abs(a - out[out.length - 1]) > EPS) out.push(a);
  }
  // Collapse a near-2π duplicate of the first angle (wraparound).
  if (out.length >= 2 && Math.abs(out[0] + TWO_PI - out[out.length - 1]) < EPS) out.pop();
  return out;
}

// ── Piece / doomed builders ───────────────────────────────────────────────────

function subLine(a: XY, b: XY, ta: number, tb: number, construction?: boolean): DraftEntity {
  const pt = (t: number): Point2 => ({ x: a[0] + t * (b[0] - a[0]), y: a[1] + t * (b[1] - a[1]) });
  return { type: "Line", ...(construction ? { construction: true } : {}), p0: pt(ta), p1: pt(tb) };
}

function subArc(c: XY, r: number, angA: number, angB: number, construction?: boolean): DraftEntity {
  const p = (ang: number): Point2 => ({ x: c[0] + r * Math.cos(ang), y: c[1] + r * Math.sin(ang) });
  return {
    type: "Arc",
    ...(construction ? { construction: true } : {}),
    center: { x: c[0], y: c[1] },
    radius: r,
    start: p(angA),
    end: p(angB),
  };
}

// ── Core ──────────────────────────────────────────────────────────────────────

export interface TrimResult {
  /** The removed span, as ONE draft (for the destructive hover ghost). */
  doomed: DraftEntity;
  /** The surviving pieces (≤2 for line/arc, ≤1 for circle). */
  pieces: DraftEntity[];
}

/**
 * Compute the trim of `target` at `clickPt` against `others`. Returns the removed
 * span + surviving pieces, or null when there is nothing to trim (no qualifying
 * crossings, or a circle with <2 crossings) — the caller then whole-deletes.
 */
export function trimPieces(
  target: SketchEntity,
  clickPt: Point2,
  others: SketchEntity[],
  opts: { minSize: number },
): TrimResult | null {
  const curve = curveOf(target);
  if (!curve) return null;
  const cons = target.construction;
  const cross = crossings(curve, others);
  const clickXY: XY = [clickPt.x, clickPt.y];
  const minSize = opts.minSize;

  if (curve.kind === "line") {
    if (cross.length === 0) return null;
    const pClick = clampParam(targetParam(curve, projectOnLine(curve.a, curve.b, clickXY)) ?? 0);
    const [a, b] = removalInterval(cross, pClick);
    const L = Math.hypot(curve.b[0] - curve.a[0], curve.b[1] - curve.a[1]);
    const leftKept = a > EPS && a * L >= minSize;
    const rightKept = b < 1 - EPS && (1 - b) * L >= minSize;
    const pieces: DraftEntity[] = [];
    if (leftKept) pieces.push(subLine(curve.a, curve.b, 0, a, cons));
    if (rightKept) pieces.push(subLine(curve.a, curve.b, b, 1, cons));
    const doomed = subLine(curve.a, curve.b, leftKept ? a : 0, rightKept ? b : 1, cons);
    return { doomed, pieces };
  }

  if (curve.kind === "arc") {
    if (cross.length === 0) return null;
    const a0 = angleOf(curve.c, curve.start);
    const sweep = arcSweep(curve.c, curve.start, curve.end);
    const angleAt = (s: number): number => a0 + s * sweep;
    const sClick = clampParam(targetParam(curve, projectOnCircle(curve.c, curve.r, clickXY)) ?? 0);
    const [a, b] = removalInterval(cross, sClick);
    const leftKept = a > EPS && a * sweep * curve.r >= minSize;
    const rightKept = b < 1 - EPS && (1 - b) * sweep * curve.r >= minSize;
    const pieces: DraftEntity[] = [];
    if (leftKept) pieces.push(subArc(curve.c, curve.r, angleAt(0), angleAt(a), cons));
    if (rightKept) pieces.push(subArc(curve.c, curve.r, angleAt(b), angleAt(1), cons));
    const doomed = subArc(curve.c, curve.r, angleAt(leftKept ? a : 0), angleAt(rightKept ? b : 1), cons);
    return { doomed, pieces };
  }

  // circle: needs ≥2 distinct crossings.
  if (cross.length < 2) return null;
  const clickAng = angleOf(curve.c, projectOnCircle(curve.c, curve.r, clickXY));
  const [lo, hi] = ringRemovalInterval(cross, clickAng); // removed span [lo, hi] CCW
  const removedSpan = hi - lo; // in (0, 2π)
  const remainingLen = curve.r * (TWO_PI - removedSpan);
  const doomed = subArc(curve.c, curve.r, lo, hi, cons); // the removed arc
  if (remainingLen < minSize) {
    // The complement is too small to survive — the whole circle is doomed.
    return { doomed: { type: "Circle", ...(cons ? { construction: true } : {}), center: { x: curve.c[0], y: curve.c[1] }, radius: curve.r }, pieces: [] };
  }
  // Remaining arc is the complement: CCW from the removed end back to its start.
  const remaining = subArc(curve.c, curve.r, hi, lo + TWO_PI, cons);
  return { doomed, pieces: [remaining] };
}

/**
 * Hover-preview variant: the doomed draft for the piece under `hoverPt`, or null
 * when the whole entity is doomed (no qualifying crossings ⇒ whole-entity delete).
 */
export function trimPreview(
  target: SketchEntity,
  hoverPt: Point2,
  others: SketchEntity[],
  opts: { minSize: number },
): DraftEntity | null {
  return trimPieces(target, hoverPt, others, opts)?.doomed ?? null;
}

/** A whole SketchEntity as a DraftEntity (the whole-entity doomed ghost). */
export function entityToDraft(e: SketchEntity): DraftEntity {
  const pt = (p?: XY): Point2 | undefined => (p ? { x: p[0], y: p[1] } : undefined);
  return {
    type: e.type,
    ...(e.construction ? { construction: true } : {}),
    ...(e.p0 ? { p0: pt(e.p0) } : {}),
    ...(e.p1 ? { p1: pt(e.p1) } : {}),
    ...(e.center ? { center: pt(e.center) } : {}),
    ...(e.radius !== undefined ? { radius: e.radius } : {}),
    ...(e.start ? { start: pt(e.start) } : {}),
    ...(e.end ? { end: pt(e.end) } : {}),
    ...(e.majorR !== undefined ? { majorR: e.majorR } : {}),
    ...(e.minorR !== undefined ? { minorR: e.minorR } : {}),
    ...(e.rotation !== undefined ? { rotation: e.rotation } : {}),
  };
}

// ── Interval location ─────────────────────────────────────────────────────────

const clampParam = (t: number): number => Math.max(0, Math.min(1, t));

/** Removal interval [a,b] on [0,1] whose bounds are the crossings straddling `p`. */
function removalInterval(cross: number[], p: number): [number, number] {
  const bounds = [0, ...cross, 1];
  for (let i = 0; i < bounds.length - 1; i++) {
    // Right boundary strict so a click exactly on a crossing falls to its right.
    if (p < bounds[i + 1] || i === bounds.length - 2) return [bounds[i], bounds[i + 1]];
  }
  return [bounds[0], bounds[1]];
}

/** Removed ring interval [lo,hi] (hi may exceed 2π) containing angle `p`. */
function ringRemovalInterval(cross: number[], p: number): [number, number] {
  const n = cross.length;
  const pn = ((p % TWO_PI) + TWO_PI) % TWO_PI;
  for (let i = 0; i < n; i++) {
    const lo = cross[i];
    const hi = i + 1 < n ? cross[i + 1] : cross[0] + TWO_PI;
    // Test both p and p+2π so a click in the wrapping interval is caught.
    if ((pn >= lo && pn < hi) || (pn + TWO_PI >= lo && pn + TWO_PI < hi)) return [lo, hi];
  }
  // Fallback: the wrapping interval.
  return [cross[n - 1], cross[0] + TWO_PI];
}

/** Nearest point on the infinite line through a,b. */
function projectOnLine(a: XY, b: XY, p: XY): XY {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const len2 = dx * dx + dy * dy;
  if (len2 < 1e-18) return [a[0], a[1]];
  const t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2;
  return [a[0] + t * dx, a[1] + t * dy];
}

/** Nearest point on the full circle centered at c. */
function projectOnCircle(c: XY, r: number, p: XY): XY {
  const dx = p[0] - c[0];
  const dy = p[1] - c[1];
  const d = Math.hypot(dx, dy);
  if (d < 1e-12) return [c[0] + r, c[1]];
  return [c[0] + (r * dx) / d, c[1] + (r * dy) / d];
}
