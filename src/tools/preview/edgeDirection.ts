/*
 * Edge outward-direction reconstruction (PURE math, no three.js import).
 *
 * FE has no edge→face topology on the wire — MESH1 ships FACE_RANGES and
 * EDGE_RANGES as separate, unlinked sections (protocol/mesh_format.md). To
 * tell "away from the body" (fillet) from "into the body" (chamfer) at a
 * picked edge, we reconstruct the two tiers documented in
 * FILLET-CHAMFER-UNIFY:
 *
 *   T1 bisector — sum of the flat normals of the faces whose PLANE contains
 *   the edge's midpoint (a membership test standing in for real adjacency).
 *   Sign-correct on convex AND concave edges — the only tier honest enough
 *   to ever drive an automatic type switch.
 *   T2 bbox proxy — direction from the mesh's bounding-box centre to the
 *   edge midpoint. Convex-only (wrong sign on a pocket edge); the fallback
 *   whenever T1 can't find ≥2 supporting faces (a curved adjacent face fails
 *   the plane test, NORMALS is absent, or the view has no edges at all).
 *
 * Every function here is a pure reducer over a BodyMeshView — no THREE
 * import, so this unit-tests with zero WebGL/jsdom cost.
 */
import type { BodyMeshView } from "@/viewport/mesh/parseMeshPayload";
import type { Vec3 } from "./depthProjection";

// Re-exported so callers of this module's Vec3-typed returns (edgeMidAndTangent,
// edgeOutward, averageOutward) don't need a second import from depthProjection.
export type { Vec3 };

/** Which tier produced an outward direction. "none" is reserved for W1
 *  callers that need to represent "no direction computed yet" distinctly
 *  from a failed lookup — this module never constructs it itself. */
export type OutwardSource = "bisector" | "bbox" | "none";

const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const add = (a: Vec3, b: Vec3): Vec3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const len = (a: Vec3): number => Math.hypot(a[0], a[1], a[2]);

function unit(a: Vec3): Vec3 | null {
  const l = len(a);
  if (l < 1e-9) return null;
  return [a[0] / l, a[1] / l, a[2] / l];
}

function bboxDiagonal(view: BodyMeshView): number {
  return Math.hypot(
    view.bboxMax[0] - view.bboxMin[0],
    view.bboxMax[1] - view.bboxMin[1],
    view.bboxMax[2] - view.bboxMin[2],
  );
}

/**
 * Edge polyline midpoint + local tangent. Midpoint = the midpoint of the
 * polyline's MIDDLE segment (simplest choice that's exact for the 2-point
 * straight edges every mock/worker box or prism ships, and still lands
 * inside the correct adjacent face's plane-membership test near a curved
 * rim). Tangent = that segment's normalized direction.
 */
export function edgeMidAndTangent(
  view: BodyMeshView,
  edgeOrdinal: number,
): { mid: Vec3; tangent: Vec3 } | null {
  const edgeRanges = view.edgeRanges;
  const edgePositions = view.edgePositions;
  if (!view.hasEdges || !edgeRanges || !edgePositions) return null;
  if (edgeOrdinal < 0 || edgeOrdinal >= view.edgeCount) return null;

  const first = edgeRanges[edgeOrdinal * 2];
  const count = edgeRanges[edgeOrdinal * 2 + 1];
  if (count < 2) return null;
  const at = (i: number): Vec3 => {
    const o = (first + i) * 3;
    return [edgePositions[o], edgePositions[o + 1], edgePositions[o + 2]];
  };

  const seg = Math.floor((count - 1) / 2);
  const p0 = at(seg);
  const p1 = at(seg + 1);
  const tangent = unit(sub(p1, p0));
  if (!tangent) return null;
  const mid: Vec3 = [(p0[0] + p1[0]) / 2, (p0[1] + p1[1]) / 2, (p0[2] + p1[2]) / 2];
  return { mid, tangent };
}

/** Per-face flat frame for the bisector membership test. */
export interface FaceFrame {
  /** Face normal, read from the mesh's own NORMALS at the first vertex of
   *  the face's first triangle. Planar (worker) faces store one flat normal
   *  per vertex, so any vertex is correct; a curved face stores smoothly
   *  varying normals, so this is only LOCALLY correct near that one vertex —
   *  which is exactly why bisectorAt's plane test rejects it everywhere else
   *  on the face (see module doc, T1 tier). */
  normal: Vec3;
  /** The vertex `normal` was read at — the plane-membership reference point. */
  v0: Vec3;
  min: Vec3;
  max: Vec3;
}

