/*
 * BodyMaterialLibrary — sharing, disposal, and the dim save/restore contract.
 *
 * The dim discipline is the load-bearing part: restore replays the state
 * OBSERVED before dimming, never hardcoded reset values, so a set that was
 * already customised comes back exactly as it was — and a set created WHILE
 * dimmed (its "prior" being the constructor defaults) undims correctly too.
 */
import { describe, it, expect, vi, afterEach } from "vitest";
import * as THREE from "three";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { BodyMaterialLibrary, BODY_EDGE_WIDTH } from "./bodyMaterials";
import { palette, resetPaletteCache } from "./palette";

/** Multiply identity for a vertex-colored material (computed, never a hex literal). */
const WHITE_HEX = new THREE.Color(1, 1, 1).getHex();

/** Put the document in `theme` and drop the cache so getters re-resolve. */
function setTheme(theme: "light" | "dark"): void {
  document.documentElement.dataset.theme = theme;
  resetPaletteCache();
}

afterEach(() => setTheme("light"));

describe("BodyMaterialLibrary.get", () => {
  it("hands back the SAME set on every call for a kind (shared materials)", () => {
    const lib = new BodyMaterialLibrary();
    const a = lib.get("standard");
    const b = lib.get("standard");
    expect(b).toBe(a);
    expect(b.face).toBe(a.face);
    expect(b.edge).toBe(a.edge);
    expect(b.edgeWire).toBe(a.edgeWire);
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
 * Edges are FAT lines (LineSegments2/LineMaterial), because WebGL ignores
 * LineBasicMaterial.linewidth — a plain line is stuck at one device pixel, which
 * on a HiDPI display is a half-CSS-pixel hairline. The two edge materials are a
 * pair: the mode's `edgeStyle` picks between them (renderModes.ts).
 */
describe("BodyMaterialSet edge materials", () => {
  it("are LineMaterials at the shared body-edge width, in the OPAQUE pass", () => {
    const lib = new BodyMaterialLibrary();
    const { edge, edgeWire } = lib.get("standard");

    for (const m of [edge, edgeWire]) {
      expect(m).toBeInstanceOf(LineMaterial);
      expect(m.linewidth).toBe(BODY_EDGE_WIDTH);
      expect(m.toneMapped).toBe(false);
      // NOT transparent: body edges belong in the opaque pass, where they still
      // depth-sort against the bodies they outline.
      expect(m.transparent).toBe(false);
    }
    expect(edge).not.toBe(edgeWire);
    lib.dispose();
  });

  it("edge is near-black in BOTH themes; edgeWire inverts", () => {
    const lib = new BodyMaterialLibrary();
    setTheme("light");
    const lightEdge = lib.get("standard").edge.color.getHex();
    const lightWire = lib.get("standard").edgeWire.color.getHex();

    setTheme("dark");
    lib.refreshColors();
    const set = lib.get("standard");

    expect(set.edge.color.getHex()).toBe(palette.bodyEdge().getHex());
    expect(set.edgeWire.color.getHex()).toBe(palette.bodyEdgeWire().getHex());
    // The outline token barely moves (near-black either way); the wireframe one
    // must flip to stay legible on a dark canvas.
    expect(set.edgeWire.color.getHex()).not.toBe(lightWire);
    expect(set.edge.color.r).toBeLessThan(0.1);
    expect(new THREE.Color(lightEdge).r).toBeLessThan(0.1);
    lib.dispose();
  });

  /*
   * The inactive-material trap (DragHandle matNormal/matHover precedent): a mode
   * SWAPS which edge material the lines point at, so a refresh that only touched
   * the one currently in use would look correct until the user hit the display
   * button, then show the previous theme.
   */
  it("refreshColors updates the material the current mode is NOT using", () => {
    const lib = new BodyMaterialLibrary();
    setTheme("light");
    const set = lib.get("standard");
    const lightWire = set.edgeWire.color.getHex();

    setTheme("dark");
    lib.refreshColors();

    expect(set.edgeWire.color.getHex()).not.toBe(lightWire);
    lib.dispose();
  });

  it("a set created AFTER a theme flip is born with both new colors", () => {
    const lib = new BodyMaterialLibrary();
    setTheme("dark");
    const set = lib.get("standard");
    expect(set.edge.color.getHex()).toBe(palette.bodyEdge().getHex());
    expect(set.edgeWire.color.getHex()).toBe(palette.bodyEdgeWire().getHex());
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
  it("disposes every material of every created set", () => {
    const lib = new BodyMaterialLibrary();
    const { face, edge, edgeWire } = lib.get("standard");
    const faceSpy = vi.spyOn(face, "dispose");
    const edgeSpy = vi.spyOn(edge, "dispose");
    const wireSpy = vi.spyOn(edgeWire, "dispose");

    lib.dispose();

    expect(faceSpy).toHaveBeenCalled();
    expect(edgeSpy).toHaveBeenCalled();
    expect(wireSpy).toHaveBeenCalled();
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

describe("BodyMaterialLibrary.setClippingPlanes (section view)", () => {
  const plane = () => [new THREE.Plane(new THREE.Vector3(0, 0, -1), 0)];

  it("reaches the face AND both edge materials of every live set", () => {
    const lib = new BodyMaterialLibrary();
    const standard = lib.get("standard");
    const vertex = lib.get("shadedVertex");
    const planes = plane();

    lib.setClippingPlanes(planes);

    for (const set of [standard, vertex]) {
      // edgeWire is the trap: the mode not currently drawn still has to be
      // clipped, or pressing the wireframe button un-cuts the model.
      expect(set.face.clippingPlanes).toBe(planes);
      expect(set.edge.clippingPlanes).toBe(planes);
      expect(set.edgeWire.clippingPlanes).toBe(planes);
    }
    lib.dispose();
  });

  it("a set built AFTER the section was enabled is born clipped", () => {
    const lib = new BodyMaterialLibrary();
    const planes = plane();
    lib.setClippingPlanes(planes);

    // A render-mode switch reaches a kind for the first time…
    const late = lib.get("shadedVertex");
    expect(late.face.clippingPlanes).toBe(planes);
    expect(late.edgeWire.clippingPlanes).toBe(planes);

    // …and so does an assembly-colored body arriving mid-section (the LAZY map).
    const assembly = lib.getAssemblyColor("body-late");
    expect(assembly.face.clippingPlanes).toBe(planes);
    expect(assembly.edge.clippingPlanes).toBe(planes);
    expect(assembly.edgeWire.clippingPlanes).toBe(planes);
    lib.dispose();
  });

  it("reaches an assembly set created BEFORE the section, and null unclips everything", () => {
    const lib = new BodyMaterialLibrary();
    const shared = lib.get("standard");
    const assembly = lib.getAssemblyColor("body1");
    const planes = plane();

    lib.setClippingPlanes(planes);
    expect(assembly.face.clippingPlanes).toBe(planes);

    lib.setClippingPlanes(null);
    for (const set of [shared, assembly]) {
      expect(set.face.clippingPlanes).toBeNull();
      expect(set.edge.clippingPlanes).toBeNull();
      expect(set.edgeWire.clippingPlanes).toBeNull();
    }
    lib.dispose();
  });

  it("recompiles ONLY when the plane count changes — a drag tick writes no material", () => {
    const lib = new BodyMaterialLibrary();
    const { face, edge, edgeWire } = lib.get("standard");
    const planes = plane();

    lib.setClippingPlanes(planes); // 0 → 1 plane: the shader bakes the count
    const afterFirst = [face.version, edge.version, edgeWire.version];

    // Five more applications of the SAME array — what an offset drag does, once
    // per tick, for every live set. The plane itself is mutated in place and is
    // read as a uniform, so nothing here may recompile.
    for (let i = 0; i < 5; i++) lib.setClippingPlanes(planes);
    expect([face.version, edge.version, edgeWire.version]).toEqual(afterFirst);

    // …and clearing them (1 → 0) does bump it again, so the guard is a COUNT
    // check and not a blanket "never recompile".
    lib.setClippingPlanes(null);
    expect(face.version).toBeGreaterThan(afterFirst[0]);
    lib.dispose();
  });

  it("keeps the assembly body color and the dim state it was already carrying", () => {
    const lib = new BodyMaterialLibrary();
    lib.setDimmed(true);
    const assembly = lib.getAssemblyColor("body1");
    const colorHex = assembly.face.color.getHex();

    lib.setClippingPlanes(plane());

    expect(assembly.face.color.getHex()).toBe(colorHex);
    expect(assembly.face.transparent).toBe(true);
    lib.dispose();
  });
});
