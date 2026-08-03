/*
 * offsetMath — PURE parallel-offset geometry for the sketch Offset tool
 * (WP-C T2b). Picking any Line/Arc walks the CHAIN it belongs to and authors a
 * parallel copy at signed distance `d` on the side the cursor is on.
 *
 * ── Chain walk ───────────────────────────────────────────────────────────────
 * Entities are joined by shared endpoints (Line → p0/p1, Arc → start/end). The
 * walk runs forward from the seed's exit, then backward from its entry, and
 * STOPS at any vertex that is not degree-2: a free end (0 candidates) ends the
 * chain, a branch (≥2 candidates) is never guessed at. Each member carries a
 * `reversed` flag — the chain is a DIRECTED path, and every sign below is taken
 * relative to that traversal. A closed loop is detected when the forward walk
 * arrives back at the seed's entry point.
 *
 * A CIRCLE is its own single-member closed chain (concentric offset, no joins).
 * An ELLIPSE is refused: its true offset is not an ellipse, so there is no
 * honest entity to author (documented V1 limit, same spirit as trimMath's).
 *
 * ── Per-entity offset ────────────────────────────────────────────────────────
 * `+d` is always the LEFT of travel.
 *   Line  : both endpoints translate by d·n̂, n̂ = rot90(direction).
 *   Arc   : concentric. For a CCW-traversed arc the left normal points INWARD
 *           (tangent (−sinθ,cosθ) rotated +90° is −(cosθ,sinθ)), so r′ = r − d;
 *           for a CW-traversed arc r′ = r + d. `r′ ≤ minSize` is REFUSED, naming
 *           the largest distance the chain admits.
 *
 * ── Joins ────────────────────────────────────────────────────────────────────
 * Consecutive offset curves are welded at the intersection of their FULL
 * underlying curves nearest the midpoint of the two raw offset endpoints —
 * which is exactly "extend/trim to the corner" for line↔line, and a no-op for a
 * tangent line↔arc join (the single tangency IS the shared point). With no
 * intersection at all (a parallel pair, or an offset that overshot) both
 * endpoints collapse onto that midpoint instead, so the chain never opens a gap
 * that would break region detection. Arc endpoints are re-projected onto their
 * own offset circle afterwards, so a fallback weld can never drift off-radius.
 * Open chains stay open (square ends); closed loops also join last→first.
 *
 * The result is a plain chain of ordinary Line/Arc drafts: the offset authors NO
 * persistent link back to its source (V1 — an "offset" association would need a
 * constraint kind the solver does not have; see sketchService.offsetSketchChain).
 *
 * Pure ⇒ every edge is unit-tested by data (offsetMath.test.ts).
 */
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import type { DraftEntity } from "./toolMachine";
import { circleCircleFull, lineCircleFull, lineLineFull, type XY } from "./trimMath";

const TWO_PI = Math.PI * 2;

// ── Chain model ───────────────────────────────────────────────────────────────

/** One entity in a directed chain. `reversed` ⇒ traversed end→start. */
export interface ChainMember {
  entity: SketchEntity;
  reversed: boolean;
}

export interface Chain {
  members: ChainMember[];
  closed: boolean;
}

/** A directed curve in traversal order (`a` → `b`). */
type DirCurve =
  | { kind: "line"; a: XY; b: XY }
  | { kind: "arc"; c: XY; r: number; a: XY; b: XY; ccw: boolean }
  | { kind: "circle"; c: XY; r: number };

const dist = (a: XY, b: XY): number => Math.hypot(a[0] - b[0], a[1] - b[1]);
const mid = (a: XY, b: XY): XY => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
const toPoint = (p: XY): Point2 => ({ x: p[0], y: p[1] });

/** A chain-walkable entity's two endpoints, in its own declared order. */
function endpointsOf(e: SketchEntity): [XY, XY] | null {
  if (e.type === "Line" && e.p0 && e.p1) return [e.p0, e.p1];
  if (e.type === "Arc" && e.start && e.end) return [e.start, e.end];
  return null;
}

/** The unused entity whose endpoint meets `at`, or null (free end OR branch). */
function nextAt(
  at: XY,
  pool: SketchEntity[],
  used: Set<string>,
  tol: number,
): { entity: SketchEntity; reversed: boolean; exit: XY } | null {
  let found: { entity: SketchEntity; reversed: boolean; exit: XY } | null = null;
  for (const e of pool) {
    if (used.has(e.id)) continue;
    const ends = endpointsOf(e);
    if (!ends) continue;
    const hitsStart = dist(ends[0], at) <= tol;
    const hitsEnd = dist(ends[1], at) <= tol;
    if (!hitsStart && !hitsEnd) continue;
    if (found) return null; // branch vertex — never guess which way the chain runs
    found = hitsStart
      ? { entity: e, reversed: false, exit: ends[1] }
      : { entity: e, reversed: true, exit: ends[0] };
  }
  return found;
}

