/*
 * GhostLayer — the Level-1 preview for the "duplicate a body" tools
 * (LinearPattern / CircularPattern / MirrorBody). It instances a SOURCE body's
 * existing registry geometry as translucent clones at a set of transforms
 * (translate / rotate / mirror), living under the engine's interactionRoot.
 *
 * This is the cheap-and-honest preview: no re-modelling, just reused geometry at
 * offsets (the exact fused body arrives from the backend regen on commit). The
 * transform MATH is pure (tools/preview/patternPreview); this layer only builds
 * the THREE.Matrix4 per descriptor and manages the clone meshes' lifetime.
 *
 * OWNERSHIP (spec §8.2, decision D6): a whole-body ghost draws the registry's
 * EXACT geometry object under a `"ghost"` lease, and a RANGED ghost owns a
 * compact copy cut by `faceSliceGeometry`. `hide()`/`show()`/`dispose()` give
 * back exactly what was taken. The single translucent material is unlit
 * (MeshBasicMaterial) so a mirror's negative-determinant matrix (flipped
 * winding) still renders correctly with DoubleSide — and, because it is unlit,
 * an owned ghost copy carries positions only.
 */
import * as THREE from "three";
import { acquireLease, type MeshEntry, type MeshLease } from "../mesh/meshRegistry";
import { buildTriangleRangeGeometry } from "../mesh/faceSliceGeometry";
import type { GhostTransform } from "@/tools/preview/patternPreview";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";

export interface GhostLayerDeps {
  root: THREE.Object3D; // interactionRoot
  invalidate: () => void;
}

/** One source body's geometry plus the placements to clone it at. */
export interface GhostInstances {
  entry: MeshEntry;
  transforms: GhostTransform[];
  /**
   * Draw only this INDEX slice of the body's geometry (index units — 3 per
   * triangle, exactly what `HighlightLayer.faceDrawRange` returns). Absent ⇒ the
   * whole body, which is what every clone-a-body tool wants.
   *
   * OffsetFace is the case this exists for: its L1 ghost is a translucent copy of
   * the OPERATIVE FACES at their offset positions, not of the whole solid — a
   * whole-body clone floating 2 mm away would show a translation, which is
   * precisely what an offset is NOT. A ranged item is drawn from OWNED compact
   * geometry (`buildTriangleRangeGeometry`), disposed when the ghost clears.
   */
  range?: { start: number; count: number };
}

/** Build a THREE.Matrix4 for one ghost transform descriptor. */
export function ghostMatrix(t: GhostTransform): THREE.Matrix4 {
  const m = new THREE.Matrix4();
  if (t.kind === "translate") {
    m.makeTranslation(t.offset[0], t.offset[1], t.offset[2]);
    return m;
  }
  if (t.kind === "rotate") {
    const axis = new THREE.Vector3(t.axis[0], t.axis[1], t.axis[2]).normalize();
    const origin = new THREE.Vector3(t.origin[0], t.origin[1], t.origin[2]);
    // T(origin) · R(axis, θ) · T(−origin)
    const rot = new THREE.Matrix4().makeRotationAxis(axis, t.angleRad);
    const toOrigin = new THREE.Matrix4().makeTranslation(origin.x, origin.y, origin.z);
    const fromOrigin = new THREE.Matrix4().makeTranslation(-origin.x, -origin.y, -origin.z);
    return toOrigin.multiply(rot).multiply(fromOrigin);
  }
  if (t.kind === "rawMatrix") {
    // Already composed by `placementMatrix` (row-major, the order Matrix4.set
    // takes) — see the GhostTransform note on why a placement is not folded here.
    m.set(...(t.m as unknown as Parameters<THREE.Matrix4["set"]>));
    return m;
  }
  // mirror: Householder reflection across the plane through `point` with `normal`.
  const n = new THREE.Vector3(t.normal[0], t.normal[1], t.normal[2]).normalize();
  const d = n.x * t.point[0] + n.y * t.point[1] + n.z * t.point[2];
  // Linear part L = I − 2·n·nᵀ ; translation = 2·d·n (plane offset from origin).
  // prettier-ignore
  m.set(
    1 - 2 * n.x * n.x,    -2 * n.x * n.y,    -2 * n.x * n.z, 2 * d * n.x,
       -2 * n.y * n.x, 1 - 2 * n.y * n.y,    -2 * n.y * n.z, 2 * d * n.y,
       -2 * n.z * n.x,    -2 * n.z * n.y, 1 - 2 * n.z * n.z, 2 * d * n.z,
                    0,                 0,                 0,           1,
  );
  return m;
}

/**
 * What one shown item did for its geometry — the two, and only two, ways a
 * borrower may hold viewport geometry (spec §8.2, decision D6).
 */
