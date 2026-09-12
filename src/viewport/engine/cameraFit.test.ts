import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { CameraRig, cameraBasisForOffset, type ProjectionKind } from "./CameraRig";
import { sphericalToOffset } from "./CadOrbitControls";
import { CAMERA_MAX_DISTANCE, computeCameraFit, type FitViewport } from "./cameraFit";

const bounds = new THREE.Box3(new THREE.Vector3(-7, -3, -2), new THREE.Vector3(11, 5, 9));

function corners(box: THREE.Box3): THREE.Vector3[] {
  return [box.min.x, box.max.x].flatMap((x) =>
    [box.min.y, box.max.y].flatMap((y) =>
      [box.min.z, box.max.z].map((z) => new THREE.Vector3(x, y, z)),
    ),
  );
}

function frame(projection: ProjectionKind, viewport: FitViewport, yaw = 0.7, pitch = 0.42) {
  const rig = new CameraRig(76);
  rig.setAspect(viewport.width / viewport.height);
  rig.setProjection(projection);
  const offset = sphericalToOffset(yaw, pitch, 1);
  const result = computeCameraFit({
    bounds,
    basis: rig.basisForOffset(offset),
    projection,
    verticalFovDeg: rig.fovDeg,
    aspect: rig.aspectRatio,
    viewport,
    yaw,
    pitch,
  });
  if (!result) throw new Error("expected fit");
  rig.apply(result.target, offset.multiplyScalar(result.distance), result.distance);
  return { rig, result };
}

function expectInside(rig: CameraRig, box: THREE.Box3, viewport: FitViewport): void {
  const safe = viewport.safeRect ?? { x: 0, y: 0, width: viewport.width, height: viewport.height };
  const left = (2 * safe.x) / viewport.width - 1;
  const right = (2 * (safe.x + safe.width)) / viewport.width - 1;
  const top = 1 - (2 * safe.y) / viewport.height;
  const bottom = 1 - (2 * (safe.y + safe.height)) / viewport.height;
  for (const corner of corners(box)) {
    const projected = corner.clone().project(rig.getCamera());
    expect(projected.x).toBeGreaterThanOrEqual(left - Number.EPSILON * 16);
    expect(projected.x).toBeLessThanOrEqual(right + Number.EPSILON * 16);
    expect(projected.y).toBeGreaterThanOrEqual(bottom - Number.EPSILON * 16);
    expect(projected.y).toBeLessThanOrEqual(top + Number.EPSILON * 16);
    expect(projected.z).toBeGreaterThanOrEqual(-1);
    expect(projected.z).toBeLessThanOrEqual(1);
  }
}

