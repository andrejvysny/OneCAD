/*
 * DragHandle — the value gizmo: a cylinder shaft with a cone arrowhead, kept at a
 * CONSTANT screen size (scaled by world-per-pixel each frame). Hover brightens it.
 * The engine raycasts it on pointerdown to decide whether a drag grabs the handle
 * (starts a value drag) or orbits.
 *
 * TWO MODES.
 *  - `forward` — one head, pointing along `dir`. The historical shape; every value
 *    tool (OffsetFace) uses it, and extrude uses it as soon as a direction exists.
 *  - `twoWay` — a mirrored second head. The RESTING state of an extrude arm, where
 *    the sign has not been chosen yet: both directions are live, so the glyph says
 *    so rather than implying the +normal side.
 *
 * The pick envelope TRACKS the visible geometry. A tail cone that renders but
 * cannot be grabbed is a dead affordance; an envelope left permanently symmetric
 * would reach backwards through the prism and the body it grew from — the visible
 * materials are `depthTest: false` and `raycast` has no occlusion test, so that
 * envelope would silently swallow selection clicks over the model.
 */
import * as THREE from "three";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";

const SHAFT_PX = 46; // handle length in screen pixels
const SHAFT_RADIUS_PX = 2.2;
const CONE_PX = 14;
const CONE_RADIUS_PX = 6;
const HIT_PAD = 1.8; // enlarge the pickable envelope for an easy grab

/** Which heads the arrow draws — and therefore what it can be grabbed by. */
export type DragHandleMode = "forward" | "twoWay";

export interface DragHandleDeps {
  root: THREE.Object3D; // interactionRoot
  invalidate: () => void;
}

export class DragHandle {
  private readonly group = new THREE.Group();
  private readonly shaft: THREE.Mesh;
  private readonly cone: THREE.Mesh;
  private readonly coneTail: THREE.Mesh;
  private readonly hitCyl: THREE.Mesh; // invisible fat pick target
  private readonly matNormal: THREE.MeshBasicMaterial;
  private readonly matHover: THREE.MeshBasicMaterial;
  private readonly matDestructive: THREE.MeshBasicMaterial;
  private readonly _q = new THREE.Quaternion();
  private readonly _dir = new THREE.Vector3();
  private readonly _up = new THREE.Vector3(0, 1, 0);
  private hovered = false;
  private destructive = false;
  private mode: DragHandleMode = "forward";

  constructor(private readonly deps: DragHandleDeps) {
    this.group.name = "extrudeHandle";
    this.group.visible = false;
    this.matNormal = new THREE.MeshBasicMaterial({ color: palette.hoverAccent(), depthTest: false, transparent: true, opacity: 0.9, toneMapped: false });
    // transparent keeps the hover state in the same (transparent) render list
    // as matNormal — an opaque hover material would drop under all fills.
    this.matHover = new THREE.MeshBasicMaterial({ color: palette.selectedEdge(), depthTest: false, transparent: true, toneMapped: false });
    // The arrow is the primary affordance once the boolean segments live behind
    // the chip's overflow, so it carries the same destructive signal the prism does.
    this.matDestructive = new THREE.MeshBasicMaterial({ color: palette.destructive(), depthTest: false, transparent: true, opacity: 0.9, toneMapped: false });

    // Geometry authored pointing +Y, sized in "px units" (scaled per frame).
    this.shaft = new THREE.Mesh(
      new THREE.CylinderGeometry(SHAFT_RADIUS_PX, SHAFT_RADIUS_PX, SHAFT_PX, 12),
      this.matNormal,
    );
    this.shaft.position.y = SHAFT_PX / 2;
    this.cone = new THREE.Mesh(new THREE.ConeGeometry(CONE_RADIUS_PX, CONE_PX, 16), this.matNormal);
    this.cone.position.y = SHAFT_PX + CONE_PX / 2;
    // The mirrored head: same cone, flipped, hanging off the origin end.
    this.coneTail = new THREE.Mesh(this.cone.geometry, this.matNormal);
    this.coneTail.position.y = -CONE_PX / 2;
    this.coneTail.rotation.x = Math.PI;
    this.coneTail.visible = false;
    this.hitCyl = new THREE.Mesh(
      new THREE.CylinderGeometry(CONE_RADIUS_PX * HIT_PAD, CONE_RADIUS_PX * HIT_PAD, SHAFT_PX + CONE_PX, 8),
      new THREE.MeshBasicMaterial({ visible: false, toneMapped: false }),
    );
    this.hitCyl.position.y = (SHAFT_PX + CONE_PX) / 2;
    this.hitCyl.userData.extrudeHandle = true;

    this.group.add(this.shaft, this.cone, this.coneTail, this.hitCyl);
    this.group.renderOrder = RENDER_ORDER.DRAG_HANDLE;
    this.shaft.renderOrder = RENDER_ORDER.DRAG_HANDLE;
    this.cone.renderOrder = RENDER_ORDER.DRAG_HANDLE;
    this.coneTail.renderOrder = RENDER_ORDER.DRAG_HANDLE;
    deps.root.add(this.group);
  }

