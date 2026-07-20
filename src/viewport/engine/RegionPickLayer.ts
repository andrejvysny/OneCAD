/*
 * RegionPickLayer — a transient multi-region picker in the engine's interactionRoot.
 * When a finished sketch yields several closed regions, the ModelToolController shows
 * this layer so the user can click the region to extrude/revolve. It owns one
 * translucent fill mesh per region (keyed by regionId), built from the region's
 * plane-local (u,v) `previewTriangles`, all under ONE group carrying the sketch-plane
 * basis so local (u,v,0) → world — exactly as SketchStaticLayer builds its region
 * fills (that layer is edit/tree-bound; this one is not, so the math is reused, not
 * the object).
 *
 * Colors come from design tokens (palette) — the engine never hard-codes hex. A
 * hovered / selected region tints brighter + more opaque. Hit-testing itself is pure
 * and lives in tools/preview/regionPick (the controller raycasts the plane, then
 * resolves u,v → regionId), so this layer is render-only.
 */
import * as THREE from "three";
import type { SketchPlane, SketchRegion } from "@/ipc/types";
import { planeBasisMatrix } from "./sketchBasis";
import { palette } from "./palette";

const FILL_OPACITY = 0.2;
const FILL_OPACITY_ACTIVE = 0.42;

export interface RegionPickDeps {
  root: THREE.Object3D; // interactionRoot
  invalidate: () => void;
}

interface RegionEntry {
  mesh: THREE.Mesh;
  mat: THREE.MeshBasicMaterial;
}

/** Merge one region's (u,v) triangles into a plane-local (x=u, y=v, z=0) geometry. */
function buildRegionGeometry(region: SketchRegion): THREE.BufferGeometry | null {
  const t = region.previewTriangles;
  if (!t || t.positions.length < 6 || t.indices.length < 3) return null;
  const vcount = t.positions.length / 2;
  const positions = new Float32Array(vcount * 3);
  for (let i = 0; i < vcount; i++) {
    positions[i * 3] = t.positions[i * 2];
    positions[i * 3 + 1] = t.positions[i * 2 + 1];
    positions[i * 3 + 2] = 0; // (u,v) → (x,y,0)
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geo.setIndex(t.indices.slice());
  geo.computeBoundingSphere();
  return geo;
}

export class RegionPickLayer {
  private readonly group = new THREE.Group();
  private readonly entries = new Map<string, RegionEntry>();
  private hoverId: string | null = null;
  private selectedId: string | null = null;
  private readonly _basis = new THREE.Matrix4();

  constructor(private readonly deps: RegionPickDeps) {
    this.group.name = "regionPick";
    this.group.matrixAutoUpdate = false;
    this.group.visible = false;
    deps.root.add(this.group);
  }

  /** Set the sketch plane basis (fill meshes are authored plane-local). */
  setPlane(plane: SketchPlane): void {
    planeBasisMatrix(plane, this._basis);
    this.group.matrix.copy(this._basis);
    this.group.matrixWorldNeedsUpdate = true;
    this.group.updateMatrixWorld(true);
    this.deps.invalidate();
  }

  /** (Re)build one translucent fill mesh per region, keyed by regionId. */
  setRegions(regions: SketchRegion[]): void {
    this.clearEntries();
    for (const r of regions) {
      const geo = buildRegionGeometry(r);
      if (!geo) continue;
      const mat = new THREE.MeshBasicMaterial({
        color: palette.bodyNeutral(),
        transparent: true,
        opacity: FILL_OPACITY,
        depthWrite: false,
        side: THREE.DoubleSide,
      });
      const mesh = new THREE.Mesh(geo, mat);
      mesh.renderOrder = 5;
      mesh.userData.regionId = r.regionId;
      this.group.add(mesh);
      this.entries.set(r.regionId, { mesh, mat });
    }
    this.hoverId = null;
    this.selectedId = null;
    this.applyTints();
    this.group.updateMatrixWorld(true);
    this.deps.invalidate();
  }

  /** Tint the hovered region (or clear when null). */
  setHover(id: string | null): void {
    if (this.hoverId === id) return;
    this.hoverId = id;
    this.applyTints();
    this.deps.invalidate();
  }

  /** Tint the selected region (or clear when null). */
  setSelected(id: string | null): void {
    if (this.selectedId === id) return;
    this.selectedId = id;
    this.applyTints();
    this.deps.invalidate();
  }

  setVisible(visible: boolean): void {
    this.group.visible = visible;
    this.deps.invalidate();
  }

  get visible(): boolean {
    return this.group.visible;
  }

  /** regionIds currently rendered (test/introspection seam). */
  get regionIds(): string[] {
    return [...this.entries.keys()];
  }

  private applyTints(): void {
    for (const [id, e] of this.entries) {
      const selected = id === this.selectedId;
      const hovered = id === this.hoverId;
      e.mat.color.copy(
        selected ? palette.selectedEdge() : hovered ? palette.hoverAccent() : palette.bodyNeutral(),
      );
      e.mat.opacity = selected || hovered ? FILL_OPACITY_ACTIVE : FILL_OPACITY;
    }
  }

  private clearEntries(): void {
    for (const e of this.entries.values()) {
      this.group.remove(e.mesh);
      e.mesh.geometry.dispose();
      e.mat.dispose();
    }
    this.entries.clear();
  }

  dispose(): void {
    this.clearEntries();
    this.deps.root.remove(this.group);
  }
}
