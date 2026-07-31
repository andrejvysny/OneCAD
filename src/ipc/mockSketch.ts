/*
 * Mock sketch "solver" + region detection (MOCK-ONLY, no real geometry kernel).
 *
 * The real solver is the C++ worker's PlaneGCS actor (SCHEMA §7.4). Until it is
 * wired in, these pure functions give the frontend a plausible, deterministic
 * stand-in so the whole sketch UX (DOF badge, constraint state colors, extrude
 * profile preview) runs with no backend. LIMITS are documented per function.
 */
import type {
  SketchConstraint,
  SketchEntity,
  SketchPlane,
  SketchPlaneKind,
  SketchRegion,
  SketchSolveStatus,
} from "./types";
import { ellipseParams, sampleEllipse } from "@/tools/sketch/ellipseMath";

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

export function freeDegrees(entities: SketchEntity[]): number {
  return entities.reduce((sum, e) => sum + entityFreedom(e), 0);
}

export function removedDegrees(constraints: SketchConstraint[]): number {
  return constraints.reduce((sum, c) => sum + constraintFreedom(c), 0);
}

/** Solve → {dof (clamped ≥0), status}. Signed surplus decides the state. */
export function solveDof(
  entities: SketchEntity[],
  constraints: SketchConstraint[],
): { dof: number; status: SketchSolveStatus } {
  const surplus = freeDegrees(entities) - removedDegrees(constraints);
  const status: SketchSolveStatus =
    surplus > 0 ? "UnderConstrained" : surplus === 0 ? "FullyConstrained" : "OverConstrained";
  return { dof: Math.max(0, surplus), status };
}

// ── Region detection (MOCK — single closed loop, or circles) ─────────────────
//
// LIMITS: detects (a) each SELF-CLOSED curve (Circle or Ellipse) as its own
// region and (b) a SINGLE closed loop of lines/arcs (every vertex degree 2,
// connected). No self-intersection handling. The real worker computes proper
// regions via LoopDetector (SCHEMA §7.4 SketchRegions). regionId here is a mock
// hash, NOT the normative FNV-1a-64 over 16-byte UUIDs the worker/Rust agree on.
//
// NESTING (mock subset): a self-closed curve whose interior lies wholly INSIDE
// the closed loop creates TWO selectable planar cells: the disc, plus the
// enclosing loop with it as a hole. This matches CAD profile semantics: clicking
// the disc extrudes a cylinder; clicking the surrounding material extrudes a
// tube. One that is separated from, or crosses, the loop stays its own region.
//
// An Ellipse is treated exactly like a Circle here (W3 P3): it is closed, its
// sampled polygon has area πab, and it is star-shaped about its centre — the two
// properties the fill + nesting code actually depend on.

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

/** Fan-triangulate a polygon (from centroid) into plane-local (u,v) preview tris. */
function polygonTriangles(points: [number, number][]): { positions: number[]; indices: number[] } {
  const cx = points.reduce((s, p) => s + p[0], 0) / points.length;
  const cy = points.reduce((s, p) => s + p[1], 0) / points.length;
  const positions = [cx, cy];
  for (const p of points) positions.push(p[0], p[1]);
  const indices: number[] = [];
  for (let i = 0; i < points.length; i++) {
    indices.push(0, 1 + i, 1 + ((i + 1) % points.length));
  }
  return { positions, indices };
}

/** Signed area of a polygon (CCW positive). */
function signedArea(pts: [number, number][]): number {
  let a = 0;
  for (let i = 0; i < pts.length; i++) {
    const [x0, y0] = pts[i];
    const [x1, y1] = pts[(i + 1) % pts.length];
    a += x0 * y1 - x1 * y0;
  }
  return a / 2;
}

