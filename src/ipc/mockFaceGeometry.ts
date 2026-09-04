/*
 * Analytic per-face geometry for the MOCK bodies (SKETCH-ON-FACE W2).
 *
 * The real lane answers "where is this face, and what is its boundary?" with two
 * worker round-trips (`ProjectFaceBoundary`, SCHEMA §7.6). The mock lane has no
 * kernel, so this module answers the same question analytically for the two
 * synthetic shapes the mock renders — the 80×60×30 box and the r25×h60 cylinder.
 *
 * WHY IT LIVES HERE (and not inline in `mockClient`): three consumers need the
 * same answer and none may import each other — `mockClient.faceSketchPlane`
 * (which body/face did the user pick?), `localSolver.enterSketch` (seed the new
 * session with the projected boundary), and the vitest suites that pin both. The
 * face table is DERIVED from `mockMeshes`' corner/face tables, so the analytic
 * geometry and the rendered triangles cannot drift.
 *
 * MOCK LIMITS (deliberate):
 *   - Only the seed box + cylinder are described. A synthesized (extruded) body
 *     has no entry: `lookupMockFace` reports `unknown`, and the caller keeps the
 *     old "+Z plane at the body" behaviour rather than refusing.
 *   - The cylinder's SIDE face is reported `nonPlanar` so the refusal path the
 *     real backend takes is reachable from the mock lane (and from e2e).
 *   - Numeric truth still lives in cargo/ctest. What this file buys is STRUCTURAL
 *     parity: a mock sketch-on-face session has the same shape (locked entities +
 *     Fixed constraints + a detectable region) as the real one.
 */
import type { SketchConstraint, SketchEntity, SketchPlane } from "./types";
import { frontendConstraintsFromDto, frontendEntitiesFromDto } from "./sketchWireMap";
import { BOX_EDGE_PAIRS, BOX_FACES, BOX_SIZE, boxCorners, type BoxCornerKey } from "./mockMeshes";

/** Small deterministic hash for mock ElementIds (FNV-1a-32 hex).
 *
 *  Lives here because face lookup has to REVERSE it: a pick arrives as a
 *  `topoKey`, but by the time a sketch is created the frontend has promoted it to
 *  an `el_<hash>` ElementId, and matching that back to `f:4` means re-deriving the
 *  hash for each candidate face. `mockClient` imports it from here so both sides
 *  mint and match with one implementation. */
export function mockElementHash(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(16).padStart(8, "0");
}

type Vec3 = [number, number, number];
type Vec2 = [number, number];

// ── The in-plane axis rule (ported VERBATIM from Rust) ───────────────────────

/**
 * Build a sketch basis from a point + normal — the TS mirror of
 * `onecad-core::sketch::plane_from_point_normal` (`sketch/plane.rs`).
 *
 * The rule (lock-tested on the Rust side, so it is ported rather than invented):
 * `x = normalize(z_seed × n)` with `z_seed` = world +Z, falling back to
 * `n × +X` when `n` is (anti)parallel to +Z and the cross product vanishes;
 * then `y = n × x`. The fallback's operand ORDER is load-bearing: it is what
 * makes a +Z normal (a flat top face) reproduce the named XY basis exactly
 * — `z × n` would give the XY basis mirrored.
 *
 * A zero / non-finite normal falls back to the XY basis rather than producing
 * NaNs, again mirroring the Rust.
 */
export function planeFromPointNormal(origin: Vec3, normal: Vec3): SketchPlane {
  const norm = (v: Vec3): Vec3 | null => {
    const len = Math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    if (!Number.isFinite(len) || len < 1e-12) return null;
    return [v[0] / len, v[1] / len, v[2] / len];
  };
  const cross = (a: Vec3, b: Vec3): Vec3 => [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];

  const n = norm(normal);
  if (!n) {
    // SCHEMA §7.3 canonical XY basis (the non-standard one), verbatim.
    return { kind: "custom", origin, xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] };
  }
  const x = norm(cross([0, 0, 1], n)) ?? norm(cross(n, [1, 0, 0])) ?? ([1, 0, 0] as Vec3);
  const y = cross(n, x);
  return { kind: "custom", origin, xAxis: x, yAxis: y, normal: n };
}

/** World point → the plane's (u,v), i.e. `((p − origin)·x, (p − origin)·y)`. */
export function worldToPlaneUv(plane: SketchPlane, p: Vec3): Vec2 {
  const d: Vec3 = [p[0] - plane.origin[0], p[1] - plane.origin[1], p[2] - plane.origin[2]];
  const dot = (a: Vec3, b: readonly number[]): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  return [dot(d, plane.xAxis), dot(d, plane.yAxis)];
}

