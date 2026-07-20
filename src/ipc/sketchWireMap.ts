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
 * its `center` point (endpoints are derived from angles), so a Coincident on an
 * arc START/END has no point id — skipped with a note; (c) Ellipse is outside the
 * slice. These only surface for arc-heavy sketches; the M2 slice (line/rect/circle
 * profiles → extrude) is fully covered.
 */
import type {
  ConstraintPosition,
  SketchConstraint,
  SketchConstraintType,
  SketchEntity,
  SketchPlaneKind,
} from "./types";
import { planeFor } from "./mockSketch";
import { fromWireDimensionValue, toWireDimensionValue } from "./angleUnits";

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
export type WireSketchEntity = WirePoint | WireLine | WireCircle | WireArc;

/** A Rust `Constraint` (internally tagged `"kind"`, camelCase). Only the kinds the
 *  vertical-slice tools author are modeled; others route through a best-effort. */
export type WireConstraint =
  | { kind: "coincident"; id: string; point1: string; point2: string }
  | { kind: "horizontal"; id: string; line: string }
  | { kind: "vertical"; id: string; line: string }
  | { kind: "fixed"; id: string; point: string; at: [number, number] }
  | { kind: "midpoint"; id: string; point: string; line: string }
  | { kind: "parallel"; id: string; line1: string; line2: string }
  | { kind: "perpendicular"; id: string; line1: string; line2: string }
  | { kind: "equal"; id: string; entity1: string; entity2: string }
  | { kind: "distance"; id: string; entity1: string; entity2: string; value: WireScalar }
  | { kind: "horizontalDistance"; id: string; point1: string; point2: string; value: WireScalar }
  | { kind: "verticalDistance"; id: string; point1: string; point2: string; value: WireScalar }
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
  | { op: "setEntityPositions"; positions: [string, [number, number]][] };

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
}

export function createIdMap(backendSketchId: string, planeKind: SketchPlaneKind): SketchIdMap {
  return {
    backendSketchId,
    planeKind,
    entity: new Map(),
    point: new Map(),
    constraint: new Map(),
    constraintValue: new Map(),
  };
}

