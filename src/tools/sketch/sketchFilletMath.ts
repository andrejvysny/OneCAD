/*
 * sketchFilletMath — PURE corner-fillet geometry for the sketch Fillet tool
 * (WP-C T2b). Two LINES that share an endpoint are replaced by:
 *   - the same two lines TRIMMED back to their tangent points, and
 *   - a tangent ARC of the requested radius bridging them.
 *
 * ── The construction ─────────────────────────────────────────────────────────
 * Let `P` be the shared corner, `u`/`v` the UNIT directions from `P` toward each
 * line's far (kept) endpoint, and θ = ∠(u,v) ∈ (0, π) the corner angle. Then:
 *
 *   tangent distance   d = r / tan(θ/2)      (along each line, from P)
 *   tangent points     T_a = P + u·d,  T_b = P + v·d
 *   centre             C   = P + ŵ·(r / sin(θ/2)),  ŵ = normalize(u + v)
 *
 * `C` is on the bisector, so its perpendicular distance to EACH line is
 * |C−P|·sin(θ/2) = r, and its foot on each line is at |C−P|·cos(θ/2) = d — i.e.
 * exactly `T_a` / `T_b`. The arc is therefore tangent-continuous with both lines
 * by construction, with sweep π − θ (always < π: the SHORT way round).
 *
 * ── Winding (both CW and CCW corners are pinned) ─────────────────────────────
 * Frontend arcs sweep CCW from `start` to `end` (snapEngine.arcContainsAngle /
 * arcTool). With `cross = u × v`:
 *   cross > 0 (v is CCW of u) ⇒ the short way T_a → T_b is CLOCKWISE, so the arc
 *                               is authored `start = T_b`, `end = T_a`;
 *   cross < 0                 ⇒ `start = T_a`, `end = T_b`.
 * Worked example (u = +x, v = +y, r): cross = +1, T_a = (r,0), T_b = (0,r),
 * C = (r,r); ∠(T_a−C) = −π/2, ∠(T_b−C) = π, and CCW from π to −π/2(≡3π/2) is
 * π/2 = π − θ. ✔
 *
 * ── Refusals (loud, never a guess) ───────────────────────────────────────────
 *   notACorner  — the two lines do not share an endpoint within tolerance
 *   collinear   — θ ≈ 0 or θ ≈ π: there is no corner to round
 *   radiusTooLarge — d ≥ the shorter available leg; carries the exact `maxRadius`
 *   badRadius   — r ≤ 0
 *   degenerate  — a zero-length line
 *
 * Pure ⇒ every edge is unit-tested by data (sketchFilletMath.test.ts). The
 * CONSTRAINT story (why the fillet seeds endpoint welds and NOT entity-level
 * Tangent) lives with the commit verb in sketchService.ts.
 */
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import type { DraftEntity } from "./toolMachine";

export type XY = [number, number];
/** Which endpoint of a Line a fillet consumes (`p0` = Start, `p1` = End). */
export type LineEnd = "Start" | "End";

/** Corner angles this close to 0 or π are collinear — no corner to round. */
const MIN_CORNER_ANGLE = 1e-3;

const sub = (a: XY, b: XY): XY => [a[0] - b[0], a[1] - b[1]];
const norm2 = (v: XY): number => Math.hypot(v[0], v[1]);
const unit = (v: XY): XY => {
  const l = norm2(v);
  return [v[0] / l, v[1] / l];
};
const along = (p: XY, dir: XY, k: number): XY => [p[0] + dir[0] * k, p[1] + dir[1] * k];
const toPoint = (p: XY): Point2 => ({ x: p[0], y: p[1] });
const distance = (a: XY, b: XY): number => Math.hypot(a[0] - b[0], a[1] - b[1]);

/** A line entity's two endpoints keyed by their constraint position, or null. */
export function lineEnds(e: SketchEntity): Record<LineEnd, XY> | null {
  if (e.type !== "Line" || !e.p0 || !e.p1) return null;
  return { Start: e.p0, End: e.p1 };
}

