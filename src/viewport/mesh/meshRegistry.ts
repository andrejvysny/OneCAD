/*
 * Mesh registry — a plain module Map<bodyId, MeshEntry> living OUTSIDE zustand.
 *
 * GPU geometry is heavy, imperative, and must be disposed deterministically, so
 * it never belongs in a projection store. `buildBodyObjects` turns a parsed
 * BodyMeshView into THREE geometry with ZERO-COPY attributes (positions/normals/
 * index alias the MESH1 blob directly); edges are expanded into fat-line segment
 * endpoints (the only non-trivial transform). `swap` double-buffers: the new
 * entry is published immediately and the old one is disposed on the NEXT frame
 * (via flushDisposals) so nothing that referenced it this frame reads freed
 * buffers. `disposeAll` + a dev leak tripwire guarantee the registry is empty
 * after a document closes.
 */
import * as THREE from "three";
import { LineSegmentsGeometry } from "three/examples/jsm/lines/LineSegmentsGeometry.js";
import { logError } from "@/debug/log";
import type { BodyMeshView } from "./parseMeshPayload";
import { TopoIndex } from "./faceRangeIndex";
import {
  bakeFaceColors,
  deIndexTriangles,
  materialFaceColorsEqual,
  needsVertexColors,
  type MaterialFaceColors,
} from "./faceColors";
import type { Rgba } from "@/ipc/types";

export interface MeshEntry {
  readonly bodyId: string;
  readonly meshRev: number;
  readonly view: BodyMeshView;
  /**
   * Faces. Plain bodies: indexed geometry whose attributes alias the blob
   * (zero-copy). Bodies with authored FACE_COLORS: DE-INDEXED, with a baked
   * per-vertex `color` attribute (see faceColors.ts — triangle order, and
   * therefore picking and highlight drawRanges, is identical either way).
   */
  readonly geometry: THREE.BufferGeometry;
  /**
   * Edges: expanded segment endpoints as a FAT-line geometry (null when the
   * mesh has no edges). `setPositions` retains the array BY REFERENCE — no
   * CPU-side copy — and computes the bounds eagerly, which the LineSegments2
   * raycast needs before the first render.
   */
  readonly edgeGeometry: LineSegmentsGeometry | null;
  /**
   * The very array `edgeGeometry` was built from (xyzxyz per segment), kept so
   * HighlightLayer can slice one edge's segments out of it without re-expanding
   * the polylines. Null when the mesh has no edges.
   */
  readonly edgeSegmentPositions: Float32Array | null;
  /** Triangle index → face ordinal → lazy face id. */
  readonly faceIndex: TopoIndex;
  /** Segment ordinal → edge ordinal → lazy edge id (null when no edges). */
  readonly edgeIndex: TopoIndex | null;
  /** Packed {firstSeg, segCount} per edge, for the edge-highlight slice (null when no edges). */
  readonly edgeSegmentRanges: Uint32Array | null;
  /** True when `geometry` carries a baked per-vertex `color` attribute. */
  readonly hasVertexColors: boolean;
  /** The body color this entry was baked with (undefined = theme neutral). */
  readonly bodyColor?: Rgba;
  /** User-authored face colors this entry was baked with, keyed by the mesh id (ElementId or TopoKey). */
  readonly authoredFaceColors?: ReadonlyMap<string, Rgba>;
  /** Assigned-material colors this entry is currently baked with (see {@link setMaterialColors}). */
  readonly materialColors?: MaterialFaceColors;
  /**
   * Theme change: re-bake the color attribute against the CURRENT body-fill
   * token. Only unset faces move — authored colors are data, not a token — so
   * this is an in-place rewrite of the existing array, never a new attribute.
   * No-op for a body with no colors (and after dispose).
   */
  rebakeFaceColors(): void;
  /**
   * Re-bake against a new set of assigned-material colors (a material edit, an
   * override added/removed, a body reassigned).
   *
   * Returns `false` when the change would flip the geometry's LAYOUT — an
   * override appearing on a body that had no vertex colors at all, or the last
   * one leaving — because that is a de-index/re-index, not an attribute rewrite.
   * The caller then rebuilds the entry through the normal load path instead of
   * this class silently swapping attributes underneath live highlight clones,
   * which share this geometry's BufferAttributes by reference.
   */
  setMaterialColors(next: MaterialFaceColors | undefined): boolean;
  dispose(): void;
}

const registry = new Map<string, MeshEntry>();
const pendingDisposal: MeshEntry[] = [];
let liveGeometryCount = 0;
/** Incremented whenever the leak tripwire catches a non-empty registry on close. */
export let leakTripwireCount = 0;

