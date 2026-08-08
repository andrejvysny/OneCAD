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
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { palette } from "./palette";
import { cssLineWidth } from "./SketchObject";
import { MATERIAL_KIND_VERTEX_COLORS, type MaterialKind } from "./renderModes";

/**
 * Body edge weight, in the DEVICE px a fat line's `linewidth` is measured in
 * (see SketchObject.cssLineWidth). Exported because the Picker's `Line2`
 * threshold is derived from it — the raycast's hit radius is
 * `(linewidth + threshold) / 2`, so a pick tolerance expressed in CSS px has to
 * subtract the drawn width. One constant, two consumers, no drift.
 */
export const BODY_EDGE_WIDTH = cssLineWidth(1.25);

export interface BodyMaterialSet {
  face: THREE.MeshStandardMaterial;
  /** Edges drawn OVER a shaded face (`edgeStyle: "onFaces"`) — near-black outline. */
  edge: LineMaterial;
  /** Edges drawn ALONE (`edgeStyle: "standalone"`, wireframe) — inverts per theme. */
  edgeWire: LineMaterial;
}

/** Face material state saved just before dimming, reapplied verbatim on restore. */
interface SavedFaceState {
  transparent: boolean;
  opacity: number;
  depthWrite: boolean;
}

/** Sketch-mode dim opacity (focus cue: the body must not compete with the sketch). */
const DIM_OPACITY = 0.35;

/**
 * Base color of a vertex-colored face material. Three multiplies `diffuse` by
 * the per-vertex color, so white is the only value that reproduces an authored
 * (imported) color unchanged. NOT a design token — it is multiply identity.
 */
const VERTEX_COLOR_BASE = new THREE.Color(1, 1, 1);

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
      set = this.create(kind);
      this.sets.set(kind, set);
      // A set born while the library is dimmed still needs the dim; its saved
      // prior is its constructor defaults, so undimming restores it correctly.
      if (this.dimmed) this.applyDim(kind, set);
    }
    return set;
  }

  private create(kind: MaterialKind): BodyMaterialSet {
    const face = new THREE.MeshStandardMaterial({
      color: this.baseColor(kind),
      // Colored (imported) bodies bake FACE_COLORS into a per-vertex attribute;
      // the shading params below are otherwise identical, so the two kinds stay
      // visually one material with one of them reading an extra attribute.
      vertexColors: MATERIAL_KIND_VERTEX_COLORS[kind],
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
    //
    // Fat lines (LineSegments2/LineMaterial): WebGL ignores
    // LineBasicMaterial.linewidth, so a plain line is stuck at 1px and a body
    // reads as a wash of hairlines. NOT `transparent` — body edges belong in the
    // OPAQUE pass exactly as they always have; a transparent edge would be
    // painted after every opaque body and stop depth-sorting against them.
    const edge = new LineMaterial({
      color: palette.bodyEdge().getHex(),
      linewidth: BODY_EDGE_WIDTH,
      toneMapped: false,
    });
    const edgeWire = new LineMaterial({
      color: palette.bodyEdgeWire().getHex(),
      linewidth: BODY_EDGE_WIDTH,
      toneMapped: false,
    });
    return { face, edge, edgeWire };
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
   * is a state, not a theme), while the edge colors — which nothing else ever
   * updates — are re-read unconditionally.
   */
  refreshColors(): void {
    this.applyPaletteColors();
  }

  /**
   * BOTH edge materials are refreshed, including the one the current render
   * mode is not using. A mode SWAPS which material the edges point at rather
   * than recoloring one, so an unrefreshed inactive material looks stale the
   * instant the user switches modes — the same trap as DragHandle's
   * matNormal/matHover pair (engine/README.md § Theming).
   */
  private applyPaletteColors(): void {
    for (const [kind, set] of this.sets) {
      set.face.color.copy(this.baseColor(kind));
      set.edge.color.copy(palette.bodyEdge());
      set.edgeWire.color.copy(palette.bodyEdgeWire());
    }
  }

  /**
   * The face base color for a kind. A Cut tint outranks everything (it is state,
   * and tinting a colored body is exactly what the Cut preview should look
   * like); otherwise a vertex-colored kind takes multiply identity and a plain
   * one takes the body token.
   */
  private baseColor(kind: MaterialKind): THREE.Color {
    if (this.faceColor) return this.faceColor;
    return MATERIAL_KIND_VERTEX_COLORS[kind] ? VERTEX_COLOR_BASE : palette.bodyNeutral();
  }

  dispose(): void {
    for (const set of this.sets.values()) {
      set.face.dispose();
      set.edge.dispose();
      set.edgeWire.dispose();
    }
    this.sets.clear();
    this.savedFaceStates.clear();
  }
}
