import { describe, it, expect } from "vitest";
import * as THREE from "three";
import { projectToScreen, HtmlOverlayDriver, CLUSTER_GAP_PX } from "./HtmlOverlayDriver";

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

  /*
   * CLUSTERS. A short line's length + angle chips project to nearly the same
   * screen point — the driver must keep cluster neighbours a constant screen
   * distance apart instead of letting them stack on top of each other.
   */

  it("cluster neighbours that project onto the same point get the gap applied", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const a = document.createElement("div");
    const b = document.createElement("div");
    const at = new THREE.Vector3(0, 0, 0);
    driver.register("a", a, at, { clusterId: "c" });
    driver.register("b", b, at, { clusterId: "c" }); // registered AFTER a
    driver.update(cam, 400, 400);
    const [, ya] = xyOf(a);
    const [, yb] = xyOf(b);
    expect(yb - ya).toBeCloseTo(CLUSTER_GAP_PX, 4);
  });

  it("cluster members already far enough apart are NOT pushed", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const a = document.createElement("div");
    const b = document.createElement("div");
    const bare = document.createElement("div");
    driver.register("a", a, new THREE.Vector3(0, 0, 0), { clusterId: "c" });
    // World +Y is UP the screen; a point far BELOW a projects well past the gap.
    driver.register("b", b, new THREE.Vector3(0, -5, 0), { clusterId: "c" });
    driver.register("bare", bare, new THREE.Vector3(0, -5, 0));
    driver.update(cam, 400, 400);
    // b is already beyond the gap below a, so its cluster y equals its raw
    // projection — the gap is a floor, not a magnet.
    expect(xyOf(b)[1]).toBeCloseTo(xyOf(bare)[1], 4);
  });

  it("items WITHOUT a cluster id never push each other (they may overlap)", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const a = document.createElement("div");
    const b = document.createElement("div");
    const at = new THREE.Vector3(0, 0, 0);
    driver.register("a", a, at);
    driver.register("b", b, at);
    driver.update(cam, 400, 400);
    expect(xyOf(a)).toEqual(xyOf(b));
  });

  it("an invisible cluster member is skipped — it pushes nobody and is not pushed", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const a = document.createElement("div");
    const hidden = document.createElement("div");
    const bare = document.createElement("div");
    driver.register("a", a, new THREE.Vector3(0, 0, 0), { clusterId: "c" });
    driver.register("hidden", hidden, new THREE.Vector3(0, 0, 30), { clusterId: "c" }); // behind
    driver.register("bare", bare, new THREE.Vector3(0, 0, 0));
    driver.update(cam, 400, 400);
    expect(hidden.style.display).toBe("none");
    expect(xyOf(a)).toEqual(xyOf(bare)); // the visible member keeps its own spot
  });

  /*
   * LEADER LINE. An axis-anchored chip (fillet/revolve, generalized off extrude
   * in UNIFY-UX Phase 0) now draws a dashed line from the raw anchor to its
   * offset position, so the offset reads as "this chip belongs to that point"
   * rather than an unexplained jump. Only created for items registered WITH an
   * axis AND a real DOM parent (`mountChip` always appends before registering —
   * a parent-less item, as every other test above uses, gets none, matching
   * "no visible change" for anything that doesn't opt in).
   */

  it("an axis-anchored item with a DOM parent gets a leader line sibling", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const parent = document.createElement("div");
    const chip = document.createElement("div");
    parent.appendChild(chip);
    driver.register("chip", chip, new THREE.Vector3(0, 2, 0), {
      axisFrom: new THREE.Vector3(0, 0, 0),
      offsetPx: 40,
    });
    // Inserted BEFORE the chip, so the chip's own content paints on top.
    expect(parent.children.length).toBe(2);
    expect(parent.children[1]).toBe(chip);
    const leader = parent.children[0] as HTMLElement;
    expect(leader.style.borderTop).toContain("dashed");

    driver.update(cam, 400, 400);
    // The line spans from the raw anchor projection to the offset chip position —
    // a horizontal offset here, so it has real width and (near) zero rotation.
    expect(Number.parseFloat(leader.style.width)).toBeCloseTo(40, 0);
  });

  it("an item WITHOUT a DOM parent at register-time gets no leader line", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const chip = document.createElement("div"); // never appended anywhere
    driver.register("chip", chip, new THREE.Vector3(0, 2, 0), {
      axisFrom: new THREE.Vector3(0, 0, 0),
      offsetPx: 40,
    });
    driver.update(cam, 400, 400);
    // No sibling was ever created — nothing to assert on but that this doesn't throw
    // and the chip itself still positions normally.
    expect(chip.style.transform).toContain("translate");
  });

  it("a leader line shorter than the minimum is hidden, not drawn as a stub", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const parent = document.createElement("div");
    const chip = document.createElement("div");
    parent.appendChild(chip);
    // A 1px offset is below LEADER_MIN_LEN_PX — the chip essentially sits on its
    // own anchor, so the line would be an invisible stub rather than a real leader.
    driver.register("chip", chip, new THREE.Vector3(0, 2, 0), {
      axisFrom: new THREE.Vector3(0, 0, 0),
      offsetPx: 1,
    });
    driver.update(cam, 400, 400);
    const leader = parent.children[0] as HTMLElement;
    expect(leader.style.display).toBe("none");
  });

  it("a leader line hides when its chip goes off-screen, and unregister removes it", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const parent = document.createElement("div");
    const chip = document.createElement("div");
    parent.appendChild(chip);
    driver.register("chip", chip, new THREE.Vector3(0, 0, 30), {
      // behind the camera
      axisFrom: new THREE.Vector3(0, 0, 0),
      offsetPx: 40,
    });
    driver.update(cam, 400, 400);
    const leader = parent.children[0] as HTMLElement;
    expect(leader.style.display).toBe("none");

    driver.unregister("chip");
    expect(parent.children.length).toBe(1); // the leader line is gone, chip untouched by the driver
    expect(parent.contains(chip)).toBe(true);
  });

  it("the leader line tracks a LIVE move (setWorldPos/setAxisFrom), not just the register-time position", () => {
    // Extrude's arrow travels with the depth every drag frame via `moveChip`
    // (`ViewportEngine.moveChip` → `setWorldPos` + `setAxisFrom`), never a
    // re-register — this is the exact path the leader line must piggyback on
    // (UNIFY-UX Phase 3 regression gate) or it would lag behind the chip.
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const parent = document.createElement("div");
    const chip = document.createElement("div");
    parent.appendChild(chip);
    driver.register("chip", chip, new THREE.Vector3(0, 2, 0), {
      axisFrom: new THREE.Vector3(0, 0, 0),
      offsetPx: 40,
    });
    driver.update(cam, 400, 400);
    const leader = parent.children[0] as HTMLElement;
    const transformBefore = leader.style.transform;

    // Simulate a depth drag: both ends move, still live-mutating the SAME item.
    driver.setWorldPos("chip", new THREE.Vector3(0, 5, 0));
    driver.setAxisFrom("chip", new THREE.Vector3(0, 1, 0));
    driver.update(cam, 400, 400);

    // The offset MAGNITUDE (width) is `offsetPx`, a constant — unchanged by
    // design. Its POSITION must still move with the new anchor, or the line
    // would keep pointing at the drag's stale starting point.
    expect(leader.style.transform).not.toBe(transformBefore);
    // And the chip itself moved too, in lockstep — no desync between the two.
    expect(xyOf(chip)[1]).toBeLessThan(200); // world +Y projects UP the screen
  });
});
