/*
 * SketchStaticLayer — always-visible, NON-editable presence of every document
 * sketch in MODEL mode (Fusion-style). One group per sketch id, keyed under a
 * root group in `sketchRoot`:
 *   - fill   : translucent triangulated profile (region previewTriangles, u,v),
 *   - curves : thin LineSegments for the entity outlines,
 *   - dots   : constant-size THREE.Points at entity vertices.
 *
 * Everything is authored in plane (u,v) coordinates inside a group carrying the
 * plane basis matrix (see sketchBasis), so local (u,v,0) → world. This layer does
 * NOT reuse SketchObject (that one is edit-mode bound); it reuses the pure
 * `entityPolyline` + palette + planeBasisMatrix only.
 *
 * State it renders: per-sketch visibility (tree eye), the ONE sketch being edited
 * (hidden — the live SketchObject owns it), hover + selection tint. Picking is via
 * `hitTest(raycaster)` → sketch id (the engine plumbs ndc + the Line threshold).
 *
 * matrixAutoUpdate is off (static geometry); the group world matrices are flushed
 * once after each build AND at the top of hitTest (stale-matrix lesson), so a
 * raycast right after setSketch resolves even before the next render frame.
 */
import * as THREE from "three";
import type { SketchEntity, SketchPlane, SketchRegion } from "@/ipc/types";
import { entityPolyline } from "./SketchObject";
import { planeBasisMatrix } from "./sketchBasis";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";

const FILL_OPACITY = 0.18;
const FILL_OPACITY_ACTIVE = 0.3;
const DOT_SIZE = 5;

export interface SketchStaticData {
  plane: SketchPlane;
  entities: SketchEntity[];
  regions: SketchRegion[];
}

interface SketchStaticDeps {
  sketchRoot: THREE.Object3D;
  invalidate: () => void;
}

interface StaticEntry {
  group: THREE.Group;
  lines: THREE.LineSegments;
  lineMat: THREE.LineBasicMaterial;
  points: THREE.Points;
  pointsMat: THREE.PointsMaterial;
  fill: THREE.Mesh | null;
  fillMat: THREE.MeshBasicMaterial | null;
  /** Intended (tree) visibility, before the editing-hide override. */
  visible: boolean;
}

/** Expand a sequential polyline (flat xyz) into LineSegments pairs. */
function polylineToSegments(poly: number[], out: number[]): void {
  const count = poly.length / 3;
  for (let i = 0; i < count - 1; i++) {
    const a = i * 3;
    const b = (i + 1) * 3;
    out.push(poly[a], poly[a + 1], poly[a + 2], poly[b], poly[b + 1], poly[b + 2]);
  }
}

/** Vertex dots for an entity (endpoints / center), flat local xyz. */
function collectDots(e: SketchEntity, out: number[]): void {
  const add = (p?: [number, number]): void => {
    if (p) out.push(p[0], p[1], 0);
  };
  if (e.type === "Line") {
    add(e.p0);
    add(e.p1);
  } else if (e.type === "Circle") {
    add(e.center);
  } else if (e.type === "Arc") {
    add(e.center);
    add(e.start);
    add(e.end);
  } else if (e.type === "Point") {
    add(e.p0);
  }
}

/** Merge every region's plane-local (u,v) triangles into one indexed geometry. */
function buildFillGeometry(regions: SketchRegion[]): THREE.BufferGeometry | null {
  const positions: number[] = [];
  const indices: number[] = [];
  let base = 0;
  for (const r of regions) {
    const t = r.previewTriangles;
    if (!t || t.positions.length < 6 || t.indices.length < 3) continue;
    const vcount = t.positions.length / 2;
    for (let i = 0; i < vcount; i++) {
      positions.push(t.positions[i * 2], t.positions[i * 2 + 1], 0); // (u,v) → (x,y,0)
    }
    for (const idx of t.indices) indices.push(base + idx);
    base += vcount;
  }
  if (positions.length === 0) return null;
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geo.setIndex(indices);
  geo.computeBoundingSphere();
  return geo;
}

export class SketchStaticLayer {
  private readonly root = new THREE.Group();
  private readonly entries = new Map<string, StaticEntry>();
  private editingId: string | null = null;
  private hoverId: string | null = null;
  private selectedIds = new Set<string>();
  private readonly _basis = new THREE.Matrix4();

  constructor(private readonly deps: SketchStaticDeps) {
    this.root.name = "sketchStaticRoot";
    deps.sketchRoot.add(this.root);
  }

  /** (Re)build one sketch's group, preserving its intended visibility across rebuilds. */
  setSketch(id: string, data: SketchStaticData): void {
    const prev = this.entries.get(id);
    const wasVisible = prev?.visible ?? true;
    if (prev) {
      this.disposeEntry(prev);
      this.root.remove(prev.group);
      this.entries.delete(id);
    }
    const entry = this.buildEntry(id, data);
    entry.visible = wasVisible;
    this.entries.set(id, entry);
    this.applyVisibility(id);
    this.applyTint(id);
    this.deps.invalidate();
  }

