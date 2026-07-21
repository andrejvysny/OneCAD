/*
 * Camera rig: a PerspectiveCamera + OrthographicCamera pair sharing one logical
 * state (target, yaw, pitch, distance). The active camera is applied from that
 * state each change. Switching persp⇄ortho preserves apparent size at the pivot
 * distance via `orthoHalfHeight` (ortho half-height = distance * tan(fov/2)), so
 * geometry at the target keeps the same on-screen size across the switch.
 *
 * HARD INVARIANT: both cameras use up = (0,0,1). The world is Z-up right-handed
 * and mesh buffers are uploaded verbatim (see README.md). Never rotate content
 * to fake a Y-up world.
 */
import * as THREE from "three";

export type ProjectionKind = "persp" | "ortho";

const UP = new THREE.Vector3(0, 0, 1);
const NEAR = 0.1;
const FAR = 100_000;

// Scratch vectors + basis for the turntable orientation (no per-frame allocation).
const _right = new THREE.Vector3();
const _fwd = new THREE.Vector3();
const _up = new THREE.Vector3();
const _basis = new THREE.Matrix4();

/**
 * Pure: orthographic half-height that matches a perspective camera's apparent
 * size at `distance`. halfH = distance * tan(fovDeg/2).
 */
export function orthoHalfHeight(distance: number, fovDeg: number): number {
  return distance * Math.tan((fovDeg * Math.PI) / 360);
}

export interface CameraState {
  target: THREE.Vector3;
  /** Eye offset from target (target → camera). */
  offset: THREE.Vector3;
  distance: number;
}

export class CameraRig {
  readonly persp: THREE.PerspectiveCamera;
  readonly ortho: THREE.OrthographicCamera;
  private kind: ProjectionKind = "persp";
  private aspect = 1;
  readonly fovDeg: number;

  constructor(fovDeg = 76) {
    this.fovDeg = fovDeg;
    this.persp = new THREE.PerspectiveCamera(fovDeg, 1, NEAR, FAR);
    this.persp.up.copy(UP);
    this.ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, NEAR, FAR);
    this.ortho.up.copy(UP);
  }

  get projection(): ProjectionKind {
    return this.kind;
  }

  getCamera(): THREE.Camera {
    return this.kind === "persp" ? this.persp : this.ortho;
  }

  setAspect(aspect: number): void {
    this.aspect = aspect > 0 ? aspect : 1;
    this.persp.aspect = this.aspect;
    this.persp.updateProjectionMatrix();
  }

  setProjection(kind: ProjectionKind): void {
    this.kind = kind;
  }

  /**
   * Place the active camera. `offset` is the target→camera vector (length =
   * distance). In ortho, the frustum half-height is derived from `distance` so
   * the perspective apparent size is preserved.
   */
  apply(target: THREE.Vector3, offset: THREE.Vector3, distance: number): void {
    const cam = this.getCamera();
    cam.position.copy(target).add(offset);
    this.orient(cam, offset);
    if (this.kind === "ortho") {
      const halfH = orthoHalfHeight(distance, this.fovDeg);
      const halfW = halfH * this.aspect;
      this.ortho.left = -halfW;
      this.ortho.right = halfW;
      this.ortho.top = halfH;
      this.ortho.bottom = -halfH;
      this.ortho.updateProjectionMatrix();
    }
    cam.updateMatrixWorld();
  }

  /**
   * Orient a turntable camera from its target→camera offset.
   *
   * NOT `lookAt`. `lookAt` derives the camera's roll from an up vector, and with
   * world-Z up that roll is decided by the offset's horizontal component — which
   * vanishes as the view approaches straight-down. The result is a roll that
   * spins ever faster near the pole, and (if you swap `up` to dodge that) snaps
   * by a finite angle the instant you cross the swap threshold. Either way the
   * view appears to rotate on its own while the user is just orbiting.
   *
   * A turntable camera has no independent roll: its right vector is ALWAYS
   * horizontal and depends only on yaw, which the offset's horizontal direction
   * encodes exactly. Building the basis from that is continuous through the
   * poles and never self-aligns. Only an explicit ViewCube / named-view snap
   * changes the heading.
   */
  private orient(cam: THREE.Camera, offset: THREE.Vector3): void {
    // Horizontal heading ⇒ right vector. Perpendicular to (x, y) in the XY plane.
    _right.set(-offset.y, offset.x, 0);
    if (_right.lengthSq() < 1e-20) {
      // Exactly down a pole: yaw is genuinely undefined here. Keep the camera's
      // current right vector so the view holds still instead of jumping.
      // `clampPitch` normally keeps us out of this case entirely.
      _right.setFromMatrixColumn(cam.matrixWorld, 0);
      _right.z = 0;
      if (_right.lengthSq() < 1e-20) _right.set(1, 0, 0);
    }
    _right.normalize();

    _fwd.copy(offset).negate().normalize(); // target→camera reversed = view dir
    _up.crossVectors(_right, _fwd).normalize();

    // Three's camera looks down its local -Z, so the third basis column is -fwd.
    _basis.makeBasis(_right, _up, _fwd.negate());
    cam.quaternion.setFromRotationMatrix(_basis);
    // Keep `up` consistent for anything that reads it (it no longer drives roll).
    cam.up.copy(UP);
  }
}
