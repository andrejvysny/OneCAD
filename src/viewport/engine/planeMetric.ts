/*
 * planeMetric — the PURE plane→screen projection math behind
 * `ViewportEngine.planeScreenMetric` (SNAP §5.3).
 *
 * Extracted from the engine so it is testable with no renderer: a THREE camera
 * and a `SketchPlane` are enough, which is what lets the oblique/perspective
 * accuracy table run in vitest instead of only in a browser.
 *
 * WHY A MATRIX AND NOT A SCALAR. `planePixelWorld()` answers "how many world
 * units is one pixel?" with ONE number, which is only correct when the plane
 * faces the camera squarely. Tilt the view and a plane unit along u projects to
 * a different pixel length than the same unit along v — so a scalar threshold
 * makes the snap radius an ellipse the user cannot see, wider in one direction
 * than the other. The 2×2 Jacobian measures the real thing.
 */
import * as THREE from "three";
import type { SketchPlane } from "@/ipc/types";
import { metricUsable, type PlaneScreenMetric, type ScreenPoint } from "@/tools/sketch/snapTypes";
import { planePointToWorld, type Point2 } from "./sketchBasis";

export interface ViewportSize {
  width: number;
  height: number;
}

/** Reusable scratch so the pointer-rate path allocates nothing. */
export interface PlaneMetricScratch {
  viewProj: THREE.Matrix4;
  world: THREE.Vector3;
  clip: THREE.Vector4;
  screen: [ScreenPoint, ScreenPoint, ScreenPoint];
  out: PlaneScreenMetric;
}

export function newPlaneMetricScratch(): PlaneMetricScratch {
  return {
    viewProj: new THREE.Matrix4(),
    world: new THREE.Vector3(),
    clip: new THREE.Vector4(),
    screen: [
      { x: 0, y: 0 },
      { x: 0, y: 0 },
      { x: 0, y: 0 },
    ],
    out: { m00: 0, m01: 0, m10: 0, m11: 0 },
  };
}

/**
 * EXACT projection of a plane point to canvas-relative CSS pixels, or null when
 * it is behind the camera / non-finite.
 *
 * `camera.updateMatrixWorld()` and `updateProjectionMatrix()` are the caller's
 * responsibility — the engine's render loop already keeps both current, and
 * re-running them here would cost a matrix rebuild per snap candidate.
 */
export function projectPlanePoint(
  plane: SketchPlane,
  camera: THREE.Camera,
  viewport: ViewportSize,
  p: Point2,
  scratch: PlaneMetricScratch = newPlaneMetricScratch(),
): ScreenPoint | null {
  scratch.viewProj.multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
  return projectWithViewProj(plane, scratch, viewport, p, { x: 0, y: 0 });
}

function projectWithViewProj(
  plane: SketchPlane,
  scratch: PlaneMetricScratch,
  viewport: ViewportSize,
  p: Point2,
  out: ScreenPoint,
): ScreenPoint | null {
  planePointToWorld(plane, p, scratch.world);
  scratch.clip
    .set(scratch.world.x, scratch.world.y, scratch.world.z, 1)
    .applyMatrix4(scratch.viewProj);
  if (!(scratch.clip.w > 0)) return null; // behind the camera / degenerate w
  const ndcX = scratch.clip.x / scratch.clip.w;
  const ndcY = scratch.clip.y / scratch.clip.w;
  if (!Number.isFinite(ndcX) || !Number.isFinite(ndcY)) return null;
  out.x = (ndcX * 0.5 + 0.5) * viewport.width;
  out.y = (-ndcY * 0.5 + 0.5) * viewport.height;
  return out;
}

/**
 * The local plane→screen Jacobian at `at`, in CSS pixels per plane unit.
 *
 * Columns are the projected unit-u and unit-v screen deltas. Null when any of
 * the three sample points is behind the camera, projects non-finitely, or the
 * result is degenerate (edge-on view).
 *
 * The returned object is `scratch.out` — copy it to keep it across frames.
 */
export function computePlaneScreenMetric(
  plane: SketchPlane,
  camera: THREE.Camera,
  viewport: ViewportSize,
  at: Point2,
  scratch: PlaneMetricScratch = newPlaneMetricScratch(),
): PlaneScreenMetric | null {
  if (viewport.width === 0 || viewport.height === 0) return null;
  scratch.viewProj.multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
  const samples: readonly Point2[] = [at, { x: at.x + 1, y: at.y }, { x: at.x, y: at.y + 1 }];
  for (let i = 0; i < 3; i++) {
    if (!projectWithViewProj(plane, scratch, viewport, samples[i], scratch.screen[i])) return null;
  }
  const [o, u, v] = scratch.screen;
  scratch.out.m00 = u.x - o.x;
  scratch.out.m10 = u.y - o.y;
  scratch.out.m01 = v.x - o.x;
  scratch.out.m11 = v.y - o.y;
  return metricUsable(scratch.out) ? scratch.out : null;
}
