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
import { PreviewMesh } from "./PreviewMesh";
import { RevolvePreview } from "./RevolvePreview";
import { BodyMaterialLibrary } from "./bodyMaterials";
import { buildBodyObjects, swap, disposeAll, __resetRegistryForTests } from "../mesh/meshRegistry";
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

/** Hexes that exist ONLY in light — seeing one after a dark refresh is stale. */
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
    const lightEdge = palette.bodyEdge().getHex();

    // Flip the attribute ONLY — deliberately no resetPaletteCache(). This is the
    // shape of every ordering bug: something reads the palette at a moment the
    // reset has not run (engine mid-construction, a module evaluated before the
    // theme was stamped). A flat cache would hand back the light values here.
    document.documentElement.dataset.theme = "dark";

    expect(palette.clear().getHex()).not.toBe(lightCanvas);
    expect(palette.bodyEdge().getHex()).not.toBe(lightEdge);
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
    const lightEdge = palette.bodyEdge().getHex();
    const lightInk = palette.sketchFull().getHex();
    const lightCanvas = palette.clear().getHex();

    setTheme("dark");

    expect(palette.bodyEdge().getHex()).not.toBe(lightEdge);
    expect(palette.sketchFull().getHex()).not.toBe(lightInk);
    expect(palette.clear().getHex()).not.toBe(lightCanvas);
  });

  it("body edges go LIGHT in dark mode — wireframe depends on it", () => {
    setTheme("dark");
    const edge = palette.bodyEdge();
    // Perceptually light: every channel well above mid.
    expect(edge.r).toBeGreaterThan(0.6);
    expect(edge.g).toBeGreaterThan(0.6);
    expect(edge.b).toBeGreaterThan(0.6);
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
    layer.setState({ kind: "body", id: "hover-body" }, [{ kind: "body", id: "body1" }]);

    setTheme("dark");
    layer.refreshColors();

    expectNoLightLeftovers(d.root, lightOnly);
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

describe("BodyMaterialLibrary.refreshColors", () => {
  beforeEach(() => setTheme("light"));
  afterEach(() => setTheme("light"));

  it("re-reads both face and edge for every live set", () => {
    const lib = new BodyMaterialLibrary();
    const set = lib.get("standard");
    expect(set.face.color.getHex()).toBe(palette.bodyNeutral().getHex());
    expect(set.edge.color.getHex()).toBe(palette.bodyEdge().getHex());

    setTheme("dark");
    lib.refreshColors();

    expect(set.face.color.getHex()).toBe(palette.bodyNeutral().getHex());
    expect(set.edge.color.getHex()).toBe(palette.bodyEdge().getHex());
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
