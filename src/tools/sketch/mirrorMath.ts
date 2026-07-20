/*
 * mirrorMath — PURE reflection math + mirror-entity builders for the sketch Mirror
 * tool (ports OneCAD-CPP MirrorTool.cpp `mirrorPoint` + `createMirroredEntity`).
 *
 * A mirror reflects each source entity across the axis line a–b and, for the kinds
 * whose points are real solver handles, ties the copy to its source with SYMMETRIC
 * (and, for circles, EQUAL) constraints so a later solve keeps them mirrored.
 *
 * ── Entity matrix (V1; the constraint column mirrors what the solver can express) ─
 *   Point  → Point  + Symmetric{original.Start ↔ mirror.Start, axis}
 *   Line   → Line   + Symmetric{Start↔Start} + Symmetric{End↔End}   (both across axis)
 *   Circle → Circle + Symmetric{Center↔Center} + Equal{original, mirror}  (radius lock)
 *   Arc    → Arc    COPY ONLY — no constraints. An arc's Start/End are DERIVED from
 *            center + angles (not draggable solver points; see selectGesture's
 *            `isDraggableHandle`), so there is no point id to make symmetric — a
 *            documented V1 seam, matching sketchWireMap's arc-endpoint limitation.
 *
 * ── Winding ──────────────────────────────────────────────────────────────────────
 * Frontend arcs sweep CCW from `start` to `end` (snapEngine.arcContainsAngle /
 * arcTool). Reflection REVERSES winding, so the reflected arc's CCW start is the
 * reflected OLD end (and its CCW end is the reflected OLD start) — we swap the two
 * when building the copy. Reflecting the endpoint COORDS already yields the correct
 * angles (a point at angle θ about the old center maps to 2φ−θ about the reflected
 * center, φ = axis angle), so no explicit angle arithmetic is needed.
 *
 * New ids are minted by injected minters (the controller passes sketchStore's
 * `nextEntityId` / `nextConstraintId`, the same convention the draw tools use on
 * commit) so this module stays free of store access and is unit-tested by data.
 */
import type { SketchConstraint, SketchEntity } from "@/ipc/types";

export type XY = [number, number];

/**
 * Reflect point `p` across the infinite line through `a` and `b`
 * (`2·proj − p`). A degenerate axis (a ≈ b) reflects to `p` itself.
 */
export function reflectPoint(p: XY, a: XY, b: XY): XY {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const lenSq = dx * dx + dy * dy;
  if (lenSq < 1e-12) return [p[0], p[1]]; // degenerate axis → identity
  const t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / lenSq;
  const projX = a[0] + t * dx;
  const projY = a[1] + t * dy;
  return [2 * projX - p[0], 2 * projY - p[1]];
}

/** Id minters for the mirrored artifacts (kept out of the store for purity). */
export interface MirrorMinters {
  entityId: () => string;
  constraintId: () => string;
}

/** One mirrored entity plus the constraints tying it to its source (may be empty). */
export interface MirroredEntity {
  entity: SketchEntity;
  constraints: SketchConstraint[];
}

/**
 * Mirror ONE source entity across the axis line `a`–`b` (see the module matrix).
 * `axisId` is the mirror line's entity id (the third ref of every Symmetric).
 * Returns null for an unmappable entity (missing coords / unsupported type).
 */
export function mirrorEntity(
  src: SketchEntity,
  axisId: string,
  a: XY,
  b: XY,
  m: MirrorMinters,
): MirroredEntity | null {
  const construction = src.construction ? { construction: true as const } : {};
  const R = (p: XY): XY => reflectPoint(p, a, b);
  const sym = (p1: "Start" | "End" | "Center", newId: string): SketchConstraint => ({
    id: m.constraintId(),
    type: "Symmetric",
    entities: [src.id, newId, axisId],
    positions: [p1, p1],
  });

  switch (src.type) {
    case "Point": {
      if (!src.p0) return null;
      const id = m.entityId();
      return {
        entity: { id, type: "Point", p0: R(src.p0), ...construction },
        constraints: [sym("Start", id)],
      };
    }
    case "Line": {
      if (!src.p0 || !src.p1) return null;
      const id = m.entityId();
      // Endpoints reflect in place (Start→Start, End→End) — no winding to flip.
      return {
        entity: { id, type: "Line", p0: R(src.p0), p1: R(src.p1), ...construction },
        constraints: [sym("Start", id), sym("End", id)],
      };
    }
    case "Circle": {
      if (!src.center || src.radius === undefined) return null;
      const id = m.entityId();
      return {
        entity: { id, type: "Circle", center: R(src.center), radius: src.radius, ...construction },
        constraints: [
          sym("Center", id),
          { id: m.constraintId(), type: "Equal", entities: [src.id, id] },
        ],
      };
    }
    case "Arc": {
      if (!src.center || src.radius === undefined || !src.start || !src.end) return null;
      const id = m.entityId();
      // Reflection reverses winding: swap start/end so the copy still sweeps CCW.
      return {
        entity: {
          id,
          type: "Arc",
          center: R(src.center),
          radius: src.radius,
          start: R(src.end),
          end: R(src.start),
          ...construction,
        },
        constraints: [], // arc endpoints aren't solver handles — copy only (V1 seam)
      };
    }
    default:
      return null;
  }
}

/**
 * Mirror a set of source entities across the axis line `a`–`b`, aggregating all new
 * entities + their constraints (source order preserved). Callers pass DISTINCT
 * entities (the axis itself excluded). Unmappable entities are skipped.
 */
export function mirrorEntities(
  sources: SketchEntity[],
  axisId: string,
  a: XY,
  b: XY,
  m: MirrorMinters,
): { entities: SketchEntity[]; constraints: SketchConstraint[] } {
  const entities: SketchEntity[] = [];
  const constraints: SketchConstraint[] = [];
  for (const src of sources) {
    const r = mirrorEntity(src, axisId, a, b, m);
    if (!r) continue;
    entities.push(r.entity);
    constraints.push(...r.constraints);
  }
  return { entities, constraints };
}
