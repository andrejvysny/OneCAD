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
 *  - `forward` — legacy one-head compatibility for value tools still migrating.
 *  - `twoWay` — centered bidirectional scalar/linear control. Extrude keeps it
 *    while armed and dragged: travel direction does not change its axis.
 *
 * The pick envelope is centered with compact glyph: 40 × 30 CSS-pixel corridor.
 * It remains a camera-facing cylinder, so billboard roll cannot change pickability.
 * DOM chrome wins before this raycast is consulted.
 */
import * as THREE from "three";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";
import { projectAxis } from "@/tools/preview/handleProjection";

const GLYPH_HALF_PX = 15; // compact 30px glyph, centered on attachment
const SHAFT_END_PX = 6; // shaft endpoint before either head
const SHAFT_W_PX = 4; // visible 2px stroke
const HEAD_W_PX = 14; // head width
const HALO_PX = 1.6; // outset of the contrast outline, per side
const HIT_RADIUS_PX = 15; // 30px corridor width
const HIT_LENGTH_PX = 40; // 40px corridor length

/** Which heads the arrow draws — and therefore what it can be grabbed by. */
export type DragHandleMode = "forward" | "twoWay";

export interface DragHandleDeps {
  root: THREE.Object3D; // interactionRoot
  invalidate: () => void;
}

/**
 * The arrow silhouette in the local XY plane, +Y along the axis, tail at the
 * origin. `inflate` grows it on every side for halo, so outline keeps a
 * uniform width instead of the uneven edge a uniform scale would give a shape
 * this thin.
 */
function arrowShape(twoWay: boolean, inflate: number): THREE.Shape {
  const sw = SHAFT_W_PX / 2 + inflate;
  const hw = HEAD_W_PX / 2 + inflate;
  const tip = GLYPH_HALF_PX + inflate;
  const base = SHAFT_END_PX;
  const shape = new THREE.Shape();
  if (twoWay) {
    shape.moveTo(0, -tip);
    shape.lineTo(hw, -base);
    shape.lineTo(sw, -base);
  } else {
    shape.moveTo(-sw, -tip);
    shape.lineTo(sw, -tip);
  }
  shape.lineTo(sw, base);
  shape.lineTo(hw, base);
  shape.lineTo(0, tip);
  shape.lineTo(-hw, base);
  shape.lineTo(-sw, base);
  if (twoWay) {
    shape.lineTo(-sw, -base);
    shape.lineTo(-hw, -base);
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
  private readonly _roll = new THREE.Quaternion();
  private readonly _z = new THREE.Vector3(0, 0, 1);
  /** Reused rather than rebuilt each frame: `orient` runs on the render path, so
   *  the 16-float matrix is worth holding. (`projectAxis` still returns a small
   *  result object, the same young-gen cost `worldAnchor()` beside it already pays.) */
  private readonly _viewProj = new THREE.Matrix4();
  private hovered = false;
  private destructive = false;
  private mode: DragHandleMode = "forward";
  /** World axis the arrow represents — `orient()` needs it every frame, and
   *  `setAxis` is only called when the tool moves it. */
  private readonly axis = new THREE.Vector3(0, 1, 0);
  /** Screen-space proxy direction (`+Y` down), used when source geometry has
   * no trustworthy world axis. Cleared by every geometric axis update. */
  private screenProxyDirection: readonly [number, number] | null = null;
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
      new THREE.CylinderGeometry(HIT_RADIUS_PX, HIT_RADIUS_PX, HIT_LENGTH_PX, 8),
      new THREE.MeshBasicMaterial({ visible: false, toneMapped: false }),
    );
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
    this.screenProxyDirection = null;
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

  /** Show an explicitly screen-space scalar proxy at a truthful attachment. */
  setScreenProxy(origin: THREE.Vector3, direction: readonly [number, number] = [0, -1]): void {
    const length = Math.hypot(direction[0], direction[1]);
    if (!(length > 1e-6) || !Number.isFinite(length)) return;
    this.group.position.copy(origin);
    this.screenProxyDirection = [direction[0] / length, direction[1] / length];
    this.setMode("twoWay");
    this.deps.invalidate();
  }

  /**
   * Billboard the silhouette: face `camera`, length along the axis's SCREEN
   * direction. Called every frame by the engine, before the render.
   *
   * The screen direction is the TRUE projected derivative of the world axis at
   * this handle's own anchor (`handleProjection.projectAxis`), not the axis
   * rotated into camera space. Those two agree only under an orthographic
   * camera: perspective divides by `w`, so a world axis projects to a different
   * screen direction depending on where in the frustum it is anchored. The old
   * camera-space shortcut therefore drew the arrow along one direction while
   * the drag mapped along another, and the gap widened the further the anchor
   * sat from the optical axis — at FOV 76 that is most of the viewport.
   *
   * When the projection refuses — the axis points at the viewer, or the anchor
   * is behind the camera plane — there is no screen direction to draw and the
   * last good angle is held, so the arrow stays a full-length, grabbable arrow
   * instead of collapsing to the dot the old cone became.
   */
  orient(camera: THREE.Camera, viewportWidth: number, viewportHeight: number): void {
    this._viewProj.multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
    const anchor = this.group.position;
    const projected = this.screenProxyDirection === null
      ? projectAxis(
        this._viewProj.elements,
        [anchor.x, anchor.y, anchor.z],
        [this.axis.x, this.axis.y, this.axis.z],
        viewportWidth,
        viewportHeight,
      )
      : null;
    const direction = this.screenProxyDirection ?? projected?.direction;
    if (direction) {
      // `direction` is CSS-pixel space (+Y DOWN); the roll below is applied in
      // the camera's own frame (+Y UP), so the vertical component flips once,
      // here, and nowhere else.
      const [dx, dy] = direction;
      this.screenAngle = Math.atan2(-dy, dx) - Math.PI / 2;
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
  }

  /** Keep the handle a constant screen size: `worldPerPx` world units per pixel. */
  setScale(worldPerPx: number): void {
    this.group.scale.setScalar(Math.max(worldPerPx, 1e-6));
    this.deps.invalidate();
  }

  /**
   * Furthest selectable corridor edge from attachment, in CSS pixels.
   */
  reachPx(): number {
    return HIT_LENGTH_PX / 2;
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
