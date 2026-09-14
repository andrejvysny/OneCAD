/*
 * Compact OWNED geometry for a set of faces (VP-HARDENING spec §8.3, VP04).
 *
 * The old face highlight was a `BufferGeometry` that SHARED the body's
 * position/normal/index attributes with a narrowed `drawRange`. It owned no GL
 * buffers, so it could never be disposed — and every hover therefore left one
 * more geometry registered in the renderer's bookkeeping, which only releases a
 * binding on the geometry's own `dispose` event (finding R02).
 *
 * The replacement owns its buffers outright: the triangles of the requested
 * faces are COPIED into a compact position array under a local vertex remap,
 * with a remapped index. Disposal then frees exactly this overlay's buffers and
 * nothing the body is still drawing with.
 *
 * What is deliberately absent:
 *  - normals: the overlay materials are `MeshBasicMaterial` (unlit), so a
 *    normal buffer would be pure upload cost;
 *  - colors: an overlay is one flat tint by construction;
 *  - the face ordinals themselves, which stay EXTERNAL in
 *    `userData.faceOrdinals` — the compact vertex numbering is a rendering
 *    detail and must never become a topology name.
 *
 * Capacity is managed, not reallocated per change: a selection that grows one
 * face at a time reuses the same typed arrays until they no longer fit, then
 * grows geometrically (×1.5). That is what makes "select 40 faces one by one"
 * bounded instead of 40 allocations.
 */
import * as THREE from "three";
import type { MeshEntry } from "./meshRegistry";

/** Growth factor once the existing capacity no longer fits the requested set. */
const GROWTH = 1.5;

/** A contiguous run of the body's triangles, in TRIANGLE units. */
export interface TriangleRange {
  readonly start: number;
  readonly count: number;
}

/**
 * What {@link buildFaceSetGeometry} WILL allocate for this set, computed before
 * anything is allocated (spec §8.3, decision D5).
 *
 * The cache used to reserve a naive per-triangle estimate and then charge the
 * capacity the ×1.5 growth rule actually produced, which is strictly larger on
 * every growth step — finding PR-04, where a 52.8 MB reservation admitted a
 * 72 MB buffer into a 64 MiB budget. `bytes` is what the built geometry will
 * report from `OwnedFaceGeometry.bytes`; `peakBytes` adds the buffer being
 * replaced, which is still allocated while the new one is filled.
 */
export interface FaceSetCapacityPlan {
  /** Float capacity of the position buffer (3 per vertex). */
  readonly positionsCapacity: number;
  /** Uint32 capacity of the index buffer. */
  readonly indexCapacity: number;
  /** Owned bytes AFTER the build. */
  readonly bytes: number;
  /** Peak owned bytes DURING the build (old + new while the copy runs). */
  readonly peakBytes: number;
  /** True when the retained capacity no longer fits and new buffers are made. */
  readonly grows: boolean;
}

/**
 * Price `faceOrdinals` of `entry` against `reuse`'s retained capacity, applying
 * the exact growth rule {@link OwnedFaceGeometry.update} applies. Pass no
 * `reuse` (or a slot the cache refused to hand back, e.g. a pinned one) and the
 * plan prices a FRESH exact-fit buffer.
 */
export function planFaceSetCapacity(
  entry: MeshEntry,
  faceOrdinals: readonly number[],
  reuse?: OwnedFaceGeometry,
): FaceSetCapacityPlan {
  let triangles = 0;
  for (const ord of faceOrdinals) triangles += entry.view.faceRanges[ord * 2 + 1];
  // Worst case one distinct vertex per corner; the remap usually beats it, and
  // the slack is reused by the next update rather than trimmed.
  const needIndices = triangles * 3;
  const needPositions = needIndices * 3;
  const havePositions = reuse ? reuse.positions.length : 0;
  const haveIndices = reuse ? reuse.indices.length : 0;
  // Either overflow replaces BOTH buffers — they are bound to one geometry.
  const grows = needPositions > havePositions || needIndices > haveIndices;
  const positionsCapacity = grows
    ? Math.max(needPositions, Math.ceil(havePositions * GROWTH))
    : havePositions;
  const indexCapacity = grows ? Math.max(needIndices, Math.ceil(haveIndices * GROWTH)) : haveIndices;
  const bytes =
    positionsCapacity * Float32Array.BYTES_PER_ELEMENT +
    indexCapacity * Uint32Array.BYTES_PER_ELEMENT;
  return {
    positionsCapacity,
    indexCapacity,
    bytes,
    peakBytes: bytes + (grows && reuse ? reuse.bytes : 0),
    grows,
  };
}

