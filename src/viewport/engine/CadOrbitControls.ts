/*
 * CadOrbitControls — a small custom orbit controller (no three examples dep).
 *
 * MOUSE
 *   LMB drag             = selection / tool drags ONLY. Never orbits — left
 *                          button is fully reserved for pick/gizmo/sketch gestures.
 *   RMB + Shift drag     = turntable orbit: yaw about WORLD Z, pitch clamped to
 *                          ±(90° − ε).
 *   MMB drag / RMB drag  = pan in the view plane (RMB pans when Shift is not held).
 *   Wheel                = zoom-to-cursor dolly (persp) / frustum zoom (ortho).
 *
 * TRACKPAD (macOS + Windows precision touchpads — the Fusion 360 convention)
 *   two-finger scroll         = pan
 *   shift + two-finger scroll = orbit (drag-direction on macOS, see wheelOrbitSign)
 *   pinch                     = zoom-to-cursor
 *
 * Touch: 1-finger drag orbits, 2-finger drag pans + pinch-zooms.
 *
 * Wheel and WebKit gesture events carry no device identity, so all routing —
 * trackpad-vs-mouse detection, gesture segmentation, modifier latching and
 * pinch-source arbitration — lives in the pure `navInput` reducer. This class
 * is only an adapter: DOM event → NavEvent → navReduce → apply the NavOp.
 *
 *   Home                 = animated iso view (250ms).
 *   Fit                  = frame the scene bbox (animated).
 *
 * No damping in v1. Camera state is (target, yaw, pitch, distance); the rig maps
 * it to the active camera. Math helpers are pure and unit-tested.
 */
import * as THREE from "three";
import type { CameraRig } from "./CameraRig";
import {
  navInit,
  navReduce,
  type DevicePref,
  type InputDevice,
  type NavEvent,
  type NavState,
} from "./navInput";

/**
 * WebKit's proprietary pinch/rotate gesture events (Safari + WKWebView, which is
 * what Tauri embeds on macOS). Not in lib.dom, so declare the shape we read.
 */
interface WebKitGestureEvent extends UIEvent {
  readonly scale: number;
  readonly rotation: number;
  readonly clientX: number;
  readonly clientY: number;
}

// ---- Pure math helpers (unit-tested) -------------------------------------

const HALF_PI = Math.PI / 2;

/** Clamp pitch to just inside ±90° so the Z-up lookAt never degenerates. */
export function clampPitch(pitch: number, eps = 1e-3): number {
  const lim = HALF_PI - eps;
  return Math.max(-lim, Math.min(lim, pitch));
}

/**
 * Turntable spherical → target→camera offset (yaw about Z, pitch from XY plane).
 *
 * `out` writes in place for per-frame callers (the light rig) that must not
 * allocate; omitting it returns a fresh vector as before.
 */
export function sphericalToOffset(
  yaw: number,
  pitch: number,
  radius: number,
  out?: THREE.Vector3,
): THREE.Vector3 {
  const cp = Math.cos(pitch);
  return (out ?? new THREE.Vector3()).set(
    radius * cp * Math.cos(yaw),
    radius * cp * Math.sin(yaw),
    radius * Math.sin(pitch),
  );
}

/**
 * Zoom about a world point: scale camera position and target toward `cursor` by
 * `factor`. The camera→target vector keeps its direction (distance *= factor)
 * and the point under the cursor stays fixed on screen.
 */
export function zoomToCursor(
  camPos: THREE.Vector3,
  target: THREE.Vector3,
  cursor: THREE.Vector3,
  factor: number,
): { camPos: THREE.Vector3; target: THREE.Vector3 } {
  const lerpToward = (p: THREE.Vector3) =>
    new THREE.Vector3().copy(cursor).lerp(p, factor);
  return { camPos: lerpToward(camPos), target: lerpToward(target) };
}