/**
 * The directed chain containing `seed`. Returns null when the seed is not an
 * offsettable curve (Point / Ellipse). Only entities matching the seed's
 * `construction` flag are walked — a construction centreline must never drag a
 * real edge into the chain.
 */
export function buildChain(seed: SketchEntity, all: SketchEntity[], tol: number): Chain | null {
  if (seed.type === "Circle") {
    if (!seed.center || seed.radius === undefined) return null;
    return { members: [{ entity: seed, reversed: false }], closed: true };
  }
  const seedEnds = endpointsOf(seed);
  if (!seedEnds) return null;

  const pool = all.filter(
    (e) => e.id !== seed.id && endpointsOf(e) !== null && !!e.construction === !!seed.construction,
  );
  const used = new Set<string>([seed.id]);
  const members: ChainMember[] = [{ entity: seed, reversed: false }];

  let cursor = seedEnds[1];
  let closed = false;
  for (;;) {
    const nxt = nextAt(cursor, pool, used, tol);
    if (!nxt) break;
    used.add(nxt.entity.id);
    members.push({ entity: nxt.entity, reversed: nxt.reversed });
    cursor = nxt.exit;
    if (dist(cursor, seedEnds[0]) <= tol) {
      closed = true;
      break;
    }
  }
  if (!closed) {
    let back = seedEnds[0];
    for (;;) {
      const nxt = nextAt(back, pool, used, tol);
      if (!nxt) break;
      used.add(nxt.entity.id);
      // The predecessor must EXIT at `back`, so its traversal is the opposite of
      // the one `nextAt` reports for an entry there.
      members.unshift({ entity: nxt.entity, reversed: !nxt.reversed });
      back = nxt.exit;
    }
  }
  return { members, closed };
}

// ── Directed curves ───────────────────────────────────────────────────────────

function directed(m: ChainMember): DirCurve | null {
  const e = m.entity;
  if (e.type === "Line" && e.p0 && e.p1) {
    return m.reversed ? { kind: "line", a: e.p1, b: e.p0 } : { kind: "line", a: e.p0, b: e.p1 };
  }
  if (e.type === "Arc" && e.start && e.end && e.center && e.radius !== undefined) {
    return m.reversed
      ? { kind: "arc", c: e.center, r: e.radius, a: e.end, b: e.start, ccw: false }
      : { kind: "arc", c: e.center, r: e.radius, a: e.start, b: e.end, ccw: true };
  }
  if (e.type === "Circle" && e.center && e.radius !== undefined) {
    return { kind: "circle", c: e.center, r: e.radius };
  }
  return null;
}

/** Unit LEFT normal of a directed line (rot90 of its direction), or null. */
function leftNormal(a: XY, b: XY): XY | null {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const l = Math.hypot(dx, dy);
  if (l < 1e-12) return null;
  return [-dy / l, dx / l];
}

/** `+1` when `p` lies to the LEFT of the directed curve, else `-1`. */
function leftSideSign(cur: DirCurve, p: XY): 1 | -1 {
  if (cur.kind === "line") {
    const n = leftNormal(cur.a, cur.b);
    if (!n) return 1;
    return (p[0] - cur.a[0]) * n[0] + (p[1] - cur.a[1]) * n[1] >= 0 ? 1 : -1;
  }
  // A CCW-traversed arc/circle has its left side INSIDE (see the module header).
  const inside = dist(p, cur.c) < cur.r;
  const ccw = cur.kind === "circle" ? true : cur.ccw;
  if (ccw) return inside ? 1 : -1;
  return inside ? -1 : 1;
}

/** Rough distance from `p` to a directed curve (curve, not extent — the caller
 *  has already hit-tested onto the chain; this only ranks members). */
function roughDistance(cur: DirCurve, p: XY): number {
  if (cur.kind !== "line") return Math.abs(dist(p, cur.c) - cur.r);
  const dx = cur.b[0] - cur.a[0];
  const dy = cur.b[1] - cur.a[1];
  const len2 = dx * dx + dy * dy;
  if (len2 < 1e-18) return dist(p, cur.a);
  let t = ((p[0] - cur.a[0]) * dx + (p[1] - cur.a[1]) * dy) / len2;
  t = Math.max(0, Math.min(1, t));
  return dist(p, [cur.a[0] + t * dx, cur.a[1] + t * dy]);
}

/**
 * `+1` / `-1`: which side of the chain `cursor` is on, in TRAVERSAL terms.
 * Multiply the user's (positive) distance by this to get the signed offset, so
 * the ghost always follows the pointer.
 */