/** Ray-cast point-in-polygon. */
function pointInPolygon(p: [number, number], poly: [number, number][]): boolean {
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
function distanceToBoundary(p: [number, number], poly: [number, number][]): number {
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

/** Sample a circle into a CCW polygon. */
function circlePolygon(center: [number, number], radius: number, segments = 32): [number, number][] {
  const pts: [number, number][] = [];
  for (let i = 0; i < segments; i++) {
    const a = (i / segments) * Math.PI * 2;
    pts.push([center[0] + radius * Math.cos(a), center[1] + radius * Math.sin(a)]);
  }
  return pts;
}

/**
 * Triangulate the annulus between an outer ring and ONE hole ring by stitching
 * them into a strip ordered by ANGLE about the hole's centre: both rings are
 * traversed CCW and whichever has the angularly-nearer next vertex advances.
 * Every connecting edge is shared by two triangles, so the only single-use edges
 * are the real outer and hole boundaries — the topology
 * `prismPreview.profileFromRegion` needs to recover both rings. Each emitted
 * triangle is normalized to CCW, which orients the recovered outer loop CCW and
 * the hole loop CW.
 *
 * Angle order (not normalized ring parameter) is what makes the strip
 * non-overlapping: a 4-vertex rectangle and a 32-vertex circle have wildly
 * different parameter spacing, and a parameter-based stitch produces crossing
 * triangles whose areas then double-count.
 *
 * MOCK LIMIT: exactly one hole, and both rings must be star-shaped about the
 * hole's centre (true for the rectangle + inner circle the mock lane draws). The
 * worker uses proper bridge-based ear clipping (`worker/src/loop/PolygonFill.cpp`).
 */
function annulusTriangles(
  outerIn: [number, number][],
  holeIn: [number, number][],
): { positions: number[]; indices: number[] } {
  const outer = signedArea(outerIn) < 0 ? [...outerIn].reverse() : outerIn;
  const hole = signedArea(holeIn) < 0 ? [...holeIn].reverse() : holeIn;
  const m = hole.length;

  const cx = hole.reduce((s, p) => s + p[0], 0) / m;
  const cy = hole.reduce((s, p) => s + p[1], 0) / m;
  const TWO_PI = Math.PI * 2;

  // Every direction in which EITHER ring has a vertex. Sampling both rings at
  // this shared, sorted direction set gives two rings with equal vertex counts
  // whose k-th vertices lie on the same ray — so the quad strip between them is
  // non-overlapping by construction, and both rings keep their original corners.
  const dirs: number[] = [];
  for (const p of [...outer, ...hole]) {
    const a = Math.atan2(p[1] - cy, p[0] - cx);
    dirs.push(((a % TWO_PI) + TWO_PI) % TWO_PI);
  }
  dirs.sort((a, b) => a - b);
  const uniq = dirs.filter((a, k) => k === 0 || a - dirs[k - 1] > 1e-12);

  // Farthest ray/boundary hit from the centre — exact at a vertex the ray passes
  // through, so corners survive sampling.
  const rayHit = (theta: number, poly: [number, number][]): [number, number] => {
    const dx = Math.cos(theta);
    const dy = Math.sin(theta);
    let best = 0;
    for (let k = 0; k < poly.length; k++) {
      const [ax, ay] = poly[k];
      const [bx, by] = poly[(k + 1) % poly.length];
      const ex = bx - ax;
      const ey = by - ay;
      const den = dx * ey - dy * ex;
      if (Math.abs(den) < 1e-15) continue;
      const s = ((ax - cx) * ey - (ay - cy) * ex) / den; // along the ray
      const t = ((ax - cx) * dy - (ay - cy) * dx) / den; // along the edge
      if (s > 0 && t >= -1e-12 && t <= 1 + 1e-12) best = Math.max(best, s);
    }
    return [cx + best * dx, cy + best * dy];
  };

  const positions: number[] = [];
  for (const t of uniq) {
    const [x, y] = rayHit(t, outer);
    positions.push(x, y);
  }
  for (const t of uniq) {
    const [x, y] = rayHit(t, hole);
    positions.push(x, y);
  }

  const k = uniq.length;
  const oi = (i: number): number => i % k;
  const hi = (j: number): number => k + (j % k);
  const indices: number[] = [];
  const push = (a: number, b: number, c: number): void => {
    const ax = positions[a * 2];
    const ay = positions[a * 2 + 1];
    const cross =
      (positions[b * 2] - ax) * (positions[c * 2 + 1] - ay) -
      (positions[b * 2 + 1] - ay) * (positions[c * 2] - ax);
    if (Math.abs(cross) < 1e-12) return; // degenerate
    if (cross > 0) indices.push(a, b, c);
    else indices.push(a, c, b);
  };
  for (let i = 0; i < k; i++) {
    push(oi(i), oi(i + 1), hi(i));
    push(oi(i + 1), hi(i + 1), hi(i));
  }
  return { positions, indices };
}

/** Fan-triangulate a closed ring about an interior `center` into plane-local
 *  (u,v) preview tris. Valid for any ring star-shaped about that centre — true
 *  for both a sampled circle and a sampled ellipse. */
function fanTriangles(center: [number, number], ring: [number, number][]): {
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
interface ClosedCurve {
  entity: SketchEntity;
  center: [number, number];
  polygon: [number, number][];
  reach: number;
}

function closedCurveOf(e: SketchEntity): ClosedCurve | null {
  if (e.type === "Circle") {
    if (!e.center || !e.radius) return null;
    return { entity: e, center: e.center, polygon: circlePolygon(e.center, e.radius), reach: e.radius };
  }
  if (e.type === "Ellipse") {
    const p = ellipseParams(e);
    if (!p || p.majorR <= 0 || p.minorR <= 0) return null;
    // 32 samples, matching `circlePolygon` — the fill/containment resolution the
    // rest of this mock is tuned for (the RENDERER samples at 72; this is fill only).
    return { entity: e, center: p.center, polygon: sampleEllipse(p, 32), reach: p.majorR };
  }
  return null;
}

export function detectRegions(entities: SketchEntity[]): SketchRegion[] {
  const regions: SketchRegion[] = [];
  const live = entities.filter((e) => !e.construction);
  const loop = orderedClosedLoop(live);

  // A self-closed curve (circle OR ellipse) wholly inside the closed loop is both a
  // selectable disc cell and a hole boundary of the enclosing cell. "Wholly inside"
  // = centre inside AND the whole curve clear of the boundary, so a crossing one
  // stays independent. `reach` (radius / semi-major) makes that clearance test
  // conservative for a rotated ellipse without sampling the boundary distance.
  const holes: ClosedCurve[] = [];
  const free: ClosedCurve[] = [];
  for (const e of live) {
    const curve = closedCurveOf(e);
    if (!curve) continue;
    const nested =
      loop !== null &&
      pointInPolygon(curve.center, loop.points) &&
      distanceToBoundary(curve.center, loop.points) > curve.reach;
    (nested ? holes : free).push(curve);
  }

  for (const c of [...free, ...holes]) {
    regions.push({
      regionId: mockRegionId([c.entity.id]),
      outerLoop: [c.entity.id],
      holes: [],
      previewTriangles: { ...fanTriangles(c.center, c.polygon), holesSubtracted: 0 },
    });
  }

  if (loop && holes.length <= 1) {
    // MOCK LIMIT: annulusTriangles supports one hole. With multiple holes, omit
    // the enclosing cell instead of publishing fill geometry for different
    // material than the declared profile.
    const first = holes[0];
    regions.push({
      // A cell's identity includes its material boundary, not only its outer
      // wire. Adding/removing a hole must invalidate an armed profile.
      regionId: mockRegionId([...loop.ids, ...holes.map((hole) => hole.entity.id)]),
      outerLoop: loop.ids,
      holes: holes.map((h) => [h.entity.id]),
      previewTriangles: {
        ...(first ? annulusTriangles(loop.points, first.polygon) : polygonTriangles(loop.points)),
        holesSubtracted: first ? 1 : 0,
      },
    });
  }

  return regions;
}

// ── util ─────────────────────────────────────────────────────────────────────

function structuredCloneLite<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}
