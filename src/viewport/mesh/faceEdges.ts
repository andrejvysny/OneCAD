/*
 * faceEdges — which of a body's edges bound one of its faces, derived from the
 * MESH1 payload the viewport already holds.
 *
 * WHY THIS IS GEOMETRY AND NOT A QUERY. The worker knows the answer exactly
 * (`worker/src/session/EdgePicks.cpp` walks `TopExp::MapShapesAndAncestors`),
 * but nothing on `CadClient` asks for it: `prepareEdgeOp` reports the inverse
 * (each PREPARED edge's `adjacentFaces`) and every other topology read —
 * `elementInfo`, `classifyElement`, `massProperties` — answers about a single
 * element. Rather than open a new wire verb for a selection convenience (C7 /
 * D9), the expansion is derived locally from the two things MESH1 already
 * carries for every body: the face's triangles and each edge's polyline.
 *
 * THE PREDICATE. An edge bounds the face when EVERY point of its polyline lies
 * on the face's triangulated surface. Point-to-TRIANGLE, deliberately not
 * point-to-plane: a coplanar neighbouring face would pass a plane test while
 * lying entirely outside this face's triangles.
 *
 * THE TOLERANCE is the tessellation's own chordal deflection, doubled. Edge
 * polylines are sampled off the true curve while faces are chordal triangles
 * (`worker/src/tess/Tessellate.cpp`: `sample_edge` and `BRepMesh` share the
 * policy but not the nodes), so a point on a curved boundary edge sits up to one
 * deflection off the face's triangles even when it genuinely bounds it. The
 * policy is mirrored from `deflections()` there; it is the one number in this
 * file that would have to move if the worker's changed.
 *
 * THE LIMIT, stated rather than hidden: this is a tessellation-accurate answer,
 * not a BRep one. Two distinct faces running within a deflection of each other
 * over an edge's whole length cannot be told apart here. The expansion feeds a
 * PREVIEWED fillet the user confirms, never a silent rebind.
 */
import * as THREE from "three";
import type { BodyMeshView } from "./parseMeshPayload";

/**
 * The worker's display tessellation deflection, in model units — mirrored from
 * `deflections()` in `worker/src/tess/Tessellate.cpp`, keyed by the LOD code
 * MESH1 carries (0 coarse, 1 medium, 2 fine).
 */
export function chordalDeflection(lod: number, bboxDiagonal: number): number {
  const [rel, min, max] =
    lod === 2 ? [0.0005, 0.0001, 0.05] : lod === 1 ? [0.0025, 0.002, 0.15] : [0.01, 0.01, 0.5];
  return Math.min(Math.max(bboxDiagonal * rel, min), max);
}

function bboxDiagonal(view: BodyMeshView): number {
  const [x0, y0, z0] = view.bboxMin;
  const [x1, y1, z1] = view.bboxMax;
  return Math.hypot(x1 - x0, y1 - y0, z1 - z0);
}

/** Scratch for the point-to-triangle walk — `orient()`'s precedent in `DragHandle`. */
const _point = new THREE.Vector3();
const _closest = new THREE.Vector3();

/** The face's triangles as reusable THREE.Triangles, in body-local coordinates. */
function faceTriangles(view: BodyMeshView, faceOrdinal: number): THREE.Triangle[] {
  const first = view.faceRanges[faceOrdinal * 2];
  const count = view.faceRanges[faceOrdinal * 2 + 1];
  const out: THREE.Triangle[] = [];
  for (let t = 0; t < count; t++) {
    const tri = new THREE.Triangle();
    const base = (first + t) * 3;
    const corners = [tri.a, tri.b, tri.c];
    for (let c = 0; c < 3; c++) {
      const v = view.indices[base + c] * 3;
      corners[c].set(view.positions[v], view.positions[v + 1], view.positions[v + 2]);
    }
    out.push(tri);
  }
  return out;
}

/**
 * One face's triangulated surface, reduced to what the predicate needs: the
 * triangles, their tolerance-grown bbox (the cheap reject) and the tolerance.
 *
 * Built ONCE per face so a search that tests many edges against many faces pays
 * for the triangles once each, not once per pair.
 */
interface FaceSurface {
  readonly triangles: THREE.Triangle[];
  readonly box: THREE.Box3;
  readonly tolerance: number;
}

function faceSurface(view: BodyMeshView, faceOrdinal: number): FaceSurface | null {
  if (faceOrdinal < 0 || faceOrdinal >= view.faceCount) return null;
  const triangles = faceTriangles(view, faceOrdinal);
  if (triangles.length === 0) return null;
  const tolerance = 2 * chordalDeflection(view.lod, bboxDiagonal(view));
  // One bbox over the face's own triangles (`faceBboxes` is optional in MESH1),
  // expanded by the tolerance: the cheap reject for the great majority of edges.
  const box = new THREE.Box3();
  for (const tri of triangles) box.expandByPoint(tri.a).expandByPoint(tri.b).expandByPoint(tri.c);
  box.expandByScalar(tolerance);
  return { triangles, box, tolerance };
}

