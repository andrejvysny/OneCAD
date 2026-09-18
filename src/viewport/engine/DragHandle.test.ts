/*
 * DragHandle — the value arrow's two invariants that are easy to break silently.
 *
 * ORIENTATION at a zero direction. `Vector3.normalize()` maps (0,0,0) to (0,0,0)
 * and `setFromUnitVectors` then takes its degenerate branch, producing an
 * arbitrary 180° flip rather than a NaN. The extrude arm hits exactly that at
 * depth 0, so the handle must HOLD its orientation instead.
 *
 * PICK CORRIDOR is a flat 30 × 40 px rectangle in the glyph's plane, centred on
 * the attachment. Both variants share it; chrome exclusion at input boundary,
 * rather than asymmetric hidden geometry, protects nearby controls. Being flat,
 * it is only hit by rays crossing the glyph plane, as a billboarded view ray does.
 */
import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { DragHandle } from "./DragHandle";
import type { LinearHandlePath } from "@/tools/preview/handleProjection";
import { palette } from "./palette";
import { worldPerPixel } from "./screenScale";

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
   * Raycast straight through the glyph plane (local −Z, the billboard's view
   * direction) at height `y` on the axis. Three's raycaster reads `matrixWorld`
   * and never refreshes it, so the update is the caller's job — in the app the
   * render loop has already done it before any pick.
   */
  const hitAt = (y: number): boolean => {
    root.updateMatrixWorld(true);
    const origin = new THREE.Vector3(0, y, 50);
    return handle.raycast(new THREE.Raycaster(origin, new THREE.Vector3(0, 0, -1)));
  };
  return { handle, root, invalidate, hitAt };
}

const Y = new THREE.Vector3(0, 1, 0);
const ZERO = new THREE.Vector3(0, 0, 0);
const O = new THREE.Vector3(0, 0, 0);

