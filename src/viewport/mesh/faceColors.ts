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

/**
 * The render module's contribution to this bake: base colors coming from
 * ASSIGNED MATERIALS rather than from authored/functional appearance.
 *
 * Both are already in three's LINEAR working space (OpenPBR colors are
 * linear-light), so unlike `Rgba` they are copied verbatim with no sRGB decode.
 *
 * They sit BELOW functional color in the precedence order — see the two-track
 * rule in {@link bakeFaceColors}.
 */
export interface MaterialFaceColors {
  /** meshFaceId → the overriding material's `base_color`. */
  readonly overrides: ReadonlyMap<string, readonly [number, number, number]>;
  /** The BODY material's `base_color`, or null when the body has no material. */
  readonly bodyBase: readonly [number, number, number] | null;
}

/**
 * Structural equality, so a re-resolve that produced the same answer costs no
 * re-bake. The caller rebuilds these records from scratch on every material
 * sync, so identity would report "changed" every single time.
 */
export function materialFaceColorsEqual(
  a: MaterialFaceColors | undefined,
  b: MaterialFaceColors | undefined,
): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (!triplesEqual(a.bodyBase, b.bodyBase)) return false;
  if (a.overrides.size !== b.overrides.size) return false;
  for (const [id, color] of a.overrides) {
    if (!triplesEqual(color, b.overrides.get(id))) return false;
  }
  return true;
}

function triplesEqual(
  a: readonly [number, number, number] | null | undefined,
  b: readonly [number, number, number] | null | undefined,
): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a[0] === b[0] && a[1] === b[1] && a[2] === b[2];
}

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

/**
 * True when the mesh needs de-indexed vertex colors (authored/import face
 * colors, a body color, or per-face MATERIAL overrides).
 *
 * A material override needs the attribute for the same reason an authored color
 * does: it colors ONE face of a body the rest of which is drawn by a single
 * shared program. A body-level material does NOT — its `base_color` is uniform
 * across the body, so it lives on the pooled material and the geometry stays
 * indexed and zero-copy.
 */
export function needsVertexColors(
  view: BodyMeshView,
  bodyColor?: Rgba,
  authoredFaceColors?: ReadonlyMap<string, Rgba>,
  materialColors?: MaterialFaceColors,
): boolean {
  return (
    hasAuthoredFaceColors(view.faceColors) ||
    bodyColor !== undefined ||
    (authoredFaceColors?.size ?? 0) > 0 ||
    (materialColors?.overrides.size ?? 0) > 0
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
 * three's linear working space. The function is re-runnable — `target`, when it
 * is the right length, is rewritten in place so a theme flip costs no
 * allocation and no attribute swap.
 *
 * ── The TWO-TRACK precedence (one bake, not two) ────────────────────────────
 * A body can carry FUNCTIONAL color (the user painted this body/face to mean
 * something, or an imported STEP file did) and an assigned MATERIAL at the same
 * time. In the modeling view functional color WINS, per face:
 *
 *     face  = authored face color ?? mesh FACE_COLORS ?? material override ?? fallback
 *     fallback = body color ?? material base_color ?? the body-fill TOKEN
 *
 * The token is last precisely so a body with NO material behaves exactly as it
 * always has — and it is also the only entry in that ladder that moves with the
 * theme, which is what makes `rebakeFaceColors` a theme concern.
 */
export function bakeFaceColors(
  view: BodyMeshView,
  bodyColor?: Rgba,
  authoredFaceColors?: ReadonlyMap<string, Rgba>,
  target?: Float32Array,
  materialColors?: MaterialFaceColors,
): Float32Array {
  const floats = view.indices.length * 3;
  const out = target && target.length === floats ? target : new Float32Array(floats);

  const materialBase = materialColors?.bodyBase;
  const fallback = bodyColor
    ? rgbaToLinear(bodyColor)
    : materialBase
      ? new THREE.Color().setRGB(materialBase[0], materialBase[1], materialBase[2])
      : palette.bodyNeutral();
  for (let i = 0; i < floats; i += 3) {
    out[i] = fallback.r;
    out[i + 1] = fallback.g;
    out[i + 2] = fallback.b;
  }

  const fc = view.faceColors;
  const c = new THREE.Color();
  const topo = new TopoIndex(view.faceRanges, view.faceCount, view.faceIdOffsets, view.faceIdChars);

  for (let f = 0; f < view.faceCount; f++) {
    // `idOf` (ordinal → id), NOT `idAt` (triangle index → id): this loop walks
    // FACE ORDINALS. `idAt(f)` range-searches `f` as a triangle, so on any mesh
    // with more than one triangle per face it answers with a different face's
    // id — six box faces resolved as f:0,f:0,f:1,f:1,f:2,f:2, which bound half
    // the authored colors to the wrong face and left f:3..f:5 unreachable.
    const id = topo.idOf(f);
    const authored = authoredFaceColors?.get(id);
    const override = materialColors?.overrides.get(id);
    if (authored) {
      c.copy(rgbaToLinear(authored));
    } else if (fc && fc[f * 4 + 3] !== UNSET_ALPHA) {
      c.setRGB(
        fc[f * 4] / U8_MAX,
        fc[f * 4 + 1] / U8_MAX,
        fc[f * 4 + 2] / U8_MAX,
        THREE.SRGBColorSpace,
      );
    } else if (override) {
      // Already linear — no sRGB decode, unlike every branch above.
      c.setRGB(override[0], override[1], override[2]);
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
