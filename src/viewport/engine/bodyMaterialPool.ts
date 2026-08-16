/*
 * BodyMaterialPool — ONE three.js face material per distinct material, handed to
 * every body wearing it.
 *
 * This is `BodyMaterialLibrary`'s sibling, not its replacement: the library owns
 * the design-token look (a body with no assigned material), this owns the
 * data-driven look (a body with one). Both obey the same SHARED-MATERIAL
 * DISCIPLINE — `acquire` returns the SAME instance for a key every time, so N
 * bodies sharing a material cost one program and one uniform upload, and nothing
 * outside this class may mutate a returned material (a per-body tweak would leak
 * onto every body sharing it). Per-body state belongs on the Object3D.
 *
 * The KEY is the caller's, and carries the whole contract: it must already
 * encode the resolved parameter hash (so a material edit produces a new key
 * rather than a stale cached program) and must carry a distinct suffix for the
 * vertex-colored twin (whose only difference is `vertexColors` + a white base).
 * The pool never derives a key itself — deriving one would mean re-implementing
 * the hash the material owner already computed.
 *
 * Lifetime is SWEEP-based rather than refcounted: after a sync pass the caller
 * hands over the set of keys still in use and everything else is disposed. A
 * refcount would need every caller to pair acquire/release perfectly across
 * async mesh loads; a sweep just asks the scene what it is actually drawing.
 */
import * as THREE from "three";
import { applyPbrParams, materialTier } from "./openPbrToThree";
import type { PbrMaterialParams } from "./pbrParams";

/** Sketch-mode dim opacity. Mirrors `BodyMaterialLibrary`'s cue exactly. */
const DIM_OPACITY = 0.35;

/** Face material state saved just before dimming, reapplied verbatim on restore. */
interface SavedFaceState {
  transparent: boolean;
  opacity: number;
  depthWrite: boolean;
}

interface PoolEntry {
  readonly material: THREE.MeshStandardMaterial;
  /** Kept so {@link BodyMaterialPool.resetFaceColor} can rebuild the base color. */
  readonly params: PbrMaterialParams;
  readonly vertexColored: boolean;
}

export class BodyMaterialPool {
  private readonly entries = new Map<string, PoolEntry>();
  private readonly savedFaceStates = new Map<string, SavedFaceState>();
  private dimmed = false;
  /** Cut-tint override; `null` = each entry's own params-derived color. Never borrowed. */
  private faceColor: THREE.Color | null = null;

  /**
   * The shared material for `key`, created on first use.
   *
   * `params`/`vertexColored` are read ONLY on a miss — on a hit the key is the
   * authority that the cached material is already correct, which is exactly what
   * makes the hash-in-the-key requirement load-bearing.
   */
  acquire(
    key: string,
    params: PbrMaterialParams,
    vertexColored: boolean,
  ): THREE.MeshStandardMaterial {
    const hit = this.entries.get(key);
    if (hit) return hit.material;

    const material = this.create(params, vertexColored);
    this.entries.set(key, { material, params, vertexColored });
    // Born under an active tint / dim: apply both, in the same order a live
    // entry would have received them. The dim's saved prior is this material's
    // constructor state, so undimming restores it correctly.
    if (this.faceColor) material.color.copy(this.faceColor);
    if (this.dimmed) this.applyDim(key, material);
    return material;
  }

  private create(params: PbrMaterialParams, vertexColored: boolean): THREE.MeshStandardMaterial {
    // Face CONVENTIONS are copied from BodyMaterialLibrary.create deliberately:
    // a pooled body and a library body sit in the same scene, are picked by the
    // same raycast and are outlined by the same edge lines, so DoubleSide (closed
    // solids, winding-independent picking), the polygonOffset that keeps edges off
    // the faces, and envMapIntensity 1.0 (scene.environmentIntensity already sets
    // the level) are not this class's choices to make differently.
    const shared = {
      vertexColors: vertexColored,
      envMapIntensity: 1.0,
      side: THREE.DoubleSide,
      polygonOffset: true,
      polygonOffsetFactor: 1,
      polygonOffsetUnits: 1,
    } as const;
    const material =
      materialTier(params) === "physical"
        ? new THREE.MeshPhysicalMaterial(shared)
        : new THREE.MeshStandardMaterial(shared);
    applyPbrParams(material, params, { vertexColored });
    return material;
  }

