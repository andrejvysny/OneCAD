/*
 * sketchWireMap — PURE translation between the frontend sketch model and the Rust
 * typed document sketch vocabulary (the F-WP9 sketch-lane marshaller).
 *
 * ── Three representations (documented; this module bridges #1 → #3) ────────────
 *  1. FRONTEND (`src/ipc/types.ts`): inlined-coordinate entities, STRING ids
 *     (`e1`/`c1`), a `type` tag (PascalCase), no separate point entities — a Line
 *     carries `p0`/`p1` coordinate pairs.
 *  2. WORKER WIRE (what `enter_sketch` RETURNS via `sketch_wire`): a `type` tag,
 *     `p0Ref`/`p1Ref` point-id references, circles/arcs with an inlined `center`
 *     coordinate, plus separate `Point` entities. (Reverse-mapped by
 *     `frontendSessionFromDto`.)
 *  3. RUST TYPED DOC (`onecad_core::edit::SketchEditOp` → `sketch::{SketchEntity,
 *     Constraint}`): internally tagged on `"kind"` (camelCase), UUID ids, entities
 *     reference points BY ID (`start`/`end`/`center`), separate `Point` entities.
 *     This is what `sketch_upsert(sketchId, ops)` consumes.
 *
 * The frontend authors #1; `sketch_upsert` wants #3. Because the frontend has no
 * separate point entities and non-UUID ids, this marshaller SYNTHESIZES a UUID
 * `Point` per line endpoint / arc-circle center and keeps an id-map so a later
 * upsert / drag references the same minted ids. Coincident/positional constraints
 * resolve to those synthesized point ids.
 *
 * LIMITS (report → M2 gate): (a) the frontend Coincident/adjacency model gives
 * each line its OWN endpoints tied by Coincident (matches the Rust separate-point
 * + Coincident model, so it is faithful); (b) an Arc's Rust form references only
 * its `center` point — its ENDPOINTS are minted by the worker (W0b), which
 * registers `<id>.start`/`<id>.end` handles and couples them with internal arc
 * rules. This marshaller therefore has no point uuid to hand out for an arc
 * Start/End and instead addresses them as "the ARC's uuid + a position"
 * (`arcEndpointRef` → the wire `point1Position`/`point2Position` fields). Only
 * `Coincident` speaks that form today; every other kind still resolves an arc
 * Start/End to `null` and is skipped; (c) an Ellipse (W3 P3) marshals exactly
 * like a Circle — one synthesized `Center` point plus `majorR`/`minorR`/
 * `rotation` scalars — so its centre is a real solver handle and its CURVE
 * carries no constraints at all (legacy parity).
 */
import type {
  ConstraintPosition,
  CurveParams,
  SketchConstraint,
  SketchConstraintType,
  SketchEntity,
  ProjectedSource,
  SketchEntityStates,
  SketchPlaneKind,
} from "./types";
import { planeFor } from "./mockSketch";
import { fromWireDimensionValue, toWireDimensionValue } from "./angleUnits";
import { logWarn } from "@/debug/log";
// `@/ipc` → `@/tools/sketch` is an established direction here (`mockSketch.ts`
// imports ellipseMath, `localSolver.ts` prismPreview, `mockClient.ts` four such
// modules); the architecture law binds `src/platform/**`, not this layer. The
// edge buys ONE owner for the point-address format and the (type × position)
// resolution table — see `sketchTopology.ts`.
import { defaultPointRole, pointCoordOf, pointKeyOf } from "@/tools/sketch/sketchTopology";

/** The 18 constraint-type tokens the worker wire emits (== `SketchConstraintType`). */
const CONSTRAINT_TYPES: ReadonlySet<string> = new Set<SketchConstraintType>([
  "Coincident",
  "Horizontal",
  "Vertical",
  "Fixed",
  "Midpoint",
  "OnCurve",
  "Parallel",
  "Perpendicular",
  "Tangent",
  "Concentric",
  "Equal",
  "Distance",
  "HorizontalDistance",
  "VerticalDistance",
  "Angle",
  "Radius",
  "Diameter",
  "Symmetric",
]);

// ── Rust typed doc wire shapes (SketchEditOp target) ──────────────────────────

/** A dimension value on the wire (Rust `Scalar {value, expr?}`). */
interface WireScalar {
  value: number;
}

interface WirePoint {
  kind: "point";
  id: string;
  at: [number, number];
  construction?: boolean;
}
interface WireLine {
  kind: "line";
  id: string;
  start: string;
  end: string;
  construction?: boolean;
}
interface WireCircle {
  kind: "circle";
  id: string;
  center: string;
  radius: number;
  construction?: boolean;
}
interface WireArc {
  kind: "arc";
  id: string;
  center: string;
  radius: number;
  startAngle: number;
  endAngle: number;
  construction?: boolean;
}
/** Rust `SketchEntity::Ellipse` (W3 P3). `rotation` is RADIANS; `majorR ≥ minorR`
 *  is guaranteed by the FRONTEND (`normalizeEllipse`, ellipseMath.ts) — the tool is
 *  the single normalization site, so the wire never carries minor > major. */
interface WireEllipse {
  kind: "ellipse";
  id: string;
  center: string;
  majorR: number;
  minorR: number;
  rotation: number;
  construction?: boolean;
}
export type WireSketchEntity = WirePoint | WireLine | WireCircle | WireArc | WireEllipse;

/** A Rust `Constraint` (internally tagged `"kind"`, camelCase). Only the kinds the
 *  vertical-slice tools author are modeled; others route through a best-effort. */
export type WireConstraint =
  // `point1Position`/`point2Position` are the Rust `CurvePosition` (camelCase),
  // OMITTED when the ref IS a point entity (Rust defaults them to `Arbitrary` and
  // skips them on the wire). Present only for an ARC-endpoint ref, where the
  // `point*` slot names the arc and the position picks its `.start`/`.end`.
  | {
      kind: "coincident";
      id: string;
      point1: string;
      point2: string;
      point1Position?: "start" | "end";
      point2Position?: "start" | "end";
    }
  | { kind: "horizontal"; id: string; line: string }
  | { kind: "vertical"; id: string; line: string }
  | {
      kind: "fixed";
      id: string;
      point: string;
      pointPosition?: "start" | "end";
      at: [number, number];
    }
  | {
      kind: "midpoint";
      id: string;
      point: string;
      pointPosition?: "start" | "end";
      line: string;
    }
  // OnCurve `position` is the Rust `CurvePosition` (camelCase); OMITTED here so it
  // defaults to `Arbitrary` (the applicability path never pins an endpoint).
  | {
      kind: "onCurve";
      id: string;
      point: string;
      // The POINT's own role. DISTINCT from `position` below, which says where on
      // the CURVE the point is pinned — two different concepts (SCHEMA §7.3).
      pointPosition?: "start" | "end";
      curve: string;
      position?: "start" | "end";
    }
  /** SNAP P3 — two POINT slots, no value. Both accept the arc owner+role form. */
  | {
      kind: "horizontalPoints" | "verticalPoints";
      id: string;
      point1: string;
      point2: string;
      point1Position?: "start" | "end";
      point2Position?: "start" | "end";
    }
  | { kind: "parallel"; id: string; line1: string; line2: string }
  | { kind: "perpendicular"; id: string; line1: string; line2: string }
  | { kind: "tangent"; id: string; entity1: string; entity2: string }
  | { kind: "concentric"; id: string; entity1: string; entity2: string }
  | { kind: "equal"; id: string; entity1: string; entity2: string }
  | {
      kind: "symmetric";
      id: string;
      point1: string;
      point1Position?: "start" | "end";
      point2: string;
      point2Position?: "start" | "end";
      axis: string;
    }
  | {
      kind: "distance";
      id: string;
      entity1: string;
      entity1Position?: "start" | "end";
      entity2: string;
      entity2Position?: "start" | "end";
      value: WireScalar;
    }
  | {
      kind: "horizontalDistance";
      id: string;
      point1: string;
      point1Position?: "start" | "end";
      point2: string;
      point2Position?: "start" | "end";
      value: WireScalar;
    }
  | {
      kind: "verticalDistance";
      id: string;
      point1: string;
      point1Position?: "start" | "end";
      point2: string;
      point2Position?: "start" | "end";
      value: WireScalar;
    }
  | { kind: "angle"; id: string; line1: string; line2: string; value: WireScalar }
  | { kind: "radius"; id: string; entity: string; value: WireScalar }
  | { kind: "diameter"; id: string; entity: string; value: WireScalar };

