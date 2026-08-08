/*
 * The refreshColors() invariant, layer by layer.
 *
 * A layer that reads `palette` but forgets refreshColors() fails SILENTLY —
 * nothing throws, the layer simply keeps the old theme's colors until something
 * else happens to rebuild it. These tests are the only thing standing between
 * that and shipping.
 *
 * They work because palette's jsdom fallback table is per-theme and selected
 * from `data-theme` on <html> (see palette.ts): flip the attribute, drop the
 * cache, and every getter returns the dark value even though no CSS is loaded.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as THREE from "three";
import { palette, resetPaletteCache } from "./palette";
import { GridPlane } from "./GridPlane";
import { OriginTriad } from "./OriginTriad";
import { HighlightLayer } from "./HighlightLayer";
import { GhostLayer } from "./GhostLayer";
import { DragHandle } from "./DragHandle";
import { TransformGizmo } from "./TransformGizmo";
import { PreviewMesh } from "./PreviewMesh";
import { RevolvePreview } from "./RevolvePreview";
import { DatumLayer, datumGhostPlane } from "./DatumLayer";
import { BodyMaterialLibrary } from "./bodyMaterials";
import {
  buildBodyObjects,
  swap,
  getEntry,
  refreshFaceColors,
  disposeAll,
  __resetRegistryForTests,
} from "../mesh/meshRegistry";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import { makeBoxMesh } from "@/ipc/mockMeshes";

/** Put the document in `theme` and drop the cache so getters re-resolve. */
function setTheme(theme: "light" | "dark"): void {
  document.documentElement.dataset.theme = theme;
  resetPaletteCache();
}

const deps = () => ({ root: new THREE.Group(), invalidate: vi.fn() });

/** Every color the palette can currently produce, as hex. */
function paletteHexes(): Set<number> {
  const out = new Set<number>();
  for (const getter of Object.values(palette)) {
    out.add((getter as () => THREE.Color)().getHex());
  }
  return out;
}

/**
 * Hexes that exist ONLY in light — seeing one after a dark refresh is stale.
 *
 * Tokens that are now theme-identical (e.g. bodyEdge, near-black in both
 * themes) contribute nothing to this set, shrinking it and weakening
 * expectNoLightLeftovers: a stale near-black edge left over from light mode is
 * indistinguishable from a fresh dark-mode one, so this check alone cannot
 * catch that class of staleness for those tokens.
 */
function lightOnlyHexes(): Set<number> {
  setTheme("light");
  const light = paletteHexes();
  setTheme("dark");
  const dark = paletteHexes();
  for (const h of dark) light.delete(h);
  return light;
}

/** Minimal world-XY plane + unit-square prism, enough to attach real meshes. */
const XY_PLANE = {
  kind: "XY" as const,
  origin: [0, 0, 0] as [number, number, number],
  xAxis: [1, 0, 0] as [number, number, number],
  yAxis: [0, 1, 0] as [number, number, number],
  normal: [0, 0, 1] as [number, number, number],
};

const UNIT_PROFILE = {
  ring: [
    [0, 0],
    [1, 0],
    [1, 1],
    [0, 1],
  ] as [number, number][],
  holes: [],
  cap: { positions: [0, 0, 1, 0, 1, 1, 0, 1], indices: [0, 1, 2, 0, 2, 3] },
};

/** Every material color in a subtree. */
function subtreeColors(root: THREE.Object3D): number[] {
  const out: number[] = [];
  root.traverse((o) => {
    const m = (o as THREE.Mesh).material;
    if (!m) return;
    for (const mat of Array.isArray(m) ? m : [m]) {
      const c = (mat as THREE.Material & { color?: THREE.Color }).color;
      if (c) out.push(c.getHex());
    }
  });
  return out;
}

