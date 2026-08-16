import { describe, it, expect } from "vitest";
import {
  OPENPBR_DEFAULTS,
  createMaterial,
  isColor3,
  isFiniteNumber,
  isMaterialId,
  mintMaterialId,
  resolveMaterial,
  resolvedMaterialHash,
  type MaterialDef,
} from "./material";

// ── Ids ──────────────────────────────────────────────────────────────────────

describe("mintMaterialId / isMaterialId", () => {
  it("mints ids matching mat_<uuid v4>", () => {
    const id = mintMaterialId();
    expect(isMaterialId(id)).toBe(true);
    expect(id).toMatch(/^mat_[0-9a-f-]{36}$/i);
  });

  it("mints unique ids", () => {
    expect(mintMaterialId()).not.toBe(mintMaterialId());
  });

  it("rejects non-strings and malformed shapes", () => {
    expect(isMaterialId(123)).toBe(false);
    expect(isMaterialId(null)).toBe(false);
    expect(isMaterialId("mat_not-a-uuid")).toBe(false);
    expect(isMaterialId("el_" + mintMaterialId().slice(4))).toBe(false);
  });
});

describe("isColor3 / isFiniteNumber", () => {
  it("accepts a 3-tuple of finite numbers", () => {
    expect(isColor3([0.1, 0.2, 0.3])).toBe(true);
  });

  it("rejects wrong length, non-arrays, and non-finite components", () => {
    expect(isColor3([0.1, 0.2])).toBe(false);
    expect(isColor3([0.1, 0.2, 0.3, 0.4])).toBe(false);
    expect(isColor3("red")).toBe(false);
    expect(isColor3([0.1, Number.NaN, 0.3])).toBe(false);
    expect(isColor3([0.1, Number.POSITIVE_INFINITY, 0.3])).toBe(false);
  });

  it("treats NaN/Infinity as non-finite", () => {
    expect(isFiniteNumber(1)).toBe(true);
    expect(isFiniteNumber(Number.NaN)).toBe(false);
    expect(isFiniteNumber(Number.POSITIVE_INFINITY)).toBe(false);
    expect(isFiniteNumber("1")).toBe(false);
  });
});

// ── createMaterial ───────────────────────────────────────────────────────────

describe("createMaterial", () => {
  it("creates a minimal base-only record", () => {
    const mat = createMaterial("Brushed Steel");
    expect(mat.name).toBe("Brushed Steel");
    expect(isMaterialId(mat.id)).toBe(true);
    expect(mat.base).toEqual({});
    expect(mat.specular).toBeUndefined();
    expect(mat.coat).toBeUndefined();
  });

  it("layers in overrides", () => {
    const mat = createMaterial("Car Paint", {
      base: { base_color: [0.9, 0.1, 0.1] },
      coat: { coat_weight: 1 },
    });
    expect(mat.base.base_color).toEqual([0.9, 0.1, 0.1]);
    expect(mat.coat).toEqual({ coat_weight: 1 });
  });
});

// ── OPENPBR_DEFAULTS — OpenPBR v1.1.1 (2026-04-17) pinned values ────────────
// https://academysoftwarefoundation.github.io/OpenPBR/ — one assertion per
// field so a future spec re-pin shows up as an explicit diff here.

describe("OPENPBR_DEFAULTS — base (OpenPBR v1.1.1)", () => {
  it("pins base_color, base_metalness, base_diffuse_roughness", () => {
    expect(OPENPBR_DEFAULTS.base.base_color).toEqual([0.8, 0.8, 0.8]);
    expect(OPENPBR_DEFAULTS.base.base_metalness).toBe(0);
    expect(OPENPBR_DEFAULTS.base.base_diffuse_roughness).toBe(0);
  });
});

describe("OPENPBR_DEFAULTS — specular (OpenPBR v1.1.1)", () => {
  it("pins specular_weight, specular_color, specular_roughness, specular_roughness_anisotropy, specular_ior", () => {
    expect(OPENPBR_DEFAULTS.specular.specular_weight).toBe(1);
    expect(OPENPBR_DEFAULTS.specular.specular_color).toEqual([1, 1, 1]);
    expect(OPENPBR_DEFAULTS.specular.specular_roughness).toBe(0.3);
    expect(OPENPBR_DEFAULTS.specular.specular_roughness_anisotropy).toBe(0);
    expect(OPENPBR_DEFAULTS.specular.specular_ior).toBe(1.5);
  });
});

describe("OPENPBR_DEFAULTS — transmission (OpenPBR v1.1.1)", () => {
  it("pins transmission_weight, transmission_color, transmission_depth", () => {
    expect(OPENPBR_DEFAULTS.transmission.transmission_weight).toBe(0);
    expect(OPENPBR_DEFAULTS.transmission.transmission_color).toEqual([1, 1, 1]);
    expect(OPENPBR_DEFAULTS.transmission.transmission_depth).toBe(0);
  });
});

