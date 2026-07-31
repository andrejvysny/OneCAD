/*
 * constraintTarget — PURE selection→target adapter for the sketch constraint-apply
 * UI (S4b design doc). Later WPs drive constraint authoring from a selection of
 * raw refs (`{entityId, point?}`) against the session's `SketchEntity[]`; this
 * module is the shared vocabulary between hit-test, applicability
 * (`constraintApplicability.ts`), and authoring — resolve a ref ONCE here, then
 * pass the resulting `SketchConstraintTarget` everywhere downstream.
 *
 * No wiring to stores/engine/ipc client here — only TYPE imports from the
 * existing `@/ipc/types` module (the frontend entity/position vocabulary).
 */
import type { ConstraintPosition, SketchEntity, SketchEntityType } from "@/ipc/types";

/** One raw selection ref the hit-test layer emits: an entity id + an optional
 *  named point on it (Line Start/End, Circle/Arc Center, Arc Start/End, Line
 *  Midpoint). Omitted `point` means "the whole entity body" (a Line/Circle/Arc),
 *  EXCEPT for a Point entity, which is always itself a point. */
export interface SketchSelectionRef {
  entityId: string;
  point?: ConstraintPosition;
}

/** A resolved point target — either a standalone Point entity (`isFreePoint`)
 *  or a named point ON a Line/Circle/Arc (its Start/End/Center/Midpoint). */
export interface SketchConstraintTargetPoint {
  kind: "point";
  entityId: string;
  position: ConstraintPosition;
  /** true when `entityId` is itself a standalone Point entity, not a virtual
   *  point derived from a Line/Circle/Arc's own Start/End/Center/Midpoint. */
  isFreePoint: boolean;
}
export interface SketchConstraintTargetLine {
  kind: "line";
  entityId: string;
}
export interface SketchConstraintTargetCircle {
  kind: "circle";
  entityId: string;
}
export interface SketchConstraintTargetArc {
  kind: "arc";
  entityId: string;
}
/** An ellipse CURVE body pick. Resolved (rather than dropped) so
 *  `evaluateApplicability` can see it and offer NOTHING — legacy parity, see
 *  `worker/tests/prototypes/proto_sketch_constraint_applicability.cpp:62-67`, where
 *  point+ellipse has no applicable constraints at all (not even OnCurve). Dropping
 *  it instead would silently degrade a 2-pick selection to a 1-pick one and start
 *  offering the OTHER pick's single-target constraints. */
export interface SketchConstraintTargetEllipse {
  kind: "ellipse";
  entityId: string;
}

/** The shared target vocabulary (selection ref + entity lookup, resolved). */
export type SketchConstraintTarget =
  | SketchConstraintTargetPoint
  | SketchConstraintTargetLine
  | SketchConstraintTargetCircle
  | SketchConstraintTargetArc
  | SketchConstraintTargetEllipse;

/** Named positions each entity type resolves (mirrors `entityPoints()` in
 *  autoConstrain.ts, plus Line's virtual "Midpoint" for the Midpoint constraint's
 *  point ref). A Point entity's Start/Center are aliases of the SAME coordinate
 *  (sketchWireMap.ts `mintPoint` seeds both keys for a Point entity) — normalized
 *  to "Start" below rather than kept distinct. */
const VALID_POSITIONS: Record<SketchEntityType, ReadonlySet<ConstraintPosition>> = {
  Point: new Set<ConstraintPosition>(["Start", "Center"]),
  Line: new Set<ConstraintPosition>(["Start", "End", "Midpoint"]),
  Circle: new Set<ConstraintPosition>(["Center"]),
  Arc: new Set<ConstraintPosition>(["Center", "Start", "End"]),
  // An Ellipse's only named point is its centre (the one minted wire point).
  Ellipse: new Set<ConstraintPosition>(["Center"]),
};

/**
 * Resolve a raw selection ref against the session entities into a
 * `SketchConstraintTarget`, or `null` when the entity is missing or the named
 * `point` is not valid for that entity's type (a defensive boundary — hit-test
 * should never produce an invalid combo, but this adapter does not trust it).
 */
export function toConstraintTarget(
  sel: SketchSelectionRef,
  entities: SketchEntity[],
): SketchConstraintTarget | null {
  const entity = entities.find((e) => e.id === sel.entityId);
  if (!entity) return null;

  if (entity.type === "Point") {
    if (sel.point !== undefined && !VALID_POSITIONS.Point.has(sel.point)) return null;
    return { kind: "point", entityId: entity.id, position: "Start", isFreePoint: true };
  }

  if (sel.point === undefined) {
    switch (entity.type) {
      case "Line":
        return { kind: "line", entityId: entity.id };
      case "Circle":
        return { kind: "circle", entityId: entity.id };
      case "Arc":
        return { kind: "arc", entityId: entity.id };
      case "Ellipse":
        return { kind: "ellipse", entityId: entity.id };
      default:
        return null;
    }
  }

  if (!VALID_POSITIONS[entity.type].has(sel.point)) return null;
  return { kind: "point", entityId: entity.id, position: sel.point, isFreePoint: false };
}

/**
 * Resolve a point-kind target's plane (u,v) coordinate against `entities`, or
 * `null` when the entity/field is missing. Shared by `constraintApplicability.ts`
 * (the `hasLineBetweenPoints` probe) and any later authoring step that needs the
 * actual coordinate (e.g. a `Fixed` constraint's `at`).
 */
export function resolveTargetPoint(
  target: SketchConstraintTargetPoint,
  entities: SketchEntity[],
): [number, number] | null {
  const entity = entities.find((e) => e.id === target.entityId);
  if (!entity) return null;

  switch (entity.type) {
    case "Point":
      return entity.p0 ?? null;
    case "Line":
      if (target.position === "Start") return entity.p0 ?? null;
      if (target.position === "End") return entity.p1 ?? null;
      if (target.position === "Midpoint" && entity.p0 && entity.p1) {
        return [(entity.p0[0] + entity.p1[0]) / 2, (entity.p0[1] + entity.p1[1]) / 2];
      }
      return null;
    case "Circle":
    case "Ellipse":
      return target.position === "Center" ? (entity.center ?? null) : null;
    case "Arc":
      if (target.position === "Center") return entity.center ?? null;
      if (target.position === "Start") return entity.start ?? null;
      if (target.position === "End") return entity.end ?? null;
      return null;
    default:
      return null;
  }
}
