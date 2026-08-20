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
 * - `buildRingFillGeometry` — `SketchObject` (the LIVE edit session, audit item
 *   #2) has no regions at all mid-draw; it has closed LOOPS of entities it is
 *   already drawing (`findClosedLoops`). It ear-clips one sampled ring straight
 *   into a fill.
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

/**
 * One closed ring of plane (u,v) points as an ear-clipped local XY geometry, or
 * null when it is degenerate.
 *
 * NO HOLES, deliberately (v1): each loop the live session finds fills on its
 * own, so a circle inside a rectangle paints two overlapping fills and the
 * "hole" reads as filled. The signal this draws is "these strokes close", not
 * "this is the exact material the extrude will take" — the region authority
 * decides that at finish time, and over-painting a hole never makes an
 * unextrudable profile look extrudable.
 */
export function buildRingFillGeometry(ring: readonly [number, number][]): THREE.BufferGeometry | null {
  if (ring.length < 3) return null;
  // `triangulateShape` wants CCW, and TRIMS a duplicated closing point in place —
  // so the vertex buffer below has to be flattened from `contour` AFTER the call,
  // not from `ring`, or the indices would address the untrimmed array.
  const contour = (signedArea(ring) < 0 ? [...ring].reverse() : ring).map(([x, y]) => new THREE.Vector2(x, y));
  const faces = THREE.ShapeUtils.triangulateShape(contour, []);
  if (faces.length === 0) return null;
  // z is left at the Float32Array's zero — the ring is plane-LOCAL, and the
  // caller's group carries the plane basis.
  const positions = new Float32Array(contour.length * 3);
  for (let i = 0; i < contour.length; i++) {
    positions[i * 3] = contour[i].x;
    positions[i * 3 + 1] = contour[i].y;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geo.setIndex(faces.flat());
  geo.computeBoundingSphere();
  return geo;
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