// ── Face geometry ────────────────────────────────────────────────────────────

/** A projected face boundary in the sketch plane's (u,v). */
export type MockFaceBoundary =
  /** A closed corner ring (the box faces). */
  | { kind: "polygon"; corners: Vec2[] }
  /** A full circular edge (the cylinder caps). */
  | { kind: "circle"; center: Vec2; radius: number };

export interface MockFaceGeometry {
  /** The frozen sketch frame, from the SAME axis rule the backend applies. */
  plane: SketchPlane;
  /** The face outline, expressed in that frame. */
  boundary: MockFaceBoundary;
}

/** What `lookupMockFace` found. `unknown` means "this mock body has no analytic
 *  description" — NOT a refusal (see the module's MOCK LIMITS). */
export type MockFaceLookup =
  | { kind: "planar"; faceId: string; geometry: MockFaceGeometry }
  | { kind: "nonPlanar"; faceId: string }
  | { kind: "unknown" };

/** Cylinder dimensions — must mirror `makeCylinderMesh`'s defaults. */
const CYLINDER = { radius: 25, height: 60 } as const;

/** The mock box's six faces, derived from the shared corner + face tables. */
function boxFaceTable(): Map<string, MockFaceGeometry> {
  const corners = boxCorners(BOX_SIZE);
  const out = new Map<string, MockFaceGeometry>();
  for (const face of BOX_FACES) {
    const ring: Vec3[] = face.corners.map((k: BoxCornerKey) => corners[k]);
    // A planar face's frame origin only has to lie ON the plane (the kernel's own
    // `gp_Pln` origin is likewise arbitrary within the plane); the corner centroid
    // is the stable, obvious choice for a rectangle.
    const origin: Vec3 = [
      ring.reduce((s, p) => s + p[0], 0) / ring.length,
      ring.reduce((s, p) => s + p[1], 0) / ring.length,
      ring.reduce((s, p) => s + p[2], 0) / ring.length,
    ];
    const plane = planeFromPointNormal(origin, face.normal);
    out.set(face.id, {
      plane,
      boundary: { kind: "polygon", corners: ring.map((p) => worldToPlaneUv(plane, p)) },
    });
  }
  return out;
}

/** The mock cylinder's two planar CAPS (`f:1` top, `f:2` bottom). The side face
 *  `f:0` is absent on purpose — it is reported `nonPlanar`. */
function cylinderFaceTable(): Map<string, MockFaceGeometry> {
  const out = new Map<string, MockFaceGeometry>();
  for (const [id, z, normal] of [
    ["f:1", CYLINDER.height / 2, [0, 0, 1]],
    ["f:2", -CYLINDER.height / 2, [0, 0, -1]],
  ] as [string, number, Vec3][]) {
    const plane = planeFromPointNormal([0, 0, z], normal);
    out.set(id, {
      plane,
      boundary: { kind: "circle", center: [0, 0], radius: CYLINDER.radius },
    });
  }
  return out;
}

const BOX_FACE_GEOMETRY = boxFaceTable();
const CYLINDER_FACE_GEOMETRY = cylinderFaceTable();

/** Which synthetic shape a mock bodyId renders as — mirrors `meshForBody`. */
function shapeForBody(bodyId: string): "box" | "cylinder" | null {
  if (bodyId === "body2") return "cylinder";
  if (bodyId === "body1") return "box";
  // Synthesized (extrude/revolve/boolean output) bodies have no analytic entry.
  return null;
}

/** Every face id the given shape can answer for, planar or not. */
function faceIdsFor(shape: "box" | "cylinder"): string[] {
  return shape === "box" ? BOX_FACES.map((f) => f.id) : ["f:0", "f:1", "f:2"];
}

/**
 * Resolve a pick (`elementId` and/or `topoKey`) to one mock face.
 *
 * Two rungs, mirroring the backend's: the `topoKey` when present, else the
 * promoted `el_<hash>` ElementId reversed against this body's face ids (the mock
 * mints element ids as `el_${mockElementHash(bodyId#topoKey)}`, so the mapping is
 * invertible by re-deriving the hash for each candidate).
 */
