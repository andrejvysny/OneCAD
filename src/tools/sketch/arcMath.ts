/*
 * arcMath — PURE arc-construction geometry (SP-4).
 *
 * Two constructions plus the small primitives they share:
 *   - `arcThroughPoints` — the arc through THREE points (the `arc3p` tool).
 *   - `tangentArcTo`     — the arc that leaves an anchor along a known tangent
 *                          and lands on the cursor (the line tool's arc mode).
 *
 * CONVENTION: every arc returned here runs CCW from `start` to `end` — what
 * `DraftEntity` / `SketchEntity` mean by those two fields, and what the worker
 * reconstructs. A construction whose natural travel direction is CW therefore
 * SWAPS its endpoints rather than emitting a reversed arc: the same geometry,
 * stated the one way the wire can express it. `sweep` is the CCW angle from
 * `start` to `end`, in (0, 2π).
 *
 * Every entry point returns `null` on a degenerate input (collinear points, a
 * zero-length direction, a cursor on the tangent ray) instead of guessing — a
 * draw tool that cannot form an arc must preview nothing and commit nothing.
 *
 * PURE, and imports only the two TYPES it speaks, so `toolMachine`,
 * `liveDimFrames` and the controller can all consume it with no import cycle.
 */
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";

/** Below this a determinant or a length carries no direction left to use. */
const DEGENERATE = 1e-12;
const TWO_PI = Math.PI * 2;

/** Fold an angle difference into [0, 2π) — the domain every sweep is stated in. */
const norm2Pi = (a: number): number => ((a % TWO_PI) + TWO_PI) % TWO_PI;

/** CCW quarter turn. `rot90(t)` is the normal a tangent-arc centre sits along. */
const rot90 = (p: Point2): Point2 => ({ x: -p.y, y: p.x });

const negate = (p: Point2): Point2 => ({ x: -p.x, y: -p.y });

const angleAt = (ctr: Point2, p: Point2): number => Math.atan2(p.y - ctr.y, p.x - ctr.x);

/** Unit vector, or null when there is no direction to normalize. */
function unitOf(v: Point2): Point2 | null {
  const len = Math.hypot(v.x, v.y);
  if (len < DEGENERATE) return null;
  return { x: v.x / len, y: v.y / len };
}

/** Unit `from` → `to`, or null when the two points coincide. */
export function unitDir(from: Point2, to: Point2): Point2 | null {
  return unitOf({ x: to.x - from.x, y: to.y - from.y });
}

/** Perpendicular distance from `p` to the infinite line a→b (0 when degenerate). */
export function perpDistanceTo(a: Point2, b: Point2, p: Point2): number {
  const d = unitDir(a, b);
  if (!d) return 0;
  const n = rot90(d);
  return Math.abs((p.x - a.x) * n.x + (p.y - a.y) * n.y);
}

/**
 * Centre of the circle through three points, or null when they are collinear.
 *
 * Solved in `a`-relative coordinates (b' = b−a, c' = c−a) so the cancellation
 * happens on the small offsets rather than on the absolute plane coordinates.
 */
export function circumcenter(a: Point2, b: Point2, c: Point2): Point2 | null {
  const bx = b.x - a.x;
  const by = b.y - a.y;
  const cx = c.x - a.x;
  const cy = c.y - a.y;
  const d = 2 * (bx * cy - by * cx);
  if (Math.abs(d) < DEGENERATE) return null;
  const b2 = bx * bx + by * by;
  const c2 = cx * cx + cy * cy;
  return { x: a.x + (cy * b2 - by * c2) / d, y: a.y + (bx * c2 - cx * b2) / d };
}

/** A CCW arc: `sweep` is the angle from `start` to `end`, in (0, 2π). */
export interface ArcThrough {
  center: Point2;
  radius: number;
  start: Point2;
  end: Point2;
  sweep: number;
}

/**
 * The arc through `start` and `end` that also passes through `through`.
 *
 * THE SWAP RULE. The circle is symmetric in the three points, but an arc is not:
 * of the two arcs the chord cuts, the one the user drew is whichever CONTAINS
 * the through point. Measuring every angle relative to `start`, the through
 * point lies inside the CCW sweep `start`→`end` exactly when its own relative
 * angle is no larger — otherwise the drawn arc is the complement, which in the
 * CCW convention is the SAME arc with its endpoints exchanged.
 */
export function arcThroughPoints(start: Point2, end: Point2, through: Point2): ArcThrough | null {
  const center = circumcenter(start, end, through);
  if (!center) return null;
  const radius = Math.hypot(start.x - center.x, start.y - center.y);
  const a0 = angleAt(center, start);
  const sweepCCW = norm2Pi(angleAt(center, end) - a0);
  const relM = norm2Pi(angleAt(center, through) - a0);
  if (relM <= sweepCCW) return { center, radius, start, end, sweep: sweepCCW };
  return { center, radius, start: end, end: start, sweep: TWO_PI - sweepCCW };
}