/**
 * The generic staleness assertion: after refreshing into dark, no material in
 * the layer may still hold a color that exists only in the light palette.
 * Layer-agnostic, and tolerant of non-palette colors (an invisible pick
 * target's default white, say).
 *
 * The non-vacuity guard is NOT optional. Most of these layers keep their shared
 * materials off the scene graph until something is actually drawn with them, so
 * a traversal of an idle layer finds zero colors and "passes" while proving
 * nothing — verified by neutering HighlightLayer.refreshColors() and watching
 * the test stay green. Every caller must attach real content first.
 */
function expectNoLightLeftovers(root: THREE.Object3D, lightOnly: Set<number>): void {
  const colors = subtreeColors(root);
  expect(colors.length, "nothing drawn — the check would be vacuous").toBeGreaterThan(0);
  const stale = colors.filter((c) => lightOnly.has(c));
  expect(stale.map((c) => `#${c.toString(16).padStart(6, "0")}`)).toEqual([]);
}

describe("palette cache is keyed by theme", () => {
  beforeEach(() => setTheme("light"));
  afterEach(() => setTheme("light"));

  it("a read under dark never returns a light color, even with NO cache reset", () => {
    // Populate the cache under light.
    const lightCanvas = palette.clear().getHex();
    const lightEdgeWire = palette.bodyEdgeWire().getHex();

    // Flip the attribute ONLY — deliberately no resetPaletteCache(). This is the
    // shape of every ordering bug: something reads the palette at a moment the
    // reset has not run (engine mid-construction, a module evaluated before the
    // theme was stamped). A flat cache would hand back the light values here.
    document.documentElement.dataset.theme = "dark";

    expect(palette.clear().getHex()).not.toBe(lightCanvas);
    // bodyEdgeWire is the token that still inverts per-theme (bodyEdge is now
    // near-black in both themes, see below).
    expect(palette.bodyEdgeWire().getHex()).not.toBe(lightEdgeWire);
  });

  it("returns a STABLE instance per theme, so .copy() consumers stay cheap", () => {
    const a = palette.clear();
    const b = palette.clear();
    expect(a).toBe(b);
  });

  it("hands back distinct instances per theme (never mutates the light one)", () => {
    const light = palette.clear();
    document.documentElement.dataset.theme = "dark";
    const dark = palette.clear();

    expect(dark).not.toBe(light);
    // Going back must find the ORIGINAL light instance, unmodified.
    document.documentElement.dataset.theme = "light";
    expect(palette.clear()).toBe(light);
  });

  it("resetPaletteCache still forces a re-read (token values changed under a fixed theme)", () => {
    const before = palette.clear();
    resetPaletteCache();
    expect(palette.clear()).not.toBe(before);
    expect(palette.clear().getHex()).toBe(before.getHex());
  });
});

describe("palette is theme-aware in jsdom", () => {
  beforeEach(() => setTheme("light"));
  afterEach(() => setTheme("light"));

  it("returns different colors per theme for the tokens that invert", () => {
    const lightEdgeWire = palette.bodyEdgeWire().getHex();
    const lightInk = palette.sketchFull().getHex();
    const lightCanvas = palette.clear().getHex();

    setTheme("dark");

    expect(palette.bodyEdgeWire().getHex()).not.toBe(lightEdgeWire);
    expect(palette.sketchFull().getHex()).not.toBe(lightInk);
    expect(palette.clear().getHex()).not.toBe(lightCanvas);
  });

  it("wireframe edges go LIGHT in dark mode — wireframe render mode depends on it", () => {
    setTheme("dark");
    const edge = palette.bodyEdgeWire();
    // Perceptually light: every channel well above mid.
    expect(edge.r).toBeGreaterThan(0.6);
    expect(edge.g).toBeGreaterThan(0.6);
    expect(edge.b).toBeGreaterThan(0.6);
  });

  it("body edge is near-black in BOTH themes — shaded+edges mode, Shapr3D convention", () => {
    for (const theme of ["light", "dark"] as const) {
      setTheme(theme);
      const edge = palette.bodyEdge();
      // three.js ColorManagement converts sRGB -> linear, so these near-black
      // token values (see tokens.css --color-body-edge) land well under 0.1 linear.
      expect(edge.r).toBeLessThan(0.1);
      expect(edge.g).toBeLessThan(0.1);
      expect(edge.b).toBeLessThan(0.1);
    }
  });

  it("body edge stays clear of the body fill in BOTH themes", () => {
    for (const theme of ["light", "dark"] as const) {
      setTheme(theme);
      const fill = palette.bodyNeutral();
      const edge = palette.bodyEdge();
      const delta = Math.abs(fill.r - edge.r) + Math.abs(fill.g - edge.g) + Math.abs(fill.b - edge.b);
      expect(delta).toBeGreaterThan(0.5);
    }
  });

  it("traffic-light close is a macOS constant — identical in both themes", () => {
    setTheme("light");
    const lightHex = palette.destructive().getHex();
    setTheme("dark");
    expect(palette.destructive().getHex()).toBe(lightHex);
  });
});

