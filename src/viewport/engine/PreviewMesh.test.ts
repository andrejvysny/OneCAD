import { describe, expect, it, vi } from "vitest";
import * as THREE from "three";
import { PreviewMesh } from "./PreviewMesh";

const plane = {
  kind: "XY" as const,
  origin: [0, 0, 0] as [number, number, number],
  xAxis: [1, 0, 0] as [number, number, number],
  yAxis: [0, 1, 0] as [number, number, number],
  normal: [0, 0, 1] as [number, number, number],
};

describe("PreviewMesh.getBounds", () => {
  it("reports visible L1 geometry and excludes external manipulators", () => {
    const root = new THREE.Group();
    const external = new THREE.Mesh(new THREE.BoxGeometry(2, 2, 2));
    external.position.set(100, 0, 0);
    root.add(external);
    const preview = new PreviewMesh({ root, invalidate: vi.fn() });
    preview.setProfile(plane, {
      ring: [[0, 0], [4, 0], [0, 3]],
      holes: [],
      cap: { positions: [0, 0, 4, 0, 0, 3], indices: [0, 1, 2] },
    });
    preview.setDepth(5, false);

    const bounds = preview.getBounds()!;
    expect(bounds.max.toArray()).toEqual([4, 3, 5]);
    expect(bounds.max.x).toBeLessThan(100);
    preview.setVisible(false);
    expect(preview.getBounds()).toBeNull();
    preview.dispose();
    external.geometry.dispose();
  });
});
