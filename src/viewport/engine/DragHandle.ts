/*
 * DragHandle — the value gizmo: a flat, SCREEN-SPACE arrow kept at a constant
 * CSS-pixel size, drawn in front of everything and grabbed to drag a value. The
 * engine raycasts it on pointerdown to decide whether a drag grabs the handle
 * (starts a value drag) or orbits.
 *
 * IT IS A 2D OVERLAY, NOT GEOMETRY. The arrow was a cylinder + cone living in
 * the world: near-edge-on it collapsed to a hairline, and pointing at the camera
 * it collapsed to a dot — the one camera angle where a depth drag is most likely.
 * The silhouette is now flat and `orient()` BILLBOARDS it: it always faces the
 * camera, with its length along the axis's own SCREEN direction. Nothing about
 * the world axis moves; only how the affordance is drawn.
 *
 * THE HALO IS LOAD-BEARING. The fill is `--color-accent`, and so is a selected
 * face (`palette.selected3d()` reads the same token) — a blue arrow on the blue
 * face it operates on. The outset silhouette behind it, in the opposite-tone
 * `--color-overlay-halo`, is what separates them, on any background in either
 * theme.
 *
 * TWO MODES.
 *  - `forward` — one head, pointing along `dir`. The historical shape; every value
 *    tool (OffsetFace) uses it, and extrude uses it as soon as a direction exists.
 *  - `twoWay` — a mirrored second head. The RESTING state of an extrude arm, where
 *    the sign has not been chosen yet: both directions are live, so the glyph says
 *    so rather than implying the +normal side.
 *
 * The pick envelope stays a fat cylinder about the local axis — rotationally
 * symmetric, so the billboard roll cannot change what is grabbable, and it
 * presents the same corridor width whichever way the arrow points on screen. It
 * TRACKS the visible heads: a tail head that renders but cannot be grabbed is a
 * dead affordance; an envelope left permanently symmetric would reach backwards
 * through the prism and the body it grew from — the visible materials are
 * `depthTest: false` and `raycast` has no occlusion test, so that envelope would
 * silently swallow selection clicks over the model.
 */
import * as THREE from "three";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";

const SHAFT_PX = 44; // shaft length in screen pixels, tail → head base
const SHAFT_W_PX = 5; // shaft width (was a 2.2px-radius cylinder — a hairline)
const HEAD_PX = 16; // head length
const HEAD_W_PX = 14; // head width
const HALO_PX = 1.6; // outset of the contrast outline, per side
const HIT_RADIUS_PX = 7; // ≥12px across for a trackpad, wider than the silhouette
/** Below this projected length the axis is pointing at the camera and has no
 *  screen direction left to draw along. */
const MIN_SCREEN_DIR = 1e-3;

/** Which heads the arrow draws — and therefore what it can be grabbed by. */
export type DragHandleMode = "forward" | "twoWay";

export interface DragHandleDeps {
  root: THREE.Object3D; // interactionRoot
  invalidate: () => void;
}

/**
 * The arrow silhouette in the local XY plane, +Y along the axis, tail at the
 * origin. `inflate` grows it on every side for the halo, so the outline keeps a
 * uniform width instead of the uneven edge a uniform scale would give a shape
 * this thin.
 */
function arrowShape(twoWay: boolean, inflate: number): THREE.Shape {
  const sw = SHAFT_W_PX / 2 + inflate;
  const hw = HEAD_W_PX / 2 + inflate;
  const tip = SHAFT_PX + HEAD_PX + inflate;
  const base = SHAFT_PX; // head base stays put; only the outline grows outward
  const shape = new THREE.Shape();
  if (twoWay) {
    // Mirrored head hanging off the tail — same silhouette, pointing back.
    const tailTip = -HEAD_PX - inflate;
    shape.moveTo(0, tailTip);
    shape.lineTo(hw, -HEAD_PX);
    shape.lineTo(sw, -HEAD_PX);
  } else {
    shape.moveTo(-sw, -inflate);
    shape.lineTo(sw, -inflate);
  }
  shape.lineTo(sw, base);
  shape.lineTo(hw, base);
  shape.lineTo(0, tip);
  shape.lineTo(-hw, base);
  shape.lineTo(-sw, base);
  if (twoWay) {
    shape.lineTo(-sw, -HEAD_PX);
    shape.lineTo(-hw, -HEAD_PX);
  }
  shape.closePath();
  return shape;
}

