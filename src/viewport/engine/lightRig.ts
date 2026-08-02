/*
 * Camera-relative light rig — pure math (no Three.js scene state).
 *
 * A single headlight sitting AT the camera lights every visible face at nearly
 * the same incidence, which flattens form: a box reads as one silhouette with no
 * shading gradient between its faces. A studio rig instead OFFSETS a key light
 * from the view direction (so faces pick up distinct intensities) and adds a
 * dimmer fill on the opposite side (so the faces turned away from the key don't
 * crush to the ambient floor).
 *
 * Both lights are camera-relative — they orbit with the view — so the shading
 * cue stays stable as the user tumbles the model instead of sweeping across it.
 * Positions are computed in the same turntable spherical basis the controls use
 * (`sphericalToOffset`: yaw about world Z, pitch from the XY plane), which is
 * what keeps the rig honest under the Z-up invariant.
 */
import * as THREE from "three";
import { clampPitch, sphericalToOffset } from "./CadOrbitControls";

const DEG = Math.PI / 180;

/** Key light azimuth offset from the camera yaw (classic over-the-shoulder key). */
export const KEY_AZ = 40 * DEG;
/** Key light elevation offset above the camera pitch. */
export const KEY_EL = 35 * DEG;
/**
 * Hard floor on the key's ABSOLUTE elevation.
 *
 * A purely camera-relative key drops below the horizon as soon as the user
 * orbits under the model, which lights the underside brightest and inverts the
 * "up is brighter" cue every CAD user reads shape from. Flooring the key while
 * leaving the fill unfloored keeps under-views legible (the fill still comes
 * from below) without ever letting a bottom face out-shine a top face.
 */
export const KEY_EL_FLOOR = 20 * DEG;
/** Fill azimuth offset — well to the other side of the view axis from the key. */
export const FILL_AZ = -75 * DEG;
/** Fill elevation offset — slightly below the camera, so it reads as bounce. */
export const FILL_EL = -15 * DEG;

/** Minimum rig radius, so a fully zoomed-in camera can't collapse the lights onto the target. */
const MIN_RADIUS = 1;

export interface LightRigPose {
  key: THREE.Vector3;
  fill: THREE.Vector3;
}

/**
 * World positions for the key and fill lights given the current camera orbit
 * state. Pass `out` to write in place — the render loop reuses one pose object
 * so a rendered frame allocates nothing.
 */
export function lightRigPose(
  yaw: number,
  pitch: number,
  target: THREE.Vector3,
  distance: number,
  out?: LightRigPose,
): LightRigPose {
  const r = Math.max(distance, MIN_RADIUS);
  const keyPitch = clampPitch(Math.max(pitch + KEY_EL, KEY_EL_FLOOR));
  const fillPitch = clampPitch(pitch + FILL_EL);

  const pose: LightRigPose = out ?? { key: new THREE.Vector3(), fill: new THREE.Vector3() };
  // Write in place (sphericalToOffset's out-param) so a rendered frame allocates nothing.
  sphericalToOffset(yaw + KEY_AZ, keyPitch, r, pose.key).add(target);
  sphericalToOffset(yaw + FILL_AZ, fillPitch, r, pose.fill).add(target);
  return pose;
}
