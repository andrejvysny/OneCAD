/*
 * SectionLayer — the plane math, the stencil construction, and the two rules
 * that make the cap correct rather than merely present:
 *   - the cap is hidden while a sketch session is open (dimmed bodies render in
 *     the transparent pass and would paint over it — renderOrder.ts rule 4),
 *   - stencil pairs mirror the VISIBLE body face meshes, so a hidden/isolated
 *     body is never capped.
 *
 * jsdom has no GL, so nothing here renders; what is checked is the scene graph
 * and the material state the renderer would consume.
 */
import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { SectionLayer, sectionPlane, sectionOffsetRange } from "./SectionLayer";
import { RENDER_ORDER } from "./renderOrder";
import { palette, resetPaletteCache } from "./palette";
import type { SectionState } from "@/stores/viewportStore";

const state = (over: Partial<SectionState> = {}): SectionState => ({
  enabled: true,
  plane: "XY",
  offsetMm: 0,
  flip: false,
  ...over,
});

/** A body-shaped group: one face mesh (what the layer mirrors) plus an edge. */
function bodyGroup(id: string): THREE.Group {
  const group = new THREE.Group();
  group.userData.bodyId = id;
  const face = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshStandardMaterial());
  face.userData = { bodyId: id, kind: "face" };
  const edge = new THREE.Object3D();
  edge.userData = { bodyId: id, kind: "edge" };
  group.add(face, edge);
  return group;
}

function harness(bodies: string[] = ["body1"]) {
  const scene = new THREE.Group();
  const bodiesRoot = new THREE.Group();
  for (const id of bodies) bodiesRoot.add(bodyGroup(id));
  const invalidate = vi.fn();
  const layer = new SectionLayer({
    root: scene,
    getBodiesRoot: () => bodiesRoot,
    getBounds: () => new THREE.Box3(new THREE.Vector3(-40, -30, -15), new THREE.Vector3(40, 30, 15)),
    invalidate,
  });
  return { scene, bodiesRoot, layer, invalidate };
}

/** Every stencil mesh currently attached to the layer. */
function stencilMeshes(layer: SectionLayer): THREE.Mesh[] {
  return layer.object3D.children.filter(
    (c) => c.name !== "sectionCap" && (c as THREE.Mesh).isMesh,
  ) as THREE.Mesh[];
}

function capMesh(layer: SectionLayer): THREE.Mesh {
  return layer.object3D.children.find((c) => c.name === "sectionCap") as THREE.Mesh;
}

describe("sectionPlane", () => {
  // three DISCARDS fragments at a NEGATIVE signed distance, so "kept" below
  // means distanceToPoint >= 0.
  const kept = (p: THREE.Plane, x: number, y: number, z: number) =>
    p.distanceToPoint(new THREE.Vector3(x, y, z)) >= 0;
  // `Vector3.equals`, not `toArray()`: negating a zero component yields -0, and
  // a signed zero is a deep-equal mismatch while being the same direction.
  const normalIs = (p: THREE.Plane, x: number, y: number, z: number) =>
    p.normal.equals(new THREE.Vector3(x, y, z));

  it("XY unflipped keeps the half BELOW the offset along +Z", () => {
    const p = sectionPlane("XY", 0, false);
    expect(normalIs(p, 0, 0, -1)).toBe(true);
    expect(p.constant).toBe(0);
    expect(kept(p, 0, 0, -5)).toBe(true);
    expect(kept(p, 0, 0, 5)).toBe(false);
  });

  it("flip keeps the other half, and nothing else moves", () => {
    const p = sectionPlane("XY", 0, true);
    expect(normalIs(p, 0, 0, 1)).toBe(true);
    expect(kept(p, 0, 0, 5)).toBe(true);
    expect(kept(p, 0, 0, -5)).toBe(false);
  });

  it("the offset slides the cut along the plane's own axis", () => {
    const p = sectionPlane("XY", 10, false);
    expect(kept(p, 0, 0, 9)).toBe(true);
    expect(kept(p, 0, 0, 11)).toBe(false);

    const flipped = sectionPlane("XY", 10, true);
    expect(kept(flipped, 0, 0, 11)).toBe(true);
    expect(kept(flipped, 0, 0, 9)).toBe(false);
  });

  it("XZ cuts along +Y and YZ along +X", () => {
    const xz = sectionPlane("XZ", 4, false);
    expect(normalIs(xz, 0, -1, 0)).toBe(true);
    expect(kept(xz, 100, 3, 100)).toBe(true);
    expect(kept(xz, 100, 5, 100)).toBe(false);

    const yz = sectionPlane("YZ", -4, false);
    expect(normalIs(yz, -1, 0, 0)).toBe(true);
    expect(kept(yz, -5, 100, 100)).toBe(true);
    expect(kept(yz, -3, 100, 100)).toBe(false);
  });

  it("writes into the SAME instance when one is passed (the live clip plane)", () => {
    const target = new THREE.Plane();
    expect(sectionPlane("YZ", 2, false, target)).toBe(target);
  });
});

