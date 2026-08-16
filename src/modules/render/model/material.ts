/*
 * OpenPBR material data model (docs/adr/0014-render-module-openpbr.md,
 * docs/RENDER_MODULE.md §3). Pure model code — no platform/document-state
 * plumbing lives here, so it is usable before `onecad.render` reads or writes
 * anything.
 *
 * A `MaterialDef` mirrors OpenPBR's layered stack: `base` is mandatory (every
 * material has a substrate), every other layer is an optional sub-object whose
 * ABSENCE means "spec default, lobe off" rather than "zeroed". Fields inside a
 * present layer are themselves optional for the same reason: a def only needs
 * to carry the values an author actually changed. `resolveMaterial` is the one
 * place that turns that sparse record into every parameter, defaulted and
 * range-clamped, for a consumer (the future viewport material pool) that
 * cannot itself know OpenPBR's defaults.
 */

// ── Ids ──────────────────────────────────────────────────────────────────────

/** `mat_<uuid v4>`. Not branded (unlike platform ids, ADR-0008) — this is
 *  document-model data, not a registry key. */
export type MaterialId = string;

const MATERIAL_ID_RE =
  /^mat_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

export function isMaterialId(value: unknown): value is MaterialId {
  return typeof value === "string" && MATERIAL_ID_RE.test(value);
}

/** Real UUID v4 (crypto.randomUUID, with an RFC-4122 fallback for older
 *  jsdom) — same shape as `ipc/sketchWireMap.ts`'s `mintUuid`, reimplemented
 *  locally so this pure model module has no dependency on the IPC layer. */
function mintUuidV4(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === "function") return c.randomUUID();
  const b = c.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = Array.from(b, (x) => x.toString(16).padStart(2, "0"));
  return `${h[0]}${h[1]}${h[2]}${h[3]}-${h[4]}${h[5]}-${h[6]}${h[7]}-${h[8]}${h[9]}-${h[10]}${h[11]}${h[12]}${h[13]}${h[14]}${h[15]}`;
}

export function mintMaterialId(): MaterialId {
  return `mat_${mintUuidV4()}`;
}

// ── Primitive value types ───────────────────────────────────────────────────

/** Linear-light RGB, each channel `0..1` at rest. NEVER a hex string — the
 *  repo greps `src` for hex color literals. Not clamped by `isColor3`/parse:
 *  emission in particular is legitimately allowed to exceed 1 (HDR); range
 *  enforcement is `resolveMaterial`'s job, and it deliberately does not touch
 *  colors (see the clamp tables below). */
export type Color3 = [number, number, number];

export function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function isColor3(value: unknown): value is Color3 {
  return Array.isArray(value) && value.length === 3 && value.every(isFiniteNumber);
}

// ── Layers ───────────────────────────────────────────────────────────────────

export interface BaseLayer {
  base_color?: Color3;
  base_metalness?: number;
  base_diffuse_roughness?: number;
}

export interface SpecularLayer {
  specular_weight?: number;
  specular_color?: Color3;
  specular_roughness?: number;
  specular_roughness_anisotropy?: number;
  specular_ior?: number;
}

export interface TransmissionLayer {
  transmission_weight?: number;
  transmission_color?: Color3;
  transmission_depth?: number;
}

export interface CoatLayer {
  coat_weight?: number;
  coat_roughness?: number;
  coat_ior?: number;
  coat_color?: Color3;
  coat_darkening?: number;
}

export interface FuzzLayer {
  fuzz_weight?: number;
  fuzz_color?: Color3;
  fuzz_roughness?: number;
}

export interface EmissionLayer {
  emission_color?: Color3;
  emission_luminance?: number;
}

export interface ThinFilmLayer {
  thin_film_weight?: number;
  /** Micrometers. */
  thin_film_thickness?: number;
  thin_film_ior?: number;
}

export interface SubsurfaceLayer {
  subsurface_weight?: number;
  subsurface_color?: Color3;
  subsurface_radius?: number;
}

export interface GeometryLayer {
  geometry_opacity?: number;
  geometry_thin_walled?: boolean;
}

export interface MaterialDef {
  id: MaterialId;
  name: string;
  base: BaseLayer;
  specular?: SpecularLayer;
  transmission?: TransmissionLayer;
  coat?: CoatLayer;
  fuzz?: FuzzLayer;
  emission?: EmissionLayer;
  thinFilm?: ThinFilmLayer;
  subsurface?: SubsurfaceLayer;
  geometry?: GeometryLayer;
}

// ── Pinned defaults ──────────────────────────────────────────────────────────

interface OpenPbrDefaults {
  readonly base: Readonly<Required<BaseLayer>>;
  readonly specular: Readonly<Required<SpecularLayer>>;
  readonly transmission: Readonly<Required<TransmissionLayer>>;
  readonly coat: Readonly<Required<CoatLayer>>;
  readonly fuzz: Readonly<Required<FuzzLayer>>;
  readonly emission: Readonly<Required<EmissionLayer>>;
  readonly thinFilm: Readonly<Required<ThinFilmLayer>>;
  readonly subsurface: Readonly<Required<SubsurfaceLayer>>;
  readonly geometry: Readonly<Required<GeometryLayer>>;
}

