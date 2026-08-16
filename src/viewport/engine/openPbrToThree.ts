/*
 * OpenPBR parameters → a three.js material. Pure, unit-tested, no palette read.
 *
 * ── Two tiers, one decision ─────────────────────────────────────────────────
 * `MeshStandardMaterial` is a metallic-roughness BRDF and nothing else;
 * `MeshPhysicalMaterial` extends it with the lobes OpenPBR layers on top (coat,
 * transmission, fuzz, thin film, anisotropy, a non-default specular). The
 * physical shader is materially more expensive — transmission in particular
 * makes three allocate a scene render target and re-render the scene behind the
 * object — so {@link materialTier} promotes ONLY when a parameter that a
 * standard material cannot express is actually in play. A default material, and
 * every plain metal/plastic, stays on the cheap program.
 *
 * ── What is stored but NOT rendered ─────────────────────────────────────────
 * `subsurface_*`, `base_diffuse_roughness`, `coat_ior`/`coat_color`/
 * `coat_darkening`, a `transmission_depth` with no `transmission_weight`, and
 * `geometry_thin_walled` have no three equivalent this mapping is willing to
 * fake. They round-trip through the document untouched and are simply not shown
 * here — which is why NONE of them forces a promotion: paying for the physical
 * shader to render nothing extra would be a pure loss.
 *
 * ── Transmission stays LIVE ─────────────────────────────────────────────────
 * The modeling viewport does not disable it. The render-target cost is only
 * incurred by an object whose material actually has `transmission > 0`, i.e.
 * only once a user assigns a transmissive material to a body — so the common
 * case pays nothing, and the case that pays asked for glass and gets glass.
 */
import * as THREE from "three";
import type { PbrColor3, PbrMaterialParams } from "./pbrParams";

/**
 * Divisor turning OpenPBR's `emission_luminance` (nits) into three's unitless
 * `emissiveIntensity`. three has no photometric unit, so any mapping is a
 * convention; 1000 nits ⇒ 1.0 puts a bright monitor-white emitter at "full
 * albedo" and keeps typical authored values in a usable range. PROVISIONAL —
 * the real exposure/units pipeline is a P5 backend concern, and this constant
 * is the single place it will be re-pinned.
 */
export const EMISSION_LUMINANCE_SCALE = 1000;

/**
 * three clamps `MeshPhysicalMaterial.ior` to `[1, 2.333]` (the range its
 * Fresnel approximation is valid over) while OpenPBR/`resolveMaterial` allow up
 * to 4. Clamping here rather than letting three do it silently keeps the
 * rendered value inspectable and the test explicit.
 */
const IOR_MIN = 1;
const IOR_MAX = 2.333;

/** Thin-film thickness is authored in µm (OpenPBR); three wants nanometres. */
const MICRONS_TO_NM = 1000;

/** Multiply identity — a vertex-colored material's base MUST be white. */
const VERTEX_COLOR_BASE: PbrColor3 = [1, 1, 1];

export type MaterialTier = "standard" | "physical";

function isDefaultSpecular(p: PbrMaterialParams): boolean {
  const [r, g, b] = p.specular_color;
  return p.specular_weight === 1 && r === 1 && g === 1 && b === 1 && p.specular_ior === 1.5;
}

/**
 * Which three material class can render `p` faithfully.
 *
 * `standard` requires every extra lobe to be OFF *and* the specular parameters
 * to be exactly at spec default — a `MeshStandardMaterial` hardcodes F0 = 0.04
 * (ior 1.5, white, full weight), so any deviation there is already unrenderable
 * on it.
 */
export function materialTier(p: PbrMaterialParams): MaterialTier {
  const flat =
    p.coat_weight === 0 &&
    p.transmission_weight === 0 &&
    p.fuzz_weight === 0 &&
    p.thin_film_weight === 0 &&
    p.specular_roughness_anisotropy === 0;
  return flat && isDefaultSpecular(p) ? "standard" : "physical";
}

function setColor(target: THREE.Color, c: PbrColor3): void {
  // No colorSpace argument: OpenPBR colors are LINEAR-light, which is already
  // three's working space. Converting here would double-decode them.
  target.setRGB(c[0], c[1], c[2]);
}

const clamp = (v: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, v));

/**
 * Write `p` onto `mat`. Tier-1 parameters land on any `MeshStandardMaterial`;
 * the rest are applied only when `mat` is actually a `MeshPhysicalMaterial`, so
 * calling this with a standard material and promoted params is safe (it renders
 * the substrate) rather than a crash.
 *
 * `vertexColored` selects multiply identity for the base color: a vertex-color
 * attribute MULTIPLIES `diffuse`, so a twin that reads baked per-face colors
 * must be white or it would tint every one of them.
 */
export function applyPbrParams(
  mat: THREE.MeshStandardMaterial,
  p: PbrMaterialParams,
  opts: { vertexColored: boolean },
): void {
  setColor(mat.color, opts.vertexColored ? VERTEX_COLOR_BASE : p.base_color);
  mat.metalness = p.base_metalness;
  mat.roughness = p.specular_roughness;
  setColor(mat.emissive, p.emission_color);
  mat.emissiveIntensity = p.emission_luminance / EMISSION_LUMINANCE_SCALE;
  mat.opacity = p.geometry_opacity;
  // `transparent` gates the render pass, so it must follow the opacity rather
  // than be left on: an always-transparent body would leave the opaque pass and
  // stop depth-sorting against the rest of the scene.
  mat.transparent = p.geometry_opacity < 1;

  const phys = mat as THREE.MeshPhysicalMaterial;
  if (!phys.isMeshPhysicalMaterial) return;

  phys.ior = clamp(p.specular_ior, IOR_MIN, IOR_MAX);
  phys.specularIntensity = Math.min(p.specular_weight, 1);
  setColor(phys.specularColor, p.specular_color);

  phys.clearcoat = p.coat_weight;
  phys.clearcoatRoughness = p.coat_roughness;

  phys.transmission = p.transmission_weight;
  setColor(phys.attenuationColor, p.transmission_color);
  // Infinity is three's own "no attenuation" default. A depth of 0 means
  // "unset" in OpenPBR, and feeding 0 through Beer-Lambert absorbs everything —
  // untinted glass would render as black glass.
  phys.attenuationDistance = p.transmission_depth > 0 ? p.transmission_depth : Infinity;

  phys.sheen = p.fuzz_weight;
  setColor(phys.sheenColor, p.fuzz_color);
  phys.sheenRoughness = p.fuzz_roughness;

  phys.iridescence = p.thin_film_weight;
  phys.iridescenceIOR = p.thin_film_ior;
  // A single authored thickness, not a range: OpenPBR's film is uniform, while
  // three interpolates across [min, max] via a thickness map it has no source
  // for here. Equal bounds collapse that to the one value.
  const nm = p.thin_film_thickness * MICRONS_TO_NM;
  phys.iridescenceThicknessRange = [nm, nm];

  phys.anisotropy = p.specular_roughness_anisotropy;
}