/** One `SketchEditOp` (Rust enum, internally tagged `"op"`, camelCase). */
export type SketchEditOp =
  | { op: "addEntity"; entity: WireSketchEntity }
  | { op: "removeEntity"; entity: string }
  | { op: "addConstraint"; constraint: WireConstraint }
  | { op: "removeConstraint"; constraint: string }
  | { op: "setDimension"; constraint: string; value: WireScalar }
  | { op: "setEntityPositions"; positions: [string, [number, number]][] }
  // W1-B: flip the construction flag on an ALREADY-ADDED entity. The add path
  // carries `construction` inline, so this op exists only for the in-place edit
  // (X key / chrome toggle) — without it a flip on a mapped id emitted nothing.
  | { op: "setEntityConstruction"; entity: string; construction: boolean };

// ── Id-map: string frontend ids ↔ minted UUIDs, kept across upserts ───────────

/** Per-sketch id map so successive upserts + drags reference the same minted ids. */
export interface SketchIdMap {
  /** The real Rust-minted `SketchId` (UUID) the backend commands target. */
  backendSketchId: string;
  planeKind: SketchPlaneKind;
  /** Frontend entity id → minted Rust entity UUID. */
  entity: Map<string, string>;
  /** `"${entityId}.${Position}"` → minted Rust POINT UUID (synthesized points). */
  point: Map<string, string>;
  /** Frontend constraint id → minted Rust constraint UUID. */
  constraint: Map<string, string>;
  /** Frontend constraint id → last dimension value sent (for SetDimension diffs). */
  constraintValue: Map<string, number | undefined>;
  /** Frontend entity id → last `construction` flag SENT (for SetEntityConstruction
   *  diffs). Mirrors `constraintValue`: the backend already holds this value, so a
   *  marshal only emits when the live entity disagrees with it. */
  entityConstruction: Map<string, boolean>;
  /** Frontend ids of REFERENCE-LOCKED entities (SCHEMA §7.3), seeded on hydration.
   *  Not a diff cache — a latch: the marshaller refuses to author a destructive op
   *  against any id in here. Locked geometry is projected from the host face, and
   *  the backend would reject the op anyway (`SketchError::ReferenceLocked`), so
   *  emitting one could only ever fail a whole upsert batch. */
  entityReferenceLocked: Set<string>;
}

export function createIdMap(backendSketchId: string, planeKind: SketchPlaneKind): SketchIdMap {
  return {
    backendSketchId,
    planeKind,
    entity: new Map(),
    point: new Map(),
    constraint: new Map(),
    constraintValue: new Map(),
    entityConstruction: new Map(),
    entityReferenceLocked: new Set(),
  };
}

/**
 * Deep-clone a per-sketch id map (the 6 id/cache collections duplicated; scalar fields copied).
 * The upsert marshaller MUTATES the map it diffs against as it mints/drops ids, so an
 * upsert works on a CLONE and the caller commits it onto the live map ONLY after the
 * RPC resolves — a rejected upsert then leaves the live map byte-for-byte unchanged
 * (transactional), instead of stranding half-applied add/remove bookkeeping.
 */
export function cloneIdMap(map: SketchIdMap): SketchIdMap {
  return {
    backendSketchId: map.backendSketchId,
    planeKind: map.planeKind,
    entity: new Map(map.entity),
    point: new Map(map.point),
    constraint: new Map(map.constraint),
    constraintValue: new Map(map.constraintValue),
    entityConstruction: new Map(map.entityConstruction),
    entityReferenceLocked: new Set(map.entityReferenceLocked),
  };
}

/** The point-address minter. Re-exported shape kept local for call-site brevity;
 *  the ONE implementation lives in `sketchTopology` (SKETCH-V2 P0.5) so this
 *  module, `selectGesture.pointRef`, `frontendSolvedPositions`' output keys and
 *  `applySolvedPositions`' parse can no longer drift apart. */
const pointKey = pointKeyOf;

/** Mint a real UUID v4 (crypto.randomUUID, with an RFC-4122 fallback for jsdom). */
export function mintUuid(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === "function") return c.randomUUID();
  const b = c.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = Array.from(b, (x) => x.toString(16).padStart(2, "0"));
  return `${h[0]}${h[1]}${h[2]}${h[3]}-${h[4]}${h[5]}-${h[6]}${h[7]}-${h[8]}${h[9]}-${h[10]}${h[11]}${h[12]}${h[13]}${h[14]}${h[15]}`;
}

// ── Entity → AddEntity ops (synthesizing points) ──────────────────────────────

/** Emit the AddEntity ops for one frontend entity (points first, then the entity),
 *  recording the minted ids in `map`. Returns `[]` for an unmappable entity. */
function addEntityOps(map: SketchIdMap, e: SketchEntity, mint: () => string): SketchEditOp[] {
  const ops: SketchEditOp[] = [];
  const construction = e.construction ? true : undefined;

  // Bind the frontend id to its minted uuid AND record the `construction` flag this
  // Add carries — the SetEntityConstruction diff (W1-B) compares against it.
  const bind = (id: string): void => {
    map.entity.set(e.id, id);
    map.entityConstruction.set(e.id, !!e.construction);
  };

  const mintPoint = (position: ConstraintPosition, at: [number, number]): string => {
    const id = mint();
    map.point.set(pointKey(e.id, position), id);
    ops.push({ op: "addEntity", entity: { kind: "point", id, at, construction } });
    return id;
  };

  switch (e.type) {
    case "Point": {
      if (!e.p0) return [];
      const id = mint();
      bind(id);
      map.point.set(pointKey(e.id, "Start"), id);
      map.point.set(pointKey(e.id, "Center"), id);
      ops.push({ op: "addEntity", entity: { kind: "point", id, at: e.p0, construction } });
      return ops;
    }
    case "Line": {
      if (!e.p0 || !e.p1) return [];
      const start = mintPoint("Start", e.p0);
      const end = mintPoint("End", e.p1);
      const id = mint();
      bind(id);
      ops.push({ op: "addEntity", entity: { kind: "line", id, start, end, construction } });
      return ops;
    }
    case "Circle": {
      if (!e.center || e.radius === undefined) return [];
      const center = mintPoint("Center", e.center);
      const id = mint();
      bind(id);
      ops.push({ op: "addEntity", entity: { kind: "circle", id, center, radius: e.radius, construction } });
      return ops;
    }
    case "Ellipse": {
      if (!e.center || e.majorR === undefined || e.minorR === undefined) return [];
      const center = mintPoint("Center", e.center);
      const id = mint();
      bind(id);
      ops.push({
        op: "addEntity",
        entity: {
          kind: "ellipse",
          id,
          center,
          majorR: e.majorR,
          minorR: e.minorR,
          rotation: e.rotation ?? 0,
          construction,
        },
      });
      return ops;
    }
    case "Arc": {
      if (!e.center || e.radius === undefined) return [];
      const center = mintPoint("Center", e.center);
      const id = mint();
      bind(id);
      // Rust Arc stores angles (radians from +X); derive from start/end coords.
      const angle = (p?: [number, number]): number =>
        p ? Math.atan2(p[1] - e.center![1], p[0] - e.center![0]) : 0;
      ops.push({
        op: "addEntity",
        entity: {
          kind: "arc",
          id,
          center,
          radius: e.radius,
          startAngle: angle(e.start),
          endAngle: angle(e.end),
          construction,
        },
      });
      return ops;
    }
    default:
      return [];
  }
}

// ── Constraint → AddConstraint op ─────────────────────────────────────────────

/** Resolve a frontend entity ref to its Rust id. With a positional selector the
 *  ref is a POINT — resolve STRICTLY to the synthesized point uuid. Without a
 *  selector the ref is the entity itself (e.g. a Point entity or a whole line).
 *
 *  An ARC's Start/End is the one position with no synthesized point uuid: those
 *  endpoints are minted by the WORKER, not by this marshaller, so they are
 *  addressed as "the arc entity + a role" and never resolve here. Only
 *  `Coincident` can carry that role today (`arcEndpointRef`) — every other kind
 *  still gets `null` and is skipped. */
function resolveRef(
  map: SketchIdMap,
  entityId: string,
  position?: ConstraintPosition,
): string | null {
  if (position) return map.point.get(pointKey(entityId, position)) ?? null;
  return map.entity.get(entityId) ?? null;
}

/** The wire form of an ARC endpoint ref: the arc's own uuid plus a lowercase
 *  role, matching the Rust `Constraint::Coincident` `point*Position` field.
 *  Returns `null` when the ref is not an arc Start/End (or the arc is unmapped),
 *  so callers fall back to the ordinary point resolution. */
function arcEndpointRef(
  map: SketchIdMap,
  entities: SketchEntity[],
  entityId: string,
  position?: ConstraintPosition,
): { id: string; position: "start" | "end" } | null {
  if (position !== "Start" && position !== "End") return null;
  if (entities.find((e) => e.id === entityId)?.type !== "Arc") return null;
  const id = map.entity.get(entityId);
  return id ? { id, position: position === "Start" ? "start" : "end" } : null;
}

