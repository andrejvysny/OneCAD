/*
 * SketchStaticLayer — always-visible, NON-editable presence of every document
 * sketch in MODEL mode (Fusion-style). One group per sketch id, keyed under a
 * root group in `sketchRoot`:
 *   - fills  : one selectable mesh per triangulated profile region,
 *   - curves : thin LineSegments for the entity outlines,
 *   - dots   : constant-size THREE.Points at entity vertices.
 *
 * Everything is authored in plane (u,v) coordinates inside a group carrying the
 * plane basis matrix (see sketchBasis), so local (u,v,0) → world. This layer does
 * NOT reuse SketchObject (that one is edit-mode bound); it reuses the pure
 * `entityPolyline` + palette + planeBasisMatrix only.
 *
 * State it renders: per-sketch visibility (tree eye), the ONE sketch being edited
 * (hidden — the live SketchObject owns it), hover + selection tint. Picking
 * distinguishes exact filled regions from sketch curves.
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

export type SketchStaticTarget =
  | { kind: "sketch"; sketchId: string }
  | { kind: "sketchRegion"; sketchId: string; regionId: string };

export type SketchStaticHit = SketchStaticTarget & { distance: number };

/** Stable hover-change key; identity fields remain explicit on the hit itself. */
export function sketchStaticHitKey(hit: SketchStaticTarget): string {
  return hit.kind === "sketch"
    ? JSON.stringify(["sketch", hit.sketchId])
    : JSON.stringify(["sketchRegion", hit.sketchId, hit.regionId]);
}

interface SketchStaticDeps {
  sketchRoot: THREE.Object3D;
  invalidate: () => void;
}

interface StaticFill {
  mesh: THREE.Mesh;
  mat: THREE.MeshBasicMaterial;
}

interface StaticEntry {
  group: THREE.Group;
  lines: THREE.LineSegments;
  lineMat: THREE.LineBasicMaterial;
  points: THREE.Points;
  pointsMat: THREE.PointsMaterial;
  fills: Map<string, StaticFill>;
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
  } else if (e.type === "Circle" || e.type === "Ellipse") {
    add(e.center);
  } else if (e.type === "Arc") {
    add(e.center);
    add(e.start);
    add(e.end);
  } else if (e.type === "Point") {
    add(e.p0);
  }
}

