/*
 * Constant-on-screen sizing for world-space gizmos (origin triad, plane picker).
 *
 * A gizmo authored at a fixed WORLD size grows and shrinks on screen as you
 * zoom — too big when close, invisible when far. Scaling it by the world size
 * of one screen pixel at its anchor cancels the projection out, so it holds a
 * fixed pixel size at every zoom level.
 */
import * as THREE from "three";

const _camPos = new THREE.Vector3();
const _viewDir = new THREE.Vector3();
const _quat = new THREE.Quaternion();
const _toAnchor = new THREE.Vector3();

/** Guard against a degenerate scale when the anchor sits on the camera. */
const MIN_DEPTH = 1e-3;

/**
 * World units covered by one CSS pixel at `anchor`.
 *
 * Perspective apparent size falls off with the view-axis DEPTH of the anchor,
 * not the camera's orbit distance — the two differ as soon as the user pans the
 * target away from the anchor, which is exactly when a distance-based scale
 * sizes the gizmo wrong. Orthographic has no falloff at all, so the frustum
 * height alone decides it.
 *
 * `height` is the viewport height in CSS px.
 */
export function worldPerPixel(camera: THREE.Camera, anchor: THREE.Vector3, height: number): number {
  const h = Math.max(height, 1);
  const persp = camera as THREE.PerspectiveCamera;
  if (persp.isPerspectiveCamera) {
    camera.getWorldPosition(_camPos);
    _viewDir.set(0, 0, -1).applyQuaternion(camera.getWorldQuaternion(_quat));
    _toAnchor.copy(anchor).sub(_camPos);
    const depth = Math.max(_toAnchor.dot(_viewDir), MIN_DEPTH);
    return (2 * depth * Math.tan((persp.fov * Math.PI) / 360)) / h;
  }
  const ortho = camera as THREE.OrthographicCamera;
  return (ortho.top - ortho.bottom) / (ortho.zoom || 1) / h;
}
