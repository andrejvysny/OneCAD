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
  /** Per-body material sets for the assembly-colors mode. */
  private readonly assemblySets = new Map<string, BodyMaterialSet>();
  private dimmed = false;
  /** Face color override (Cut tint); `null` = the palette default. Never a borrowed instance. */
  private faceColor: THREE.Color | null = null;
  /** Section-view clipping planes; `null` = unclipped. See {@link setClippingPlanes}. */
  private clippingPlanes: THREE.Plane[] | null = null;

  /** The shared material set for `kind`, created on first use. */
  get(kind: MaterialKind): BodyMaterialSet {
    let set = this.sets.get(kind);
    if (!set) {
      set = this.create(kind);
      this.sets.set(kind, set);
      // A set born while the library is dimmed still needs the dim; its saved
      // prior is its constructor defaults, so undimming restores it correctly.
      if (this.dimmed) this.applyDim(kind, set);
      applyClipping(set, this.clippingPlanes);
    }
    return set;
  }

  /**
   * Material set for a specific body in `assemblyColor` mode. Colors are
   * deterministic from `bodyId` so they are stable across reloads and sessions,
   * and the set is cached per body so repeated calls return the same instance.
   */
  getAssemblyColor(bodyId: string): BodyMaterialSet {
    let set = this.assemblySets.get(bodyId);
    if (!set) {
      set = this.create("assemblyColor");
      set.face.color.copy(assemblyColorForBody(bodyId));
      this.assemblySets.set(bodyId, set);
      if (this.dimmed) this.applyDim("assemblyColor", set);
      applyClipping(set, this.clippingPlanes);
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
   * Section view: clip every face and BOTH edge materials against `planes`
   * (`null` = unclipped). One write per material — the shared-material
   * discipline is exactly what makes a whole-document clip this cheap.
   *
   * The planes are RETAINED, not copied, on purpose: `SectionLayer` mutates the
   * single `THREE.Plane` in place as the offset slider moves, and three reads
   * `material.clippingPlanes` per frame, so a live reference is what makes a
   * drag repaint without touching a material at all — see `applyClipping` for
   * the other half of that (no `needsUpdate` unless the plane COUNT moves).
   *
   * Applied to future sets too (see `get` / `getAssemblyColor`): a render-mode
   * switch or an assembly-colored body arriving AFTER the section was enabled
   * builds its set here, and an unclipped set born late is exactly the kind of
   * silent half-applied state the dim path already guards against.
   */
  setClippingPlanes(planes: THREE.Plane[] | null): void {
    this.clippingPlanes = planes;
    for (const set of this.sets.values()) applyClipping(set, planes);
    for (const set of this.assemblySets.values()) applyClipping(set, planes);
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
    // Assembly-color sets keep their per-body face color; only the edges follow
    // the theme, exactly like the shared sets.
    for (const set of this.assemblySets.values()) {
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
    for (const set of this.assemblySets.values()) {
      set.face.dispose();
      set.edge.dispose();
      set.edgeWire.dispose();
    }
    this.assemblySets.clear();
    this.savedFaceStates.clear();
  }
}

/**
 * Point every material of `set` at `planes` (or clear them with `null`).
 *
 * `needsUpdate` is set ONLY when the plane COUNT changes, because the count is
 * all the shader bakes in — a plane's normal and constant are uniforms read per
 * frame. This is what keeps an offset drag free of material writes: the slider
 * mutates the one plane in place and re-runs this for every set, and an
 * unconditional `needsUpdate` would recompile every body material on every tick.
 */
function applyClipping(set: BodyMaterialSet, planes: THREE.Plane[] | null): void {
  const after = planes?.length ?? 0;
  for (const mat of [set.face, set.edge, set.edgeWire] as THREE.Material[]) {
    const before = mat.clippingPlanes?.length ?? 0;
    mat.clippingPlanes = planes;
    if (before !== after) mat.needsUpdate = true;
  }
}

/**
 * Deterministic, visually-distinct color for a body id. Uses a hash to spread
 * body ids evenly around the hue wheel, then fixes saturation and lightness to
 * the pastel/mid-tone range common in CAD assembly views.
 */
function assemblyColorForBody(bodyId: string): THREE.Color {
  const c = new THREE.Color();
  c.setHSL(hashHue(bodyId), 0.58, 0.62);
  return c;
}

/** Hash a string to a hue in [0, 1). */
function hashHue(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  // Mix the low bits so similar UUIDs (same prefix) still spread well.
  h = (h ^ (h >>> 16)) >>> 0;
  return (h % 1000) / 1000;
}
