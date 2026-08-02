/*
 * Light-rig pose math.
 *
 * The load-bearing property is the key's elevation FLOOR: a purely
 * camera-relative key swings below the horizon when the user orbits under the
 * model, which would light bottom faces brightest and invert the shape cue.
 * These tests pin that floor, the camera-relative azimuths, and the degenerate
 * inputs (pole pitch, zero distance).
 */
import { describe, it, expect } from "vitest";
import * as THREE from "three";
import {
  lightRigPose,
  KEY_AZ,
  KEY_EL,
  KEY_EL_FLOOR,
  FILL_AZ,
  type LightRigPose,
} from "./lightRig";

const DEG = Math.PI / 180;
const ORIGIN = new THREE.Vector3(0, 0, 0);

/** Elevation above the XY plane of an offset vector (the inverse of the turntable basis). */
function elevationOf(p: THREE.Vector3, target: THREE.Vector3): number {
  const d = p.clone().sub(target);
  return Math.atan2(d.z, Math.hypot(d.x, d.y));
}

function azimuthOf(p: THREE.Vector3, target: THREE.Vector3): number {
  const d = p.clone().sub(target);
  return Math.atan2(d.y, d.x);
}

/** Signed shortest angular difference in (−π, π]. */
function angleDelta(a: number, b: number): number {
  let d = (a - b) % (Math.PI * 2);
  if (d > Math.PI) d -= Math.PI * 2;
  if (d <= -Math.PI) d += Math.PI * 2;
  return d;
}

describe("lightRigPose key elevation", () => {
  it("never lets the key fall below the floor, at any camera pitch", () => {
    for (let deg = -80; deg <= 80; deg += 5) {
      const pose = lightRigPose(0.7, deg * DEG, ORIGIN, 100);
      const el = elevationOf(pose.key, ORIGIN);
      expect(el).toBeGreaterThanOrEqual(KEY_EL_FLOOR - 1e-9);
    }
  });

  it("tracks the camera pitch once above the floor", () => {
    const pitch = 30 * DEG; // 30 + 35 = 65°, well clear of the floor
    const pose = lightRigPose(0, pitch, ORIGIN, 100);
    expect(elevationOf(pose.key, ORIGIN)).toBeCloseTo(pitch + KEY_EL, 6);
  });

  it("clamps at the floor when the camera looks up from far below", () => {
    const pose = lightRigPose(0, -70 * DEG, ORIGIN, 100);
    expect(elevationOf(pose.key, ORIGIN)).toBeCloseTo(KEY_EL_FLOOR, 6);
  });
});

describe("lightRigPose azimuths", () => {
  it("puts the key at yaw + KEY_AZ (mod 2π) for any yaw", () => {
    for (const yaw of [-3, -1.2, 0, 0.9, 2.5, 6.0]) {
      const pose = lightRigPose(yaw, 20 * DEG, ORIGIN, 50);
      expect(angleDelta(azimuthOf(pose.key, ORIGIN), yaw + KEY_AZ)).toBeCloseTo(0, 6);
    }
  });

  it("puts the fill on the opposite side of the view axis from the key", () => {
    const yaw = 1.3;
    const pose = lightRigPose(yaw, 10 * DEG, ORIGIN, 50);
    expect(angleDelta(azimuthOf(pose.fill, ORIGIN), yaw + FILL_AZ)).toBeCloseTo(0, 6);
    // Key is CCW of the view heading, fill is CW — opposite signs.
    const keyRel = angleDelta(azimuthOf(pose.key, ORIGIN), yaw);
    const fillRel = angleDelta(azimuthOf(pose.fill, ORIGIN), yaw);
    expect(Math.sign(keyRel)).toBe(1);
    expect(Math.sign(fillRel)).toBe(-1);
  });

  it("offsets both lights from the TARGET, not the world origin", () => {
    const target = new THREE.Vector3(120, -40, 15);
    const pose = lightRigPose(0.4, 20 * DEG, target, 60);
    expect(pose.key.distanceTo(target)).toBeCloseTo(60, 6);
    expect(pose.fill.distanceTo(target)).toBeCloseTo(60, 6);
  });
});

describe("lightRigPose degenerate inputs", () => {
  it("stays finite at the pitch poles", () => {
    for (const pitch of [Math.PI / 2 - 1e-9, -Math.PI / 2 + 1e-9, Math.PI / 2, -Math.PI / 2]) {
      const pose = lightRigPose(0.2, pitch, ORIGIN, 100);
      for (const v of [pose.key, pose.fill]) {
        expect(Number.isFinite(v.x) && Number.isFinite(v.y) && Number.isFinite(v.z)).toBe(true);
        expect(v.length()).toBeGreaterThan(0);
      }
    }
  });

  it("floors the radius at 1 so a zoomed-in camera can't collapse the rig", () => {
    for (const d of [0, 0.2, -5]) {
      const pose = lightRigPose(0, 0, ORIGIN, d);
      expect(pose.key.length()).toBeCloseTo(1, 6);
      expect(pose.fill.length()).toBeCloseTo(1, 6);
    }
  });
});

describe("lightRigPose out-param", () => {
  it("writes in place and allocates nothing when given an out", () => {
    const out: LightRigPose = { key: new THREE.Vector3(), fill: new THREE.Vector3() };
    const key = out.key;
    const fill = out.fill;
    const got = lightRigPose(0.5, 0.3, ORIGIN, 80, out);
    expect(got).toBe(out);
    expect(got.key).toBe(key);
    expect(got.fill).toBe(fill);
    expect(key.length()).toBeCloseTo(80, 6);
  });
});
