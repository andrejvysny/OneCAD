/*
 * Compact OWNED face geometry (VP-HARDENING spec §8.3, finding R02).
 *
 * The fixture is deliberately uneven — faces of 2, 7 and 1 triangles — because
 * the defect this replaces was a shared wrapper with a narrowed `drawRange`,
 * which is correct for ANY range and owns nothing. Here the buffer must contain
 * exactly the requested face's triangles and nothing else.
 */
import { describe, it, expect, vi } from "vitest";
import { buildFaceSetGeometry, estimateFaceSetBytes } from "./faceSliceGeometry";
import { buildBodyObjects, type MeshEntry } from "./meshRegistry";
import { validateMeshView } from "./validateMesh";
import { makeBodyMeshViewFixture, type FixtureFace } from "@/test/fixtures/bodyMeshView";

const TRIANGLE_COUNTS = [2, 7, 1] as const;
const TOTAL_TRIS = TRIANGLE_COUNTS.reduce((a, b) => a + b, 0);

/** One unshared vertex per corner, so a face's vertex set is unambiguous. */
function fixtureEntry(): MeshEntry {
  const positions: number[] = [];
  for (let v = 0; v < TOTAL_TRIS * 3; v++) positions.push(v, v * 2, v * 3);
  const faces: FixtureFace[] = [];
  let vertex = 0;
  TRIANGLE_COUNTS.forEach((count, f) => {
    const triangles: Array<readonly [number, number, number]> = [];
    for (let t = 0; t < count; t++) {
      triangles.push([vertex, vertex + 1, vertex + 2]);
      vertex += 3;
    }
    faces.push({ id: `f:${f}`, triangles });
  });
  const view = makeBodyMeshViewFixture({ positions, faces });
  const validated = validateMeshView(view, "body1");
  if (!validated.ok) throw new Error(`fixture failed validation: ${validated.code}`);
  return buildBodyObjects(validated.mesh, "body1", 1);
}