export function lookupMockFace(
  bodyId: string,
  elementId?: string,
  topoKey?: string,
): MockFaceLookup {
  const shape = shapeForBody(bodyId);
  if (!shape) return { kind: "unknown" };
  let faceId = topoKey && topoKey.startsWith("f:") ? topoKey : undefined;
  if (!faceId && elementId) {
    faceId = faceIdsFor(shape).find((id) => `el_${mockElementHash(`${bodyId}#${id}`)}` === elementId);
  }
  if (!faceId) return { kind: "unknown" };
  const table = shape === "box" ? BOX_FACE_GEOMETRY : CYLINDER_FACE_GEOMETRY;
  const geometry = table.get(faceId);
  if (!geometry) {
    // A known face id with no planar entry is the cylinder's curved side.
    return faceIdsFor(shape).includes(faceId) ? { kind: "nonPlanar", faceId } : { kind: "unknown" };
  }
  return { kind: "planar", faceId, geometry };
}

/**
 * The faces adjacent to one mock EDGE, as TopoKeys sorted by face ordinal ASCENDING
 * — the mock's answer to SCHEMA §7.6 `adjacentFaces` (kernel-hardening WP-F).
 *
 * Derived, not invented: an edge is adjacent to exactly the faces whose corner ring
 * contains BOTH of its endpoints, and both tables (`BOX_EDGE_PAIRS`, `BOX_FACES`)
 * are the same ones `makeBoxMesh` renders from, so the answer describes the box the
 * viewport is actually drawing. Every box edge is manifold, so this returns two
 * entries; the ordering is `BOX_FACES` order, which IS the `f:<k>` ordinal order.
 *
 * `[]` for anything that is not a seed-box edge — the cylinder's caps meet its
 * curved side across a seam this module deliberately does not describe, and a
 * synthesized body has no analytic entry at all. A caller must treat an empty list
 * as "not known", never as "no adjacency": fabricating a face here is exactly the
 * silent wrong bind WP-F exists to remove.
 */
export function mockAdjacentFaces(bodyId: string, edgeTopoKey: string): string[] {
  if (shapeForBody(bodyId) !== "box") return [];
  const m = /^e:(\d+)$/.exec(edgeTopoKey);
  const pair = m ? BOX_EDGE_PAIRS[Number(m[1])] : undefined;
  if (!pair) return [];
  return BOX_FACES.filter(
    (face) => face.corners.includes(pair[0]) && face.corners.includes(pair[1]),
  ).map((face) => face.id);
}

// ── The projected boundary, in the shape the REAL DTO carries ────────────────

/**
 * Build the locked reference geometry a sketch on this face is seeded with.
 *
 * The two invariants are the backend's (`onecad-core::sketch::projection`): every
 * entity is `referenceLocked`, and every projected POINT carries a `Fixed`
 * constraint at its own (u,v). Pinning is what makes the boundary immovable in
 * the solver as well as in the edit layer.
 *
 * It is assembled in the WORKER WIRE form and then reverse-mapped through the
 * very functions `tauriClient.enterSketch` uses (`frontendEntitiesFromDto` /
 * `frontendConstraintsFromDto`). That is not a detour — it is what guarantees the
 * mock session is shaped EXACTLY like a hydrated real one (owned child points
 * dropped, Fixed constraints re-keyed onto `(owner, position)`), with no second
 * copy of the hydration rules to keep in step.
 *
 * `idPrefix` keys the deterministic ids; it must not collide with the frontend's
 * own `e<N>` / `c<N>` minting (hence the underscore forms).
 */
export function mockProjectedContent(
  geometry: MockFaceGeometry,
  idPrefix = "ref",
): { entities: SketchEntity[]; constraints: SketchConstraint[] } {
  const wireEntities: Record<string, unknown>[] = [];
  const wireConstraints: Record<string, unknown>[] = [];
  const point = (at: Vec2, i: number): string => {
    const id = `${idPrefix}_p${i}`;
    wireEntities.push({ id, type: "Point", at, referenceLocked: true });
    wireConstraints.push({ id: `${idPrefix}_c${i}`, type: "Fixed", entities: [id] });
    return id;
  };

  if (geometry.boundary.kind === "polygon") {
    const ring = geometry.boundary.corners;
    const ids = ring.map((at, i) => point(at, i));
    for (let i = 0; i < ids.length; i++) {
      wireEntities.push({
        id: `${idPrefix}_e${i}`,
        type: "Line",
        p0Ref: ids[i],
        p1Ref: ids[(i + 1) % ids.length],
        referenceLocked: true,
      });
    }
  } else {
    const centerRef = point(geometry.boundary.center, 0);
    wireEntities.push({
      id: `${idPrefix}_e0`,
      type: "Circle",
      center: geometry.boundary.center,
      centerRef,
      radius: geometry.boundary.radius,
      referenceLocked: true,
    });
  }

  return {
    entities: frontendEntitiesFromDto(wireEntities),
    constraints: frontendConstraintsFromDto(wireConstraints, wireEntities),
  };
}
