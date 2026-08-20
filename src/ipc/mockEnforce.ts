/*
 * mockEnforce — bounded, DELTA-DRIVEN driving of dimensional constraints onto
 * geometry, for the MOCK sketch lane's `sketchUpsert` (localSolver.ts). Same
 * philosophy as `mockConflicts.ts`: only PROVABLE, deterministic cases move
 * geometry — this is a rule set, never a solver.
 *
 * ── What "delta-driven" means, and why ──────────────────────────────────────
 * Every pass takes the PREVIOUS constraint list as well as the current one, and
 * looks at NOTHING but the difference between them:
 *   - CREATED — the constraint's id is not in `prevConstraints`.
 *   - EDITED  — the id IS in `prevConstraints`, at a DIFFERENT `value`.
 *   - everything else is UNCHANGED and is never even measured.
 *
 * An earlier version enforced the WHOLE constraint set on every upsert. That is
 * wrong three ways, all of them reproduced:
 *   - It refused constraints nobody touched. `isWelded` fires on Coincident, so
 *     every edge of a rectangle is un-drivable — authoring a Distance on a rect
 *     edge (the single most common sketch action) came back "Dimension removed".
 *   - A dimension that went stale via a DRAG (the drag lane never re-checks) was
 *     then refused on every LATER, unrelated upsert, wedging the sketch and
 *     painting an untouched constraint red forever.
 *   - Re-running it drifted geometry: Distance + HorizontalDistance on one line
 *     fight each other, and each fights again on the next upsert (26.83 → 24.91
 *     → 23.02 …). Nothing here is a solver, so nothing here converges.
 * Only re-evaluating the delta removes all three at once, and makes a repeated
 * upsert of an unchanged constraint set a strict identity.
 *
 * THE DELIBERATE TRADE: geometry that has drifted away from an UNCHANGED
 * dimension's value (a drag, a prior accept-without-driving, an H/V projection
 * that shortened a line) stays drifted and stays silent — exactly as it did
 * before any enforcement existed. The mock lane drives what it can prove,
 * refuses the edits it would otherwise lie about, and accepts-without-driving
 * the rest. The real PlaneGCS lane is the only authority that can actually
 * reconcile a whole constraint set; pretending otherwise here produced worse
 * failures than the staleness it was trying to remove.
 *
 * ── Decision table (per constraint, per upsert) ─────────────────────────────
 *                         │ CREATED           │ EDITED   │ UNCHANGED
 *   already matches geom.  │ ignore            │ ignore   │ ignore
 *   drivable (below)       │ DRIVE             │ DRIVE    │ ignore
 *   not drivable           │ accept, no move   │ REFUSE   │ ignore
 *   not even measurable    │ ignore            │ ignore   │ ignore
 * A CREATED constraint is never refused: refusing creation breaks the common
 * workflows (any dimension on a welded rectangle edge), and the real solver lane
 * handles those cases properly. Only an EDIT is refused, because an edit that
 * silently fails to move geometry is the one case that leaves a chip lying about
 * what is drawn (P1 audit finding, SKETCH_UX_AUDIT.md Part C item #8 — a
 * Diameter chip edited 25→30 used to leave the circle at r=12.5).
 *
 * ── The drivable shapes ─────────────────────────────────────────────────────
 * All guarded by (a) the entity is not `referenceLocked`, and (b) every point
 * actually MOVED is weld-free — no Coincident/Midpoint/OnCurve/Fixed constraint
 * references it anywhere (the mock cannot propagate a weld to whatever else that
 * point touches):
 *   - Radius on a CIRCLE: radius = value. Diameter on a CIRCLE: radius = value/2.
 *     The center does not move, so the weld guard is vacuous — only the lock
 *     guard applies. An ARC is NEVER driven: its `start`/`end` are stored
 *     coordinates that a radius rewrite would detach from the stroke, silently
 *     breaking any weld on them.
 *   - Distance whose two point refs are the SAME Line's Start and End: scale the
 *     line about its (fixed) midpoint along its current direction.
 *   - HorizontalDistance / VerticalDistance on the same-line endpoints: set that
 *     axis's delta to the value, keeping the midpoint and the other axis's delta.
 *   - Horizontal / Vertical (no `.value`, so only ever CREATED): project both
 *     endpoints onto the axis line through the current midpoint, making the line
 *     exactly horizontal/vertical instead of leaving an auto-inferred line ~0.4°
 *     off forever. Never refusable — there is no numeric label to lie.
 * `Angle` and every unsupported SHAPE of the cases above (a Distance NOT between
 * one line's own two endpoints, a locked or welded entity, a degenerate
 * zero-length line, …) are never driven — accepted on creation, refused on edit.
 * A value that cannot even be MEASURED (a stale/unresolvable ref) is left alone
 * either way; it is not this module's concern.
 *
 * ── Order within one upsert (fixed, documented, testable) ───────────────────
 *   1. newly CREATED Horizontal/Vertical projections, in constraint-array order;
 *   2. then the delta's dimensional drives, in constraint-array order.
 * A ⇧H applied together with a length edit therefore flattens the line first and
 * the length is then measured along the flattened direction, not the reverse.
 * The pass is pure and deterministic: re-running it on its own output with the
 * same arguments changes nothing further.
 */
