import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { DatumLayer, datumGhostPlane, type DatumVisual } from "./DatumLayer";
import { HtmlOverlayDriver } from "./HtmlOverlayDriver";
import { planeFor } from "@/ipc/mockSketch";

function makeLayer(): {
  layer: DatumLayer;
  root: THREE.Object3D;
  overlay: HtmlOverlayDriver;
  overlayEl: HTMLElement;
  invalidate: ReturnType<typeof vi.fn>;
} {
  const root = new THREE.Object3D();
  const overlay = new HtmlOverlayDriver();
  const overlayEl = document.createElement("div");
  const invalidate = vi.fn();
  const layer = new DatumLayer({ root, overlay, overlayEl, invalidate });
  return { layer, root, overlay, overlayEl, invalidate };
}

/** A datum on the XY base offset +10 along its normal (world +Z). */
function datum(id: string, name: string, offset = 10): DatumVisual {
  return { id, name, plane: datumGhostPlane("XY", offset), resolvedValid: true };
}

function group(root: THREE.Object3D, id: string): THREE.Object3D {
  const layerRoot = root.children[0];
  const found = layerRoot.children.find((c) => c.name === `datum_${id}`);
  if (!found) throw new Error(`no group for datum ${id}`);
  return found;
}

function quadMesh(root: THREE.Object3D, id: string): THREE.Mesh {
  return group(root, id).children[0] as THREE.Mesh;
}

const camAt = (d: number): THREE.PerspectiveCamera => {
  const cam = new THREE.PerspectiveCamera(76, 4 / 3, 0.1, 10_000);
  cam.position.set(0, 0, d);
  cam.lookAt(0, 0, 0);
  cam.updateMatrixWorld(true);
  return cam;
};

describe("datumGhostPlane", () => {
  it("slides the base origin along the base NORMAL and carries the axes verbatim", () => {
    const base = planeFor("XY");
    const p = datumGhostPlane("XY", 10);
    expect(p.kind).toBe("custom");
    expect(p.origin).toEqual([0, 0, 10]);
    expect(p.xAxis).toEqual(base.xAxis);
    expect(p.yAxis).toEqual(base.yAxis);
    expect(p.normal).toEqual(base.normal);
  });

  it("honours the LEGACY-SWAPPED bases: 'XZ' offsets along world +X, 'YZ' along +Y", () => {
    // Sketch.h's XZ() has normal +X and YZ() has +Y (mockSketch.PLANES). A datum
    // must follow that, not the standard convention — the core resolves it the
    // same way, so any "correction" here would put the ghost where the commit isn't.
    expect(datumGhostPlane("XZ", 10).origin).toEqual([10, 0, 0]);
    expect(datumGhostPlane("YZ", 10).origin).toEqual([0, 10, 0]);
  });

  it("a negative offset slides the other way", () => {
    expect(datumGhostPlane("XY", -4).origin).toEqual([0, 0, -4]);
  });
});

