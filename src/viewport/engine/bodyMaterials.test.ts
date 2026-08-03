/*
 * BodyMaterialLibrary — sharing, disposal, and the dim save/restore contract.
 *
 * The dim discipline is the load-bearing part: restore replays the state
 * OBSERVED before dimming, never hardcoded reset values, so a set that was
 * already customised comes back exactly as it was — and a set created WHILE
 * dimmed (its "prior" being the constructor defaults) undims correctly too.
 */
import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { BodyMaterialLibrary } from "./bodyMaterials";
import { palette } from "./palette";

/** Multiply identity for a vertex-colored material (computed, never a hex literal). */
const WHITE_HEX = new THREE.Color(1, 1, 1).getHex();

describe("BodyMaterialLibrary.get", () => {
  it("hands back the SAME set on every call for a kind (shared materials)", () => {
    const lib = new BodyMaterialLibrary();
    const a = lib.get("standard");
    const b = lib.get("standard");
    expect(b).toBe(a);
    expect(b.face).toBe(a.face);
    expect(b.edge).toBe(a.edge);
    lib.dispose();
  });

  it("builds the face material with the neutral token + the edge polygon-offset setup", () => {
    const lib = new BodyMaterialLibrary();
    const { face, edge } = lib.get("standard");
    expect(face.color.getHex()).toBe(palette.bodyNeutral().getHex());
    expect(face.metalness).toBe(0);
    expect(face.roughness).toBe(0.5);
    expect(face.envMapIntensity).toBe(1);
    expect(face.side).toBe(THREE.DoubleSide);
    expect([face.polygonOffset, face.polygonOffsetFactor, face.polygonOffsetUnits]).toEqual([
      true,
      1,
      1,
    ]);
    expect(edge.color.getHex()).toBe(palette.bodyEdge().getHex());
    // Edges are annotation, not lit surface — they render their token exactly.
    expect(edge.toneMapped).toBe(false);
    lib.dispose();
  });
});

/*
 * `shadedVertex` is the same PBR material reading a per-vertex color. Its base
 * MUST be white, in every theme, because the shader multiplies base × vertex —
 * a tinted base would silently recolor authored (imported) STEP colors.
 */
describe("BodyMaterialLibrary shadedVertex kind", () => {
  it("is the standard shading with vertexColors on and a WHITE base", () => {
    const lib = new BodyMaterialLibrary();
    const std = lib.get("standard").face;
    const vtx = lib.get("shadedVertex").face;

    expect(vtx.vertexColors).toBe(true);
    expect(std.vertexColors).toBe(false);
    expect(vtx.color.getHex()).toBe(WHITE_HEX);
    expect(vtx.color.getHex()).not.toBe(std.color.getHex());
    // Everything else is shared with the standard kind.
    expect([vtx.metalness, vtx.roughness, vtx.envMapIntensity, vtx.side]).toEqual([
      std.metalness,
      std.roughness,
      std.envMapIntensity,
      std.side,
    ]);
    expect(vtx.polygonOffset).toBe(std.polygonOffset);
    lib.dispose();
  });

  it("keeps its white base across a palette refresh (the theme must not tint it)", () => {
    const lib = new BodyMaterialLibrary();
    const vtx = lib.get("shadedVertex").face;
    lib.refreshColors();
    expect(vtx.color.getHex()).toBe(WHITE_HEX);
    // The edge, which is genuinely theme-driven, still follows.
    expect(lib.get("shadedVertex").edge.color.getHex()).toBe(palette.bodyEdge().getHex());
    lib.dispose();
  });

  it("takes a Cut tint like any other kind, and resets back to white", () => {
    const lib = new BodyMaterialLibrary();
    const vtx = lib.get("shadedVertex").face;
    lib.setFaceColor(palette.destructive());
    expect(vtx.color.getHex()).toBe(palette.destructive().getHex());

    lib.resetFaceColor();
    expect(vtx.color.getHex()).toBe(WHITE_HEX);
    lib.dispose();
  });

  it("dims and restores independently of the standard set", () => {
    const lib = new BodyMaterialLibrary();
    const std = lib.get("standard").face;
    const vtx = lib.get("shadedVertex").face;

    lib.setDimmed(true);
    expect([std.opacity, vtx.opacity]).toEqual([0.35, 0.35]);
    expect(vtx.transparent).toBe(true);

    lib.setDimmed(false);
    expect([std.opacity, vtx.opacity]).toEqual([1, 1]);
    expect(vtx.transparent).toBe(false);
    lib.dispose();
  });

  it("a set born WHILE dimmed still undims (per-kind saved state)", () => {
    const lib = new BodyMaterialLibrary();
    lib.setDimmed(true);
    const vtx = lib.get("shadedVertex").face; // first use under the dim
    expect(vtx.opacity).toBe(0.35);

    lib.setDimmed(false);
    expect(vtx.opacity).toBe(1);
    expect(vtx.color.getHex()).toBe(WHITE_HEX); // and still multiply-identity
    lib.dispose();
  });
});