/** Current plane coord of a point ref (entity + optional position) — the `at` a
 *  `Fixed` constraint pins to. Delegates to `sketchTopology.pointCoordOf`, the
 *  single owner of the (type × position) table; this used to be a third private
 *  copy that disagreed with the other two by answering a wrong position with a
 *  fallback coordinate. */
function pointCoord(
  entities: SketchEntity[],
  entityId: string,
  position?: ConstraintPosition,
): [number, number] | null {
  const e = entities.find((x) => x.id === entityId);
  if (!e) return null;
  // An ABSENT position keeps its old meaning exactly — "this slot IS the point"
  // (SCHEMA §7.3), i.e. the entity's defining point. What changed (SKETCH-V2
  // P0.5) is that a position the entity does not actually have now resolves to
  // null instead of silently falling back to that same defining point: a
  // hydrated `Fixed` naming `Circle.Start` used to pin the CENTRE. Dropping it
  // loudly beats binding the wrong point — and the drop is now reported.
  return pointCoordOf(e, position ?? defaultPointRole(e.type));
}

const DIMENSIONAL: ReadonlySet<SketchConstraintType> = new Set([
  "Distance",
  "HorizontalDistance",
  "VerticalDistance",
  "Angle",
  "Radius",
  "Diameter",
]);

/** Map one frontend constraint to a Rust `WireConstraint` (or `null` if it cannot
 *  be expressed — e.g. a ref that is not in the id-map yet).
 *
 *  `entities` is the FULL authoritative entity array (needed for `Fixed`, whose
 *  wire shape carries the point's CURRENT plane coords as `at`, and for
 *  `Coincident`, which must know whether a Start/End ref names an ARC). */
function toWireConstraint(
  map: SketchIdMap,
  c: SketchConstraint,
  id: string,
  entities: SketchEntity[],
): WireConstraint | null {
  const pos = c.positions ?? [];
  const ref = (i: number): string | null => resolveRef(map, c.entities[i], pos[i]);
  // Dimensional value crosses in the WIRE domain (Angle: deg→rad; BUG-2).
  const val: WireScalar = { value: toWireDimensionValue(c.type, c.value ?? 0) };

  /**
   * A POINT slot: either an ordinary point ref, or an ARC endpoint addressed as
   * the arc's uuid + a role. The worker mints an arc's endpoints as real solver
   * points and registers `<id>.start`/`<id>.end` (W0b).
   *
   * SKETCH-V2 P3 hoisted this out of `Coincident`. It used to be the ONLY kind
   * that could express the owner+role form, so every other constraint on an arc
   * endpoint resolved null and was silently dropped by `marshalUpsert` — the
   * user clicked, the DOF did not move, and nothing said why.
   */
  const slot = (i: number): { id: string; position?: "start" | "end" } | null =>
    arcEndpointRef(map, entities, c.entities[i], pos[i]) ??
    (ref(i) ? { id: ref(i)! } : null);

  switch (c.type) {
    case "Coincident": {
      const a = slot(0);
      const b = slot(1);
      if (!a || !b) return null;
      const out: WireConstraint = { kind: "coincident", id, point1: a.id, point2: b.id };
      if (a.position) out.point1Position = a.position;
      if (b.position) out.point2Position = b.position;
      return out;
    }
    case "Horizontal": {
      const line = map.entity.get(c.entities[0]);
      return line ? { kind: "horizontal", id, line } : null;
    }
    case "Vertical": {
      const line = map.entity.get(c.entities[0]);
      return line ? { kind: "vertical", id, line } : null;
    }
    case "Midpoint": {
      const point = slot(0);
      const line = map.entity.get(c.entities[1]);
      if (!point || !line) return null;
      const out: WireConstraint = { kind: "midpoint", id, point: point.id, line };
      if (point.position) out.pointPosition = point.position;
      return out;
    }
    case "HorizontalPoints":
    case "VerticalPoints": {
      const a = slot(0);
      const b = slot(1);
      if (!a || !b) return null;
      const out: WireConstraint = {
        kind: c.type === "HorizontalPoints" ? "horizontalPoints" : "verticalPoints",
        id,
        point1: a.id,
        point2: b.id,
      };
      if (a.position) out.point1Position = a.position;
      if (b.position) out.point2Position = b.position;
      return out;
    }
    case "Parallel":
    case "Perpendicular": {
      const l1 = map.entity.get(c.entities[0]);
      const l2 = map.entity.get(c.entities[1]);
      return l1 && l2 ? { kind: c.type.toLowerCase() as "parallel" | "perpendicular", id, line1: l1, line2: l2 } : null;
    }
    case "Equal": {
      const e1 = map.entity.get(c.entities[0]);
      const e2 = map.entity.get(c.entities[1]);
      return e1 && e2 ? { kind: "equal", id, entity1: e1, entity2: e2 } : null;
    }
    case "Distance": {
      const e1 = slot(0);
      const e2 = slot(1);
      if (!e1 || !e2) return null;
      const out: WireConstraint = {
        kind: "distance", id, entity1: e1.id, entity2: e2.id, value: val,
      };
      if (e1.position) out.entity1Position = e1.position;
      if (e2.position) out.entity2Position = e2.position;
      return out;
    }
    case "HorizontalDistance": {
      const p1 = slot(0);
      const p2 = slot(1);
      if (!p1 || !p2) return null;
      const out: WireConstraint = {
        kind: "horizontalDistance", id, point1: p1.id, point2: p2.id, value: val,
      };
      if (p1.position) out.point1Position = p1.position;
      if (p2.position) out.point2Position = p2.position;
      return out;
    }
    case "VerticalDistance": {
      const p1 = slot(0);
      const p2 = slot(1);
      if (!p1 || !p2) return null;
      const out: WireConstraint = {
        kind: "verticalDistance", id, point1: p1.id, point2: p2.id, value: val,
      };
      if (p1.position) out.point1Position = p1.position;
      if (p2.position) out.point2Position = p2.position;
      return out;
    }
    case "Angle": {
      const l1 = map.entity.get(c.entities[0]);
      const l2 = map.entity.get(c.entities[1]);
      return l1 && l2 ? { kind: "angle", id, line1: l1, line2: l2, value: val } : null;
    }
    case "Radius": {
      const entity = map.entity.get(c.entities[0]);
      return entity ? { kind: "radius", id, entity, value: val } : null;
    }
    case "Diameter": {
      const entity = map.entity.get(c.entities[0]);
      return entity ? { kind: "diameter", id, entity, value: val } : null;
    }
    // ── S4b wire extension: the 5 user-applicable geometric kinds ──────────────
    // Point slots resolve position-aware via `ref`; whole-entity slots (curve /
    // axis / circle-arc) resolve directly through `map.entity`.
    case "Fixed": {
      const point = slot(0);
      const at = pointCoord(entities, c.entities[0], pos[0]);
      if (!point || !at) return null;
      const out: WireConstraint = { kind: "fixed", id, point: point.id, at };
      if (point.position) out.pointPosition = point.position;
      return out;
    }
    case "OnCurve": {
      const point = slot(0);
      const curve = map.entity.get(c.entities[1]);
      if (!point || !curve) return null;
      // `position` (where on the CURVE) stays omitted — authored from
      // applicability ⇒ Arbitrary. `pointPosition` is the separate question of
      // WHICH point is being constrained; conflating them is a wire bug.
      const out: WireConstraint = { kind: "onCurve", id, point: point.id, curve };
      if (point.position) out.pointPosition = point.position;
      return out;
    }
    case "Tangent":
    case "Concentric": {
      const e1 = map.entity.get(c.entities[0]);
      const e2 = map.entity.get(c.entities[1]);
      return e1 && e2
        ? { kind: c.type.toLowerCase() as "tangent" | "concentric", id, entity1: e1, entity2: e2 }
        : null;
    }
    case "Symmetric": {
      const point1 = slot(0);
      const point2 = slot(1);
      const axis = map.entity.get(c.entities[2]);
      if (!point1 || !point2 || !axis) return null;
      const sym: WireConstraint = {
        kind: "symmetric", id, point1: point1.id, point2: point2.id, axis,
      };
      if (point1.position) sym.point1Position = point1.position;
      if (point2.position) sym.point2Position = point2.position;
      return sym;
    }
    default:
      return null;
  }
}

/** True for a dimensional constraint whose value can change in place (SetDimension). */
export function isDimensional(type: SketchConstraintType): boolean {
  return DIMENSIONAL.has(type);
}

// ── The upsert marshaller (diff frontend arrays → SketchEditOp[]) ─────────────

/**
 * Compute the ordered `SketchEditOp[]` to bring the backend sketch from its
 * previously-marshalled state (tracked in `map`) to `next`. The frontend sends the
 * FULL authoritative arrays each upsert, so the diff is by id:
 *   - entity id not yet in the map        → AddEntity (+ synthesized points)
 *   - mapped entity id absent from `next`  → RemoveEntity (drop from the map)
 *   - constraint id not yet in the map     → AddConstraint
 *   - mapped constraint id absent          → RemoveConstraint
 *   - dimensional value changed in place   → SetDimension
 * Mutates `map` to record the new mappings.
 */