export class DragHandle {
  private readonly group = new THREE.Group();
  private readonly fill: THREE.Mesh;
  private readonly halo: THREE.Mesh;
  private readonly hitCyl: THREE.Mesh; // invisible fat pick target
  private readonly matNormal: THREE.MeshBasicMaterial;
  private readonly matHover: THREE.MeshBasicMaterial;
  private readonly matDestructive: THREE.MeshBasicMaterial;
  private readonly matHalo: THREE.MeshBasicMaterial;
  private readonly _q = new THREE.Quaternion();
  private readonly _dir = new THREE.Vector3();
  private readonly _up = new THREE.Vector3(0, 1, 0);
  private readonly _camDir = new THREE.Vector3();
  private readonly _roll = new THREE.Quaternion();
  private readonly _invCam = new THREE.Quaternion();
  private readonly _z = new THREE.Vector3(0, 0, 1);
  private hovered = false;
  private destructive = false;
  private mode: DragHandleMode = "forward";
  /** World axis the arrow represents — `orient()` needs it every frame, and
   *  `setAxis` is only called when the tool moves it. */
  private readonly axis = new THREE.Vector3(0, 1, 0);
  /** Last screen angle that was well defined, held through the degenerate case
   *  so an axis rotating INTO the camera does not spin the arrow on its way. */
  private screenAngle = 0;

  constructor(private readonly deps: DragHandleDeps) {
    this.group.name = "extrudeHandle";
    this.group.visible = false;
    this.matNormal = new THREE.MeshBasicMaterial({ color: palette.hoverAccent(), depthTest: false, transparent: true, opacity: 0.95, toneMapped: false, side: THREE.DoubleSide });
    // transparent keeps the hover state in the same (transparent) render list
    // as matNormal — an opaque hover material would drop under all fills.
    this.matHover = new THREE.MeshBasicMaterial({ color: palette.selectedEdge(), depthTest: false, transparent: true, toneMapped: false, side: THREE.DoubleSide });
    // The arrow is the primary affordance once the boolean segments live behind
    // the chip's overflow, so it carries the same destructive signal the prism does.
    this.matDestructive = new THREE.MeshBasicMaterial({ color: palette.destructive(), depthTest: false, transparent: true, opacity: 0.95, toneMapped: false, side: THREE.DoubleSide });
    this.matHalo = new THREE.MeshBasicMaterial({ color: palette.overlayHalo(), depthTest: false, transparent: true, opacity: 0.9, toneMapped: false, side: THREE.DoubleSide });

    // Geometry authored in the local XY plane in "px units" (scaled per frame).
    this.fill = new THREE.Mesh(new THREE.ShapeGeometry(arrowShape(false, 0)), this.matNormal);
    this.halo = new THREE.Mesh(new THREE.ShapeGeometry(arrowShape(false, HALO_PX)), this.matHalo);
    // Both are depthTest:false and coplanar, so the painter's ladder cannot
    // separate them; pushing the halo a hair AWAY from the camera (the billboard
    // makes local -Z exactly that) makes the transparent pass's back-to-front
    // sort do it deterministically instead.
    this.halo.position.z = -0.5;

    this.hitCyl = new THREE.Mesh(
      new THREE.CylinderGeometry(HIT_RADIUS_PX, HIT_RADIUS_PX, SHAFT_PX + HEAD_PX, 8),
      new THREE.MeshBasicMaterial({ visible: false, toneMapped: false }),
    );
    this.hitCyl.position.y = (SHAFT_PX + HEAD_PX) / 2;
    this.hitCyl.userData.extrudeHandle = true;

    this.group.add(this.halo, this.fill, this.hitCyl);
    this.group.renderOrder = RENDER_ORDER.DRAG_HANDLE;
    this.fill.renderOrder = RENDER_ORDER.DRAG_HANDLE;
    this.halo.renderOrder = RENDER_ORDER.DRAG_HANDLE;
    deps.root.add(this.group);
  }

  /** Where the handle sits, for a depth-accurate screen scale (U5). */
  worldAnchor(): THREE.Vector3 {
    return this.group.position.clone();
  }

