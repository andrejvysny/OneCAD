/*
 * Compact OWNED face geometry (VP-HARDENING spec §8.3, finding R02).
 *
 * The fixture is deliberately uneven — faces of 2, 7 and 1 triangles — because
 * the defect this replaces was a shared wrapper with a narrowed `drawRange`,
 * which is correct for ANY range and owns nothing. Here the buffer must contain
 * exactly the requested face's triangles and nothing else.
 */
import { describe, it, expect, vi } from "vitest";
import {
  buildFaceSetGeometry,
  buildTriangleRangeGeometry,
  estimateFaceSetBytes,
  planFaceSetCapacity,
} from "./faceSliceGeometry";
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

/**
 * Five ONE-triangle faces — the PR-04 counterexample in miniature. Four of them
 * cost 192 bytes; the fifth estimates 240 and actually allocates 288 under the
 * ×1.5 growth rule.
 */
function fourAndFiveTriangleEntry(): MeshEntry {
  const positions: number[] = [];
  for (let v = 0; v < 5 * 3; v++) positions.push(v, v * 2, v * 3);
  const faces: FixtureFace[] = [];
  for (let f = 0; f < 5; f++) {
    faces.push({ id: `f:${f}`, triangles: [[f * 3, f * 3 + 1, f * 3 + 2]] });
  }
  const view = makeBodyMeshViewFixture({ positions, faces });
  const validated = validateMeshView(view, "small");
  if (!validated.ok) throw new Error(`fixture failed validation: ${validated.code}`);
  return buildBodyObjects(validated.mesh, "small", 1);
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

/*
 * D5 — the capacity PLAN (PR-04). The cache used to reserve
 * `estimateFaceSetBytes` and then charge whatever the ×1.5 growth rule actually
 * allocated, which is strictly larger whenever the buffer grows: 4 triangles
 * (192 B) followed by 5 (estimate 240 B) allocates 288 B. The plan is the one
 * authority for both numbers, and `peakBytes` also carries the outgoing buffer,
 * which is alive at the same time as the new one during the copy.
 */
describe("planFaceSetCapacity", () => {
  it("predicts buildFaceSetGeometry bytes exactly — fresh, grow, fit, regrow", () => {
    const entry = fixtureEntry();

    // FRESH: no reuse ⇒ exact fit, nothing else alive.
    const fresh = planFaceSetCapacity(entry, [1]);
    const owned = buildFaceSetGeometry(entry, [1]);
    expect(fresh.grows).toBe(true);
    expect(owned.bytes).toBe(fresh.bytes);
    expect(fresh.peakBytes).toBe(fresh.bytes);

    // GROW: the old buffer is still allocated while the new one is filled.
    const before = owned.bytes;
    const grow = planFaceSetCapacity(entry, [0, 1, 2], owned);
    expect(grow.grows).toBe(true);
    expect(grow.peakBytes).toBe(grow.bytes + before);
    buildFaceSetGeometry(entry, [0, 1, 2], owned);
    expect(owned.bytes).toBe(grow.bytes);

    // FIT: a smaller set keeps the capacity, so the price is unchanged and the
    // reservation must not double-count a buffer that is never replaced.
    const fit = planFaceSetCapacity(entry, [2], owned);
    expect(fit.grows).toBe(false);
    expect(fit.bytes).toBe(grow.bytes);
    expect(fit.peakBytes).toBe(fit.bytes);
    buildFaceSetGeometry(entry, [2], owned);
    expect(owned.bytes).toBe(fit.bytes);

    // REGROW after the shrink: still inside the retained capacity.
    const regrow = planFaceSetCapacity(entry, [0, 1, 2], owned);
    expect(regrow.grows).toBe(false);
    expect(regrow.bytes).toBe(grow.bytes);
    buildFaceSetGeometry(entry, [0, 1, 2], owned);
    expect(owned.bytes).toBe(regrow.bytes);

    owned.dispose();
    entry.dispose();
  });

  it("prices the ×1.5 growth the estimate misses (the PR-04 counterexample)", () => {
    const entry = fourAndFiveTriangleEntry();

    const four = planFaceSetCapacity(entry, [0, 1, 2, 3]);
    expect(estimateFaceSetBytes(entry, [0, 1, 2, 3])).toBe(192);
    expect(four.bytes).toBe(192);
    const owned = buildFaceSetGeometry(entry, [0, 1, 2, 3]);
    expect(owned.bytes).toBe(192);

    // The estimate says 240; the buffer the build will actually own is 288,
    // and 192 of the old one is still live while it is copied.
    const five = planFaceSetCapacity(entry, [0, 1, 2, 3, 4], owned);
    expect(estimateFaceSetBytes(entry, [0, 1, 2, 3, 4])).toBe(240);
    expect(five.bytes).toBe(288);
    expect(five.peakBytes).toBe(288 + 192);
    buildFaceSetGeometry(entry, [0, 1, 2, 3, 4], owned);
    expect(owned.bytes).toBe(288);

    owned.dispose();
    entry.dispose();
  });
});

describe("buildTriangleRangeGeometry", () => {
  it("owns a compact copy of exactly that triangle range", () => {
    const entry = fixtureEntry();
    // Face 1 is triangles [2, 9) — take its middle three.
    const geometry = buildTriangleRangeGeometry(entry, { start: 3, count: 3 });

    expect(geometry.getIndex()!.count).toBe(9);
    expect(geometry.drawRange.count).toBe(9);
    expect(geometry.getAttribute("position").array).not.toBe(entry.view.positions);
    expect(geometry.getAttribute("normal")).toBeUndefined();

    const source = entry.geometry.getAttribute("position").array as Float32Array;
    const sourceIndices = entry.geometry.getIndex()!.array as ArrayLike<number>;
    const positions = geometry.getAttribute("position").array as Float32Array;
    const index = geometry.getIndex()!.array as ArrayLike<number>;
    for (let c = 0; c < 9; c++) {
      const sourceVertex = sourceIndices[(3 + Math.floor(c / 3)) * 3 + (c % 3)];
      for (let k = 0; k < 3; k++) {
        expect(positions[index[c] * 3 + k]).toBe(source[sourceVertex * 3 + k]);
      }
    }

    const sourceSpy = vi.spyOn(entry.geometry, "dispose");
    geometry.dispose();
    expect(sourceSpy).not.toHaveBeenCalled();
    entry.dispose();
  });
});