/** Why a constraint could not be put on the wire (SKETCH-V2 P0.5). */
export interface DroppedConstraint {
  /** FRONTEND constraint id. */
  id: string;
  type: SketchConstraintType;
  reason:
    /** Targets an arc Start/End under a kind other than `Coincident` — the only
     *  kind whose wire shape carries the owner+role form today. Lifted by the P3
     *  protocol change (SCHEMA §7.3 generalizes `PointRef` to every slot). */
    | "arcEndpointRole"
    /** A ref this id-map has never minted (a stale or out-of-table point). */
    | "unmappedRef";
}

/** Classify why `toWireConstraint` refused `c`. */
function dropReason(
  c: SketchConstraint,
  entities: SketchEntity[],
): DroppedConstraint["reason"] {
  const arcEndpoint = c.entities.some((eid, i) => {
    const p = c.positions?.[i];
    return (
      (p === "Start" || p === "End") && entities.find((e) => e.id === eid)?.type === "Arc"
    );
  });
  return arcEndpoint && c.type !== "Coincident" ? "arcEndpointRole" : "unmappedRef";
}

export function marshalUpsert(
  map: SketchIdMap,
  next: { entities: SketchEntity[]; constraints: SketchConstraint[] },
  mint: () => string = mintUuid,
  /** Called once per constraint that could not be marshalled. Additive: the
   *  return value and every existing call site are unchanged.
   *
   *  DO NOT wire this on the hydration / re-marshal path. A slot's arc-endpoint
   *  Coincidents are routinely unmappable (TODO.md:6373), so every sketch
   *  re-entry would report drops the user did not cause. It exists for the
   *  AUTHORING path, where a drop means the constraint the user just asked for
   *  silently did nothing. */
  onDrop?: (d: DroppedConstraint) => void,
): SketchEditOp[] {
  const ops: SketchEditOp[] = [];

  // Removals first (so re-adds of a reused id are unambiguous).
  const liveEntities = new Set(next.entities.map((e) => e.id));
  for (const [fid, uuid] of [...map.entity]) {
    if (!liveEntities.has(fid)) {
      // REFERENCE-LOCK LATCH. Locked entities must stay in `next.entities` — they
      // are part of the session like any other geometry. If one ever goes missing
      // (a tool filtering the session, a hydration gap) the id-only diff below
      // reads it as a deletion, and this loop would author the one op the backend
      // refuses outright, failing the whole batch. Skip instead, and keep the id
      // mapped so the next upsert is still coherent.
      if (map.entityReferenceLocked.has(fid)) continue;
      ops.push({ op: "removeEntity", entity: uuid });
      map.entity.delete(fid);
      map.entityConstruction.delete(fid);
      // BUG-5: also drop the entity's synthesized child points (line endpoints /
      // circle-arc center). The core `RemoveEntity` cascade drops constraints + curves
      // that REFERENCE a removed point, but a point is never a dependent of its owning
      // curve — so without this the child points orphan as stray dots on re-entry.
      const dropped = new Set<string>([uuid]);
      for (const k of [...map.point.keys()]) {
        if (!k.startsWith(`${fid}.`)) continue;
        const childUuid = map.point.get(k)!;
        map.point.delete(k);
        if (!dropped.has(childUuid)) {
          dropped.add(childUuid);
          ops.push({ op: "removeEntity", entity: childUuid });
        }
      }
    }
  }
  const liveConstraints = new Set(next.constraints.map((c) => c.id));
  for (const [fid, uuid] of [...map.constraint]) {
    if (!liveConstraints.has(fid)) {
      ops.push({ op: "removeConstraint", constraint: uuid });
      map.constraint.delete(fid);
      map.constraintValue.delete(fid);
    }
  }

  // Additions (entities before constraints so refs resolve). An ALREADY-mapped
  // entity whose `construction` flag no longer matches what was last sent flips in
  // place (W1-B) — the id-only diff above would otherwise emit nothing and the
  // backend would keep the stale flag (silent loss).
  for (const e of next.entities) {
    if (!map.entity.has(e.id)) {
      ops.push(...addEntityOps(map, e, mint));
    } else if (
      !!e.construction !== !!map.entityConstruction.get(e.id) &&
      // A construction flip is a geometry-semantics edit; the backend refuses it
      // on locked geometry. Nothing in the UI can reach this today (the flip
      // affordances gate on selection, which never carries locked entities), so
      // a disagreement here means the flag drifted — resync the cache silently
      // rather than emit an op that fails the batch.
      !map.entityReferenceLocked.has(e.id)
    ) {
      map.entityConstruction.set(e.id, !!e.construction);
      ops.push({
        op: "setEntityConstruction",
        entity: map.entity.get(e.id)!,
        construction: !!e.construction,
      });
    }
  }
  for (const c of next.constraints) {
    if (!map.constraint.has(c.id)) {
      const id = mint();
      const wire = toWireConstraint(map, c, id, next.entities);
      if (wire) {
        map.constraint.set(c.id, id);
        map.constraintValue.set(c.id, c.value);
        ops.push({ op: "addConstraint", constraint: wire });
      } else if (onDrop) {
        // Unmappable (a ref this map never minted, or an arc Start/End under a
        // kind other than Coincident). The solver still runs on the geometry, so
        // nothing breaks — but the constraint the caller asked for is NOT there.
        // Reporting it is the whole point: this used to be silent.
        onDrop({ id: c.id, type: c.type, reason: dropReason(c, next.entities) });
      }
    } else if (isDimensional(c.type) && map.constraintValue.get(c.id) !== c.value) {
      // In-place dimension edit (the DimensionInput chip / editConstraintValue). The
      // cache holds UI-domain values; the wire carries the wire domain (Angle: deg→rad).
      map.constraintValue.set(c.id, c.value);
      ops.push({
        op: "setDimension",
        constraint: map.constraint.get(c.id)!,
        value: { value: toWireDimensionValue(c.type, c.value ?? 0) },
      });
    }
  }

  return ops;
}

// ── AddSketch payload (create the backend sketch before enter_sketch) ─────────

/** The minimal Rust `Sketch` (SketchData serde) for a fresh world-plane sketch —
 *  `entities`/`constraints`/`regions` default to `[]`, but `plane` is REQUIRED
 *  (Rust `SketchPlane`: origin/xAxis/yAxis/normal as `[x,y,z]` arrays, carried
 *  verbatim — omitting it fails deserialization before the command handler runs). */
export interface WireAddSketch {
  cmd: "addSketch";
  sketch: {
    id: string;
    name: string;
    plane: {
      origin: [number, number, number];
      xAxis: [number, number, number];
      yAxis: [number, number, number];
      normal: [number, number, number];
    };
    attachment:
      | { kind: "world"; plane: "XY" | "XZ" | "YZ" }
      // DATUM W1. Rust `SketchAttachment` is internally tagged on `"kind"` with
      // camelCase variants + camelCase fields, so `Datum { datum: DatumPlaneId }`
      // renders exactly like this (`DatumPlaneId` is a transparent UUID string).
      | { kind: "datum"; datum: string };
  };
}

// NOTE (SKETCH-ON-FACE W2): there is deliberately no `hostFace` arm here and no
// `buildAddSketchOnFace`. A sketch on a model face is created by its own command
// (`add_sketch_on_face`), which projects the face boundary into the new sketch
// across two worker round-trips before applying ONE `AddSketch`. An `AddSketch`
// assembled on this side could only ever produce an EMPTY host-face sketch with
// `projectedBoundaryVersion: 0`, which is precisely the seam W2 closed.

/** Build the `AddSketch` EditCommand for a new world-plane sketch. `custom` planes
 *  fall back to XY (a custom basis reaches the wire only via a datum or the
 *  backend's own `add_sketch_on_face`). The plane basis is the SCHEMA §7.3
 *  canonical basis for the kind (same table Rust's `SketchPlane::xy/xz/yz()`
 *  carry — `planeFor` mirrors it verbatim). */
export function buildAddSketch(
  backendSketchId: string,
  name: string,
  planeKind: SketchPlaneKind,
): WireAddSketch {
  const plane = planeKind === "XZ" || planeKind === "YZ" ? planeKind : "XY";
  const { origin, xAxis, yAxis, normal } = planeFor(plane);
  return {
    cmd: "addSketch",
    sketch: {
      id: backendSketchId,
      name,
      plane: { origin, xAxis, yAxis, normal },
      attachment: { kind: "world", plane },
    },
  };
}