  /**
   * Position the handle at `origin`, pointing along `dir`, drawing `mode`'s heads.
   *
   * A ZERO-LENGTH `dir` keeps the current orientation. `Vector3.normalize()` maps
   * it to (0,0,0) and `setFromUnitVectors` then takes its degenerate branch, which
   * produces an arbitrary 180° flip rather than a NaN — silent, and camera-
   * dependent. The extrude arm reaches exactly that case at depth 0.
   */
  setAxis(origin: THREE.Vector3, dir: THREE.Vector3, mode: DragHandleMode = "forward"): void {
    this.group.position.copy(origin);
    this._dir.copy(dir);
    if (this._dir.lengthSq() > 1e-12) {
      this._dir.normalize();
      this.axis.copy(this._dir);
      this.group.quaternion.copy(this._q.setFromUnitVectors(this._up, this._dir));
    }
    this.setMode(mode);
    this.deps.invalidate();
  }

  /** Position/orient only, keeping the current mode (the historical entry point). */
  setAnchor(origin: THREE.Vector3, dir: THREE.Vector3): void {
    this.setAxis(origin, dir, this.mode);
  }

  /**
   * Billboard the silhouette: face `camera`, length along the axis's SCREEN
   * direction. Called every frame by the engine, before the render.
   *
   * The screen direction is the axis in CAMERA space — its x/y are exactly
   * screen right/up for both projections, so no unprojection is needed and an
   * orthographic camera behaves identically. When those two components vanish
   * the axis points at the viewer and has no screen direction at all: the last
   * good angle is held, so the arrow stays a full-length, grabbable arrow
   * instead of collapsing to the dot the old cone became.
   */
  orient(camera: THREE.Camera): void {
    this._invCam.copy(camera.quaternion).invert();
    this._camDir.copy(this.axis).applyQuaternion(this._invCam);
    if (Math.hypot(this._camDir.x, this._camDir.y) > MIN_SCREEN_DIR) {
      this.screenAngle = Math.atan2(this._camDir.y, this._camDir.x) - Math.PI / 2;
    }
    this._roll.setFromAxisAngle(this._z, this.screenAngle);
    this.group.quaternion.copy(camera.quaternion).multiply(this._roll);
  }

  private setMode(mode: DragHandleMode): void {
    if (mode === this.mode) return;
    this.mode = mode;
    const twoWay = mode === "twoWay";
    this.fill.geometry.dispose();
    this.fill.geometry = new THREE.ShapeGeometry(arrowShape(twoWay, 0));
    this.halo.geometry.dispose();
    this.halo.geometry = new THREE.ShapeGeometry(arrowShape(twoWay, HALO_PX));
    // The envelope follows the heads: symmetric ONLY while both heads are drawn.
    const length = twoWay ? SHAFT_PX + 2 * HEAD_PX : SHAFT_PX + HEAD_PX;
    this.hitCyl.geometry.dispose();
    this.hitCyl.geometry = new THREE.CylinderGeometry(HIT_RADIUS_PX, HIT_RADIUS_PX, length, 8);
    this.hitCyl.position.y = twoWay ? SHAFT_PX / 2 : (SHAFT_PX + HEAD_PX) / 2;
  }

  /** Keep the handle a constant screen size: `worldPerPx` world units per pixel. */
  setScale(worldPerPx: number): void {
    this.group.scale.setScalar(Math.max(worldPerPx, 1e-6));
    this.deps.invalidate();
  }

  /**
   * How far the arrow reaches from its anchor, in CSS pixels — what a chip must
   * clear. The far head is the same distance in both modes; `twoWay` only adds a
   * head on the near side, which is inside that radius.
   */
  reachPx(): number {
    return SHAFT_PX + HEAD_PX;
  }

  /**
   * Theme change: re-read every material. `setHover`/`setDestructive` SWAP between
   * them rather than recoloring, so all three must stay current regardless of the
   * live state — and the halo is the one that INVERTS between themes, so leaving
   * it out would strand a white outline on a dark canvas.
   */
  refreshColors(): void {
    this.matNormal.color.copy(palette.hoverAccent());
    this.matHover.color.copy(palette.selectedEdge());
    this.matDestructive.color.copy(palette.destructive());
    this.matHalo.color.copy(palette.overlayHalo());
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
    this.fill.material = this.hovered
      ? this.matHover
      : this.destructive
        ? this.matDestructive
        : this.matNormal;
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
    this.fill.geometry.dispose();
    this.halo.geometry.dispose();
    this.hitCyl.geometry.dispose();
    (this.hitCyl.material as THREE.Material).dispose();
    this.matNormal.dispose();
    this.matHover.dispose();
    this.matDestructive.dispose();
    this.matHalo.dispose();
    this.deps.root.remove(this.group);
  }
}
