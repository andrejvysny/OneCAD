/*
 * HighlightLayer — hover + selected highlighting.
 *
 * FACE and BODY highlights ride the plan's zero-copy path: a tiny THREE.Mesh
 * whose geometry is a SHALLOW clone sharing the body geometry's BufferAttributes
 * (position/normal/index), with `drawRange` narrowed to the face slice. Those
 * geometries own NO unique GPU buffers, so they are NEVER disposed — disposing
 * would free the attributes the real body is still drawing with.
 *
 * EDGE highlights cannot: body edges are fat lines now (LineSegments2), whose
 * geometry is INSTANCED — one instance per segment — so `drawRange` has no
 * meaning on it and there is no way to narrow a shared buffer to one edge. Each
 * edge highlight therefore builds its own LineSegmentsGeometry over a `subarray`
 * VIEW of the entry's segment positions: no CPU copy, but a fresh
 * InstancedInterleavedBuffer and hence its own GL buffer. Those, and only those,
 * ARE disposed on teardown (see `clearObjects`). This is a deliberate, narrow
 * amendment to the never-dispose contract above.
 *
 * N selected elements ⇒ N tiny objects in interactionRoot.
 *
 * `setState` (selection change) and `refresh` (after a mesh swap) rebuild against
 * the CURRENT registry, dropping any highlight whose body no longer has a mesh.
 */
import * as THREE from "three";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/examples/jsm/lines/LineSegmentsGeometry.js";
import type { EntityRef } from "@/stores/selectionStore";
import { getEntry, type MeshEntry } from "../mesh/meshRegistry";
import { ordinalForRef } from "../mesh/rebindPick";
import { palette } from "./palette";
import { cssLineWidth } from "./SketchObject";
import { RENDER_ORDER } from "./renderOrder";

// ── Pure slice math (unit-tested) ───────────────────────────────────────────

/** Face ordinal → indexed-geometry drawRange (index units: 3 per triangle). */
export function faceDrawRange(
  faceRanges: Uint32Array,
  faceOrdinal: number,
): { start: number; count: number } {
  return { start: faceRanges[faceOrdinal * 2] * 3, count: faceRanges[faceOrdinal * 2 + 1] * 3 };
}

/**
 * Edge ordinal → its span of the entry's segment buffer, in SEGMENT units.
 *
 * Segments, not vertices: a fat-line geometry is instanced one-per-segment, and
 * the slice is taken as `positions.subarray(first * 6, (first + count) * 6)`
 * (6 floats = two xyz endpoints).
 */
export function edgeSegmentSlice(
  segRanges: Uint32Array,
  edgeOrdinal: number,
): { first: number; count: number } {
  return { first: segRanges[edgeOrdinal * 2], count: segRanges[edgeOrdinal * 2 + 1] };
}

export interface HighlightDeps {
  root: THREE.Object3D; // interactionRoot
  invalidate: () => void;
}

const HOVER_OPACITY = 0.45;
const SELECT_OPACITY = 0.55;
/** A whole selected body is a large area — the same tint as one face would shout. */
const SELECT_BODY_OPACITY = 0.45;
/** Highlight edges are drawn heavier than a body edge so they read THROUGH it. */
const HIGHLIGHT_EDGE_WIDTH = cssLineWidth(3);

export class HighlightLayer {
  private hover: EntityRef | null = null;
  private selected: EntityRef[] = [];
  private readonly objects: THREE.Object3D[] = [];

  // Shared materials (layer-owned — the only thing disposed on teardown).
  private readonly hoverFaceMat: THREE.MeshBasicMaterial;
  private readonly selFaceMat: THREE.MeshBasicMaterial;
  private readonly selBodyMat: THREE.MeshBasicMaterial;
  private readonly hoverEdgeMat: LineMaterial;
  private readonly selEdgeMat: LineMaterial;

  constructor(private readonly deps: HighlightDeps) {
    const faceOverlay = {
      transparent: true,
      depthWrite: false,
      polygonOffset: true,
      polygonOffsetFactor: -1,
      polygonOffsetUnits: -1,
      side: THREE.DoubleSide,
      toneMapped: false,
    } as const;
    // Hover is the CYAN viewport token, not the UI accent: the accent is also
    // the preview/handle color, so hovering a face used to read as a preview.
    this.hoverFaceMat = new THREE.MeshBasicMaterial({
      color: palette.hover3d(),
      opacity: HOVER_OPACITY,
      ...faceOverlay,
    });
    this.selFaceMat = new THREE.MeshBasicMaterial({
      color: palette.selected3d(),
      opacity: SELECT_OPACITY,
      ...faceOverlay,
    });
    // Same token as selFaceMat, lighter — a whole body's worth of tint at the
    // per-face opacity buries the shading that tells you what shape it is.
    this.selBodyMat = new THREE.MeshBasicMaterial({
      color: palette.selected3d(),
      opacity: SELECT_BODY_OPACITY,
      ...faceOverlay,
    });
    const edgeOverlay = {
      linewidth: HIGHLIGHT_EDGE_WIDTH,
      // depthTest off: a highlighted edge lies exactly on the body edge it
      // replaces and must win outright. transparent keeps it in the same
      // (transparent) list as the face overlays, ordered by RENDER_ORDER.
      depthTest: false,
      transparent: true,
      toneMapped: false,
    } as const;
    this.hoverEdgeMat = new LineMaterial({ color: palette.hover3d().getHex(), ...edgeOverlay });
    this.selEdgeMat = new LineMaterial({ color: palette.selectedEdge().getHex(), ...edgeOverlay });
  }

