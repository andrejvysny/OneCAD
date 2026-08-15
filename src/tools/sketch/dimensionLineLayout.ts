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
import type { SketchConstraint, SketchEntity } from "@/ipc/types";
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

/** Build one dimension line's geometry for a Line entity, or null if its
 *  endpoints are coincident (no direction to offset perpendicular to). */
function buildLine(
  e: SketchEntity,
  id: string,
  value: number,
  center: Point2,
  offset: number,
  armLen: number,
): DimensionLine | null {
  if (!e.p0 || !e.p1) return null;
  const p0 = xy(e.p0);
  const p1 = xy(e.p1);
  const edgeDir = norm(sub(p1, p0));
  if (edgeDir.x === 0 && edgeDir.y === 0) return null; // degenerate edge
  const mid = scale(add(p0, p1), 0.5);
  // Perpendicular candidate that points AWAY from the sketch's own centroid —
  // this is what puts the baseline outside the shape, matching the reference:
  // a rectangle's top/right edges dimension outward, not inward.
  let n = rot90(edgeDir);
  if ((mid.x - center.x) * n.x + (mid.y - center.y) * n.y < 0) n = scale(n, -1);

  const base0 = add(p0, scale(n, offset));
  const base1 = add(p1, scale(n, offset));
  return {
    id,
    ticks: [
      [p0, base0],
      [p1, base1],
    ],
    baseline: [base0, base1],
    arrows: [...arrowStrokes(base0, edgeDir, armLen), ...arrowStrokes(base1, scale(edgeDir, -1), armLen)],
    value,
  };
}

/**
 * One dimension line per Line entity carrying a Distance/HorizontalDistance/
 * VerticalDistance constraint with a defined value — same trigger
 * `badgeLayout.ts`'s dimensional badges already use for the SAME entity, so
 * this is purely an additional visual for a value that's already shown
 * (editable via that existing chip). Authored constraints ONLY — see the
 * module header for the passive-selection case.
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
    const entityId = c.entities[0];
    if (!entityId || seen.has(entityId)) continue;
    const e = byId.get(entityId);
    if (!e || e.type !== "Line") continue;
    seen.add(entityId);
    const dim = buildLine(e, c.id, c.value, center, offset, armLen);
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
 *  chip, so a passive label on it would be a duplicate. */
export function hasAuthoredLength(entityId: string, constraints: SketchConstraint[]): boolean {
  return constraints.some(
    (c) => LENGTH_TYPES.has(c.type) && c.value !== undefined && c.entities[0] === entityId,
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