describe("BodyMaterialLibrary.dispose", () => {
  it("disposes every created set", () => {
    const lib = new BodyMaterialLibrary();
    const { face, edge } = lib.get("standard");
    const faceSpy = vi.spyOn(face, "dispose");
    const edgeSpy = vi.spyOn(edge, "dispose");

    lib.dispose();

    expect(faceSpy).toHaveBeenCalled();
    expect(edgeSpy).toHaveBeenCalled();
  });
});

describe("BodyMaterialLibrary.setDimmed", () => {
  it("dims a live face material (edge untouched) and repeats are no-ops", () => {
    const lib = new BodyMaterialLibrary();
    const { face, edge } = lib.get("standard");
    const edgeOpacity = edge.opacity;

    lib.setDimmed(true);
    expect(face.transparent).toBe(true);
    expect(face.opacity).toBe(0.35);
    expect(edge.opacity).toBe(edgeOpacity);

    lib.setDimmed(true); // must not re-save 0.35 as the "prior"
    lib.setDimmed(false);
    expect(face.opacity).toBe(1);
    lib.dispose();
  });

  it("restores the SAVED prior state, not hardcoded defaults", () => {
    const lib = new BodyMaterialLibrary();
    const { face } = lib.get("standard");
    // Deliberately far from MeshStandardMaterial defaults.
    face.transparent = true;
    face.opacity = 0.62;
    face.depthWrite = false;

    lib.setDimmed(true);
    expect(face.opacity).toBe(0.35);

    lib.setDimmed(false);
    expect(face.transparent).toBe(true);
    expect(face.opacity).toBe(0.62);
    expect(face.depthWrite).toBe(false);
    lib.dispose();
  });

  it("dims a set created WHILE dimmed, and undims it to the constructor defaults", () => {
    const lib = new BodyMaterialLibrary();
    lib.setDimmed(true);

    const { face } = lib.get("standard"); // first use happens under the dim
    expect(face.transparent).toBe(true);
    expect(face.opacity).toBe(0.35);

    lib.setDimmed(false);
    expect(face.transparent).toBe(false);
    expect(face.opacity).toBe(1);
    expect(face.depthWrite).toBe(true);
    lib.dispose();
  });
});

describe("BodyMaterialLibrary face color", () => {
  it("copies the color — it never retains the shared palette instance", () => {
    const lib = new BodyMaterialLibrary();
    const { face } = lib.get("standard");
    const tint = palette.destructive();
    const tintHex = tint.getHex();

    lib.setFaceColor(tint);
    expect(face.color.getHex()).toBe(tintHex);
    expect(face.color).not.toBe(tint);

    face.color.setRGB(0, 0, 0); // mutating the material must not reach the palette
    expect(palette.destructive().getHex()).toBe(tintHex);
    lib.dispose();
  });

  it("applies the override to a set created later, and resetFaceColor goes back to neutral", () => {
    const lib = new BodyMaterialLibrary();
    lib.setFaceColor(palette.destructive());
    expect(lib.get("standard").face.color.getHex()).toBe(palette.destructive().getHex());

    lib.resetFaceColor();
    expect(lib.get("standard").face.color.getHex()).toBe(palette.bodyNeutral().getHex());
    lib.dispose();
  });
});
