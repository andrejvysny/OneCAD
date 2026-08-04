/*
 * mockConflicts — deterministic conflict detection for the MOCK sketch lane.
 *
 * These are RULES, not a solver: each fires ONLY on an unambiguous, computable
 * contradiction in the authored constraint set — never a heuristic guess about
 * what the real PlaneGCS Jacobian would report. The real lane's solver remains
 * the only authority on genuine (non-)solvability (SolverLane.cpp `upsert_state`);
 * this module exists so the mock lane's reject-on-conflict UX (a NAMED clashing
 * constraint, SCHEMA §7.4 `conflicting[]`) is reachable and e2e-testable without
 * the C++ worker.
 *
 * Four provable-contradiction shapes:
 *   R1 — two dimensional constraints of the SAME kind (Distance/HorizontalDistance/
 *        VerticalDistance/Radius/Diameter/Angle) pinning the SAME normalized target
 *        to two different values.
 *   R2 — Horizontal AND Vertical authored on the same line.
 *   R3 — Parallel AND Perpendicular authored on the same unordered line pair.
 *   R4 — Fixed contradictions: (a) two Fixed points declared Coincident but
 *        pinned at two different actual locations, or (b) a Distance between two
 *        Fixed points whose authored value disagrees with their actual current
 *        separation.
 *
 * `autoConstrain.ts`'s inferred batches (Horizontal/Vertical/Coincident/
 * Perpendicular/Parallel/Tangent/Concentric/Equal) never trip any rule here —
 * see that module's own mutual-exclusion gates (H xor V; H/V gates Perp/Parallel;
 * a fixed pair can't satisfy both the near-0 and near-90° folded-angle windows)
 * and `mockConflicts.test.ts`'s regression case built from a real inferred batch.
 */
import type { ConstraintPosition, SketchConstraint, SketchConstraintType, SketchEntity } from "./types";

/** Dimensional-value tolerance for R1 — two authored values differing by more
 *  than float noise are a genuine contradiction, not a rounding artifact. */
const VALUE_EPS = 1e-9;
/** Geometric tolerance for R4's point/distance comparisons (mm). */
const FIXED_EPS = 1e-6;

const DIMENSIONAL_KINDS: ReadonlySet<SketchConstraintType> = new Set([
  "Distance",
  "HorizontalDistance",
  "VerticalDistance",
  "Radius",
  "Diameter",
  "Angle",
]);

/** Current plane coord of a point ref (entity + optional position). Mirrors the
 *  local `pointCoord` in sketchWireMap.ts (the `at` a Fixed constraint pins to);
 *  kept local here too so this pure mock module carries no cross-module import. */
function pointCoord(
  entities: SketchEntity[],
  entityId: string,
  position?: ConstraintPosition,
): [number, number] | null {
  const e = entities.find((x) => x.id === entityId);
  if (!e) return null;
  switch (e.type) {
    case "Point":
      return e.p0 ?? null;
    case "Line":
      if (position === "End") return e.p1 ?? null;
      if (position === "Midpoint" && e.p0 && e.p1)
        return [(e.p0[0] + e.p1[0]) / 2, (e.p0[1] + e.p1[1]) / 2];
      return e.p0 ?? null; // Start (default)
    case "Circle":
    case "Ellipse":
      return e.center ?? null;
    case "Arc":
      if (position === "Start") return e.start ?? null;
      if (position === "End") return e.end ?? null;
      return e.center ?? null;
    default:
      return null;
  }
}

const dist = (a: [number, number], b: [number, number]): number => Math.hypot(a[0] - b[0], a[1] - b[1]);

/** Normalized per-slot tag: a named point when a position slot exists, else the
 *  whole entity ("body") — e.g. a point-line Distance's line slot, or an Angle's
 *  two whole-line refs. Sorting a constraint's tags makes target identity
 *  independent of authoring order (Start,End vs End,Start is the same target). */
function slotTag(entityId: string, position: ConstraintPosition | undefined): string {
  return `${entityId}#${position ?? "body"}`;
}

function targetKey(c: SketchConstraint): string {
  const tags = c.entities.map((id, i) => slotTag(id, c.positions?.[i])).sort();
  return `${c.type}::${tags.join("|")}`;
}

/** R1: duplicate-incompatible dimension — same kind, same normalized target,
 *  authored values that disagree by more than float noise. */
function detectDuplicateDimension(constraints: SketchConstraint[]): string[] {
  const groups = new Map<string, SketchConstraint[]>();
  for (const c of constraints) {
    if (!DIMENSIONAL_KINDS.has(c.type)) continue;
    const key = targetKey(c);
    const group = groups.get(key);
    if (group) group.push(c);
    else groups.set(key, [c]);
  }
  for (const group of groups.values()) {
    if (group.length < 2) continue;
    const values = group.map((c) => c.value ?? 0);
    if (Math.max(...values) - Math.min(...values) > VALUE_EPS) {
      return group.map((c) => c.id);
    }
  }
  return [];
}