/** The corner two lines share: which end of each meets, and where. */
export interface SharedCorner {
  aEnd: LineEnd;
  bEnd: LineEnd;
  corner: XY;
}

/**
 * The endpoint pair `a` and `b` share, within `tol`, or null. The CLOSEST of the
 * four pairings wins, so a short line whose both ends sit near the other's is
 * still resolved deterministically rather than by declaration order.
 */
export function sharedCorner(a: SketchEntity, b: SketchEntity, tol: number): SharedCorner | null {
  const ea = lineEnds(a);
  const eb = lineEnds(b);
  if (!ea || !eb || a.id === b.id) return null;
  let best: SharedCorner | null = null;
  let bestD = Infinity;
  for (const aEnd of ["Start", "End"] as LineEnd[]) {
    for (const bEnd of ["Start", "End"] as LineEnd[]) {
      const d = distance(ea[aEnd], eb[bEnd]);
      if (d < bestD) {
        bestD = d;
        best = { aEnd, bEnd, corner: ea[aEnd] };
      }
    }
  }
  return bestD <= tol ? best : null;
}

/** The solved fillet: the two trimmed line drafts + the tangent arc. */
export interface FilletGeometry {
  corner: XY;
  center: XY;
  radius: number;
  /** Tangent point on line A / B — the endpoint each line is trimmed back to. */
  tangentA: XY;
  tangentB: XY;
  /** Which end of each SOURCE line the fillet consumed. */
  aEnd: LineEnd;
  bEnd: LineEnd;
  /** Which end of the ARC touches line A / B (its Coincident weld position). */
  arcEndA: LineEnd;
  arcEndB: LineEnd;
  /** Trimmed A, trimmed B, and the bridging arc (commit order). */
  drafts: [DraftEntity, DraftEntity, DraftEntity];
  /** The CCW sweep of the authored arc, in radians (π − θ). */
  sweep: number;
}

export type FilletFailure =
  | { ok: false; reason: "notACorner" }
  | { ok: false; reason: "collinear" }
  | { ok: false; reason: "degenerate" }
  | { ok: false; reason: "badRadius" }
  | { ok: false; reason: "radiusTooLarge"; maxRadius: number };

export type FilletResult = { ok: true; geom: FilletGeometry } | FilletFailure;

/**
 * Solve the fillet of radius `radius` at the corner `a` and `b` share.
 * `tol` is the endpoint-coincidence reach (screen-scaled by the caller).
 */
export function filletGeometry(
  a: SketchEntity,
  b: SketchEntity,
  radius: number,
  opts: { tol: number },
): FilletResult {
  const corner = sharedCorner(a, b, opts.tol);
  if (!corner) return { ok: false, reason: "notACorner" };
  if (!(radius > 0)) return { ok: false, reason: "badRadius" };
  const ea = lineEnds(a)!;
  const eb = lineEnds(b)!;
  const P = corner.corner;
  const farA = ea[corner.aEnd === "Start" ? "End" : "Start"];
  const farB = eb[corner.bEnd === "Start" ? "End" : "Start"];
  const legA = norm2(sub(farA, P));
  const legB = norm2(sub(farB, P));
  if (legA < 1e-9 || legB < 1e-9) return { ok: false, reason: "degenerate" };

  const u = unit(sub(farA, P));
  const v = unit(sub(farB, P));
  const cross = u[0] * v[1] - u[1] * v[0];
  const theta = Math.atan2(Math.abs(cross), u[0] * v[0] + u[1] * v[1]); // [0, π]
  if (theta < MIN_CORNER_ANGLE || theta > Math.PI - MIN_CORNER_ANGLE) {
    return { ok: false, reason: "collinear" };
  }

  const half = theta / 2;
  const tanHalf = Math.tan(half);
  const d = radius / tanHalf;
  const shortestLeg = Math.min(legA, legB);
  if (d >= shortestLeg) {
    return { ok: false, reason: "radiusTooLarge", maxRadius: shortestLeg * tanHalf };
  }

  const tangentA = along(P, u, d);
  const tangentB = along(P, v, d);
  const bisector = unit([u[0] + v[0], u[1] + v[1]]);
  const center = along(P, bisector, radius / Math.sin(half));

  // Winding: cross > 0 ⇒ the SHORT way T_a → T_b runs clockwise, so the CCW arc
  // is authored from T_b to T_a (see the module header's worked example).
  const cw = cross > 0;

  return {
    ok: true,
    geom: {
      corner: P,
      center,
      radius,
      tangentA,
      tangentB,
      aEnd: corner.aEnd,
      bEnd: corner.bEnd,
      arcEndA: cw ? "End" : "Start",
      arcEndB: cw ? "Start" : "End",
      drafts: filletDrafts(a, b, corner, { tangentA, tangentB, center, radius, cw }),
      sweep: Math.PI - theta,
    },
  };
}