describe("refreshColors picks up a theme flip", () => {
  beforeEach(() => setTheme("light"));
  afterEach(() => {
    setTheme("light");
    disposeAll();
    __resetRegistryForTests();
  });

  it("GridPlane re-bakes the vertex fade against the new background", () => {
    const grid = new GridPlane({
      minor: palette.gridMinor(),
      major: palette.gridMajor(),
      clear: palette.clear(),
    });
    // Build it: colors are only baked once there IS a step.
    grid.update(new THREE.Vector3(), 500);
    const majorMat = () =>
      (grid.object3D.children[1] as THREE.LineSegments).material as THREE.LineBasicMaterial;
    expect(majorMat().color.getHex()).toBe(palette.gridMajor().getHex());

    setTheme("dark");
    grid.setColors({
      minor: palette.gridMinor(),
      major: palette.gridMajor(),
      clear: palette.clear(),
    });

    expect(majorMat().color.getHex()).toBe(palette.gridMajor().getHex());
    grid.dispose();
  });

  it("GridPlane.setColors before any update does not throw", () => {
    const grid = new GridPlane({
      minor: palette.gridMinor(),
      major: palette.gridMajor(),
      clear: palette.clear(),
    });
    setTheme("dark");
    expect(() =>
      grid.setColors({
        minor: palette.gridMinor(),
        major: palette.gridMajor(),
        clear: palette.clear(),
      }),
    ).not.toThrow();
    grid.dispose();
  });

  it("OriginTriad rewrites its baked per-vertex axis colors", () => {
    const triad = new OriginTriad({ x: palette.axisX(), y: palette.axisY(), z: palette.axisZ() });
    const colorsOf = () => {
      const attr = triad.object3D.children[0] as THREE.Object3D & {
        geometry: THREE.BufferGeometry;
      };
      return Array.from(
        (attr.geometry.getAttribute("instanceColorStart") as THREE.BufferAttribute).array,
      );
    };
    const before = colorsOf();

    setTheme("dark");
    triad.setColors({ x: palette.axisX(), y: palette.axisY(), z: palette.axisZ() });

    expect(colorsOf()).not.toEqual(before);
    triad.dispose();
  });

  it("HighlightLayer leaves no light-theme color behind", () => {
    const lightOnly = lightOnlyHexes();
    setTheme("light");
    const d = deps();
    const layer = new HighlightLayer(d);
    // Real REGISTERED entry: the shared materials only reach the scene graph
    // via a built highlight, and addHighlight resolves the body through
    // getEntry() — buildBodyObjects alone returns an entry without registering
    // it, so the highlight would silently build nothing.
    swap("body1", buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1));
    // All FIVE shared materials at once: hover face + hover edge (cyan), the
    // per-element selected tint, the lighter whole-body tint, and the selected
    // edge. Anything left off this call is untested — the materials only reach
    // the scene graph through a built highlight.
    layer.setState({ kind: "face", id: "body1#f:1", bodyId: "body1", topoKey: "f:1" }, [
      { kind: "body", id: "body1" },
      { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" },
      { kind: "edge", id: "body1#e:0", bodyId: "body1", topoKey: "e:0" },
    ]);

    setTheme("dark");
    layer.refreshColors();

    expectNoLightLeftovers(d.root, lightOnly);

    // Hover is its own material and only appears while something is hovered —
    // rebuild under the new theme with an edge hovered so the cyan pair shows.
    layer.setState({ kind: "edge", id: "body1#e:1", bodyId: "body1", topoKey: "e:1" }, []);
    expectNoLightLeftovers(d.root, lightOnly);
    layer.dispose();
  });

  it("DatumLayer leaves no light-theme color behind", () => {
    // NEW with the viewport-contribution port. DatumLayer had NO case here, and
    // it is the one layer the host can no longer list in applyTheme() by hand:
    // it subscribes through `ViewportContext.onThemeChange`, so a forgotten
    // subscription is invisible to the engine. Negative-checked by neutering
    // `DatumLayer.refreshColors()` and confirming this goes red.
    const lightOnly = lightOnlyHexes();
    setTheme("light");
    const d = deps();
    const labels: HTMLElement[] = [];
    const layer = new DatumLayer({
      root: d.root,
      invalidate: d.invalidate,
      createLabel: (el) => {
        labels.push(el);
        return { setPosition: () => {}, dispose: () => el.remove() };
      },
    });
    // Both material families: a committed quad (fill + outline) AND the tool
    // ghost, which is built lazily and would otherwise never be traversed.
    layer.syncDatums([
      { id: "d1", name: "Base", plane: datumGhostPlane("XY", 10), resolvedValid: true },
    ]);
    layer.setGhost(datumGhostPlane("XY", 25), "XY");

    setTheme("dark");
    layer.refreshColors();

    expectNoLightLeftovers(d.root, lightOnly);
    layer.dispose();
  });

  it("GhostLayer leaves no light-theme color behind", () => {
    const lightOnly = lightOnlyHexes();
    setTheme("light");
    const d = deps();
    const layer = new GhostLayer(d);
    const entry = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1);
    layer.show(entry, [{ kind: "translate", offset: [10, 0, 0] }]);

    setTheme("dark");
    layer.refreshColors();

    expectNoLightLeftovers(d.root, lightOnly);
  });

  it("DragHandle refreshes BOTH materials, not just the visible one", () => {
    const lightOnly = lightOnlyHexes();
    setTheme("light");
    const d = deps();
    const handle = new DragHandle(d);

    setTheme("dark");
    handle.refreshColors();

    // Hover SWAPS materials rather than recoloring, so both states must be
    // current — check the subtree in each.
    expectNoLightLeftovers(d.root, lightOnly);
    handle.setHover(true);
    expectNoLightLeftovers(d.root, lightOnly);
  });

  it("TransformGizmo refreshes every handle, INCLUDING the highlighted one", () => {
    const lightOnly = lightOnlyHexes();
    setTheme("light");
    const d = deps();
    const gizmo = new TransformGizmo(d);
    gizmo.setVisible(true);
    // A theme can flip mid-gesture, so refresh under a live hover + active
    // highlight too: `applyColors` is the single writer of both, and a refresh
    // that only rewrote the resting state would leave the lit handle stale.
    gizmo.setHover({ kind: "ring", axis: "Z" });
    gizmo.setActive({ kind: "axis", axis: "X" });

    setTheme("dark");
    gizmo.refreshColors();

    expectNoLightLeftovers(d.root, lightOnly);
    gizmo.setActive(null);
    gizmo.setHover(null);
    expectNoLightLeftovers(d.root, lightOnly);
  });

  it("PreviewMesh leaves no light-theme color behind", () => {
    const lightOnly = lightOnlyHexes();
    setTheme("light");
    const d = deps();
    const mesh = new PreviewMesh(d);
    // Attach a real prism: the shared material is not in the scene graph until
    // there is something drawing with it.
    mesh.setProfile(XY_PLANE, UNIT_PROFILE);

    setTheme("dark");
    mesh.refreshColors();

    expectNoLightLeftovers(d.root, lightOnly);
  });

  it("PreviewMesh keeps a Cut tint across a theme flip", () => {
    setTheme("light");
    const d = deps();
    const mesh = new PreviewMesh(d);
    mesh.setProfile(XY_PLANE, UNIT_PROFILE);
    mesh.setTint(true); // Cut

    setTheme("dark");
    mesh.refreshColors();

    // destructive() is theme-stable, so finding it proves refreshColors did NOT
    // silently revert a live Cut preview to the Add accent.
    expect(subtreeColors(d.root)).toContain(palette.destructive().getHex());
    expect(subtreeColors(d.root)).not.toContain(palette.hoverAccent().getHex());
  });

  it("RevolvePreview refreshes the candidate + hover lines setTint never touches", () => {
    const lightOnly = lightOnlyHexes();
    setTheme("light");
    const d = deps();
    const preview = new RevolvePreview(d);
    preview.setPlane(XY_PLANE);
    // Revolve the unit square about the v axis (u = -1), well clear of it.
    preview.setLathe(UNIT_PROFILE.ring, { a: [-1, 0], b: [-1, 1] }, 90);
    preview.setTint(true);

    setTheme("dark");
    preview.refreshColors();

    expectNoLightLeftovers(d.root, lightOnly);
    // meshMat keeps its tint; candMat/hoverMat — which setTint never touches —
    // are the ones only refreshColors can rescue.
    expect(subtreeColors(d.root)).toContain(palette.destructive().getHex());
  });
});