describe("sectionOffsetRange", () => {
  const box = new THREE.Box3(new THREE.Vector3(-40, -30, -15), new THREE.Vector3(40, 30, 15));

  it("spans the scene along the plane's axis", () => {
    expect(sectionOffsetRange(box, "XY")).toEqual({ min: -15, max: 15 });
    expect(sectionOffsetRange(box, "XZ")).toEqual({ min: -30, max: 30 });
    expect(sectionOffsetRange(box, "YZ")).toEqual({ min: -40, max: 40 });
  });

  it("is null with nothing to cut — the control has no meaningful range", () => {
    expect(sectionOffsetRange(null, "XY")).toBeNull();
    expect(sectionOffsetRange(new THREE.Box3(), "XY")).toBeNull();
  });
});

describe("SectionLayer state", () => {
  it("attaches hidden, and setState publishes the live plane + one repaint", () => {
    const { scene, layer, invalidate } = harness();
    expect(scene.children).toContain(layer.object3D);
    expect(layer.object3D.visible).toBe(false);
    expect(layer.clippingPlanes()).toBeNull();

    layer.setState(state({ offsetMm: 7 }));

    expect(invalidate).toHaveBeenCalledTimes(1); // one state change, one frame
    expect(layer.object3D.visible).toBe(true);
    const planes = layer.clippingPlanes();
    expect(planes).toHaveLength(1);
    expect(planes![0].normal.equals(new THREE.Vector3(0, 0, -1))).toBe(true);
    expect(planes![0].constant).toBe(7);
    layer.dispose();
  });

  it("mutates the SAME plane instance across updates — a drag writes no materials", () => {
    const { layer } = harness();
    layer.setState(state({ offsetMm: 1 }));
    const first = layer.clippingPlanes()![0];
    layer.setState(state({ offsetMm: 2 }));
    expect(layer.clippingPlanes()![0]).toBe(first);
    expect(first.constant).toBe(2);
    layer.dispose();
  });

  it("disabling drops the planes and every stencil mesh", () => {
    const { layer } = harness();
    layer.setState(state());
    layer.update();
    expect(layer.stencilCount).toBe(1);

    layer.setState(state({ enabled: false }));
    expect(layer.clippingPlanes()).toBeNull();
    expect(layer.stencilCount).toBe(0);
    expect(stencilMeshes(layer)).toHaveLength(0);
    layer.dispose();
  });
});

describe("SectionLayer stencil pairs", () => {
  it("mirrors each visible body face mesh as an increment/decrement pair", () => {
    const { bodiesRoot, layer } = harness(["body1"]);
    layer.setState(state());
    layer.update();

    const [back, front] = stencilMeshes(layer);
    const sourceFace = bodiesRoot.children[0].children[0] as THREE.Mesh;

    // Shared geometry — no copies, no GPU buffers of the layer's own.
    expect(back.geometry).toBe(sourceFace.geometry);
    expect(front.geometry).toBe(sourceFace.geometry);

    const backMat = back.material as THREE.MeshBasicMaterial;
    const frontMat = front.material as THREE.MeshBasicMaterial;
    expect(backMat.side).toBe(THREE.BackSide);
    expect(backMat.stencilZPass).toBe(THREE.IncrementWrapStencilOp);
    expect(frontMat.side).toBe(THREE.FrontSide);
    expect(frontMat.stencilZPass).toBe(THREE.DecrementWrapStencilOp);

    // Stencil only: no color, no depth, and clipped by the section's own plane.
    for (const mat of [backMat, frontMat]) {
      expect(mat.colorWrite).toBe(false);
      expect(mat.depthWrite).toBe(false);
      expect(mat.stencilWrite).toBe(true);
      expect(mat.clippingPlanes).toBe(layer.clippingPlanes());
    }

    // Drawn BEFORE the cap, which is drawn before the bodies (default 0).
    expect(back.renderOrder).toBe(RENDER_ORDER.SECTION_STENCIL_BACK);
    expect(front.renderOrder).toBe(RENDER_ORDER.SECTION_STENCIL_FRONT);
    expect(capMesh(layer).renderOrder).toBe(RENDER_ORDER.SECTION_CAP);
    expect(RENDER_ORDER.SECTION_CAP).toBeLessThan(0);
    layer.dispose();
  });

  it("follows bodies in and out of the scene, and never re-creates a live pair", () => {
    const { bodiesRoot, layer } = harness(["body1", "body2"]);
    layer.setState(state());
    layer.update();
    expect(layer.stencilCount).toBe(2);
    const before = stencilMeshes(layer)[0];

    layer.update(); // idle re-sync allocates nothing
    expect(stencilMeshes(layer)[0]).toBe(before);

    bodiesRoot.remove(bodiesRoot.children[1]);
    layer.update();
    expect(layer.stencilCount).toBe(1);

    bodiesRoot.add(bodyGroup("body3"));
    layer.update();
    expect(layer.stencilCount).toBe(2);
    layer.dispose();
  });

  it("skips a hidden body — a cut surface for an off-screen body is a ghost", () => {
    const { bodiesRoot, layer } = harness(["body1", "body2"]);
    layer.setState(state());
    bodiesRoot.children[1].visible = false; // tree eye / isolation mask
    layer.update();
    expect(layer.stencilCount).toBe(1);

    bodiesRoot.visible = false; // Layers → Bodies off
    layer.update();
    expect(layer.stencilCount).toBe(0);
    expect(layer.capVisible).toBe(false);
    layer.dispose();
  });

  it("carries the source mesh's world matrix", () => {
    const { bodiesRoot, layer } = harness(["body1"]);
    bodiesRoot.children[0].position.set(5, 6, 7);
    layer.setState(state());
    layer.update();

    const back = stencilMeshes(layer)[0];
    const face = bodiesRoot.children[0].children[0] as THREE.Mesh;
    expect(back.matrixAutoUpdate).toBe(false);
    expect(back.matrix.elements).toEqual(face.matrixWorld.elements);
    layer.dispose();
  });
});

