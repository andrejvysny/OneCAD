/*
 * SketchStaticLayer: builds curves + dots (+ fill), applies the plane basis,
 * resolves a sketch id from a raycast immediately (self-flushes world matrices),
 * tints on hover/selection, hides the edited sketch, and disposes cleanly.
 * THREE is real (jsdom-safe geometry); no renderer.
 */
import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import type { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { SketchStaticLayer } from "./SketchStaticLayer";
import { palette } from "./palette";
import { planeFor } from "@/ipc/mockSketch";
import type { SketchEntity, SketchPlane, SketchRegion } from "@/ipc/types";

const IDENTITY_PLANE: SketchPlane = {
  kind: "custom",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

const RECT: SketchEntity[] = [
  { id: "e1", type: "Line", p0: [-10, -10], p1: [10, -10] },
  { id: "e2", type: "Line", p0: [10, -10], p1: [10, 10] },
  { id: "e3", type: "Line", p0: [10, 10], p1: [-10, 10] },
  { id: "e4", type: "Line", p0: [-10, 10], p1: [-10, -10] },
];

// A square profile fill covering the origin (two triangles), plane-local (u,v).
const REGION: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [-10, -10, 10, -10, 10, 10, -10, 10], indices: [0, 1, 2, 0, 2, 3] },
};

const REGION_RIGHT: SketchRegion = {
  regionId: "r1",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [20, -10, 40, -10, 40, 10, 20, 10], indices: [0, 1, 2, 0, 2, 3] },
};

function makeLayer() {
  const sketchRoot = new THREE.Group();
  const invalidate = vi.fn();
  const layer = new SketchStaticLayer({ sketchRoot, invalidate });
  return { layer, sketchRoot, invalidate };
}

/** The layer's own root group (sole child of sketchRoot). */
function layerRoot(sketchRoot: THREE.Group): THREE.Group {
  return sketchRoot.children[0] as THREE.Group;
}

function groupFor(sketchRoot: THREE.Group, id: string): THREE.Group {
  const g = layerRoot(sketchRoot).children.find((c) => c.name === `sketchStatic_${id}`);
  if (!g) throw new Error(`no group for ${id}`);
  return g as THREE.Group;
}

const childOfType = <T extends THREE.Object3D>(g: THREE.Object3D, type: string): T | undefined =>
  g.children.find((c) => c.type === type) as T | undefined;

function fillFor(g: THREE.Object3D, regionId: string): THREE.Mesh {
  const fill = g.children.find((c) => c.userData.regionId === regionId);
  if (!fill) throw new Error(`no fill for ${regionId}`);
  return fill as THREE.Mesh;
}

describe("SketchStaticLayer.setSketch", () => {
  it("builds curves + dots + one fill per region and applies the plane basis matrix", () => {
    const { layer, sketchRoot } = makeLayer();
    layer.setSketch("s1", {
      plane: planeFor("XZ"),
      entities: RECT,
      regions: [REGION, REGION_RIGHT],
    });

    const g = groupFor(sketchRoot, "s1");
    const lines = childOfType<THREE.LineSegments>(g, "LineSegments");
    const drawLines = g.children.find((c) => c instanceof LineSegments2) as LineSegments2 | undefined;
    const points = childOfType<THREE.Points>(g, "Points");
    const fills = g.children.filter((c) => c.type === "Mesh") as THREE.Mesh[];
    expect(lines).toBeDefined();
    expect(points).toBeDefined();
    expect(fills).toHaveLength(2);
    expect(fills.map((fill) => fill.userData.regionId)).toEqual(["r0", "r1"]);

    // 4 lines → 4 segments → 8 vertices; 8 endpoints (2 per line).
    expect(lines!.geometry.getAttribute("position").count).toBe(8);
    expect(points!.geometry.getAttribute("position").count).toBe(8);

    // Pick proxy is invisible (zero GPU cost); the fat LineSegments2 is what draws.
    expect(lines!.visible).toBe(false);
    expect(drawLines).toBeDefined();
    expect(drawLines).toBeInstanceOf(LineSegments2);
    // Each fill: 4 verts (u,v)→(x,y,0), 2 triangles.
    expect(fills[0].geometry.getAttribute("position").count).toBe(4);
    expect(fills[0].geometry.getIndex()!.count).toBe(6);

    // Local x/y map to the plane's world xAxis/yAxis (basis applied).
    const plane = planeFor("XZ");
    const x = new THREE.Vector3(1, 0, 0).applyMatrix4(g.matrix);
    const y = new THREE.Vector3(0, 1, 0).applyMatrix4(g.matrix);
    expect(x.toArray().map((n) => Math.round(n))).toEqual(plane.xAxis.map((n) => Math.round(n)));
    expect(y.toArray().map((n) => Math.round(n))).toEqual(plane.yAxis.map((n) => Math.round(n)));
  });

  it("omits the fill mesh when there are no regions (curves only)", () => {
    const { layer, sketchRoot } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [] });
    expect(childOfType(groupFor(sketchRoot, "s1"), "Mesh")).toBeUndefined();
  });

  it("omits a region whose fill did not subtract every declared hole", () => {
    const { layer, sketchRoot } = makeLayer();
    const incomplete: SketchRegion = {
      ...REGION,
      holes: [["hole"]],
      previewTriangles: { ...REGION.previewTriangles!, holesSubtracted: 0 },
    };
    layer.setSketch("s1", {
      plane: IDENTITY_PLANE,
      entities: RECT,
      regions: [incomplete],
    });
    expect(childOfType(groupFor(sketchRoot, "s1"), "Mesh")).toBeUndefined();
  });
});

