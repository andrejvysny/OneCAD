import * as THREE from "three";
import type { MeshEntry } from "../mesh/meshRegistry";

export interface CapturedFaceBounds {
  readonly bodyId: string;
  readonly bounds: THREE.Box3;
}

const tokens = new WeakMap<CapturedFaceBounds, {
  entry: MeshEntry;
  body: THREE.Object3D;
  matrixWorld: THREE.Matrix4;
}>();

export function captureFaceBounds(
  entry: MeshEntry,
  body: THREE.Object3D,
  identity: { elementId?: string; topoKey?: string },
): CapturedFaceBounds | null {
  const ordinal = resolveOrdinal(entry, identity);
  if (ordinal < 0) return null;
  const firstTriangle = entry.view.faceRanges[ordinal * 2];
  const triangleCount = entry.view.faceRanges[ordinal * 2 + 1];
  if (triangleCount === 0) return null;
  body.updateMatrixWorld(true);
  const bounds = new THREE.Box3();
  const point = new THREE.Vector3();
  for (let i = firstTriangle * 3; i < (firstTriangle + triangleCount) * 3; i++) {
    const vertex = entry.view.indices[i] * 3;
    point
      .set(entry.view.positions[vertex], entry.view.positions[vertex + 1], entry.view.positions[vertex + 2])
      .applyMatrix4(body.matrixWorld);
    bounds.expandByPoint(point);
  }
  if (bounds.isEmpty() || !finiteBox(bounds)) return null;
  const capture = { bodyId: entry.bodyId, bounds: bounds.clone() };
  tokens.set(capture, { entry, body, matrixWorld: body.matrixWorld.clone() });
  return capture;
}

export function isFaceBoundsCurrent(
  capture: CapturedFaceBounds,
  entry: MeshEntry | undefined,
  body: THREE.Object3D | undefined,
): boolean {
  const token = tokens.get(capture);
  body?.updateMatrixWorld(true);
  return !!token
    && token.entry === entry
    && token.body === body
    && body.visible
    && token.matrixWorld.equals(body.matrixWorld);
}

function resolveOrdinal(entry: MeshEntry, identity: { elementId?: string; topoKey?: string }): number {
  if (identity.elementId) {
    const ordinal = entry.faceIndex.ordinalForId(identity.elementId);
    if (ordinal >= 0) return ordinal;
  }
  return identity.topoKey ? entry.faceIndex.ordinalForId(identity.topoKey) : -1;
}

function finiteBox(box: THREE.Box3): boolean {
  return box.min.toArray().every(Number.isFinite) && box.max.toArray().every(Number.isFinite);
}
