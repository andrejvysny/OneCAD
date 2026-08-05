/*
 * Auto-constraint inference (PURE) — ports OneCAD-CPP `AutoConstrainer` semantics
 * (NEW_SPEC §14 interaction model). On commit the tools infer:
 *
 *   - Horizontal / Vertical: a line within ±5° of an axis (verbatim tolerance
 *     from AutoConstrainer.h `horizontalTolerance = 5° in radians`).
 *   - Coincident: a new endpoint that lands ON an existing endpoint (snap-to-
 *     endpoint). C++ uses a 2mm proximity; our tools SNAP endpoints exactly, so
 *     the default match tolerance is tight (1e-6). The mock solver is an identity
 *     solve — it will not pull non-coincident points together — so coincidence is
 *     only inferred where snapping already made the points equal.
 *   - Perpendicular: a NON-axis line meeting a reference line at 90±5°
 *     (AutoConstrainer.h `perpendicularTolerance = 5°`). Gated behind H/V exactly
 *     like C++ `inferLineConstraints` (`hasOrientationConstraint` blocks it).
 *   - Parallel: a NON-axis line within ±5° of a reference line's direction
 *     (`parallelTolerance = 5°`). Also gated behind H/V — H/V wins over Parallel,
 *     matching the legacy precedence.
 *   - Tangent: an ARC whose START sits on a reference line's endpoint (within the
 *     2mm `coincidenceTolerance`) and whose start tangent aligns with the line to
 *     within ±5° (`tangentTolerance`). This is the exact legacy rule — the C++
 *     `inferTangent` only handles arc-start-tangent-to-line. (Line↔circle and
 *     arc↔arc tangency exist as SNAPS, not as legacy auto-constraints — seam.)
 *     DIVERGENCE FROM LEGACY: skipped when this same batch already emitted a
 *     Coincident between the SAME two entities — see `weldedTo`.
 *   - Concentric: a new circle/arc's CENTER within `CONCENTRIC_TOLERANCE` (2mm,
 *     `coincidenceTolerance`, AutoConstrainer.h:61) of an existing circle/arc's
 *     center, nearest wins (`AutoConstrainer::inferConcentric`,
 *     AutoConstrainer.cpp:645). DIVERGENCE FROM LEGACY: skipped when this same
 *     batch already emitted a Coincident between the SAME two entities' Center
 *     points — our tools SNAP centers exactly (unlike legacy's identity solve),
 *     so that Coincident already zeroes the gap; both constraints would each
 *     remove 2 DOF in the mock's coarse count (`mockSketch.ts constraintFreedom`)
 *     for what is really one 2-DOF relation, producing a false OverConstrained
 *     the real solver would never report.
 *   - Equal: a new circle/arc's RADIUS within `EQUAL_RADIUS_TOLERANCE` (0.5mm,
 *     function-local `constexpr RADIUS_TOLERANCE`, AutoConstrainer.cpp:699) of an
 *     existing circle/arc's radius, nearest wins (`AutoConstrainer::inferEqualRadius`,
 *     AutoConstrainer.cpp:694). Circles and arcs only — an Ellipse has no single
 *     scalar radius and contributes to neither Concentric nor Equal (see
 *     `entityPoints`).
 *
 * A line is never both Horizontal and Vertical, and never gets Perpendicular /
 * Parallel once it is H or V. Intra-batch relationships (e.g. a polyline's chained
 * segments) are detected by folding each processed entity into the reference sets
 * as we go.
 */
import type {
  ConstraintPosition,
  SketchConstraint,
  SketchEntity,
} from "@/ipc/types";