/** The trimmed leg A, trimmed leg B, and bridging arc drafts (commit order). */
function filletDrafts(
  a: SketchEntity,
  b: SketchEntity,
  corner: SharedCorner,
  arc: { tangentA: XY; tangentB: XY; center: XY; radius: number; cw: boolean },
): [DraftEntity, DraftEntity, DraftEntity] {
  const trimmed = (src: SketchEntity, end: LineEnd, tangent: XY): DraftEntity => ({
    type: "Line",
    ...(src.construction ? { construction: true as const } : {}),
    p0: toPoint(end === "Start" ? tangent : src.p0!),
    p1: toPoint(end === "End" ? tangent : src.p1!),
  });
  return [
    trimmed(a, corner.aEnd, arc.tangentA),
    trimmed(b, corner.bEnd, arc.tangentB),
    {
      type: "Arc",
      // The arc inherits construction only when BOTH legs are construction —
      // a fillet between a real edge and a construction line is real geometry.
      ...(a.construction && b.construction ? { construction: true as const } : {}),
      center: toPoint(arc.center),
      radius: arc.radius,
      start: toPoint(arc.cw ? arc.tangentB : arc.tangentA),
      end: toPoint(arc.cw ? arc.tangentA : arc.tangentB),
    },
  ];
}

/** A corner candidate: the two lines meeting at `corner`. */
export interface CornerPick {
  a: SketchEntity;
  b: SketchEntity;
  corner: XY;
}

/**
 * The filletable corner nearest `at` within `reach`, or null.
 *
 * The two tolerances are deliberately different. `weldTol` decides whether two
 * lines actually MEET (tight — the construction places the corner at one line's
 * endpoint, so welding a visibly-apart pair would author wrong geometry), while
 * `reach` is how close the CURSOR has to be (loose — a click target). Picking
 * the corner closest to the cursor is what makes one click on the vertex of an
 * L unambiguous. O(n²) over lines only; the caller runs it once per hover frame
 * (rAF-coalesced).
 */
export function findFilletCorner(
  at: Point2,
  entities: SketchEntity[],
  opts: { reach: number; weldTol: number },
): CornerPick | null {
  const lines = entities.filter((e) => e.type === "Line" && e.p0 && e.p1);
  const cursor: XY = [at.x, at.y];
  let best: CornerPick | null = null;
  let bestD = Infinity;
  for (let i = 0; i < lines.length; i++) {
    for (let j = i + 1; j < lines.length; j++) {
      const c = sharedCorner(lines[i], lines[j], opts.weldTol);
      if (!c) continue;
      const d = distance(c.corner, cursor);
      if (d < bestD && d <= opts.reach) {
        bestD = d;
        best = { a: lines[i], b: lines[j], corner: c.corner };
      }
    }
  }
  return best;
}
