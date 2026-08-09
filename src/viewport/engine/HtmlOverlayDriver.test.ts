import { describe, it, expect } from "vitest";
import * as THREE from "three";
import { projectToScreen, HtmlOverlayDriver } from "./HtmlOverlayDriver";

function viewProjFor(camera: THREE.Camera): THREE.Matrix4 {
  camera.updateMatrixWorld();
  return new THREE.Matrix4().multiplyMatrices(
    camera.projectionMatrix,
    camera.matrixWorldInverse,
  );
}

describe("projectToScreen (pure)", () => {
  const cam = new THREE.PerspectiveCamera(60, 800 / 600, 0.1, 1000);
  cam.position.set(0, 0, 10);
  cam.up.set(0, 1, 0);
  cam.lookAt(0, 0, 0);
  cam.updateProjectionMatrix();
  const vp = viewProjFor(cam);

  it("projects the look-at point to screen center", () => {
    const p = projectToScreen({ x: 0, y: 0, z: 0 }, vp, 800, 600);
    expect(p.visible).toBe(true);
    expect(p.x).toBeCloseTo(400, 0);
    expect(p.y).toBeCloseTo(300, 0);
  });

  it("hides points behind the camera", () => {
    const p = projectToScreen({ x: 0, y: 0, z: 20 }, vp, 800, 600);
    expect(p.visible).toBe(false);
  });

  it("hides points outside the frustum", () => {
    const p = projectToScreen({ x: 1000, y: 0, z: 0 }, vp, 800, 600);
    expect(p.visible).toBe(false);
  });
});

describe("HtmlOverlayDriver", () => {
  it("writes a transform for a visible item and hides behind-camera ones", () => {
    const cam = new THREE.PerspectiveCamera(60, 1, 0.1, 100);
    cam.position.set(0, 0, 10);
    cam.up.set(0, 1, 0);
    cam.lookAt(0, 0, 0);
    cam.updateProjectionMatrix();
    cam.updateMatrixWorld();

    const driver = new HtmlOverlayDriver();
    const front = document.createElement("div");
    const back = document.createElement("div");
    driver.register("front", front, new THREE.Vector3(0, 0, 0));
    driver.register("back", back, new THREE.Vector3(0, 0, 30));
    expect(driver.size).toBe(2);

    driver.update(cam, 400, 400);
    expect(front.style.transform).toContain("translate");
    expect(front.style.display).toBe("");
    expect(back.style.display).toBe("none");

    driver.unregister("back");
    expect(driver.size).toBe(1);
  });

  /*
   * AXIS PLACEMENT. A value chip must sit BESIDE the arrow it belongs to, not on
   * the geometry that arrow is growing. The offset is perpendicular to the
   * projected axis and picked deterministically — a per-frame "nearest side"
   * choice would make the chip jump across the arrow mid-drag.
   */
  function camAt(z: number): THREE.PerspectiveCamera {
    const cam = new THREE.PerspectiveCamera(60, 1, 0.1, 1000);
    cam.position.set(0, 0, z);
    cam.up.set(0, 1, 0);
    cam.lookAt(0, 0, 0);
    cam.updateProjectionMatrix();
    cam.updateMatrixWorld();
    return cam;
  }

  const xyOf = (el: HTMLElement): [number, number] => {
    const m = /translate\((-?[\d.]+)px, (-?[\d.]+)px\)/.exec(el.style.transform);
    if (!m) throw new Error(`no pixel translate in ${el.style.transform}`);
    return [Number(m[1]), Number(m[2])];
  };

  it("an item WITHOUT an axis is byte-identical to before", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const plain = document.createElement("div");
    driver.register("plain", plain, new THREE.Vector3(1, 0, 0));
    driver.update(cam, 400, 400);
    const [x, y] = xyOf(plain);

    // The same point, registered with a placement that carries no offset.
    const other = document.createElement("div");
    driver.register("other", other, new THREE.Vector3(1, 0, 0), { axisFrom: new THREE.Vector3() });
    driver.update(cam, 400, 400);
    expect(xyOf(other)).toEqual([x, y]);
  });

  it("offsets PERPENDICULAR to the axis, on a stable side", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const chip = document.createElement("div");
    // Axis runs +Y in world = UP the screen, so the perpendicular is horizontal.
    driver.register("chip", chip, new THREE.Vector3(0, 2, 0), {
      axisFrom: new THREE.Vector3(0, 0, 0),
      offsetPx: 40,
    });
    driver.update(cam, 400, 400);
    const [x, y] = xyOf(chip);

    const bare = document.createElement("div");
    driver.register("bare", bare, new THREE.Vector3(0, 2, 0));
    driver.update(cam, 400, 400);
    const [bx, by] = xyOf(bare);

    expect(Math.abs(x - bx)).toBeCloseTo(40, 4); // purely sideways…
    expect(y).toBeCloseTo(by, 4); // …and not along the axis
    const side = Math.sign(x - bx);

    // Reverse the axis: the chip crosses to the other side, and stays there.
    driver.setWorldPos("chip", new THREE.Vector3(0, -2, 0));
    driver.setAxisFrom("chip", new THREE.Vector3(0, 0, 0));
    driver.update(cam, 400, 400);
    driver.setWorldPos("bare", new THREE.Vector3(0, -2, 0));
    driver.update(cam, 400, 400);
    expect(Math.sign(xyOf(chip)[0] - xyOf(bare)[0])).toBe(-side);
  });

  it("falls back to a fixed offset when the axis projects to a point", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const chip = document.createElement("div");
    // Head and tail on the view axis: no perpendicular exists.
    driver.register("chip", chip, new THREE.Vector3(0, 0, 0), {
      axisFrom: new THREE.Vector3(0, 0, 2),
      offsetPx: 40,
    });
    driver.update(cam, 400, 400);
    const [x, y] = xyOf(chip);
    // The fallback is a fixed DIAGONAL (up-right), so each component is 40/√2.
    const diag = 40 * Math.SQRT1_2;
    expect(Number.isFinite(x)).toBe(true);
    expect(Number.isFinite(y)).toBe(true);
    expect(x).toBeCloseTo(200 + diag, 4);
    expect(y).toBeCloseTo(200 - diag, 4);
  });

  it("a tail BEHIND the camera does not drag the anchor with it", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const chip = document.createElement("div");
    driver.register("chip", chip, new THREE.Vector3(0, 0, 0), {
      axisFrom: new THREE.Vector3(0, 0, 40), // behind the camera
      offsetPx: 40,
    });
    driver.update(cam, 400, 400);
    const [x, y] = xyOf(chip);
    const diag = 40 * Math.SQRT1_2;
    expect(x).toBeCloseTo(200 + diag, 4); // the fixed fallback, not a garbage direction
    expect(y).toBeCloseTo(200 - diag, 4);
  });
});
