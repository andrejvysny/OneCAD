/*
 * PBR material parameters + the seam the viewport reads them through.
 *
 * VIEWPORT-OWNED, deliberately. `src/viewport/**` must never import from
 * `src/modules/**` (docs/ARCHITECTURE.md), so the shading parameter set the
 * viewport renders is declared HERE and the render module's `ResolvedMaterial`
 * satisfies it STRUCTURALLY — no nominal dependency in either direction, and no
 * duplicated defaulting/clamping logic (that stays `resolveMaterial`'s job; by
 * the time params reach the viewport every field is already filled and clamped).
 *
 * The field set mirrors OpenPBR's flat resolved form in full, including the
 * parameters `openPbrToThree.ts` deliberately does NOT render (subsurface,
 * `base_diffuse_roughness`, the coat's ior/color/darkening, `geometry_thin_walled`).
 * Carrying them costs nothing, keeps the structural match exact, and means a
 * later tier can start rendering one without touching this interface.
 *
 * Every field is `readonly`: the viewport is a CONSUMER of shading state. A
 * mutable tuple is assignable to a readonly one, so a `ResolvedMaterial` still
 * matches.
 */

/** Linear-light RGB. Not sRGB, and never a hex literal — see `model/material.ts`. */
export type PbrColor3 = readonly [number, number, number];

/** The OpenPBR parameter set, flat, defaulted and clamped by whoever resolved it. */
export interface PbrMaterialParams {
  readonly base_color: PbrColor3;
  readonly base_metalness: number;
  readonly base_diffuse_roughness: number;

  readonly specular_weight: number;
  readonly specular_color: PbrColor3;
  readonly specular_roughness: number;
  readonly specular_roughness_anisotropy: number;
  readonly specular_ior: number;

  readonly transmission_weight: number;
  readonly transmission_color: PbrColor3;
  readonly transmission_depth: number;

  readonly coat_weight: number;
  readonly coat_roughness: number;
  readonly coat_ior: number;
  readonly coat_color: PbrColor3;
  readonly coat_darkening: number;

  readonly fuzz_weight: number;
  readonly fuzz_color: PbrColor3;
  readonly fuzz_roughness: number;

  readonly emission_color: PbrColor3;
  readonly emission_luminance: number;

  readonly thin_film_weight: number;
  readonly thin_film_thickness: number;
  readonly thin_film_ior: number;

  readonly subsurface_weight: number;
  readonly subsurface_color: PbrColor3;
  readonly subsurface_radius: number;

  readonly geometry_opacity: number;
  readonly geometry_thin_walled: boolean;
}

/**
 * Everything the mesh-ingest lane needs in order to shade a body with an
 * assigned material, and the one thing it reports back.
 *
 * Kept to five methods on purpose: this is the whole surface an outside owner
 * of appearance data has to implement, and every method is a synchronous read
 * of already-parsed state — the ingest lane calls them on a repaint path.
 */
export interface BodyMaterialSource {
  /**
   * A key that is EQUAL for two bodies wearing interchangeable shading state and
   * different otherwise — the pool caches on it, so it must already encode the
   * resolved parameters. `null` = no material assigned; the body then keeps the
   * shared `BodyMaterialLibrary` look, unchanged.
   */
  poolKeyForBody(bodyId: string): string | null;
  /** The body's resolved parameters. `null` whenever {@link poolKeyForBody} is. */
  paramsForBody(bodyId: string): PbrMaterialParams | null;
  /**
   * `faceId → base_color` for those of `faceElementIds` a per-face material
   * override names. Only the BASE COLOR crosses this seam: the modeling view
   * draws one program per body (V1), so an override can move a face's albedo
   * but not its roughness or its lobes.
   */
  faceOverrideBaseColors(
    bodyId: string,
    faceElementIds: readonly string[],
  ): Record<string, [number, number, number]>;
  /**
   * Session evidence, pushed back the other way: which of the body's overrides
   * the CURRENT tessellation actually has a face for, and which it does not.
   * Only the viewport knows this — the persisted override map is face-keyed and
   * global, so body attribution exists nowhere else.
   */
  reportFaceOverrideBindings(
    bodyId: string,
    bound: readonly string[],
    unbound: readonly string[],
  ): void;
  /** Fires when any answer above may have changed. Returns the unsubscribe. */
  subscribe(cb: () => void): () => void;
}