/**
 * Build the `AddSketch` EditCommand for a sketch placed on a DATUM plane
 * (DATUM W1).
 *
 * The basis is NOT computed here: `plane` is the
 * datum's backend-RESOLVED frame (`documentStore.datums[id].plane`, itself
 * produced by `DocumentSession::resolve_datum_frame`). Re-deriving it from
 * `basePlaneId + offset` on this side would give the sketch a second, competing
 * source of truth for its own coordinate system — the exact defect class the
 * migration exists to remove. The frame is FROZEN with the sketch (V1 policy).
 *
 * The datum must be RESOLVED (`resolvedValid`) before this is called; the caller
 * refuses an unresolved datum rather than sending a meaningless identity frame.
 */
export function buildAddSketchOnDatum(
  backendSketchId: string,
  name: string,
  plane: WireAddSketch["sketch"]["plane"],
  datumId: string,
): WireAddSketch {
  return {
    cmd: "addSketch",
    sketch: {
      id: backendSketchId,
      name,
      plane: {
        origin: plane.origin,
        xAxis: plane.xAxis,
        yAxis: plane.yAxis,
        normal: plane.normal,
      },
      attachment: { kind: "datum", datum: datumId },
    },
  };
}

// ── UpdateSketchAttachment payload (REATTACH V1, H9) ─────────────────────────

/**
 * The `UpdateSketchAttachment` EditCommand wire shape (Rust
 * `EditCommand::UpdateSketchAttachment`, internally tagged `cmd`, camelCase).
 *
 * `sketch` is a BARE `SketchId` uuid (`#[serde(transparent)]`); `plane` and
 * `attachment` are the SAME shapes `AddSketch` carries, because the core re-uses
 * `SketchPlane` / `SketchAttachment` verbatim on both entry points.
 *
 * There is deliberately no `hostFace` arm — same reason `buildAddSketchOnDatum`
 * has none (see the note above `buildAddSketch`). A host-face sketch's frame is
 * derived from the face by the WORKER, and its projected boundary is expressed in
 * that frame; assembling one here could only produce a `projectedBoundaryVersion:
 * 0` attachment with no projection, which is exactly the seam SKETCH-ON-FACE W2
 * closed. Reattach V1 is world ⇄ datum only.
 */
export interface WireUpdateSketchAttachment {
  cmd: "updateSketchAttachment";
  sketch: string;
  plane: WireAddSketch["sketch"]["plane"];
  attachment: WireAddSketch["sketch"]["attachment"];
}

/**
 * Build `UpdateSketchAttachment` for a reattach to a named WORLD plane. The basis
 * is the SCHEMA §7.3 canonical one for the kind (`planeFor` mirrors Rust's
 * `SketchPlane::xy/xz/yz()` verbatim) — the same source `buildAddSketch` uses, so
 * a sketch created on XZ and one reattached to XZ end up on the identical frame.
 */
export function buildUpdateSketchAttachmentWorld(
  sketchId: string,
  planeKind: "XY" | "XZ" | "YZ",
): WireUpdateSketchAttachment {
  const { origin, xAxis, yAxis, normal } = planeFor(planeKind);
  return {
    cmd: "updateSketchAttachment",
    sketch: sketchId,
    plane: { origin, xAxis, yAxis, normal },
    attachment: { kind: "world", plane: planeKind },
  };
}

/**
 * Build `UpdateSketchAttachment` for a reattach to a DATUM plane.
 *
 * As with `buildAddSketchOnDatum`, the basis is NOT computed here: `plane` is the
 * datum's backend-RESOLVED frame (`documentStore.datums[id].plane`). The core
 * OVERWRITES it anyway (`stamp_datum_plane` — pinned by
 * `reattaching_to_a_datum_stamps_the_resolved_frame_on_the_record`), so sending
 * the resolved frame keeps the two entry points identical rather than making this
 * one a second source of truth.
 */
export function buildUpdateSketchAttachmentDatum(
  sketchId: string,
  plane: WireAddSketch["sketch"]["plane"],
  datumId: string,
): WireUpdateSketchAttachment {
  return {
    cmd: "updateSketchAttachment",
    sketch: sketchId,
    plane: {
      origin: plane.origin,
      xAxis: plane.xAxis,
      yAxis: plane.yAxis,
      normal: plane.normal,
    },
    attachment: { kind: "datum", datum: datumId },
  };
}

// ── DeleteSketch payload (compensation: remove a sketch from the document) ────

/** The `DeleteSketch` EditCommand wire shape (Rust `EditCommand::DeleteSketch`,
 *  internally tagged `cmd`, camelCase). `sketch` is a BARE `SketchId` uuid string —
 *  `SketchId` serde is `#[serde(transparent)]` over a `Uuid` (command.rs). */
export interface WireDeleteSketch {
  cmd: "deleteSketch";
  sketch: string;
}

/** Build the `DeleteSketch` EditCommand for `sketchId` — the compensation/cleanup
 *  verb that removes a sketch feature from the document (e.g. the empty sketch left
 *  behind when `enter_sketch` rejects right after its AddSketch committed). */
export function buildDeleteSketch(sketchId: string): WireDeleteSketch {
  return { cmd: "deleteSketch", sketch: sketchId };
}

// ── Reverse map: SketchSessionDto (worker wire) → frontend SketchSession ──────

/** One entity from the worker wire form (`enter_sketch` returns these). */
interface WireDtoEntity {
  id: string;
  type: "Point" | "Line" | "Circle" | "Arc" | "Ellipse";
  at?: [number, number];
  p0Ref?: string;
  p1Ref?: string;
  center?: [number, number];
  /** Backend point uuid of a circle/arc/ellipse center (BUG-5: the ownership link
   *  the wire now carries alongside the inlined `center` coords; absent on a legacy
   *  worker). */
  centerRef?: string;
  radius?: number;
  startAngle?: number;
  endAngle?: number;
  /** Ellipse semi-axes + rotation (radians). */
  majorR?: number;
  minorR?: number;
  rotation?: number;
  construction?: boolean;
  /** SCHEMA §7.3: emitted ONLY when true. Absent ⇒ false. */
  referenceLocked?: boolean;
}

/** A backend point uuid's owner: the entity that carries it as `position`. */
interface PointOwner {
  entityId: string;
  position: ConstraintPosition;
}

/**
 * Map every backend point uuid on the wire to its OWNER entity + position (BUG-5):
 *   - a Line's `p0Ref`/`p1Ref` → `(lineId, Start|End)`;
 *   - a Circle/Arc/Ellipse's `centerRef` → `(id, Center)`;
 *   - a free-standing Point (NOT referenced by any of the above) → `(id, Start)`.
 * `ownedChildIds` is the subset of point uuids OWNED by a line/circle/arc/ellipse —
 * these are NOT hydrated as independent frontend entities (their coords ride on the
 * owner).
 */
function ownershipFromWire(wire: WireDtoEntity[]): {
  owners: Map<string, PointOwner>;
  ownedChildIds: Set<string>;
} {
  const owners = new Map<string, PointOwner>();
  const ownedChildIds = new Set<string>();
  for (const e of wire) {
    if (!e || typeof e.id !== "string") continue;
    if (e.type === "Line" && e.p0Ref && e.p1Ref) {
      owners.set(e.p0Ref, { entityId: e.id, position: "Start" });
      owners.set(e.p1Ref, { entityId: e.id, position: "End" });
      ownedChildIds.add(e.p0Ref);
      ownedChildIds.add(e.p1Ref);
    } else if ((e.type === "Circle" || e.type === "Arc" || e.type === "Ellipse") && e.centerRef) {
      owners.set(e.centerRef, { entityId: e.id, position: "Center" });
      ownedChildIds.add(e.centerRef);
    }
  }
  // Free-standing points key to THEMSELVES (position Start, matching entityPoints)
  // — but only when no line/circle/arc already owns them.
  for (const e of wire) {
    if (e && e.type === "Point" && typeof e.id === "string" && e.at && !ownedChildIds.has(e.id)) {
      owners.set(e.id, { entityId: e.id, position: "Start" });
    }
  }
  return { owners, ownedChildIds };
}

/**
 * Reverse-map the worker-wire entities `enter_sketch` returns into the frontend
 * inlined-coordinate form the engine renders. A fresh sketch returns `[]` (the M2
 * slice); re-entering a sketch WITH geometry resolves Line `p0Ref`/`p1Ref` against
 * the returned Point entities. Arc endpoints are derived from center + angles.
 */