describe("OPENPBR_DEFAULTS — coat (OpenPBR v1.1.1)", () => {
  it("pins coat_weight, coat_roughness, coat_ior, coat_color, coat_darkening", () => {
    expect(OPENPBR_DEFAULTS.coat.coat_weight).toBe(0);
    expect(OPENPBR_DEFAULTS.coat.coat_roughness).toBe(0);
    expect(OPENPBR_DEFAULTS.coat.coat_ior).toBe(1.6);
    expect(OPENPBR_DEFAULTS.coat.coat_color).toEqual([1, 1, 1]);
    expect(OPENPBR_DEFAULTS.coat.coat_darkening).toBe(1);
  });
});

describe("OPENPBR_DEFAULTS — fuzz (OpenPBR v1.1.1)", () => {
  it("pins fuzz_weight, fuzz_color, fuzz_roughness", () => {
    expect(OPENPBR_DEFAULTS.fuzz.fuzz_weight).toBe(0);
    expect(OPENPBR_DEFAULTS.fuzz.fuzz_color).toEqual([1, 1, 1]);
    expect(OPENPBR_DEFAULTS.fuzz.fuzz_roughness).toBe(0.5);
  });
});

describe("OPENPBR_DEFAULTS — emission (OpenPBR v1.1.1)", () => {
  it("pins emission_color, emission_luminance", () => {
    expect(OPENPBR_DEFAULTS.emission.emission_color).toEqual([1, 1, 1]);
    expect(OPENPBR_DEFAULTS.emission.emission_luminance).toBe(0);
  });
});

describe("OPENPBR_DEFAULTS — thinFilm (OpenPBR v1.1.1)", () => {
  it("pins thin_film_weight, thin_film_thickness, thin_film_ior", () => {
    expect(OPENPBR_DEFAULTS.thinFilm.thin_film_weight).toBe(0);
    expect(OPENPBR_DEFAULTS.thinFilm.thin_film_thickness).toBe(0.5);
    expect(OPENPBR_DEFAULTS.thinFilm.thin_film_ior).toBe(1.4);
  });
});

describe("OPENPBR_DEFAULTS — subsurface (OpenPBR v1.1.1)", () => {
  it("pins subsurface_weight, subsurface_color, subsurface_radius", () => {
    expect(OPENPBR_DEFAULTS.subsurface.subsurface_weight).toBe(0);
    expect(OPENPBR_DEFAULTS.subsurface.subsurface_color).toEqual([0.8, 0.8, 0.8]);
    expect(OPENPBR_DEFAULTS.subsurface.subsurface_radius).toBe(1);
  });
});

describe("OPENPBR_DEFAULTS — geometry (OpenPBR v1.1.1)", () => {
  it("pins geometry_opacity, geometry_thin_walled", () => {
    expect(OPENPBR_DEFAULTS.geometry.geometry_opacity).toBe(1);
    expect(OPENPBR_DEFAULTS.geometry.geometry_thin_walled).toBe(false);
  });
});

describe("OPENPBR_DEFAULTS is frozen", () => {
  it("cannot be reassigned at any level", () => {
    expect(Object.isFrozen(OPENPBR_DEFAULTS)).toBe(true);
    expect(Object.isFrozen(OPENPBR_DEFAULTS.base)).toBe(true);
    expect(Object.isFrozen(OPENPBR_DEFAULTS.coat)).toBe(true);
  });
});

// ── resolveMaterial ──────────────────────────────────────────────────────────

