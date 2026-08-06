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
 * Geometry is registry-owned (zero-copy), so a clone Mesh is disposed by removing
 * it — never by disposing the shared geometry. The single translucent material is
 * unlit (MeshBasicMaterial) so a mirror's negative-determinant matrix (flipped
 * winding) still renders correctly with DoubleSide.
 */
import * as THREE from "three";
import type { MeshEntry } from "../mesh/meshRegistry";
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
   * precisely what an offset is NOT. The geometry stays registry-owned and
   * zero-copy: a shallow clone that shares the attributes + index carries its own
   * `drawRange`, which is the same trick `HighlightLayer.buildFace` uses.
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
 * Shallow clone that SHARES the source's attributes + index (zero-copy), so the
 * clone can carry its own `drawRange` without duplicating a single vertex. Twin of
 * `HighlightLayer.shareIndexed`; kept local because the two layers own their
 * object lifetimes independently and neither may dispose the other's.
 */
function shareIndexed(src: THREE.BufferGeometry): THREE.BufferGeometry {
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", src.getAttribute("position"));
  const normal = src.getAttribute("normal");
  if (normal) g.setAttribute("normal", normal);
  const index = src.getIndex();
  if (index) g.setIndex(index);
  return g;
}

export class GhostLayer {
  private readonly group = new THREE.Group();
  private readonly material: THREE.MeshBasicMaterial;
  private meshes: THREE.Mesh[] = [];

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
      // A ranged item needs its OWN geometry object to carry the drawRange, so it
      // gets one shallow clone per item (shared attributes — still zero-copy) and
      // every transform instances that. An unranged item draws the registry
      // geometry directly, byte-for-byte the behaviour before ranges existed.
      const geometry = range ? shareIndexed(entry.geometry) : entry.geometry;
      if (range) geometry.setDrawRange(range.start, range.count);
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
    // NOT disposed, deliberately — every ghost geometry (the registry's own, and
    // the shallow ranged clones above) SHARES the body's BufferAttributes, and
    // `BufferGeometry.dispose()` frees those shared GL buffers out from under the
    // real body that is still drawing with them. Same rule, same reason, as
    // `HighlightLayer.clearObjects`.
    for (const m of this.meshes) this.group.remove(m);
    this.meshes = [];
  }

  dispose(): void {
    this.clearMeshes();
    this.material.dispose();
    this.deps.root.remove(this.group);
  }
}