describe("buildFaceSetGeometry", () => {
  it("holds exactly the requested face's triangles, copied not shared", () => {
    const entry = fixtureEntry();
    const owned = buildFaceSetGeometry(entry, [1]);

    expect(owned.triangleCount).toBe(7);
    expect(owned.geometry.drawRange).toEqual({ start: 0, count: 21 });
    expect(owned.vertexCount).toBe(21); // no shared corners in this fixture

    // Not one attribute object, and not one array, in common with the source.
    const source = entry.geometry.getAttribute("position");
    const overlay = owned.geometry.getAttribute("position");
    expect(overlay).not.toBe(source);
    expect(overlay.array).not.toBe(source.array);
    expect(owned.geometry.getIndex()).not.toBe(entry.geometry.getIndex());
    // Unlit overlay: no normals and no colors are carried.
    expect(owned.geometry.getAttribute("normal")).toBeUndefined();
    expect(owned.geometry.getAttribute("color")).toBeUndefined();

    // Every corner lands on the SOURCE coordinate of the vertex it remapped.
    const sourcePositions = source.array as Float32Array;
    const sourceIndices = entry.geometry.getIndex()!.array as ArrayLike<number>;
    const index = owned.geometry.getIndex()!.array as Uint32Array;
    const firstCorner = entry.view.faceRanges[2] * 3;
    for (let c = 0; c < 21; c++) {
      const sourceVertex = sourceIndices[firstCorner + c];
      const compact = index[c];
      for (let k = 0; k < 3; k++) {
        expect(owned.positions[compact * 3 + k]).toBe(sourcePositions[sourceVertex * 3 + k]);
      }
    }
    owned.dispose();
    entry.dispose();
  });

  it("combines several faces into ONE buffer, in ascending ordinal order", () => {
    const entry = fixtureEntry();
    const owned = buildFaceSetGeometry(entry, [2, 0]);

    expect(owned.faceOrdinals).toEqual([0, 2]);
    expect(owned.triangleCount).toBe(3); // 2 + 1
    expect(owned.geometry.userData.faceOrdinals).toEqual([0, 2]);
    owned.dispose();
    entry.dispose();
  });

  it("reuse keeps the same typed arrays while capacity suffices", () => {
    const entry = fixtureEntry();
    const owned = buildFaceSetGeometry(entry, [1]); // 7 triangles
    const positions = owned.positions;
    const indices = owned.indices;
    const geometry = owned.geometry;

    const again = buildFaceSetGeometry(entry, [0], owned); // 2 triangles — fits

    expect(again).toBe(owned);
    expect(again.positions).toBe(positions);
    expect(again.indices).toBe(indices);
    expect(again.geometry).toBe(geometry);
    expect(again.geometry.drawRange.count).toBe(6);
    owned.dispose();
    entry.dispose();
  });

  it("grows by ×1.5 (at least) when the set no longer fits, and frees the old buffers", () => {
    const entry = fixtureEntry();
    const owned = buildFaceSetGeometry(entry, [1]); // 21 indices, 63 floats
    const before = { positions: owned.positions.length, indices: owned.indices.length };
    const oldGeometry = owned.geometry;
    const oldSpy = vi.spyOn(oldGeometry, "dispose");

    buildFaceSetGeometry(entry, [0, 1, 2], owned); // 30 indices, 90 floats

    expect(owned.positions.length).toBeGreaterThanOrEqual(Math.ceil(before.positions * 1.5));
    expect(owned.indices.length).toBeGreaterThanOrEqual(Math.ceil(before.indices * 1.5));
    expect(owned.geometry).not.toBe(oldGeometry);
    // The outgrown buffers are released, not orphaned in the renderer.
    expect(oldSpy).toHaveBeenCalledTimes(1);
    expect(owned.geometry.drawRange.count).toBe(30);
    owned.dispose();
    entry.dispose();
  });

  it("dispose frees ONLY the overlay's buffers", () => {
    const entry = fixtureEntry();
    const owned = buildFaceSetGeometry(entry, [1]);
    const sourceSpy = vi.spyOn(entry.geometry, "dispose");
    const overlaySpy = vi.spyOn(owned.geometry, "dispose");

    owned.dispose();
    owned.dispose(); // idempotent

    expect(overlaySpy).toHaveBeenCalledTimes(1);
    expect(sourceSpy).not.toHaveBeenCalled();
    expect(entry.geometry.getAttribute("position").array).toBe(entry.view.positions);
    entry.dispose();
  });

  it("estimateFaceSetBytes bounds what the build will own", () => {
    const entry = fixtureEntry();
    const estimate = estimateFaceSetBytes(entry, [1]);
    const owned = buildFaceSetGeometry(entry, [1]);

    expect(owned.bytes).toBeLessThanOrEqual(estimate);
    expect(estimate).toBe(7 * 3 * (3 * 4 + 4));
    owned.dispose();
    entry.dispose();
  });

  it("works on a DE-INDEXED source (a colored body has no index buffer)", () => {
    // A body color de-indexes the registry geometry (meshRegistry's colored
    // arm). Triangle ordinals are identical either way, so the slice must
    // follow the FACE RANGES and not the presence of an index.
    const indexed = fixtureEntry();
    const colored = buildBodyObjects(indexed.view, "body1", 2, [200, 40, 40, 255]);
    expect(colored.geometry.getIndex()).toBeNull();

    const fromIndexed = buildFaceSetGeometry(indexed, [1]);
    const fromColored = buildFaceSetGeometry(colored, [1]);

    expect(fromColored.triangleCount).toBe(7);
    // Same triangles ⇒ the same coordinates reach the overlay from both layouts.
    const a = [...fromIndexed.indices.subarray(0, 21)].map((i) => [
      fromIndexed.positions[i * 3],
      fromIndexed.positions[i * 3 + 1],
      fromIndexed.positions[i * 3 + 2],
    ]);
    const b = [...fromColored.indices.subarray(0, 21)].map((i) => [
      fromColored.positions[i * 3],
      fromColored.positions[i * 3 + 1],
      fromColored.positions[i * 3 + 2],
    ]);
    expect(b).toEqual(a);

    fromIndexed.dispose();
    fromColored.dispose();
    colored.dispose();
    indexed.dispose();
  });
});