/** ±5° in radians (AutoConstrainer.h default; shared by H/V/perp/parallel/tangent). */
export const HV_TOLERANCE_RAD = (5 * Math.PI) / 180;
/** perpendicularTolerance = 90±5° (AutoConstrainer.h). */
export const PERPENDICULAR_TOLERANCE_RAD = (5 * Math.PI) / 180;
/** parallelTolerance = ±5° (AutoConstrainer.h). */
export const PARALLEL_TOLERANCE_RAD = (5 * Math.PI) / 180;
/** tangentTolerance = ±5° (AutoConstrainer.h). */
export const TANGENT_TOLERANCE_RAD = (5 * Math.PI) / 180;
/** coincidenceTolerance = 2mm — used to detect an arc START on a line endpoint. */
export const TANGENT_COINCIDENCE_TOL = 2.0;
/** MIN_GEOMETRY_SIZE = 0.01mm (SketchTypes.h) — skip degenerate lines. */
export const MIN_GEOMETRY_SIZE = 0.01;
/** coincidenceTolerance = 2mm (AutoConstrainer.h:61) — reused for Concentric center matching. */
export const CONCENTRIC_TOLERANCE = 2.0;
/** function-local `constexpr RADIUS_TOLERANCE = 0.5` (AutoConstrainer.cpp:699). */
export const EQUAL_RADIUS_TOLERANCE = 0.5;

export interface InferOptions {
  angleToleranceRad?: number;
  coincidenceTol?: number;
  /** Id minter for the emitted constraints. */
  nextConstraintId: () => string;
}

interface EntPoint {
  entityId: string;
  position: ConstraintPosition;
  coord: [number, number];
}

/** A reference line for perpendicular / parallel / tangent inference. */
interface RefLine {
  id: string;
  p0: [number, number];
  p1: [number, number];
}

/** A reference circle/arc for concentric / equal-radius inference. */
interface RefCircular {
  id: string;
  center: [number, number];
  radius: number;
}

/** Constrainable points of an entity (Start/End/Center), for coincidence. */
export function entityPoints(e: SketchEntity): EntPoint[] {
  const out: EntPoint[] = [];
  if (e.type === "Point" && e.p0) out.push({ entityId: e.id, position: "Start", coord: e.p0 });
  if (e.type === "Line") {
    if (e.p0) out.push({ entityId: e.id, position: "Start", coord: e.p0 });
    if (e.p1) out.push({ entityId: e.id, position: "End", coord: e.p1 });
  }
  // An Ellipse contributes its CENTRE exactly like a Circle — that centre IS a
  // minted wire point (`sketchWireMap.addEntityOps`), so it selects, drags and
  // auto-coincides for free. Its CURVE contributes nothing (legacy parity) — nor
  // does it have a single scalar radius, so it feeds neither Concentric nor Equal
  // (`inferConcentricPartner`/`inferEqualRadiusPartner` take Circle/Arc only).
  if ((e.type === "Circle" || e.type === "Ellipse") && e.center)
    out.push({ entityId: e.id, position: "Center", coord: e.center });
  if (e.type === "Arc") {
    if (e.center) out.push({ entityId: e.id, position: "Center", coord: e.center });
    if (e.start) out.push({ entityId: e.id, position: "Start", coord: e.start });
    if (e.end) out.push({ entityId: e.id, position: "End", coord: e.end });
  }
  return out;
}

/** Angle of a line to +X in [-π, π]. */
export function lineAngle(p0: [number, number], p1: [number, number]): number {
  return Math.atan2(p1[1] - p0[1], p1[0] - p0[0]);
}

const dist2 = (a: [number, number], b: [number, number]): number => Math.hypot(a[0] - b[0], a[1] - b[1]);

/** Normalize an angle to [-π, π]. */
function normalizeAngle(a: number): number {
  while (a > Math.PI) a -= 2 * Math.PI;
  while (a < -Math.PI) a += 2 * Math.PI;
  return a;
}

/**
 * Angle between two lines, folded to [0, π/2] (verbatim from
 * `AutoConstrainer::angleBetweenLines`: `min(|Δ|, π-|Δ|)`). 0 ⇒ parallel,
 * π/2 ⇒ perpendicular.
 */
export function angleBetweenLines(
  a0: [number, number],
  a1: [number, number],
  b0: [number, number],
  b1: [number, number],
): number {
  const diff = Math.abs(normalizeAngle(lineAngle(a0, a1) - lineAngle(b0, b1)));
  return Math.min(diff, Math.PI - diff);
}