/** R2: Horizontal + Vertical authored on the same line. */
function detectHVClash(constraints: SketchConstraint[]): string[] {
  const byLine = new Map<string, { h: SketchConstraint[]; v: SketchConstraint[] }>();
  for (const c of constraints) {
    if (c.type !== "Horizontal" && c.type !== "Vertical") continue;
    const lineId = c.entities[0];
    if (!lineId) continue;
    const bucket = byLine.get(lineId) ?? { h: [], v: [] };
    (c.type === "Horizontal" ? bucket.h : bucket.v).push(c);
    byLine.set(lineId, bucket);
  }
  for (const { h, v } of byLine.values()) {
    if (h.length > 0 && v.length > 0) return [...h, ...v].map((c) => c.id);
  }
  return [];
}

/** R3: Parallel + Perpendicular authored on the same unordered line pair. */
function detectParallelPerpClash(constraints: SketchConstraint[]): string[] {
  const byPair = new Map<string, { par: SketchConstraint[]; perp: SketchConstraint[] }>();
  for (const c of constraints) {
    if (c.type !== "Parallel" && c.type !== "Perpendicular") continue;
    if (c.entities.length < 2) continue;
    const key = [...c.entities].sort().join(",");
    const bucket = byPair.get(key) ?? { par: [], perp: [] };
    (c.type === "Parallel" ? bucket.par : bucket.perp).push(c);
    byPair.set(key, bucket);
  }
  for (const { par, perp } of byPair.values()) {
    if (par.length > 0 && perp.length > 0) return [...par, ...perp].map((c) => c.id);
  }
  return [];
}

/**
 * R4: Fixed contradictions.
 *   (a) two points declared Coincident (i.e. the user asserted "same point")
 *       each carry their OWN Fixed pin, and those pins resolve to different
 *       actual locations right now.
 *   (b) a Distance constraint between two Fixed points whose authored value
 *       disagrees with their actual current separation.
 * `entities` supplies the live coordinates a Fixed constraint has no field for
 * (its wire `at` is always derived from the entity, not stored on the
 * constraint — see sketchWireMap.ts `pointCoord`).
 */
function detectFixedContradiction(entities: SketchEntity[], constraints: SketchConstraint[]): string[] {
  const fixedByPoint = new Map<string, SketchConstraint>();
  for (const c of constraints) {
    if (c.type !== "Fixed") continue;
    const key = slotTag(c.entities[0], c.positions?.[0]);
    if (!fixedByPoint.has(key)) fixedByPoint.set(key, c);
  }
  if (fixedByPoint.size < 2) return [];

  for (const c of constraints) {
    if (c.type !== "Coincident" || c.entities.length < 2) continue;
    const fixedA = fixedByPoint.get(slotTag(c.entities[0], c.positions?.[0]));
    const fixedB = fixedByPoint.get(slotTag(c.entities[1], c.positions?.[1]));
    if (!fixedA || !fixedB || fixedA === fixedB) continue;
    const atA = pointCoord(entities, c.entities[0], c.positions?.[0]);
    const atB = pointCoord(entities, c.entities[1], c.positions?.[1]);
    if (atA && atB && dist(atA, atB) > FIXED_EPS) return [fixedA.id, fixedB.id];
  }

  for (const c of constraints) {
    if (c.type !== "Distance" || c.entities.length < 2 || c.value === undefined) continue;
    const fixedA = fixedByPoint.get(slotTag(c.entities[0], c.positions?.[0]));
    const fixedB = fixedByPoint.get(slotTag(c.entities[1], c.positions?.[1]));
    if (!fixedA || !fixedB) continue;
    const atA = pointCoord(entities, c.entities[0], c.positions?.[0]);
    const atB = pointCoord(entities, c.entities[1], c.positions?.[1]);
    if (!atA || !atB) continue;
    if (Math.abs(c.value - dist(atA, atB)) > FIXED_EPS) return [c.id, fixedA.id, fixedB.id];
  }

  return [];
}

/**
 * Ids of ONE clashing constraint group, or `[]` when no provable contradiction
 * exists. Checked R1 → R2 → R3 → R4, first hit wins — a sketch could in
 * principle trip more than one rule at once, but the reject-on-conflict UX only
 * needs to NAME one clashing party (`sketchService.ts` `rejectConflictHint`).
 */
export function detectConflicts(entities: SketchEntity[], constraints: SketchConstraint[]): string[] {
  const dup = detectDuplicateDimension(constraints);
  if (dup.length > 0) return dup;
  const hv = detectHVClash(constraints);
  if (hv.length > 0) return hv;
  const pp = detectParallelPerpClash(constraints);
  if (pp.length > 0) return pp;
  return detectFixedContradiction(entities, constraints);
}