import type { ConstraintPosition, CurveParams, SketchConstraint, SketchEntity } from "./types";

/** Geometry-match tolerance (mm) — mirrors `mockConflicts.ts` FIXED_EPS. Below
 *  this, "already matches" and no driving (or refusal) is needed. */
const EPS = 1e-6;

/** Weld constraint kinds the mock cannot propagate through (module header). */
const WELD_TYPES: ReadonlySet<SketchConstraint["type"]> = new Set([
  "Coincident",
  "Midpoint",
  "OnCurve",
  "Fixed",
]);

/** How a constraint differs from the previous upsert's set (module header). */
type DeltaKind = "created" | "edited";

/**
 * The delta: every constraint whose id is new, or whose `value` changed since
 * `prevConstraints`. Ids that DISAPPEARED are absent — a removal has nothing to
 * drive. `prevConstraints` is empty on the very first upsert of a session, which
 * makes every constraint CREATED (drivable ones drive, the rest are accepted).
 */
function constraintDelta(
  constraints: SketchConstraint[],
  prevConstraints: SketchConstraint[],
): Map<string, DeltaKind> {
  const prevValues = new Map<string, number | undefined>();
  for (const p of prevConstraints) prevValues.set(p.id, p.value);
  const delta = new Map<string, DeltaKind>();
  for (const c of constraints) {
    if (!prevValues.has(c.id)) delta.set(c.id, "created");
    else if (prevValues.get(c.id) !== c.value) delta.set(c.id, "edited");
  }
  return delta;
}

/** True if ANY weld-type constraint references `entityId` at all (any point
 *  slot) — a blanket, entity-level check: the mock has no way to know whether
 *  a specific slot survives a move, only that SOMETHING elsewhere may depend
 *  on this entity's points. */
function isWelded(entityId: string, constraints: SketchConstraint[]): boolean {
  return constraints.some((c) => WELD_TYPES.has(c.type) && c.entities.includes(entityId));
}

/** Current plane coord of a point ref — Point/Line/Circle/Ellipse/Arc, mirrors
 *  `mockConflicts.ts`'s local `pointCoord` (kept local here too, same reason:
 *  this pure mock module carries no cross-module import). Used only to decide
 *  whether a dimensional constraint's value already MATCHES current geometry —
 *  general-shape, unlike the narrow shape `enforceDriving` actually drives. */
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

/**
 * The narrow drivable shape for Distance/HorizontalDistance/VerticalDistance:
 * both point refs are the SAME Line entity's Start and End (order-independent).
 * Returns that Line, or null when the shape is anything else (point↔line,
 * line↔line, a whole-body single-ref Distance, …) — those are never driven,
 * only ever matched, accepted-on-creation or refused-on-edit.
 */
function sameLineEndpoints(entities: SketchEntity[], c: SketchConstraint): SketchEntity | null {
  if (c.entities.length !== 2 || c.entities[0] !== c.entities[1]) return null;
  const positions = c.positions ?? [];
  if (positions.length !== 2) return null;
  const set = new Set(positions);
  if (set.size !== 2 || !set.has("Start") || !set.has("End")) return null;
  const e = entities.find((x) => x.id === c.entities[0]);
  return e && e.type === "Line" ? e : null;
}

