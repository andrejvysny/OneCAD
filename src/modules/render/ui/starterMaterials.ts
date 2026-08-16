/*
 * The P1 STARTER SEED — eight materials a document can be given without
 * authoring one first.
 *
 * These are TEMPLATES, not library entries: a `MaterialDef` minus its id. Ids
 * are minted per document by `createMaterial` when the user adds one, so two
 * documents that both start from "Steel" hold two independent materials and
 * editing one never reaches the other. Nothing here is persisted until the user
 * adds it — an untouched document grows no render slice (ADR-0004's
 * skip-if-empty rule).
 *
 * TIER-1/2 PARAMETERS ONLY (base + specular + transmission + coat). Every value
 * is a plausible starting point a user is expected to edit, not a measured
 * spectrum: this seed exists so the assignment UX has something to assign, and
 * it is SUPERSEDED by P3's curated library (measured metals, a real glass, the
 * gray-out editor's full parameter set). Deliberately no `emission`, `fuzz`,
 * `subsurface` or `thinFilm` — a starter that lights the scene or scatters
 * light is a surprise, not a default.
 *
 * Sparse by construction: a field absent from a layer means "OpenPBR spec
 * default" (`model/material.ts`), so only what actually differs is written.
 */
import type { MaterialDef } from "../model/material";

/** A library entry waiting for an id. `createMaterial(name, layers)` mints one. */
export type MaterialTemplate = Omit<MaterialDef, "id">;

export const STARTER_MATERIALS: readonly MaterialTemplate[] = [
  {
    name: "Steel",
    base: { base_metalness: 1, base_color: [0.62, 0.63, 0.65] },
    specular: { specular_roughness: 0.35 },
  },
  {
    name: "Aluminum",
    base: { base_metalness: 1, base_color: [0.91, 0.92, 0.92] },
    specular: { specular_roughness: 0.45 },
  },
  {
    name: "Brass",
    base: { base_metalness: 1, base_color: [0.85, 0.68, 0.3] },
    specular: { specular_roughness: 0.3 },
  },
  {
    name: "Copper",
    base: { base_metalness: 1, base_color: [0.95, 0.54, 0.42] },
    specular: { specular_roughness: 0.35 },
  },
  {
    name: "ABS Plastic",
    base: { base_metalness: 0, base_color: [0.85, 0.85, 0.87] },
    specular: { specular_roughness: 0.4 },
  },
  {
    // The one starter with a transmission lobe: `specular_ior` is stated
    // explicitly even though 1.5 IS the spec default, because glass is the
    // material whose ior a user reaches for first and an absent field would
    // read as "this build has no opinion".
    name: "Clear Glass",
    base: { base_color: [1, 1, 1] },
    specular: { specular_roughness: 0.05, specular_ior: 1.5 },
    transmission: { transmission_weight: 1 },
  },
  {
    name: "Rubber",
    base: { base_metalness: 0, base_color: [0.12, 0.12, 0.13] },
    specular: { specular_roughness: 0.9 },
  },
  {
    // Automotive paint shape: a rough coloured metal flake under a smooth clear
    // coat. The coat is what makes it read as painted rather than as bare metal.
    name: "Coated Metal",
    base: { base_metalness: 1, base_color: [0.35, 0.08, 0.08] },
    specular: { specular_roughness: 0.5 },
    coat: { coat_weight: 1, coat_roughness: 0.15 },
  },
];
