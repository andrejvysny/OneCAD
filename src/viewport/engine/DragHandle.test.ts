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
import { palette } from "./palette";

/** The two drawn meshes, in the order the handle adds them (halo first). */
function parts(root: THREE.Object3D): { halo: THREE.Mesh; fill: THREE.Mesh } {
  const group = root.children[0] as THREE.Group;
  return { halo: group.children[0] as THREE.Mesh, fill: group.children[1] as THREE.Mesh };
}

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
  it("destructive swaps the arrow, hover wins over it", () => {
    const { handle, root } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    const { fill } = parts(root);
    const base = fill.material;

    handle.setDestructive(true);
    expect(fill.material).not.toBe(base);
    const destructive = fill.material;

    handle.setHover(true);
    expect(fill.material).not.toBe(destructive);

    handle.setHover(false);
    expect(fill.material).toBe(destructive);
  });

  it("reset() drops the extrude-only state so a value tool starts clean", () => {
    const { handle, root, hitAt } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    handle.setDestructive(true);
    const tinted = parts(root).fill.material;

    handle.reset();

    expect(handle.axisMode).toBe("forward");
    expect(parts(root).fill.material).not.toBe(tinted);
    expect(hitAt(-10)).toBe(false);
  });

  /*
   * The halo is the ONLY thing separating the arrow from the face it sits on:
   * the fill reads `--color-accent` and so does a selected face
   * (`palette.selected3d()`), so "blue arrow on blue face" is the default state
   * without it. It also INVERTS between themes, which is exactly the class of
   * material that goes stale when `refreshColors()` forgets one.
   */
  it("draws a halo behind the fill, in neither of the fill's colors", () => {
    const { root } = makeHandle();
    const { fill, halo } = parts(root);

    expect(halo.position.z).toBeLessThan(fill.position.z);
    const haloColor = (halo.material as THREE.MeshBasicMaterial).color;
    expect(haloColor.getHex()).not.toBe(palette.hoverAccent().getHex());
    expect(haloColor.getHex()).not.toBe(palette.selected3d().getHex());
  });

  it("refreshColors re-reads the halo too, not just the three fills", () => {
    const { handle, root } = makeHandle();
    const halo = parts(root).halo.material as THREE.MeshBasicMaterial;
    halo.color.set("rgb(1, 2, 3)");

    handle.refreshColors();

    expect(halo.color.getHex()).toBe(palette.overlayHalo().getHex());
  });

  it("dispose releases the halo's geometry as well", () => {
    const { handle, root } = makeHandle();
    const { fill, halo } = parts(root);
    const disposed: string[] = [];
    fill.geometry.dispose = () => disposed.push("fill");
    halo.geometry.dispose = () => disposed.push("halo");

    handle.dispose();

    expect(disposed).toContain("fill");
    expect(disposed).toContain("halo");
  });
});

/*
 * The arrow is a SCREEN overlay: it faces the camera and runs along its axis's
 * projected direction. The old cone lived in the world, so an axis pointing at
 * the viewer — the commonest camera for a depth drag — drew a dot.
 */
describe("DragHandle billboard", () => {
  const camera = (position: [number, number, number]) => {
    const cam = new THREE.PerspectiveCamera(50, 1, 0.1, 100);
    cam.up.set(0, 0, 1); // the world is Z-up, and so is CameraRig's UP
    cam.position.set(...position);
    cam.lookAt(0, 0, 0);
    cam.updateMatrixWorld(true);
    return cam;
  };

  /** The group's local +Z in world space — the direction the flat arrow faces. */
  const facing = (root: THREE.Object3D) =>
    new THREE.Vector3(0, 0, 1).applyQuaternion((root.children[0] as THREE.Group).quaternion);

  /** The group's local +Y — the direction the arrow's length runs. */
  const along = (root: THREE.Object3D) =>
    new THREE.Vector3(0, 1, 0).applyQuaternion((root.children[0] as THREE.Group).quaternion);

  it("faces the camera from any angle", () => {
    const { handle, root } = makeHandle();
    handle.setAxis(O, Y);
    for (const pos of [
      [0, 0, 60],
      [60, 0, 0],
      [30, 40, 50],
    ] as [number, number, number][]) {
      const cam = camera(pos);
      handle.orient(cam);
      const toCamera = cam.position.clone().normalize();
      expect(facing(root).dot(toCamera)).toBeGreaterThan(0.999);
    }
  });

  it("runs along the axis's screen direction", () => {
    const { handle, root } = makeHandle();
    // World +Z, viewed from +X: straight up the screen.
    handle.setAxis(O, new THREE.Vector3(0, 0, 1));
    const cam = camera([60, 0, 0]);
    handle.orient(cam);

    const screenUp = new THREE.Vector3(0, 1, 0).applyQuaternion(cam.quaternion);
    expect(along(root).dot(screenUp)).toBeGreaterThan(0.999);
  });

  it("holds a full-length arrow when the axis points at the camera", () => {
    const { handle, root } = makeHandle();
    handle.setAxis(O, new THREE.Vector3(0, 0, 1));
    const side = camera([60, 0, 0]);
    handle.orient(side);
    const before = along(root).clone();

    // Now look straight down that axis: it has NO screen direction left.
    const down = camera([0, 0, 60]);
    handle.orient(down);

    // The last good angle is held rather than spinning to an arbitrary one, and
    // the arrow still faces the viewer at full length — not the old dot.
    const heldInCamera = before.clone().applyQuaternion(side.quaternion.clone().invert());
    const nowInCamera = along(root).clone().applyQuaternion(down.quaternion.clone().invert());
    expect(nowInCamera.dot(heldInCamera)).toBeGreaterThan(0.999);
    expect(facing(root).dot(down.position.clone().normalize())).toBeGreaterThan(0.999);
  });
});

describe("DragHandle pick corridor", () => {
  /** Ray parallel to the axis-crossing plane, offset `off` px sideways. */
  const hitOffAxis = (handle: DragHandle, root: THREE.Group, off: number): boolean => {
    root.updateMatrixWorld(true);
    const origin = new THREE.Vector3(off, -50, 0);
    return handle.raycast(new THREE.Raycaster(origin, new THREE.Vector3(0, 1, 0)));
  };

  it("is at least 12 px across — a trackpad must not have to land on a hairline", () => {
    const { handle, root } = makeHandle();
    handle.setAxis(O, Y);
    expect(hitOffAxis(handle, root, 5.9)).toBe(true);
    expect(hitOffAxis(handle, root, 40)).toBe(false);
  });

  it("scales with the handle, so the corridor is constant in CSS pixels", () => {
    const { handle, root } = makeHandle();
    handle.setAxis(O, Y);
    handle.setScale(2); // e.g. a 2× device scale / zoomed-out camera
    expect(hitOffAxis(handle, root, 11.8)).toBe(true);
  });
});
