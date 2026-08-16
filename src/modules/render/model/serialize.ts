/*
 * Hand-rolled parse/serialize for `RenderModuleState` — no schema library
 * (repo convention; see `ipc/operationDiagnostics.ts`'s
 * `parseOperationDiagnostics` for the same tolerant-parse-with-warnings shape
 * this file follows).
 *
 * UNKNOWN-KEY PRESERVATION, at every level (root, material, layer): parsing
 * starts from ONE `structuredClone` of the raw payload and every validator
 * below mutates that clone IN PLACE — deleting a field it rejects, leaving
 * everything it does not recognize untouched. A round trip through
 * `serializeRenderModuleState` therefore reproduces foreign bytes verbatim,
 * per ADR-0005's "unknown module state survives load/save" rule, without this
 * file having to know what a future schema revision might add.
 */
import {
  isColor3,
  isFiniteNumber,
  isMaterialId,
  type MaterialDef,
} from "./material";
import type { RenderAssignments } from "./assignment";
import { CURRENT_RENDER_SCHEMA_VERSION, PINNED_OPENPBR_REVISION, type RenderModuleState } from "./state";

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// ── Layer field tables (drives the generic per-field validator below) ────────

const NUMERIC_FIELDS_BY_LAYER: Readonly<Record<string, readonly string[]>> = {
  base: ["base_metalness", "base_diffuse_roughness"],
  specular: ["specular_weight", "specular_roughness", "specular_roughness_anisotropy", "specular_ior"],
  transmission: ["transmission_weight", "transmission_depth"],
  coat: ["coat_weight", "coat_roughness", "coat_ior", "coat_darkening"],
  fuzz: ["fuzz_weight", "fuzz_roughness"],
  emission: ["emission_luminance"],
  thinFilm: ["thin_film_weight", "thin_film_thickness", "thin_film_ior"],
  subsurface: ["subsurface_weight", "subsurface_radius"],
  geometry: ["geometry_opacity"],
};

const COLOR_FIELDS_BY_LAYER: Readonly<Record<string, readonly string[]>> = {
  base: ["base_color"],
  specular: ["specular_color"],
  transmission: ["transmission_color"],
  coat: ["coat_color"],
  fuzz: ["fuzz_color"],
  emission: ["emission_color"],
  subsurface: ["subsurface_color"],
};

const BOOLEAN_FIELDS_BY_LAYER: Readonly<Record<string, readonly string[]>> = {
  geometry: ["geometry_thin_walled"],
};

const OPTIONAL_LAYER_NAMES = [
  "specular",
  "transmission",
  "coat",
  "fuzz",
  "emission",
  "thinFilm",
  "subsurface",
  "geometry",
] as const;

/** Validates the known fields of one layer object IN PLACE: an invalid known
 *  field is deleted (so `resolveMaterial` falls back to the spec default)
 *  with a warning; anything not in the tables above is left alone. */
function sanitizeLayer(
  layer: Record<string, unknown>,
  layerName: string,
  materialKey: string,
  warnings: string[],
): void {
  for (const field of NUMERIC_FIELDS_BY_LAYER[layerName] ?? []) {
    if (field in layer && !isFiniteNumber(layer[field])) {
      delete layer[field];
      warnings.push(`material "${materialKey}": ${layerName}.${field} dropped (not a finite number)`);
    }
  }
  for (const field of COLOR_FIELDS_BY_LAYER[layerName] ?? []) {
    if (field in layer && !isColor3(layer[field])) {
      delete layer[field];
      warnings.push(`material "${materialKey}": ${layerName}.${field} dropped (not a Color3)`);
    }
  }
  for (const field of BOOLEAN_FIELDS_BY_LAYER[layerName] ?? []) {
    if (field in layer && typeof layer[field] !== "boolean") {
      delete layer[field];
      warnings.push(`material "${materialKey}": ${layerName}.${field} dropped (not a boolean)`);
    }
  }
}

/** `null` when the record is structurally broken (missing/invalid id, name,
 *  or base) — the whole entry is dropped rather than admitting a material
 *  with no usable identity. Any other malformed field is corrected in place
 *  instead, so one bad field never costs the rest of the material. */