/** Reused bounds probe — a selection rebuild must not allocate per vertex. */
const BOUNDS_SCRATCH = new THREE.Vector3();

/**
 * One owned compact overlay buffer. The wrapper is stable across updates; the
 * `geometry` object is NOT — exceeding capacity disposes it and builds a new
 * one, because replacing a live `BufferAttribute` in place would strand the old
 * GL buffer in the renderer until the geometry itself was disposed.
 */
export class OwnedFaceGeometry {
  geometry: THREE.BufferGeometry;
  /** Capacity-managed compact positions (xyz), longer than `vertexCount·3` after a shrink. */
  positions: Float32Array;
  /** Capacity-managed compact index, longer than `triangleCount·3` after a shrink. */
  indices: Uint32Array;
  vertexCount = 0;
  triangleCount = 0;
  /** Face ordinals the buffer currently holds, in ascending order. */
  faceOrdinals: readonly number[] = [];
  private disposed = false;

  constructor(positionCapacity: number, indexCapacity: number) {
    this.positions = new Float32Array(positionCapacity);
    this.indices = new Uint32Array(indexCapacity);
    this.geometry = makeGeometry(this.positions, this.indices);
  }

  /** Owned typed-array bytes — CAPACITY, which is what the cache budget spends. */
  get bytes(): number {
    return this.positions.byteLength + this.indices.byteLength;
  }

  /** Free only this overlay's buffers. The source body is untouched. */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.geometry.dispose();
  }

  /** Swap in larger buffers, disposing the geometry the old ones were bound to. */
  private grow(positionCapacity: number, indexCapacity: number): void {
    this.geometry.dispose();
    this.positions = new Float32Array(positionCapacity);
    this.indices = new Uint32Array(indexCapacity);
    this.geometry = makeGeometry(this.positions, this.indices);
  }

  /** Rewrite the buffer to hold exactly `faceOrdinals` of `entry`. */
  update(entry: MeshEntry, faceOrdinals: readonly number[]): void {
    // The cache disposes a slot THROUGH its owner so that the wrapper knows it
    // is spent; rewriting one afterwards would hand the renderer a geometry
    // whose GL buffers are already released, and silently draw nothing. Loud,
    // because it is unreachable by construction and can only mean a caller
    // kept a reference past a `drop`/`retireSource`.
    if (this.disposed) throw new Error("OwnedFaceGeometry.update on a disposed buffer");
    const ordinals = [...faceOrdinals].sort((a, b) => a - b);
    let triangles = 0;
    for (const ord of ordinals) triangles += entry.view.faceRanges[ord * 2 + 1];
    const needIndices = triangles * 3;
    // The SAME function the cache reserved against (spec §8.3, D5): sizing the
    // buffers anywhere else would let the reservation and the allocation drift,
    // which is exactly finding PR-04.
    const plan = planFaceSetCapacity(entry, ordinals, this);
    if (plan.grows) this.grow(plan.positionsCapacity, plan.indexCapacity);
    const written = writeFaceSlice(entry, ordinals, this.positions, this.indices);
    this.vertexCount = written.vertexCount;
    this.triangleCount = triangles;
    this.faceOrdinals = ordinals;
    this.geometry.setDrawRange(0, needIndices);
    this.geometry.userData.faceOrdinals = ordinals;
    const position = this.geometry.getAttribute("position") as THREE.BufferAttribute;
    position.needsUpdate = true;
    const index = this.geometry.getIndex();
    if (index) index.needsUpdate = true;
    computeSliceBounds(this.geometry, this.positions, written.vertexCount);
  }
}

function makeGeometry(positions: Float32Array, indices: Uint32Array): THREE.BufferGeometry {
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geometry.setIndex(new THREE.BufferAttribute(indices, 1));
  return geometry;
}

/**
 * Copy the triangles of `ordinals` into `positions`/`indices` under a local
 * vertex remap. Handles both source layouts: the zero-copy INDEXED one and the
 * DE-INDEXED one a body with authored FACE_COLORS carries (triangle ordinals
 * are identical in both — see meshRegistry's buildFaceGeometry).
 */
