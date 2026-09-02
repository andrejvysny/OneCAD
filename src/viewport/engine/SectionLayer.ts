/*
 * SectionLayer — the capped section ("cut-away") view.
 *
 * Two halves, and only the first is this file's own geometry:
 *
 * 1. THE CLIP is a single `THREE.Plane` handed to the BODY materials
 *    (`BodyMaterialLibrary.setClippingPlanes`) by the two owners ViewportRoot
 *    drives. This layer OWNS that plane instance and mutates it in place, which
 *    is what makes dragging the offset slider a per-frame repaint with zero
 *    material writes. `renderer.localClippingEnabled` (renderer.ts) must be on
 *    or three skips material-local planes entirely, silently.
 *
 * 2. THE CAP is what stops the cut reading as a hollow shell. Per visible body
 *    face mesh a pair of stencil-only meshes SHARING that mesh's geometry
 *    (no copies, no GPU buffers of our own) writes the cross-section into the
 *    stencil buffer — back faces increment, front faces decrement, so whatever
 *    is left non-zero is exactly "inside solid, in front of the plane" — and one
 *    full-plane quad then paints there and resets the stencil to 0. This is the
 *    standard three.js capped-clipping construction (webgl_clipping_stencil),
 *    with ONE cap for the union of every body rather than one per body.
 *
 * Ordering is the whole game and lives in `renderOrder.ts`: stencil pair → cap →
 * bodies, all inside the ONE `renderer.render()` call. renderOrder sorts only
 * WITHIN a render list, so the moment sketch mode dims the bodies (transparent:
 * true ⇒ transparent list ⇒ drawn after the cap) no tier can order them
 * correctly. The cap is therefore HIDDEN while a sketch session is open; the
 * clip stays, which is what the user asked for.
 *
 * Body meshes are reconciled from `bodiesRoot` every frame the section is on
 * (`update`), not hooked at the point bodies register: visibility, the tree eye,
 * isolation and the Layers filter all change what should be capped without ever
 * adding or removing a body, and `traverseVisible` answers all four at once.
 * The walk is O(bodies) and only runs while the section is enabled.
 */
import * as THREE from "three";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";
import type { SectionPlaneId, SectionState } from "@/stores/viewportStore";

/**
 * The world axis each named plane cuts ALONG — i.e. its normal. Shared,
 * read-only instances: copy before you touch one.
 */
const PLANE_AXIS: Record<SectionPlaneId, THREE.Vector3> = {
  XY: new THREE.Vector3(0, 0, 1),
  XZ: new THREE.Vector3(0, 1, 0),
  YZ: new THREE.Vector3(1, 0, 0),
};

/** `PlaneGeometry`'s own normal — the rotation the cap quad is measured from. */
const UNIT_Z = new THREE.Vector3(0, 0, 1);

/** Cap quad size as a multiple of the scene's bounding-box diagonal. */
const CAP_OVERSIZE = 2;

/** Reused scratch for the bounds measurement — `positionCap` allocates nothing. */
const SCRATCH = new THREE.Vector3();

/**
 * Write `state` into `target` as a three clipping plane.
 *
 * three DISCARDS the fragments whose signed distance to the plane is negative,
 * so the kept half is `normal · p + constant >= 0`. Unflipped, the kept half is
 * everything BELOW the offset along the axis (`axis · p <= offsetMm`) — cutting
 * XY at 0 takes the top off and you look down into the part, which is what a
 * section view is for. `flip` keeps the other half instead.
 */
export function sectionPlane(
  plane: SectionPlaneId,
  offsetMm: number,
  flip: boolean,
  target = new THREE.Plane(),
): THREE.Plane {
  const sign = flip ? 1 : -1;
  target.normal.copy(PLANE_AXIS[plane]).multiplyScalar(sign);
  target.constant = -sign * offsetMm;
  return target;
}

/**
 * The offset range the slider spans: the scene's extent along the plane's axis.
 * `null` when there is nothing in the scene to cut — the control has no
 * meaningful range then, and inventing one would let the user park the plane
 * where it does nothing.
 */
export function sectionOffsetRange(
  bounds: THREE.Box3 | null,
  plane: SectionPlaneId,
): { min: number; max: number } | null {
  if (!bounds || bounds.isEmpty()) return null;
  const axis = PLANE_AXIS[plane];
  const min = bounds.min.dot(axis);
  const max = bounds.max.dot(axis);
  return { min, max };
}

/** The two stencil-only meshes that mark one body's cross-section. */
interface StencilPair {
  back: THREE.Mesh;
  front: THREE.Mesh;
}

export interface SectionLayerDeps {
  /** Scene root to attach to (the layer never transforms its own group). */
  root: THREE.Object3D;
  /** Body scene root — reconciled every enabled frame (see file header). */
  getBodiesRoot: () => THREE.Object3D;
  /** Scene extent, for sizing the cap quad. */
  getBounds: () => THREE.Box3 | null;
  invalidate: () => void;
}