export function chainSideSign(chain: Chain, cursor: Point2): 1 | -1 {
  const p: XY = [cursor.x, cursor.y];
  let best: DirCurve | null = null;
  let bestD = Infinity;
  for (const m of chain.members) {
    const cur = directed(m);
    if (!cur) continue;
    const d = roughDistance(cur, p);
    if (d < bestD) {
      bestD = d;
      best = cur;
    }
  }
  return best ? leftSideSign(best, p) : 1;
}

// ── Offset ────────────────────────────────────────────────────────────────────

export type OffsetFailure =
  | { ok: false; reason: "unsupported" }
  | { ok: false; reason: "badDistance" }
  | { ok: false; reason: "arcCollapsed"; maxDistance: number }
  | { ok: false; reason: "segmentVanished" };

export type OffsetResult = { ok: true; entities: DraftEntity[] } | OffsetFailure;

/** How much an arc's radius SHRINKS under a signed left-offset of `d`. */
const shrinkOf = (cur: { ccw: boolean }, d: number): number => (cur.ccw ? d : -d);

function offsetOne(cur: DirCurve, d: number): DirCurve | null {
  if (cur.kind === "line") {
    const n = leftNormal(cur.a, cur.b);
    if (!n) return null;
    return {
      kind: "line",
      a: [cur.a[0] + n[0] * d, cur.a[1] + n[1] * d],
      b: [cur.b[0] + n[0] * d, cur.b[1] + n[1] * d],
    };
  }
  if (cur.kind === "circle") {
    return { kind: "circle", c: cur.c, r: cur.r - d };
  }
  const r = cur.r - shrinkOf(cur, d);
  const scale = (p: XY): XY => [cur.c[0] + ((p[0] - cur.c[0]) * r) / cur.r, cur.c[1] + ((p[1] - cur.c[1]) * r) / cur.r];
  return { kind: "arc", c: cur.c, r, a: scale(cur.a), b: scale(cur.b), ccw: cur.ccw };
}

/** All intersections of two FULL offset curves (extent-free — joins extend). */
function fullIntersections(p: DirCurve, q: DirCurve): XY[] {
  const circOf = (c: DirCurve): { c: XY; r: number } | null =>
    c.kind === "line" ? null : { c: c.c, r: c.r };
  const cp = circOf(p);
  const cq = circOf(q);
  if (p.kind === "line" && q.kind === "line") return lineLineFull(p.a, p.b, q.a, q.b);
  if (p.kind === "line" && cq) return lineCircleFull(p.a, p.b, cq.c, cq.r);
  if (cp && q.kind === "line") return lineCircleFull(q.a, q.b, cp.c, cp.r);
  if (cp && cq) return circleCircleFull(cp.c, cp.r, cq.c, cq.r);
  return [];
}

/** Weld `prev`'s exit to `next`'s entry (extend/trim to their intersection). */
function joinPair(prev: DirCurve, next: DirCurve): void {
  if (prev.kind === "circle" || next.kind === "circle") return; // never chained
  const target = mid(prev.b, next.a);
  const candidates = fullIntersections(prev, next);
  let weld = target;
  let bestD = Infinity;
  for (const p of candidates) {
    const d = dist(p, target);
    if (d < bestD) {
      bestD = d;
      weld = p;
    }
  }
  prev.b = onOwnCurve(prev, weld);
  next.a = onOwnCurve(next, weld);
}

/** An arc endpoint must stay ON its own offset circle — the fallback weld (used
 *  when the two curves do not meet at all) is a midpoint that generally is not. */
function onOwnCurve(cur: DirCurve, p: XY): XY {
  if (cur.kind !== "arc") return p;
  const dx = p[0] - cur.c[0];
  const dy = p[1] - cur.c[1];
  const l = Math.hypot(dx, dy);
  if (l < 1e-12) return [cur.c[0] + cur.r, cur.c[1]];
  return [cur.c[0] + (dx / l) * cur.r, cur.c[1] + (dy / l) * cur.r];
}

/** CCW sweep magnitude from `a` to `b` about `c`, in (0, 2π]. */
function sweepOf(c: XY, r: number, a: XY, b: XY, ccw: boolean): number {
  const ang = (p: XY): number => {
    const t = Math.atan2(p[1] - c[1], p[0] - c[0]);
    return t < 0 ? t + TWO_PI : t;
  };
  let s = ccw ? ang(b) - ang(a) : ang(a) - ang(b);
  if (s < 0) s += TWO_PI;
  return r > 0 && s < 1e-9 ? TWO_PI : s;
}