describe("computeCameraFit", () => {
  it.each(["persp", "ortho"] as const)("contains every corner in an offset safe rect for %s", (projection) => {
    const viewport = { width: 1200, height: 700, safeRect: { x: 220, y: 90, width: 610, height: 500 } };
    const { rig } = frame(projection, viewport);
    expectInside(rig, bounds, viewport);
  });

  it.each([
    ["persp", { width: 1600, height: 500, safeRect: { x: 300, y: 20, width: 900, height: 430 } }],
    ["persp", { width: 500, height: 1200, safeRect: { x: 20, y: 180, width: 450, height: 700 } }],
    ["ortho", { width: 1600, height: 500, safeRect: { x: 300, y: 20, width: 900, height: 430 } }],
    ["ortho", { width: 500, height: 1200, safeRect: { x: 20, y: 180, width: 450, height: 700 } }],
  ] as const)("contains corners for %s viewport $1.width x $1.height", (projection, viewport) => {
    const { rig } = frame(projection, viewport);
    expectInside(rig, bounds, viewport);
  });

  it("uses the full viewport only while safe-area measurement is uninitialized", () => {
    const viewport = { width: 1000, height: 600 };
    const { rig } = frame("persp", viewport);
    expectInside(rig, bounds, viewport);
  });

  it("refuses an initialized safe rect wholly outside the viewport", () => {
    const offset = sphericalToOffset(0, 0, 1);
    expect(computeCameraFit({
      bounds,
      basis: cameraBasisForOffset(offset),
      projection: "persp",
      verticalFovDeg: 76,
      aspect: 1,
      viewport: { width: 500, height: 500, safeRect: { x: 600, y: 0, width: 100, height: 100 } },
      yaw: 0,
      pitch: 0,
    })).toBeNull();
  });

  it("refuses empty, non-finite, and initialized zero-area inputs", () => {
    const base = {
      basis: cameraBasisForOffset(new THREE.Vector3(1, 0, 0)),
      projection: "persp" as const,
      verticalFovDeg: 76,
      aspect: 1,
      viewport: { width: 500, height: 500 },
      yaw: 0,
      pitch: 0,
    };
    expect(computeCameraFit({ ...base, bounds: new THREE.Box3() })).toBeNull();
    expect(computeCameraFit({
      ...base,
      bounds: new THREE.Box3(new THREE.Vector3(0, 0, 0), new THREE.Vector3(Infinity, 1, 1)),
    })).toBeNull();
    expect(computeCameraFit({
      ...base,
      bounds,
      viewport: { width: 500, height: 500, safeRect: { x: 10, y: 10, width: 0, height: 100 } },
    })).toBeNull();
  });

  it("clips a partly offscreen safe rect to the viewport", () => {
    const viewport = { width: 800, height: 600, safeRect: { x: -100, y: 80, width: 700, height: 620 } };
    const clipped = { width: 800, height: 600, safeRect: { x: 0, y: 80, width: 600, height: 520 } };
    const { rig } = frame("persp", viewport);
    expectInside(rig, bounds, clipped);
  });

  it.each(["persp", "ortho"] as const)("keeps a thin deep box inside near/far for %s", (projection) => {
    const deep = new THREE.Box3(new THREE.Vector3(-900, -1, -1), new THREE.Vector3(900, 1, 1));
    const viewport = { width: 800, height: 600 };
    const rig = new CameraRig(76);
    rig.setAspect(viewport.width / viewport.height);
    rig.setProjection(projection);
    const yaw = 0;
    const pitch = 0;
    const offset = sphericalToOffset(yaw, pitch, 1);
    const result = computeCameraFit({
      bounds: deep,
      basis: rig.basisForOffset(offset),
      projection,
      verticalFovDeg: rig.fovDeg,
      aspect: rig.aspectRatio,
      viewport,
      yaw,
      pitch,
    });
    expect(result).not.toBeNull();
    rig.apply(result!.target, offset.multiplyScalar(result!.distance), result!.distance);
    expectInside(rig, deep, viewport);
  });

  it("refuses a frame beyond the controls distance limit", () => {
    const huge = new THREE.Box3(
      new THREE.Vector3(-CAMERA_MAX_DISTANCE, -CAMERA_MAX_DISTANCE, -1),
      new THREE.Vector3(CAMERA_MAX_DISTANCE, CAMERA_MAX_DISTANCE, 1),
    );
    expect(computeCameraFit({
      bounds: huge,
      basis: cameraBasisForOffset(new THREE.Vector3(1, 0, 0)),
      projection: "persp",
      verticalFovDeg: 76,
      aspect: 1,
      viewport: { width: 500, height: 500 },
      yaw: 0,
      pitch: 0,
    })).toBeNull();
  });

  it("matches the actual rig orientation basis", () => {
    const viewport = { width: 900, height: 700, safeRect: { x: 170, y: 50, width: 650, height: 570 } };
    const { rig } = frame("persp", viewport, -1.2, 1.1);
    const actual = rig.getCamera();
    const right = new THREE.Vector3().setFromMatrixColumn(actual.matrixWorld, 0);
    const up = new THREE.Vector3().setFromMatrixColumn(actual.matrixWorld, 1);
    const back = new THREE.Vector3().setFromMatrixColumn(actual.matrixWorld, 2);
    const expected = rig.basisForOffset(sphericalToOffset(-1.2, 1.1, 1));
    expect(right.distanceTo(expected.right)).toBeLessThan(1e-12);
    expect(up.distanceTo(expected.up)).toBeLessThan(1e-12);
    expect(back.distanceTo(expected.back)).toBeLessThan(1e-12);
  });
});
