/*
 * FACE_COLORS ingestion (WP-A W4.5).
 *
 * Two things can silently go wrong here and neither throws:
 *   - de-indexing REORDERS triangles → every triangle→face lookup (picking,
 *     highlight drawRanges) binds to the wrong face,
 *   - the sRGB→linear conversion is skipped → authored colors render washed out
 *     while still "having a color attribute".
 * Both are pinned below against the raw MESH1 view.
 */
import { describe, it, expect, afterEach } from "vitest";
import * as THREE from "three";
import { bakeFaceColors, deIndexTriangles, hasAuthoredFaceColors } from "./faceColors";
import { parseMeshPayload } from "./parseMeshPayload";
import { palette, resetPaletteCache } from "../engine/palette";
import { makeBoxMesh, type FaceColor } from "@/ipc/mockMeshes";
import { makeBodyMeshViewFixture } from "@/test/fixtures/bodyMeshView";
import type { Rgba } from "@/ipc/types";

/** f:0 red, f:4 blue, the other four faces unset. */
const RED: FaceColor = [214, 74, 62, 255];
const BLUE: FaceColor = [58, 122, 196, 255];
const COLORS: ReadonlyArray<FaceColor | null> = [RED, null, null, null, BLUE, null];

const coloredBox = () => parseMeshPayload(makeBoxMesh(40, 40, 40, 0, [0, 0, 0], COLORS));

/** Expectations go through f32 — the attribute is a Float32Array, the palette is f64. */
const f32 = (t: readonly [number, number, number]): number[] => [...Float32Array.from(t)];

/** The linear-working-space triple three would give a MATERIAL of this sRGB color. */
function linear(c: FaceColor): number[] {
  const col = new THREE.Color().setRGB(c[0] / 255, c[1] / 255, c[2] / 255, THREE.SRGBColorSpace);
  return f32([col.r, col.g, col.b]);
}

/** The body-fill token as it lands in the attribute. */
function neutralTriple(): number[] {
  const n = palette.bodyNeutral();
  return f32([n.r, n.g, n.b]);
}

/** The baked rgb of de-indexed vertex `i`. */
function vertexColor(baked: Float32Array, i: number): number[] {
  return [baked[i * 3], baked[i * 3 + 1], baked[i * 3 + 2]];
}

afterEach(() => {
  document.documentElement.dataset.theme = "light";
  resetPaletteCache();
});

describe("hasAuthoredFaceColors", () => {
  it("is false for no section and for an all-unset table", () => {
    expect(hasAuthoredFaceColors(null)).toBe(false);
    expect(hasAuthoredFaceColors(new Uint8Array(8))).toBe(false);
    // Nonzero RGB with alpha 0 is still UNSET — alpha is the only marker.
    expect(hasAuthoredFaceColors(new Uint8Array([9, 9, 9, 0, 9, 9, 9, 0]))).toBe(false);
  });

  it("is true as soon as one face carries alpha", () => {
    expect(hasAuthoredFaceColors(new Uint8Array([0, 0, 0, 0, 1, 2, 3, 255]))).toBe(true);
  });
});

describe("deIndexTriangles", () => {
  it("preserves triangle ORDER — vertex 3t+k is exactly indices[3t+k]", () => {
    const v = coloredBox();
    const { positions, normals } = deIndexTriangles(v);

    expect(positions.length).toBe(v.indices.length * 3);
    expect(normals).not.toBeNull();
    for (let i = 0; i < v.indices.length; i++) {
      const src = v.indices[i] * 3;
      expect([positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2]]).toEqual([
        v.positions[src],
        v.positions[src + 1],
        v.positions[src + 2],
      ]);
      expect([normals![i * 3], normals![i * 3 + 1], normals![i * 3 + 2]]).toEqual([
        v.normals![src],
        v.normals![src + 1],
        v.normals![src + 2],
      ]);
    }
  });

  it("leaves normals null when the mesh carries none", () => {
    const v = coloredBox();
    const noNormals = { ...v, normals: null };
    expect(deIndexTriangles(noNormals).normals).toBeNull();
  });
});

describe("bakeFaceColors", () => {
  it("bakes the AUTHORED color, sRGB→linear, on every vertex of a colored face", () => {
    const v = coloredBox();
    const baked = bakeFaceColors(v);
    expect(baked.length).toBe(v.indices.length * 3);

    // f:0 owns triangles [0,2) → de-indexed vertices [0,6); f:4 owns [8,10) → [24,30).
    for (const i of [0, 1, 2, 3, 4, 5]) {
      expect(vertexColor(baked, i)).toEqual(linear(RED));
    }
    for (let i = 24; i < 30; i++) {
      expect(vertexColor(baked, i)).toEqual(linear(BLUE));
    }
    // The conversion is real: the linear value is NOT the raw 0–1 sRGB ratio.
    expect(baked[0]).not.toBeCloseTo(RED[0] / 255, 4);
  });

  it("bakes the CURRENT body-fill token on every unset face", () => {
    const v = coloredBox();
    const baked = bakeFaceColors(v);
    const neutral = neutralTriple();
    // f:1 owns triangles [2,4) → vertices [6,12).
    for (let i = 6; i < 12; i++) {
      expect(vertexColor(baked, i)).toEqual(neutral);
    }
  });

  it("re-bakes IN PLACE into a correctly-sized target (the theme path)", () => {
    const v = coloredBox();
    const baked = bakeFaceColors(v);
    const lightNeutral = palette.bodyNeutral().clone();

    document.documentElement.dataset.theme = "dark";
    resetPaletteCache();
    const again = bakeFaceColors(v, undefined, undefined, baked);

    expect(again).toBe(baked); // same array, no attribute swap needed
    // guard: the token really moved
    expect(palette.bodyNeutral().getHex()).not.toBe(lightNeutral.getHex());
    // Unset face follows the theme…
    expect(vertexColor(baked, 6)).toEqual(neutralTriple());
    // …while the authored face does not (it is data, not a token).
    expect(vertexColor(baked, 0)).toEqual(linear(RED));
  });

  it("allocates a fresh array when the target is the wrong length", () => {
    const v = coloredBox();
    const tooSmall = new Float32Array(3);
    const out = bakeFaceColors(v, undefined, undefined, tooSmall);
    expect(out).not.toBe(tooSmall);
    expect(out.length).toBe(v.indices.length * 3);
  });

  it("falls back to the body token everywhere when there is no FACE_COLORS section", () => {
    const v = parseMeshPayload(makeBoxMesh());
    const baked = bakeFaceColors(v);
    const neutral = neutralTriple();
    for (let i = 0; i < v.indices.length; i++) {
      expect(vertexColor(baked, i)).toEqual(neutral);
    }
  });

  it("applies authored face colors keyed by ElementId when the mesh ids are ElementIds", () => {
    const id = "el_top";
    const view = makeBodyMeshViewFixture({
      faces: [{ id, triangles: [[0, 1, 2]] }],
      idsHaveElementIds: true,
    });
    const authored = new Map<string, Rgba>([[id, RED]]);
    const baked = bakeFaceColors(view, undefined, authored);
    expect(vertexColor(baked, 0)).toEqual(linear(RED));
    expect(vertexColor(baked, 1)).toEqual(linear(RED));
    expect(vertexColor(baked, 2)).toEqual(linear(RED));
  });
});