type GhostBorrow =
  | { readonly kind: "leased"; readonly lease: MeshLease }
  | { readonly kind: "owned"; readonly geometry: THREE.BufferGeometry };

export class GhostLayer {
  private readonly group = new THREE.Group();
  private readonly material: THREE.MeshBasicMaterial;
  private meshes: THREE.Mesh[] = [];
  /** One per shown item, given back in `clearMeshes`. */
  private borrows: GhostBorrow[] = [];

  constructor(private readonly deps: GhostLayerDeps) {
    this.group.name = "ghostLayer";
    this.group.visible = false;
    this.material = new THREE.MeshBasicMaterial({
      color: palette.hoverAccent(),
      transparent: true,
      opacity: 0.28,
      depthWrite: false,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
    deps.root.add(this.group);
  }

  /** Theme change: re-read the palette into the shared ghost material. */
  refreshColors(): void {
    this.material.color.copy(palette.hoverAccent());
  }

  /**
   * Section view: clip the ghost clones (`null` = unclipped).
   *
   * A pattern/mirror ghost is a copy of a REAL body, so it has to obey the same
   * cut the original does — otherwise the tool previews solid clones floating in
   * the half the user just removed. The L1 previews that are NOT body copies
   * (prism, revolve shell, drag handles) stay unclipped on purpose: half an
   * armed preview is worse than an inconsistent one.
   */
  setClippingPlanes(planes: THREE.Plane[] | null): void {
    const before = this.material.clippingPlanes?.length ?? 0;
    this.material.clippingPlanes = planes;
    if (before !== (planes?.length ?? 0)) this.material.needsUpdate = true;
    this.deps.invalidate();
  }

  /** Show translucent clones of `entry`'s geometry at each transform. */
  show(entry: MeshEntry, transforms: GhostTransform[]): void {
    this.showMulti([{ entry, transforms }]);
  }

  /**
   * Show clones for SEVERAL source bodies at once. The pattern/mirror tools clone
   * one body, but a `TransformBody` places a multi-body selection, and each of
   * those bodies has its own geometry — one `show` per body would clear the
   * previous body's clones (`show` is a replace, not an append).
   */
  showMulti(items: readonly GhostInstances[]): void {
    this.clearMeshes();
    for (const { entry, transforms, range } of items) {
      // RANGED: compact OWNED geometry, cut the same way a face highlight is.
      // The shallow `drawRange` clone this replaces shared the body's
      // BufferAttributes and could therefore never be disposed (R02/PR-06).
      // `range` is in INDEX units (3 per triangle — what `faceDrawRange`
      // returns); the slice builder speaks triangles.
      // UNRANGED: the registry's exact geometry object, under a lease, which is
      // the only thing that stops the registry freeing it mid-preview.
      let geometry: THREE.BufferGeometry;
      if (range) {
        geometry = buildTriangleRangeGeometry(entry, {
          start: range.start / 3,
          count: range.count / 3,
        });
        this.borrows.push({ kind: "owned", geometry });
      } else {
        const lease = acquireLease(entry, "ghost");
        if (lease.entry.resourceState === "disposed") {
          lease.release();
          continue;
        }
        geometry = entry.geometry;
        this.borrows.push({ kind: "leased", lease });
      }
      for (const t of transforms) {
        const mesh = new THREE.Mesh(geometry, this.material);
        mesh.matrixAutoUpdate = false;
        mesh.matrix.copy(ghostMatrix(t));
        mesh.renderOrder = RENDER_ORDER.GHOST;
        this.group.add(mesh);
        this.meshes.push(mesh);
      }
    }
    this.group.visible = this.meshes.length > 0;
    this.deps.invalidate();
  }

  hide(): void {
    this.clearMeshes();
    this.group.visible = false;
    this.deps.invalidate();
  }

  get visible(): boolean {
    return this.group.visible;
  }

  get instanceCount(): number {
    return this.meshes.length;
  }

  private clearMeshes(): void {
    // Off the scene first, then give each borrow back the way it was taken: a
    // lease is released (the registry stays the unique disposer of installed
    // geometry) and an owned compact copy is disposed by its owner, which is
    // this layer. Never the other way round — disposing a leased body geometry
    // would free the buffers the real body is still drawing with.
    for (const m of this.meshes) this.group.remove(m);
    this.meshes = [];
    for (const borrow of this.borrows) {
      if (borrow.kind === "leased") borrow.lease.release();
      else borrow.geometry.dispose();
    }
    this.borrows = [];
  }

  dispose(): void {
    this.clearMeshes();
    this.material.dispose();
    this.deps.root.remove(this.group);
  }
}