  /**
   * Position the handle at `origin`, pointing along `dir`, drawing `mode`'s heads.
   *
   * A ZERO-LENGTH `dir` keeps the current orientation. `Vector3.normalize()` maps
   * it to (0,0,0) and `setFromUnitVectors` then takes its degenerate branch, which
   * produces an arbitrary 180° flip rather than a NaN — silent, and camera-
   * dependent. The extrude arm reaches exactly that case at depth 0.
   */
  /** Where the handle sits, for a depth-accurate screen scale (U5). */
  worldAnchor(): THREE.Vector3 {
    return this.group.position.clone();
  }

  setAxis(origin: THREE.Vector3, dir: THREE.Vector3, mode: DragHandleMode = "forward"): void {
    this.group.position.copy(origin);
    this._dir.copy(dir);
    if (this._dir.lengthSq() > 1e-12) {
      this._dir.normalize();
      this.group.quaternion.copy(this._q.setFromUnitVectors(this._up, this._dir));
    }
    this.setMode(mode);
    this.deps.invalidate();
  }

  /** Position/orient only, keeping the current mode (the historical entry point). */
  setAnchor(origin: THREE.Vector3, dir: THREE.Vector3): void {
    this.setAxis(origin, dir, this.mode);
  }

  private setMode(mode: DragHandleMode): void {
    if (mode === this.mode) return;
    this.mode = mode;
    const twoWay = mode === "twoWay";
    this.coneTail.visible = twoWay;
    // The envelope follows the heads: symmetric ONLY while both heads are drawn.
    const length = twoWay ? SHAFT_PX + 2 * CONE_PX : SHAFT_PX + CONE_PX;
    this.hitCyl.geometry.dispose();
    this.hitCyl.geometry = new THREE.CylinderGeometry(
      CONE_RADIUS_PX * HIT_PAD,
      CONE_RADIUS_PX * HIT_PAD,
      length,
      8,
    );
    this.hitCyl.position.y = twoWay ? SHAFT_PX / 2 : (SHAFT_PX + CONE_PX) / 2;
  }

  /** Keep the handle a constant screen size: `worldPerPx` world units per pixel. */
  setScale(worldPerPx: number): void {
    this.group.scale.setScalar(Math.max(worldPerPx, 1e-6));
    this.deps.invalidate();
  }

  /**
   * Theme change: re-read every material. `setHover`/`setDestructive` SWAP between
   * them rather than recoloring, so all three must stay current regardless of the
   * live state.
   */
  refreshColors(): void {
    this.matNormal.color.copy(palette.hoverAccent());
    this.matHover.color.copy(palette.selectedEdge());
    this.matDestructive.color.copy(palette.destructive());
  }

  setHover(hovered: boolean): void {
    if (hovered === this.hovered) return;
    this.hovered = hovered;
    this.applyMaterial();
    this.deps.invalidate();
  }

  /** Mirror the preview's destructive tint (the drag is resolving to a Cut). */
  setDestructive(destructive: boolean): void {
    if (destructive === this.destructive) return;
    this.destructive = destructive;
    this.applyMaterial();
    this.deps.invalidate();
  }

  private applyMaterial(): void {
    // Hover wins over the tint: it is the transient state the pointer is asking about.
    const mat = this.hovered ? this.matHover : this.destructive ? this.matDestructive : this.matNormal;
    this.shaft.material = mat;
    this.cone.material = mat;
    this.coneTail.material = mat;
  }

  setVisible(visible: boolean): void {
    this.group.visible = visible;
    this.deps.invalidate();
  }

  /** Drop the extrude-specific state so a plain value tool inherits a clean arrow. */
  reset(): void {
    this.setMode("forward");
    this.setDestructive(false);
    this.setHover(false);
  }

  /** True while the arrow is on screen (the engine's introspection probe). */
  get visible(): boolean {
    return this.group.visible;
  }

  /** Which heads are currently drawn (introspection probe). */
  get axisMode(): DragHandleMode {
    return this.mode;
  }

  /** True when `raycaster` hits the handle's (fat) pick envelope. */
  raycast(raycaster: THREE.Raycaster): boolean {
    if (!this.group.visible) return false;
    return raycaster.intersectObject(this.hitCyl, false).length > 0;
  }

  dispose(): void {
    this.shaft.geometry.dispose();
    this.cone.geometry.dispose(); // shared with coneTail
    this.hitCyl.geometry.dispose();
    (this.hitCyl.material as THREE.Material).dispose();
    this.matNormal.dispose();
    this.matHover.dispose();
    this.matDestructive.dispose();
    this.deps.root.remove(this.group);
  }
}
