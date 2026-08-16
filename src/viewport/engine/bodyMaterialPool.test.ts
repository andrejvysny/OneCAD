/*
 * BodyMaterialPool — sharing, tier selection, sweep, and the dim/tint state
 * mechanics it mirrors from BodyMaterialLibrary.
 *
 * The sharing test is the contract this class exists for: N bodies wearing one
 * material must compile to ONE program and one uniform upload. Everything else
 * here protects a body with an assigned material from behaving differently to an
 * unassigned one in sketch mode or under a Cut preview.
 */
import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { BodyMaterialPool } from "./bodyMaterialPool";
import type { PbrMaterialParams } from "./pbrParams";

const WHITE_HEX = new THREE.Color(1, 1, 1).getHex();

const PARAMS: PbrMaterialParams = {
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

const params = (over: Partial<PbrMaterialParams> = {}): PbrMaterialParams => ({
  ...PARAMS,
  ...over,
});

describe("BodyMaterialPool.acquire — ONE PROGRAM PER MATERIAL", () => {
  /*
   * The whole reason this class exists. Two bodies wearing the same material
   * must end up holding the SAME THREE material instance: a per-body copy would
   * mean a per-body shader program, a per-body uniform upload, and a per-body
   * place for a state change (dim, tint) to be forgotten.
   */
  it("hands back the SAME instance for a key, every time", () => {
    const pool = new BodyMaterialPool();
    const a = pool.acquire("mat_1:abcd", params(), false);
    const b = pool.acquire("mat_1:abcd", params(), false);
    expect(b).toBe(a);
    expect(pool.size()).toBe(1);
    pool.dispose();
  });

  it("gives a different key its own instance", () => {
    const pool = new BodyMaterialPool();
    const a = pool.acquire("mat_1:abcd", params(), false);
    const b = pool.acquire("mat_2:efgh", params({ base_color: [1, 0, 0] }), false);
    expect(b).not.toBe(a);
    expect(pool.size()).toBe(2);
    pool.dispose();
  });

  /*
   * The `:vc` twin differs from its plain sibling ONLY in reading a per-vertex
   * attribute and carrying multiply identity as its base. Sharing one instance
   * between them would either tint every baked face color or drop the attribute.
   */
  it("keeps the vertex-colored twin separate, white-based, and attribute-reading", () => {
    const pool = new BodyMaterialPool();
    const plain = pool.acquire("k", params({ base_color: [0.2, 0.4, 0.6] }), false);
    const twin = pool.acquire("k:vc", params({ base_color: [0.2, 0.4, 0.6] }), true);

    expect(twin).not.toBe(plain);
    expect(twin.vertexColors).toBe(true);
    expect(plain.vertexColors).toBe(false);
    expect(twin.color.getHex()).toBe(WHITE_HEX);
    expect(plain.color.getHex()).not.toBe(WHITE_HEX);
    pool.dispose();
  });

  it("copies the library's face conventions so a pooled body picks + outlines identically", () => {
    const pool = new BodyMaterialPool();
    const mat = pool.acquire("k", params(), false);
    expect(mat.side).toBe(THREE.DoubleSide);
    expect([mat.polygonOffset, mat.polygonOffsetFactor, mat.polygonOffsetUnits]).toEqual([
      true,
      1,
      1,
    ]);
    expect(mat.envMapIntensity).toBe(1);
    pool.dispose();
  });

  it("builds a physical material only when a promoted lobe is in play", () => {
    const pool = new BodyMaterialPool();
    const plain = pool.acquire("plain", params(), false);
    const glass = pool.acquire("glass", params({ transmission_weight: 0.8 }), false);

    expect((plain as THREE.MeshPhysicalMaterial).isMeshPhysicalMaterial).toBeFalsy();
    expect((glass as THREE.MeshPhysicalMaterial).isMeshPhysicalMaterial).toBe(true);
    expect((glass as THREE.MeshPhysicalMaterial).transmission).toBe(0.8);
    pool.dispose();
  });
});

describe("BodyMaterialPool.sweep", () => {
  it("disposes and drops every entry the live key set does not name", () => {
    const pool = new BodyMaterialPool();
    const kept = pool.acquire("kept", params(), false);
    const dropped = pool.acquire("dropped", params({ base_metalness: 1 }), false);
    const keptSpy = vi.spyOn(kept, "dispose");
    const droppedSpy = vi.spyOn(dropped, "dispose");

    pool.sweep(new Set(["kept"]));

    expect(droppedSpy).toHaveBeenCalled();
    expect(keptSpy).not.toHaveBeenCalled();
    expect(pool.size()).toBe(1);
    // …and the surviving entry is still the SAME instance, not a rebuild.
    expect(pool.acquire("kept", params(), false)).toBe(kept);
    pool.dispose();
  });

  it("an empty live set clears the pool", () => {
    const pool = new BodyMaterialPool();
    pool.acquire("a", params(), false);
    pool.acquire("b", params(), false);
    pool.sweep(new Set());
    expect(pool.size()).toBe(0);
    pool.dispose();
  });
});

describe("BodyMaterialPool.setDimmed", () => {
  it("dims live materials and repeats are no-ops", () => {
    const pool = new BodyMaterialPool();
    const mat = pool.acquire("k", params(), false);

    pool.setDimmed(true);
    expect(mat.transparent).toBe(true);
    expect(mat.opacity).toBe(0.35);

    pool.setDimmed(true); // must not re-save 0.35 as the "prior"
    pool.setDimmed(false);
    expect(mat.opacity).toBe(1);
    expect(mat.transparent).toBe(false);
    pool.dispose();
  });

  /*
   * Sharper here than in the library: a pooled material's undimmed transparency
   * is DATA (`geometry_opacity`), so restoring hardcoded defaults would silently
   * turn every translucent material solid on leaving sketch mode.
   */
  it("restores the SAVED prior state — a translucent material stays translucent", () => {
    const pool = new BodyMaterialPool();
    const mat = pool.acquire("k", params({ geometry_opacity: 0.4 }), false);
    expect([mat.transparent, mat.opacity]).toEqual([true, 0.4]);

    pool.setDimmed(true);
    expect(mat.opacity).toBe(0.35);

    pool.setDimmed(false);
    expect(mat.transparent).toBe(true);
    expect(mat.opacity).toBe(0.4);
    pool.dispose();
  });

  it("dims a material ACQUIRED while dimmed, and undims it to its own params", () => {
    const pool = new BodyMaterialPool();
    pool.setDimmed(true);

    const mat = pool.acquire("k", params({ geometry_opacity: 0.6 }), false); // first use under the dim
    expect(mat.opacity).toBe(0.35);
    expect(mat.transparent).toBe(true);

    pool.setDimmed(false);
    expect(mat.opacity).toBe(0.6);
    expect(mat.transparent).toBe(true);
    pool.dispose();
  });
});

describe("BodyMaterialPool face color (Cut tint)", () => {
  it("copies the tint onto every live material — never retaining the instance", () => {
    const pool = new BodyMaterialPool();
    const mat = pool.acquire("k", params(), false);
    const tint = new THREE.Color(0.9, 0.1, 0.1);

    pool.setFaceColor(tint);
    expect(mat.color.getHex()).toBe(tint.getHex());
    expect(mat.color).not.toBe(tint);
    pool.dispose();
  });

  it("tints a material acquired while the tint is active", () => {
    const pool = new BodyMaterialPool();
    const tint = new THREE.Color(0.9, 0.1, 0.1);
    pool.setFaceColor(tint);
    expect(pool.acquire("late", params(), false).color.getHex()).toBe(tint.getHex());
    pool.dispose();
  });

  it("reset restores each entry's OWN params color — white for the :vc twin", () => {
    const pool = new BodyMaterialPool();
    const plain = pool.acquire("k", params({ base_color: [0.2, 0.4, 0.6] }), false);
    const twin = pool.acquire("k:vc", params({ base_color: [0.2, 0.4, 0.6] }), true);
    pool.setFaceColor(new THREE.Color(0.9, 0.1, 0.1));

    pool.resetFaceColor();

    expect([plain.color.r, plain.color.g, plain.color.b]).toEqual([0.2, 0.4, 0.6]);
    // Multiply identity, NOT the material's own base — the vertex attribute
    // already carries that, and a tinted base would double-apply it.
    expect(twin.color.getHex()).toBe(WHITE_HEX);
    pool.dispose();
  });
});

/*
 * The theming invariant (engine/README.md § Theming) says every palette consumer
 * exposes refreshColors. This class is the documented exception on the INSIDE —
 * it reads no palette, so there is nothing to re-read — and the method exists
 * precisely so it is a deliberate no-op rather than a silent omission.
 */
describe("BodyMaterialPool.refreshColors", () => {
  it("is a no-op: pooled materials are data-colored, not token-colored", () => {
    const pool = new BodyMaterialPool();
    const mat = pool.acquire("k", params({ base_color: [0.2, 0.4, 0.6] }), false);
    const before = mat.color.getHex();

    pool.refreshColors();

    expect(mat.color.getHex()).toBe(before);
    pool.dispose();
  });

  it("does not disturb an active Cut tint either", () => {
    const pool = new BodyMaterialPool();
    const mat = pool.acquire("k", params(), false);
    const tint = new THREE.Color(0.9, 0.1, 0.1);
    pool.setFaceColor(tint);

    pool.refreshColors();

    expect(mat.color.getHex()).toBe(tint.getHex());
    pool.dispose();
  });
});

describe("BodyMaterialPool.dispose", () => {
  it("disposes every pooled material and empties the pool", () => {
    const pool = new BodyMaterialPool();
    const a = pool.acquire("a", params(), false);
    const b = pool.acquire("b", params({ coat_weight: 1 }), false);
    const spies = [vi.spyOn(a, "dispose"), vi.spyOn(b, "dispose")];

    pool.dispose();

    for (const s of spies) expect(s).toHaveBeenCalled();
    expect(pool.size()).toBe(0);
  });
});
