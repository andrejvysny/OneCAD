/*
 * constraintAuthoring — PURE: turn an `ApplicableConstraint` (the ordered targets
 * `evaluateApplicability` emits for the current selection) into the authored
 * artifact for each of the two commit paths (S4b design items 2 + 3):
 *
 *   - GEOMETRIC kinds → `buildAppliedConstraint`: a ready `SketchConstraint` the
 *     apply-flow appends + upserts (reject-on-conflict). Point slots carry their
 *     `positions`; trailing whole-entity slots (OnCurve curve, Symmetric axis) get
 *     NO position slot, so `toWireConstraint`'s `ref(i)` resolves them as entity
 *     refs (matches sketchWireMap.ts's uniform resolution).
 *   - DIMENSIONAL kinds → `buildAppliedDimension`: the chip seed (measured value +
 *     plane anchor + unit suffix) plus a `build(value, id)` closure that mints the
 *     `SketchConstraint` on commit. Mirrors dimensionTool's spec builders but is
 *     TYPE-directed (the user chose the kind), so a 2-line selection can author
 *     either Distance OR Angle on demand.
 *
 * No store / engine / ipc-client wiring — only TYPE imports plus the sibling pure
 * `constraintTarget` (coord resolution) + `dimensionTool` (angle math) modules.
 */
import type { ConstraintPosition, SketchConstraint, SketchEntity } from "@/ipc/types";
import { LENGTH_SUFFIX } from "@/units/format";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import type { ApplicableConstraint } from "./constraintApplicability";
import {
  resolveTargetPoint,
  type SketchConstraintTarget,
  type SketchConstraintTargetPoint,
} from "./constraintTarget";
import { angleBetweenDeg } from "./dimensionTool";

// ── small plane geometry ──────────────────────────────────────────────────────

type XY = [number, number];
const len = (a: XY, b: XY): number => Math.hypot(a[0] - b[0], a[1] - b[1]);
const mid = (a: XY, b: XY): Point2 => ({ x: (a[0] + b[0]) / 2, y: (a[1] + b[1]) / 2 });
const midXY = (a: XY, b: XY): XY => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];

/** Perpendicular distance from point `p` to the infinite line through `s0`,`s1`. */
function pointToLineDist(p: XY, s0: XY, s1: XY): number {
  const dx = s1[0] - s0[0];
  const dy = s1[1] - s0[1];
  const l = Math.hypot(dx, dy);
  if (l < 1e-12) return len(p, s0);
  return Math.abs((p[0] - s0[0]) * dy - (p[1] - s0[1]) * dx) / l;
}

/** A point target's position (Start/End/Center); whole-entity targets never reach
 *  the point slots, so the "Start" fallback is unreachable (keeps types total). */
const posOf = (t: SketchConstraintTarget): ConstraintPosition =>
  t.kind === "point" ? t.position : "Start";

/** The two endpoints of a Line entity, or null. */
function lineSeg(entities: SketchEntity[], id: string): [XY, XY] | null {
  const e = entities.find((x) => x.id === id);
  return e && e.type === "Line" && e.p0 && e.p1 ? [e.p0, e.p1] : null;
}

/** A Circle/Arc entity's center + radius, or null. */
function circular(entities: SketchEntity[], id: string): { center: XY; radius: number } | null {
  const e = entities.find((x) => x.id === id);
  if ((e?.type === "Circle" || e?.type === "Arc") && e.center && e.radius !== undefined) {
    return { center: e.center, radius: e.radius };
  }
  return null;
}

const pt = (t: SketchConstraintTarget, entities: SketchEntity[]): XY | null =>
  t.kind === "point" ? resolveTargetPoint(t as SketchConstraintTargetPoint, entities) : null;

// ── geometric author ──────────────────────────────────────────────────────────

/**
 * Build the authored `SketchConstraint` for a GEOMETRIC applicable (id minted by
 * the caller). Returns null for a dimensional kind (routed through the chip) or a
 * malformed target set.
 */
export function buildAppliedConstraint(
  a: ApplicableConstraint,
  id: string,
): SketchConstraint | null {
  const t = a.targets;
  switch (a.type) {
    case "Coincident": {
      const [p, q] = t;
      return { id, type: "Coincident", entities: [p.entityId, q.entityId], positions: [posOf(p), posOf(q)] };
    }
    case "Horizontal":
    case "Vertical":
      return { id, type: a.type, entities: [t[0].entityId] };
    case "Parallel":
    case "Perpendicular":
    case "Tangent":
    case "Equal":
      return { id, type: a.type, entities: [t[0].entityId, t[1].entityId] };
    case "Concentric":
      return { id, type: "Concentric", entities: [t[0].entityId, t[1].entityId] };
    case "Midpoint": {
      // evaluateApplicability already orders these [point, line], but normalize
      // defensively (mirrors the OnCurve case just below).
      const point = t.find((x) => x.kind === "point");
      const line = t.find((x) => x.kind === "line");
      if (!point || !line) return null;
      return { id, type: "Midpoint", entities: [point.entityId, line.entityId], positions: [posOf(point)] };
    }
    case "Fixed": {
      const p = t[0];
      return { id, type: "Fixed", entities: [p.entityId], positions: [posOf(p)] };
    }
    case "OnCurve": {
      // Normalize point-first (targets may arrive point+curve or curve+point).
      const point = t.find((x) => x.kind === "point");
      const curve = t.find((x) => x.kind !== "point");
      if (!point || !curve) return null;
      return { id, type: "OnCurve", entities: [point.entityId, curve.entityId], positions: [posOf(point)] };
    }
    case "Symmetric": {
      // evaluateApplicability orders these [point1, point2, axisLine].
      const [p1, p2, axis] = t;
      return {
        id,
        type: "Symmetric",
        entities: [p1.entityId, p2.entityId, axis.entityId],
        positions: [posOf(p1), posOf(p2)],
      };
    }
    default:
      return null; // dimensional kinds → buildAppliedDimension
  }
}

