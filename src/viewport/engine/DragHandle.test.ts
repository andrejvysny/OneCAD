/*
 * DragHandle — the value arrow's two invariants that are easy to break silently.
 *
 * ORIENTATION at a zero direction. `Vector3.normalize()` maps (0,0,0) to (0,0,0)
 * and `setFromUnitVectors` then takes its degenerate branch, producing an
 * arbitrary 180° flip rather than a NaN. The extrude arm hits exactly that at
 * depth 0, so the handle must HOLD its orientation instead.
 *
 * PICK ENVELOPE tracks the visible heads. `twoWay` draws a mirrored head and must
 * be grabbable from that side; `forward` must NOT be, or the envelope reaches
 * backwards through the prism and the body (the visible materials are
 * `depthTest: false` and `raycast` has no occlusion test) and swallows selection.
 */
import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { DragHandle } from "./DragHandle";

function makeHandle() {
  const root = new THREE.Group();
  const invalidate = vi.fn();
  const handle = new DragHandle({ root, invalidate });
  handle.setVisible(true);
  handle.setScale(1);
  /**
   * Raycast a horizontal ray at height `y`, aimed at the axis. Three's raycaster
   * reads `matrixWorld` and never refreshes it, so the update is the caller's job
   * — in the app the render loop has already done it before any pick.
   */
  const hitAt = (y: number): boolean => {
    root.updateMatrixWorld(true);
    const origin = new THREE.Vector3(50, y, 0);
    const dir = new THREE.Vector3(0, y, 0).sub(origin).normalize();
    return handle.raycast(new THREE.Raycaster(origin, dir));
  };
  return { handle, root, invalidate, hitAt };
}

const Y = new THREE.Vector3(0, 1, 0);
const ZERO = new THREE.Vector3(0, 0, 0);
const O = new THREE.Vector3(0, 0, 0);

describe("DragHandle orientation", () => {
  it("holds its orientation when handed a zero-length direction", () => {
    const { handle, root } = makeHandle();
    handle.setAxis(O, new THREE.Vector3(0, 0, 1));
    const group = root.children[0];
    const before = group.quaternion.clone();

    handle.setAxis(O, ZERO);

    expect(group.quaternion.x).toBeCloseTo(before.x, 12);
    expect(group.quaternion.y).toBeCloseTo(before.y, 12);
    expect(group.quaternion.z).toBeCloseTo(before.z, 12);
    expect(group.quaternion.w).toBeCloseTo(before.w, 12);
    expect(Number.isNaN(group.quaternion.w)).toBe(false);
  });

  it("flips a full 180° for a reversed direction", () => {
    const { handle, root } = makeHandle();
    const group = root.children[0];
    handle.setAxis(O, Y);
    const up = new THREE.Vector3(0, 1, 0).applyQuaternion(group.quaternion);
    handle.setAxis(O, new THREE.Vector3(0, -1, 0));
    const down = new THREE.Vector3(0, 1, 0).applyQuaternion(group.quaternion);

    expect(up.y).toBeCloseTo(1, 6);
    expect(down.y).toBeCloseTo(-1, 6);
  });
});

describe("DragHandle pick envelope", () => {
  it("twoWay is grabbable from the NEGATIVE side", () => {
    const { handle, hitAt } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    expect(handle.axisMode).toBe("twoWay");
    // Across the arrow well below the origin — inside the mirrored head.
    expect(hitAt(-10)).toBe(true);
  });

  it("forward is NOT grabbable behind its origin", () => {
    const { handle, hitAt } = makeHandle();
    handle.setAxis(O, Y, "forward");
    expect(hitAt(-10)).toBe(false);
    // …but the forward body still picks.
    expect(hitAt(20)).toBe(true);
  });

  it("switching back to forward shrinks the envelope again", () => {
    const { handle, hitAt } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    expect(hitAt(-10)).toBe(true);
    handle.setAxis(O, Y, "forward");
    expect(hitAt(-10)).toBe(false);
  });

  it("a hidden handle is never picked", () => {
    const { handle, hitAt } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    handle.setVisible(false);
    expect(hitAt(20)).toBe(false);
  });
});

describe("DragHandle materials", () => {
  const meshes = (root: THREE.Object3D): THREE.Mesh[] =>
    (root.children[0] as THREE.Group).children.filter(
      (c): c is THREE.Mesh => (c as THREE.Mesh).isMesh && c !== undefined,
    );

  it("destructive swaps every visible head, hover wins over it", () => {
    const { handle, root } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    const [shaft, cone, tail] = meshes(root);
    const base = shaft.material;

    handle.setDestructive(true);
    expect(shaft.material).not.toBe(base);
    expect(cone.material).toBe(shaft.material);
    expect(tail.material).toBe(shaft.material);
    const destructive = shaft.material;

    handle.setHover(true);
    expect(shaft.material).not.toBe(destructive);
    expect(tail.material).toBe(shaft.material);

    handle.setHover(false);
    expect(shaft.material).toBe(destructive);
  });

  it("reset() drops the extrude-only state so a value tool starts clean", () => {
    const { handle, root, hitAt } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    handle.setDestructive(true);
    const [shaft] = meshes(root);
    const tinted = shaft.material;

    handle.reset();

    expect(handle.axisMode).toBe("forward");
    expect(shaft.material).not.toBe(tinted);
    expect(hitAt(-10)).toBe(false);
  });
});