/*
 * Baked FACE_COLORS are the "geometry rebuild" shape of the invariant, and the
 * nastiest one: the stale color lives in a VERTEX ATTRIBUTE, so every material
 * in the scene can be perfectly fresh while an imported body still wears the old
 * theme's neutral on the faces its STEP file left unset.
 *
 * Negative-checked: neutering `MeshEntry.rebakeFaceColors` (or dropping the
 * `refreshFaceColors()` call from MeshIngest.refreshColors) turns these red.
 */
describe("baked FACE_COLORS follow the theme", () => {
  const RED = [214, 74, 62, 255] as const;
  const coloredEntry = () =>
    buildBodyObjects(
      parseMeshPayload(makeBoxMesh(40, 40, 40, 0, [0, 0, 0], [RED, null, null, null, null, null])),
      "imported1",
      1,
    );

  /** The baked rgb triple of de-indexed vertex `i`. */
  const vertexColor = (entry: ReturnType<typeof coloredEntry>, i: number): number[] => {
    const a = entry.geometry.getAttribute("color").array;
    return [a[i * 3], a[i * 3 + 1], a[i * 3 + 2]];
  };
  /** The body token as it lands in a Float32 attribute. */
  const neutralTriple = (): number[] => {
    const n = palette.bodyNeutral();
    return [...Float32Array.from([n.r, n.g, n.b])];
  };
  /** The authored color as a material would resolve it (sRGB → working space). */
  const authoredTriple = (): number[] => {
    const c = new THREE.Color().setRGB(RED[0] / 255, RED[1] / 255, RED[2] / 255, THREE.SRGBColorSpace);
    return [...Float32Array.from([c.r, c.g, c.b])];
  };

  afterEach(() => {
    setTheme("light");
    disposeAll();
    __resetRegistryForTests();
  });

  it("rebakes the UNSET faces and leaves the authored ones alone", () => {
    setTheme("light");
    const entry = coloredEntry();
    // f:0 is authored (vertices 0–5); f:1 is unset (vertices 6–11).
    expect(vertexColor(entry, 6)).toEqual(neutralTriple());
    expect(vertexColor(entry, 0)).toEqual(authoredTriple());

    setTheme("dark");
    entry.rebakeFaceColors();

    expect(vertexColor(entry, 6)).toEqual(neutralTriple()); // now the DARK token
    expect(vertexColor(entry, 0)).toEqual(authoredTriple()); // data, unchanged
    entry.dispose();
  });

  it("without the rebake the unset face is left holding the LIGHT token", () => {
    // The non-vacuity guard: proves the assertion above can actually fail.
    setTheme("light");
    const entry = coloredEntry();
    const lightNeutral = neutralTriple();

    setTheme("dark");
    expect(neutralTriple()).not.toEqual(lightNeutral); // the token really moved
    expect(vertexColor(entry, 6)).toEqual(lightNeutral); // …and the bake is stale
    entry.dispose();
  });

  it("refreshFaceColors reaches every registered body", () => {
    setTheme("light");
    swap("imported1", coloredEntry());

    setTheme("dark");
    refreshFaceColors();

    const entry = getEntry("imported1")!;
    expect(vertexColor(entry as ReturnType<typeof coloredEntry>, 6)).toEqual(neutralTriple());
  });
});