describe("resolveMaterial", () => {
  it("fills every default for a base-only material", () => {
    const mat = createMaterial("Bare");
    const resolved = resolveMaterial(mat);
    expect(resolved).toEqual({
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
    });
  });

  it("passes through in-range values verbatim", () => {
    const mat = createMaterial("Tinted Glass", {
      base: { base_color: [0.2, 0.4, 0.6], base_metalness: 0.5 },
      transmission: { transmission_weight: 0.9, transmission_depth: 3.2 },
    });
    const resolved = resolveMaterial(mat);
    expect(resolved.base_color).toEqual([0.2, 0.4, 0.6]);
    expect(resolved.base_metalness).toBe(0.5);
    expect(resolved.transmission_weight).toBe(0.9);
    expect(resolved.transmission_depth).toBe(3.2);
  });

  it("clamps weight/roughness/metalness/opacity to [0,1] only at resolve time", () => {
    const mat = createMaterial("Overdriven", {
      base: { base_metalness: 5, base_diffuse_roughness: -3 },
      specular: { specular_weight: -1, specular_roughness: 2, specular_roughness_anisotropy: 9 },
      coat: { coat_weight: 10, coat_roughness: -10, coat_darkening: 4 },
      geometry: { geometry_opacity: -0.5 },
    });
    const resolved = resolveMaterial(mat);
    expect(resolved.base_metalness).toBe(1);
    expect(resolved.base_diffuse_roughness).toBe(0);
    expect(resolved.specular_weight).toBe(0);
    expect(resolved.specular_roughness).toBe(1);
    expect(resolved.specular_roughness_anisotropy).toBe(1);
    expect(resolved.coat_weight).toBe(1);
    expect(resolved.coat_roughness).toBe(0);
    expect(resolved.coat_darkening).toBe(1);
    expect(resolved.geometry_opacity).toBe(0);

    // The at-rest def is untouched by resolving it.
    expect(mat.base.base_metalness).toBe(5);
    expect(mat.specular?.specular_weight).toBe(-1);
    expect(mat.coat?.coat_weight).toBe(10);
  });

  it("clamps iors to [1,4]", () => {
    const mat = createMaterial("Extreme IOR", {
      specular: { specular_ior: 0.2 },
      coat: { coat_ior: 100 },
      thinFilm: { thin_film_ior: -1 },
    });
    const resolved = resolveMaterial(mat);
    expect(resolved.specular_ior).toBe(1);
    expect(resolved.coat_ior).toBe(4);
    expect(resolved.thin_film_ior).toBe(1);
  });

  it("clamps thickness/luminance/radius/depth to >= 0", () => {
    const mat = createMaterial("Negative Everything", {
      transmission: { transmission_depth: -5 },
      emission: { emission_luminance: -1 },
      thinFilm: { thin_film_thickness: -0.5 },
      subsurface: { subsurface_radius: -2 },
    });
    const resolved = resolveMaterial(mat);
    expect(resolved.transmission_depth).toBe(0);
    expect(resolved.emission_luminance).toBe(0);
    expect(resolved.thin_film_thickness).toBe(0);
    expect(resolved.subsurface_radius).toBe(0);
  });

  it("does not clamp colors (emission may legitimately exceed 1 for HDR)", () => {
    const mat = createMaterial("Hot Emitter", {
      emission: { emission_color: [3, 3, 3] },
    });
    expect(resolveMaterial(mat).emission_color).toEqual([3, 3, 3]);
  });

  it("resolves non-finite numbers to the default instead of clamping garbage", () => {
    const mat: MaterialDef = {
      id: mintMaterialId(),
      name: "NaN Bomb",
      base: { base_metalness: Number.NaN },
      specular: { specular_ior: Number.POSITIVE_INFINITY },
      transmission: { transmission_depth: Number.NEGATIVE_INFINITY },
    };
    const resolved = resolveMaterial(mat);
    expect(resolved.base_metalness).toBe(OPENPBR_DEFAULTS.base.base_metalness);
    expect(resolved.specular_ior).toBe(OPENPBR_DEFAULTS.specular.specular_ior);
    expect(resolved.transmission_depth).toBe(OPENPBR_DEFAULTS.transmission.transmission_depth);
  });

  it("resolves a malformed color to the default color", () => {
    const mat: MaterialDef = {
      id: mintMaterialId(),
      name: "Bad Color",
      base: { base_color: [1, Number.NaN, 0] as unknown as [number, number, number] },
    };
    expect(resolveMaterial(mat).base_color).toEqual(OPENPBR_DEFAULTS.base.base_color);
  });

  it("resolves booleans strictly (no truthy coercion)", () => {
    const mat = createMaterial("Thin Walled", { geometry: { geometry_thin_walled: true } });
    expect(resolveMaterial(mat).geometry_thin_walled).toBe(true);
    expect(resolveMaterial(createMaterial("Default Walled")).geometry_thin_walled).toBe(false);
  });
});

// ── resolvedMaterialHash ─────────────────────────────────────────────────────

describe("resolvedMaterialHash", () => {
  it("is identical for two defs that resolve to the same parameters, built in different key order", () => {
    const a: MaterialDef = {
      id: mintMaterialId(),
      name: "A",
      base: { base_color: [0.5, 0.5, 0.5], base_metalness: 0.2 },
      coat: { coat_weight: 1, coat_ior: 1.6 },
    };
    const b: MaterialDef = {
      coat: { coat_ior: 1.6, coat_weight: 1 },
      base: { base_metalness: 0.2, base_color: [0.5, 0.5, 0.5] },
      name: "B", // name differs — hash must not depend on it
      id: mintMaterialId(), // id differs too — hash must not depend on it
    };
    expect(resolvedMaterialHash(a)).toBe(resolvedMaterialHash(b));
  });

  it("differs when any single parameter differs", () => {
    const base = createMaterial("Base", { base: { base_metalness: 0.2 } });
    const changed = createMaterial("Base", { base: { base_metalness: 0.3 } });
    expect(resolvedMaterialHash(base)).not.toBe(resolvedMaterialHash(changed));
  });

  it("is a hex string", () => {
    expect(resolvedMaterialHash(createMaterial("X"))).toMatch(/^[0-9a-f]+$/);
  });
});
