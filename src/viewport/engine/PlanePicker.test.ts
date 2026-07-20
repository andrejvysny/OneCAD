import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { PlanePicker, type PickablePlane } from "./PlanePicker";
import { HtmlOverlayDriver } from "./HtmlOverlayDriver";
import { planeFor } from "@/ipc/mockSketch";

const KINDS: PickablePlane[] = ["XY", "XZ", "YZ"];

function makePicker(): { picker: PlanePicker; root: THREE.Object3D; overlay: HtmlOverlayDriver; overlayEl: HTMLElement } {
  const root = new THREE.Object3D();
  const overlay = new HtmlOverlayDriver();
  const overlayEl = document.createElement("div");
  const picker = new PlanePicker({ root, overlay, overlayEl, invalidate: vi.fn() });
  return { picker, root, overlay, overlayEl };
}

function quadGroup(root: THREE.Object3D, kind: PickablePlane): THREE.Object3D {
  const pickerGroup = root.children[0];
  const found = pickerGroup.children.find((c) => c.name === `planePick_${kind}`);
  if (!found) throw new Error(`no quad group for ${kind}`);
  return found;
}

function quadMesh(root: THREE.Object3D, kind: PickablePlane): THREE.Mesh {
  return quadGroup(root, kind).children[0] as THREE.Mesh;
}

describe("PlanePicker", () => {
  it("each quad's group matrix maps local (1,0,0)/(0,1,0) to the plane's xAxis/yAxis", () => {
    const { root } = makePicker();
    for (const kind of KINDS) {
      const plane = planeFor(kind);
      const group = quadGroup(root, kind);
      const x = new THREE.Vector3(1, 0, 0).applyMatrix4(group.matrix);
      const y = new THREE.Vector3(0, 1, 0).applyMatrix4(group.matrix);
      expect(x.x).toBeCloseTo(plane.xAxis[0], 9);
      expect(x.y).toBeCloseTo(plane.xAxis[1], 9);
      expect(x.z).toBeCloseTo(plane.xAxis[2], 9);
      expect(y.x).toBeCloseTo(plane.yAxis[0], 9);
      expect(y.y).toBeCloseTo(plane.yAxis[1], 9);
      expect(y.z).toBeCloseTo(plane.yAxis[2], 9);
    }
  });

  it("hitTest resolves the front-most quad immediately after construction: +Z ray hits XY, +X ray hits XZ (repo kind, normal +X)", () => {
    const { picker } = makePicker();
    picker.setVisible(true);
    // No manual root.updateMatrixWorld here (and no render loop): hitTest must flush
    // the quad world matrices itself, so a raycast right after construction resolves.

    const fromZ = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
    const hitZ = picker.hitTest(fromZ);
    expect(hitZ?.kind).toBe("XY");
    expect(hitZ?.point.toArray().map((n) => Math.round(n))).toEqual([0, 0, 0]);

    const fromX = new THREE.Raycaster(new THREE.Vector3(50, 0, 0), new THREE.Vector3(-1, 0, 0));
    const hitX = picker.hitTest(fromX);
    expect(hitX?.kind).toBe("XZ");
  });

  it("hitTest returns null while not visible", () => {
    const { picker } = makePicker();
    const fromZ = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
    expect(picker.hitTest(fromZ)).toBeNull();
  });

  it("setHover bumps only the hovered quad's opacity and shows the GEOMETRIC label (repo kind XZ -> label YZ)", () => {
    const { picker, root, overlay, overlayEl } = makePicker();
    picker.setVisible(true);

    picker.setHover({ kind: "XZ", point: new THREE.Vector3(1, 2, 3) });

    const matXY = (quadMesh(root, "XY").material as THREE.MeshBasicMaterial).opacity;
    const matXZ = (quadMesh(root, "XZ").material as THREE.MeshBasicMaterial).opacity;
    const matYZ = (quadMesh(root, "YZ").material as THREE.MeshBasicMaterial).opacity;
    expect(matXZ).toBeCloseTo(0.35, 5);
    expect(matXY).toBeCloseTo(0.16, 5);
    expect(matYZ).toBeCloseTo(0.16, 5);

    // "XZ" (repo kind, normal +X) is geometrically the YZ plane — bases are
    // legacy-swapped (mockSketch.ts) — so the chip must show the geometric name.
    expect(overlay.size).toBe(1);
    const label = overlayEl.querySelector<HTMLElement>("[data-plane-pick-label]");
    expect(label?.textContent).toBe("YZ");
    expect(label?.style.display).toBe("");
  });

  it("setHover(null) clears all quads back to base opacity and unregisters the chip", () => {
    const { picker, root, overlay, overlayEl } = makePicker();
    picker.setVisible(true);
    picker.setHover({ kind: "YZ", point: new THREE.Vector3(0, 0, 0) });
    expect(overlay.size).toBe(1);

    picker.setHover(null);

    for (const kind of KINDS) {
      const mat = quadMesh(root, kind).material as THREE.MeshBasicMaterial;
      expect(mat.opacity).toBeCloseTo(0.16, 5);
    }
    expect(overlay.size).toBe(0);
    const label = overlayEl.querySelector<HTMLElement>("[data-plane-pick-label]");
    expect(label?.style.display).toBe("none");
  });

  it("setVisible(false) clears hover as a side effect", () => {
    const { picker, root, overlay } = makePicker();
    picker.setVisible(true);
    picker.setHover({ kind: "XY", point: new THREE.Vector3(0, 0, 0) });
    expect(overlay.size).toBe(1);

    picker.setVisible(false);

    expect(picker.visible).toBe(false);
    expect(overlay.size).toBe(0);
    for (const kind of KINDS) {
      const mat = quadMesh(root, kind).material as THREE.MeshBasicMaterial;
      expect(mat.opacity).toBeCloseTo(0.16, 5);
    }
  });

  it("dispose releases geometries/materials and empties the root", () => {
    const { picker, root, overlayEl } = makePicker();
    picker.setVisible(true);

    const mesh = quadMesh(root, "XY");
    const geoSpy = vi.spyOn(mesh.geometry, "dispose");
    const matSpy = vi.spyOn(mesh.material as THREE.Material, "dispose");

    expect(root.children.length).toBe(1);
    picker.dispose();

    expect(geoSpy).toHaveBeenCalledOnce();
    expect(matSpy).toHaveBeenCalledOnce();
    expect(root.children.length).toBe(0);
    expect(overlayEl.querySelector("[data-plane-pick-label]")).toBeNull();
  });
});