describe("BodyMaterialLibrary.refreshColors", () => {
  beforeEach(() => setTheme("light"));
  afterEach(() => setTheme("light"));

  /*
   * BOTH edge materials, not just the active one. A render mode SWAPS which of
   * them the edges point at (renderModes `edgeStyle`), so this is the
   * DragHandle matNormal/matHover trap in another costume: refresh only the
   * material in use and the viewport looks correct until the user presses the
   * display-mode button, at which point it shows the previous theme.
   */
  it("re-reads face and BOTH edge materials for every live set", () => {
    const lib = new BodyMaterialLibrary();
    const set = lib.get("standard");
    expect(set.face.color.getHex()).toBe(palette.bodyNeutral().getHex());
    expect(set.edge.color.getHex()).toBe(palette.bodyEdge().getHex());
    const lightWire = set.edgeWire.color.getHex();
    expect(lightWire).toBe(palette.bodyEdgeWire().getHex());

    setTheme("dark");
    lib.refreshColors();

    expect(set.face.color.getHex()).toBe(palette.bodyNeutral().getHex());
    expect(set.edge.color.getHex()).toBe(palette.bodyEdge().getHex());
    expect(set.edgeWire.color.getHex()).toBe(palette.bodyEdgeWire().getHex());
    // Non-vacuity: bodyEdge is near-black in both themes, so only edgeWire can
    // actually prove a refresh happened at all.
    expect(set.edgeWire.color.getHex()).not.toBe(lightWire);
    lib.dispose();
  });

  it("PRESERVES an active Cut tint — a theme is not a state change", () => {
    const lib = new BodyMaterialLibrary();
    const set = lib.get("standard");
    const tint = new THREE.Color(0.9, 0.1, 0.1);
    lib.setFaceColor(tint);

    setTheme("dark");
    lib.refreshColors();

    expect(set.face.color.getHex()).toBe(tint.getHex());
    // ...while the edge, which no tint ever touches, DOES follow the theme.
    expect(set.edge.color.getHex()).toBe(palette.bodyEdge().getHex());
    lib.dispose();
  });

  it("resetFaceColor after a theme flip lands on the NEW neutral", () => {
    const lib = new BodyMaterialLibrary();
    const set = lib.get("standard");
    lib.setFaceColor(new THREE.Color(0.9, 0.1, 0.1));

    setTheme("dark");
    lib.resetFaceColor();

    expect(set.face.color.getHex()).toBe(palette.bodyNeutral().getHex());
    lib.dispose();
  });

  it("does not mutate the shared palette instance", () => {
    const lib = new BodyMaterialLibrary();
    const set = lib.get("standard");
    const before = palette.bodyEdge().getHex();
    set.edge.color.setRGB(0, 1, 0);
    expect(palette.bodyEdge().getHex()).toBe(before);
    lib.dispose();
  });

  it("a set created AFTER a theme flip is born with the new colors", () => {
    const lib = new BodyMaterialLibrary();
    setTheme("dark");
    const set = lib.get("standard");
    expect(set.edge.color.getHex()).toBe(palette.bodyEdge().getHex());
    lib.dispose();
  });
});
