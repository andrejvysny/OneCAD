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
 * THE PICK CORRIDOR is an invisible flat 30 × 40 CSS-pixel rectangle in the
 * glyph's own plane, centred on the attachment, length along the glyph axis. The
 * group is billboarded, so the rectangle is camera-parallel: every point of it
 * sits at the same view depth, and the per-anchor `worldPerPixel` scale makes it
 * project to exactly 30 × 40 px, rolled with the glyph. A 3D pick volume does
 * not: the camera-facing cylinder it replaces had its radius along the view
 * direction, and perspective parallax off the optical axis stretched its
 * corridor past the keep-out box (measured 32 × 48 px at FOV 76), so the chip
 * covered part of the grab area. `corridorHalfExtentsPx()` is that rectangle's
 * rolled screen box. DOM chrome wins before this raycast is consulted.
 */
import * as THREE from "three";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";
import {
  classifyHandleMapping,
  projectRejectedAxis,
  type AxisProjection,
  type HandleMapping,
  type HandleStrategy,
} from "@/tools/preview/handleProjection";

const GLYPH_HALF_PX = 15; // compact 30px glyph, centered on attachment
const SHAFT_END_PX = 6; // shaft endpoint before either head
const SHAFT_W_PX = 4; // visible 2px stroke
const HEAD_W_PX = 14; // head width
const HALO_PX = 1.6; // outset of the contrast outline, per side
const HIT_WIDTH_PX = 30; // corridor width, across the glyph axis
const HIT_LENGTH_PX = 40; // corridor length, along the glyph axis
/** Relative change below which `setScale` is a no-op (and schedules no frame). */
const SCALE_EPS = 1e-9;
/** Up the screen, CSS px (+Y down): the proxy's increasing direction. */
const PROXY_UP: readonly [number, number] = [0, -1];

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
  private readonly hitPlane: THREE.Mesh; // invisible flat pick target
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
  /** World edge tangent whose projection is REJECTED from the axis (edge ops),
   *  or null. Replaced by every `setAxis`. */
  private tangent: THREE.Vector3 | null = null;
  /** Strategy frozen at grab, or null. Survives `setAxis` — the controller
   *  re-issues that every drag frame — and is cleared only by an explicit
   *  `freezeStrategy(null)` or `reset()`. */
  private frozenStrategy: HandleStrategy | null = null;
  /** The mapping `orient` last drew; null before the first `orient`. */
  private lastMapping: HandleMapping | null = null;
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

    // Local XY like the glyph, length along +Y; DoubleSide so the pick never
    // depends on which way the billboard's roll left the face winding.
    this.hitPlane = new THREE.Mesh(
      new THREE.PlaneGeometry(HIT_WIDTH_PX, HIT_LENGTH_PX),
      new THREE.MeshBasicMaterial({ visible: false, toneMapped: false, side: THREE.DoubleSide }),
    );
    this.hitPlane.userData.extrudeHandle = true;

    this.group.add(this.halo, this.fill, this.hitPlane);
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
   *
   * `tangent`, for an edge op, is the world edge tangent at `origin`: its
   * projection is rejected from the axis's, so the glyph and the gesture share
   * `handleProjection.projectRejectedAxis`. Omitting it clears any previous one.
   * A frozen strategy is deliberately NOT cleared here.
   */
  setAxis(
    origin: THREE.Vector3,
    dir: THREE.Vector3,
    mode: DragHandleMode = "forward",
    tangent: THREE.Vector3 | null = null,
  ): void {
    this.screenProxyDirection = null;
    this.tangent = tangent ? tangent.clone() : null;
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

  /**
   * Position/orient only, keeping the current mode (the historical entry point).
   * The tangent is CLEARED, not kept: this handle is shared, the only caller is
   * the extrude arm (no tangent), and keeping one would let a fillet's edge
   * tangent leak into an extrude arrow armed without a `reset()`.
   */
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
   * The mapping this handle offers under `camera` RIGHT NOW, ignoring any freeze:
   * the axis (tangent-rejected when a tangent is set) classified by
   * `handleProjection.classifyHandleMapping`, or the explicit screen proxy when
   * one is set. Changes no handle state. `worldPerPx` must be measured at this
   * handle's anchor (`screenScale.worldPerPixel(camera, worldAnchor(), height)`).
   */
  computeMapping(
    camera: THREE.Camera,
    viewportWidth: number,
    viewportHeight: number,
    worldPerPx: number,
  ): HandleMapping {
    if (this.screenProxyDirection !== null) return this.proxyMapping(worldPerPx);
    return classifyHandleMapping(this.projectedAxis(camera, viewportWidth, viewportHeight, worldPerPx), worldPerPx);
  }

  /**
   * Freeze the strategy `orient` draws — called at grab so glyph, pick and
   * gesture cannot swap mapping under the user mid-drag. `null` unfreezes.
   * An explicit screen proxy (`setScreenProxy`) always draws as a proxy.
   */
  freezeStrategy(strategy: HandleStrategy | null): void {
    this.frozenStrategy = strategy;
  }

  /** The mapping `orient` last drew, or null before the first `orient`. */
  mapping(): HandleMapping | null {
    return this.lastMapping;
  }

  /**
   * Billboard the silhouette: face `camera`, length along the mapping's SCREEN
   * direction. Called every frame by the engine, before the render, with the
   * same per-anchor `worldPerPx` it just passed to `setScale`.
   *
   * The screen direction is the TRUE projected derivative of the world axis at
   * this handle's own anchor (`handleProjection.projectRejectedAxis`), not the
   * axis rotated into camera space. Those two agree only under an orthographic
   * camera: perspective divides by `w`, so a world axis projects to a different
   * screen direction depending on where in the frustum it is anchored.
   *
   * UNFROZEN, the glyph draws `computeMapping`: an axis too close to the view
   * ray to drag draws the vertical proxy rather than a stale angle, so an
   * arrow never looks like it drags along a direction whose drag maps to zero.
   *
   * FROZEN `axis` keeps drawing the axis even ill-conditioned; when its
   * projection refuses outright (end-on, anchor behind the camera plane) the
   * last good angle is held, so the arrow stays a full-length, grabbable arrow
   * and never spins. FROZEN `screenProxy` draws the proxy.
   */
  orient(camera: THREE.Camera, viewportWidth: number, viewportHeight: number, worldPerPx: number): void {
    const mapping = this.frozenStrategy === "axis" && this.screenProxyDirection === null
      ? this.frozenAxisMapping(camera, viewportWidth, viewportHeight, worldPerPx)
      : this.frozenStrategy === "screenProxy"
        ? this.proxyMapping(worldPerPx)
        : this.computeMapping(camera, viewportWidth, viewportHeight, worldPerPx);
    // `direction` is CSS-pixel space (+Y DOWN); the roll below is applied in
    // the camera's own frame (+Y UP), so the vertical component flips once,
    // here, and nowhere else.
    const [dx, dy] = mapping.direction;
    this.screenAngle = Math.atan2(-dy, dx) - Math.PI / 2;
    this.lastMapping = mapping;
    this._roll.setFromAxisAngle(this._z, this.screenAngle);
    this.group.quaternion.copy(camera.quaternion).multiply(this._roll);
  }

  /** Exact (tangent-rejected) projection of the axis at the anchor, or null. */
  private projectedAxis(
    camera: THREE.Camera,
    viewportWidth: number,
    viewportHeight: number,
    worldPerPx: number,
  ): AxisProjection | null {
    this._viewProj.multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
    const anchor = this.group.position;
    const tangent = this.tangent;
    return projectRejectedAxis(
      this._viewProj.elements,
      [anchor.x, anchor.y, anchor.z],
      [this.axis.x, this.axis.y, this.axis.z],
      tangent ? [tangent.x, tangent.y, tangent.z] : null,
      viewportWidth,
      viewportHeight,
      worldPerPx,
    );
  }

  private proxyMapping(worldPerPx: number): HandleMapping {
    return {
      strategy: "screenProxy",
      direction: this.screenProxyDirection ?? PROXY_UP,
      pxPerWorld: null,
      worldPerPx: Number.isFinite(worldPerPx) && worldPerPx > 0 ? worldPerPx : 0,
    };
  }

  /**
   * A frozen axis: the projection whatever its conditioning; on refusal the
   * held angle with no gain (`pxPerWorld: null`). The held direction inverts
   * `orient`'s roll: glyph axis (dx, dy) = (−sin a, −cos a).
   */
  private frozenAxisMapping(
    camera: THREE.Camera,
    viewportWidth: number,
    viewportHeight: number,
    worldPerPx: number,
  ): HandleMapping {
    const scale = Number.isFinite(worldPerPx) && worldPerPx > 0 ? worldPerPx : 0;
    const projected = this.projectedAxis(camera, viewportWidth, viewportHeight, worldPerPx);
    if (projected) {
      return { strategy: "axis", direction: projected.direction, pxPerWorld: projected.pxPerWorld, worldPerPx: scale };
    }
    return {
      strategy: "axis",
      direction: [-Math.sin(this.screenAngle), -Math.cos(this.screenAngle)],
      pxPerWorld: null,
      worldPerPx: scale,
    };
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

  /**
   * Keep the handle a constant screen size: `worldPerPx` world units per pixel.
   * Invalidates only on a real change — the engine calls this inside every
   * rendered frame, and an unconditional invalidate there schedules a wasted
   * frame after each one, so idle never reaches zero frames.
   */
  setScale(worldPerPx: number): void {
    const next = Math.max(worldPerPx, 1e-6);
    const current = this.group.scale.x;
    if (Math.abs(next - current) <= SCALE_EPS * Math.max(Math.abs(next), Math.abs(current))) return;
    this.group.scale.setScalar(next);
    this.deps.invalidate();
  }

  /**
   * Half extents, in CSS pixels, of the screen box that holds the pick corridor
   * at its current roll: the 30 × 40 rectangle's axis-aligned bounds. The glyph
   * axis runs along screen (dx, dy) = (−sin a, −cos a) for roll `a` (see
   * `orient`), so |dx| = |sin a| and |dy| = |cos a|.
   */
  corridorHalfExtentsPx(): { x: number; y: number } {
    const along = HIT_LENGTH_PX / 2;
    const across = HIT_WIDTH_PX / 2;
    const ax = Math.abs(Math.sin(this.screenAngle));
    const ay = Math.abs(Math.cos(this.screenAngle));
    return { x: ax * along + ay * across, y: ay * along + ax * across };
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
    this.frozenStrategy = null;
    this.tangent = null;
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

  /** True when `raycaster` hits the handle's pick corridor. */
  raycast(raycaster: THREE.Raycaster): boolean {
    if (!this.group.visible) return false;
    return raycaster.intersectObject(this.hitPlane, false).length > 0;
  }

  dispose(): void {
    this.fill.geometry.dispose();
    this.halo.geometry.dispose();
    this.hitPlane.geometry.dispose();
    (this.hitPlane.material as THREE.Material).dispose();
    this.matNormal.dispose();
    this.matHover.dispose();
    this.matDestructive.dispose();
    this.matHalo.dispose();
    this.deps.root.remove(this.group);
  }
}