describe("DragHandle orientation", () => {
  it("uses requested vertical screen direction for an explicit scalar proxy", () => {
    const { handle, root } = makeHandle();
    const camera = new THREE.PerspectiveCamera(60, 1, 0.1, 100);
    camera.position.set(0, 0, 20);
    camera.lookAt(0, 0, 0);
    camera.updateMatrixWorld();
    camera.updateProjectionMatrix();
    handle.setScreenProxy(O);
    handle.orient(camera, 800, 800, worldPerPixel(camera, handle.worldAnchor(), 800));

    const up = new THREE.Vector3(0, 1, 0).applyQuaternion(root.children[0].quaternion);
    expect(up.y).toBeGreaterThan(0.99);
  });

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
  it("uses a compact centered two-way glyph inside a 30 × 40 px corridor", () => {
    const { handle, root, hitAt } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    const geometry = parts(root).fill.geometry;
    geometry.computeBoundingBox();
    const bounds = geometry.boundingBox;

    expect(bounds).not.toBeNull();
    expect(bounds?.min.y).toBeCloseTo(-15, 6);
    expect(bounds?.max.y).toBeCloseTo(15, 6);
    expect([hitAt(18), hitAt(-18), hitAt(22), hitAt(-22)]).toEqual([true, true, false, false]);
  });

  it("reports the corridor's screen box at the glyph's current roll", () => {
    const { handle } = makeHandle();
    const camera = new THREE.PerspectiveCamera(60, 1, 0.1, 100);
    camera.position.set(0, 0, 20);
    camera.lookAt(0, 0, 0);
    camera.updateMatrixWorld();
    // Rolled by a real world axis, since the glyph angle now comes from the
    // projected path and not from a hand-supplied screen direction.
    const extentsFor = (axis: [number, number, number]) => {
      handle.setAxis(O, new THREE.Vector3(...axis));
      handle.orient(camera, 800, 800, worldPerPixel(camera, handle.worldAnchor(), 800));
      return handle.corridorHalfExtentsPx();
    };

    const vertical = extentsFor([0, 1, 0]);
    expect(vertical.x).toBeCloseTo(15, 9);
    expect(vertical.y).toBeCloseTo(20, 9);
    const horizontal = extentsFor([1, 0, 0]);
    expect(horizontal.x).toBeCloseTo(20, 9);
    expect(horizontal.y).toBeCloseTo(15, 9);
    const diagonal = extentsFor([1, 1, 0]);
    expect(diagonal.x).toBeCloseTo(35 / Math.SQRT2, 9);
    expect(diagonal.y).toBeCloseTo(35 / Math.SQRT2, 9);
  });

  it("twoWay is grabbable from the NEGATIVE side", () => {
    const { handle, hitAt } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    expect(handle.axisMode).toBe("twoWay");
    // Across the arrow well below the origin — inside the mirrored head.
    expect(hitAt(-10)).toBe(true);
  });

  it("forward keeps the same centered corridor during compatibility migration", () => {
    const { handle, hitAt } = makeHandle();
    handle.setAxis(O, Y, "forward");
    expect(hitAt(-10)).toBe(true);
    expect(hitAt(18)).toBe(true);
  });

  it("switching mode never creates a mismatched pick envelope", () => {
    const { handle, hitAt } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    expect(hitAt(-10)).toBe(true);
    handle.setAxis(O, Y, "forward");
    expect(hitAt(-10)).toBe(true);
  });

  it("a hidden handle is never picked", () => {
    const { handle, hitAt } = makeHandle();
    handle.setAxis(O, Y, "twoWay");
    handle.setVisible(false);
    expect(hitAt(0)).toBe(false);
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
    expect(hitAt(-10)).toBe(true);
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
  /** Square, to match the aspect-1 cameras below. */
  const VW = 600;
  const VH = 600;
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
      handle.orient(cam, VW, VH, worldPerPixel(cam, handle.worldAnchor(), VH));
      const toCamera = cam.position.clone().normalize();
      expect(facing(root).dot(toCamera)).toBeGreaterThan(0.999);
    }
  });

  it("runs along the axis's screen direction", () => {
    const { handle, root } = makeHandle();
    // World +Z, viewed from +X: straight up the screen.
    handle.setAxis(O, new THREE.Vector3(0, 0, 1));
    const cam = camera([60, 0, 0]);
    handle.orient(cam, VW, VH, worldPerPixel(cam, handle.worldAnchor(), VH));

    const screenUp = new THREE.Vector3(0, 1, 0).applyQuaternion(cam.quaternion);
    expect(along(root).dot(screenUp)).toBeGreaterThan(0.999);
  });

  it("a FROZEN axis holds a full-length arrow when it points at the camera", () => {
    const { handle, root } = makeHandle();
    handle.setAxis(O, new THREE.Vector3(0, 0, 1));
    const side = camera([60, 0, 0]);
    handle.orient(side, VW, VH, worldPerPixel(side, handle.worldAnchor(), VH));
    const before = along(root).clone();
    handle.freeze(handle.mapping()); // the WHOLE mapping, as at grab

    // Now look straight down that axis: it has NO screen direction left.
    const down = camera([0, 0, 60]);
    handle.orient(down, VW, VH, worldPerPixel(down, handle.worldAnchor(), VH));

    // The last good angle is held rather than spinning to an arbitrary one, and
    // the arrow still faces the viewer at full length — not the old dot.
    const heldInCamera = before.clone().applyQuaternion(side.quaternion.clone().invert());
    const nowInCamera = along(root).clone().applyQuaternion(down.quaternion.clone().invert());
    expect(nowInCamera.dot(heldInCamera)).toBeGreaterThan(0.999);
    expect(facing(root).dot(down.position.clone().normalize())).toBeGreaterThan(0.999);
  });

  /*
   * The PERSPECTIVE case the old camera-space shortcut could not express.
   *
   * Both handles carry the SAME world axis (+Z) under the SAME camera. The
   * shortcut derived the screen angle from the axis and the camera rotation
   * alone, so it necessarily gave both the identical angle. The true projection
   * depends on the anchor too: off the optical axis, +Z runs towards the
   * vanishing point and picks up a sideways component whose sign follows which
   * side of centre the anchor sits on.
   */
  it("gives the SAME world axis different screen angles at different anchors", () => {
    // Camera on −Y looking at the origin, world Z up: screen right is +X and the
    // VIEW direction is +Y. An axis along +Y is therefore end-on at the centre
    // and purely radial away from it — the cleanest read of the anchor term.
    const cam = camera([0, -60, 0]);
    const axis = new THREE.Vector3(0, 1, 0);

    const left = makeHandle();
    left.handle.setAxis(new THREE.Vector3(-25, 0, 0), axis);
    left.handle.orient(cam, VW, VH, worldPerPixel(cam, left.handle.worldAnchor(), VH));

    const right = makeHandle();
    right.handle.setAxis(new THREE.Vector3(25, 0, 0), axis);
    right.handle.orient(cam, VW, VH, worldPerPixel(cam, right.handle.worldAnchor(), VH));

    const inCam = (root: THREE.Object3D) =>
      along(root).clone().applyQuaternion(cam.quaternion.clone().invert());

    // Mirror-image anchors ⇒ opposite horizontal components, and not a tie.
    expect(Math.sign(inCam(left.root).x)).toBe(-Math.sign(inCam(right.root).x));
    expect(Math.abs(inCam(left.root).x)).toBeGreaterThan(0.1);
    expect(inCam(left.root).dot(inCam(right.root))).toBeLessThan(0.999);
  });
});

