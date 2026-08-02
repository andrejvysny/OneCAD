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