/**
 * Build THREE geometry from a parsed view. Does NOT insert into the registry —
 * call {@link swap} to publish it. Face attributes are zero-copy views over the
 * MESH1 blob; only the edge segment buffer is materialised.
 */
export function buildBodyObjects(
  view: BodyMeshView,
  bodyId: string,
  meshRev: number,
  bodyColor?: Rgba,
  authoredFaceColors?: ReadonlyMap<string, Rgba>,
  materialColors?: MaterialFaceColors,
): MeshEntry {
  const geometry = new THREE.BufferGeometry();
  // `drawRange` counts INDICES when the geometry is indexed and VERTICES when it
  // is not — and de-indexing produces exactly `indices.length` vertices, so the
  // same count is right in both arms (and so are HighlightLayer's face ranges).
  const colorAttr = buildFaceGeometry(geometry, view, bodyColor, authoredFaceColors, materialColors);
  geometry.setDrawRange(0, view.indices.length);
  const bmin = view.bboxMin;
  const bmax = view.bboxMax;
  geometry.boundingBox = new THREE.Box3(
    new THREE.Vector3(bmin[0], bmin[1], bmin[2]),
    new THREE.Vector3(bmax[0], bmax[1], bmax[2]),
  );
  geometry.boundingSphere = geometry.boundingBox.getBoundingSphere(new THREE.Sphere());

  const faceIndex = new TopoIndex(view.faceRanges, view.faceCount, view.faceIdOffsets, view.faceIdChars);

  let edgeGeometry: LineSegmentsGeometry | null = null;
  let edgeSegmentPositions: Float32Array | null = null;
  let edgeIndex: TopoIndex | null = null;
  let edgeSegmentRanges: Uint32Array | null = null;

  if (view.hasEdges && view.edgeRanges && view.edgePositions && view.edgeIdOffsets && view.edgeIdChars) {
    const { positions, segRanges } = expandEdgeSegments(
      view.edgePositions,
      view.edgeRanges,
      view.edgeCount,
    );
    // Fat lines are instanced quads: one INSTANCE per segment, so `instanceCount`
    // (set by setPositions) is the whole draw extent — `setDrawRange` is
    // meaningless on this geometry and is deliberately not called.
    edgeGeometry = new LineSegmentsGeometry();
    edgeGeometry.setPositions(positions);
    edgeSegmentPositions = positions;
    edgeSegmentRanges = segRanges;
    edgeIndex = new TopoIndex(segRanges, view.edgeCount, view.edgeIdOffsets, view.edgeIdChars);
  }

  liveGeometryCount += edgeGeometry ? 2 : 1;
  let disposed = false;
  // Mutable ONLY through setMaterialColors — every other bake input is fixed for
  // the entry's lifetime (a new mesh means a new entry).
  let liveMaterialColors = materialColors;
  const rebake = (): void => {
    if (!colorAttr || disposed) return;
    bakeFaceColors(
      view,
      bodyColor,
      authoredFaceColors,
      colorAttr.array as Float32Array,
      liveMaterialColors,
    );
    colorAttr.needsUpdate = true;
  };
  return {
    bodyId,
    meshRev,
    view,
    geometry,
    edgeGeometry,
    edgeSegmentPositions,
    faceIndex,
    edgeIndex,
    edgeSegmentRanges,
    hasVertexColors: colorAttr !== null,
    bodyColor,
    authoredFaceColors,
    get materialColors() {
      return liveMaterialColors;
    },
    rebakeFaceColors: rebake,
    setMaterialColors(next) {
      if (disposed) return true;
      // The layout question, asked BEFORE anything is written: an entry built
      // without a color attribute has no place to put a per-face override, and
      // one built with it cannot give the attribute back.
      if (needsVertexColors(view, bodyColor, authoredFaceColors, next) !== (colorAttr !== null)) {
        return false;
      }
      if (materialFaceColorsEqual(liveMaterialColors, next)) return true;
      liveMaterialColors = next;
      rebake();
      return true;
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      geometry.dispose();
      edgeGeometry?.dispose();
      liveGeometryCount -= edgeGeometry ? 2 : 1;
    },
  };
}

/**
 * Populate the face geometry's attributes. Returns the baked `color` attribute
 * when the mesh carries authored FACE_COLORS, else null (and the plain indexed,
 * zero-copy layout).
 */