/**
 * Offset `chain` by the SIGNED distance `d` (`+` = left of travel) into a plain
 * chain of Line/Arc/Circle drafts, or a loud refusal.
 *
 * Output order matches the chain's traversal order (which is NOT necessarily the
 * session order). Drafts are REGULAR geometry: the construction flag is
 * deliberately not inherited — an offset is new user geometry, and a
 * construction centreline's offset is normally a real edge (the user can flip it
 * afterwards with X). `minSize` is the caller's screen-scaled degeneracy floor.
 */
export function offsetChain(chain: Chain, d: number, opts: { minSize: number }): OffsetResult {
  if (!Number.isFinite(d) || Math.abs(d) < 1e-12) return { ok: false, reason: "badDistance" };
  const curves: DirCurve[] = [];
  for (const m of chain.members) {
    const cur = directed(m);
    if (!cur) return { ok: false, reason: "unsupported" };
    curves.push(cur);
  }
  if (curves.length === 0) return { ok: false, reason: "unsupported" };

  const raw = offsetCurves(curves, d, opts.minSize);
  if ("reason" in raw) return raw;
  const offset = raw.offset;

  const last = offset.length - 1;
  for (let i = 0; i < last; i++) joinPair(offset[i], offset[i + 1]);
  if (chain.closed && offset.length > 1) joinPair(offset[last], offset[0]);

  return emitDrafts(curves, offset, opts.minSize);
}

/** Per-curve offset with the radius-collapse refusal (before any welding). */
function offsetCurves(
  curves: DirCurve[],
  d: number,
  minSize: number,
): { offset: DirCurve[] } | OffsetFailure {
  // The largest usable distance is the smallest radius among the arcs THIS
  // offset shrinks — reported so the refusal can name it.
  let limit = Infinity;
  for (const cur of curves) {
    if (cur.kind === "line") continue;
    const shrink = cur.kind === "circle" ? d : shrinkOf(cur, d);
    if (shrink > 0) limit = Math.min(limit, cur.r);
  }
  const offset: DirCurve[] = [];
  for (const cur of curves) {
    const o = offsetOne(cur, d);
    if (!o) return { ok: false, reason: "segmentVanished" };
    if (o.kind !== "line" && o.r <= minSize) {
      return { ok: false, reason: "arcCollapsed", maxDistance: Number.isFinite(limit) ? limit : 0 };
    }
    offset.push(o);
  }
  return { offset };
}

/**
 * Welded offset curves → drafts, with OVER-OFFSET detection.
 *
 * Past the chain's self-intersection distance a join does not shorten a segment
 * to nothing and stop — it drags the segment's two ends PAST each other, so the
 * offset "succeeds" with an inside-out chain (offset a 100×50 rectangle inward
 * by 30 and you get a valid-looking 40×10 rectangle whose walls run backwards).
 * A length test alone only catches the exact critical distance, so each segment
 * is also checked against the ORIENTATION of the source it came from: a line
 * whose direction reversed, or an arc whose sweep grew by more than half a turn,
 * means the offset folded.
 */
function emitDrafts(curves: DirCurve[], offset: DirCurve[], minSize: number): OffsetResult {
  const drafts: DraftEntity[] = [];
  for (let i = 0; i < offset.length; i++) {
    const o = offset[i];
    const src = curves[i];
    if (o.kind === "circle") {
      drafts.push({ type: "Circle", center: toPoint(o.c), radius: o.r });
      continue;
    }
    if (o.kind === "line" && src.kind === "line") {
      if (dist(o.a, o.b) < minSize) return { ok: false, reason: "segmentVanished" };
      const dot =
        (o.b[0] - o.a[0]) * (src.b[0] - src.a[0]) + (o.b[1] - o.a[1]) * (src.b[1] - src.a[1]);
      if (dot <= 0) return { ok: false, reason: "segmentVanished" };
      drafts.push({ type: "Line", p0: toPoint(o.a), p1: toPoint(o.b) });
      continue;
    }
    if (o.kind !== "arc" || src.kind !== "arc") return { ok: false, reason: "unsupported" };
    const sweep = sweepOf(o.c, o.r, o.a, o.b, o.ccw);
    if (sweep * o.r < minSize) return { ok: false, reason: "segmentVanished" };
    if (sweep > sweepOf(src.c, src.r, src.a, src.b, src.ccw) + Math.PI) {
      return { ok: false, reason: "segmentVanished" };
    }
    drafts.push({
      type: "Arc",
      center: toPoint(o.c),
      radius: o.r,
      // DraftEntity arcs always sweep CCW start→end, so a CW-traversed member
      // authors its endpoints the other way round.
      start: toPoint(o.ccw ? o.a : o.b),
      end: toPoint(o.ccw ? o.b : o.a),
    });
  }
  return { ok: true, entities: drafts };
}
