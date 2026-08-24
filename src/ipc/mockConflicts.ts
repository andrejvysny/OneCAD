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
 * Five provable-contradiction shapes:
 *   R1 — two dimensional constraints of the SAME kind (Distance/HorizontalDistance/
 *        VerticalDistance/Radius/Diameter/Angle) pinning the SAME normalized target
 *        to two different values.
 *   R2 — Horizontal AND Vertical authored on the same line.
 *   R3 — Parallel AND Perpendicular authored on the same unordered line pair.
 *   R4 — Fixed contradictions: (a) two Fixed points declared Coincident but
 *        pinned at two different actual locations, or (b) a Distance between two
 *        Fixed points whose authored value disagrees with their actual current
 *        separation.
 *   R5 — direction transitivity: Parallel/Perpendicular constraints partition
 *        LINE entities into axis-parity classes (a signed union-find — Parallel
 *        unions same parity, Perpendicular unions flipped parity), and a
 *        Horizontal/Vertical constraint asserts a parity-to-axis mapping for its
 *        line's whole class. A contradiction is two H/V constraints in the SAME
 *        class asserting DIFFERENT mappings — e.g. `H(a) + V(b) + Parallel(a,b)`,
 *        or `H(a) + H(b) + Perpendicular(a,b)` — at ANY chain depth, not just a
 *        direct pair (R2 already covers the direct same-line case; R5 is its
 *        transitive generalization). A standalone Parallel/Perpendicular cycle
 *        with no H/V anchor is OUT OF SCOPE (not requested, and R3 already
 *        covers the direct same-pair case).
 *
 * `autoConstrain.ts`'s inferred batches (Horizontal/Vertical/Coincident/
 * Perpendicular/Parallel/Tangent/Concentric/Equal) never trip any rule here —
 * see that module's own mutual-exclusion gates (H xor V; H/V gates Perp/Parallel;
 * a fixed pair can't satisfy both the near-0 and near-90° folded-angle windows)
 * and `mockConflicts.test.ts`'s regression case built from a real inferred batch.
 * The H/V-gates-Perp/Parallel rule also makes R5's target contradiction
 * structurally unreachable from inference alone: a line Parallel to an H line
 * shares its exact direction, so it would itself already classify as H, not
 * get a separate Parallel constraint — R5 only fires on HAND-AUTHORED chains.
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
 * A signed (weighted) union-find over line ids: `union(a, b, rel)` with
 * `rel = 0` (Parallel — same parity) or `rel = 1` (Perpendicular — flipped
 * parity). `find` path-compresses and returns the parity of `id` RELATIVE TO
 * its class root. A line never seen before is its own singleton root at
 * parity 0, so two lines with no Parallel/Perpendicular chain between them are
 * never in the same class — R5 only fires within a HAND-linked class.
 *
 * A union that finds the pair ALREADY in the same class with disagreeing
 * parity (a standalone Parallel/Perpendicular cycle, no H/V involved) is
 * silently ignored — out of scope, see the module header.
 */
class AxisParityClasses {
  private readonly parent = new Map<string, string>();
  private readonly parity = new Map<string, 0 | 1>();

  find(id: string): { root: string; parity: 0 | 1 } {
    const p = this.parent.get(id);
    if (p === undefined) {
      this.parent.set(id, id);
      this.parity.set(id, 0);
      return { root: id, parity: 0 };
    }
    if (p === id) return { root: id, parity: 0 };
    const above = this.find(p);
    const total = ((this.parity.get(id)! ^ above.parity) as 0 | 1);
    this.parent.set(id, above.root);
    this.parity.set(id, total);
    return { root: above.root, parity: total };
  }

  union(a: string, b: string, rel: 0 | 1): void {
    const fa = this.find(a);
    const fb = this.find(b);
    if (fa.root === fb.root) return; // consistent-or-not, out of scope either way
    this.parent.set(fa.root, fb.root);
    this.parity.set(fa.root, (rel ^ fa.parity ^ fb.parity) as 0 | 1);
  }
}

/**
 * R5: direction transitivity. Builds the axis-parity classes from every
 * Parallel/Perpendicular pair, then for every Horizontal/Vertical constraint
 * computes `f = Horizontal ? parity : 1 − parity` — the class-relative parity
 * that would make "this line's parity means Horizontal" a consistent global
 * rule. Two DIFFERENT `f` values inside the same class is the contradiction
 * (see the module header for the derivation); the offending class's full H/V
 * constraint set is returned so the reject-on-conflict UX can name a loser.
 */
function detectDirectionTransitivityClash(constraints: SketchConstraint[]): string[] {
  const classes = new AxisParityClasses();
  for (const c of constraints) {
    if (c.type !== "Parallel" && c.type !== "Perpendicular") continue;
    if (c.entities.length < 2) continue;
    classes.union(c.entities[0], c.entities[1], c.type === "Perpendicular" ? 1 : 0);
  }

  const byRoot = new Map<string, Map<0 | 1, SketchConstraint[]>>();
  for (const c of constraints) {
    if (c.type !== "Horizontal" && c.type !== "Vertical") continue;
    const lineId = c.entities[0];
    if (!lineId) continue;
    const { root, parity } = classes.find(lineId);
    const f: 0 | 1 = c.type === "Horizontal" ? parity : ((1 - parity) as 0 | 1);
    const buckets = byRoot.get(root) ?? new Map<0 | 1, SketchConstraint[]>();
    const bucket = buckets.get(f) ?? [];
    bucket.push(c);
    buckets.set(f, bucket);
    byRoot.set(root, buckets);
  }

  for (const buckets of byRoot.values()) {
    if (buckets.size < 2) continue;
    const ids: string[] = [];
    for (const bucket of buckets.values()) for (const c of bucket) ids.push(c.id);
    return ids;
  }
  return [];
}

/**
 * Ids of ONE clashing constraint group, or `[]` when no provable contradiction
 * exists. Checked R1 → R2 → R3 → R4 → R5, first hit wins — a sketch could in
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
  const fixed = detectFixedContradiction(entities, constraints);
  if (fixed.length > 0) return fixed;
  return detectDirectionTransitivityClash(constraints);
}
