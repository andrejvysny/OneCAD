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
  const { edgeRanges, edgePositions } = view;
  if (!edgeRanges || !edgePositions) return [];
  if (faceOrdinal < 0 || faceOrdinal >= view.faceCount) return [];

  const triangles = faceTriangles(view, faceOrdinal);
  if (triangles.length === 0) return [];
  const tolerance = 2 * chordalDeflection(view.lod, bboxDiagonal(view));

  // One bbox over the face's own triangles (`faceBboxes` is optional in MESH1),
  // expanded by the tolerance: the cheap reject for the great majority of edges.
  const box = new THREE.Box3();
  for (const tri of triangles) box.expandByPoint(tri.a).expandByPoint(tri.b).expandByPoint(tri.c);
  box.expandByScalar(tolerance);

  const point = new THREE.Vector3();
  const closest = new THREE.Vector3();
  const out: number[] = [];
  for (let e = 0; e < view.edgeCount; e++) {
    const firstPoint = edgeRanges[e * 2];
    const pointCount = edgeRanges[e * 2 + 1];
    if (pointCount === 0) continue; // degenerate edge: no polyline to test
    let onFace = true;
    for (let p = 0; p < pointCount && onFace; p++) {
      const i = (firstPoint + p) * 3;
      point.set(edgePositions[i], edgePositions[i + 1], edgePositions[i + 2]);
      if (!box.containsPoint(point)) {
        onFace = false;
        break;
      }
      let best = Infinity;
      for (const tri of triangles) {
        tri.closestPointToPoint(point, closest);
        best = Math.min(best, closest.distanceTo(point));
        if (best <= tolerance) break;
      }
      onFace = best <= tolerance;
    }
    if (onFace) out.push(e);
  }
  return out;
}