describe("DragHandle pick corridor", () => {
  /** Ray through the glyph plane, offset `off` px sideways from the axis. */
  const hitOffAxis = (handle: DragHandle, root: THREE.Group, off: number): boolean => {
    root.updateMatrixWorld(true);
    const origin = new THREE.Vector3(off, 0, 50);
    return handle.raycast(new THREE.Raycaster(origin, new THREE.Vector3(0, 0, -1)));
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

/*
 * ONE mapping for glyph, pick and gesture (R03, R06). `computeMapping` is the
 * classifier's answer for the current camera; `orient` draws it; a strategy
 * frozen at grab survives the per-frame `setAxis` the controller issues.
 */
describe("DragHandle mapping", () => {
  const VW = 600;
  const VH = 600;
  const camera = (position: [number, number, number]) => {
    const cam = new THREE.PerspectiveCamera(50, 1, 0.1, 500);
    cam.up.set(0, 0, 1);
    cam.position.set(...position);
    cam.lookAt(0, 0, 0);
    cam.updateMatrixWorld(true);
    return cam;
  };
  const wpp = (cam: THREE.Camera, handle: DragHandle) => worldPerPixel(cam, handle.worldAnchor(), VH);
  const orient = (handle: DragHandle, cam: THREE.Camera) => handle.orient(cam, VW, VH, wpp(cam, handle));

  /** The drawn glyph's length direction in CSS pixels (+Y down), unit. */
  const glyphDir = (root: THREE.Object3D, cam: THREE.Camera): [number, number] => {
    const v = new THREE.Vector3(0, 1, 0)
      .applyQuaternion((root.children[0] as THREE.Group).quaternion)
      .applyQuaternion(cam.quaternion.clone().invert());
    const l = Math.hypot(v.x, v.y);
    return [v.x / l, -v.y / l];
  };

  /** CSS-px screen derivative of `dir` at `anchor`, by finite difference through three's own `project`. */
  const screenDerivative = (cam: THREE.Camera, anchor: THREE.Vector3, dir: THREE.Vector3): [number, number] => {
    const px = (p: THREE.Vector3): [number, number] => {
      const n = p.clone().project(cam);
      return [((n.x + 1) / 2) * VW, ((1 - n.y) / 2) * VH];
    };
    const eps = 1e-5;
    const a = px(anchor);
    const b = px(anchor.clone().addScaledVector(dir.clone().normalize(), eps));
    return [(b[0] - a[0]) / eps, (b[1] - a[1]) / eps];
  };
  const unit2 = (v: [number, number]): [number, number] => {
    const l = Math.hypot(v[0], v[1]);
    return [v[0] / l, v[1] / l];
  };

  // Perpendicular in 3D, like a fillet's outward axis and its edge tangent.
  const outward = new THREE.Vector3(1, -1, -1).normalize();
  const tangent = new THREE.Vector3(1, 0, 1).normalize();
  const anchor = new THREE.Vector3(8, 3, -2);

  /*
   * β = 0 (docs/design/astra/modeling-handle-attachment.md §3). The glyph draws
   * the TRUE projected derivative of the point that moves, with nothing rejected
   * from it. Rejecting the projected edge tangent drew a direction H(q) cannot
   * follow: at a 1° screen angle it rotated the drawn direction ~89° and
   * multiplied the gain by 57.2987.
   */
  it("draws the RAW projected derivative for an oblique perspective edge op", () => {
    const { handle, root } = makeHandle();
    const cam = camera([40, -70, 50]);
    handle.setAxis(anchor, outward);
    orient(handle, cam);

    const raw = screenDerivative(cam, anchor, outward);
    const t = unit2(screenDerivative(cam, anchor, tangent));
    const dot = raw[0] * t[0] + raw[1] * t[1];
    const rejected = unit2([raw[0] - dot * t[0], raw[1] - dot * t[1]]);
    const rawDir = unit2(raw);
    // The case only means something if rejection WOULD have moved the axis.
    expect(Math.acos(rawDir[0] * rejected[0] + rawDir[1] * rejected[1])).toBeGreaterThan((10 * Math.PI) / 180);

    const drawn = glyphDir(root, cam);
    expect(drawn[0]).toBeCloseTo(rawDir[0], 3);
    expect(drawn[1]).toBeCloseTo(rawDir[1], 3);
  });

  it("the glyph direction IS computeMapping's direction", () => {
    const { handle, root } = makeHandle();
    const cam = camera([40, -70, 50]);
    handle.setAxis(anchor, outward);
    const mapping = handle.computeMapping(cam, VW, VH, wpp(cam, handle));
    expect(mapping.kind).toBe("world");
    expect(handle.mapping()).toBeNull();

    orient(handle, cam);
    const drawn = glyphDir(root, cam);
    expect(drawn[0]).toBeCloseTo(mapping.kind === "disabled" ? 0 : mapping.direction[0], 9);
    expect(drawn[1]).toBeCloseTo(mapping.kind === "disabled" ? 0 : mapping.direction[1], 9);
    expect(handle.mapping()).toEqual(mapping);
  });

  /*
   * THE HANDLE MOVES (derivation §5). H(q) = E + q·b, so the attachment is a
   * function of the value: an arrow that stays put while the radius grows is
   * not attached to anything.
   */
  it("sits at H(q) and travels along the path as the value changes", () => {
    const { handle } = makeHandle();
    const path: LinearHandlePath = { q0Mm: 0, point0Mm: [40, 40, 20], dPointDValue: [Math.SQRT1_2, Math.SQRT1_2, 0] };
    handle.setValuePath(path, 0);
    expect(handle.worldAnchor().toArray()).toEqual([40, 40, 20]);

    handle.setValue(2);
    const at2 = handle.worldAnchor();
    expect(at2.x).toBeCloseTo(41.414214, 6);
    expect(at2.y).toBeCloseTo(41.414214, 6);
    expect(at2.z).toBeCloseTo(20, 9);

    handle.setValue(0.1);
    expect(handle.worldAnchor().x).toBeCloseTo(40 + 0.1 * Math.SQRT1_2, 9);
  });

  it("a frozen mapping is drawn VERBATIM, whatever the camera does next", () => {
    const { handle, root } = makeHandle();
    const cam = camera([40, -70, 50]);
    handle.setAxis(anchor, outward);
    const frozen = handle.computeMapping(cam, VW, VH, wpp(cam, handle));
    handle.freeze(frozen);
    orient(handle, camera([0, 0, 90]));
    expect(handle.mapping()).toBe(frozen);
    const drawn = glyphDir(root, camera([0, 0, 90]));
    if (frozen.kind === "world") {
      expect(drawn[0]).toBeCloseTo(frozen.direction[0], 6);
      expect(drawn[1]).toBeCloseTo(frozen.direction[1], 6);
    }
  });

  it("an unfrozen end-on axis draws the vertical proxy and reports it", () => {
    const { handle, root } = makeHandle();
    handle.setAxis(O, new THREE.Vector3(1, 0, 0));
    const side = camera([0, -60, 0]);
    orient(handle, side);
    expect(handle.mapping()!.kind).toBe("world");
    expect(glyphDir(root, side)[0]).toBeCloseTo(1, 6);

    const endOn = camera([60, 0, 0]);
    orient(handle, endOn);
    expect(handle.mapping()!.kind).toBe("proxy");
    expect(handle.computeMapping(endOn, VW, VH, wpp(endOn, handle)).kind).toBe("proxy");
    const drawn = glyphDir(root, endOn);
    expect(drawn[0]).toBeCloseTo(0, 6);
    expect(drawn[1]).toBeCloseTo(-1, 6);
    expect(handle.corridorHalfExtentsPx().x).toBeCloseTo(15, 6);
    expect(handle.corridorHalfExtentsPx().y).toBeCloseTo(20, 6);
  });

  it("a frozen world mapping survives per-frame setAxis and a camera turned end-on", () => {
    const { handle, root } = makeHandle();
    const axis = new THREE.Vector3(1, 0, 0);
    handle.setAxis(O, axis);
    const side = camera([0, -60, 0]);
    orient(handle, side);
    const frozen = handle.mapping()!;
    expect(frozen.kind).toBe("world");
    handle.freeze(frozen);

    const endOn = camera([60, 0, 0]);
    for (let i = 0; i < 3; i++) {
      handle.setAxis(O, axis, "twoWay");
      orient(handle, endOn);
    }
    // The FROZEN object keeps its gain: a re-derived projection is exactly what
    // changes the mapping under the user's hand mid-drag.
    expect(handle.mapping()).toBe(frozen);
    const drawn = glyphDir(root, endOn);
    expect(drawn[0]).toBeCloseTo(1, 6);
    expect(drawn[1]).toBeCloseTo(0, 6);
    expect(handle.corridorHalfExtentsPx().x).toBeCloseTo(20, 6);
    // The unfrozen classifier still reports the truth about the geometry.
    expect(handle.computeMapping(endOn, VW, VH, wpp(endOn, handle)).kind).toBe("proxy");
  });

  it("a frozen screen proxy persists across setAxis on a well-conditioned axis", () => {
    const { handle, root } = makeHandle();
    const axis = new THREE.Vector3(1, 0, 0);
    handle.setAxis(O, axis);
    handle.freeze({ kind: "proxy", q0Mm: 0, direction: [0, -1], mmPerPx: 0.5, reason: "poorScreenSensitivity" });
    const side = camera([0, -60, 0]);
    for (let i = 0; i < 3; i++) {
      handle.setAxis(O, axis);
      orient(handle, side);
    }
    expect(handle.mapping()).toMatchObject({ kind: "proxy", direction: [0, -1], mmPerPx: 0.5 });
    const drawn = glyphDir(root, side);
    expect(drawn[0]).toBeCloseTo(0, 6);
    expect(drawn[1]).toBeCloseTo(-1, 6);
  });

  it("reset() clears the freeze", () => {
    const { handle } = makeHandle();
    const cam = camera([40, -70, 50]);
    handle.setAxis(anchor, outward);
    handle.freeze({ kind: "proxy", q0Mm: 0, direction: [0, -1], mmPerPx: 1, reason: "noScale" });
    handle.reset();
    orient(handle, cam);
    const m = handle.mapping()!;
    expect(m.kind).toBe("world");
    const raw = unit2(screenDerivative(cam, anchor, outward));
    if (m.kind !== "disabled") {
      expect(m.direction[0]).toBeCloseTo(raw[0], 3);
      expect(m.direction[1]).toBeCloseTo(raw[1], 3);
    }
  });

  it("an explicit screen proxy maps up the screen at the anchor's own scale", () => {
    const { handle } = makeHandle();
    const cam = camera([40, -70, 50]);
    handle.setScreenProxy(O, 3);
    expect(handle.computeMapping(cam, VW, VH, wpp(cam, handle))).toMatchObject({
      kind: "proxy",
      direction: [0, -1],
      q0Mm: 3,
    });
  });
});

/* N9: `setScale` runs inside every rendered frame; an unconditional invalidate
 * there schedules one wasted frame after each one, so idle never reaches zero. */
describe("DragHandle scale invalidation", () => {
  it("does not invalidate when the scale has not changed", () => {
    const { handle, invalidate } = makeHandle();
    handle.setScale(0.25);
    invalidate.mockClear();
    handle.setScale(0.25);
    handle.setScale(0.25 * (1 + 1e-12));
    expect(invalidate).not.toHaveBeenCalled();

    handle.setScale(0.5);
    expect(invalidate).toHaveBeenCalledTimes(1);
  });
});
