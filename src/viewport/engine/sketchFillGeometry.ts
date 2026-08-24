/*
 * Sketch fill triangulation — the two sketch layers' shared geometry builder.
 *
 * Two entry points, because the two layers hold different evidence:
 *
 * - `buildFillGeometry` — `SketchStaticLayer` (model mode) has SOLVED regions
 *   from the region authority (the worker on the real lane), so it renders a
 *   `SketchRegion.previewTriangles` payload directly. The hole-completeness
 *   guard lives here rather than in the layer: a fill built from a payload that
 *   did not subtract every declared hole paints solid over a void, which reads
 *   as "this region is extrudable" when it is not.
 * - `buildNestedFillGeometries` — `SketchObject` (the LIVE edit session, audit
 *   item #2) has no regions at all mid-draw; it has closed LOOPS of entities it
 *   is already drawing (`findClosedLoops` plus the self-closed curves). It
 *   classifies their nesting EVEN-ODD and ear-clips each painted ring with its
 *   holes subtracted, so a donut's hole stays a hole.
 */
import * as THREE from "three";
import type { SketchRegion } from "@/ipc/types";

/** Shoelace signed area of a plane (u,v) ring; >0 is CCW. */
function signedArea(ring: readonly [number, number][]): number {
  let sum = 0;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    sum += (ring[j][0] - ring[i][0]) * (ring[j][1] + ring[i][1]);
  }
  return sum / 2;
}

/** A ring as CCW `Vector2`s — `triangulateShape` wants the contour that way. */
function ccwContour(ring: readonly [number, number][]): THREE.Vector2[] {
  const ordered = signedArea(ring) < 0 ? [...ring].reverse() : ring;
  return ordered.map(([x, y]) => new THREE.Vector2(x, y));
}

/**
 * One closed ring of plane (u,v) points, minus `holes`, as an ear-clipped local
 * XY geometry — or null when it is degenerate.
 */
function buildRingFillGeometry(
  ring: readonly [number, number][],
  holes: readonly (readonly [number, number][])[],
): THREE.BufferGeometry | null {
  if (ring.length < 3) return null;
  // `triangulateShape` TRIMS a duplicated closing point in place, in the contour
  // AND in every hole — so the vertex buffer below has to be flattened from
  // these arrays AFTER the call, not from the inputs, or the indices would
  // address the untrimmed arrays. Hole winding is left alone: earcut normalizes
  // each ring's direction itself.
  const contour = ccwContour(ring);
  const holeContours = holes.map(ccwContour);
  const faces = THREE.ShapeUtils.triangulateShape(contour, holeContours);
  if (faces.length === 0) return null;
  // Index space is contour first, then each hole in the order it was passed.
  const verts = [contour, ...holeContours].flat();
  // z is left at the Float32Array's zero — the ring is plane-LOCAL, and the
  // caller's group carries the plane basis.
  const positions = new Float32Array(verts.length * 3);
  for (let i = 0; i < verts.length; i++) {
    positions[i * 3] = verts[i].x;
    positions[i * 3 + 1] = verts[i].y;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geo.setIndex(faces.flat());
  geo.computeBoundingSphere();
  return geo;
}

/** Tolerance for "this sample landed ON that ring's boundary". */
const BOUNDARY_EPS = 1e-9;

function onRingBoundary(p: readonly [number, number], ring: readonly [number, number][]): boolean {
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [ax, ay] = ring[j];
    const [bx, by] = ring[i];
    const dx = bx - ax;
    const dy = by - ay;
    const len2 = dx * dx + dy * dy;
    const t = len2 > 0 ? Math.max(0, Math.min(1, ((p[0] - ax) * dx + (p[1] - ay) * dy) / len2)) : 0;
    const cx = p[0] - (ax + t * dx);
    const cy = p[1] - (ay + t * dy);
    if (cx * cx + cy * cy <= BOUNDARY_EPS * BOUNDARY_EPS) return true;
  }
  return false;
}

/** Crossing-number containment. Callers screen out on-boundary points first —
 *  a crossing test says nothing useful about one. */
function pointInRing(p: readonly [number, number], ring: readonly [number, number][]): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, yi] = ring[i];
    const [xj, yj] = ring[j];
    // The half-open `>` pair is what keeps a ray through a shared vertex from
    // being counted twice.
    if ((yi > p[1]) !== (yj > p[1]) && p[0] < ((xj - xi) * (p[1] - yi)) / (yj - yi) + xi) {
      inside = !inside;
    }
  }
  return inside;
}

