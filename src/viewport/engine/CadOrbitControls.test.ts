import { describe, it, expect } from "vitest";
import * as THREE from "three";
import {
  clampPitch,
  sphericalToOffset,
  zoomToCursor,
  shortestAngleLerp,
  easeInOutCubic,
  yawForPlaneXAxis,
} from "./CadOrbitControls";
import { CameraRig } from "./CameraRig";

describe("clampPitch", () => {
  it("clamps to just inside ±90°", () => {
    expect(clampPitch(Math.PI)).toBeCloseTo(Math.PI / 2 - 1e-3, 6);
    expect(clampPitch(-Math.PI)).toBeCloseTo(-(Math.PI / 2 - 1e-3), 6);
    expect(clampPitch(0.3)).toBe(0.3);
    expect(clampPitch(Math.PI / 2)).toBeLessThan(Math.PI / 2);
  });
});

describe("sphericalToOffset (turntable, yaw about Z)", () => {
  it("yaw=0 pitch=0 points along +X", () => {
    const o = sphericalToOffset(0, 0, 5);
    expect(o.x).toBeCloseTo(5, 6);
    expect(o.y).toBeCloseTo(0, 6);
    expect(o.z).toBeCloseTo(0, 6);
  });
  it("pitch=+90° points along +Z (top)", () => {
    const o = sphericalToOffset(0, Math.PI / 2, 5);
    expect(o.z).toBeCloseTo(5, 6);
    expect(Math.hypot(o.x, o.y)).toBeCloseTo(0, 6);
  });
  it("radius is preserved", () => {
    const o = sphericalToOffset(1.1, 0.4, 7);
    expect(o.length()).toBeCloseTo(7, 6);
  });
});

/*
 * yawForPlaneXAxis / viewAlongNormal — entering a sketch must put the PLANE's
 * own xAxis on screen-right, not a generic "world +X is right" heading. The
 * XY sketch plane's pinned basis (SCHEMA §"non-standard XY basis") is
 * xAxis = world +Y — a generic X-right pole heading gets it backwards (SP-1
 * W/H swap bug).
 */
describe("yawForPlaneXAxis", () => {
  it("XY plane's own xAxis (world +Y) yaws to 0 — right vector becomes +Y", () => {
    const yaw = yawForPlaneXAxis(new THREE.Vector3(0, 1, 0));
    expect(yaw).toBeCloseTo(0, 9);
  });

  it("XZ plane's own xAxis (world +Y) matches its already-correct normal-derived yaw (0)", () => {
    const yaw = yawForPlaneXAxis(new THREE.Vector3(0, 1, 0));
    expect(yaw).toBeCloseTo(0, 9);
  });

  it("YZ plane's own xAxis (world −X) matches its already-correct normal-derived yaw (π/2)", () => {
    const yaw = yawForPlaneXAxis(new THREE.Vector3(-1, 0, 0));
    expect(yaw).toBeCloseTo(Math.PI / 2, 9);
  });

  it("a naive world +X axis yaws to −π/2 (today's old default pole heading)", () => {
    const yaw = yawForPlaneXAxis(new THREE.Vector3(1, 0, 0));
    expect(yaw).toBeCloseTo(-Math.PI / 2, 9);
  });

  it("returns null for a tilted (non-horizontal) plane axis — no exact zero-roll match", () => {
    expect(yawForPlaneXAxis(new THREE.Vector3(0, 0.7, 0.7))).toBeNull();
  });
});

describe("viewAlongNormal + CameraRig — plane axis ends up on screen-right/up", () => {
  const ORIGIN = new THREE.Vector3(0, 0, 0);
  const R = 100;

  function offsetFor(yaw: number, pitch: number): THREE.Vector3 {
    const cp = Math.cos(pitch);
    return new THREE.Vector3(R * cp * Math.cos(yaw), R * cp * Math.sin(yaw), R * Math.sin(pitch));
  }
  const rightVector = (rig: CameraRig) => new THREE.Vector3().setFromMatrixColumn(rig.persp.matrixWorld, 0);
  const upVector = (rig: CameraRig) => new THREE.Vector3().setFromMatrixColumn(rig.persp.matrixWorld, 1);
  // Pole pitch, matching `viewAlongNormal`'s `clampPitch(asin(normal.z))` for a Z-pole normal.
  const POLE_PITCH = clampPitch(Math.PI / 2);

  it("XY plane (xAxis world +Y, yAxis world −X): screen-right/up match the plane's own axes", () => {
    const rig = new CameraRig();
    const yaw = yawForPlaneXAxis(new THREE.Vector3(0, 1, 0))!;
    rig.apply(ORIGIN, offsetFor(yaw, POLE_PITCH), R);
    const right = rightVector(rig);
    const up = upVector(rig);
    expect(right.x).toBeCloseTo(0, 3);
    expect(right.y).toBeCloseTo(1, 3); // plane xAxis = world +Y
    expect(up.x).toBeCloseTo(-1, 3); // plane yAxis = world −X
    expect(up.y).toBeCloseTo(0, 3);
  });
});

describe("zoomToCursor", () => {
  const cam = new THREE.Vector3(0, 0, 10);
  const target = new THREE.Vector3(0, 0, 0);

  it("factor 1 is a no-op", () => {
    const r = zoomToCursor(cam, target, new THREE.Vector3(3, 4, 0), 1);
    expect(r.camPos.toArray()).toEqual([0, 0, 10]);
    expect(r.target.toArray()).toEqual([0, 0, 0]);
  });

  it("zooming toward the target just halves the eye distance", () => {
    const r = zoomToCursor(cam, target, target, 0.5);
    expect(r.target.toArray()).toEqual([0, 0, 0]);
    expect(r.camPos.z).toBeCloseTo(5, 6);
  });

  it("zooming toward an off-axis cursor pulls the target toward it", () => {
    const cursor = new THREE.Vector3(10, 0, 0);
    const r = zoomToCursor(cam, target, cursor, 0.5);
    expect(r.target.x).toBeCloseTo(5, 6); // halfway to cursor
  });
});

describe("shortestAngleLerp", () => {
  it("takes the short way around the circle", () => {
    expect(shortestAngleLerp(0, (3 * Math.PI) / 2, 1)).toBeCloseTo(-Math.PI / 2, 6);
    expect(shortestAngleLerp(0, Math.PI / 2, 0.5)).toBeCloseTo(Math.PI / 4, 6);
  });
});

describe("easeInOutCubic", () => {
  it("has fixed endpoints and midpoint", () => {
    expect(easeInOutCubic(0)).toBe(0);
    expect(easeInOutCubic(1)).toBe(1);
    expect(easeInOutCubic(0.5)).toBeCloseTo(0.5, 6);
  });
});