const pointKey = (entityId: string, position: ConstraintPosition): string =>
  `${entityId}.${position}`;

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
      map.entity.set(e.id, id);
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
      map.entity.set(e.id, id);
      ops.push({ op: "addEntity", entity: { kind: "line", id, start, end, construction } });
      return ops;
    }
    case "Circle": {
      if (!e.center || e.radius === undefined) return [];
      const center = mintPoint("Center", e.center);
      const id = mint();
      map.entity.set(e.id, id);
      ops.push({ op: "addEntity", entity: { kind: "circle", id, center, radius: e.radius, construction } });
      return ops;
    }
    case "Arc": {
      if (!e.center || e.radius === undefined) return [];
      const center = mintPoint("Center", e.center);
      const id = mint();
      map.entity.set(e.id, id);
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
 *  ref is a POINT — resolve STRICTLY to the synthesized point uuid (an arc START/
 *  END has none → `null`, so the constraint is skipped). Without a selector the
 *  ref is the entity itself (e.g. a Point entity or a whole line). */
function resolveRef(
  map: SketchIdMap,
  entityId: string,
  position?: ConstraintPosition,
): string | null {
  if (position) return map.point.get(pointKey(entityId, position)) ?? null;
  return map.entity.get(entityId) ?? null;
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
 *  be expressed — e.g. an arc-endpoint Coincident, reported as an M2 seam). */
function toWireConstraint(map: SketchIdMap, c: SketchConstraint, id: string): WireConstraint | null {
  const pos = c.positions ?? [];
  const ref = (i: number): string | null => resolveRef(map, c.entities[i], pos[i]);
  // Dimensional value crosses in the WIRE domain (Angle: deg→rad; BUG-2).
  const val: WireScalar = { value: toWireDimensionValue(c.type, c.value ?? 0) };

  switch (c.type) {
    case "Coincident": {
      const p1 = ref(0);
      const p2 = ref(1);
      return p1 && p2 ? { kind: "coincident", id, point1: p1, point2: p2 } : null;
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
      const point = ref(0);
      const line = map.entity.get(c.entities[1]);
      return point && line ? { kind: "midpoint", id, point, line } : null;
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
      const e1 = ref(0);
      const e2 = ref(1);
      return e1 && e2 ? { kind: "distance", id, entity1: e1, entity2: e2, value: val } : null;
    }
    case "HorizontalDistance": {
      const p1 = ref(0);
      const p2 = ref(1);
      return p1 && p2 ? { kind: "horizontalDistance", id, point1: p1, point2: p2, value: val } : null;
    }
    case "VerticalDistance": {
      const p1 = ref(0);
      const p2 = ref(1);
      return p1 && p2 ? { kind: "verticalDistance", id, point1: p1, point2: p2, value: val } : null;
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
    // Fixed / OnCurve / Tangent / Concentric / Symmetric need point coords or
    // curve refs the frontend authoring path does not yet produce in the slice.
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
export function marshalUpsert(
  map: SketchIdMap,
  next: { entities: SketchEntity[]; constraints: SketchConstraint[] },
  mint: () => string = mintUuid,
): SketchEditOp[] {
  const ops: SketchEditOp[] = [];

  // Removals first (so re-adds of a reused id are unambiguous).
  const liveEntities = new Set(next.entities.map((e) => e.id));
  for (const [fid, uuid] of [...map.entity]) {
    if (!liveEntities.has(fid)) {
      ops.push({ op: "removeEntity", entity: uuid });
      map.entity.delete(fid);
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

  // Additions (entities before constraints so refs resolve).
  for (const e of next.entities) {
    if (!map.entity.has(e.id)) ops.push(...addEntityOps(map, e, mint));
  }
  for (const c of next.constraints) {
    if (!map.constraint.has(c.id)) {
      const id = mint();
      const wire = toWireConstraint(map, c, id);
      if (wire) {
        map.constraint.set(c.id, id);
        map.constraintValue.set(c.id, c.value);
        ops.push({ op: "addConstraint", constraint: wire });
      }
      // Unmappable constraints (e.g. arc-endpoint coincidence) are skipped — the
      // solver still runs on the geometry; documented M2 seam.
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
    attachment: { kind: "world"; plane: "XY" | "XZ" | "YZ" };
  };
}

/** Build the `AddSketch` EditCommand for a new world-plane sketch. `custom` planes
 *  fall back to XY (host-face/datum attachment is an M2+ concern). The plane basis
 *  is the SCHEMA §7.3 canonical basis for the kind (same table Rust's
 *  `SketchPlane::xy/xz/yz()` carry — `planeFor` mirrors it verbatim). */
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
  type: "Point" | "Line" | "Circle" | "Arc";
  at?: [number, number];
  p0Ref?: string;
  p1Ref?: string;
  center?: [number, number];
  /** Backend point uuid of a circle/arc center (BUG-5: the ownership link the wire
   *  now carries alongside the inlined `center` coords; absent on a legacy worker). */
  centerRef?: string;
  radius?: number;
  startAngle?: number;
  endAngle?: number;
  construction?: boolean;
}

/** A backend point uuid's owner: the entity that carries it as `position`. */
interface PointOwner {
  entityId: string;
  position: ConstraintPosition;
}

/**
 * Map every backend point uuid on the wire to its OWNER entity + position (BUG-5):
 *   - a Line's `p0Ref`/`p1Ref` → `(lineId, Start|End)`;
 *   - a Circle/Arc's `centerRef` → `(id, Center)`;
 *   - a free-standing Point (NOT referenced by any of the above) → `(id, Start)`.
 * `ownedChildIds` is the subset of point uuids OWNED by a line/circle/arc — these
 * are NOT hydrated as independent frontend entities (their coords ride on the owner).
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
    } else if ((e.type === "Circle" || e.type === "Arc") && e.centerRef) {
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
    switch (e.type) {
      case "Point":
        if (e.at && !ownedChildIds.has(e.id))
          out.push({ id: e.id, type: "Point", p0: e.at, construction: e.construction });
        break;
      case "Line": {
        const p0 = e.p0Ref ? pointAt.get(e.p0Ref) : undefined;
        const p1 = e.p1Ref ? pointAt.get(e.p1Ref) : undefined;
        if (p0 && p1) out.push({ id: e.id, type: "Line", p0, p1, construction: e.construction });
        break;
      }
      case "Circle":
        if (e.center && e.radius !== undefined)
          out.push({ id: e.id, type: "Circle", center: e.center, radius: e.radius, construction: e.construction });
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
 * Re-key backend-UUID-keyed `solvedPositions` to the frontend `entityId.Position`
 * keys the entities carry, using `map.point` (frontend `"entityId.Position"` →
 * backend point UUID). Keys not in the id-map are skipped silently with a dev
 * warn (common on re-entry, before the id-map is seeded — the DTO entities are
 * already solved there, so no movement is needed).
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
    else unknown.push(uuid);
  }
  if (unknown.length > 0 && typeof import.meta !== "undefined" && import.meta.env?.DEV) {
    // eslint-disable-next-line no-console
    console.warn(`[sketchWireMap] solvedPositions: ${unknown.length} unmapped point key(s) skipped`);
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
      return g.Center ? { ...e, center: g.Center } : e;
    default:
      return e;
  }
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
    if (owners && raw.entities.some((e) => owners.has(e))) {
      // Remap owned point refs → owner entity id; non-point refs (lines/circles) stay.
      c.entities = raw.entities.map((e) => owners.get(e)?.entityId ?? e);
      // A `""` slot is falsy, so `resolveRef` resolves it as an ENTITY ref (matches a
      // direct line/circle operand, e.g. Midpoint's line or Symmetric's axis).
      const positions = raw.entities.map((e) => owners.get(e)?.position ?? "");
      if (positions.some((p) => p !== "")) c.positions = positions as ConstraintPosition[];
    } else if (Array.isArray(raw.positions)) {
      c.positions = raw.positions.filter(
        (p): p is ConstraintPosition =>
          p === "Start" || p === "End" || p === "Center" || p === "Midpoint",
      );
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
 * Seed `map` from the worker-wire entities + constraints `enter_sketch` returns,
 * mirroring `addEntityOps`' key/value conventions with the wire (backend) ids:
 *   - every entity id → itself (`map.entity`, 1:1);
 *   - Point   → `${id}.Start` + `${id}.Center` → its own id (points key under both);
 *   - Line    → `${id}.Start`/`${id}.End` → the `p0Ref`/`p1Ref` backend point ids;
 *   - Circle/Arc → the wire inlines the center as COORDS (no center point id), so
 *     `${id}.Center` cannot be keyed to a backend point uuid here; the center Point
 *     is emitted separately in the entities list and seeded via its own id (a
 *     re-entry circle/arc center is not directly draggable until the wire carries
 *     the center ref — documented seam);
 *   - every constraint id → itself (`map.constraint`) + its value into the
 *     `constraintValue` cache, so an in-place `SetDimension` diff is a no-op.
 * Idempotent (re-seeding the same wire is harmless) and a no-op for a fresh sketch
 * (empty entities/constraints).
 */
export function seedIdMapFromWire(
  map: SketchIdMap,
  dtoEntities: unknown,
  dtoConstraints: unknown,
): void {
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