describe("SectionLayer cap", () => {
  it("is hidden while a sketch session is active, and the CLIP survives", () => {
    const { layer } = harness();
    layer.setState(state());
    layer.update();
    expect(layer.capVisible).toBe(true);
    expect(capMesh(layer).visible).toBe(true);

    layer.setSketchActive(true);
    expect(layer.capVisible).toBe(false);
    expect(capMesh(layer).visible).toBe(false);
    for (const m of stencilMeshes(layer)) expect(m.visible).toBe(false);
    // The cut itself is untouched — only the cap could be mis-ordered.
    expect(layer.clippingPlanes()).toHaveLength(1);

    layer.setSketchActive(false);
    expect(layer.capVisible).toBe(true);
    expect(capMesh(layer).visible).toBe(true);
    layer.dispose();
  });

  it("does not re-sync while a sketch session is open (the cap is not being drawn)", () => {
    const { bodiesRoot, layer } = harness();
    layer.setState(state());
    layer.setSketchActive(true);
    layer.update();
    expect(layer.stencilCount).toBe(0);

    bodiesRoot.visible = true;
    layer.setSketchActive(false);
    layer.update();
    expect(layer.stencilCount).toBe(1);
    layer.dispose();
  });

  it("is not capped when nothing is on screen to cut", () => {
    const { layer } = harness([]);
    layer.setState(state());
    layer.update();
    expect(layer.capVisible).toBe(false);
    layer.dispose();
  });

  it("sits ON the plane, oriented to its normal and oversized past the scene", () => {
    const { layer } = harness();
    layer.setState(state({ plane: "XZ", offsetMm: 12 }));
    layer.update();

    const cap = capMesh(layer);
    const plane = layer.clippingPlanes()![0];
    expect(Math.abs(plane.distanceToPoint(cap.position))).toBeLessThan(1e-6);
    // PlaneGeometry's own +Z rotated onto the section normal.
    const facing = new THREE.Vector3(0, 0, 1).applyQuaternion(cap.quaternion);
    expect(facing.distanceTo(plane.normal)).toBeLessThan(1e-6);
    // Bigger than the 80×60×30 scene it has to cover.
    expect(cap.scale.x).toBeGreaterThan(100);
    layer.dispose();
  });

  it("resets the stencil buffer after it draws, so no later pass misreads it", () => {
    const { layer } = harness();
    const capMat = capMesh(layer).material as THREE.MeshStandardMaterial;
    expect(capMat.stencilFunc).toBe(THREE.NotEqualStencilFunc);
    expect(capMat.stencilRef).toBe(0);
    expect(capMat.stencilZPass).toBe(THREE.ReplaceStencilOp);

    const clearStencil = vi.fn();
    capMesh(layer).onAfterRender(
      { clearStencil } as unknown as THREE.WebGLRenderer,
      null as never,
      null as never,
      null as never,
      null as never,
      null as never,
    );
    expect(clearStencil).toHaveBeenCalledTimes(1);
    layer.dispose();
  });

  it("refreshColors re-reads the cut token after a theme flip", () => {
    const { layer } = harness();
    const capMat = capMesh(layer).material as THREE.MeshStandardMaterial;
    const lightHex = capMat.color.getHex();

    document.documentElement.dataset.theme = "dark";
    resetPaletteCache();
    layer.refreshColors();

    expect(capMat.color.getHex()).toBe(palette.sectionCap().getHex());
    expect(capMat.color.getHex()).not.toBe(lightHex);
    document.documentElement.dataset.theme = "light";
    resetPaletteCache();
    layer.dispose();
  });

  it("dispose detaches the group and leaves the shared body geometry alive", () => {
    const { scene, bodiesRoot, layer } = harness();
    layer.setState(state());
    layer.update();
    const geometry = (bodiesRoot.children[0].children[0] as THREE.Mesh).geometry;
    const disposeSpy = vi.spyOn(geometry, "dispose");

    layer.dispose();

    expect(scene.children).not.toContain(layer.object3D);
    expect(disposeSpy).not.toHaveBeenCalled();
  });
});