/** Unsigned angle (degrees, [0,180]) between two line directions as drawn —
 *  mirrors `dimensionTool.ts`'s `angleBetweenDeg` (kept local, same reason as
 *  `pointCoord` above). Used ONLY to detect a mismatch; Angle is never driven. */
function angleBetweenDeg(a0: [number, number], a1: [number, number], b0: [number, number], b1: [number, number]): number {
  const a = Math.atan2(a1[1] - a0[1], a1[0] - a0[0]);
  const b = Math.atan2(b1[1] - b0[1], b1[0] - b0[0]);
  let d = Math.abs(a - b);
  while (d > 2 * Math.PI) d -= 2 * Math.PI;
  if (d > Math.PI) d = 2 * Math.PI - d; // fold to [0, π]
  return (d * 180) / Math.PI;
}

/** Result of one `enforceDriving` pass — `entities` mirrors `applySketchSolveResult`'s
 *  contract: the SAME array reference when nothing moved. */
export interface EnforceResult {
  entities: SketchEntity[];
  /** Absolute new point coordinates, keyed `"entityId.Start"`/`"entityId.End"` —
   *  the exact `SketchUpsertResult.solvedPositions` shape (frontend-domain
   *  keys; the mock lane needs no wire re-keying). */
  solvedPositions: Record<string, [number, number]>;
  /** Absolute new curve params, keyed by entity id — `SketchUpsertResult.solvedCurves`. */
  solvedCurves: Record<string, CurveParams>;
  /** Ids of EDITED dimensional constraints whose new value could not be driven
   *  (guard failure or unsupported shape) — the caller folds these into
   *  `conflicting`/`status: "Conflicting"`, the same signal shape
   *  `mockConflicts.ts` R1–R5 already use, so the caller reverts the edit
   *  instead of showing a label the geometry disagrees with. A CREATED
   *  constraint is never listed here (module header decision table). */
  refusedIds: string[];
}

function moveLine(
  working: SketchEntity[],
  lineId: string,
  p0: [number, number],
  p1: [number, number],
  solvedPositions: Record<string, [number, number]>,
): SketchEntity[] {
  solvedPositions[`${lineId}.Start`] = p0;
  solvedPositions[`${lineId}.End`] = p1;
  return working.map((e) => (e.id === lineId ? { ...e, p0, p1 } : e));
}

/**
 * Drive the DELTA between `prevConstraints` and `constraints` onto `entities`.
 * Pure; deterministic; a strict identity when the delta is empty. Callers MUST
 * have already checked `detectConflicts` — this never runs on an already-
 * Conflicting sketch (module header + `localSolver.ts`'s `sketchUpsert`).
 */
