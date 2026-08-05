/*
 * extendMath — PURE geometry for the sketch Extend tool, Trim's counterpart.
 *
 * Hovering a line/arc near one of its ends grows THAT end along its own
 * unbounded continuation until it meets another entity; a click commits the
 * grown entity. Where Trim removes the span between two crossings, Extend adds
 * the span between an endpoint and the first crossing BEYOND it.
 *
 * ── Why this cannot reuse trim's param function ────────────────────────────────
 * `trimMath.targetParam` gates the parameter to the target's own extent ([0,1]
 * for a line/arc). Every candidate an extend cares about lies OUTSIDE that
 * range — past t=1 or before t=0 — so trim's gate rejects precisely the points
 * this module is looking for. The param functions below are UNBOUNDED; the
 * curve model, the full-curve intersections and the `withinExtent` gate on the
 * OTHER entity are shared with `trimMath` so the two tools agree on what a
 * crossing is.
 *
 * ── Param spaces ──────────────────────────────────────────────────────────────
 *   Line : t along p0→p1, unbounded. t∈[0,1] is ON the segment; t>1 is past the
 *          `end`, t<0 is before the `start`.
 *   Arc  : s sweep-relative, signed by DIRECTION of travel. Growing the `end`
 *          continues CCW, so s runs (1, 2π/sweep); growing the `start`
 *          continues CW, so s runs (−2π/sweep, 0).
 *
 * ── Rules ─────────────────────────────────────────────────────────────────────
 *   - Which end grows is decided by the HOVER point, not by the crossing: the
 *     user aims at the end they want (`t < 0.5` ⇒ start, else end).
 *   - Candidates are gated by `withinExtent` on the OTHER entity — the
 *     continuation must actually reach that entity's drawn span, not the
 *     infinite line or full circle behind it.
 *   - The NEAREST forward crossing wins (smallest overshoot), which is what
 *     makes repeated extends walk outward one entity at a time.
 *   - An arc may never wrap a full turn back onto itself: a crossing at (or
 *     numerically at) the arc's own far endpoint is not a place to grow to.
 *   - Growth shorter than `minSize` is rejected — an "extension" the user cannot
 *     see is a no-op that would still burn an undo step.
 *   - A CIRCLE has no end to grow, and an ELLIPSE is not modelled by `curveOf`;
 *     both return null, and the caller keeps the entity untouched.
 *
 * PURE ⇒ every edge is unit-tested by data (extendMath.test.ts).
 */
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import type { DraftEntity } from "./toolMachine";
import {
  angleOf,
  arcSweep,
  curveOf,
  fullIntersections,
  withinExtent,
  type Curve,
  type XY,
} from "./trimMath";

/** Param-equality epsilon, FIXED in param space — the same 1e-9 `trimMath` uses
 *  (see the note there on why it is not scaled by length). */
const EPS = 1e-9;
const TWO_PI = Math.PI * 2;

/** Which endpoint of the target the extension moved. */
export type ExtendEnd = "start" | "end";

export interface ExtendResult {
  /** The target re-stated with the grown end — REPLACES the original entity. */
  extended: DraftEntity;
  /** ONLY the span that was gained (the ADDITIVE hover ghost). */
  addition: DraftEntity;
  /** Which end grew. */
  end: ExtendEnd;
  /** The crossing point the end grew to. */
  target: Point2;
  /** Length gained (arc length for an arc). */
  grew: number;
}

const pt = (p: XY): Point2 => ({ x: p[0], y: p[1] });

/** Fold an angle into [0, 2π) — the domain both arc params are stated in. */
const norm2Pi = (a: number): number => ((a % TWO_PI) + TWO_PI) % TWO_PI;

function lineDraft(a: Point2, b: Point2, construction?: boolean): DraftEntity {
  return { type: "Line", ...(construction ? { construction: true } : {}), p0: a, p1: b };
}

const onCircle = (c: XY, r: number, ang: number): Point2 => ({
  x: c[0] + r * Math.cos(ang),
  y: c[1] + r * Math.sin(ang),
});

function arcDraft(c: XY, r: number, start: Point2, end: Point2, construction?: boolean): DraftEntity {
  return {
    type: "Arc",
    ...(construction ? { construction: true } : {}),
    center: pt(c),
    radius: r,
    start,
    end,
  };
}

/**
 * Params (in the caller's own space) of every point where the target's UNBOUNDED
 * continuation meets an entity in `others`, gated to that entity's drawn extent.
 * The target's own extent is deliberately NOT applied — that is the whole point.
 */
function continuationParams(
  curve: Curve,
  others: SketchEntity[],
  paramOf: (p: XY) => number,
): number[] {
  const out: number[] = [];
  for (const o of others) {
    const oc = curveOf(o);
    if (!oc) continue;
    for (const p of fullIntersections(curve, oc)) {
      if (!withinExtent(oc, p)) continue;
      out.push(paramOf(p));
    }
  }
  return out;
}

/** Smallest param strictly above `floor`, or null when nothing qualifies. */
function nearestAbove(params: number[], floor: number): number | null {
  let best: number | null = null;
  for (const p of params) {
    if (p > floor && (best === null || p < best)) best = p;
  }
  return best;
}

/** Largest param strictly below `ceil`, or null when nothing qualifies. */
function nearestBelow(params: number[], ceil: number): number | null {
  let best: number | null = null;
  for (const p of params) {
    if (p < ceil && (best === null || p > best)) best = p;
  }
  return best;
}