function parseMaterial(raw: unknown, key: string, warnings: string[]): MaterialDef | null {
  if (!isPlainObject(raw)) {
    warnings.push(`material "${key}" dropped (not an object)`);
    return null;
  }

  if (typeof raw.id !== "string" || !isMaterialId(raw.id)) {
    warnings.push(`material "${key}" dropped (missing or invalid id)`);
    return null;
  }
  if (typeof raw.name !== "string") {
    warnings.push(`material "${key}" dropped (missing or invalid name)`);
    return null;
  }
  if (!isPlainObject(raw.base)) {
    warnings.push(`material "${key}" dropped (missing or invalid base layer)`);
    return null;
  }
  sanitizeLayer(raw.base, "base", key, warnings);

  for (const layerName of OPTIONAL_LAYER_NAMES) {
    const layerValue = raw[layerName];
    if (layerValue === undefined) continue;
    if (!isPlainObject(layerValue)) {
      delete raw[layerName];
      warnings.push(`material "${key}": layer "${layerName}" dropped (not an object)`);
      continue;
    }
    sanitizeLayer(layerValue, layerName, key, warnings);
  }

  return raw as unknown as MaterialDef;
}

function sanitizeAssignmentMap(
  raw: unknown,
  mapName: "bodies" | "faces",
  warnings: string[],
): Record<string, string> {
  if (raw === undefined) return {};
  if (!isPlainObject(raw)) {
    warnings.push(`assignments.${mapName} dropped (not an object)`);
    return {};
  }
  for (const [id, value] of Object.entries(raw)) {
    if (typeof value !== "string" || !isMaterialId(value)) {
      delete raw[id];
      warnings.push(`assignments.${mapName}["${id}"] dropped (invalid material id)`);
    }
  }
  return raw as Record<string, string>;
}

function parseAssignments(raw: unknown, warnings: string[]): RenderAssignments {
  if (raw === undefined) return { bodies: {}, faces: {} };
  if (!isPlainObject(raw)) {
    warnings.push("assignments dropped (not an object)");
    return { bodies: {}, faces: {} };
  }
  raw.bodies = sanitizeAssignmentMap(raw.bodies, "bodies", warnings);
  raw.faces = sanitizeAssignmentMap(raw.faces, "faces", warnings);
  return raw as unknown as RenderAssignments;
}

/**
 * v1 passthrough. A future schema bump's version-specific migration steps go
 * here; `parseRenderModuleState` always calls this before checking
 * `schemaVersion`, so this is the one seam a migration needs to extend.
 */
export function migrateRenderModuleState(raw: unknown): unknown {
  return raw;
}

export interface ParseRenderModuleStateResult {
  state: RenderModuleState | null;
  warnings: string[];
}

/**
 * `schemaVersion` missing or not `1` is a hard failure (`state: null`): the
 * caller is expected to refuse to write in that case, which is what preserves
 * a future/foreign schema's bytes rather than clobbering them with a v1
 * reinterpretation. Every OTHER validation problem is tolerant — a malformed
 * material or assignment is dropped with a warning and the rest of the state
 * still parses.
 */
export function parseRenderModuleState(raw: unknown): ParseRenderModuleStateResult {
  const migrated = migrateRenderModuleState(raw);

  if (!isPlainObject(migrated)) {
    return { state: null, warnings: ["root is not an object"] };
  }
  if (migrated.schemaVersion !== CURRENT_RENDER_SCHEMA_VERSION) {
    return {
      state: null,
      warnings: [
        `unsupported schemaVersion ${JSON.stringify(migrated.schemaVersion)} (expected ${CURRENT_RENDER_SCHEMA_VERSION})`,
      ],
    };
  }

  const warnings: string[] = [];
  const clone = structuredClone(migrated) as Record<string, unknown>;

  const library: Record<string, MaterialDef> = {};
  if (clone.library !== undefined) {
    if (!isPlainObject(clone.library)) {
      warnings.push("library dropped (not an object)");
    } else {
      for (const [key, value] of Object.entries(clone.library)) {
        const material = parseMaterial(value, key, warnings);
        if (material) library[key] = material;
      }
    }
  }

  const assignments = parseAssignments(clone.assignments, warnings);

  const state = {
    ...clone,
    schemaVersion: CURRENT_RENDER_SCHEMA_VERSION,
    openPbrRevision: PINNED_OPENPBR_REVISION,
    library,
    assignments,
  } as RenderModuleState;

  return { state, warnings };
}

/** Plain JSON-safe object; round-trips through `parseRenderModuleState`. A
 *  defensive clone, so the caller cannot mutate live state through the
 *  serialized value. */
export function serializeRenderModuleState(state: RenderModuleState): unknown {
  return structuredClone(state);
}
