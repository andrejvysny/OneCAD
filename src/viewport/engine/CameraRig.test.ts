import { describe, it, expect } from "vitest";
import * as THREE from "three";
import { CameraRig, orthoHalfHeight } from "./CameraRig";

describe("orthoHalfHeight (pure)", () => {
  it("matches distance * tan(fov/2)", () => {
    expect(orthoHalfHeight(100, 90)).toBeCloseTo(100 * Math.tan(Math.PI / 4), 6);
    expect(orthoHalfHeight(0, 76)).toBe(0);
    // larger fov → larger half-height at fixed distance
    expect(orthoHalfHeight(100, 90)).toBeGreaterThan(orthoHalfHeight(100, 60));
  });
});

describe("CameraRig persp⇄ortho apparent size", () => {
  it("ortho frustum preserves apparent size at the pivot distance", () => {
    const rig = new CameraRig(76);
    rig.setAspect(2);
    const target = new THREE.Vector3(0, 0, 0);
    const distance = 300;
    const offset = new THREE.Vector3(0, -distance, 0);

    rig.setProjection("ortho");
    rig.apply(target, offset, distance);

    const halfH = orthoHalfHeight(distance, 76);
    expect(rig.ortho.top).toBeCloseTo(halfH, 4);
    expect(rig.ortho.bottom).toBeCloseTo(-halfH, 4);
    expect(rig.ortho.right).toBeCloseTo(halfH * 2, 4); // aspect = 2
    expect(rig.ortho.left).toBeCloseTo(-halfH * 2, 4);
  });

  it("keeps up = world Z on both cameras", () => {
    const rig = new CameraRig();
    expect(rig.persp.up.toArray()).toEqual([0, 0, 1]);
    expect(rig.ortho.up.toArray()).toEqual([0, 0, 1]);
  });

  it("places the active camera at target + offset", () => {
    const rig = new CameraRig();
    rig.setProjection("persp");
    rig.apply(new THREE.Vector3(1, 2, 3), new THREE.Vector3(0, 0, 10), 10);
    expect(rig.persp.position.toArray()).toEqual([1, 2, 13]);
  });
});

describe("CameraRig turntable orientation", () => {
  const ORIGIN = new THREE.Vector3(0, 0, 0);
  const R = 100;

  /** target→camera offset for a yaw/pitch, mirroring sphericalToOffset. */
  function offsetFor(yaw: number, pitch: number): THREE.Vector3 {
    const cp = Math.cos(pitch);
    return new THREE.Vector3(R * cp * Math.cos(yaw), R * cp * Math.sin(yaw), R * Math.sin(pitch));
  }

  function rightVector(rig: CameraRig): THREE.Vector3 {
    return new THREE.Vector3().setFromMatrixColumn(rig.persp.matrixWorld, 0);
  }

  function upVector(rig: CameraRig): THREE.Vector3 {
    return new THREE.Vector3().setFromMatrixColumn(rig.persp.matrixWorld, 1);
  }

  it("keeps the right vector horizontal at every pitch", () => {
    const rig = new CameraRig();
    for (const pitch of [0, 0.5, 1.0, 1.5, Math.PI / 2 - 1e-3]) {
      rig.apply(ORIGIN, offsetFor(-Math.PI / 2, pitch), R);
      expect(rightVector(rig).z).toBeCloseTo(0, 9);
    }
  });

  it("does not roll as the view approaches straight down", () => {
    // The bug: crossing a pole threshold swapped the up vector and rotated the
    // view by a finite angle while the user was merely orbiting.
    const rig = new CameraRig();
    const yaw = -Math.PI / 3;
    rig.apply(ORIGIN, offsetFor(yaw, 1.5), R);
    const before = rightVector(rig).clone();
    // Step well past where POLE_COS = 0.99999 used to trigger (~89.74°).
    rig.apply(ORIGIN, offsetFor(yaw, Math.PI / 2 - 1e-3), R);
    const after = rightVector(rig);
    expect(after.angleTo(before)).toBeCloseTo(0, 6);
  });

  it("varies continuously across the former pole threshold", () => {
    const rig = new CameraRig();
    const yaw = 0.3;
    // Just below and just above sin(pitch) = 0.99999.
    rig.apply(ORIGIN, offsetFor(yaw, Math.asin(0.999985)), R);
    const a = rightVector(rig).clone();
    rig.apply(ORIGIN, offsetFor(yaw, Math.asin(0.999995)), R);
    expect(rightVector(rig).angleTo(a)).toBeLessThan(1e-4);
  });

  it("orients a canonical TOP view as X right, Y up", () => {
    const rig = new CameraRig();
    rig.apply(ORIGIN, offsetFor(-Math.PI / 2, Math.PI / 2 - 1e-3), R);
    const right = rightVector(rig);
    const up = upVector(rig);
    expect(right.x).toBeCloseTo(1, 3);
    expect(right.y).toBeCloseTo(0, 3);
    expect(up.y).toBeCloseTo(1, 3);
  });

  it("still looks at the target", () => {
    const rig = new CameraRig();
    const target = new THREE.Vector3(5, -2, 3);
    const offset = offsetFor(0.7, 0.9);
    rig.apply(target, offset, R);
    // Camera looks down its local -Z.
    const fwd = new THREE.Vector3().setFromMatrixColumn(rig.persp.matrixWorld, 2).negate();
    const toTarget = target.clone().sub(rig.persp.position).normalize();
    expect(fwd.angleTo(toTarget)).toBeCloseTo(0, 6);
  });

  it("keeps a right-handed basis", () => {
    const rig = new CameraRig();
    rig.apply(ORIGIN, offsetFor(1.1, -0.4), R);
    const right = rightVector(rig);
    const up = upVector(rig);
    const back = new THREE.Vector3().setFromMatrixColumn(rig.persp.matrixWorld, 2);
    expect(new THREE.Vector3().crossVectors(right, up).angleTo(back)).toBeCloseTo(0, 6);
  });
});
