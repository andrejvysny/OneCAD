import { describe, it, expect } from "vitest";
import * as THREE from "three";
import {
  projectToScreen,
  HtmlOverlayDriver,
  CLUSTER_GAP_PX,
  KEEP_OUT_PAD_PX,
  keepOutShiftY,
} from "./HtmlOverlayDriver";

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

  /*
   * A NEGATIVE offsetPx is the same clearance on the OTHER side of the axis, so
   * two families of items anchored to the same point can be sent opposite ways
   * instead of stacking (sketch constraint glyphs vs dimension chips —
   * badgeLayout.ts). The half-own-size edge translate has to flip with it: left
   * unsigned it would drag the element back across the axis, which is the one
   * thing `offsetPx` (a NEAR-EDGE clearance) promises never happens.
   */
  it("a NEGATIVE offsetPx mirrors the side, keeping the same clearance", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const plus = document.createElement("div");
    const minus = document.createElement("div");
    const bare = document.createElement("div");
    const at = new THREE.Vector3(0, 2, 0);
    const axisFrom = new THREE.Vector3(0, 0, 0);
    driver.register("plus", plus, at, { axisFrom, offsetPx: 40 });
    driver.register("minus", minus, at, { axisFrom, offsetPx: -40 });
    driver.register("bare", bare, at);
    driver.update(cam, 400, 400);

    const [bx, by] = xyOf(bare);
    const [px, py] = xyOf(plus);
    const [mx, my] = xyOf(minus);
    expect(px - bx).toBeCloseTo(-(mx - bx), 4); // mirrored…
    expect(Math.abs(mx - bx)).toBeCloseTo(40, 4); // …at the same clearance
    expect(py).toBeCloseTo(by, 4);
    expect(my).toBeCloseTo(by, 4);

    // The percentage translate (half the element's own box) points the same way
    // as the pixel one, so the element grows AWAY from the axis on both sides.
    const edgeX = (el: HTMLElement): number => {
      const all = [...el.style.transform.matchAll(/translate\((-?[\d.]+)%, (-?[\d.]+)%\)/g)];
      return Number(all[all.length - 1][1]);
    };
    expect(Math.sign(edgeX(plus))).toBe(Math.sign(px - bx));
    expect(Math.sign(edgeX(minus))).toBe(Math.sign(mx - bx));
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

  it("cluster push-down follows actual screen position, not registration order", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const below = document.createElement("div");
    const above = document.createElement("div");
    // "below" is registered FIRST but projects BELOW "above" (world +Y is up
    // the screen, so a smaller Y is lower on screen). A push-down keyed on
    // registration order would treat "below" as the top of the group and
    // never push it down to clear "above" — reversing their visual order.
    driver.register("below", below, new THREE.Vector3(0, -1, 0), { clusterId: "c" });
    driver.register("above", above, new THREE.Vector3(0, 1, 0), { clusterId: "c" });
    driver.update(cam, 400, 400);
    const [, yBelow] = xyOf(below);
    const [, yAbove] = xyOf(above);
    expect(yBelow).toBeGreaterThan(yAbove);
    expect(yBelow - yAbove).toBeGreaterThanOrEqual(CLUSTER_GAP_PX);
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

  /*
   * Keep-out (MC-R9). `ce3d6bf` moved the value arrow into screen space and left
   * the chip on the same anchor, so the chip covered the arrow's grab area and
   * `isExcludedClickAwayTarget` refused every press meant for the arrow — the
   * arrow could not be grabbed at all wherever the chip sat on it.
   *
   * The numbers below are MEASURED from the live app (armed Fillet on the mock
   * box, `?vpdebug`): the arrow's box was {778.76, 329.69, 120, 120}, the chip's
   * rect {722.78, 372.36, 181.27, 35.25}, and `document.elementFromPoint` at the
   * arrow's own grab pixel (888, 398) resolved to `chip-cancel`.
   */
  const CHIP_SIZE = { w: 181.27, h: 35.25 };

  function intersects(
    a: { x: number; y: number; width: number; height: number },
    b: { x: number; y: number; width: number; height: number },
  ): boolean {
    return (
      a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
    );
  }

  /** Register a chip whose measured size is fixed (jsdom reports 0×0 rects). */
  function sizedChip(parent: HTMLElement): HTMLElement {
    const chip = document.createElement("div");
    chip.getBoundingClientRect = () =>
      ({ width: CHIP_SIZE.w, height: CHIP_SIZE.h }) as DOMRect;
    parent.appendChild(chip);
    return chip;
  }

  it("pushes an opted-in chip clear of the keep-out box, and leaves it alone without one", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const parent = document.createElement("div");
    const chip = sizedChip(parent);
    driver.register("chip", chip, new THREE.Vector3(0, 0, 0), { avoidKeepOut: true });

    // No keep-out: the chip stays exactly on its projected anchor.
    driver.update(cam, 400, 400);
    const [, restingY] = xyOf(chip);
    expect(restingY).toBeCloseTo(200, 0);

    // A box centred on that same anchor — the real overlap case.
    const box = { x: 200 - 60, y: 200 - 60, width: 120, height: 120 };
    driver.update(cam, 400, 400, box);
    const [cx, cy] = xyOf(chip);
    expect(cy).not.toBeCloseTo(restingY, 0);
    expect(
      intersects(
        { x: cx - CHIP_SIZE.w / 2, y: cy - CHIP_SIZE.h / 2, ...{ width: CHIP_SIZE.w, height: CHIP_SIZE.h } },
        box,
      ),
    ).toBe(false);

    // …and it returns to the anchor once the arrow is gone (null keep-out).
    driver.update(cam, 400, 400, null);
    expect(xyOf(chip)[1]).toBeCloseTo(restingY, 0);
  });

  it("keeps its side while it still overlaps, so the chip cannot flip across the arrow mid-drag", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const parent = document.createElement("div");
    const chip = sizedChip(parent);
    driver.register("chip", chip, new THREE.Vector3(0, 0, 0), { avoidKeepOut: true });

    // A box sitting slightly BELOW the anchor: the shorter push is upward.
    driver.update(cam, 400, 400, { x: 140, y: 190, width: 120, height: 120 });
    const upY = xyOf(chip)[1];
    expect(upY).toBeLessThan(200);

    // Now a box slightly ABOVE it — the shorter push is downward, but the item is
    // already committed to the up side and must stay there.
    driver.update(cam, 400, 400, { x: 140, y: 90, width: 120, height: 120 });
    expect(xyOf(chip)[1]).toBeLessThan(200);
  });

  it("does not displace a chip that never opted in", () => {
    const cam = camAt(10);
    const driver = new HtmlOverlayDriver();
    const parent = document.createElement("div");
    const chip = sizedChip(parent);
    driver.register("chip", chip, new THREE.Vector3(0, 0, 0));
    driver.update(cam, 400, 400, { x: 140, y: 140, width: 120, height: 120 });
    expect(xyOf(chip)[1]).toBeCloseTo(200, 0);
  });
});