/**
 * Per-face flat frames for the bisector test. Requires the mesh's optional
 * NORMALS section: without it there is no honest per-face direction to
 * reconstruct (T1 needs the worker's own shading normals, not a
 * geometry-only guess), so an absent NORMALS section yields zero frames and
 * callers correctly degrade to the T2 bbox tier.
 */
export function faceFrames(view: BodyMeshView): FaceFrame[] {
  const normals = view.normals;
  if (!normals) return [];
  const { positions, indices, faceRanges, faceCount } = view;
  const frames: FaceFrame[] = [];
  for (let f = 0; f < faceCount; f++) {
    const firstTri = faceRanges[f * 2];
    const triCount = faceRanges[f * 2 + 1];
    if (triCount === 0) continue;
    const i0 = indices[firstTri * 3];
    const normal: Vec3 = [normals[i0 * 3], normals[i0 * 3 + 1], normals[i0 * 3 + 2]];
    const v0: Vec3 = [positions[i0 * 3], positions[i0 * 3 + 1], positions[i0 * 3 + 2]];
    const min: Vec3 = [Infinity, Infinity, Infinity];
    const max: Vec3 = [-Infinity, -Infinity, -Infinity];
    for (let t = firstTri; t < firstTri + triCount; t++) {
      for (let k = 0; k < 3; k++) {
        const vi = indices[t * 3 + k];
        for (let a = 0; a < 3; a++) {
          const v = positions[vi * 3 + a];
          if (v < min[a]) min[a] = v;
          if (v > max[a]) max[a] = v;
        }
      }
    }
    frames.push({ normal, v0, min, max });
  }
  return frames;
}

function insideGrown(min: Vec3, max: Vec3, p: Vec3, tol: number): boolean {
  for (let a = 0; a < 3; a++) {
    if (p[a] < min[a] - tol || p[a] > max[a] + tol) return false;
  }
  return true;
}

/**
 * Sum the flat normals of every face whose plane contains `p` (within `tol`)
 * AND whose vertex bbox (grown by `tol`) contains `p`. Requires ≥2 kept
 * faces — a single face can't bisect an edge, and that floor is also the
 * plane test's real teeth: one lucky facet on a curved face must not pass
 * alone.
 */
export function bisectorAt(frames: readonly FaceFrame[], p: Vec3, tol: number): Vec3 | null {
  let sum: Vec3 = [0, 0, 0];
  let kept = 0;
  for (const f of frames) {
    if (Math.abs(dot(f.normal, sub(p, f.v0))) > tol) continue;
    if (!insideGrown(f.min, f.max, p, tol)) continue;
    sum = add(sum, f.normal);
    kept++;
  }
  if (kept < 2) return null;
  return unit(sum);
}

/** Direction from the mesh's bbox centre to `p` — the T2 convex-only proxy.
 *  Null when `p` sits at the centre (degenerate, no direction to report). */
export function bboxOutward(view: BodyMeshView, p: Vec3): Vec3 | null {
  const center: Vec3 = [
    (view.bboxMin[0] + view.bboxMax[0]) / 2,
    (view.bboxMin[1] + view.bboxMax[1]) / 2,
    (view.bboxMin[2] + view.bboxMax[2]) / 2,
  ];
  return unit(sub(p, center));
}

/**
 * Edge outward direction with T1→T2 tier fallback. `tol` defaults to
 * 1e-4 × the mesh's bbox diagonal — scale-relative, so the same call works
 * on a 1mm bracket and a 1m frame.
 */
export function edgeOutward(
  view: BodyMeshView,
  edgeOrdinal: number,
  tol?: number,
): { mid: Vec3; tangent: Vec3; outward: Vec3; source: OutwardSource } | null {
  const mt = edgeMidAndTangent(view, edgeOrdinal);
  if (!mt) return null;
  const { mid, tangent } = mt;
  const effTol = tol ?? bboxDiagonal(view) * 1e-4;

  const bisector = bisectorAt(faceFrames(view), mid, effTol);
  if (bisector) return { mid, tangent, outward: bisector, source: "bisector" };

  const bbox = bboxOutward(view, mid);
  if (bbox) return { mid, tangent, outward: bbox, source: "bbox" };

  return null;
}

/**
 * Unit mean of several outward directions — null when they substantially
 * disagree (|Σ| < 0.2·N), e.g. two edges on opposite sides of a body. A weak
 * mean is worse than no mean: callers must not average their way into a
 * fabricated direction.
 */
export function averageOutward(dirs: readonly Vec3[]): Vec3 | null {
  if (dirs.length === 0) return null;
  let sum: Vec3 = [0, 0, 0];
  for (const d of dirs) sum = add(sum, d);
  if (len(sum) < 0.2 * dirs.length) return null;
  return unit(sum);
}