/** How many of a ring's vertices vote on where it sits relative to another. */
const CONTAINMENT_SAMPLES = 9;

/**
 * Is ring `a` enclosed by ring `b`?
 *
 * The evidence has to come from `a`'s BOUNDARY, not its interior: a point deep
 * inside a donut's outer ring can sit inside the hole ring as well, which would
 * make the two rings look mutually nested and classify the whole donut away.
 *
 * Vertices lying ON `b` abstain — that is what keeps two loops sharing an edge
 * or a corner (a filleted rectangle chain, a pocket flush with a wall) from
 * flipping each other's classification. The rest vote, rather than the first one
 * deciding, because tessellated curves that touch (a hole tangent to a round
 * wall) put a vertex or two on the wrong side of the chord that approximates
 * the other curve.
 */
function ringInside(a: readonly [number, number][], b: readonly [number, number][]): boolean {
  const step = Math.max(1, Math.floor(a.length / CONTAINMENT_SAMPLES));
  let inVotes = 0;
  let outVotes = 0;
  for (let i = 0; i < a.length; i += step) {
    if (onRingBoundary(a[i], b)) continue;
    if (pointInRing(a[i], b)) inVotes++;
    else outVotes++;
  }
  return inVotes > outVotes;
}

/**
 * Every live closure fill for one set of closed rings, classified EVEN-ODD.
 *
 * A ring enclosed by an even number of others (zero included) is MATERIAL and
 * gets a mesh; an odd one is a HOLE and is subtracted from the innermost ring
 * that encloses it. So a circle in a rectangle paints an annulus, and an island
 * drawn inside that hole paints again — the same rule the kernel applies when
 * it builds the real region at finish time.
 *
 * Rings arrive already sampled at the stroke's own tessellation, so a fill
 * boundary can never disagree with the curve drawn on top of it.
 */
export function buildNestedFillGeometries(
  rings: readonly (readonly [number, number][])[],
): THREE.BufferGeometry[] {
  // A ring with no area bounds nothing: it can neither be filled nor be a hole.
  const loops = rings.filter((r) => r.length >= 3 && Math.abs(signedArea(r)) > 0);

  // containers[i] = the loops enclosing loop i. Depth parity decides material
  // vs hole; the deepest container is the ring a hole belongs to.
  const containers = loops.map((ring, i) => loops.map((_, j) => j).filter((j) => j !== i && ringInside(ring, loops[j])));
  const depth = containers.map((c) => c.length);

  const holesOf = new Map<number, (readonly [number, number][])[]>();
  for (let i = 0; i < loops.length; i++) {
    if (depth[i] % 2 === 0) continue;
    // Innermost enclosing loop. Overlapping (not nested) rings can tie on depth;
    // the smaller one wins so the choice is deterministic either way.
    let parent = -1;
    for (const j of containers[i]) {
      const deeper = parent < 0 || depth[j] > depth[parent];
      const sameDepthButSmaller =
        parent >= 0 && depth[j] === depth[parent] && Math.abs(signedArea(loops[j])) < Math.abs(signedArea(loops[parent]));
      if (deeper || sameDepthButSmaller) parent = j;
    }
    if (parent >= 0) (holesOf.get(parent) ?? holesOf.set(parent, []).get(parent)!).push(loops[i]);
  }

  const geos: THREE.BufferGeometry[] = [];
  for (let i = 0; i < loops.length; i++) {
    if (depth[i] % 2 !== 0) continue;
    const geo = buildRingFillGeometry(loops[i], holesOf.get(i) ?? []);
    if (geo) geos.push(geo);
  }
  return geos;
}

/** One region's plane-local (u,v) triangles as an indexed local XY geometry,
 *  or null when the payload is missing, degenerate, or hole-incomplete. */
export function buildFillGeometry(region: SketchRegion): THREE.BufferGeometry | null {
  const triangles = region.previewTriangles;
  if (!triangles || triangles.positions.length < 6 || triangles.indices.length < 3) {
    return null;
  }
  if ((triangles.holesSubtracted ?? 0) < region.holes.length) return null;
  const positions = new Float32Array((triangles.positions.length / 2) * 3);
  for (let i = 0; i < triangles.positions.length / 2; i++) {
    positions[i * 3] = triangles.positions[i * 2];
    positions[i * 3 + 1] = triangles.positions[i * 2 + 1];
    positions[i * 3 + 2] = 0;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geo.setIndex(triangles.indices.slice());
  geo.computeBoundingSphere();
  return geo;
}