describe("keepOutShiftY (pure)", () => {
  const box = { x: 100, y: 100, width: 120, height: 120 };

  it("returns null when the rects do not overlap on either axis", () => {
    expect(keepOutShiftY({ x: 300, y: 100, width: 50, height: 20 }, box, 6)).toBeNull();
    expect(keepOutShiftY({ x: 100, y: 300, width: 50, height: 20 }, box, 6)).toBeNull();
  });

  it("clears the box with the pad, on the shorter side", () => {
    // Rect centred well above the box's middle ⇒ up is shorter.
    const rect = { x: 110, y: 110, width: 50, height: 20 };
    const shift = keepOutShiftY(rect, box, 6);
    expect(shift).not.toBeNull();
    expect(shift?.dir).toBe(-1);
    // Its bottom edge lands exactly `pad` above the box's top.
    expect(rect.y + shift!.dy + rect.height).toBeCloseTo(box.y - 6, 5);
  });

  it("honours a sticky side even when the other one is shorter", () => {
    const rect = { x: 110, y: 110, width: 50, height: 20 };
    const shift = keepOutShiftY(rect, box, 6, 1);
    expect(shift?.dir).toBe(1);
    expect(rect.y + shift!.dy).toBeCloseTo(box.y + box.height + 6, 5);
  });

  it("clears the MEASURED production overlap (armed Fillet on the mock box)", () => {
    const chip = { x: 722.78, y: 372.36, width: 181.27, height: 35.25 };
    const arrow = { x: 778.76, y: 329.69, width: 120, height: 120 };
    const shift = keepOutShiftY(chip, arrow, KEEP_OUT_PAD_PX);
    expect(shift).not.toBeNull();
    const moved = { ...chip, y: chip.y + shift!.dy };
    expect(
      moved.x < arrow.x + arrow.width &&
        arrow.x < moved.x + moved.width &&
        moved.y < arrow.y + arrow.height &&
        arrow.y < moved.y + moved.height,
    ).toBe(false);
    // The arrow's own grab pixel (888, 398) is no longer inside the chip.
    expect(
      888 >= moved.x && 888 <= moved.x + moved.width && 398 >= moved.y && 398 <= moved.y + moved.height,
    ).toBe(false);
  });
});