export function frontendEntitiesFromDto(dtoEntities: unknown): SketchEntity[] {
  if (!Array.isArray(dtoEntities)) return [];
  const wire = dtoEntities as WireDtoEntity[];
  const pointAt = new Map<string, [number, number]>();
  for (const e of wire) if (e.type === "Point" && e.at) pointAt.set(e.id, e.at);
  // BUG-5: a point OWNED by a line endpoint / circle-arc center is NOT a
  // free-standing entity — its coords ride on the owner (no stray dots on re-entry).
  const { ownedChildIds } = ownershipFromWire(wire);

  const out: SketchEntity[] = [];
  for (const e of wire) {
    // `referenceLocked` rides along on every kind — it drives rendering (solid,
    // dimmed) and the marshaller's destructive-op latch, so dropping it here
    // would make locked geometry indistinguishable from free geometry on re-entry.
    const referenceLocked = e.referenceLocked;
    switch (e.type) {
      case "Point":
        if (e.at && !ownedChildIds.has(e.id))
          out.push({ id: e.id, type: "Point", p0: e.at, construction: e.construction, referenceLocked });
        break;
      case "Line": {
        const p0 = e.p0Ref ? pointAt.get(e.p0Ref) : undefined;
        const p1 = e.p1Ref ? pointAt.get(e.p1Ref) : undefined;
        if (p0 && p1)
          out.push({ id: e.id, type: "Line", p0, p1, construction: e.construction, referenceLocked });
        break;
      }
      case "Circle":
        if (e.center && e.radius !== undefined)
          out.push({
            id: e.id,
            type: "Circle",
            center: e.center,
            radius: e.radius,
            construction: e.construction,
            referenceLocked,
          });
        break;
      case "Ellipse":
        if (e.center && e.majorR !== undefined && e.minorR !== undefined)
          out.push({
            id: e.id,
            type: "Ellipse",
            center: e.center,
            majorR: e.majorR,
            minorR: e.minorR,
            rotation: e.rotation ?? 0,
            construction: e.construction,
            referenceLocked,
          });
        break;
      case "Arc":
        if (e.center && e.radius !== undefined) {
          const at = (a?: number): [number, number] => [
            e.center![0] + e.radius! * Math.cos(a ?? 0),
            e.center![1] + e.radius! * Math.sin(a ?? 0),
          ];
          out.push({
            id: e.id,
            type: "Arc",
            center: e.center,
            radius: e.radius,
            start: at(e.startAngle),
            end: at(e.endAngle),
            construction: e.construction,
            referenceLocked,
          });
        }
        break;
    }
  }
  return out;
}

// ── Solved-positions reverse map (F-WP9: "solvedPositions reverse map missing") ─
//
// The worker keys `solvedPositions` (SketchUpsert/EndGesture) by backend POINT-
// entity UUID (dto.rs `solved_positions` "keyed by the point entity id"). The
// frontend never had a reverse map, so a solve/drag write-back never moved the
// geometry. These two pure steps close it: (1) re-key backend UUID → frontend
// `entityId.Position` via the id-map's `point` map; (2) apply those to the
// frontend entities (move line endpoints / circle+arc centers / points).

/**
 * A worker-minted ARC ENDPOINT handle (`"<arcWireId>.start"` / `".end"`, SCHEMA
 * §7.4) — never a backend point uuid, so it can never resolve through `map.point`.
 * Every arc-touching drag now reports these, and the `curves` channel supersedes
 * them, so they are expected rather than anomalous (see the seam note below).
 */
const ARC_ENDPOINT_HANDLE = /\.(start|end)$/;

/**
 * Re-key backend-UUID-keyed `solvedPositions` to the frontend `entityId.Position`
 * keys the entities carry, using `map.point` (frontend `"entityId.Position"` →
 * backend point UUID). Keys not in the id-map are skipped silently with a dev
 * warn (common on re-entry, before the id-map is seeded — the DTO entities are
 * already solved there, so no movement is needed).
 *
 * SEAM (W0b) — CLOSED by SP-2's `curves` channel: an arc's endpoints are minted
 * by the WORKER, whose primary handle for them is `"<arcWireId>.start"`/`".end"`
 * (SCHEMA §7.4), not a backend point uuid, so they arrive here as unmapped keys
 * and are skipped. They are no longer LOST: SCHEMA §7.4 `curves` reports the arc's
 * solved radius + start/end ANGLES on the very same response, for every drag kind,
 * and {@link applySolvedCurves} rebuilds the `start`/`end` coordinates from them —
 * the one path on which radius, angles and coordinates cannot disagree. Those
 * handles are therefore EXPECTED here (every arc-touching drag reports them) and
 * are filtered out of the unmapped-key warning below rather than flagged.
 */
export function frontendSolvedPositions(
  map: SketchIdMap,
  dtoPositions: Record<string, [number, number]> | undefined | null,
): Record<string, [number, number]> {
  const out: Record<string, [number, number]> = {};
  if (!dtoPositions) return out;
  // Reverse map.point: backend point UUID → frontend "entityId.Position".
  const byUuid = new Map<string, string>();
  for (const [frontendKey, uuid] of map.point) if (!byUuid.has(uuid)) byUuid.set(uuid, frontendKey);
  const unknown: string[] = [];
  for (const [uuid, xy] of Object.entries(dtoPositions)) {
    const key = byUuid.get(uuid);
    if (key) out[key] = xy;
    else if (!ARC_ENDPOINT_HANDLE.test(uuid)) unknown.push(uuid);
  }
  if (unknown.length > 0) {
    logWarn("sketch", `solvedPositions: ${unknown.length} unmapped point key(s) skipped`, {
      unknown: unknown.slice(0, 8),
    });
  }
  return out;
}

/**
 * Re-key backend-UUID conflicting constraint ids (SCHEMA §7.4 `conflicting[]`,
 * emitted by SketchUpsert/SolveDrag/EndGesture) to the FRONTEND constraint ids the
 * inspector/badges render, by reversing `map.constraint` (frontend id → backend
 * UUID). Ids not in the id-map are DROPPED (a constraint the frontend never
 * authored/hydrated has nothing to tint). Pure; order-preserving; deduped.
 */
export function frontendConflictingIds(
  map: SketchIdMap,
  ids: string[] | undefined | null,
): string[] {
  if (!ids || ids.length === 0) return [];
  // Reverse map.constraint: backend UUID → frontend id (first wins on a dup value).
  const byUuid = new Map<string, string>();
  for (const [frontendId, uuid] of map.constraint) if (!byUuid.has(uuid)) byUuid.set(uuid, frontendId);
  const out: string[] = [];
  const seen = new Set<string>();
  for (const uuid of ids) {
    const fid = byUuid.get(uuid);
    if (fid && !seen.has(fid)) {
      seen.add(fid);
      out.push(fid);
    }
  }
  return out;
}

/**
 * Apply frontend-keyed (`"entityId.Position"`) solved positions to the sketch
 * entities, moving each entity's geometry per kind (Line Start/End → p0/p1;
 * Circle/Arc Center → center; Point Start/Center → p0). Pure: returns a NEW array
 * only when something moved (else the same reference — no React churn). Positions
 * for unknown entity ids are ignored.
 */
export function applySolvedPositions(
  entities: SketchEntity[],
  positions: Record<string, [number, number]>,
): SketchEntity[] {
  const keys = Object.keys(positions);
  if (keys.length === 0) return entities;
  // Group positions by frontend entity id.
  const byEntity = new Map<string, Record<string, [number, number]>>();
  for (const key of keys) {
    const dot = key.lastIndexOf(".");
    if (dot < 0) continue;
    const entityId = key.slice(0, dot);
    const position = key.slice(dot + 1);
    const g = byEntity.get(entityId) ?? {};
    g[position] = positions[key];
    byEntity.set(entityId, g);
  }
  let changed = false;
  const out = entities.map((e) => {
    const g = byEntity.get(e.id);
    if (!g) return e;
    const moved = moveEntity(e, g);
    if (moved !== e) changed = true;
    return moved;
  });
  return changed ? out : entities;
}

/** Move one entity's coordinates per its solved point positions (immutable). */
function moveEntity(e: SketchEntity, g: Record<string, [number, number]>): SketchEntity {
  switch (e.type) {
    case "Point": {
      const p0 = g.Start ?? g.Center;
      return p0 ? { ...e, p0 } : e;
    }
    case "Line": {
      if (!g.Start && !g.End) return e;
      return { ...e, p0: g.Start ?? e.p0, p1: g.End ?? e.p1 };
    }
    case "Circle":
    case "Arc":
    case "Ellipse":
      return g.Center ? { ...e, center: g.Center } : e;
    default:
      return e;
  }
}

// ── Solved-curves channel (SP-2; SCHEMA §7.4 `curves`) ───────────────────────
//
// `positions` is point-only, which is not the whole result of a drag: a `radius`
// gesture moves no point, an `arcEnd` gesture reshapes an arc's radius+angles
// (the Rust Arc owns no endpoint ENTITIES), and a `Tangent` propagates even a
// plain point drag into a neighbouring curve's radius. `curves` carries exactly
// those members, keyed by curve entity id, under the same incremental discipline.