/** Horizontal (`H`), Vertical (`V`) or null for a line within the tolerance. */
export function inferHV(
  p0: [number, number],
  p1: [number, number],
  tol = HV_TOLERANCE_RAD,
): "Horizontal" | "Vertical" | null {
  if (p0[0] === p1[0] && p0[1] === p1[1]) return null; // zero-length
  const a = lineAngle(p0, p1);
  const hDev = Math.min(Math.abs(a), Math.abs(Math.abs(a) - Math.PI));
  if (hDev <= tol) return "Horizontal";
  const vDev = Math.abs(Math.abs(a) - Math.PI / 2);
  if (vDev <= tol) return "Vertical";
  return null;
}

/**
 * Best perpendicular reference line for `line`: the one meeting it at 90±tol with
 * the smallest deviation from 90° (mirrors `inferPerpendicular`'s best-deviation
 * scan). Returns the partner id or null.
 */
export function inferPerpendicularPartner(
  p0: [number, number],
  p1: [number, number],
  refs: RefLine[],
  tol = PERPENDICULAR_TOLERANCE_RAD,
): string | null {
  let best: string | null = null;
  let bestDev = Infinity;
  for (const r of refs) {
    if (dist2(r.p0, r.p1) < MIN_GEOMETRY_SIZE) continue;
    const dev = Math.abs(angleBetweenLines(p0, p1, r.p0, r.p1) - Math.PI / 2);
    if (dev <= tol && dev < bestDev) {
      bestDev = dev;
      best = r.id;
    }
  }
  return best;
}

/**
 * Best parallel reference line for `line`: the one whose direction is within tol
 * (folded angle near 0). Returns the partner id or null. (`inferParallel`.)
 */
export function inferParallelPartner(
  p0: [number, number],
  p1: [number, number],
  refs: RefLine[],
  tol = PARALLEL_TOLERANCE_RAD,
): string | null {
  let best: string | null = null;
  let bestDev = Infinity;
  for (const r of refs) {
    if (dist2(r.p0, r.p1) < MIN_GEOMETRY_SIZE) continue;
    // angleBetweenLines is already folded to [0, π/2]; near 0 ⇒ parallel.
    const dev = angleBetweenLines(p0, p1, r.p0, r.p1);
    if (dev <= tol && dev < bestDev) {
      bestDev = dev;
      best = r.id;
    }
  }
  return best;
}

/**
 * Tangent partner line for an arc: a reference line whose endpoint the arc START
 * sits on (within 2mm) and whose direction aligns with the arc's start tangent to
 * within tol (verbatim from `AutoConstrainer::inferTangent`). Returns the line id
 * or null.
 */
export function inferTangentPartner(
  center: [number, number],
  start: [number, number],
  refs: RefLine[],
  tol = TANGENT_TOLERANCE_RAD,
): string | null {
  // Arc start tangent = perpendicular to the radial (CCW rotation), normalized.
  const rad: [number, number] = [start[0] - center[0], start[1] - center[1]];
  const radLen = Math.hypot(rad[0], rad[1]);
  if (radLen < 1e-12) return null;
  const tangent: [number, number] = [-rad[1] / radLen, rad[0] / radLen];

  let best: string | null = null;
  let bestDev = Infinity;
  for (const r of refs) {
    const dStart = dist2(start, r.p0);
    const dEnd = dist2(start, r.p1);
    let lineDir: [number, number];
    if (dStart < TANGENT_COINCIDENCE_TOL) {
      lineDir = [r.p0[0] - r.p1[0], r.p0[1] - r.p1[1]];
    } else if (dEnd < TANGENT_COINCIDENCE_TOL) {
      lineDir = [r.p1[0] - r.p0[0], r.p1[1] - r.p0[1]];
    } else {
      continue; // arc does not start at either endpoint of this line
    }
    const len = Math.hypot(lineDir[0], lineDir[1]);
    if (len < 1e-12) continue;
    const dot = Math.abs((lineDir[0] / len) * tangent[0] + (lineDir[1] / len) * tangent[1]);
    if (dot > Math.cos(tol)) {
      const dev = Math.acos(Math.min(1, dot));
      if (dev < bestDev) {
        bestDev = dev;
        best = r.id;
      }
    }
  }
  return best;
}

