/*
 * sketchTopology — the ONE owner of frontend sketch point addressing (SKETCH-V2 P0.5).
 *
 * The frontend has no point entities: `SketchEntity` inlines coordinates
 * (`p0`/`p1`/`center`/`start`/`end`), so a point is addressed as an
 * `(entityId, ConstraintPosition)` pair. That address is spelled independently
 * in four places today — `sketchWireMap.pointKey`, `selectGesture.pointRef`,
 * the keys `frontendSolvedPositions` emits, and the `lastIndexOf(".")` parse in
 * `applySolvedPositions` — and RESOLVED to a coordinate in three more
 * (`badgeLayout.entityPointCoord`, `dimensionLineLayout.resolvePoint`,
 * `sketchWireMap.pointCoord`). This module replaces all seven.
 *
 * P0.5 is deliberately ADDRESSING ONLY. The vertex graph (union of coincident
 * points, adjacency, degree, connected components) lands in P4 once entities
 * reference shared vertices; nothing here computes topology yet, despite the
 * module name — it is the file that will grow those operations.
 *
 * ── Two traps this module exists to close ────────────────────────────────────
 *
 * 1. A free `Point` entity has TWO addresses for ONE coordinate.
 *    `sketchWireMap.addEntityOps` seeds both `${id}.Start` and `${id}.Center`
 *    to the same minted uuid, and `constraintTarget.VALID_POSITIONS` lists both.
 *    Left alone that is two identities no constraint can ever merge, and which
 *    address wins on the way back is decided by `Map` insertion order in
 *    `frontendSolvedPositions`' reverse map. `canonicalRole` folds
 *    `Point.Center → Point.Start` so there is exactly one address.
 *
 * 2. The three existing coordinate resolvers DISAGREE.
 *    `badgeLayout`/`dimensionLineLayout` are strict — they return null for a
 *    position the entity does not actually have. `sketchWireMap.pointCoord` is
 *    lenient: it falls back to the entity's "main" coordinate, so a Circle
 *    resolves ANY position to its centre, and a `Point` resolves `End` to `p0`.
 *    This module is STRICT, with the `Point.Center` alias as the one blessed
 *    leniency. See `sketchTopology.test.ts` for the exhaustive (type ×
 *    position) table proving which cells change.
 */
import type { ConstraintPosition, SketchEntity, SketchEntityType } from "@/ipc/types";

/**
 * `"<entityId>.<ConstraintPosition>"` — the frontend point address.
 *
 * Byte-identical to the string the wire already uses: `SolveDrag.pointId`
 * (SCHEMA §7.4, e.g. `"e3.start"` modulo case), the keys of the solver's
 * `positions` map, and `sketchWireMap`'s id-map keys. Not a branded type: it
 * crosses the IPC boundary as a plain string and branding it here would only
 * force casts at every marshalling site.
 */
export type PointKey = string;

/** How the marshaller can address a point on the wire TODAY. */
export type PointWireForm =
  /** A minted point uuid — Point.Start, Line.Start/End, Circle/Arc/Ellipse.Center. */
  | "pointId"
  /** The owning ARC's uuid plus a role — its endpoints are real solver points
   *  with no independent id in the Rust document. Expressible under EVERY
   *  point-taking kind since SKETCH-V2 P3 generalized SCHEMA §7.3; before that
   *  only `Coincident` carried it and the rest were silently dropped. */
  | "arcRole"
  /** Never a wire point — a Line's virtual midpoint. */
  | "none";

/** One addressable point slot on an entity. */
export interface PointSlot {
  position: ConstraintPosition;
  wireForm: PointWireForm;
  /** Derived from other geometry rather than an independent coordinate. A
   *  derived slot can carry no constraint of its own and never merges. */
  derived: boolean;
}

/**
 * The canonical slot table — the single source that
 * `autoConstrain.entityPoints`, `snapEngine.entitySnapPoints`,
 * `SketchObject.entityMarkers` and `constraintTarget.VALID_POSITIONS` each
 * re-declare today.
 *
 * `Point.Center` is absent BY DESIGN: it is an alias of `Point.Start`
 * (`canonicalRole`), not a slot of its own.
 */
