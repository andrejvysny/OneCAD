/*
 * Mock sketch "solver" + region-detection HELPERS (MOCK-ONLY, no real geometry
 * kernel). `detectRegions` itself lives in `./mockRegions` — split out so this
 * module (imported by every mock-lane consumer) does not drag three.js's
 * `ShapeUtils`/`Vector2` into the app entry chunk.
 *
 * The real solver is the C++ worker's PlaneGCS actor (SCHEMA §7.4). Until it is
 * wired in, these pure functions give the frontend a plausible, deterministic
 * stand-in so the whole sketch UX (DOF badge, constraint state colors, extrude
 * profile preview) runs with no backend. LIMITS are documented per function.
 */
import type {
  SketchConstraint,
  SketchEntity,
  SketchEntityStates,
  SketchPlane,
  SketchPlaneKind,
  SketchSolveStatus,
} from "./types";
import { ellipseParams, sampleEllipse } from "@/tools/sketch/ellipseMath";
import { visualSegmentsForClosedCurve } from "@/tools/preview/visualTessellation";
import { detectConflicts } from "./mockConflicts";

// ── Canonical planes — SCHEMA §7.3 EXACT bases (non-standard XY basis) ───────

const PLANES: Record<Exclude<SketchPlaneKind, "custom">, Omit<SketchPlane, "kind">> = {
  // User X → World Y+, User Y → World X− (ported verbatim from Sketch.h XY()).
  XY: { origin: [0, 0, 0], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
  XZ: { origin: [0, 0, 0], xAxis: [0, 1, 0], yAxis: [0, 0, 1], normal: [1, 0, 0] },
  YZ: { origin: [0, 0, 0], xAxis: [-1, 0, 0], yAxis: [0, 0, 1], normal: [0, 1, 0] },
};

export function planeFor(kind: SketchPlaneKind): SketchPlane {
  const base = PLANES[kind === "custom" ? "XY" : kind];
  return { kind, ...structuredCloneLite(base) };
}

// ── Naive DOF heuristic (MOCK) ───────────────────────────────────────────────
//
// Real DOF comes from the constraint solver's Jacobian rank. The mock uses a
// coarse "2·points (+radius/angle terms) − removed" count: it moves the right
// direction (DOF falls as constraints are added, hits 0 when fully defined) but
// is NOT geometrically exact. It never reports Conflicting (that needs a real
// solver detecting contradictory equations).

/** Free parameters an entity contributes: 2 per point, +1 per radius/angle. */
export function entityFreedom(e: SketchEntity): number {
  switch (e.type) {
    case "Point":
      return 2; // 1 point
    case "Line":
      return 4; // 2 endpoints
    case "Circle":
      return 3; // center (2) + radius (1)
    case "Ellipse":
      return 5; // center (2) + majorR + minorR + rotation (3)
    case "Arc":
      return 5; // center (2) + radius (1) + 2 sweep angles
    default:
      return 0;
  }
}

/** DOFs one constraint removes (coarse: 2 for pairing/fixing, else 1). */
export function constraintFreedom(c: SketchConstraint): number {
  switch (c.type) {
    case "Coincident":
    case "Fixed":
    case "Symmetric":
    case "Concentric":
    case "Midpoint":
      return 2;
    default:
      return 1;
  }
}

/** Total free parameters, ignoring the reference-lock rule (see `solveDof`). */
export function freeDegrees(entities: SketchEntity[]): number {
  return entities.reduce((sum, e) => sum + entityFreedom(e), 0);
}

export function removedDegrees(constraints: SketchConstraint[]): number {
  return constraints.reduce((sum, c) => sum + constraintFreedom(c), 0);
}

/**
 * Solve → {dof (clamped ≥0), status}. Signed surplus decides the state.
 *
 * REFERENCE-LOCK RULE (SKETCH-ON-FACE W2). A `referenceLocked` entity is
 * projected host-face geometry: it is pinned by `Fixed` constraints and cannot
 * move, so it must contribute ZERO net degrees of freedom. The coarse count is
 * therefore taken over the USER geometry only —
 *
 *   - a locked entity adds 0 free degrees, and
 *   - a constraint whose every referenced entity is locked removes 0
 *     (that is exactly the machine `Fixed` set the projection minted).
 *
 * Counting them naively would give a freshly projected rectangle 4·4 − 4·2 = 8
 * DOF, i.e. a sketch-on-face session that opens "under-constrained" with nothing
 * drawn — which the real solver never reports. A constraint the USER later adds
 * between their own geometry and the locked boundary still counts in full: it
 * has a non-locked operand, so it removes DOF from the user's side.
 */
export function solveDof(
  entities: SketchEntity[],
  constraints: SketchConstraint[],
): { dof: number; status: SketchSolveStatus } {
  const locked = new Set(entities.filter((e) => e.referenceLocked).map((e) => e.id));
  const free = entities.reduce((sum, e) => sum + (locked.has(e.id) ? 0 : entityFreedom(e)), 0);
  const removed = constraints.reduce((sum, c) => {
    const machine = c.entities.length > 0 && c.entities.every((id) => locked.has(id));
    return sum + (machine ? 0 : constraintFreedom(c));
  }, 0);
  const surplus = free - removed;
  const status: SketchSolveStatus =
    surplus > 0 ? "UnderConstrained" : surplus === 0 ? "FullyConstrained" : "OverConstrained";
  return { dof: Math.max(0, surplus), status };
}

/**
 * `solveDof` + deterministic conflict detection (`mockConflicts.ts`), combined
 * with the precedence the real worker uses (`SolverLane.cpp` `upsert_state`):
 * Conflicting > OverConstrained > FullyConstrained > UnderConstrained. This is
 * the mock lane's actual "solve" entry point — `solveDof` alone never reports
 * Conflicting and is kept that way (existing callers rely on it staying a pure
 * DOF heuristic); this wraps it rather than replacing it.
 */
export function solveSketch(
  entities: SketchEntity[],
  constraints: SketchConstraint[],
): { dof: number; status: SketchSolveStatus; conflicting: string[] } {
  const conflicting = detectConflicts(entities, constraints);
  const { dof, status } = solveDof(entities, constraints);
  return { dof, status: conflicting.length > 0 ? "Conflicting" : status, conflicting };
}

/**
 * Per-entity constrained state for the MOCK lane (SCHEMA §7.4 `entityStates`).
 *
 * The map is deliberately SPARSE, because "absent" means UNKNOWN and unknown is
 * the honest answer for almost everything this lane can see. It reports exactly
 * two things and nothing else:
 *
 *   - `conflicting` for every entity a PROVABLY clashing constraint NAMES —
 *     `detectConflicts` output only, projected over `c.entities`. That is the
 *     same deliberate over-attribution §7.4 specifies for the real worker (a
 *     dimension between two entities reds both); `conflicting[]` stays
 *     authoritative for WHICH constraints are at fault. `detectConflicts` is
 *     called HERE rather than passed in so `mockEnforce`'s `refusedIds` — a
 *     limitation of the mock's bounded driver, not a geometric contradiction —
 *     can never reach this projection.
 *   - `fullyConstrained` for `referenceLocked` entities: projected host-face
 *     geometry is pinned by machine `Fixed` constraints and provably cannot
 *     move (the same rule `solveDof` counts it under).
 *
 * Everything else is OMITTED. In particular `fullyConstrained` is NEVER derived
 * from `dof === 0`: this lane's DOF is a coarse Σ-heuristic with no Jacobian
 * behind it, so a zero total says nothing about any individual entity, and a
 * map built from it would be a fabricated diagnosis wearing the real one's
 * clothes. Per-entity truth for user geometry is the PlaneGCS lane's.
 */
export function mockEntityStates(
  entities: SketchEntity[],
  constraints: SketchConstraint[],
): SketchEntityStates {
  const states: SketchEntityStates = {};
  for (const e of entities) {
    if (e.referenceLocked) states[e.id] = "fullyConstrained";
  }
  const clashing = new Set(detectConflicts(entities, constraints));
  if (clashing.size > 0) {
    const live = new Set(entities.map((e) => e.id));
    for (const c of constraints) {
      if (!clashing.has(c.id)) continue;
      // `conflicting` outranks `fullyConstrained` (§7.4), so this overwrites.
      for (const id of c.entities) if (live.has(id)) states[id] = "conflicting";
    }
  }
  return states;
}

// ── Region-detection helpers ──────────────────────────────────────────────────
//
// `detectRegions` itself (the exported entry point) lives in `./mockRegions`
// (bundle split — it's the only entry-reachable importer of three's
// `ShapeUtils`/`Vector2`). Everything below is exported so that module can build
// on it one-way; nothing here is dead code despite looking internal-only.

const QUANT = 1e6; // 1e-6 endpoint-match tolerance
const key = (p: [number, number]): string =>
  `${Math.round(p[0] * QUANT)},${Math.round(p[1] * QUANT)}`;

/** Deterministic mock region id from member ids (NOT the normative scheme). */
export function mockRegionId(memberIds: string[]): string {
  let h = 0x811c9dc5; // FNV-1a-32
  for (const s of [...memberIds].sort()) {
    for (let i = 0; i < s.length; i++) {
      h ^= s.charCodeAt(i);
      h = Math.imul(h, 0x01000193);
    }
  }
  return `r_${(h >>> 0).toString(16).padStart(8, "0")}`;
}

interface Seg {
  id: string;
  a: [number, number];
  b: [number, number];
}

function segEndpoints(e: SketchEntity): Seg | null {
  if (e.type === "Line" && e.p0 && e.p1) return { id: e.id, a: e.p0, b: e.p1 };
  if (e.type === "Arc" && e.start && e.end) return { id: e.id, a: e.start, b: e.end };
  return null;
}

/** Walk segments into a single closed loop; ordered points or null if not one loop. */
export function orderedClosedLoop(entities: SketchEntity[]): { ids: string[]; points: [number, number][] } | null {
  const segs = entities.map(segEndpoints).filter((s): s is Seg => s !== null);
  if (segs.length < 3) return null;

  // Degree check: every endpoint vertex must have exactly two incident segments.
  const degree = new Map<string, number>();
  for (const s of segs) {
    degree.set(key(s.a), (degree.get(key(s.a)) ?? 0) + 1);
    degree.set(key(s.b), (degree.get(key(s.b)) ?? 0) + 1);
  }
  if ([...degree.values()].some((d) => d !== 2)) return null;
  if (degree.size !== segs.length) return null; // #vertices == #edges ⇒ single cycle

  // Walk the cycle.
  const remaining = new Set(segs.map((_, i) => i));
  const ids: string[] = [];
  const points: [number, number][] = [];
  let start = segs[0].a;
  let cursor = start;
  let steps = 0;
  while (remaining.size > 0 && steps <= segs.length) {
    steps++;
    let advanced = false;
    for (const i of remaining) {
      const s = segs[i];
      if (key(s.a) === key(cursor)) {
        ids.push(s.id);
        points.push(cursor);
        cursor = s.b;
        remaining.delete(i);
        advanced = true;
        break;
      }
      if (key(s.b) === key(cursor)) {
        ids.push(s.id);
        points.push(cursor);
        cursor = s.a;
        remaining.delete(i);
        advanced = true;
        break;
      }
    }
    if (!advanced) return null; // broken chain
  }
  return key(cursor) === key(start) ? { ids, points } : null;
}

/** Signed area of a polygon (CCW positive). */
export function signedArea(pts: [number, number][]): number {
  let a = 0;
  for (let i = 0; i < pts.length; i++) {
    const [x0, y0] = pts[i];
    const [x1, y1] = pts[(i + 1) % pts.length];
    a += x0 * y1 - x1 * y0;
  }
  return a / 2;
}

/** Ray-cast point-in-polygon. */
export function pointInPolygon(p: [number, number], poly: [number, number][]): boolean {
  let inside = false;
  for (let i = 0; i < poly.length; i++) {
    const [ax, ay] = poly[i];
    const [bx, by] = poly[(i + 1) % poly.length];
    if (ay > p[1] !== by > p[1]) {
      const t = (p[1] - ay) / (by - ay);
      if (p[0] < ax + t * (bx - ax)) inside = !inside;
    }
  }
  return inside;
}

/** Shortest distance from `p` to the polygon's boundary. */
export function distanceToBoundary(p: [number, number], poly: [number, number][]): number {
  let best = Infinity;
  for (let i = 0; i < poly.length; i++) {
    const [ax, ay] = poly[i];
    const [bx, by] = poly[(i + 1) % poly.length];
    const dx = bx - ax;
    const dy = by - ay;
    const len2 = dx * dx + dy * dy;
    const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((p[0] - ax) * dx + (p[1] - ay) * dy) / len2));
    best = Math.min(best, Math.hypot(p[0] - (ax + t * dx), p[1] - (ay + t * dy)));
  }
  return best;
}