  private buildEntry(id: string, { plane, entities, regions }: SketchStaticData): StaticEntry {
    const group = new THREE.Group();
    group.name = `sketchStatic_${id}`;
    group.matrixAutoUpdate = false;
    planeBasisMatrix(plane, this._basis);
    group.matrix.copy(this._basis);
    group.matrixWorldNeedsUpdate = true;

    // Fill (translucent profile) — rendered first so curves sit on top.
    let fill: THREE.Mesh | null = null;
    let fillMat: THREE.MeshBasicMaterial | null = null;
    const fillGeo = buildFillGeometry(regions);
    if (fillGeo) {
      fillMat = new THREE.MeshBasicMaterial({
        color: palette.hoverAccent(),
        transparent: true,
        opacity: FILL_OPACITY,
        depthWrite: false,
        side: THREE.DoubleSide,
      });
      fill = new THREE.Mesh(fillGeo, fillMat);
      fill.renderOrder = RENDER_ORDER.STATIC_FILL;
      fill.userData.sketchId = id;
      group.add(fill);
    }

    // Curves + dots are coplanar with the fill (and with the ground grid when
    // the sketch sits on Z=0): render everything in the transparent pass with
    // depthWrite off so layering is purely RENDER_ORDER (see renderOrder.ts) —
    // an opaque curve would otherwise be painted BEFORE the fill and get
    // tinted/stippled by it. Depth test stays on: bodies still occlude.
    const segPos: number[] = [];
    for (const e of entities) polylineToSegments(entityPolyline(e), segPos);
    const lineGeo = new THREE.BufferGeometry();
    lineGeo.setAttribute("position", new THREE.Float32BufferAttribute(segPos, 3));
    lineGeo.computeBoundingSphere();
    const lineMat = new THREE.LineBasicMaterial({
      color: palette.sketchFull(),
      transparent: true,
      depthWrite: false,
    });
    const lines = new THREE.LineSegments(lineGeo, lineMat);
    lines.renderOrder = RENDER_ORDER.STATIC_CURVES;
    lines.userData.sketchId = id;
    group.add(lines);

    // Vertex dots.
    const dotPos: number[] = [];
    for (const e of entities) collectDots(e, dotPos);
    const ptGeo = new THREE.BufferGeometry();
    ptGeo.setAttribute("position", new THREE.Float32BufferAttribute(dotPos, 3));
    ptGeo.computeBoundingSphere();
    const pointsMat = new THREE.PointsMaterial({
      color: palette.sketchFull(),
      size: DOT_SIZE,
      sizeAttenuation: false,
      transparent: true,
      depthWrite: false,
    });
    const points = new THREE.Points(ptGeo, pointsMat);
    points.renderOrder = RENDER_ORDER.STATIC_POINTS;
    group.add(points);

    this.root.add(group);
    group.updateMatrixWorld(true);
    return { group, lines, lineMat, points, pointsMat, fill, fillMat, visible: true };
  }

  removeSketch(id: string): void {
    const e = this.entries.get(id);
    if (!e) return;
    this.disposeEntry(e);
    this.root.remove(e.group);
    this.entries.delete(id);
    this.deps.invalidate();
  }

  /** Set a sketch's tree visibility (the editing-hide override still applies). */
  setVisible(id: string, visible: boolean): void {
    const e = this.entries.get(id);
    if (!e) return;
    e.visible = visible;
    this.applyVisibility(id);
    this.deps.invalidate();
  }

  /** Hide the ONE sketch being edited (its live SketchObject owns it), show the rest. */
  setEditingSketch(id: string | null): void {
    if (this.editingId === id) return;
    this.editingId = id;
    for (const key of this.entries.keys()) this.applyVisibility(key);
    this.deps.invalidate();
  }

  setHover(id: string | null): void {
    if (this.hoverId === id) return;
    this.hoverId = id;
    for (const key of this.entries.keys()) this.applyTint(key);
    this.deps.invalidate();
  }

  setSelected(ids: string[]): void {
    this.selectedIds = new Set(ids);
    for (const key of this.entries.keys()) this.applyTint(key);
    this.deps.invalidate();
  }

  private applyVisibility(id: string): void {
    const e = this.entries.get(id);
    if (!e) return;
    e.group.visible = e.visible && id !== this.editingId;
  }

  private applyTint(id: string): void {
    const e = this.entries.get(id);
    if (!e) return;
    const selected = this.selectedIds.has(id);
    const hovered = this.hoverId === id;
    const color = selected
      ? palette.sketchSelected()
      : hovered
        ? palette.hoverAccent()
        : palette.sketchFull();
    e.lineMat.color.copy(color);
    e.pointsMat.color.copy(color);
    if (e.fillMat) e.fillMat.opacity = selected || hovered ? FILL_OPACITY_ACTIVE : FILL_OPACITY;
  }

  /**
   * Nearest visible sketch under `raycaster` (fill meshes + curves), or null. The
   * caller sets `raycaster.params.Line.threshold` (px→world). Flushes the group
   * world matrices first so a raycast right after setSketch resolves.
   */
  hitTest(raycaster: THREE.Raycaster): string | null {
    this.root.updateMatrixWorld(true);
    const targets: THREE.Object3D[] = [];
    for (const e of this.entries.values()) {
      if (!e.group.visible) continue;
      if (e.fill) targets.push(e.fill);
      targets.push(e.lines);
    }
    if (targets.length === 0) return null;
    const hit = raycaster.intersectObjects(targets, false)[0];
    return (hit?.object.userData.sketchId as string | undefined) ?? null;
  }

  private disposeEntry(e: StaticEntry): void {
    e.lines.geometry.dispose();
    e.lineMat.dispose();
    e.points.geometry.dispose();
    e.pointsMat.dispose();
    e.fill?.geometry.dispose();
    e.fillMat?.dispose();
  }

  dispose(): void {
    for (const e of this.entries.values()) {
      this.disposeEntry(e);
      this.root.remove(e.group);
    }
    this.entries.clear();
    this.deps.sketchRoot.remove(this.root);
  }
}