function writeFaceSlice(
  entry: MeshEntry,
  ordinals: readonly number[],
  positions: Float32Array,
  indices: Uint32Array,
): { vertexCount: number } {
  return writeTriangleSpans(
    entry,
    ordinals.map((ord) => ({
      start: entry.view.faceRanges[ord * 2],
      count: entry.view.faceRanges[ord * 2 + 1],
    })),
    positions,
    indices,
  );
}

/** The shared copy loop: triangle SPANS (triangle units) → compact buffers. */
function writeTriangleSpans(
  entry: MeshEntry,
  spans: readonly TriangleRange[],
  positions: Float32Array,
  indices: Uint32Array,
): { vertexCount: number } {
  const source = entry.geometry.getAttribute("position").array as Float32Array;
  const sourceIndex = entry.geometry.getIndex();
  const sourceIndices = sourceIndex ? (sourceIndex.array as ArrayLike<number>) : null;
  const remap = new Map<number, number>();
  let vertexCount = 0;
  let out = 0;
  for (const span of spans) {
    const firstTri = span.start;
    const triCount = span.count;
    for (let t = firstTri; t < firstTri + triCount; t++) {
      for (let c = 0; c < 3; c++) {
        const corner = t * 3 + c;
        const sourceVertex = sourceIndices ? sourceIndices[corner] : corner;
        let compact = remap.get(sourceVertex);
        if (compact === undefined) {
          compact = vertexCount++;
          remap.set(sourceVertex, compact);
          positions[compact * 3] = source[sourceVertex * 3];
          positions[compact * 3 + 1] = source[sourceVertex * 3 + 1];
          positions[compact * 3 + 2] = source[sourceVertex * 3 + 2];
        }
        indices[out++] = compact;
      }
    }
  }
  return { vertexCount };
}

/**
 * Bounds from the USED prefix only — the capacity slack is stale coordinates
 * from a previous, larger selection and would inflate the frustum test.
 */
function computeSliceBounds(
  geometry: THREE.BufferGeometry,
  positions: Float32Array,
  vertexCount: number,
): void {
  const box = geometry.boundingBox ?? new THREE.Box3();
  box.makeEmpty();
  for (let v = 0; v < vertexCount; v++) {
    BOUNDS_SCRATCH.set(positions[v * 3], positions[v * 3 + 1], positions[v * 3 + 2]);
    box.expandByPoint(BOUNDS_SCRATCH);
  }
  geometry.boundingBox = box;
  geometry.boundingSphere = box.getBoundingSphere(geometry.boundingSphere ?? new THREE.Sphere());
}

/**
 * Build (or refresh in place) the compact owned geometry for `faceOrdinals` of
 * `entry`. Passing the previous buffer as `reuse` is what keeps a growing
 * multi-face selection to one allocation instead of one per change.
 */
export function buildFaceSetGeometry(
  entry: MeshEntry,
  faceOrdinals: readonly number[],
  reuse?: OwnedFaceGeometry,
): OwnedFaceGeometry {
  const owned = reuse ?? new OwnedFaceGeometry(0, 0);
  owned.update(entry, faceOrdinals);
  return owned;
}

/**
 * Compact OWNED geometry for one contiguous TRIANGLE range of `entry` — the
 * ranged L1 ghost (OffsetFace draws translucent copies of the operative faces
 * at their offset positions, not of the whole solid).
 *
 * Same vertex-remap copy as the face-set path, and owned for the same reason:
 * the shallow `drawRange` clone it replaces shared the body's BufferAttributes,
 * so it could never be disposed without freeing the body's buffers (R02). The
 * ghost material is an unlit `MeshBasicMaterial`, so no normals are carried.
 */
export function buildTriangleRangeGeometry(
  entry: MeshEntry,
  range: TriangleRange,
): THREE.BufferGeometry {
  const indexCount = range.count * 3;
  const positions = new Float32Array(indexCount * 3);
  const indices = new Uint32Array(indexCount);
  const written = writeTriangleSpans(entry, [range], positions, indices);
  const geometry = makeGeometry(positions, indices);
  geometry.setDrawRange(0, indexCount);
  computeSliceBounds(geometry, positions, written.vertexCount);
  return geometry;
}