/** A tangent arc plus the two facts a CHAIN needs: which end the anchor is, and
 *  the direction the chain leaves the far end travelling in. */
export interface TangentArc {
  center: Point2;
  radius: number;
  start: Point2;
  end: Point2;
  /** True when the arc runs CCW, i.e. `start` IS the anchor (else it is `end`). */
  startsAtAnchor: boolean;
  /** Unit travel direction at the CURSOR end — the next segment's tangent. */
  exitTangent: Point2;
  sweep: number;
}

/**
 * The unique arc that leaves `anchor` along `tangent` and passes through
 * `cursor`; null when the cursor sits on the tangent line (no finite circle).
 *
 * The centre lies on the normal `n = rot90(tangent)`, at `C = anchor + s·n` with
 * `s` fixed by |C − cursor| = |s|: with `d = cursor − anchor`, |d|² − 2s(d·n) = 0.
 *
 * WHY `s > 0` MEANS CCW: a CCW arc's velocity at P is ẑ×(P − C). At the anchor
 * P − C = −s·n, and ẑ×n = −t, so the velocity is ∝ s·t — it agrees with the
 * given tangent exactly when s is positive. A negative s is the same circle
 * travelled the other way, so the arc is stated with its endpoints exchanged and
 * the exit direction negated.
 */
export function tangentArcTo(anchor: Point2, tangent: Point2, cursor: Point2): TangentArc | null {
  // Normalized defensively: `radius = |s|` only reads as a length when `n` is a
  // unit normal, and callers hand in whatever direction they happen to hold.
  const t = unitOf(tangent);
  if (!t) return null;
  const n = rot90(t);
  const dx = cursor.x - anchor.x;
  const dy = cursor.y - anchor.y;
  const denom = 2 * (dx * n.x + dy * n.y);
  if (Math.abs(denom) < DEGENERATE) return null;
  const s = (dx * dx + dy * dy) / denom;
  const center = { x: anchor.x + s * n.x, y: anchor.y + s * n.y };
  const radius = Math.abs(s);
  const outward = unitDir(center, cursor);
  if (!outward) return null;
  const ccw = s > 0;
  const start = ccw ? anchor : cursor;
  const end = ccw ? cursor : anchor;
  const turn = rot90(outward);
  return {
    center,
    radius,
    start,
    end,
    startsAtAnchor: ccw,
    exitTangent: ccw ? turn : negate(turn),
    sweep: norm2Pi(angleAt(center, end) - angleAt(center, start)),
  };
}

const toPoint = (p: [number, number]): Point2 => ({ x: p[0], y: p[1] });

const within = (a: Point2, b: Point2, tol: number): boolean => Math.hypot(a.x - b.x, a.y - b.y) <= tol;

const mapNull = (p: Point2 | null, f: (p: Point2) => Point2): Point2 | null => (p ? f(p) : null);

/**
 * Outgoing tangent(s) of ONE entity at `at` — the direction a chain continues in
 * when it leaves that endpoint, which is why a match on the START runs backwards
 * along the entity. A closed curve (circle, ellipse) has no endpoint to leave
 * from and contributes nothing.
 */
function endTangentsOf(e: SketchEntity, at: Point2, tol: number): Point2[] {
  const out: Point2[] = [];
  const push = (d: Point2 | null): void => {
    if (d) out.push(d);
  };
  if (e.type === "Line" && e.p0 && e.p1) {
    const p0 = toPoint(e.p0);
    const p1 = toPoint(e.p1);
    if (within(at, p1, tol)) push(unitDir(p0, p1));
    if (within(at, p0, tol)) push(unitDir(p1, p0));
  } else if (e.type === "Arc" && e.center && e.start && e.end) {
    const c = toPoint(e.center);
    const start = toPoint(e.start);
    const end = toPoint(e.end);
    // The arc runs CCW, so its velocity is ẑ×(P−C) at the end and the reverse of
    // that at the start (leaving the arc means travelling back out of it).
    if (within(at, end, tol)) push(mapNull(unitDir(c, end), rot90));
    if (within(at, start, tol)) push(mapNull(unitDir(c, start), (d) => negate(rot90(d))));
  }
  return out;
}

/**
 * The single entity tangent at `at`, or null when NO entity ends there — or more
 * than one does. A chain seeded from an ambiguous junction would pick an
 * arbitrary neighbour's direction, so it refuses instead (the `NeedsRepair`
 * discipline, one dimension down).
 */
export function entityEndTangent(at: Point2, entities: SketchEntity[], tol: number): Point2 | null {
  let found: Point2 | null = null;
  for (const e of entities) {
    for (const d of endTangentsOf(e, at, tol)) {
      if (found) return null;
      found = d;
    }
  }
  return found;
}
