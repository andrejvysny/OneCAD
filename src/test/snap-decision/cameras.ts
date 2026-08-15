/*
 * Camera / plane fixtures for the snap decision table (SNAP P0).
 *
 * Shared by every phase's tests so "normal", "oblique" and "near-edge" mean the
 * same thing in the metric accuracy table, the grid-ladder table and the P2
 * arbitration scenarios. Building real THREE cameras (rather than hand-writing
 * matrices) is deliberate: the thing under test is the projection the app
 * actually renders through, and a hand-written matrix could agree with a wrong
 * implementation.
 */
import * as THREE from "three";
import type { SketchPlane } from "@/ipc/types";

/** World XY (Z-up world, so this plane faces +Z). */
export const XY_PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

/** World XZ — a plane whose v axis runs along world +Z. */
export const XZ_PLANE: SketchPlane = {
  kind: "XZ",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 0, 1],
  normal: [0, -1, 0],
};

export interface CameraCase {
  name: string;
  projection: "perspective" | "orthographic";
  plane: SketchPlane;
  camera: THREE.Camera;
  viewport: { width: number; height: number };
  /** A plane point comfortably inside the frame for this case. */
  at: { x: number; y: number };
}

function perspective(
  width: number,
  height: number,
  eye: [number, number, number],
  target: [number, number, number],
): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(50, width / height, 0.1, 10_000);
  cam.up.set(0, 0, 1); // Z-up world (viewport README HARD INVARIANT)
  cam.position.set(...eye);
  cam.lookAt(new THREE.Vector3(...target));
  cam.updateMatrixWorld(true);
  cam.updateProjectionMatrix();
  return cam;
}

function orthographic(
  width: number,
  height: number,
  halfHeight: number,
  eye: [number, number, number],
  target: [number, number, number],
): THREE.OrthographicCamera {
  const aspect = width / height;
  const cam = new THREE.OrthographicCamera(
    -halfHeight * aspect,
    halfHeight * aspect,
    halfHeight,
    -halfHeight,
    0.1,
    10_000,
  );
  cam.up.set(0, 0, 1);
  cam.position.set(...eye);
  cam.lookAt(new THREE.Vector3(...target));
  cam.updateMatrixWorld(true);
  cam.updateProjectionMatrix();
  return cam;
}

/** The camera table every metric-sensitive test runs over. */
export function cameraCases(width = 1280, height = 768): CameraCase[] {
  return [
    {
      name: "perspective straight-on",
      projection: "perspective",
      plane: XY_PLANE,
      camera: perspective(width, height, [0, 0, 260], [0, 0, 0]),
      viewport: { width, height },
      at: { x: 0, y: 0 },
    },
    {
      name: "perspective oblique",
      projection: "perspective",
      plane: XY_PLANE,
      camera: perspective(width, height, [180, -160, 120], [0, 0, 0]),
      viewport: { width, height },
      at: { x: 5, y: -3 },
    },
    {
      name: "perspective steep oblique",
      projection: "perspective",
      plane: XY_PLANE,
      camera: perspective(width, height, [40, -260, 18], [0, 0, 0]),
      viewport: { width, height },
      at: { x: 2, y: 1 },
    },
    {
      name: "orthographic straight-on",
      projection: "orthographic",
      plane: XY_PLANE,
      camera: orthographic(width, height, 120, [0, 0, 260], [0, 0, 0]),
      viewport: { width, height },
      at: { x: 0, y: 0 },
    },
    {
      name: "orthographic oblique",
      projection: "orthographic",
      plane: XY_PLANE,
      camera: orthographic(width, height, 120, [160, -140, 110], [0, 0, 0]),
      viewport: { width, height },
      at: { x: -8, y: 6 },
    },
    {
      name: "orthographic on XZ plane",
      projection: "orthographic",
      plane: XZ_PLANE,
      camera: orthographic(width, height, 90, [30, -220, 40], [0, 0, 0]),
      viewport: { width, height },
      at: { x: 4, y: -2 },
    },
    {
      // Near the frame edge, where a perspective projection is least linear —
      // the worst case for a LOCAL Jacobian standing in for exact projection.
      name: "perspective near viewport edge",
      projection: "perspective",
      plane: XY_PLANE,
      camera: perspective(width, height, [0, -40, 120], [0, 0, 0]),
      viewport: { width, height },
      at: { x: 74, y: 46 },
    },
  ];
}

/** Viewport heights the grid ladder table is swept at. */
export const VIEWPORT_HEIGHTS = [400, 600, 768, 1080] as const;
