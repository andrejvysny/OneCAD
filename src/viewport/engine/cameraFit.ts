import * as THREE from "three";
import { CAMERA_FAR, CAMERA_NEAR, type CameraBasis, type ProjectionKind } from "./CameraRig";

export const CAMERA_MIN_DISTANCE = 0.5;
export const CAMERA_MAX_DISTANCE = 50_000;
export const CAMERA_FIT_MARGIN = 1.15;

export interface FitViewport {
  readonly width: number;
  readonly height: number;
  /** Local CSS pixels. Undefined means work-area measurement is not initialized. */
  readonly safeRect?: { readonly x: number; readonly y: number; readonly width: number; readonly height: number };
}

export interface CameraFitRequest {
  readonly bounds: THREE.Box3;
  readonly basis: CameraBasis;
  readonly projection: ProjectionKind;
  readonly verticalFovDeg: number;
  readonly aspect: number;
  readonly viewport: FitViewport;
  readonly yaw: number;
  readonly pitch: number;
}

export interface CameraFitTarget {
  readonly target: THREE.Vector3;
  readonly distance: number;
  readonly yaw: number;
  readonly pitch: number;
}

interface NdcRect { left: number; right: number; bottom: number; top: number }

export function computeCameraFit(request: CameraFitRequest): CameraFitTarget | null {
  if (request.bounds.isEmpty() || !finiteBox(request.bounds)) return null;
  const ndc = safeNdcRect(request.viewport);
  if (!ndc || !Number.isFinite(request.aspect) || request.aspect <= 0) return null;
  const tanY = Math.tan((request.verticalFovDeg * Math.PI) / 360);
  const tanX = tanY * request.aspect;
  if (!(tanX > 0) || !(tanY > 0)) return null;

  const center = request.bounds.getCenter(new THREE.Vector3());
  const cx = (ndc.left + ndc.right) / 2;
  const cy = (ndc.bottom + ndc.top) / 2;
  let required = CAMERA_MIN_DISTANCE;
  let maxZ = -Infinity;
  let minZ = Infinity;
  for (const corner of boxCorners(request.bounds)) {
    const relative = corner.sub(center);
    const x = relative.dot(request.basis.right);
    const y = relative.dot(request.basis.up);
    const z = relative.dot(request.basis.back);
    maxZ = Math.max(maxZ, z);
    minZ = Math.min(minZ, z);
    if (request.projection === "persp") {
      required = Math.max(
        required,
        (x + ndc.right * tanX * z) / ((ndc.right - cx) * tanX),
        (-x - ndc.left * tanX * z) / ((cx - ndc.left) * tanX),
        (y + ndc.top * tanY * z) / ((ndc.top - cy) * tanY),
        (-y - ndc.bottom * tanY * z) / ((cy - ndc.bottom) * tanY),
      );
    } else {
      required = Math.max(
        required,
        x / ((ndc.right - cx) * tanX),
        -x / ((cx - ndc.left) * tanX),
        y / ((ndc.top - cy) * tanY),
        -y / ((cy - ndc.bottom) * tanY),
      );
    }
  }
  let distance = Math.max(required, maxZ + CAMERA_NEAR) * CAMERA_FIT_MARGIN;
  if (!Number.isFinite(distance) || distance > CAMERA_MAX_DISTANCE || distance - minZ > CAMERA_FAR) return null;
  distance = Math.max(CAMERA_MIN_DISTANCE, distance);
  const target = center
    .clone()
    .addScaledVector(request.basis.right, -cx * tanX * distance)
    .addScaledVector(request.basis.up, -cy * tanY * distance);
  return { target, distance, yaw: request.yaw, pitch: request.pitch };
}

function safeNdcRect(viewport: FitViewport): NdcRect | null {
  const { width, height } = viewport;
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) return null;
  const r = viewport.safeRect;
  if (r === undefined) return { left: -1, right: 1, bottom: -1, top: 1 };
  if (![r.x, r.y, r.width, r.height].every(Number.isFinite) || r.width <= 0 || r.height <= 0) return null;
  const leftPx = Math.max(0, r.x);
  const rightPx = Math.min(width, r.x + r.width);
  const topPx = Math.max(0, r.y);
  const bottomPx = Math.min(height, r.y + r.height);
  if (rightPx <= leftPx || bottomPx <= topPx) return null;
  return {
    left: (2 * leftPx) / width - 1,
    right: (2 * rightPx) / width - 1,
    bottom: 1 - (2 * bottomPx) / height,
    top: 1 - (2 * topPx) / height,
  };
}

function finiteBox(box: THREE.Box3): boolean {
  return box.min.toArray().every(Number.isFinite) && box.max.toArray().every(Number.isFinite);
}

function boxCorners(box: THREE.Box3): THREE.Vector3[] {
  const out: THREE.Vector3[] = [];
  for (const x of [box.min.x, box.max.x]) {
    for (const y of [box.min.y, box.max.y]) {
      for (const z of [box.min.z, box.max.z]) out.push(new THREE.Vector3(x, y, z));
    }
  }
  return out;
}