  setState(hover: EntityRef | null, selected: EntityRef[]): void {
    this.hover = hover;
    this.selected = selected;
    this.rebuild();
  }

  /** Rebuild against the current registry (call after a mesh swap). */
  refresh(): void {
    this.rebuild();
  }

  /**
   * Theme change: re-read the palette into the five shared materials.
   * `rebuild()` only recreates OBJECTS — it reuses these materials, so it can
   * never pick up a new theme on its own.
   */
  refreshColors(): void {
    this.hoverFaceMat.color.copy(palette.hover3d());
    this.selFaceMat.color.copy(palette.selected3d());
    this.selBodyMat.color.copy(palette.selected3d());
    this.hoverEdgeMat.color.copy(palette.hover3d());
    this.selEdgeMat.color.copy(palette.selectedEdge());
  }

  private rebuild(): void {
    this.clearObjects();
    for (const ref of this.selected) this.addHighlight(ref, false);
    if (this.hover && !this.selected.some((r) => r.id === this.hover!.id && r.kind === this.hover!.kind)) {
      this.addHighlight(this.hover, true);
    }
    this.deps.invalidate();
  }

  private addHighlight(ref: EntityRef, isHover: boolean): void {
    if (ref.kind === "face" || ref.kind === "edge") {
      if (!ref.bodyId || !ref.topoKey) return;
      const entry = getEntry(ref.bodyId);
      if (!entry) return;
      const obj =
        ref.kind === "face"
          ? this.buildFace(entry, ref, isHover)
          : this.buildEdge(entry, ref, isHover);
      if (obj) this.attach(obj);
    } else if (ref.kind === "body") {
      const entry = getEntry(ref.id);
      if (entry) this.attach(this.buildBody(entry, isHover));
    }
    // sketch / feature refs have no viewport geometry.
  }

  private buildFace(entry: MeshEntry, ref: EntityRef, isHover: boolean): THREE.Mesh | null {
    // Through `ordinalForRef`, not the raw TopoKey: a ref that survived a regen
    // may only be nameable by its promoted ElementId (see mesh/rebindPick).
    const ord = ordinalForRef(entry.faceIndex, entry.view, ref);
    if (ord < 0) return null;
    const g = this.shareIndexed(entry.geometry);
    const { start, count } = faceDrawRange(entry.view.faceRanges, ord);
    g.setDrawRange(start, count);
    const mesh = new THREE.Mesh(g, isHover ? this.hoverFaceMat : this.selFaceMat);
    mesh.renderOrder = RENDER_ORDER.HIGHLIGHT_FACE;
    return mesh;
  }

  private buildEdge(entry: MeshEntry, ref: EntityRef, isHover: boolean): LineSegments2 | null {
    if (!entry.edgeSegmentPositions || !entry.edgeIndex || !entry.edgeSegmentRanges) return null;
    const ord = ordinalForRef(entry.edgeIndex, entry.view, ref);
    if (ord < 0) return null;
    const { first, count } = edgeSegmentSlice(entry.edgeSegmentRanges, ord);
    if (count === 0) return null;
    // A subarray VIEW, not a copy — the CPU side stays zero-copy; only the GL
    // buffer setPositions creates is this object's own (and gets disposed).
    const g = new LineSegmentsGeometry();
    g.setPositions(entry.edgeSegmentPositions.subarray(first * 6, (first + count) * 6));
    const line = new LineSegments2(g, isHover ? this.hoverEdgeMat : this.selEdgeMat);
    line.renderOrder = RENDER_ORDER.HIGHLIGHT_EDGE;
    return line;
  }

  private buildBody(entry: MeshEntry, isHover: boolean): THREE.Mesh {
    const g = this.shareIndexed(entry.geometry);
    g.setDrawRange(0, entry.view.indices.length); // all faces
    const mesh = new THREE.Mesh(g, isHover ? this.hoverFaceMat : this.selBodyMat);
    mesh.renderOrder = RENDER_ORDER.HIGHLIGHT_FACE;
    return mesh;
  }

  /** Shallow clone that SHARES the source's attributes + index (zero-copy). */
  private shareIndexed(src: THREE.BufferGeometry): THREE.BufferGeometry {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", src.getAttribute("position"));
    const normal = src.getAttribute("normal");
    if (normal) g.setAttribute("normal", normal);
    const index = src.getIndex();
    if (index) g.setIndex(index);
    return g;
  }

  private attach(obj: THREE.Object3D): void {
    this.deps.root.add(obj);
    this.objects.push(obj);
  }

  private clearObjects(): void {
    for (const o of this.objects) {
      this.deps.root.remove(o);
      // Face/body overlays share the body's BufferAttributes — disposing one
      // would free buffers the real body is still drawing with. Edge overlays
      // (LineSegmentsGeometry) own the InstancedInterleavedBuffer setPositions
      // built for them, so they are the one kind that MUST be disposed; the
      // geometry's own type flag is the discriminator.
      const geo = (o as Partial<THREE.Mesh>).geometry as
        | (THREE.BufferGeometry & { isLineSegmentsGeometry?: boolean })
        | undefined;
      if (geo?.isLineSegmentsGeometry) geo.dispose();
    }
    this.objects.length = 0;
  }

  dispose(): void {
    this.clearObjects();
    this.hoverFaceMat.dispose();
    this.selFaceMat.dispose();
    this.selBodyMat.dispose();
    this.hoverEdgeMat.dispose();
    this.selEdgeMat.dispose();
  }
}