const SLOTS: Record<SketchEntityType, readonly PointSlot[]> = {
  Point: [{ position: "Start", wireForm: "pointId", derived: false }],
  Line: [
    { position: "Start", wireForm: "pointId", derived: false },
    { position: "End", wireForm: "pointId", derived: false },
    { position: "Midpoint", wireForm: "none", derived: true },
  ],
  Circle: [{ position: "Center", wireForm: "pointId", derived: false }],
  Ellipse: [{ position: "Center", wireForm: "pointId", derived: false }],
  Arc: [
    { position: "Center", wireForm: "pointId", derived: false },
    // Worker-minted: real solver points (`Sketch.cpp` — "an arc's minted
    // endpoint points are ordinary SketchPoint entities") with no independent
    // id in the Rust document, so they ride as owner+role. P3 generalizes this.
    { position: "Start", wireForm: "arcRole", derived: false },
    { position: "End", wireForm: "arcRole", derived: false },
  ],
};

/**
 * Fold a raw position onto the entity's canonical slot, or `null` when the
 * entity has no such point.
 *
 * The ONE alias is `Point.Center → Point.Start`. Everything else is exact, so
 * `Circle.Start`, `Line.Center` and `Arc.Midpoint` — all of which
 * `sketchWireMap.pointCoord` silently answers today — become `null`.
 */
export function canonicalRole(
  type: SketchEntityType,
  position: ConstraintPosition,
): ConstraintPosition | null {
  if (type === "Point" && position === "Center") return "Start";
  return SLOTS[type].some((s) => s.position === position) ? position : null;
}

/** Every addressable slot on `e`, including derived ones, in canonical order. */
export function entityPointSlots(e: SketchEntity): readonly PointSlot[] {
  return SLOTS[e.type] ?? [];
}

/**
 * The point an entity means when a constraint names it with NO position.
 *
 * On the wire an absent (or empty) `positions` slot means "this slot IS the
 * point" (SCHEMA §7.3), which for a curve means its defining point. This
 * reproduces the fallthrough of the lenient resolver being retired, so
 * absent-position behaviour is preserved EXACTLY across the migration — the
 * strict resolver then narrows only the cells where a WRONG position was
 * being silently answered with a fallback coordinate.
 */
export function defaultPointRole(type: SketchEntityType): ConstraintPosition {
  return type === "Point" || type === "Line" ? "Start" : "Center";
}

/** The wire form of a point, or `null` when the entity has no such point. */
export function pointWireForm(
  type: SketchEntityType,
  position: ConstraintPosition,
): PointWireForm | null {
  const role = canonicalRole(type, position);
  if (role === null) return null;
  return SLOTS[type].find((s) => s.position === role)?.wireForm ?? null;
}

/**
 * Mint a point address. The ONLY place this string is built — `pointKey`,
 * `pointRef` and the solved-position keys all route here.
 *
 * Does NOT canonicalize: callers that hold an entity should canonicalize first
 * (`canonicalRole`), because minting needs the entity type and this signature
 * deliberately does not require one — hydration and the wire both hand us an
 * id plus a role with no entity in scope.
 */
export function pointKeyOf(entityId: string, position: ConstraintPosition): PointKey {
  return `${entityId}.${position}`;
}

const POSITIONS: ReadonlySet<string> = new Set<ConstraintPosition>([
  "Start",
  "End",
  "Center",
  "Midpoint",
]);

/**
 * Split a point address back into its parts, or `null` if it is not one.
 *
 * Splits on the LAST dot, matching `applySolvedPositions`' existing parse: a
 * frontend id (`e1`) and a backend uuid both contain no dot, but pinning the
 * rule here means a future id scheme changes one function, not four.
 */
export function parsePointKey(
  key: string,
): { entityId: string; position: ConstraintPosition } | null {
  const dot = key.lastIndexOf(".");
  if (dot <= 0 || dot === key.length - 1) return null;
  const position = key.slice(dot + 1);
  if (!POSITIONS.has(position)) return null;
  return { entityId: key.slice(0, dot), position: position as ConstraintPosition };
}