/** One region's plane-local (u,v) triangles as an indexed local XY geometry. */
function buildFillGeometry(region: SketchRegion): THREE.BufferGeometry | null {
  const triangles = region.previewTriangles;
  if (!triangles || triangles.positions.length < 6 || triangles.indices.length < 3) {
    return null;
  }
  if ((triangles.holesSubtracted ?? 0) < region.holes.length) return null;
  const positions = new Float32Array((triangles.positions.length / 2) * 3);
  for (let i = 0; i < triangles.positions.length / 2; i++) {
    positions[i * 3] = triangles.positions[i * 2];
    positions[i * 3 + 1] = triangles.positions[i * 2 + 1];
    positions[i * 3 + 2] = 0;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geo.setIndex(triangles.indices.slice());
  geo.computeBoundingSphere();
  return geo;
}

export class SketchStaticLayer {
  private readonly root = new THREE.Group();
  private readonly entries = new Map<string, StaticEntry>();
  private editingId: string | null = null;
  private hover: SketchStaticTarget | null = null;
  private selectedKeys = new Set<string>();
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

    // One fill per exact planar cell. Hit-testing still gives a coplanar curve
    // within the line threshold priority, so boundaries remain sketch targets.
    const fills = new Map<string, StaticFill>();
    for (const region of regions) {
      if (fills.has(region.regionId)) continue;
      const fillGeo = buildFillGeometry(region);
      if (!fillGeo) continue;
      const fillMat = new THREE.MeshBasicMaterial({
        color: palette.hoverAccent(),
        transparent: true,
        opacity: FILL_OPACITY,
        depthWrite: false,
        side: THREE.DoubleSide,
      });
      const fill = new THREE.Mesh(fillGeo, fillMat);
      fill.name = `sketchStaticRegion_${region.regionId}`;
      fill.renderOrder = RENDER_ORDER.STATIC_FILL;
      fill.userData.sketchId = id;
      fill.userData.regionId = region.regionId;
      fill.userData.sketchStaticKind = "sketchRegion";
      group.add(fill);
      fills.set(region.regionId, { mesh: fill, mat: fillMat });
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
    lines.userData.sketchStaticKind = "sketch";
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
    return { group, lines, lineMat, points, pointsMat, fills, visible: true };
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

  setHover(hit: SketchStaticTarget | null): void {
    const before = this.hover ? sketchStaticHitKey(this.hover) : null;
    const next = hit ? sketchStaticHitKey(hit) : null;
    if (before === next) return;
    this.hover = hit;
    for (const key of this.entries.keys()) this.applyTint(key);
    this.deps.invalidate();
  }

  setSelected(hits: readonly SketchStaticTarget[]): void {
    this.selectedKeys = new Set(hits.map(sketchStaticHitKey));
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
    const sketchKey = sketchStaticHitKey({ kind: "sketch", sketchId: id });
    const sketchSelected = this.selectedKeys.has(sketchKey);
    const sketchHovered = this.hover?.kind === "sketch" && this.hover.sketchId === id;
    const color = sketchSelected
      ? palette.sketchSelected()
      : sketchHovered
        ? palette.hoverAccent()
        : palette.sketchFull();
    e.lineMat.color.copy(color);
    e.pointsMat.color.copy(color);
    for (const [regionId, fill] of e.fills) {
      const regionKey = sketchStaticHitKey({ kind: "sketchRegion", sketchId: id, regionId });
      const selected = sketchSelected || this.selectedKeys.has(regionKey);
      const hovered =
        sketchHovered ||
        (this.hover?.kind === "sketchRegion" &&
          this.hover.sketchId === id &&
          this.hover.regionId === regionId);
      fill.mat.color.copy(selected ? palette.sketchSelected() : palette.hoverAccent());
      fill.mat.opacity = selected || hovered ? FILL_OPACITY_ACTIVE : FILL_OPACITY;
    }
  }

  /**
   * Nearest visible sketch under `raycaster` (fill meshes + curves), or null. The
   * caller sets `raycaster.params.Line.threshold` (px→world). Flushes the group
   * world matrices first so a raycast right after setSketch resolves.
   */
  hitTest(raycaster: THREE.Raycaster): SketchStaticHit | null {
    this.root.updateMatrixWorld(true);
    const fills: THREE.Object3D[] = [];
    const lines: THREE.Object3D[] = [];
    for (const e of this.entries.values()) {
      if (!e.group.visible) continue;
      for (const fill of e.fills.values()) fills.push(fill.mesh);
      lines.push(e.lines);
    }
    const fillHit = raycaster.intersectObjects(fills, false)[0];
    const lineHit = raycaster.intersectObjects(lines, false)[0];
    const coplanarEpsilon = Math.max(1, fillHit?.distance ?? 0, lineHit?.distance ?? 0) * 1e-7;
    const hit =
      lineHit && (!fillHit || lineHit.distance <= fillHit.distance + coplanarEpsilon)
        ? lineHit
        : fillHit;
    if (!hit) return null;
    const sketchId = hit.object.userData.sketchId as string | undefined;
    if (!sketchId) return null;
    if (hit.object.userData.sketchStaticKind === "sketchRegion") {
      const regionId = hit.object.userData.regionId as string | undefined;
      return regionId
        ? { kind: "sketchRegion", sketchId, regionId, distance: hit.distance }
        : null;
    }
    return { kind: "sketch", sketchId, distance: hit.distance };
  }

  private disposeEntry(e: StaticEntry): void {
    e.lines.geometry.dispose();
    e.lineMat.dispose();
    e.points.geometry.dispose();
    e.pointsMat.dispose();
    for (const fill of e.fills.values()) {
      fill.mesh.geometry.dispose();
      fill.mat.dispose();
    }
    e.fills.clear();
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
