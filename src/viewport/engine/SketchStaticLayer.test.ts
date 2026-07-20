/*
 * SketchStaticLayer: builds curves + dots (+ fill), applies the plane basis,
 * resolves a sketch id from a raycast immediately (self-flushes world matrices),
 * tints on hover/selection, hides the edited sketch, and disposes cleanly.
 * THREE is real (jsdom-safe geometry); no renderer.
 */
import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
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
  regionId: "r",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [-10, -10, 10, -10, 10, 10, -10, 10], indices: [0, 1, 2, 0, 2, 3] },
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

describe("SketchStaticLayer.setSketch", () => {
  it("builds curves + dots + fill and applies the plane basis matrix", () => {
    const { layer, sketchRoot } = makeLayer();
    layer.setSketch("s1", { plane: planeFor("XZ"), entities: RECT, regions: [REGION] });

    const g = groupFor(sketchRoot, "s1");
    const lines = childOfType<THREE.LineSegments>(g, "LineSegments");
    const points = childOfType<THREE.Points>(g, "Points");
    const fill = childOfType<THREE.Mesh>(g, "Mesh");
    expect(lines).toBeDefined();
    expect(points).toBeDefined();
    expect(fill).toBeDefined();

    // 4 lines → 4 segments → 8 vertices; 8 endpoints (2 per line).
    expect(lines!.geometry.getAttribute("position").count).toBe(8);
    expect(points!.geometry.getAttribute("position").count).toBe(8);
    // Fill: 4 verts (u,v)→(x,y,0), 2 triangles.
    expect(fill!.geometry.getAttribute("position").count).toBe(4);
    expect(fill!.geometry.getIndex()!.count).toBe(6);

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
});

describe("SketchStaticLayer.hitTest", () => {
  it("resolves the sketch id from a fill raycast immediately (no manual updateMatrixWorld)", () => {
    const { layer } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [REGION] });
    const ray = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
    expect(layer.hitTest(ray)).toBe("s1");
  });

  it("resolves a curve hit within the Line threshold when there is no fill", () => {
    const { layer } = makeLayer();
    layer.setSketch("s1", {
      plane: IDENTITY_PLANE,
      entities: [{ id: "e1", type: "Line", p0: [-10, 0], p1: [10, 0] }],
      regions: [],
    });
    const ray = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
    ray.params.Line = { threshold: 1 };
    expect(layer.hitTest(ray)).toBe("s1");
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
  it("tints curves + fill on hover and selection (selection wins)", () => {
    const { layer, sketchRoot } = makeLayer();
    layer.setSketch("s1", { plane: IDENTITY_PLANE, entities: RECT, regions: [REGION] });
    const g = groupFor(sketchRoot, "s1");
    const mat = (childOfType<THREE.LineSegments>(g, "LineSegments")!.material as THREE.LineBasicMaterial);
    const fillMat = (childOfType<THREE.Mesh>(g, "Mesh")!.material as THREE.MeshBasicMaterial);

    expect(mat.color.getHex()).toBe(palette.sketchFull().getHex());
    expect(fillMat.opacity).toBeCloseTo(0.18, 5);

    layer.setHover("s1");
    expect(mat.color.getHex()).toBe(palette.hoverAccent().getHex());
    expect(fillMat.opacity).toBeCloseTo(0.3, 5);

    layer.setSelected(["s1"]);
    expect(mat.color.getHex()).toBe(palette.sketchSelected().getHex()); // selection wins over hover

    layer.setHover(null);
    layer.setSelected([]);
    expect(mat.color.getHex()).toBe(palette.sketchFull().getHex());
    expect(fillMat.opacity).toBeCloseTo(0.18, 5);
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
});