/** Shortest-path angular lerp (handles wraparound). */
export function shortestAngleLerp(a: number, b: number, t: number): number {
  let d = (b - a) % (Math.PI * 2);
  if (d > Math.PI) d -= Math.PI * 2;
  if (d < -Math.PI) d += Math.PI * 2;
  return a + d * t;
}

export function easeInOutCubic(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

/**
 * Heading a TOP/BOTTOM view snaps to. Looking straight down a pole the offset
 * has no horizontal component, so yaw carries no information — `atan2(0, 0)`
 * would silently pick 0 and give a sideways TOP. −90° is the conventional CAD
 * top: X to the right, Y up.
 */
const CANONICAL_POLE_YAW = -Math.PI / 2;
/** Below this horizontal component, a direction counts as straight down a pole. */
const POLE_EPS = 1e-6;

/**
 * Yaw for a target→camera direction. Falls back to the canonical heading only
 * when the direction is an exact pole, where yaw is undefined — an ordinary
 * near-pole orbit still uses its real yaw and is never re-aligned.
 */
export function yawForDirection(d: THREE.Vector3): number {
  return Math.hypot(d.x, d.y) < POLE_EPS ? CANONICAL_POLE_YAW : Math.atan2(d.y, d.x);
}

/**
 * macOS "natural" scrolling delivers wheel deltas inverted relative to finger
 * motion, so scroll-direction orbit feels backwards there — the model tilts
 * against the fingers. macOS trackpad orbit therefore runs in drag-direction
 * (same negation pan already applies); mouse and Windows-precision-touchpad
 * behaviour is unchanged. Exported for tests.
 */
export function isMacPlatform(
  nav: Pick<Navigator, "platform" | "userAgent"> = navigator,
): boolean {
  return /Mac/i.test(nav.platform) || /Macintosh/i.test(nav.userAgent);
}

// ---- Controller ----------------------------------------------------------

const ORBIT_SPEED = 0.008;
const PAN_SENS = 1.4;
const TWEEN_MS = 250;
const ISO_YAW = -Math.PI / 4;
const ISO_PITCH = Math.atan(1 / Math.SQRT2); // ~35.26°

interface TweenTarget {
  yaw: number;
  pitch: number;
  distance: number;
  target: THREE.Vector3;
}

interface Tween {
  from: TweenTarget;
  to: TweenTarget;
  start: number;
  dur: number;
}

export interface OrbitOptions {
  rig: CameraRig;
  element: HTMLElement;
  onChange: () => void;
  /** Scene bounds for Fit; null when empty. */
  getBounds: () => THREE.Box3 | null;
  /** User's input-device preference. Absent ⇒ "auto". */
  getDevicePref?: () => DevicePref;
  /**
   * True while a tool drag is IN FLIGHT (extrude depth, fillet radius, revolve
   * angle…). Those drags project the pointer against the current camera, so
   * moving it mid-drag makes the value jump — wheel-orbit is gated on this.
   *
   * Deliberately NOT `lmbOrbitSuppressed`: that flag is also set while a tool is
   * merely ARMED, and orbiting to look around while armed is expected CAD
   * behaviour that must keep working.
   */
  isDragActive?: () => boolean;
  /** Fires only when the detected device changes (drives the "Auto" label). */
  onDeviceChange?: (device: InputDevice) => void;
}

export class CadOrbitControls {
  private readonly rig: CameraRig;
  private readonly el: HTMLElement;
  private readonly onChange: () => void;
  private readonly getBounds: () => THREE.Box3 | null;
  private readonly getDevicePref: () => DevicePref;
  private readonly isDragActive: () => boolean;
  private readonly onDeviceChange: ((device: InputDevice) => void) | null;

  target = new THREE.Vector3(0, 0, 0);
  yaw = ISO_YAW;
  pitch = ISO_PITCH;
  distance = 260;

  private readonly pointers = new Map<number, THREE.Vector2>();
  /** Last seen pointer position — the pinch-origin fallback. */
  private lastPointer: { x: number; y: number } | null = null;
  private button = -1;
  private lastPinch = 0;
  private tween: Tween | null = null;
  /** Sticky suppression of LMB orbit (sketch drawing tools own LMB). */
  private lmbOrbitSuppressed = false;
  /** Pure reducer state for wheel + gesture routing. */
  private nav: NavState = navInit();
  private lastDevice: InputDevice | null = null;
  /** −1 on macOS: trackpad orbit runs in drag-direction there (see isMacPlatform). */
  private readonly wheelOrbitSign: 1 | -1;

  constructor(opts: OrbitOptions) {
    this.rig = opts.rig;
    this.el = opts.element;
    this.onChange = opts.onChange;
    this.getBounds = opts.getBounds;
    this.getDevicePref = opts.getDevicePref ?? (() => "auto");
    this.isDragActive = opts.isDragActive ?? (() => false);
    this.onDeviceChange = opts.onDeviceChange ?? null;
    this.wheelOrbitSign = isMacPlatform() ? -1 : 1;
    this.el.addEventListener("pointerdown", this.onPointerDown);
    this.el.addEventListener("pointermove", this.onPointerMove);
    this.el.addEventListener("pointerup", this.onPointerUp);
    this.el.addEventListener("pointercancel", this.onPointerUp);
    this.el.addEventListener("wheel", this.onWheel, { passive: false });
    this.el.addEventListener("contextmenu", this.preventContext);
    // WebKit pinch (Safari / WKWebView — i.e. Tauri on macOS). preventDefault on
    // these is what stops the webview magnifying the whole page.
    this.el.addEventListener("gesturestart", this.onGesture, { passive: false });
    this.el.addEventListener("gesturechange", this.onGesture, { passive: false });
    this.el.addEventListener("gestureend", this.onGesture, { passive: false });
  }

  getDistance(): number {
    return this.distance;
  }

  getTarget(): THREE.Vector3 {
    return this.target;
  }

  /** target→camera direction (unit), for the view label. */
  getViewDirection(): THREE.Vector3 {
    return sphericalToOffset(this.yaw, this.pitch, 1);
  }

  /** Push current state to the rig (no notification). */
  applyToRig(): void {
    const offset = sphericalToOffset(this.yaw, this.pitch, this.distance);
    this.rig.apply(this.target, offset, this.distance);
  }

  private commit(): void {
    this.applyToRig();
    this.onChange();
  }

  // ---- Pointer handling ----

  private onPointerDown = (e: PointerEvent): void => {
    this.el.setPointerCapture(e.pointerId);
    this.pointers.set(e.pointerId, new THREE.Vector2(e.clientX, e.clientY));
    if (this.pointers.size === 1) this.button = e.button;
    if (this.pointers.size === 2) this.lastPinch = this.pinchDistance();
    this.tween = null; // user input cancels any animation
  };

  private onPointerMove = (e: PointerEvent): void => {
    this.lastPointer = { x: e.clientX, y: e.clientY };
    const prev = this.pointers.get(e.pointerId);
    if (!prev) return;
    const dx = e.clientX - prev.x;
    const dy = e.clientY - prev.y;
    prev.set(e.clientX, e.clientY);

    if (this.pointers.size >= 2) {
      this.pan(dx, dy);
      this.applyPinchZoom(this.pinchDistance());
      return;
    }
    // RMB + Shift orbits; middle button and plain right button pan (CAD
    // convention). Left button never orbits — it is reserved for
    // selection / tool drags, handled outside this controller.
    if (this.button === 2 && e.shiftKey) this.orbit(dx, dy);
    else if (this.button === 1 || this.button === 2) this.pan(dx, dy);
  };

  private onPointerUp = (e: PointerEvent): void => {
    this.pointers.delete(e.pointerId);
    if (this.el.hasPointerCapture(e.pointerId)) {
      this.el.releasePointerCapture(e.pointerId);
    }
    if (this.pointers.size < 2) this.lastPinch = 0;
    if (this.pointers.size === 0) this.button = -1;
  };

  private onWheel = (e: WheelEvent): void => {
    if (e.cancelable) e.preventDefault();
    this.dispatchNav({
      type: "wheel",
      deltaX: e.deltaX,
      deltaY: e.deltaY,
      deltaMode: e.deltaMode,
      ctrlKey: e.ctrlKey,
      shiftKey: e.shiftKey,
      clientX: e.clientX,
      clientY: e.clientY,
      t: e.timeStamp,
    });
  };

  /** WebKit `gesturestart`/`gesturechange`/`gestureend` — macOS trackpad pinch. */
  private onGesture = (e: Event): void => {
    if (e.cancelable) e.preventDefault();
    const g = e as WebKitGestureEvent;
    if (e.type === "gestureend") {
      this.dispatchNav({ type: "gestureend", t: e.timeStamp });
      return;
    }
    // If the embedded webview ever omits coordinates, fall back to the last
    // pointer position so pinch still zooms somewhere sensible rather than (0,0).
    const at = this.gestureOrigin(g);
    this.dispatchNav({
      type: e.type === "gesturestart" ? "gesturestart" : "gesturechange",
      scale: g.scale,
      clientX: at.x,
      clientY: at.y,
      t: e.timeStamp,
    });
  };

  private gestureOrigin(g: WebKitGestureEvent): { x: number; y: number } {
    if (Number.isFinite(g.clientX) && Number.isFinite(g.clientY) && (g.clientX || g.clientY)) {
      return { x: g.clientX, y: g.clientY };
    }
    if (this.lastPointer) return { x: this.lastPointer.x, y: this.lastPointer.y };
    const rect = this.el.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  }

  /** Feed the reducer and apply whatever operation it returns. */
  private dispatchNav(ev: NavEvent): void {
    const { state, op, detected } = navReduce(this.nav, ev, {
      pref: this.getDevicePref(),
      viewportH: this.el.clientHeight,
    });
    this.nav = state;
    if (detected !== this.lastDevice) {
      this.lastDevice = detected;
      this.onDeviceChange?.(detected);
    }
    if (!op) return;
    switch (op.kind) {
      case "pan":
        // Wheel deltas are scroll-direction; pan() takes drag-direction.
        this.pan(-op.dx, -op.dy);
        break;
      case "orbit":
        // A camera move mid-drag would make the dragged value jump.
        if (!this.isDragActive())
          this.applyOrbit(this.wheelOrbitSign * op.dx, this.wheelOrbitSign * op.dy);
        break;
      case "zoom":
        this.zoomAtScreen(op.clientX, op.clientY, op.factor);
        break;
    }
  }

  private preventContext = (e: Event): void => e.preventDefault();

  // ---- Gestures ----

  /** Sticky: while true, an LMB drag never orbits (sketch tools own LMB). */
  setLmbOrbitSuppressed(suppressed: boolean): void {
    this.lmbOrbitSuppressed = suppressed;
  }

  /** RMB+Shift-drag orbit — gated by the modal-tool suppression flag. */
  private orbit(dx: number, dy: number): void {
    if (this.lmbOrbitSuppressed) return; // a modal tool (LMB-driven) owns the drag
    this.applyOrbit(dx, dy);
  }

  /**
   * The orbit math itself, ungated. Shift+scroll orbit routes here directly: the
   * suppression flag above exists to stop a pointer-drag orbit while a modal tool
   * owns the pointer, and a wheel gesture cannot collide with a pointer drag.
   */
  private applyOrbit(dx: number, dy: number): void {
    this.yaw -= dx * ORBIT_SPEED;
    this.pitch = clampPitch(this.pitch + dy * ORBIT_SPEED);
    this.commit();
  }

  /**
   * Public ungated orbit by raw screen deltas — the path a ViewCube drag uses.
   * Same turntable math + speed as the RMB+Shift drag so both gestures feel
   * identical. Never runs through the LMB-suppression flag (the cube owns its
   * pointer). Direction is drag-following: the grabbed cube spins with the
   * pointer, matching RMB-orbit's `yaw -= dx`.
   */
  orbitBy(dx: number, dy: number): void {
    this.applyOrbit(dx, dy);
  }

  /** Stop any in-flight snap/zoom tween (a ViewCube drag grabs the camera). */
  cancelAnimation(): void {
    this.tween = null;
  }

  private pan(dx: number, dy: number): void {
    const cam = this.rig.getCamera();
    const right = new THREE.Vector3().setFromMatrixColumn(cam.matrixWorld, 0);
    const up = new THREE.Vector3().setFromMatrixColumn(cam.matrixWorld, 1);
    const scale = (this.distance * PAN_SENS) / Math.max(this.el.clientHeight, 1);
    this.target.addScaledVector(right, -dx * scale);
    this.target.addScaledVector(up, dy * scale);
    this.commit();
  }

  /** Wheel/pinch zoom at a screen point (zoom-to-cursor). */
  private zoomAtScreen(clientX: number, clientY: number, factor: number): void {
    const cursor = this.worldOnTargetPlane(clientX, clientY);
    const cam = this.rig.getCamera();
    if (cursor) {
      const { target } = zoomToCursor(cam.position.clone(), this.target, cursor, factor);
      this.target.copy(target);
    }
    this.distance = clampDistance(this.distance * factor);
    this.commit();
  }

  private applyPinchZoom(pinch: number): void {
    if (this.lastPinch <= 0) {
      this.lastPinch = pinch;
      return;
    }
    const factor = clampFactor(this.lastPinch / pinch);
    this.lastPinch = pinch;
    this.distance = clampDistance(this.distance * factor);
    this.commit();
  }

  private pinchDistance(): number {
    const pts = [...this.pointers.values()];
    if (pts.length < 2) return 0;
    return pts[0].distanceTo(pts[1]);
  }

  /** Ray from the cursor onto the plane through the target facing the camera. */
  private worldOnTargetPlane(clientX: number, clientY: number): THREE.Vector3 | null {
    const rect = this.el.getBoundingClientRect();
    const ndc = new THREE.Vector2(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -(((clientY - rect.top) / rect.height) * 2 - 1),
    );
    const cam = this.rig.getCamera();
    const ray = new THREE.Raycaster();
    ray.setFromCamera(ndc, cam);
    const normal = new THREE.Vector3().subVectors(cam.position, this.target).normalize();
    const plane = new THREE.Plane().setFromNormalAndCoplanarPoint(normal, this.target);
    const hit = new THREE.Vector3();
    return ray.ray.intersectPlane(plane, hit) ? hit : null;
  }

  // ---- Named views / framing ----

  homeView(animated = true): void {
    const to: TweenTarget = {
      yaw: ISO_YAW,
      pitch: ISO_PITCH,
      distance: this.defaultDistance(),
      target: new THREE.Vector3(0, 0, 0),
    };
    if (animated) this.animateTo(to);
    else this.setImmediate(to);
  }

  private setImmediate(to: TweenTarget): void {
    this.yaw = to.yaw;
    this.pitch = to.pitch;
    this.distance = to.distance;
    this.target.copy(to.target);
    this.tween = null;
    this.commit();
  }

  /**
   * Frame `bounds`, or the whole scene when omitted (zoom-to-fit). Empty or
   * missing bounds fall back to the default framed distance at the origin.
   */
  fitView(bounds?: THREE.Box3): void {
    const box = bounds ?? this.getBounds();
    if (!box || box.isEmpty()) {
      this.animateTo({
        yaw: this.yaw,
        pitch: this.pitch,
        distance: this.defaultDistance(),
        target: new THREE.Vector3(0, 0, 0),
      });
      return;
    }
    const sphere = box.getBoundingSphere(new THREE.Sphere());
    const dist = (sphere.radius / Math.sin((this.rig.fovDeg * Math.PI) / 360)) * 1.15;
    this.animateTo({
      yaw: this.yaw,
      pitch: this.pitch,
      distance: clampDistance(dist),
      target: sphere.center.clone(),
    });
  }

  /** Snap to a canonical view given a target→camera direction (ViewCube). */
  snapToViewDirection(dir: THREE.Vector3): void {
    const d = dir.clone().normalize();
    const yaw = yawForDirection(d);
    const pitch = clampPitch(Math.asin(Math.max(-1, Math.min(1, d.z))));
    this.animateTo({ yaw, pitch, distance: this.distance, target: this.target.clone() });
  }

  /** Snapshot of the full view state (for sketch enter → restore on exit). */
  getViewState(): TweenTarget {
    return { yaw: this.yaw, pitch: this.pitch, distance: this.distance, target: this.target.clone() };
  }

  /** Animate (or jump) to a full view state. */
  setView(view: TweenTarget, animated = true): void {
    const to = { ...view, target: view.target.clone() };
    if (animated) this.animateTo(to);
    else this.setImmediate(to);
  }

  /**
   * Look straight at a sketch plane: camera on the +normal side, target at the
   * plane origin. `normal` is the plane normal (target→camera direction).
   */
  viewAlongNormal(normal: THREE.Vector3, target: THREE.Vector3, distance: number, animated = true): void {
    const d = normal.clone().normalize();
    const yaw = yawForDirection(d);
    const pitch = clampPitch(Math.asin(Math.max(-1, Math.min(1, d.z))));
    this.setView({ yaw, pitch, distance, target: target.clone() }, animated);
  }

  private defaultDistance(): number {
    const bounds = this.getBounds();
    if (bounds && !bounds.isEmpty()) {
      const r = bounds.getBoundingSphere(new THREE.Sphere()).radius;
      return clampDistance((r / Math.sin((this.rig.fovDeg * Math.PI) / 360)) * 1.4);
    }
    return 260;
  }

  private animateTo(to: TweenTarget): void {
    this.tween = {
      from: { yaw: this.yaw, pitch: this.pitch, distance: this.distance, target: this.target.clone() },
      to,
      start: now(),
      dur: TWEEN_MS,
    };
    this.commit(); // kick the rAF loop
  }

  /** Advance an active tween. Returns true while an animation is running. */
  update(nowMs: number): boolean {
    if (!this.tween) return false;
    const { from, to, start, dur } = this.tween;
    const t = Math.min(1, (nowMs - start) / dur);
    const e = easeInOutCubic(t);
    this.yaw = shortestAngleLerp(from.yaw, to.yaw, e);
    this.pitch = from.pitch + (to.pitch - from.pitch) * e;
    this.distance = from.distance + (to.distance - from.distance) * e;
    this.target.copy(from.target).lerp(to.target, e);
    this.commit();
    if (t >= 1) this.tween = null;
    return true;
  }

  dispose(): void {
    this.el.removeEventListener("pointerdown", this.onPointerDown);
    this.el.removeEventListener("pointermove", this.onPointerMove);
    this.el.removeEventListener("pointerup", this.onPointerUp);
    this.el.removeEventListener("pointercancel", this.onPointerUp);
    this.el.removeEventListener("wheel", this.onWheel);
    this.el.removeEventListener("contextmenu", this.preventContext);
    this.el.removeEventListener("gesturestart", this.onGesture);
    this.el.removeEventListener("gesturechange", this.onGesture);
    this.el.removeEventListener("gestureend", this.onGesture);
    this.pointers.clear();
    this.lastPointer = null;
    this.nav = navInit();
    this.lastDevice = null;
    this.tween = null;
  }
}

// ---- module-local helpers ----

const MIN_DIST = 0.5;
const MAX_DIST = 50_000;

function clampDistance(d: number): number {
  return Math.max(MIN_DIST, Math.min(MAX_DIST, d));
}

function clampFactor(f: number): number {
  return Math.max(0.2, Math.min(5, f));
}

function now(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}