describe("SketchStaticLayer.hitTest", () => {
  it("resolves the exact region from a fill immediately (no manual updateMatrixWorld)", () => {
    const { layer } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [REGION] });
    const ray = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
    expect(layer.hitTest(ray)).toEqual({
      kind: "sketchRegion",
      sketchId: "s1",
      regionId: "r0",
      distance: 50,
    });
  });

  it("returns a non-first region's exact identity", () => {
    const { layer } = makeLayer();
    layer.setSketch("s1", {
      plane: IDENTITY_PLANE,
      entities: RECT,
      regions: [REGION, REGION_RIGHT],
    });
    const ray = new THREE.Raycaster(new THREE.Vector3(30, 0, 50), new THREE.Vector3(0, 0, -1));
    expect(layer.hitTest(ray)).toEqual({
      kind: "sketchRegion",
      sketchId: "s1",
      regionId: "r1",
      distance: 50,
    });
  });

  it("resolves a whole-sketch curve hit within the Line threshold when there is no fill", () => {
    const { layer } = makeLayer();
    layer.setSketch("s1", {
      plane: IDENTITY_PLANE,
      entities: [{ id: "e1", type: "Line", p0: [-10, 0], p1: [10, 0] }],
      regions: [],
    });
    const ray = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
    ray.params.Line = { threshold: 1 };
    expect(layer.hitTest(ray)).toEqual({ kind: "sketch", sketchId: "s1", distance: 50 });
  });

  it("keeps a closed profile boundary selectable as the whole sketch", () => {
    const { layer } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [REGION] });
    const ray = new THREE.Raycaster(
      new THREE.Vector3(-10, 0, 50),
      new THREE.Vector3(0, 0, -1),
    );
    ray.params.Line = { threshold: 1 };
    expect(layer.hitTest(ray)).toEqual({ kind: "sketch", sketchId: "s1", distance: 50 });
  });

  it("returns null when nothing is under the ray", () => {
    const { layer } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [REGION] });
    const ray = new THREE.Raycaster(new THREE.Vector3(500, 500, 50), new THREE.Vector3(0, 0, -1));
    expect(layer.hitTest(ray)).toBeNull();
  });

  it("ignores an edited (hidden) sketch", () => {
    const { layer } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [REGION] });
    layer.setEditingSketch("s1");
    const ray = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
    expect(layer.hitTest(ray)).toBeNull();
  });
});