/**
 * Nearest concentric partner for a circle/arc CENTER: the closest existing
 * circle/arc center within tol (verbatim from `AutoConstrainer::inferConcentric`
 * — the running best is SEEDED at `tol` itself, so a candidate must beat it with
 * a strict `<`; a center exactly `tol` away never fires). Returns the partner id
 * or null.
 */
export function inferConcentricPartner(
  center: [number, number],
  refs: RefCircular[],
  tol = CONCENTRIC_TOLERANCE,
): string | null {
  let best: string | null = null;
  let bestDist = tol;
  for (const r of refs) {
    const d = dist2(center, r.center);
    if (d < bestDist) {
      bestDist = d;
      best = r.id;
    }
  }
  return best;
}

/**
 * Nearest equal-radius partner for a circle/arc RADIUS: the closest existing
 * circle/arc radius within tol (verbatim from `AutoConstrainer::inferEqualRadius`
 * — running best seeded at `tol`, strict `<`; a radius delta of exactly `tol`
 * never fires). Returns the partner id or null.
 */
export function inferEqualRadiusPartner(
  radius: number,
  refs: RefCircular[],
  tol = EQUAL_RADIUS_TOLERANCE,
): string | null {
  let best: string | null = null;
  let bestDiff = tol;
  for (const r of refs) {
    const diff = Math.abs(radius - r.radius);
    if (diff < bestDiff) {
      bestDiff = diff;
      best = r.id;
    }
  }
  return best;
}

/**
 * Is this batch already WELDING the arc to the line — a Coincident naming both?
 *
 * THE WELD GATE (divergence from legacy, same shape as the Concentric one above).
 * An entity-level `Tangent(line, arc)` is `distance(arcCenter, line) == radius`.
 * Once an arc ENDPOINT is coincident with the line's endpoint, the line already
 * passes through a point exactly `radius` from the centre, so that distance sits
 * at its MAXIMUM: the gradient vanishes on the manifold of the weld and PlaneGCS
 * diagnoses the Tangent as redundant, which `upsert_state` renders as a false
 * OverConstrained. Pinned by the worker ctest
 * `test_sketch_arc_endpoints.cpp::test_endpoint_weld_makes_entity_tangent_redundant`;
 * the same reasoning is why `slotTool` removed its four entity-level Tangents
 * (toolMachine.ts slot header) and why `filletSketchCorner` authors none
 * (sketchService.ts). Legacy could not hit this — its identity solve never made
 * the endpoints actually equal, so the weld was never exact enough to be
 * redundant; our tools SNAP them.
 *
 * Position-blind on purpose: ANY weld between the two entities makes the entity-
 * level Tangent redundant, whichever endpoints it married.
 */
function weldedTo(out: SketchConstraint[], arcId: string, lineId: string): boolean {
  return out.some(
    (c) =>
      c.type === "Coincident" &&
      ((c.entities[0] === arcId && c.entities[1] === lineId) ||
        (c.entities[0] === lineId && c.entities[1] === arcId)),
  );
}

/**
 * Emit Concentric then Equal for one committed Circle/Arc `e` (LEGACY ORDER —
 * `inferCircleConstraints`/`inferArcConstraints`), applying the Coincident
 * divergence gate (see header comment). Mutates `out`; `refCircular` is the
 * running reference set (existing + already-processed batch entities, NOT `e`).
 */
function inferCircularConstraints(
  e: SketchEntity,
  center: [number, number],
  radius: number,
  refCircular: RefCircular[],
  out: SketchConstraint[],
  nextConstraintId: () => string,
): void {
  const concentricPartner = inferConcentricPartner(center, refCircular);
  if (concentricPartner) {
    const centerCoincidentAlready = out.some(
      (c) =>
        c.type === "Coincident" &&
        c.entities[0] === e.id &&
        c.entities[1] === concentricPartner &&
        c.positions?.[0] === "Center" &&
        c.positions?.[1] === "Center",
    );
    if (!centerCoincidentAlready) {
      out.push({ id: nextConstraintId(), type: "Concentric", entities: [e.id, concentricPartner] });
    }
  }
  const equalPartner = inferEqualRadiusPartner(radius, refCircular);
  if (equalPartner) out.push({ id: nextConstraintId(), type: "Equal", entities: [e.id, equalPartner] });
}