// ── Line ──────────────────────────────────────────────────────────────────────

function extendLine(
  curve: Extract<Curve, { kind: "line" }>,
  construction: boolean | undefined,
  hoverPt: Point2,
  others: SketchEntity[],
  minSize: number,
): ExtendResult | null {
  const dx = curve.b[0] - curve.a[0];
  const dy = curve.b[1] - curve.a[1];
  const len2 = dx * dx + dy * dy;
  if (len2 < 1e-18) return null;
  const length = Math.sqrt(len2);
  const paramOf = (p: XY): number => ((p[0] - curve.a[0]) * dx + (p[1] - curve.a[1]) * dy) / len2;
  const end: ExtendEnd = paramOf([hoverPt.x, hoverPt.y]) < 0.5 ? "start" : "end";

  const params = continuationParams(curve, others, paramOf);
  // Past the far endpoint (t>1) growing the end; before t=0 growing the start.
  // The bound is strict-plus-ε so a crossing sitting exactly ON the endpoint is
  // not mistaken for somewhere to grow to.
  const t = end === "end" ? nearestAbove(params, 1 + EPS) : nearestBelow(params, -EPS);
  if (t === null) return null;
  const grew = (end === "end" ? t - 1 : -t) * length;
  if (grew < minSize) return null;

  const tip: Point2 = { x: curve.a[0] + t * dx, y: curve.a[1] + t * dy };
  const p0 = pt(curve.a);
  const p1 = pt(curve.b);
  return {
    extended: end === "end" ? lineDraft(p0, tip, construction) : lineDraft(tip, p1, construction),
    addition: end === "end" ? lineDraft(p1, tip, construction) : lineDraft(tip, p0, construction),
    end,
    target: tip,
    grew,
  };
}

// ── Arc ───────────────────────────────────────────────────────────────────────

function extendArc(
  curve: Extract<Curve, { kind: "arc" }>,
  construction: boolean | undefined,
  hoverPt: Point2,
  others: SketchEntity[],
  minSize: number,
): ExtendResult | null {
  const sweep = arcSweep(curve.c, curve.start, curve.end);
  const a0 = angleOf(curve.c, curve.start);
  const forward = (p: XY): number => norm2Pi(angleOf(curve.c, p) - a0) / sweep;
  const backward = (p: XY): number => -norm2Pi(a0 - angleOf(curve.c, p)) / sweep;

  // The hover is hit-tested onto the arc, so its forward param lands in [0,1];
  // the same "nearer which end" split the line uses applies.
  const end: ExtendEnd = forward([hoverPt.x, hoverPt.y]) < 0.5 ? "start" : "end";
  const paramOf = end === "end" ? forward : backward;
  const growthOf = (s: number): number => (end === "end" ? s - 1 : -s) * sweep;
  // WRAP GUARD. `freeAngle` is all the circle the arc does not already occupy —
  // growing by exactly that much would close the arc onto its OWN far endpoint,
  // which is a full turn, not an extension. It is a real case, not a paranoia
  // guard: any other entity crossing at that endpoint offers such a candidate,
  // and when it is the only one the nearest-crossing pick would take it.
  const freeAngle = TWO_PI - sweep;
  const params = continuationParams(curve, others, paramOf).filter(
    (s) => growthOf(s) < freeAngle - EPS,
  );
  const s = end === "end" ? nearestAbove(params, 1 + EPS) : nearestBelow(params, -EPS);
  if (s === null) return null;
  const grew = growthOf(s) * curve.r;
  if (grew < minSize) return null;

  const tip = onCircle(curve.c, curve.r, a0 + s * sweep);
  const start = pt(curve.start);
  const finish = pt(curve.end);
  // Both drafts stay CCW start→end (the one form the wire can express).
  return {
    extended:
      end === "end"
        ? arcDraft(curve.c, curve.r, start, tip, construction)
        : arcDraft(curve.c, curve.r, tip, finish, construction),
    addition:
      end === "end"
        ? arcDraft(curve.c, curve.r, finish, tip, construction)
        : arcDraft(curve.c, curve.r, tip, start, construction),
    end,
    target: tip,
    grew,
  };
}

// ── Core ──────────────────────────────────────────────────────────────────────

/**
 * Grow the end of `target` nearest `hoverPt` out to the first entity in `others`
 * its continuation reaches. Returns null when there is nothing to grow to (no
 * qualifying crossing, growth below `minSize`, or a closed/unmodelled curve) —
 * the caller then leaves the entity alone.
 */
export function extendPieces(
  target: SketchEntity,
  hoverPt: Point2,
  others: SketchEntity[],
  opts: { minSize: number },
): ExtendResult | null {
  const curve = curveOf(target);
  // A circle has no end to grow; an ellipse is not modelled by `curveOf` at all
  // (same whole-entity opt-out Trim takes).
  if (!curve || curve.kind === "circle") return null;
  if (curve.kind === "line") {
    return extendLine(curve, target.construction, hoverPt, others, opts.minSize);
  }
  return extendArc(curve, target.construction, hoverPt, others, opts.minSize);
}

/**
 * Hover-preview variant: ONLY the span the click would add, for the ADDITIVE
 * ghost. Null when there is nothing to extend to.
 */
export function extendPreview(
  target: SketchEntity,
  hoverPt: Point2,
  others: SketchEntity[],
  opts: { minSize: number },
): DraftEntity | null {
  return extendPieces(target, hoverPt, others, opts)?.addition ?? null;
}