/**
 * OpenPBR v1.1.1 (2026-04-17) spec defaults — https://academysoftwarefoundation.github.io/OpenPBR/
 * Every value here is cited against that revision in `material.test.ts`, one
 * assertion per field, so a future spec re-pin shows up as a diff instead of a
 * silent drift.
 */
export const OPENPBR_DEFAULTS: OpenPbrDefaults = Object.freeze({
  base: Object.freeze({
    base_color: [0.8, 0.8, 0.8] as Color3,
    base_metalness: 0,
    base_diffuse_roughness: 0,
  }),
  specular: Object.freeze({
    specular_weight: 1,
    specular_color: [1, 1, 1] as Color3,
    specular_roughness: 0.3,
    specular_roughness_anisotropy: 0,
    specular_ior: 1.5,
  }),
  transmission: Object.freeze({
    transmission_weight: 0,
    transmission_color: [1, 1, 1] as Color3,
    transmission_depth: 0,
  }),
  coat: Object.freeze({
    coat_weight: 0,
    coat_roughness: 0,
    coat_ior: 1.6,
    coat_color: [1, 1, 1] as Color3,
    coat_darkening: 1,
  }),
  fuzz: Object.freeze({
    fuzz_weight: 0,
    fuzz_color: [1, 1, 1] as Color3,
    fuzz_roughness: 0.5,
  }),
  emission: Object.freeze({
    emission_color: [1, 1, 1] as Color3,
    emission_luminance: 0,
  }),
  thinFilm: Object.freeze({
    thin_film_weight: 0,
    thin_film_thickness: 0.5,
    thin_film_ior: 1.4,
  }),
  subsurface: Object.freeze({
    subsurface_weight: 0,
    subsurface_color: [0.8, 0.8, 0.8] as Color3,
    subsurface_radius: 1,
  }),
  geometry: Object.freeze({
    geometry_opacity: 1,
    geometry_thin_walled: false,
  }),
});

// ── Construction ─────────────────────────────────────────────────────────────

/** Minimal record: `base: {}` unless `overrides` adds fields or other layers.
 *  `id` is always freshly minted — a caller cannot inject one here. */
export function createMaterial(
  name: string,
  overrides?: Partial<Omit<MaterialDef, "id" | "name">>,
): MaterialDef {
  return {
    id: mintMaterialId(),
    name,
    base: {},
    ...overrides,
  };
}

// ── Resolution ───────────────────────────────────────────────────────────────

/** Every OpenPBR parameter, flat and always present — defaults filled,
 *  values clamped. No `id`/`name`: this is shading state only, so two
 *  materials that resolve to identical parameters hash identically
 *  (`resolvedMaterialHash`) and can share one pool entry regardless of which
 *  document record produced them. */
export interface ResolvedMaterial {
  base_color: Color3;
  base_metalness: number;
  base_diffuse_roughness: number;

  specular_weight: number;
  specular_color: Color3;
  specular_roughness: number;
  specular_roughness_anisotropy: number;
  specular_ior: number;

  transmission_weight: number;
  transmission_color: Color3;
  transmission_depth: number;

  coat_weight: number;
  coat_roughness: number;
  coat_ior: number;
  coat_color: Color3;
  coat_darkening: number;

  fuzz_weight: number;
  fuzz_color: Color3;
  fuzz_roughness: number;

  emission_color: Color3;
  emission_luminance: number;

  thin_film_weight: number;
  thin_film_thickness: number;
  thin_film_ior: number;

  subsurface_weight: number;
  subsurface_color: Color3;
  subsurface_radius: number;

  geometry_opacity: number;
  geometry_thin_walled: boolean;
}

const clampUnit = (v: number): number => Math.min(1, Math.max(0, v));
const clampIor = (v: number): number => Math.min(4, Math.max(1, v));
const clampNonNegative = (v: number): number => Math.max(0, v);

function resolveScalar(raw: number | undefined, fallback: number, clamp: (v: number) => number): number {
  return raw !== undefined && isFiniteNumber(raw) ? clamp(raw) : fallback;
}

function resolveColor(raw: Color3 | undefined, fallback: Color3): Color3 {
  return raw !== undefined && isColor3(raw) ? [raw[0], raw[1], raw[2]] : [fallback[0], fallback[1], fallback[2]];
}

function resolveBool(raw: boolean | undefined, fallback: boolean): boolean {
  return typeof raw === "boolean" ? raw : fallback;
}

/**
 * At-rest `def` is read-only here, never mutated — the caller's sparse record
 * survives untouched. Clamping (weights/roughness/metalness/opacity → `[0,1]`,
 * iors → `[1,4]`, thickness/luminance/radius/depth → `>= 0`) and the
 * non-finite-number-becomes-default rule both apply ONLY at resolve time, not
 * at rest, so a temporarily out-of-range value typed mid-edit is never
 * silently rewritten in the document.
 */
