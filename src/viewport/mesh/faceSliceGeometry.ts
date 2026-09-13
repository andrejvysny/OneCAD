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
    const ordinals = [...faceOrdinals].sort((a, b) => a - b);
    let triangles = 0;
    for (const ord of ordinals) triangles += entry.view.faceRanges[ord * 2 + 1];
    // Worst case one distinct vertex per corner; the remap usually beats it,
    // and the slack is reused by the next update rather than trimmed.
    const needIndices = triangles * 3;
    const needPositions = needIndices * 3;
    if (needPositions > this.positions.length || needIndices > this.indices.length) {
      this.grow(
        Math.max(needPositions, Math.ceil(this.positions.length * GROWTH)),
        Math.max(needIndices, Math.ceil(this.indices.length * GROWTH)),
      );
    }
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
  const source = entry.geometry.getAttribute("position").array as Float32Array;
  const sourceIndex = entry.geometry.getIndex();
  const sourceIndices = sourceIndex ? (sourceIndex.array as ArrayLike<number>) : null;
  const remap = new Map<number, number>();
  let vertexCount = 0;
  let out = 0;
  for (const ord of ordinals) {
    const firstTri = entry.view.faceRanges[ord * 2];
    const triCount = entry.view.faceRanges[ord * 2 + 1];
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
 * Upper bound on the bytes {@link buildFaceSetGeometry} would own for this set,
 * BEFORE building it. The cache reserves against this so an overlay that cannot
 * fit the budget is never allocated only to be thrown away.
 */
export function estimateFaceSetBytes(
  entry: MeshEntry,
  faceOrdinals: readonly number[],
): number {
  let triangles = 0;
  for (const ord of faceOrdinals) triangles += entry.view.faceRanges[ord * 2 + 1];
  return triangles * 3 * (3 * Float32Array.BYTES_PER_ELEMENT + Uint32Array.BYTES_PER_ELEMENT);
}