/**
 * The plane (u,v) coordinate of an entity's named point — STRICT.
 *
 * Replaces `badgeLayout.entityPointCoord`, `dimensionLineLayout.resolvePoint`
 * and `sketchWireMap.pointCoord`. Returns the storage-native tuple; the
 * `features` layer wraps it to its own `Point2`.
 */
export function pointCoordOf(
  e: SketchEntity,
  position: ConstraintPosition,
): [number, number] | null {
  const role = canonicalRole(e.type, position);
  if (role === null) return null;

  switch (role) {
    case "Center":
      return e.center ?? null;
    case "Start":
      if (e.type === "Arc") return e.start ?? null;
      return e.p0 ?? null;
    case "End":
      if (e.type === "Arc") return e.end ?? null;
      return e.p1 ?? null;
    case "Midpoint":
      return e.p0 && e.p1 ? [(e.p0[0] + e.p1[0]) / 2, (e.p0[1] + e.p1[1]) / 2] : null;
  }
}

/**
 * Why a constraint cannot reach the wire, independent of any id-map.
 *
 * `arcEndpointRole` USED to be a member: before SKETCH-V2 P3 only `Coincident`
 * could name an arc endpoint, and every other kind was silently dropped. §7.3
 * now generalizes the owner+role form to every point-taking slot, so the only
 * remaining unexpressible point is a derived one.
 */
export type ConstraintDropReason =
  /** Names a derived point (a Line's virtual midpoint) as a POINT slot. */
  | "derivedPoint"
  /** Names a position the entity does not have at all (e.g. `Circle.Start`,
   *  `Line.Center`) — `pointWireForm` returns `null`, not `"none"`, for this. */
  | "invalidPoint";

/**
 * Can this constraint be put on the wire at all? `null` means yes.
 *
 * PURE — decided from `(entityType, position)` alone, with no id-map, so the
 * mock and tauri lanes give the SAME answer. That matters: `SketchIdMap` lives
 * in the tauri client's closure, so an id-map-based check would be invisible to
 * vitest and every Playwright spec, and the authoring guard would be untestable.
 *
 * This is the frontend half of the seam that SCHEMA §7.3 closes in P3; until
 * then `Coincident` is the only kind whose wire shape carries the owner+role
 * form, and everything else targeting an arc endpoint is dropped.
 */
export function constraintMarshalBlocker(
  c: {
    type: string;
    entities: readonly string[];
    /** An entity-level slot is encoded EITHER as an `undefined` element (the
     *  `ToolConstraintSpec` form) or by the array simply being shorter than
     *  `entities` (the `SketchConstraint` form `toWireConstraint` reads via
     *  `c.positions?.[i]`). Both are accepted so callers need no normalization. */
    positions?: readonly (ConstraintPosition | undefined)[];
  },
  byId: ReadonlyMap<string, SketchEntity>,
): ConstraintDropReason | null {
  for (let i = 0; i < c.entities.length; i++) {
    const position = c.positions?.[i];
    if (!position) continue; // entity-level slot (a whole line / circle / axis)
    const e = byId.get(c.entities[i]);
    if (!e) continue; // a missing entity is a different failure, not this one
    // `arcRole` is expressible under EVERY kind since P3 — only a derived
    // point (a Line's virtual midpoint) has no wire form at all.
    const form = pointWireForm(e.type, position);
    if (form === "none") return "derivedPoint";
    // A position the entity does not have (e.g. `Circle.Start`) used to sail
    // through this preflight — only `"none"` was checked — and fail or drop
    // silently downstream in marshalling instead of being refused loudly here.
    if (form === null) return "invalidPoint";
  }
  return null;
}

/** `pointCoordOf` addressed by key against a lookup map. */
export function coordOfPointKey(
  key: PointKey,
  byId: ReadonlyMap<string, SketchEntity>,
): [number, number] | null {
  const parsed = parsePointKey(key);
  if (!parsed) return null;
  const e = byId.get(parsed.entityId);
  return e ? pointCoordOf(e, parsed.position) : null;
}