/** Sample a circle into a CCW polygon at the shared visual quality. */
function circlePolygon(center: [number, number], radius: number): [number, number][] {
  const segments = visualSegmentsForClosedCurve(radius);
  const pts: [number, number][] = [];
  for (let i = 0; i < segments; i++) {
    const a = (i / segments) * Math.PI * 2;
    pts.push([center[0] + radius * Math.cos(a), center[1] + radius * Math.sin(a)]);
  }
  return pts;
}

/** Fan-triangulate a closed ring about an interior `center` into plane-local
 *  (u,v) preview tris. Valid for any ring star-shaped about that centre — true
 *  for both a sampled circle and a sampled ellipse. */
export function fanTriangles(center: [number, number], ring: [number, number][]): {
  positions: number[];
  indices: number[];
} {
  const positions = [center[0], center[1]];
  for (const p of ring) positions.push(p[0], p[1]);
  const indices: number[] = [];
  for (let i = 0; i < ring.length; i++) indices.push(0, 1 + i, 1 + ((i + 1) % ring.length));
  return { positions, indices };
}

/**
 * A SELF-CLOSED curve entity (Circle or Ellipse) — one that bounds a cell all by
 * itself, unlike a line/arc which only closes a loop with its neighbours.
 * `reach` is the largest distance from `center` to the curve (radius / semi-major):
 * the conservative clearance used by the nesting test.
 */
export interface ClosedCurve {
  entity: SketchEntity;
  center: [number, number];
  polygon: [number, number][];
  reach: number;
}

export function closedCurveOf(e: SketchEntity): ClosedCurve | null {
  if (e.type === "Circle") {
    if (!e.center || !e.radius) return null;
    return { entity: e, center: e.center, polygon: circlePolygon(e.center, e.radius), reach: e.radius };
  }
  if (e.type === "Ellipse") {
    const p = ellipseParams(e);
    if (!p || p.majorR <= 0 || p.minorR <= 0) return null;
    return {
      entity: e,
      center: p.center,
      polygon: sampleEllipse(p, visualSegmentsForClosedCurve(p.majorR)),
      reach: p.majorR,
    };
  }
  return null;
}

// ── util ─────────────────────────────────────────────────────────────────────

function structuredCloneLite<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}
