/*
 * OpenPBR → three mapping: the tier decision and the parameter translation.
 *
 * The tier matrix is the load-bearing half. Promoting too eagerly costs every
 * body the physical shader (and, with transmission, a scene render target);
 * promoting too late silently drops a lobe the user authored and paid for. Each
 * case below turns exactly ONE parameter and asserts which way it goes.
 */
import { describe, it, expect } from "vitest";
import * as THREE from "three";
import { applyPbrParams, materialTier, EMISSION_LUMINANCE_SCALE } from "./openPbrToThree";
import type { PbrMaterialParams } from "./pbrParams";

/**
 * OpenPBR v1.1.1 spec defaults, written out LOCALLY rather than imported from
 * the render module: `src/viewport/**` must not depend on `src/modules/**`, and
 * a test is not an exemption from that. The module's own `material.test.ts`
 * pins these values against the spec; the structural match between
 * `ResolvedMaterial` and `PbrMaterialParams` is asserted from the module side,
 * in `modules/render/viewportBridge.test.ts`.
 */
const DEFAULT_PARAMS: PbrMaterialParams = {
  base_color: [0.8, 0.8, 0.8],
  base_metalness: 0,
  base_diffuse_roughness: 0,
  specular_weight: 1,
  specular_color: [1, 1, 1],
  specular_roughness: 0.3,
  specular_roughness_anisotropy: 0,
  specular_ior: 1.5,
  transmission_weight: 0,
  transmission_color: [1, 1, 1],
  transmission_depth: 0,
  coat_weight: 0,
  coat_roughness: 0,
  coat_ior: 1.6,
  coat_color: [1, 1, 1],
  coat_darkening: 1,
  fuzz_weight: 0,
  fuzz_color: [1, 1, 1],
  fuzz_roughness: 0.5,
  emission_color: [1, 1, 1],
  emission_luminance: 0,
  thin_film_weight: 0,
  thin_film_thickness: 0.5,
  thin_film_ior: 1.4,
  subsurface_weight: 0,
  subsurface_color: [0.8, 0.8, 0.8],
  subsurface_radius: 1,
  geometry_opacity: 1,
  geometry_thin_walled: false,
};

/** Spec-default parameters — the shape every case below perturbs by one field. */
const defaults = (): PbrMaterialParams => ({ ...DEFAULT_PARAMS });

const withParams = (over: Partial<PbrMaterialParams>): PbrMaterialParams => ({
  ...DEFAULT_PARAMS,
  ...over,
});

describe("materialTier", () => {
  it("keeps a spec-default material on the cheap standard program", () => {
    expect(materialTier(defaults())).toBe("standard");
  });

  it("keeps plain metalness/roughness/emission/opacity edits on standard", () => {
    expect(
      materialTier(
        withParams({
          base_color: [0.1, 0.2, 0.3],
          base_metalness: 1,
          specular_roughness: 0.9,
          emission_luminance: 500,
          geometry_opacity: 0.4,
        }),
      ),
    ).toBe("standard");
  });

  it.each([
    ["coat_weight", { coat_weight: 0.5 }],
    ["transmission_weight", { transmission_weight: 0.2 }],
    ["fuzz_weight", { fuzz_weight: 0.3 }],
    ["thin_film_weight", { thin_film_weight: 1 }],
    ["specular_roughness_anisotropy", { specular_roughness_anisotropy: 0.4 }],
    ["specular_weight", { specular_weight: 0.5 }],
    ["specular_color", { specular_color: [1, 0.5, 0.5] as [number, number, number] }],
    ["specular_ior", { specular_ior: 1.8 }],
  ])("promotes to physical for %s", (_name, over) => {
    expect(materialTier(withParams(over))).toBe("physical");
  });

  /*
   * The stored-but-unrendered set. None of these has a three equivalent this
   * mapping is willing to fake, so paying for the physical shader would buy
   * exactly nothing — the promotion rule must ignore them.
   */
  it.each([
    ["subsurface_weight", { subsurface_weight: 1 }],
    ["subsurface_radius", { subsurface_radius: 5 }],
    ["base_diffuse_roughness", { base_diffuse_roughness: 0.8 }],
    ["coat_ior", { coat_ior: 2 }],
    ["coat_color", { coat_color: [0.2, 0.2, 0.9] as [number, number, number] }],
    ["coat_darkening", { coat_darkening: 0.3 }],
    ["geometry_thin_walled", { geometry_thin_walled: true }],
    // Depth with no WEIGHT is inert: there is no transmission for it to attenuate.
    ["transmission_depth without weight", { transmission_depth: 12 }],
  ])("does NOT promote for %s (stored, not rendered)", (_name, over) => {
    expect(materialTier(withParams(over))).toBe("standard");
  });
});