export class SectionLayer {
  readonly object3D = new THREE.Group();
  /** THE plane. Mutated in place; every clipped material holds this instance. */
  private readonly plane = new THREE.Plane(new THREE.Vector3(0, 0, -1), 0);
  private readonly planes: THREE.Plane[] = [this.plane];

  private readonly stencils = new Map<THREE.Mesh, StencilPair>();
  private readonly backMat: THREE.MeshBasicMaterial;
  private readonly frontMat: THREE.MeshBasicMaterial;
  private readonly capMat: THREE.MeshStandardMaterial;
  private readonly cap: THREE.Mesh;

  private enabled = false;
  private sketchActive = false;
  /** Cap transform needs recomputing (the plane moved, or the body set changed). */
  private capDirty = true;
  /**
   * The cap's SIZE and centre need a scene-bounds traversal; its POSITION and
   * orientation do not. Separated because an offset drag moves the plane every
   * tick while the bodies stand still — re-measuring the scene per tick would
   * put a full `Box3.setFromObject` on the drag path for nothing.
   */
  private boundsDirty = true;
  private readonly capCentre = new THREE.Vector3();
  private capSize = 1;

  constructor(private readonly deps: SectionLayerDeps) {
    this.object3D.name = "sectionRoot";
    this.object3D.visible = false;

    // Stencil-only: no color, no depth, and depthTest OFF so an occluded piece
    // of the cross-section still counts (the algorithm is about being INSIDE a
    // solid, not about what is nearest the camera).
    const baseStencil = {
      depthWrite: false,
      depthTest: false,
      colorWrite: false,
      stencilWrite: true,
      stencilFunc: THREE.AlwaysStencilFunc,
      clippingPlanes: this.planes,
    };
    this.backMat = new THREE.MeshBasicMaterial({
      ...baseStencil,
      side: THREE.BackSide,
      stencilFail: THREE.IncrementWrapStencilOp,
      stencilZFail: THREE.IncrementWrapStencilOp,
      stencilZPass: THREE.IncrementWrapStencilOp,
    });
    this.frontMat = new THREE.MeshBasicMaterial({
      ...baseStencil,
      side: THREE.FrontSide,
      stencilFail: THREE.DecrementWrapStencilOp,
      stencilZFail: THREE.DecrementWrapStencilOp,
      stencilZPass: THREE.DecrementWrapStencilOp,
    });

    // The cut material. Lit like a body face (it IS material), and NOT clipped
    // by our own plane — it lies exactly on it.
    this.capMat = new THREE.MeshStandardMaterial({
      color: palette.sectionCap(),
      metalness: 0.0,
      roughness: 0.6,
      side: THREE.DoubleSide,
      // Draw only where the stencil is non-zero, and RESET it to 0 as we go so
      // no later pass can misread a value we wrote.
      stencilWrite: true,
      stencilRef: 0,
      stencilFunc: THREE.NotEqualStencilFunc,
      stencilFail: THREE.ReplaceStencilOp,
      stencilZFail: THREE.ReplaceStencilOp,
      stencilZPass: THREE.ReplaceStencilOp,
      // Pulled TOWARD the camera: the cap is coplanar with the world grid
      // whenever the user cuts XY at 0, and the grid (depthWrite:false, painted
      // after us) would otherwise stipple straight through solid material.
      polygonOffset: true,
      polygonOffsetFactor: -1,
      polygonOffsetUnits: -1,
    });

    this.cap = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), this.capMat);
    this.cap.name = "sectionCap";
    this.cap.renderOrder = RENDER_ORDER.SECTION_CAP;
    this.cap.frustumCulled = false; // it is sized from the scene, not framed by it
    // Belt and braces over the Replace ops above: the stencil buffer is left as
    // three found it, whatever the fragment coverage was.
    this.cap.onAfterRender = (renderer) => renderer.clearStencil();
    this.object3D.add(this.cap);
    deps.root.add(this.object3D);
  }

  /** The plane array clipped materials hold, or `null` while the section is off. */
  clippingPlanes(): THREE.Plane[] | null {
    return this.enabled ? this.planes : null;
  }

  /**
   * Apply the store's section state. Always repaints — an offset drag calls this
   * once per tick and every tick has to reach the screen.
   */
  setState(state: SectionState): void {
    this.enabled = state.enabled;
    sectionPlane(state.plane, state.offsetMm, state.flip, this.plane);
    this.object3D.visible = state.enabled;
    if (!state.enabled) this.clearStencils();
    // Cheap now: an offset-only change re-projects the CACHED centre onto the
    // moved plane and touches nothing else (see positionCap).
    this.capDirty = true;
    this.applyCapVisibility();
    this.deps.invalidate();
  }

  /**
   * A sketch session opened or closed. The clip is unaffected; the CAP goes away
   * for the duration, because dimmed bodies render in the transparent pass and
   * would paint over it (see renderOrder.ts rule 4).
   */
  setSketchActive(active: boolean): void {
    if (this.sketchActive === active) return;
    this.sketchActive = active;
    this.applyCapVisibility();
    this.deps.invalidate();
  }

  /**
   * Whether the cap quad is being DRAWN — the e2e/debug probe.
   *
   * Not "the cut has a visible cross-section": with the plane parked off the
   * model every stencil pair still exists and the cap still renders, over a
   * stencil that sums to zero everywhere. Only picking (or a pixel) can tell
   * those two apart.
   */
  get capVisible(): boolean {
    return this.enabled && !this.sketchActive && this.stencils.size > 0;
  }

  /** Live stencil pair count (one per visible body face mesh). */
  get stencilCount(): number {
    return this.stencils.size;
  }

  /**
   * Per-frame reconcile, called from the render loop while the section is on.
   * Cheap and allocation-free once the body set is stable.
   */
  update(): void {
    if (!this.enabled || this.sketchActive) return;
    this.syncStencils();
    if (this.capDirty) {
      this.positionCap();
      this.capDirty = false;
    }
  }

  /** Theme change: the cut material re-reads its token. */
  refreshColors(): void {
    this.capMat.color.copy(palette.sectionCap());
  }

  dispose(): void {
    this.clearStencils();
    this.object3D.removeFromParent();
    this.cap.geometry.dispose();
    this.capMat.dispose();
    this.backMat.dispose();
    this.frontMat.dispose();
  }

  // ── internals ──────────────────────────────────────────────────────────────

  private applyCapVisibility(): void {
    const on = this.enabled && !this.sketchActive;
    this.cap.visible = on;
    for (const pair of this.stencils.values()) {
      pair.back.visible = on;
      pair.front.visible = on;
    }
  }

  /**
   * Mirror the VISIBLE body face meshes into stencil pairs.
   *
   * `traverseVisible` is doing four jobs at once: it skips a body the tree eye
   * hid, one isolation masked away, every body when the Layers filter turns
   * bodies off (it returns immediately if `bodiesRoot` itself is hidden), and
   * the face mesh of a body in wireframe mode. Capping any of those would draw
   * a cut surface for a body that is not on screen.
   */
  private syncStencils(): void {
    const seen = new Set<THREE.Mesh>();
    let changed = false;
    this.deps.getBodiesRoot().traverseVisible((o) => {
      if (o.userData.kind !== "face") return;
      const mesh = o as THREE.Mesh;
      seen.add(mesh);
      let pair = this.stencils.get(mesh);
      if (!pair) {
        mesh.updateWorldMatrix(true, false); // may be new since the last render
        pair = this.createPair(mesh);
        this.stencils.set(mesh, pair);
        this.object3D.add(pair.back, pair.front);
        changed = true;
      }
      // Bodies sit at identity today, but a transformed one must not leave its
      // cross-section behind at the origin.
      pair.back.matrix.copy(mesh.matrixWorld);
      pair.front.matrix.copy(mesh.matrixWorld);
      pair.back.matrixWorldNeedsUpdate = true;
      pair.front.matrixWorldNeedsUpdate = true;
    });
    for (const [mesh, pair] of this.stencils) {
      if (seen.has(mesh)) continue;
      this.disposePair(pair);
      this.stencils.delete(mesh);
      changed = true;
    }
    // The body set moved ⇒ the scene extent may have too; that is the ONLY
    // thing that re-measures the bounds.
    if (changed) {
      this.capDirty = true;
      this.boundsDirty = true;
    }
  }

  /** Two meshes over `mesh`'s geometry — SHARED, never cloned, never disposed here. */
  private createPair(mesh: THREE.Mesh): StencilPair {
    const make = (material: THREE.Material, order: number): THREE.Mesh => {
      const m = new THREE.Mesh(mesh.geometry, material);
      m.renderOrder = order;
      m.matrixAutoUpdate = false; // driven from the source mesh's world matrix
      m.frustumCulled = false; // culling a stencil write would punch a hole in the cap
      m.visible = this.enabled && !this.sketchActive;
      return m;
    };
    return {
      back: make(this.backMat, RENDER_ORDER.SECTION_STENCIL_BACK),
      front: make(this.frontMat, RENDER_ORDER.SECTION_STENCIL_FRONT),
    };
  }

  private disposePair(pair: StencilPair): void {
    // Geometry belongs to the mesh registry and the materials to this layer —
    // removing the objects is the whole teardown.
    this.object3D.remove(pair.back, pair.front);
  }

  private clearStencils(): void {
    for (const pair of this.stencils.values()) this.disposePair(pair);
    this.stencils.clear();
  }

  /**
   * Sit the cap quad ON the plane, centred over the scene and large enough that
   * no cross-section can reach its edge.
   */
  private positionCap(): void {
    if (this.boundsDirty) {
      const bounds = this.deps.getBounds();
      if (bounds) {
        bounds.getCenter(this.capCentre);
        this.capSize = Math.max(bounds.getSize(SCRATCH).length() * CAP_OVERSIZE, 1);
      } else {
        this.capCentre.set(0, 0, 0);
        this.capSize = 1;
      }
      this.boundsDirty = false;
    }
    this.plane.projectPoint(this.capCentre, this.cap.position);
    this.cap.quaternion.setFromUnitVectors(UNIT_Z, this.plane.normal);
    this.cap.scale.set(this.capSize, this.capSize, 1);
    this.cap.updateMatrix();
  }
}