  /** Dispose and drop every entry whose key is not in `liveKeys`. */
  sweep(liveKeys: ReadonlySet<string>): void {
    for (const [key, entry] of [...this.entries]) {
      if (liveKeys.has(key)) continue;
      entry.material.dispose();
      this.entries.delete(key);
      this.savedFaceStates.delete(key);
    }
  }

  /**
   * Dim (sketch mode) or restore (model mode) every pooled face material, with
   * `BodyMaterialLibrary`'s save-the-prior-state discipline reproduced exactly:
   * restore replays what was OBSERVED before dimming rather than hardcoded
   * defaults, which matters more here than it does there — a pooled material's
   * "undimmed" transparency is data (`geometry_opacity`), so resetting to
   * `opacity 1` would silently turn every translucent material solid.
   */
  setDimmed(dimmed: boolean): void {
    if (this.dimmed === dimmed) return;
    this.dimmed = dimmed;
    for (const [key, entry] of this.entries) {
      if (dimmed) this.applyDim(key, entry.material);
      else this.restoreDim(key, entry.material);
    }
  }

  private applyDim(key: string, face: THREE.MeshStandardMaterial): void {
    this.savedFaceStates.set(key, {
      transparent: face.transparent,
      opacity: face.opacity,
      depthWrite: face.depthWrite,
    });
    face.transparent = true;
    face.opacity = DIM_OPACITY;
    face.needsUpdate = true;
  }

  private restoreDim(key: string, face: THREE.MeshStandardMaterial): void {
    const saved = this.savedFaceStates.get(key);
    if (!saved) return;
    face.transparent = saved.transparent;
    face.opacity = saved.opacity;
    face.depthWrite = saved.depthWrite;
    face.needsUpdate = true;
    this.savedFaceStates.delete(key);
  }

  /**
   * Override the base color of every live AND future material (the model-tool
   * Cut tint), so a pooled body reads as "about to be cut" identically to a
   * library one. COPIED, never retained — `palette.*()` hands out a shared
   * cached instance and holding it would let a tint mutate the palette.
   */
  setFaceColor(c: THREE.Color): void {
    this.faceColor = (this.faceColor ?? new THREE.Color()).copy(c);
    for (const entry of this.entries.values()) entry.material.color.copy(c);
  }

  /** Drop the tint; every material goes back to its OWN params-derived color. */
  resetFaceColor(): void {
    this.faceColor = null;
    for (const entry of this.entries.values()) {
      applyPbrParams(entry.material, entry.params, { vertexColored: entry.vertexColored });
    }
  }

  /**
   * Theme change: DELIBERATE NO-OP, and the one place in the viewport where that
   * is the correct answer to `refreshColors()`.
   *
   * The theming invariant (engine/README.md § Theming) exists because materials
   * built from `palette` hold a value SNAPSHOT. Nothing here reads the palette —
   * every color is document data — so there is nothing to re-read, and a repaint
   * would be busywork. The method exists so this class is a first-class member of
   * `applyTheme`'s consumer list rather than a silent omission from it: an
   * absent method is indistinguishable from a forgotten one.
   *
   * The token-colored part of a pooled body — the fallback baked into unset
   * faces of a vertex-colored twin — lives in the mesh registry's attribute, and
   * `MeshIngest.refreshColors` re-bakes it there.
   */
  refreshColors(): void {
    /* nothing to re-read — see above */
  }

  /** Live entry count. Test/diagnostic seam; the pool is not a public registry. */
  size(): number {
    return this.entries.size;
  }

  dispose(): void {
    for (const entry of this.entries.values()) entry.material.dispose();
    this.entries.clear();
    this.savedFaceStates.clear();
  }
}