export function resolveMaterial(def: MaterialDef): ResolvedMaterial {
  const base = def.base ?? {};
  const specular = def.specular ?? {};
  const transmission = def.transmission ?? {};
  const coat = def.coat ?? {};
  const fuzz = def.fuzz ?? {};
  const emission = def.emission ?? {};
  const thinFilm = def.thinFilm ?? {};
  const subsurface = def.subsurface ?? {};
  const geometry = def.geometry ?? {};
  const d = OPENPBR_DEFAULTS;

  return {
    base_color: resolveColor(base.base_color, d.base.base_color),
    base_metalness: resolveScalar(base.base_metalness, d.base.base_metalness, clampUnit),
    base_diffuse_roughness: resolveScalar(
      base.base_diffuse_roughness,
      d.base.base_diffuse_roughness,
      clampUnit,
    ),

    specular_weight: resolveScalar(specular.specular_weight, d.specular.specular_weight, clampUnit),
    specular_color: resolveColor(specular.specular_color, d.specular.specular_color),
    specular_roughness: resolveScalar(
      specular.specular_roughness,
      d.specular.specular_roughness,
      clampUnit,
    ),
    specular_roughness_anisotropy: resolveScalar(
      specular.specular_roughness_anisotropy,
      d.specular.specular_roughness_anisotropy,
      clampUnit,
    ),
    specular_ior: resolveScalar(specular.specular_ior, d.specular.specular_ior, clampIor),

    transmission_weight: resolveScalar(
      transmission.transmission_weight,
      d.transmission.transmission_weight,
      clampUnit,
    ),
    transmission_color: resolveColor(transmission.transmission_color, d.transmission.transmission_color),
    transmission_depth: resolveScalar(
      transmission.transmission_depth,
      d.transmission.transmission_depth,
      clampNonNegative,
    ),

    coat_weight: resolveScalar(coat.coat_weight, d.coat.coat_weight, clampUnit),
    coat_roughness: resolveScalar(coat.coat_roughness, d.coat.coat_roughness, clampUnit),
    coat_ior: resolveScalar(coat.coat_ior, d.coat.coat_ior, clampIor),
    coat_color: resolveColor(coat.coat_color, d.coat.coat_color),
    coat_darkening: resolveScalar(coat.coat_darkening, d.coat.coat_darkening, clampUnit),

    fuzz_weight: resolveScalar(fuzz.fuzz_weight, d.fuzz.fuzz_weight, clampUnit),
    fuzz_color: resolveColor(fuzz.fuzz_color, d.fuzz.fuzz_color),
    fuzz_roughness: resolveScalar(fuzz.fuzz_roughness, d.fuzz.fuzz_roughness, clampUnit),

    emission_color: resolveColor(emission.emission_color, d.emission.emission_color),
    emission_luminance: resolveScalar(
      emission.emission_luminance,
      d.emission.emission_luminance,
      clampNonNegative,
    ),

    thin_film_weight: resolveScalar(thinFilm.thin_film_weight, d.thinFilm.thin_film_weight, clampUnit),
    thin_film_thickness: resolveScalar(
      thinFilm.thin_film_thickness,
      d.thinFilm.thin_film_thickness,
      clampNonNegative,
    ),
    thin_film_ior: resolveScalar(thinFilm.thin_film_ior, d.thinFilm.thin_film_ior, clampIor),

    subsurface_weight: resolveScalar(
      subsurface.subsurface_weight,
      d.subsurface.subsurface_weight,
      clampUnit,
    ),
    subsurface_color: resolveColor(subsurface.subsurface_color, d.subsurface.subsurface_color),
    subsurface_radius: resolveScalar(
      subsurface.subsurface_radius,
      d.subsurface.subsurface_radius,
      clampNonNegative,
    ),

    geometry_opacity: resolveScalar(geometry.geometry_opacity, d.geometry.geometry_opacity, clampUnit),
    geometry_thin_walled: resolveBool(
      geometry.geometry_thin_walled,
      d.geometry.geometry_thin_walled,
    ),
  };
}

// ── Hashing ──────────────────────────────────────────────────────────────────

/** Recursively key-sorts an already-JSON-safe value so the stringified form
 *  is stable regardless of the source object's own key insertion order. */
function canonicalize(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value as Record<string, unknown>).sort()) {
      out[key] = canonicalize((value as Record<string, unknown>)[key]);
    }
    return out;
  }
  return value;
}

/** FNV-1a-32, hex — same algorithm `ipc/mockFaceGeometry.ts`'s
 *  `mockElementHash` uses for mock ElementIds, reimplemented locally to keep
 *  this module out of the IPC layer. Not the worker/Rust FNV-1a-64 that mints
 *  real ElementIds; this hash's job is a viewport material-pool cache key,
 *  not identity. */
function fnv1aHex(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(16).padStart(8, "0");
}

/** Deterministic, key-order-independent — stable across `MaterialDef` field
 *  ordering because it hashes `resolveMaterial`'s canonicalized output, not
 *  the def itself. */
export function resolvedMaterialHash(def: MaterialDef): string {
  return fnv1aHex(JSON.stringify(canonicalize(resolveMaterial(def))));
}
