/*
 * FACE_COLORS → a per-vertex `color` attribute (MESH1 §4 type 12, WP-A W4.5).
 *
 * The worker emits authored appearance (an imported STEP body's XCAF colors) as
 * one sRGB RGBA per face, in FACE_RANGES face order, with `{0,0,0,0}` meaning
 * "unset — use the body material".
 *
 * ── Why DE-INDEX ────────────────────────────────────────────────────────────
 * INDICES is grouped by face over SHARED positions, so a vertex sitting on the
 * boundary between two differently-colored faces belongs to both. A per-vertex
 * attribute cannot express two colors at one vertex, so the color would bleed
 * across the crease. Duplicating each triangle's three vertices removes the
 * sharing; the cost (3·T vertices instead of V) is paid only by bodies that
 * actually carry authored colors.
 *
 * ── The order invariant ─────────────────────────────────────────────────────
 * De-indexing walks INDICES front to back, so TRIANGLE ORDINALS ARE UNCHANGED:
 * de-indexed triangle `t` occupies vertices `[3t, 3t+3)` exactly where indexed
 * triangle `t` occupied indices `[3t, 3t+3)`. Everything downstream addresses
 * triangles by ordinal — `TopoIndex.ordinalOf` binary-searches FACE_RANGES with
 * the raycaster's `faceIndex` (three reports `floor(i/3)` for indexed AND
 * non-indexed geometry alike), and `HighlightLayer.faceDrawRange` returns
 * `firstTri·3`, which is a valid `drawRange.start` in index units when indexed
 * and in vertex units when not. Both stay correct only because this order holds.
 *
 * ── Color space ─────────────────────────────────────────────────────────────
 * Authored colors are sRGB; three's vertex-color attribute is read RAW by the
 * shader (unlike `material.color`, nothing converts it), so the conversion to
 * the linear working space happens HERE, through the same `THREE.Color` path a
 * material color takes. `palette.bodyNeutral()` is already in working space (it
 * came through the Color constructor), so it is copied verbatim.
 */
import * as THREE from "three";
import { palette } from "../engine/palette";
import { TopoIndex } from "./faceRangeIndex";
import type { BodyMeshView } from "./parseMeshPayload";
import type { Rgba } from "@/ipc/types";

/** Alpha 0 = "unset" (mesh_format.md §4 type 12). */
const UNSET_ALPHA = 0;
const U8_MAX = 255;

/** True when at least one face carries an authored color — i.e. worth coloring. */
export function hasAuthoredFaceColors(faceColors: Uint8Array | null): boolean {
  if (!faceColors) return false;
  for (let i = 3; i < faceColors.length; i += 4) {
    if (faceColors[i] !== UNSET_ALPHA) return true;
  }
  return false;
}

/** Convert an sRGB+A tuple to a THREE.Color in linear working space. */
export function rgbaToLinear(c: Rgba): THREE.Color {
  return new THREE.Color().setRGB(c[0] / U8_MAX, c[1] / U8_MAX, c[2] / U8_MAX, THREE.SRGBColorSpace);
}

/** True when the mesh needs de-indexed vertex colors (authored/import face colors or a body color). */
export function needsVertexColors(
  view: BodyMeshView,
  bodyColor?: Rgba,
  authoredFaceColors?: ReadonlyMap<string, Rgba>,
): boolean {
  return (
    hasAuthoredFaceColors(view.faceColors) || bodyColor !== undefined || (authoredFaceColors?.size ?? 0) > 0
  );
}

/**
 * Expand the indexed face triangles so every triangle owns its three vertices,
 * in the SAME triangle order (see the order invariant above). Normals follow
 * their vertices; a mesh with no normals gets none.
 */
export function deIndexTriangles(view: BodyMeshView): {
  positions: Float32Array;
  normals: Float32Array | null;
} {
  const n = view.indices.length; // 3·T
  const positions = new Float32Array(n * 3);
  const srcNormals = view.normals;
  const normals = srcNormals ? new Float32Array(n * 3) : null;
  for (let i = 0; i < n; i++) {
    const v = view.indices[i] * 3;
    const o = i * 3;
    positions[o] = view.positions[v];
    positions[o + 1] = view.positions[v + 1];
    positions[o + 2] = view.positions[v + 2];
    if (normals && srcNormals) {
      normals[o] = srcNormals[v];
      normals[o + 1] = srcNormals[v + 1];
      normals[o + 2] = srcNormals[v + 2];
    }
  }
  return { positions, normals };
}

/**
 * Bake the per-vertex colors for a DE-INDEXED geometry: `3 · 3·T` floats in
 * three's linear working space. Unset faces (and any triangle no face range
 * claims) fall back to `bodyColor` when present, else the CURRENT body-fill
 * token. Authored face colors from the mesh override `bodyColor`. The function
 * is re-runnable — `target`, when it is the right length, is rewritten in place
 * so a theme flip costs no allocation and no attribute swap.
 */
export function bakeFaceColors(
  view: BodyMeshView,
  bodyColor?: Rgba,
  authoredFaceColors?: ReadonlyMap<string, Rgba>,
  target?: Float32Array,
): Float32Array {
  const floats = view.indices.length * 3;
  const out = target && target.length === floats ? target : new Float32Array(floats);

  const fallback = bodyColor ? rgbaToLinear(bodyColor) : palette.bodyNeutral();
  for (let i = 0; i < floats; i += 3) {
    out[i] = fallback.r;
    out[i + 1] = fallback.g;
    out[i + 2] = fallback.b;
  }

  const fc = view.faceColors;
  const c = new THREE.Color();
  const topo = new TopoIndex(view.faceRanges, view.faceCount, view.faceIdOffsets, view.faceIdChars);

  for (let f = 0; f < view.faceCount; f++) {
    const id = topo.idAt(f);
    const authored = id ? authoredFaceColors?.get(id) : undefined;
    if (authored) {
      c.copy(rgbaToLinear(authored));
    } else if (fc && fc[f * 4 + 3] !== UNSET_ALPHA) {
      c.setRGB(
        fc[f * 4] / U8_MAX,
        fc[f * 4 + 1] / U8_MAX,
        fc[f * 4 + 2] / U8_MAX,
        THREE.SRGBColorSpace,
      );
    } else {
      continue; // unset ⇒ keep fallback
    }
    const firstTri = view.faceRanges[f * 2];
    const triCount = view.faceRanges[f * 2 + 1];
    for (let t = firstTri; t < firstTri + triCount; t++) {
      for (let k = 0; k < 3; k++) {
        const o = (t * 3 + k) * 3;
        out[o] = c.r;
        out[o + 1] = c.g;
        out[o + 2] = c.b;
      }
    }
  }
  return out;
}
