/*
 * Dimension-line layout (PURE) — the witness/extension-line geometry drawn
 * beside a committed straight edge that already carries a driving length
 * constraint (Distance/HorizontalDistance/VerticalDistance), mirroring
 * `badgeLayout.ts`'s constraint-driven trigger for the SAME constraints (the
 * value chip badgeLayout already places stays the label; this only adds the
 * witness lines + arrowheads Shapr3D draws alongside it).
 *
 * AUTHORED-ONLY (Sketcher UX cleanup, Track B3). This used to also emit a
 * full witness for a merely-SELECTED, unconstrained edge — but that made
 * passive selection feedback look identical to a real dimension constraint.
 * The passive case now lives in `unconstrainedEdgeLabel`/`hasAuthoredLength`
 * below: a lightweight midpoint + value, no offset/tick/arrow geometry at
 * all (`SelectionDimensionLabels` is the only reader of those). `editable`
 * accordingly dropped off `DimensionLine` — every line this module emits now
 * IS a real, authored constraint.
 *
 * Geometry lives in plane (u,v) — the engine draws it in `SketchObject`'s
 * plane-local group, same as every other sketch entity.
 */
import type { ConstraintPosition, SketchConstraint, SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";

const LENGTH_TYPES = new Set(["Distance", "HorizontalDistance", "VerticalDistance"]);

export interface DimensionLine {
  id: string;
  /** Extension ticks: each edge endpoint out to its point on the offset baseline. */
  ticks: [[Point2, Point2], [Point2, Point2]];
  /** The offset baseline between the two tick ends. */
  baseline: [Point2, Point2];
  /** Two short inward-pointing arrowhead strokes at each baseline end. */
  arrows: [Point2, Point2][];
  /** mm length shown on the baseline. */
  value: number;
}

const sub = (a: Point2, b: Point2): Point2 => ({ x: a.x - b.x, y: a.y - b.y });
const add = (a: Point2, b: Point2): Point2 => ({ x: a.x + b.x, y: a.y + b.y });
const scale = (a: Point2, k: number): Point2 => ({ x: a.x * k, y: a.y * k });
const len = (a: Point2): number => Math.hypot(a.x, a.y);
const norm = (a: Point2): Point2 => {
  const l = len(a);
  return l < 1e-9 ? { x: 0, y: 0 } : scale(a, 1 / l);
};
const rot90 = (a: Point2): Point2 => ({ x: -a.y, y: a.x });
const xy = (p: [number, number]): Point2 => ({ x: p[0], y: p[1] });

/** The (u,v) coord of an entity's named point — local twin of
 *  `badgeLayout.ts`'s `entityPointCoord` (not imported: `tools/sketch` has
 *  no existing dependency on `features/sketch` and this is a ~10-line pure
 *  lookup, not worth introducing a new cross-layer edge for). */
function resolvePoint(e: SketchEntity, position: ConstraintPosition): Point2 | null {
  if (position === "Center" && e.center) return xy(e.center);
  if (position === "Start") {
    if (e.type === "Arc" && e.start) return xy(e.start);
    if (e.p0) return xy(e.p0);
  }
  if (position === "End") {
    if (e.type === "Arc" && e.end) return xy(e.end);
    if (e.p1) return xy(e.p1);
  }
  if (position === "Midpoint" && e.p0 && e.p1) return scale(add(xy(e.p0), xy(e.p1)), 0.5);
  return null;
}

/**
 * The two points a length constraint actually measures between. Two shapes:
 *  - two entities + `positions` (point-to-point, e.g. a Distance between a
 *    line's Start and a different entity's End) — resolved via each one's
 *    own named point;
 *  - one entity, no positions — the constraint measures that entity's own
 *    length end to end (a Line's own p0→p1, the only shape this module
 *    handled before).
 * Null if either point can't be resolved (missing entity, non-Line single
 * target, or a position an entity of that type doesn't carry).
 */
function resolveConstraintPoints(
  c: SketchConstraint,
  byId: Map<string, SketchEntity>,
): [Point2, Point2] | null {
  const e0 = byId.get(c.entities[0]);
  if (!e0) return null;
  if (c.entities.length >= 2 && c.positions && c.positions.length >= 2) {
    const e1 = byId.get(c.entities[1]);
    if (!e1) return null;
    const p0 = resolvePoint(e0, c.positions[0]);
    const p1 = resolvePoint(e1, c.positions[1]);
    return p0 && p1 ? [p0, p1] : null;
  }
  if (e0.type !== "Line" || !e0.p0 || !e0.p1) return null;
  return [xy(e0.p0), xy(e0.p1)];
}

/** Rough plane-space size of the whole sketch, from every entity's own
 *  points — used only to scale the offset/arrowhead to the sketch's own
 *  size, not to any fixed screen quantity. */
function sketchExtent(entities: SketchEntity[]): { center: Point2; diag: number } {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  const consider = (p?: [number, number]) => {
    if (!p) return;
    minX = Math.min(minX, p[0]);
    maxX = Math.max(maxX, p[0]);
    minY = Math.min(minY, p[1]);
    maxY = Math.max(maxY, p[1]);
  };
  for (const e of entities) {
    consider(e.p0);
    consider(e.p1);
    consider(e.center);
    consider(e.start);
    consider(e.end);
  }
  if (!Number.isFinite(minX)) return { center: { x: 0, y: 0 }, diag: 1 };
  return {
    center: { x: (minX + maxX) / 2, y: (minY + maxY) / 2 },
    diag: Math.max(Math.hypot(maxX - minX, maxY - minY), 1e-6),
  };
}

/** Fraction of the sketch's own bounding diagonal the baseline clears the edge by. */
const OFFSET_FRACTION = 0.08;
/** Fraction of the offset distance an arrowhead stroke spans. */
const ARROW_FRACTION = 0.3;
const ARROW_ANGLE_RAD = (20 * Math.PI) / 180;

function arrowStrokes(base: Point2, dirTowardOther: Point2, armLen: number): [Point2, Point2][] {
  const cos = Math.cos(ARROW_ANGLE_RAD);
  const sin = Math.sin(ARROW_ANGLE_RAD);
  const rotate = (v: Point2, s: number): Point2 => ({
    x: v.x * cos - v.y * s * sin,
    y: v.x * s * sin + v.y * cos,
  });
  const a = add(base, scale(rotate(dirTowardOther, 1), armLen));
  const b = add(base, scale(rotate(dirTowardOther, -1), armLen));
  return [
    [base, a],
    [base, b],
  ];
}

/** Which axis a dimension's baseline/extension-tick geometry follows.
 *  `auto` (Distance): the baseline runs parallel to the actual p0→p1
 *  segment, whatever its angle — correct for a true point-to-point distance.
 *  `horizontal`/`vertical` (HorizontalDistance/VerticalDistance): the
 *  baseline is FORCED onto that axis regardless of the segment's own angle —
 *  an angled line's HorizontalDistance must read as the horizontal
 *  projection (a plumb-square witness), not a witness parallel to the line
 *  itself, which was visually false engineering information for anything
 *  but an already-horizontal edge. */
type DimensionAxis = "auto" | "horizontal" | "vertical";

/** Build one dimension line's geometry between two target points (same
 *  entity's own p0/p1, or two different entities' named points — see
 *  `resolveConstraintPoints`), or null if the points don't support the
 *  requested axis (coincident for `auto`; equal on the measured axis for
 *  `horizontal`/`vertical`, which would collapse the baseline to a point). */
function buildDimension(
  p0: Point2,
  p1: Point2,
  id: string,
  value: number,
  center: Point2,
  offset: number,
  armLen: number,
  axis: DimensionAxis,
): DimensionLine | null {
  const mid = scale(add(p0, p1), 0.5);
  let n: Point2; // baseline offset direction (perpendicular to the baseline)
  let dir: Point2; // baseline's own direction (for arrow orientation)
  if (axis === "horizontal") {
    if (p0.x === p1.x) return null; // no horizontal span to project
    n = { x: 0, y: 1 };
    dir = { x: 1, y: 0 };
  } else if (axis === "vertical") {
    if (p0.y === p1.y) return null; // no vertical span to project
    n = { x: 1, y: 0 };
    dir = { x: 0, y: 1 };
  } else {
    dir = norm(sub(p1, p0));
    if (dir.x === 0 && dir.y === 0) return null; // coincident points
    n = rot90(dir);
  }
  // Perpendicular candidate that points AWAY from the sketch's own centroid —
  // this is what puts the baseline outside the shape, matching the reference:
  // a rectangle's top/right edges dimension outward, not inward.
  if ((mid.x - center.x) * n.x + (mid.y - center.y) * n.y < 0) n = scale(n, -1);

  // horizontal/vertical: BOTH points project onto ONE shared baseline
  // coordinate (offset from their midpoint) — offsetting each point
  // independently by `n` would just translate the original (possibly
  // angled) segment sideways, not force it onto the axis. `auto`'s p0/p1
  // share `dir` exactly, so offsetting each independently by the same `n`
  // already produces a baseline parallel to the segment — the two formulas
  // agree when the segment is already axis-aligned, which is why this was
  // never wrong for a horizontal/vertical Distance, only for an angled one.
  let base0: Point2;
  let base1: Point2;
  if (axis === "auto") {
    base0 = add(p0, scale(n, offset));
    base1 = add(p1, scale(n, offset));
  } else {
    const baselineRef = add(mid, scale(n, offset));
    base0 = axis === "horizontal" ? { x: p0.x, y: baselineRef.y } : { x: baselineRef.x, y: p0.y };
    base1 = axis === "horizontal" ? { x: p1.x, y: baselineRef.y } : { x: baselineRef.x, y: p1.y };
  }

  return {
    id,
    ticks: [
      [p0, base0],
      [p1, base1],
    ],
    baseline: [base0, base1],
    arrows: [...arrowStrokes(base0, dir, armLen), ...arrowStrokes(base1, scale(dir, -1), armLen)],
    value,
  };
}

/**
 * One dimension line per Distance/HorizontalDistance/VerticalDistance
 * constraint with a defined value — same trigger `badgeLayout.ts`'s
 * dimensional badges already use for the SAME constraint, so this is purely
 * an additional visual for a value that's already shown (editable via that
 * existing chip). Authored constraints ONLY — see the module header for the
 * passive-selection case.
 *
 * Witness geometry is now type-correct rather than uniform (P1 hardening):
 * Distance draws parallel to the ACTUAL two target points (which may belong
 * to different entities), while Horizontal/VerticalDistance force the
 * baseline onto their own axis regardless of the target's angle — an angled
 * line's HorizontalDistance used to draw a witness parallel to the line
 * itself, which read as false engineering information for anything but an
 * already-horizontal edge. See `buildDimension`'s `axis` parameter.
 */
export function layoutDimensionLines(
  entities: SketchEntity[],
  constraints: SketchConstraint[],
): DimensionLine[] {
  const byId = new Map(entities.map((e) => [e.id, e]));
  const { center, diag } = sketchExtent(entities);
  const offset = diag * OFFSET_FRACTION;
  const armLen = offset * ARROW_FRACTION;

  const out: DimensionLine[] = [];
  const seen = new Set<string>();
  for (const c of constraints) {
    if (!LENGTH_TYPES.has(c.type) || c.value === undefined) continue;
    // Dedup key: the target span, order-independent — stops a second length
    // constraint on the exact same pair of points from drawing an
    // overlapping witness. Single-entity form (`entities.length < 2`) keys
    // on that one entity, same as before this module supported point-to-
    // point targets.
    const dedupeKey =
      c.entities.length >= 2 && c.positions && c.positions.length >= 2
        ? [c.entities[0], c.positions[0], c.entities[1], c.positions[1]].sort().join("|")
        : c.entities[0];
    if (!dedupeKey || seen.has(dedupeKey)) continue;
    const pts = resolveConstraintPoints(c, byId);
    if (!pts) continue;
    seen.add(dedupeKey);
    const axis: DimensionAxis =
      c.type === "HorizontalDistance" ? "horizontal" : c.type === "VerticalDistance" ? "vertical" : "auto";
    const dim = buildDimension(pts[0], pts[1], c.id, c.value, center, offset, armLen, axis);
    if (dim) out.push(dim);
  }
  return out;
}

/** Whether `entityId` already carries an authored length constraint
 *  (Distance/Horizontal/VerticalDistance) with a defined value — same
 *  trigger `layoutDimensionLines` itself keys off. Exported so a passive-
 *  measurement caller (`SelectionDimensionLabels`) can dedupe against an
 *  already-constrained edge without re-deriving `LENGTH_TYPES` itself: that
 *  edge's value already shows through the authored witness + its editable
 *  chip, so a passive label on it would be a duplicate. Checks EVERY
 *  referenced entity, not just `entities[0]` — a point-to-point Distance
 *  targets `entityId` just as much when it's the SECOND entity. */
export function hasAuthoredLength(entityId: string, constraints: SketchConstraint[]): boolean {
  return constraints.some(
    (c) => LENGTH_TYPES.has(c.type) && c.value !== undefined && c.entities.includes(entityId),
  );
}

export interface UnconstrainedEdgeLabel {
  id: string;
  at: Point2;
  value: number;
}

/**
 * Live length label anchor for a Line entity — the lightweight, witness-free
 * passive-measurement case (Sketcher UX cleanup, Track B3): just a midpoint
 * + value, no offset/tick/arrow geometry at all, so a merely-selected edge
 * reads as a measurement, not a dimension constraint. Null for a non-Line or
 * degenerate (missing p0/p1) entity. The caller is expected to have already
 * excluded any entity `hasAuthoredLength` on.
 */
export function unconstrainedEdgeLabel(e: SketchEntity): UnconstrainedEdgeLabel | null {
  if (e.type !== "Line" || !e.p0 || !e.p1) return null;
  return {
    id: `sel-${e.id}`,
    at: { x: (e.p0[0] + e.p1[0]) / 2, y: (e.p0[1] + e.p1[1]) / 2 },
    value: Math.hypot(e.p1[0] - e.p0[0], e.p1[1] - e.p0[1]),
  };
}