// ── dimensional author (chip seed + deferred build) ───────────────────────────

/** The seed a dimensional applicable feeds the Dimension chip. */
export interface AppliedDimension {
  /** Measured seed value (mm, or degrees for Angle). */
  value: number;
  /** Plane-coord anchor for the chip. */
  anchor: Point2;
  /** Unit suffix (`mm` / `°`). */
  suffix: string;
  /** Mint the authored constraint once the user commits a value. */
  build(value: number, id: string): SketchConstraint;
}

/**
 * Build the chip seed for a DIMENSIONAL applicable (Distance / Radius / Diameter /
 * Angle / H-/V-Distance). Returns null for a geometric kind or malformed targets.
 * The value is MEASURED from the current geometry; the user edits it in the chip.
 */
export function buildAppliedDimension(
  a: ApplicableConstraint,
  entities: SketchEntity[],
): AppliedDimension | null {
  const t = a.targets;
  switch (a.type) {
    case "Radius":
    case "Diameter": {
      const c = circular(entities, t[0].entityId);
      if (!c) return null;
      const value = a.type === "Diameter" ? 2 * c.radius : c.radius;
      const id0 = t[0].entityId;
      return {
        value,
        anchor: { x: c.center[0], y: c.center[1] },
        suffix: LENGTH_SUFFIX,
        build: (v, id) => ({ id, type: a.type, entities: [id0], value: v }),
      };
    }
    case "Angle": {
      const sa = lineSeg(entities, t[0].entityId);
      const sb = lineSeg(entities, t[1].entityId);
      if (!sa || !sb) return null;
      const value = angleBetweenDeg(sa[0], sa[1], sb[0], sb[1]);
      const [aId, bId] = [t[0].entityId, t[1].entityId];
      return {
        value,
        anchor: mid(midXY(sa[0], sa[1]), midXY(sb[0], sb[1])),
        suffix: "°",
        build: (v, id) => ({ id, type: "Angle", entities: [aId, bId], value: v }),
      };
    }
    case "HorizontalDistance":
    case "VerticalDistance": {
      const [p, q] = t;
      const cp = pt(p, entities);
      const cq = pt(q, entities);
      if (!cp || !cq) return null;
      const value = a.type === "HorizontalDistance" ? cq[0] - cp[0] : cq[1] - cp[1];
      return {
        value,
        anchor: mid(cp, cq),
        suffix: LENGTH_SUFFIX,
        build: (v, id) => ({
          id,
          type: a.type,
          entities: [p.entityId, q.entityId],
          positions: [posOf(p), posOf(q)],
          value: v,
        }),
      };
    }
    case "Distance":
      return distanceDimension(t, entities);
    default:
      return null; // geometric kinds → buildAppliedConstraint
  }
}

/** Distance across the three target shapes the matrix offers (pt-pt, pt-line,
 *  line-line). Normalizes point-first for pt-line so the point's `positions` slot
 *  is index 0 and the line resolves as a plain entity ref. */
function distanceDimension(
  t: SketchConstraintTarget[],
  entities: SketchEntity[],
): AppliedDimension | null {
  const points = t.filter((x) => x.kind === "point");
  const lines = t.filter((x) => x.kind === "line");

  // point ↔ point
  if (points.length === 2) {
    const [p, q] = points;
    const cp = pt(p, entities);
    const cq = pt(q, entities);
    if (!cp || !cq) return null;
    return {
      value: len(cp, cq),
      anchor: mid(cp, cq),
      suffix: LENGTH_SUFFIX,
      build: (v, id) => ({
        id,
        type: "Distance",
        entities: [p.entityId, q.entityId],
        positions: [posOf(p), posOf(q)],
        value: v,
      }),
    };
  }

  // point ↔ line (perpendicular)
  if (points.length === 1 && lines.length === 1) {
    const p = points[0];
    const cp = pt(p, entities);
    const seg = lineSeg(entities, lines[0].entityId);
    if (!cp || !seg) return null;
    return {
      value: pointToLineDist(cp, seg[0], seg[1]),
      anchor: mid(cp, midXY(seg[0], seg[1])),
      suffix: LENGTH_SUFFIX,
      build: (v, id) => ({
        id,
        type: "Distance",
        entities: [p.entityId, lines[0].entityId],
        positions: [posOf(p)],
        value: v,
      }),
    };
  }

  // line ↔ line (distance of one line's midpoint to the other)
  if (lines.length === 2) {
    const sa = lineSeg(entities, lines[0].entityId);
    const sb = lineSeg(entities, lines[1].entityId);
    if (!sa || !sb) return null;
    const ma = midXY(sa[0], sa[1]);
    return {
      value: pointToLineDist(ma, sb[0], sb[1]),
      anchor: mid(ma, midXY(sb[0], sb[1])),
      suffix: LENGTH_SUFFIX,
      build: (v, id) => ({
        id,
        type: "Distance",
        entities: [lines[0].entityId, lines[1].entityId],
        value: v,
      }),
    };
  }

  return null;
}