/**
 * Re-key backend-UUID-keyed `curves` (SCHEMA §7.4, `SolveDrag`/`EndGesture`) to the
 * FRONTEND entity ids the sketch entities carry, by reversing `map.entity`. Keys not
 * in the id-map are dropped with a dev warn — unlike a stale POINT key, an unmapped
 * curve key means a solved radius/angle silently never lands (the exact staleness
 * this channel exists to end).
 */
export function frontendSolvedCurves(
  map: SketchIdMap,
  dtoCurves: Record<string, CurveParams> | undefined | null,
): Record<string, CurveParams> {
  const out: Record<string, CurveParams> = {};
  if (!dtoCurves) return out;
  const byUuid = new Map<string, string>();
  for (const [frontendId, uuid] of map.entity) if (!byUuid.has(uuid)) byUuid.set(uuid, frontendId);
  const unknown: string[] = [];
  for (const [uuid, params] of Object.entries(dtoCurves)) {
    const id = byUuid.get(uuid);
    if (id) out[id] = params;
    else unknown.push(uuid);
  }
  if (unknown.length > 0) {
    logWarn("sketch", `curves: ${unknown.length} unmapped curve key(s) skipped`, {
      unknown: unknown.slice(0, 8),
    });
  }
  return out;
}

/**
 * Re-key backend-UUID-keyed `entityStates` (SCHEMA §7.4) to the FRONTEND entity
 * ids the sketch entities carry, by reversing `map.entity` — the same keyspace
 * and the same reversal `frontendSolvedCurves` performs just above.
 *
 * An unmapped wire id is DROPPED, never guessed: absence from the map is the
 * protocol's own encoding of "unknown", so dropping degrades that entity to the
 * whole-sketch tint, which is exactly what "we have nothing to say about it"
 * should look like. Inventing a frontend id, or defaulting the entry to
 * `underConstrained`, would assert a diagnosis the solver never made.
 *
 * Unlike the curves channel this does NOT warn: a dropped state key costs a
 * shade of colour, not a solved radius that silently never lands, and this map
 * is republished on every solve — a transient id-map lag would otherwise warn on
 * an ordinary frame. An absent/null field is `{}` (older backend, or a worker
 * that omitted the map for an ellipse-bearing sketch).
 */
export function frontendEntityStates(
  map: SketchIdMap,
  dtoStates: SketchEntityStates | undefined | null,
): SketchEntityStates {
  const out: SketchEntityStates = {};
  if (!dtoStates) return out;
  const byUuid = new Map<string, string>();
  for (const [frontendId, uuid] of map.entity) if (!byUuid.has(uuid)) byUuid.set(uuid, frontendId);
  for (const [uuid, state] of Object.entries(dtoStates)) {
    const id = byUuid.get(uuid);
    if (id) out[id] = state;
  }
  return out;
}

/**
 * Re-key a `projections` map from backend ENTITY uuids to frontend ids (WP-P).
 * Same drop-unknown rule as `frontendEntityStates`. Without an id-map (a pure
 * `getSketch` read of a never-entered sketch) the frontend ids ARE the backend
 * uuids, so the map passes through unchanged. Values are backend-rendered ids
 * (`body_<uuid>`, ElementId) and are NOT re-keyed — they address the model, not
 * the sketch.
 */
export function frontendProjections(
  map: SketchIdMap | undefined,
  dto: Record<string, ProjectedSource> | undefined | null,
): Record<string, ProjectedSource> | undefined {
  if (!dto) return undefined;
  if (!map) return { ...dto };
  const byUuid = new Map<string, string>();
  for (const [frontendId, uuid] of map.entity) if (!byUuid.has(uuid)) byUuid.set(uuid, frontendId);
  const out: Record<string, ProjectedSource> = {};
  for (const [uuid, source] of Object.entries(dto)) {
    const id = byUuid.get(uuid);
    if (id) out[id] = source;
  }
  return out;
}

/**
 * Apply frontend-keyed solved `curves` to the sketch entities: a Circle's radius,
 * an Arc's radius + start/end ANGLES — and, because a frontend Arc stores endpoint
 * COORDINATES rather than angles, the recomputed `start`/`end` points.
 *
 * That recomputation is the exact inverse of the `at()` helper
 * `frontendEntitiesFromDto` uses to derive an arc's endpoints on re-entry
 * (`center + radius·[cos θ, sin θ]`; see the `case "Arc"` branch above, the
 * `const at = …` lines). The two MUST stay consistent: they are the only two
 * places that turn (center, radius, angle) into a coordinate, and a drag that
 * disagreed with re-entry would make the arc jump when the sketch is reopened.
 *
 * An unspecified member is UNCHANGED (§7.4 incremental discipline), so a
 * radius-only report keeps the current angles and vice versa. Pure: returns a NEW
 * array only when something changed (else the same reference — no React churn).
 * Ids that are unknown, non-curve, or carry no usable member are ignored.
 */
export function applySolvedCurves(
  entities: SketchEntity[],
  curves: Record<string, CurveParams> | undefined | null,
): SketchEntity[] {
  if (!curves || Object.keys(curves).length === 0) return entities;
  let changed = false;
  const out = entities.map((e) => {
    const c = curves[e.id];
    if (!c) return e;
    const next = reshapeCurve(e, c);
    if (next !== e) changed = true;
    return next;
  });
  return changed ? out : entities;
}