describe("SketchStaticLayer tint", () => {
  it("tints exact regions independently; whole-sketch selection still tints all", () => {
    const { layer, sketchRoot } = makeLayer();
    layer.setSketch("s1", {
      plane: IDENTITY_PLANE,
      entities: RECT,
      regions: [REGION, REGION_RIGHT],
    });
    const g = groupFor(sketchRoot, "s1");
    const mat = (childOfType<THREE.LineSegments>(g, "LineSegments")!.material as THREE.LineBasicMaterial);
    const drawMat = (g.children.find((c) => c instanceof LineSegments2) as LineSegments2).material as LineMaterial;
    const fill0 = fillFor(g, "r0").material as THREE.MeshBasicMaterial;
    const fill1 = fillFor(g, "r1").material as THREE.MeshBasicMaterial;

    // Model-mode sketches are ALWAYS blue (sketchUnder), regardless of constraint
    // status — this layer never renders sketchFull; both the pick-proxy material
    // and the visible LineSegments2 draw material stay in lockstep.
    expect(mat.color.getHex()).toBe(palette.sketchUnder().getHex());
    expect(drawMat.color.getHex()).toBe(palette.sketchUnder().getHex());
    expect(fill0.opacity).toBeCloseTo(0.18, 5);
    expect(fill1.opacity).toBeCloseTo(0.18, 5);

    layer.setHover({ kind: "sketchRegion", sketchId: "s1", regionId: "r1" });
    expect(mat.color.getHex()).toBe(palette.sketchUnder().getHex());
    expect(fill0.opacity).toBeCloseTo(0.18, 5);
    expect(fill1.opacity).toBeCloseTo(0.3, 5);

    layer.setSelected([{ kind: "sketchRegion", sketchId: "s1", regionId: "r0" }]);
    expect(fill0.color.getHex()).toBe(palette.sketchSelected().getHex());
    expect(fill1.color.getHex()).toBe(palette.hover3d().getHex());

    layer.setHover(null);
    layer.setSelected([]);
    expect(mat.color.getHex()).toBe(palette.sketchUnder().getHex());
    expect(fill0.opacity).toBeCloseTo(0.18, 5);
    expect(fill1.opacity).toBeCloseTo(0.18, 5);

    layer.setSelected([{ kind: "sketch", sketchId: "s1" }]);
    expect(mat.color.getHex()).toBe(palette.sketchSelected().getHex());
    expect(drawMat.color.getHex()).toBe(palette.sketchSelected().getHex());
    expect(fill0.color.getHex()).toBe(palette.sketchSelected().getHex());
    expect(fill1.color.getHex()).toBe(palette.sketchSelected().getHex());
  });

  it("tints hover on the whole sketch as cyan (hover3d), matching the DRAW material", () => {
    const { layer, sketchRoot } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [] });
    const g = groupFor(sketchRoot, "s1");
    const drawMat = (g.children.find((c) => c instanceof LineSegments2) as LineSegments2).material as LineMaterial;

    layer.setHover({ kind: "sketch", sketchId: "s1" });
    expect(drawMat.color.getHex()).toBe(palette.hover3d().getHex());
  });
});

describe("SketchStaticLayer visibility", () => {
  it("hides the edited sketch group and restores it, respecting the editing override", () => {
    const { layer, sketchRoot } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [REGION] });
    const g = groupFor(sketchRoot, "s1");
    expect(g.visible).toBe(true);

    layer.setEditingSketch("s1");
    expect(g.visible).toBe(false);

    layer.setVisible("s1", true); // still hidden while it is the edited sketch
    expect(g.visible).toBe(false);

    layer.setEditingSketch(null);
    expect(g.visible).toBe(true);

    layer.setVisible("s1", false); // tree eye off
    expect(g.visible).toBe(false);
  });

  it("preserves the intended visibility across a rebuild", () => {
    const { layer, sketchRoot } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [] });
    layer.setVisible("s1", false);
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [REGION] }); // rebuild
    expect(groupFor(sketchRoot, "s1").visible).toBe(false);
  });
});

describe("SketchStaticLayer teardown", () => {
  it("removeSketch drops the group; dispose empties + detaches the root", () => {
    const { layer, sketchRoot } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [REGION] });
    layer.setSketch("s2", { plane: IDENTITY_PLANE, entities: RECT, regions: [] });

    const lines = childOfType<THREE.LineSegments>(groupFor(sketchRoot, "s1"), "LineSegments")!;
    const geoSpy = vi.spyOn(lines.geometry, "dispose");

    expect(layerRoot(sketchRoot).children.length).toBe(2);
    layer.removeSketch("s1");
    expect(layerRoot(sketchRoot).children.length).toBe(1);
    expect(geoSpy).toHaveBeenCalledOnce();

    layer.dispose();
    expect(sketchRoot.children.length).toBe(0);
  });

  // ── Locked reference geometry renders like any other curve (W2) ────────────

  it("renders the projected host-face boundary of a NON-ACTIVE sketch", () => {
    // A sketch-on-face sketch that is not being edited is drawn by this layer, and
    // its projected boundary is a real part of the sketch — dropping it would make
    // the outline vanish the moment the user leaves the sketch. This layer applies
    // no per-entity styling at all (construction geometry is likewise undifferentiated
    // here), so "renders" is the whole contract: same segments, same material.
    const { layer, sketchRoot } = makeLayer();
    const locked = RECT.map((e) => ({ ...e, referenceLocked: true }));
    layer.setSketch("plain", { plane: IDENTITY_PLANE, entities: RECT, regions: [] });
    layer.setSketch("onFace", { plane: IDENTITY_PLANE, entities: locked, regions: [] });

    const seg = (id: string): number =>
      childOfType<THREE.LineSegments>(groupFor(sketchRoot, id), "LineSegments")!.geometry.getAttribute("position")
        .count;
    const dots = (id: string): number =>
      childOfType<THREE.Points>(groupFor(sketchRoot, id), "Points")!.geometry.getAttribute("position").count;

    expect(seg("onFace")).toBe(seg("plain"));
    expect(seg("onFace")).toBeGreaterThan(0);
    expect(dots("onFace")).toBe(dots("plain"));

    layer.dispose();
  });
});