describe("DatumLayer", () => {
  it("builds one group per datum, carrying the datum's basis, plus an overlay name label", () => {
    const { layer, root, overlay, overlayEl } = makeLayer();
    layer.syncDatums([datum("d1", "Datum 1"), datum("d2", "Datum 2", -5)]);

    expect(layer.datumIds.sort()).toEqual(["d1", "d2"]);
    const g = group(root, "d1");
    const origin = new THREE.Vector3().setFromMatrixPosition(g.matrix);
    expect(origin.toArray()).toEqual([0, 0, 10]);

    // One overlay item per datum; the label carries the tree name.
    expect(overlay.size).toBe(2);
    const label = overlayEl.querySelector<HTMLElement>('[data-datum-label="d1"]');
    expect(label?.textContent).toBe("Datum 1");
  });

  it("re-syncing an unchanged datum keeps the SAME group (no rebuild churn)", () => {
    const { layer, root } = makeLayer();
    layer.syncDatums([datum("d1", "Datum 1")]);
    const before = group(root, "d1");
    layer.syncDatums([datum("d1", "Datum 1")]);
    expect(group(root, "d1")).toBe(before);
  });

  it("a moved frame rebuilds the group at the new origin; a removed datum is dropped", () => {
    const { layer, root, overlay, overlayEl } = makeLayer();
    layer.syncDatums([datum("d1", "Datum 1", 10), datum("d2", "Datum 2")]);
    const before = group(root, "d1");

    layer.syncDatums([datum("d1", "Datum 1", 25)]);

    const after = group(root, "d1");
    expect(after).not.toBe(before);
    expect(new THREE.Vector3().setFromMatrixPosition(after.matrix).toArray()).toEqual([0, 0, 25]);
    expect(layer.datumIds).toEqual(["d1"]);
    // The removed datum takes its overlay label with it (no leak).
    expect(overlay.size).toBe(1);
    expect(overlayEl.querySelector('[data-datum-label="d2"]')).toBeNull();
  });

  it("a renamed datum re-renders its label", () => {
    const { layer, overlayEl } = makeLayer();
    layer.syncDatums([datum("d1", "Datum 1")]);
    layer.syncDatums([{ ...datum("d1", "Base plane") }]);
    expect(overlayEl.querySelector('[data-datum-label="d1"]')?.textContent).toBe("Base plane");
  });

  it("hitTest resolves the datum under the ray straight after syncDatums (no render frame)", () => {
    const { layer } = makeLayer();
    layer.syncDatums([datum("d1", "Datum 1", 10)]);
    // syncDatums flushes the world matrices itself; hitTest flushes again — a
    // raycast issued outside the render loop must still resolve.
    const ray = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
    expect(layer.hitTest(ray)).toBe("d1");
  });

  it("hitTest returns null with no datums, and null off the quad", () => {
    const { layer } = makeLayer();
    const ray = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
    expect(layer.hitTest(ray)).toBeNull();

    layer.syncDatums([datum("d1", "Datum 1")]);
    const miss = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(1, 0, 0));
    expect(layer.hitTest(miss)).toBeNull();
  });

  it("holds a constant on-screen quad size across zoom, and the hit target with it", () => {
    const { layer, root } = makeLayer();
    // Anchor at the world origin so camera depth == orbit distance in this rig.
    layer.syncDatums([datum("d1", "Datum 1", 0)]);
    const g = group(root, "d1");

    layer.update(camAt(100), 800);
    const near = new THREE.Vector3().setFromMatrixScale(g.matrix).x;
    layer.update(camAt(400), 800);
    const far = new THREE.Vector3().setFromMatrixScale(g.matrix).x;
    expect(far / near).toBeCloseTo(4, 6);

    // Re-scaling is IDEMPOTENT: a second update at the same camera must not
    // compound the scale (the un-scaled basis is kept per entry for this).
    layer.update(camAt(400), 800);
    expect(new THREE.Vector3().setFromMatrixScale(g.matrix).x).toBeCloseTo(far, 9);

    // The quad IS the raycast geometry, so the hit target follows the scale.
    const justOutside = 120 * near * 0.5 * 1.5;
    const ray = new THREE.Raycaster(new THREE.Vector3(0, justOutside, 50), new THREE.Vector3(0, 0, -1));
    expect(layer.hitTest(ray)).toBe("d1"); // still scaled up (far)
    layer.update(camAt(100), 800);
    expect(layer.hitTest(ray)).toBeNull(); // shrunk back
  });

  it("hover + selection tint only the addressed datum; an unresolved datum stays dimmed", () => {
    const { layer, root } = makeLayer();
    layer.syncDatums([
      datum("d1", "Datum 1"),
      datum("d2", "Datum 2"),
      { ...datum("d3", "Datum 3"), resolvedValid: false },
    ]);
    const op = (id: string) => (quadMesh(root, id).material as THREE.MeshBasicMaterial).opacity;
    const base = op("d1");

    layer.setHover("d2");
    expect(op("d2")).toBeGreaterThan(base);
    expect(op("d1")).toBeCloseTo(base, 6);

    layer.setSelected(["d1"]);
    expect(op("d1")).toBeGreaterThan(base);

    // Unresolved: never lit, whatever the hover/selection says.
    layer.setHover("d3");
    layer.setSelected(["d3"]);
    expect(op("d3")).toBeLessThan(base);

    layer.setHover(null);
    layer.setSelected([]);
    expect(op("d1")).toBeCloseTo(base, 6);
    expect(op("d2")).toBeCloseTo(base, 6);
  });

  it("the ghost shows/moves/hides and is NOT hit-testable (it is not a datum yet)", () => {
    const { layer, overlayEl } = makeLayer();
    expect(layer.ghostVisible).toBe(false);

    layer.setGhost(datumGhostPlane("XY", 10), "XY");
    expect(layer.ghostVisible).toBe(true);
    expect(overlayEl.querySelector<HTMLElement>("[data-datum-ghost-label]")?.textContent).toBe("XY");

    // A ray straight through the ghost resolves NOTHING — hitTest only sees
    // committed datums, so the offset phase can never select its own ghost.
    const ray = new THREE.Raycaster(new THREE.Vector3(0, 0, 50), new THREE.Vector3(0, 0, -1));
    expect(layer.hitTest(ray)).toBeNull();

    layer.setGhost(datumGhostPlane("XY", 40), "XY");
    layer.update(camAt(300), 800);
    expect(layer.ghostVisible).toBe(true);

    layer.setGhost(null);
    expect(layer.ghostVisible).toBe(false);
  });

  it("dispose releases every geometry/material, the overlay items and the labels", () => {
    const { layer, root, overlay, overlayEl } = makeLayer();
    layer.syncDatums([datum("d1", "Datum 1")]);
    layer.setGhost(datumGhostPlane("XY", 10), "XY");

    const mesh = quadMesh(root, "d1");
    const geoSpy = vi.spyOn(mesh.geometry, "dispose");
    const matSpy = vi.spyOn(mesh.material as THREE.Material, "dispose");
    expect(root.children.length).toBe(1);

    layer.dispose();

    expect(geoSpy).toHaveBeenCalledOnce();
    expect(matSpy).toHaveBeenCalledOnce();
    expect(root.children.length).toBe(0);
    expect(overlay.size).toBe(0);
    expect(overlayEl.querySelector("[data-datum-label]")).toBeNull();
    expect(overlayEl.querySelector("[data-datum-ghost-label]")).toBeNull();
  });
});