/** A finite number, or undefined (a non-finite member is dropped, never applied). */
function finite(v: number | undefined): number | undefined {
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

/** The angle of `p` about `center`, matching `at()`'s convention (absent ⇒ 0). */
function angleAbout(center: [number, number], p: [number, number] | undefined): number {
  return p ? Math.atan2(p[1] - center[1], p[0] - center[0]) : 0;
}

/** Reshape one Circle/Arc from its CHANGED curve members (immutable). */
function reshapeCurve(e: SketchEntity, c: CurveParams): SketchEntity {
  const radius = finite(c.radius);
  // Clamp ≥0 mirrors the core's `apply_solved_curves` (a negative radius is not
  // geometry); the 0.01mm floor is the BACKEND's, applied before the solve.
  const clamped = radius === undefined ? undefined : Math.max(radius, 0);
  if (e.type === "Circle") return clamped === undefined ? e : { ...e, radius: clamped };
  if (e.type !== "Arc") return e; // Ellipse is never solver-registered (§7.4)
  const startAngle = finite(c.startAngle);
  const endAngle = finite(c.endAngle);
  if (clamped === undefined && startAngle === undefined && endAngle === undefined) return e;
  const center = e.center;
  const r = clamped ?? e.radius;
  if (!center || r === undefined) return e;
  // Inverse of frontendEntitiesFromDto's `at()` — keep the two in step.
  const at = (a: number): [number, number] => [
    center[0] + r * Math.cos(a),
    center[1] + r * Math.sin(a),
  ];
  return {
    ...e,
    radius: r,
    start: at(startAngle ?? angleAbout(center, e.start)),
    end: at(endAngle ?? angleAbout(center, e.end)),
  };
}

/** One constraint from the worker wire form (`enter_sketch` returns these — the
 *  Rust `wire_constraint` shape: `{id, type, entities, positions?, value?}`). */
interface WireDtoConstraint {
  id: string;
  type: string;
  entities: string[];
  positions?: string[];
  value?: number;
}

/**
 * Reverse-map the worker-wire constraints `enter_sketch` returns into the frontend
 * `SketchConstraint` form. The wire shape is field-identical to the frontend type
 * (id / PascalCase type / entity refs / optional positions + value), so this is a
 * validated pass-through: entries with an unknown `type` or a missing id/entities
 * are dropped. The referenced ids are backend UUIDs (kept verbatim); the inspector
 * summarizes constraints by type, and the marshaller only re-adds constraints it
 * has NOT already seen, so re-entry constraints hydrate the panel without churn.
 */
export function frontendConstraintsFromDto(
  dtoConstraints: unknown,
  dtoEntities?: unknown,
): SketchConstraint[] {
  if (!Array.isArray(dtoConstraints)) return [];
  // BUG-5: the wire references POINTS by backend uuid; a fresh sketch references the
  // OWNER entity + position (`{entityId, position}`). Remap every point uuid back to
  // its owner so the hydrated constraints land in the same shape as a fresh sketch
  // (and don't dangle now that child points aren't independent entities).
  const owners = Array.isArray(dtoEntities)
    ? ownershipFromWire(dtoEntities as WireDtoEntity[]).owners
    : null;
  const out: SketchConstraint[] = [];
  for (const raw of dtoConstraints as WireDtoConstraint[]) {
    if (!raw || typeof raw.id !== "string" || !CONSTRAINT_TYPES.has(raw.type)) continue;
    if (!Array.isArray(raw.entities)) continue;
    const type = raw.type as SketchConstraintType;
    const c: SketchConstraint = { id: raw.id, type, entities: raw.entities };
    // The wire may carry positions of its own — W0b arc-endpoint Coincidents ride
    // as "the ARC's uuid + Start/End", and that role exists ONLY in `positions`
    // (the ref is already an entity id, so the owner map has nothing to say about
    // it). Merge: owner-derived wins where a slot names an owned child POINT,
    // wire-carried fills every other slot. Dropping the wire's roles here is what
    // would make a welded slot lose its caps on re-entry.
    const wirePosition = (i: number): ConstraintPosition | "" => {
      const p = Array.isArray(raw.positions) ? raw.positions[i] : undefined;
      return p === "Start" || p === "End" || p === "Center" || p === "Midpoint" ? p : "";
    };
    if (owners && raw.entities.some((e) => owners.has(e))) {
      // Remap owned point refs → owner entity id; non-point refs (lines/circles) stay.
      c.entities = raw.entities.map((e) => owners.get(e)?.entityId ?? e);
      // A `""` slot is falsy, so `resolveRef` resolves it as an ENTITY ref (matches a
      // direct line/circle operand, e.g. Midpoint's line or Symmetric's axis).
      const positions = raw.entities.map((e, i) => owners.get(e)?.position ?? wirePosition(i));
      if (positions.some((p) => p !== "")) c.positions = positions as ConstraintPosition[];
    } else if (Array.isArray(raw.positions)) {
      const positions = raw.entities.map((_, i) => wirePosition(i));
      if (positions.some((p) => p !== "")) c.positions = positions as ConstraintPosition[];
    }
    // Dimensional value arrives in the WIRE domain (Angle: rad→deg; BUG-2).
    if (typeof raw.value === "number") c.value = fromWireDimensionValue(type, raw.value);
    out.push(c);
  }
  return out;
}

// ── Re-entry hydration: seed the id-map from the enter_sketch wire ────────────
//
// After a reopen the in-memory id-map is empty, but `enter_sketch` returns the
// sketch's REAL geometry. Without seeding, the next `marshalUpsert` would treat
// every hydrated entity/constraint as new and re-emit it as an Add op (SCHEMA
// §7.4) — duplicating the sketch. This seeds the map so a re-marshal of the same
// hydrated arrays diffs to ZERO ops. The wire ids ARE the authoritative backend
// ids, so everything maps 1:1 (unlike the frontend-authoring path, which mints).
//
// The inclusion tests MIRROR `frontendEntitiesFromDto`/`frontendConstraintsFromDto`
// exactly: only entities/constraints that reverse-map into the frontend session
// are seeded, so the seeded set == the live frontend set (else marshalUpsert would
// spuriously Remove a seeded-but-absent id). Pure: no store access.

/**
 * REBASE `map`'s id bindings onto the worker-wire entities + constraints
 * `enter_sketch` returns — an authoritative REPLACE, not a merge.
 *
 * The wire is the whole truth for this sketch, and the frontend session is
 * rebuilt from it (`frontendEntitiesFromDto`), so every id the previous session
 * minted (`e1`, `c1`, …) is gone from the live arrays the moment the session
 * re-opens. Leaving those bindings behind made `marshalUpsert`'s removals-first
 * diff read them as "present in the map, absent from `next`" and emit a
 * `removeEntity` for each — so the FIRST edit after re-entering a sketch deleted
 * everything drawn in the previous session (the user-visible "only the
 * latest-drawn object survives"). Clearing first is what makes the map describe
 * the sketch that actually exists.
 *
 * `backendSketchId` / `planeKind` are session-independent and survive.
 *
 * Key/value conventions mirror `addEntityOps`, with the wire (backend) ids:
 *   - every entity id → itself (`map.entity`, 1:1);
 *   - Point   → `${id}.Start` + `${id}.Center` → its own id (points key under both);
 *   - Line    → `${id}.Start`/`${id}.End` → the `p0Ref`/`p1Ref` backend point ids;
 *   - Circle/Arc → the wire inlines the center as COORDS (no center point id), so
 *     `${id}.Center` cannot be keyed to a backend point uuid here; the center Point
 *     is emitted separately in the entities list and seeded via its own id (a
 *     re-entry circle/arc center is not directly draggable until the wire carries
 *     the center ref — documented seam);
 *   - Ellipse → identical to Circle/Arc: the id maps 1:1 and `${id}.Center` binds
 *     to the wire's `centerRef` (W3 P3);
 *   - every constraint id → itself (`map.constraint`) + its value into the
 *     `constraintValue` cache, so an in-place `SetDimension` diff is a no-op.
 * Idempotent (re-seeding the same wire is harmless); an EMPTY wire (a fresh
 * sketch) leaves the map empty — which is the point: nothing exists to bind.
 */
export function seedIdMapFromWire(
  map: SketchIdMap,
  dtoEntities: unknown,
  dtoConstraints: unknown,
): void {
  // Authoritative rebase: drop every binding from a prior session BEFORE seeding
  // (see the block comment above — stale ids marshal as removals and delete the
  // geometry they name).
  map.entity.clear();
  map.point.clear();
  map.constraint.clear();
  map.constraintValue.clear();
  map.entityConstruction.clear();
  map.entityReferenceLocked.clear();

  if (Array.isArray(dtoEntities)) {
    const wire = dtoEntities as WireDtoEntity[];
    // Point ids that carry a position back a resolvable line endpoint (matches
    // `frontendEntitiesFromDto`'s `pointAt` gate so the seeded set stays aligned).
    const hasPoint = new Set<string>();
    for (const e of wire) if (e && e.type === "Point" && e.at) hasPoint.add(e.id);
    // BUG-5: a point owned by a line/circle/arc is NOT a free-standing entity — it is
    // reachable ONLY via its owner's `${ownerId}.${position}` key (seeded below), so
    // it must NOT be seeded as an entity (mirrors `frontendEntitiesFromDto`).
    const { ownedChildIds } = ownershipFromWire(wire);

    for (const e of wire) {
      if (!e || typeof e.id !== "string") continue;
      switch (e.type) {
        case "Point":
          if (!e.at || ownedChildIds.has(e.id)) continue;
          map.entity.set(e.id, e.id);
          map.point.set(pointKey(e.id, "Start"), e.id);
          map.point.set(pointKey(e.id, "Center"), e.id);
          break;
        case "Line":
          if (!e.p0Ref || !e.p1Ref || !hasPoint.has(e.p0Ref) || !hasPoint.has(e.p1Ref)) continue;
          map.entity.set(e.id, e.id);
          map.point.set(pointKey(e.id, "Start"), e.p0Ref);
          map.point.set(pointKey(e.id, "End"), e.p1Ref);
          break;
        case "Ellipse":
          // Mirrors `frontendEntitiesFromDto`'s Ellipse gate (centre + both radii),
          // so the seeded set stays exactly the hydrated set.
          if (!e.center || e.majorR === undefined || e.minorR === undefined) continue;
          map.entity.set(e.id, e.id);
          if (e.centerRef) map.point.set(pointKey(e.id, "Center"), e.centerRef);
          break;
        case "Circle":
        case "Arc":
          if (!e.center || e.radius === undefined) continue;
          map.entity.set(e.id, e.id);
          // BUG-5: the wire now carries the center point uuid (`centerRef`) alongside
          // the inlined coords, so `.Center` resolves to the backend point on re-entry
          // (a re-entry circle/arc center is now directly draggable). Absent on a
          // legacy worker → unseeded (the prior documented seam).
          if (e.centerRef) map.point.set(pointKey(e.id, "Center"), e.centerRef);
          break;
      }
      // W1-B: seed the last-SENT construction flag from the wire (only for entities
      // the switch actually bound — every skipped case `continue`s). Mirrors
      // `frontendEntitiesFromDto`, which carries the same flag onto the live
      // session, so the first marshal after re-entry emits no spurious flip.
      if (map.entity.has(e.id)) {
        map.entityConstruction.set(e.id, !!e.construction);
        // W0b: latch the reference lock the same way, so the marshaller can
        // refuse destructive ops against host-face geometry.
        if (e.referenceLocked) map.entityReferenceLocked.add(e.id);
      }
    }
  }

  if (Array.isArray(dtoConstraints)) {
    for (const raw of dtoConstraints as WireDtoConstraint[]) {
      if (!raw || typeof raw.id !== "string") continue;
      if (!CONSTRAINT_TYPES.has(raw.type) || !Array.isArray(raw.entities)) continue;
      map.constraint.set(raw.id, raw.id);
      // BUG-2: the cache holds UI-domain values (Angle: rad→deg), so the first marshal
      // after re-entry does NOT emit a spurious SetDimension for every Angle.
      const value =
        typeof raw.value === "number"
          ? fromWireDimensionValue(raw.type as SketchConstraintType, raw.value)
          : undefined;
      map.constraintValue.set(raw.id, value);
    }
  }
}