export function inferConstraints(
  newEntities: SketchEntity[],
  existing: SketchEntity[],
  opts: InferOptions,
): SketchConstraint[] {
  const tol = opts.angleToleranceRad ?? HV_TOLERANCE_RAD;
  const coincTol = opts.coincidenceTol ?? 1e-6;
  const out: SketchConstraint[] = [];

  // Reference points + lines grow as we process each new entity (intra-batch).
  const refs: EntPoint[] = existing.flatMap(entityPoints);
  const refLines: RefLine[] = existing
    .filter((e) => e.type === "Line" && e.p0 && e.p1)
    .map((e) => ({ id: e.id, p0: e.p0!, p1: e.p1! }));
  const refCircular: RefCircular[] = existing
    .filter((e) => (e.type === "Circle" || e.type === "Arc") && e.center && e.radius !== undefined)
    .map((e) => ({ id: e.id, center: e.center!, radius: e.radius! }));
  const seenPairs = new Set<string>();

  for (const e of newEntities) {
    // H/V for lines; if neither fires, try Perpendicular then Parallel (never both
    // when a line is already H/V — the legacy `hasOrientationConstraint` gate).
    if (e.type === "Line" && e.p0 && e.p1) {
      const hv = inferHV(e.p0, e.p1, tol);
      if (hv) {
        out.push({ id: opts.nextConstraintId(), type: hv, entities: [e.id] });
      } else if (dist2(e.p0, e.p1) >= MIN_GEOMETRY_SIZE) {
        const perp = inferPerpendicularPartner(e.p0, e.p1, refLines);
        if (perp) out.push({ id: opts.nextConstraintId(), type: "Perpendicular", entities: [e.id, perp] });
        const par = inferParallelPartner(e.p0, e.p1, refLines);
        if (par) out.push({ id: opts.nextConstraintId(), type: "Parallel", entities: [e.id, par] });
      }
    }

    // Coincidence for each of this entity's points against the reference set.
    for (const pt of entityPoints(e)) {
      const hit = refs.find(
        (r) => r.entityId !== e.id && Math.hypot(r.coord[0] - pt.coord[0], r.coord[1] - pt.coord[1]) <= coincTol,
      );
      if (hit) {
        const pairKey = [`${e.id}.${pt.position}`, `${hit.entityId}.${hit.position}`].sort().join("|");
        if (!seenPairs.has(pairKey)) {
          seenPairs.add(pairKey);
          out.push({
            id: opts.nextConstraintId(),
            type: "Coincident",
            entities: [e.id, hit.entityId],
            positions: [pt.position, hit.position],
          });
        }
      }
    }

    // Tangent for an arc starting on a reference line's endpoint. Runs BELOW the
    // Coincident loop — not for ordering's sake but because the weld gate reads
    // the Coincidents this entity just emitted (`weldedTo`).
    if (e.type === "Arc" && e.center && e.start) {
      const tangentLine = inferTangentPartner(e.center, e.start, refLines);
      if (tangentLine && !weldedTo(out, e.id, tangentLine)) {
        out.push({ id: opts.nextConstraintId(), type: "Tangent", entities: [e.id, tangentLine] });
      }
    }

    // Concentric / Equal for a committed Circle or Arc (Ellipse contributes
    // neither — see `entityPoints`). Runs after Tangent (just above) and
    // Coincident, satisfying both legacy orderings in one pass.
    if ((e.type === "Circle" || e.type === "Arc") && e.center && e.radius !== undefined) {
      inferCircularConstraints(e, e.center, e.radius, refCircular, out, opts.nextConstraintId);
    }

    refs.push(...entityPoints(e));
    if (e.type === "Line" && e.p0 && e.p1) refLines.push({ id: e.id, p0: e.p0, p1: e.p1 });
    if ((e.type === "Circle" || e.type === "Arc") && e.center && e.radius !== undefined)
      refCircular.push({ id: e.id, center: e.center, radius: e.radius });
  }

  return out;
}