describe("applyPbrParams — tier 1", () => {
  it("maps the substrate onto a MeshStandardMaterial", () => {
    const mat = new THREE.MeshStandardMaterial();
    applyPbrParams(
      mat,
      withParams({
        base_color: [0.2, 0.4, 0.6],
        base_metalness: 0.75,
        specular_roughness: 0.25,
      }),
      { vertexColored: false },
    );
    // Linear in, linear out — an sRGB decode here would darken every material.
    expect([mat.color.r, mat.color.g, mat.color.b]).toEqual([0.2, 0.4, 0.6]);
    expect(mat.metalness).toBe(0.75);
    expect(mat.roughness).toBe(0.25);
  });

  it("takes a WHITE base for the vertex-colored twin (multiply identity)", () => {
    const mat = new THREE.MeshStandardMaterial();
    applyPbrParams(mat, withParams({ base_color: [0.2, 0.4, 0.6] }), { vertexColored: true });
    expect(mat.color.getHex()).toBe(new THREE.Color(1, 1, 1).getHex());
  });

  it("scales emission_luminance (nits) into three's unitless intensity", () => {
    const mat = new THREE.MeshStandardMaterial();
    applyPbrParams(
      mat,
      withParams({ emission_color: [1, 0.5, 0], emission_luminance: 2500 }),
      { vertexColored: false },
    );
    expect(mat.emissiveIntensity).toBe(2500 / EMISSION_LUMINANCE_SCALE);
    expect([mat.emissive.r, mat.emissive.g, mat.emissive.b]).toEqual([1, 0.5, 0]);
  });

  it("only marks a material transparent when it is actually translucent", () => {
    const opaque = new THREE.MeshStandardMaterial();
    applyPbrParams(opaque, defaults(), { vertexColored: false });
    expect(opaque.opacity).toBe(1);
    // An always-transparent body would leave the opaque pass and stop
    // depth-sorting against the rest of the scene.
    expect(opaque.transparent).toBe(false);

    const glassy = new THREE.MeshStandardMaterial();
    applyPbrParams(glassy, withParams({ geometry_opacity: 0.3 }), { vertexColored: false });
    expect(glassy.opacity).toBe(0.3);
    expect(glassy.transparent).toBe(true);
  });

  it("leaves physical-only params alone when handed a standard material", () => {
    const mat = new THREE.MeshStandardMaterial();
    // Promoted params on an unpromoted material: renders the substrate, never throws.
    expect(() =>
      applyPbrParams(mat, withParams({ coat_weight: 1, transmission_weight: 1 }), {
        vertexColored: false,
      }),
    ).not.toThrow();
    expect(mat.roughness).toBe(DEFAULT_PARAMS.specular_roughness);
  });
});

describe("applyPbrParams — tier 2/3 (MeshPhysicalMaterial)", () => {
  it("maps coat, sheen and iridescence, converting µm → nm", () => {
    const mat = new THREE.MeshPhysicalMaterial();
    applyPbrParams(
      mat,
      withParams({
        coat_weight: 0.8,
        coat_roughness: 0.15,
        fuzz_weight: 0.6,
        fuzz_color: [0.9, 0.8, 0.7],
        fuzz_roughness: 0.4,
        thin_film_weight: 0.5,
        thin_film_ior: 2,
        thin_film_thickness: 0.35, // µm
        specular_roughness_anisotropy: 0.7,
      }),
      { vertexColored: false },
    );
    expect([mat.clearcoat, mat.clearcoatRoughness]).toEqual([0.8, 0.15]);
    expect([mat.sheen, mat.sheenRoughness]).toEqual([0.6, 0.4]);
    expect([mat.sheenColor.r, mat.sheenColor.g, mat.sheenColor.b]).toEqual([0.9, 0.8, 0.7]);
    expect([mat.iridescence, mat.iridescenceIOR]).toEqual([0.5, 2]);
    // 0.35 µm is 350 nm — a value in µm would read as sub-nanometre film.
    expect(mat.iridescenceThicknessRange).toEqual([350, 350]);
    expect(mat.anisotropy).toBe(0.7);
  });

  it("clamps ior into the range three's Fresnel approximation is valid over", () => {
    const mat = new THREE.MeshPhysicalMaterial();
    // resolveMaterial allows up to 4; three's shader does not.
    applyPbrParams(mat, withParams({ specular_ior: 4 }), { vertexColored: false });
    expect(mat.ior).toBe(2.333);

    applyPbrParams(mat, withParams({ specular_ior: 1.45 }), { vertexColored: false });
    expect(mat.ior).toBe(1.45);
  });

  it("clamps specular_weight into specularIntensity's unit range", () => {
    const mat = new THREE.MeshPhysicalMaterial();
    applyPbrParams(mat, withParams({ specular_weight: 1, specular_color: [1, 0.2, 0.2] }), {
      vertexColored: false,
    });
    expect(mat.specularIntensity).toBe(1);
    expect([mat.specularColor.r, mat.specularColor.g, mat.specularColor.b]).toEqual([1, 0.2, 0.2]);
  });

  /*
   * OpenPBR's `transmission_depth: 0` means "unset". Feeding 0 straight into
   * three's Beer-Lambert attenuation absorbs everything, so untinted glass would
   * render as BLACK glass — Infinity is three's own "no attenuation" default.
   */
  it("turns a zero transmission_depth into Infinity, not into black glass", () => {
    const mat = new THREE.MeshPhysicalMaterial();
    applyPbrParams(
      mat,
      withParams({ transmission_weight: 1, transmission_color: [0.8, 0.9, 1] }),
      { vertexColored: false },
    );
    expect(mat.transmission).toBe(1);
    expect(mat.attenuationDistance).toBe(Infinity);
    expect([mat.attenuationColor.r, mat.attenuationColor.g, mat.attenuationColor.b]).toEqual([
      0.8, 0.9, 1,
    ]);

    applyPbrParams(mat, withParams({ transmission_weight: 1, transmission_depth: 25 }), {
      vertexColored: false,
    });
    expect(mat.attenuationDistance).toBe(25);
  });
});
