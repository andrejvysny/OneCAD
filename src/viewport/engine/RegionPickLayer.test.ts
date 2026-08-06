/*
 * RegionPickLayer: builds one fill mesh per region from plane-local (u,v)
 * previewTriangles, applies the plane basis, tints on hover/selection (tokens,
 * no hex), and disposes cleanly. THREE is real (jsdom-safe geometry); no renderer.
 */
import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { RegionPickLayer } from "./RegionPickLayer";
import { palette } from "./palette";
import { planeFor } from "@/ipc/mockSketch";
import type { SketchPlane, SketchRegion } from "@/ipc/types";

const IDENTITY_PLANE: SketchPlane = {
  kind: "custom",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};
const R1: SketchRegion = {
  regionId: "r1",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [100, 100, 140, 100, 140, 140, 100, 140], indices: [0, 1, 2, 0, 2, 3] },
};

function makeLayer() {
  const root = new THREE.Group();
  const invalidate = vi.fn();
  const layer = new RegionPickLayer({ root, invalidate });
  return { layer, root, invalidate };
}

/** The layer's own group (sole child of root). */
function layerGroup(root: THREE.Group): THREE.Group {
  return root.children[0] as THREE.Group;
}

const meshes = (root: THREE.Group): THREE.Mesh[] =>
  layerGroup(root).children.filter((c) => c.type === "Mesh") as THREE.Mesh[];

const meshFor = (root: THREE.Group, id: string): THREE.Mesh =>
  meshes(root).find((m) => m.userData.regionId === id)!;

describe("RegionPickLayer.setRegions", () => {
  it("builds one fill mesh per region and applies the plane basis", () => {
    const { layer, root } = makeLayer();
    layer.setPlane(planeFor("XZ"));
    layer.setRegions([R0, R1]);

    expect(layer.regionIds).toEqual(["r0", "r1"]);
    const m0 = meshFor(root, "r0");
    expect(m0.geometry.getAttribute("position").count).toBe(4); // (u,v)→(x,y,0)
    expect(m0.geometry.getIndex()!.count).toBe(6);

    // Local x/y map to the plane's world xAxis/yAxis (basis applied).
    const g = layerGroup(root);
    const plane = planeFor("XZ");
    const x = new THREE.Vector3(1, 0, 0).applyMatrix4(g.matrix);
    const y = new THREE.Vector3(0, 1, 0).applyMatrix4(g.matrix);
    expect(x.toArray().map((n) => Math.round(n))).toEqual(plane.xAxis.map((n) => Math.round(n)));
    expect(y.toArray().map((n) => Math.round(n))).toEqual(plane.yAxis.map((n) => Math.round(n)));
  });

  it("skips regions without a triangulation", () => {
    const { layer, root } = makeLayer();
    const empty: SketchRegion = { regionId: "empty", outerLoop: [], holes: [], previewTriangles: undefined };
    layer.setRegions([empty, R0]);
    expect(layer.regionIds).toEqual(["r0"]);
    expect(meshes(root).length).toBe(1);
  });
});

describe("RegionPickLayer tint", () => {
  it("tints hover (cyan hover3d) and selection (orange sketchSelected) per region, tokens only", () => {
    const { layer, root } = makeLayer();
    layer.setPlane(IDENTITY_PLANE);
    layer.setRegions([R0, R1]);

    const mat = (id: string) => meshFor(root, id).material as THREE.MeshBasicMaterial;
    expect(mat("r0").color.getHex()).toBe(palette.referenceNeutral().getHex());
    expect(mat("r0").opacity).toBeCloseTo(0.18, 5);

    layer.setHover("r1");
    expect(mat("r1").color.getHex()).toBe(palette.hover3d().getHex());
    expect(mat("r1").opacity).toBeCloseTo(0.32, 5);
    expect(mat("r1").opacity).toBeGreaterThan(mat("r0").opacity);

    layer.setSelected(["r0"]);
    expect(mat("r0").color.getHex()).toBe(palette.sketchSelected().getHex());
    expect(mat("r0").opacity).toBeCloseTo(0.45, 5);
  });

  it("tints MULTIPLE selected regions (Wave 2 multi-select)", () => {
    const { layer, root } = makeLayer();
    layer.setPlane(IDENTITY_PLANE);
    layer.setRegions([R0, R1]);
    const mat = (id: string) => meshFor(root, id).material as THREE.MeshBasicMaterial;

    layer.setSelected(["r0", "r1"]);
    expect(mat("r0").color.getHex()).toBe(palette.sketchSelected().getHex());
    expect(mat("r1").color.getHex()).toBe(palette.sketchSelected().getHex());

    layer.setSelected([]); // clears back to neutral
    expect(mat("r0").color.getHex()).toBe(palette.referenceNeutral().getHex());
    expect(mat("r0").opacity).toBeCloseTo(0.18, 5);
  });
});

describe("RegionPickLayer lifecycle", () => {
  it("toggles visibility and disposes cleanly", () => {
    const { layer, root } = makeLayer();
    layer.setRegions([R0]);
    layer.setVisible(true);
    expect(layer.visible).toBe(true);

    const geo = meshFor(root, "r0").geometry;
    const geoSpy = vi.spyOn(geo, "dispose");
    layer.dispose();
    expect(geoSpy).toHaveBeenCalledOnce();
    expect(root.children.length).toBe(0);
  });
});