function buildFaceGeometry(
  geometry: THREE.BufferGeometry,
  view: BodyMeshView,
  bodyColor?: Rgba,
  authoredFaceColors?: ReadonlyMap<string, Rgba>,
  materialColors?: MaterialFaceColors,
): THREE.BufferAttribute | null {
  if (!needsVertexColors(view, bodyColor, authoredFaceColors, materialColors)) {
    geometry.setAttribute("position", new THREE.BufferAttribute(view.positions, 3));
    if (view.normals) {
      geometry.setAttribute("normal", new THREE.BufferAttribute(view.normals, 3));
    }
    geometry.setIndex(new THREE.BufferAttribute(view.indices, 1));
    return null;
  }
  // Colored: vertices are duplicated per triangle so a crease between two
  // authored colors stays crisp instead of interpolating across the shared
  // vertex. Zero-copy is traded away here, and only here.
  const { positions, normals } = deIndexTriangles(view);
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  if (normals) geometry.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
  const colorAttr = new THREE.BufferAttribute(
    bakeFaceColors(view, bodyColor, authoredFaceColors, undefined, materialColors),
    3,
  );
  geometry.setAttribute("color", colorAttr);
  return colorAttr;
}

/**
 * Expand per-edge polylines into GL_LINES segment endpoints. Edge `e` occupies
 * segments `[firstSeg, firstSeg+pointCount-1)`; segment ordinal → edge maps via
 * `segRanges` (packed {firstSeg, segCount}).
 */
export function expandEdgeSegments(
  edgePositions: Float32Array,
  edgeRanges: Uint32Array,
  edgeCount: number,
): { positions: Float32Array; segRanges: Uint32Array; segTotal: number } {
  let segTotal = 0;
  for (let e = 0; e < edgeCount; e++) {
    segTotal += Math.max(0, edgeRanges[e * 2 + 1] - 1);
  }
  const positions = new Float32Array(segTotal * 6); // 2 verts × 3 floats per segment
  const segRanges = new Uint32Array(edgeCount * 2);
  let segCursor = 0;
  let out = 0;
  for (let e = 0; e < edgeCount; e++) {
    const firstPoint = edgeRanges[e * 2];
    const pointCount = edgeRanges[e * 2 + 1];
    const segCount = Math.max(0, pointCount - 1);
    segRanges[e * 2] = segCursor;
    segRanges[e * 2 + 1] = segCount;
    for (let p = 0; p < segCount; p++) {
      const a = (firstPoint + p) * 3;
      const b = (firstPoint + p + 1) * 3;
      positions[out++] = edgePositions[a];
      positions[out++] = edgePositions[a + 1];
      positions[out++] = edgePositions[a + 2];
      positions[out++] = edgePositions[b];
      positions[out++] = edgePositions[b + 1];
      positions[out++] = edgePositions[b + 2];
    }
    segCursor += segCount;
  }
  return { positions, segRanges, segTotal };
}

/** Publish `next` for its body; queue any previous entry for next-frame disposal. */
export function swap(bodyId: string, next: MeshEntry): void {
  const prev = registry.get(bodyId);
  registry.set(bodyId, next);
  if (prev && prev !== next) pendingDisposal.push(prev);
}

/** Remove a body's entry, queuing its geometry for next-frame disposal. */
export function remove(bodyId: string): void {
  const prev = registry.get(bodyId);
  if (prev) {
    registry.delete(bodyId);
    pendingDisposal.push(prev);
  }
}

export function getEntry(bodyId: string): MeshEntry | undefined {
  return registry.get(bodyId);
}

export function registrySize(): number {
  return registry.size;
}

/**
 * Theme change: re-bake every registered body's face colors, because the UNSET
 * faces carry the body-fill TOKEN and that token just moved. Authored colors
 * are unaffected. A body with no colors is a no-op, so this stays cheap even
 * with the whole registry loaded. Driven from `MeshIngest.refreshColors()`
 * (see engine/README.md § Theming) — it does NOT repaint on its own.
 */
export function refreshFaceColors(): void {
  for (const e of registry.values()) e.rebakeFaceColors();
}

/** Dispose entries queued by a previous swap/remove. Call once per rendered frame. */
export function flushDisposals(): void {
  if (pendingDisposal.length === 0) return;
  for (const e of pendingDisposal) e.dispose();
  pendingDisposal.length = 0;
}

/**
 * Dispose everything and clear the registry (document close). Dev leak tripwire:
 * after this the registry MUST be empty and no live geometry may remain — a
 * violation logs an `err`-level `vp` event and bumps {@link leakTripwireCount}.
 */
export function disposeAll(): void {
  for (const e of registry.values()) e.dispose();
  registry.clear();
  flushDisposals();
  if (registrySize() !== 0 || liveGeometryCount !== 0) {
    leakTripwireCount++;
    logError("vp", "meshRegistry leak tripwire after disposeAll", {
      size: registrySize(),
      liveGeometries: liveGeometryCount,
    });
  }
}

/** Test-only: reset internal counters (does NOT dispose — call disposeAll first). */
export function __resetRegistryForTests(): void {
  registry.clear();
  pendingDisposal.length = 0;
  liveGeometryCount = 0;
  leakTripwireCount = 0;
}
