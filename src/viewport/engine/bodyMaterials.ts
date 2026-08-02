/*
 * BodyMaterialLibrary — the shared face + edge materials every BodyObject draws
 * with, keyed by {@link MaterialKind}.
 *
 * SHARED-MATERIAL DISCIPLINE: `get` hands back the SAME instance for a kind
 * every time, so N bodies cost one material pair and a library-wide state change
 * (dim, tint) is one write instead of a scene walk. The flip side is that
 * nothing outside this class may mutate a returned material — a per-body change
 * would leak onto every body of that kind. Per-body state belongs on the
 * Object3D (visibility), not on the material.
 *
 * One library instance per owner (MeshIngest owns the committed one, the engine
 * lazily makes a second for previews), which is what lets a Cut tint recolor
 * previews without touching committed geometry.
 */
import * as THREE from "three";
import { palette } from "./palette";
import type { MaterialKind } from "./renderModes";

export interface BodyMaterialSet {
  face: THREE.MeshStandardMaterial;
  edge: THREE.LineBasicMaterial;
}

/** Face material state saved just before dimming, reapplied verbatim on restore. */
interface SavedFaceState {
  transparent: boolean;
  opacity: number;
  depthWrite: boolean;
}

/** Sketch-mode dim opacity (focus cue: the body must not compete with the sketch). */
const DIM_OPACITY = 0.35;

export class BodyMaterialLibrary {
  private readonly sets = new Map<MaterialKind, BodyMaterialSet>();
  private readonly savedFaceStates = new Map<MaterialKind, SavedFaceState>();
  private dimmed = false;
  /** Face color override (Cut tint); `null` = the palette default. Never a borrowed instance. */
  private faceColor: THREE.Color | null = null;

  /** The shared material set for `kind`, created on first use. */
  get(kind: MaterialKind): BodyMaterialSet {
    let set = this.sets.get(kind);
    if (!set) {
      set = this.create();
      this.sets.set(kind, set);
      // A set born while the library is dimmed still needs the dim; its saved
      // prior is its constructor defaults, so undimming restores it correctly.
      if (this.dimmed) this.applyDim(kind, set);
    }
    return set;
  }

  private create(): BodyMaterialSet {
    const face = new THREE.MeshStandardMaterial({
      color: this.faceColor ?? palette.bodyNeutral(),
      // Machined-surface dielectric: rough enough to stay matte, smooth enough
      // that the environment map produces a readable gradient across a face.
      // At 0.75 the specular lobe is so wide the IBL contributes nothing.
      metalness: 0.0,
      roughness: 0.5,
      envMapIntensity: 1.0, // scene.environmentIntensity already sets the level
      side: THREE.DoubleSide, // closed solids; robust picking regardless of winding
      polygonOffset: true, // edge lines sit on top of faces without z-fighting
      polygonOffsetFactor: 1,
      polygonOffsetUnits: 1,
    });
    // Edges are an annotation, not a lit surface: tone mapping would lift the
    // near-black token off its design value. Only lit body faces get tone-mapped.
    const edge = new THREE.LineBasicMaterial({ color: palette.bodyEdge(), toneMapped: false });
    return { face, edge };
  }

  /**
   * Dim (sketch mode) or restore (model mode) every face material — a focus cue
   * so bodies aren't visually competing with the sketch on top of them. Edge
   * materials are left untouched so silhouettes stay crisp. Saves each
   * material's PRIOR state before dimming and reapplies it verbatim on restore
   * rather than hardcoding reset values, so a future change to the default face
   * styling can't drift out of sync with what "restore" means here.
   */
  setDimmed(dimmed: boolean): void {
    if (this.dimmed === dimmed) return;
    this.dimmed = dimmed;
    for (const [kind, set] of this.sets) {
      if (dimmed) this.applyDim(kind, set);
      else this.restoreDim(kind, set);
    }
  }

  private applyDim(kind: MaterialKind, set: BodyMaterialSet): void {
    const face = set.face;
    this.savedFaceStates.set(kind, {
      transparent: face.transparent,
      opacity: face.opacity,
      depthWrite: face.depthWrite,
    });
    face.transparent = true;
    face.opacity = DIM_OPACITY;
    face.needsUpdate = true;
  }

  private restoreDim(kind: MaterialKind, set: BodyMaterialSet): void {
    const saved = this.savedFaceStates.get(kind);
    if (!saved) return;
    const face = set.face;
    face.transparent = saved.transparent;
    face.opacity = saved.opacity;
    face.depthWrite = saved.depthWrite;
    face.needsUpdate = true;
    this.savedFaceStates.delete(kind);
  }

  /**
   * Override the face color of every live AND future set (the model-tool Cut
   * tint). The color is COPIED, never retained: `palette.*()` returns a shared
   * cached THREE.Color, and holding it would let one tint mutate the palette
   * itself.
   */
  setFaceColor(c: THREE.Color): void {
    this.faceColor = (this.faceColor ?? new THREE.Color()).copy(c);
    for (const set of this.sets.values()) set.face.color.copy(c);
  }

  /** Drop the override and go back to the neutral body token. */
  resetFaceColor(): void {
    this.faceColor = null;
    this.applyPaletteColors();
  }

  /**
   * Theme change: re-read the palette into every live set.
   *
   * Distinct from {@link resetFaceColor}: an active Cut tint is PRESERVED (it
   * is a state, not a theme), while the edge color — which nothing else ever
   * updates — is re-read unconditionally. That edge re-read is load-bearing in
   * wireframe mode, where edges are all that draws.
   */
  refreshColors(): void {
    this.applyPaletteColors();
  }

  private applyPaletteColors(): void {
    for (const set of this.sets.values()) {
      set.face.color.copy(this.faceColor ?? palette.bodyNeutral());
      set.edge.color.copy(palette.bodyEdge());
    }
  }

  dispose(): void {
    for (const set of this.sets.values()) {
      set.face.dispose();
      set.edge.dispose();
    }
    this.sets.clear();
    this.savedFaceStates.clear();
  }
}