export function enforceDriving(
  entities: SketchEntity[],
  constraints: SketchConstraint[],
  prevConstraints: SketchConstraint[],
): EnforceResult {
  const solvedPositions: Record<string, [number, number]> = {};
  const solvedCurves: Record<string, CurveParams> = {};
  const refusedIds: string[] = [];
  const delta = constraintDelta(constraints, prevConstraints);
  let working = entities;

  // Order step 1: newly CREATED Horizontal/Vertical — an unconditional geometric
  // projection, no `.value` to compare, never a refusal (a guard failure just
  // leaves the tilt in place). An UNCHANGED H/V is not re-projected: after a
  // drag moved the line the mock has no way to tell an intended move from drift,
  // and re-projecting would fight the drag every upsert.
  for (const c of constraints) {
    if (c.type !== "Horizontal" && c.type !== "Vertical") continue;
    if (delta.get(c.id) !== "created") continue;
    const lineId = c.entities[0];
    if (!lineId) continue;
    const line = working.find((e) => e.id === lineId && e.type === "Line");
    if (!line || !line.p0 || !line.p1 || line.referenceLocked || isWelded(lineId, constraints)) continue;
    const [p0, p1] = [line.p0, line.p1];
    if (c.type === "Horizontal") {
      if (Math.abs(p0[1] - p1[1]) <= EPS) continue; // already exact
      const my = (p0[1] + p1[1]) / 2;
      working = moveLine(working, lineId, [p0[0], my], [p1[0], my], solvedPositions);
    } else {
      if (Math.abs(p0[0] - p1[0]) <= EPS) continue; // already exact
      const mx = (p0[0] + p1[0]) / 2;
      working = moveLine(working, lineId, [mx, p0[1]], [mx, p1[1]], solvedPositions);
    }
  }

  // Order step 2: the delta's dimensional constraints, in constraint-array order.
  for (const c of constraints) {
    const kind = delta.get(c.id);
    if (!kind) continue; // unchanged — never measured, never refused
    if (c.value === undefined) continue; // H/V handled above; nothing else to drive
    switch (c.type) {
      case "Radius":
      case "Diameter": {
        const entity = working.find((e) => e.id === c.entities[0]);
        if (!entity || entity.radius === undefined) continue; // unmeasurable
        const target = c.type === "Diameter" ? c.value / 2 : c.value;
        if (Math.abs(entity.radius - target) <= EPS) continue; // already matches
        // Arcs are excluded on purpose: `start`/`end` are stored coordinates
        // that a radius rewrite would leave off the new stroke (module header).
        if (entity.type !== "Circle" || entity.referenceLocked) {
          if (kind === "edited") refusedIds.push(c.id);
          continue;
        }
        working = working.map((e) => (e.id === entity.id ? { ...e, radius: target } : e));
        solvedCurves[entity.id] = { ...(solvedCurves[entity.id] ?? {}), radius: target };
        break;
      }
      case "Distance":
      case "HorizontalDistance":
      case "VerticalDistance": {
        const a = pointCoord(working, c.entities[0], c.positions?.[0]);
        const b = pointCoord(working, c.entities[1], c.positions?.[1]);
        if (!a || !b) continue; // unmeasurable
        const measured =
          c.type === "Distance"
            ? Math.hypot(b[0] - a[0], b[1] - a[1])
            : c.type === "HorizontalDistance"
              ? b[0] - a[0]
              : b[1] - a[1];
        if (Math.abs(measured - c.value) <= EPS) continue; // already matches

        const line = sameLineEndpoints(working, c);
        if (!line || !line.p0 || !line.p1 || line.referenceLocked || isWelded(line.id, constraints)) {
          if (kind === "edited") refusedIds.push(c.id);
          continue;
        }
        const mid: [number, number] = [(line.p0[0] + line.p1[0]) / 2, (line.p0[1] + line.p1[1]) / 2];
        if (c.type === "Distance") {
          const dx = line.p1[0] - line.p0[0];
          const dy = line.p1[1] - line.p0[1];
          const len = Math.hypot(dx, dy);
          if (len <= EPS) {
            // degenerate (zero-length) — no direction to scale along
            if (kind === "edited") refusedIds.push(c.id);
            continue;
          }
          const ux = dx / len;
          const uy = dy / len;
          const half = c.value / 2;
          working = moveLine(
            working,
            line.id,
            [mid[0] - ux * half, mid[1] - uy * half],
            [mid[0] + ux * half, mid[1] + uy * half],
            solvedPositions,
          );
        } else if (c.type === "HorizontalDistance") {
          const half = c.value / 2;
          working = moveLine(
            working,
            line.id,
            [mid[0] - half, line.p0[1]],
            [mid[0] + half, line.p1[1]],
            solvedPositions,
          );
        } else {
          const half = c.value / 2;
          working = moveLine(
            working,
            line.id,
            [line.p0[0], mid[1] - half],
            [line.p1[0], mid[1] + half],
            solvedPositions,
          );
        }
        break;
      }
      case "Angle": {
        // Never driven (no provable rule for it) — but an EDIT that can't be
        // honoured refuses instead of silently showing a label the geometry
        // disagrees with.
        const la = working.find((e) => e.id === c.entities[0]);
        const lb = working.find((e) => e.id === c.entities[1]);
        if (!la || !lb || la.type !== "Line" || lb.type !== "Line" || !la.p0 || !la.p1 || !lb.p0 || !lb.p1) {
          continue; // unmeasurable
        }
        const measured = angleBetweenDeg(la.p0, la.p1, lb.p0, lb.p1);
        if (Math.abs(measured - c.value) > EPS && kind === "edited") refusedIds.push(c.id);
        break;
      }
      default:
        continue; // not this module's concern
    }
  }

  return { entities: working, solvedPositions, solvedCurves, refusedIds };
}
