/*
 * PreviewMesh — the Level-1 extrude preview (NEW_SPEC §15) living in the engine's
 * interactionRoot. A UNIT prism (built once per profile from the region
 * triangulation, plane-local) sits under a group carrying the sketch-plane basis;
 * the live drag only writes `mesh.scale.z` / `mesh.position.z`, so each pointer
 * frame is ZERO-allocation. Translucent accent material (design palette) so the
 * exact L2 body can swap in underneath while L1 stays on top.
 */
import * as THREE from "three";
import type { SketchPlane } from "@/ipc/types";
import { planeBasisMatrix } from "./sketchBasis";
import { unitPrismGeometry, type PrismProfile } from "@/tools/preview/prismPreview";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";

export interface PreviewMeshDeps {
  root: THREE.Object3D; // interactionRoot
  invalidate: () => void;
}

export class PreviewMesh {
  private readonly group = new THREE.Group();
  private readonly material: THREE.MeshBasicMaterial;
  // One unit prism per armed region (N==1 in the single-region path; N in Wave 2's
  // multi-select). All share the one material, so a tint / depth write hits them all.
  private meshes: THREE.Mesh[] = [];
  /** Last tint applied, so a theme change can re-derive instead of resetting. */
  private cut = false;
  private readonly _basis = new THREE.Matrix4();

  constructor(private readonly deps: PreviewMeshDeps) {
    this.group.name = "extrudePreview";
    this.group.matrixAutoUpdate = false;
    this.material = new THREE.MeshBasicMaterial({
      color: palette.hoverAccent(),
      transparent: true,
      opacity: 0.3,
      depthWrite: false,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
    deps.root.add(this.group);
  }

  /** Single-region convenience: build one unit prism (delegates to setProfiles). */
  setProfile(plane: SketchPlane, profile: PrismProfile): void {
    this.setProfiles(plane, [profile]);
  }

  /** Build one unit prism per profile on `plane` (rebuild only when the set changes). */
  setProfiles(plane: SketchPlane, profiles: PrismProfile[]): void {
    this.disposeMeshes();
    for (const profile of profiles) {
      const { positions, indices } = unitPrismGeometry(profile);
      const geo = new THREE.BufferGeometry();
      geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
      geo.setIndex(new THREE.BufferAttribute(indices, 1));
      const mesh = new THREE.Mesh(geo, this.material);
      mesh.renderOrder = RENDER_ORDER.PREVIEW_MESH; // over bodies + highlights
      this.group.add(mesh);
      this.meshes.push(mesh);
    }

    planeBasisMatrix(plane, this._basis);
    this.group.matrix.copy(this._basis);
    this.group.matrixWorldNeedsUpdate = true;
    this.deps.invalidate();
  }

  /**
   * Set the live depth on every prism. Symmetric grows both ways from the plane
   * with `depth` as the TOTAL span (half on each side) — exactly what the worker
   * builds for `extrudeMode: "Symmetric"` (SCHEMA §7.3; the L1 used to draw
   * 2·|depth| and the user approved a body twice the committed size). A negative
   * depth extrudes the other side (drag-through-zero flip).
   */
  setDepth(depth: number, symmetric: boolean): void {
    for (const mesh of this.meshes) {
      if (symmetric) {
        const h = Math.abs(depth) || 1e-4;
        mesh.scale.z = h;
        mesh.position.z = -h / 2;
      } else {
        mesh.scale.z = Math.abs(depth) < 1e-4 ? 1e-4 : depth;
        mesh.position.z = 0;
      }
    }
    this.deps.invalidate();
  }

  /** Recolor the prisms: destructive (Cut boolean) vs the normal accent (Wave 2). */
  setTint(cut: boolean): void {
    this.cut = cut;
    this.material.color.copy(cut ? palette.destructive() : palette.hoverAccent());
    this.deps.invalidate();
  }

  /**
   * Theme change: re-read the palette, PRESERVING the current tint. Resetting
   * to the accent here would silently turn a live Cut preview back to Add.
   */
  refreshColors(): void {
    this.material.color.copy(this.cut ? palette.destructive() : palette.hoverAccent());
  }

  setVisible(visible: boolean): void {
    this.group.visible = visible;
    this.deps.invalidate();
  }

  get visible(): boolean {
    return this.group.visible && this.meshes.length > 0;
  }

  private disposeMeshes(): void {
    for (const mesh of this.meshes) {
      this.group.remove(mesh);
      mesh.geometry.dispose();
    }
    this.meshes = [];
  }

  dispose(): void {
    this.disposeMeshes();
    this.material.dispose();
    this.deps.root.remove(this.group);
  }
}