/** Whether EVERY point of edge `e`'s polyline lies on `surface` (see module doc). */
function edgeOnSurface(view: BodyMeshView, surface: FaceSurface, edgeOrdinal: number): boolean {
  const { edgeRanges, edgePositions } = view;
  if (!edgeRanges || !edgePositions) return false;
  const firstPoint = edgeRanges[edgeOrdinal * 2];
  const pointCount = edgeRanges[edgeOrdinal * 2 + 1];
  if (pointCount === 0) return false; // degenerate edge: no polyline to test
  for (let p = 0; p < pointCount; p++) {
    const i = (firstPoint + p) * 3;
    _point.set(edgePositions[i], edgePositions[i + 1], edgePositions[i + 2]);
    if (!surface.box.containsPoint(_point)) return false;
    let best = Infinity;
    for (const tri of surface.triangles) {
      tri.closestPointToPoint(_point, _closest);
      best = Math.min(best, _closest.distanceTo(_point));
      if (best <= surface.tolerance) break;
    }
    if (!(best <= surface.tolerance)) return false;
  }
  return true;
}

/**
 * Ordinals of the edges bounding `faceOrdinal`, ascending.
 *
 * Empty — never a guess — when the mesh carries no edges, when the face has no
 * triangles (a face MESH1 lists but could not tessellate), or when no edge
 * passes the predicate.
 */
export function faceBoundaryEdgeOrdinals(
  view: BodyMeshView,
  faceOrdinal: number,
): number[] {
  if (!view.edgeRanges || !view.edgePositions) return [];
  const surface = faceSurface(view, faceOrdinal);
  if (!surface) return [];
  const out: number[] = [];
  for (let e = 0; e < view.edgeCount; e++) {
    if (edgeOnSurface(view, surface, e)) out.push(e);
  }
  return out;
}

/** One retained face and the removed-face rim edge it is the sole neighbour of. */
export interface RimNeighbour {
  readonly faceOrdinal: number;
  readonly edgeOrdinal: number;
}

/**
 * The walls left standing around a set of removed faces
 * (docs/design/astra/modeling-handle-attachment.md §5 "Shell"): the boundary
 * edges of the removed faces, each paired with the ONE retained face that also
 * bounds it. Ascending by face ordinal, then edge ordinal — the snapshot order
 * a caller's tie-break reads.
 *
 * AMBIGUITY IS DISCARDED, NOT RESOLVED. The predicate above is accurate to the
 * tessellation, not to the BRep: a 40 mm cube at fine LOD carries a 0.069282 mm
 * adjacency tolerance, so an unrelated face 0.05 mm away passes the same
 * distance test as the real wall (§7). An edge two retained faces match is
 * therefore dropped; so is one no retained face matches. What survives is a
 * DISPLAY location with a single candidate, never a topological claim.
 */
export function retainedRimNeighbours(
  view: BodyMeshView,
  removedFaceOrdinals: readonly number[],
): RimNeighbour[] {
  if (!view.edgeRanges || !view.edgePositions) return [];
  const removed = new Set(
    removedFaceOrdinals.filter((f) => Number.isInteger(f) && f >= 0 && f < view.faceCount),
  );
  if (removed.size === 0) return [];

  const rim = new Set<number>();
  for (const f of removed) for (const e of faceBoundaryEdgeOrdinals(view, f)) rim.add(e);
  if (rim.size === 0) return [];

  // One match per rim edge, or the sentinel -2 once a second retained face
  // claims it: the whole point is that a second match kills the edge.
  const AMBIGUOUS = -2;
  const match = new Map<number, number>();
  for (let f = 0; f < view.faceCount; f++) {
    if (removed.has(f)) continue;
    const surface = faceSurface(view, f);
    if (!surface) continue;
    for (const e of rim) {
      if (match.get(e) === AMBIGUOUS) continue;
      if (!edgeOnSurface(view, surface, e)) continue;
      match.set(e, match.has(e) ? AMBIGUOUS : f);
    }
  }

  const out: RimNeighbour[] = [];
  for (const [edgeOrdinal, faceOrdinal] of match) {
    if (faceOrdinal !== AMBIGUOUS) out.push({ faceOrdinal, edgeOrdinal });
  }
  out.sort((a, b) => a.faceOrdinal - b.faceOrdinal || a.edgeOrdinal - b.edgeOrdinal);
  return out;
}
